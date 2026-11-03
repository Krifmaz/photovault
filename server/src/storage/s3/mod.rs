//! S3-compatible object store (AWS, Backblaze B2, Wasabi, Garage, MinIO, …).
//!
//! Presigned URLs (SigV4 query signing, [`sigv4`]) are handed to the apps for
//! uploads, multipart parts, multipart completion and downloads, exactly like
//! museum does with the AWS SDK: a v2 upload URL binds `content-length` and
//! `content-md5` as signed headers. Server-side operations (create/abort
//! multipart, HEAD, DELETE, copy, small PUT/GET) use header-signed requests
//! over [`http::Client`], the only module that touches TLS.

pub mod http;
pub mod sigv4;

use super::{MultipartUrls, ObjectStore, PartSpec, StorageError, StorageResult, validate_key};
use crate::config::S3Config;
use bytes::Bytes;
use sigv4::{Request, Signer, sha256_hex, uri_encode};
use std::time::Duration;

/// SigV4 presigned URLs are valid for at most 7 days.
const MAX_PRESIGN: u64 = 7 * 24 * 3600;
/// CopyObject's single-request limit; larger objects are copied part by part.
const MAX_SINGLE_COPY: u64 = 5 * 1024 * 1024 * 1024;
/// Part size for multipart copies.
const COPY_PART: u64 = 512 * 1024 * 1024;

/// A parsed endpoint: scheme, authority (host[:port]) and optional base path.
#[derive(Debug, Clone)]
struct Endpoint {
    scheme: String,
    authority: String,
    base_path: String,
}

impl Endpoint {
    fn parse(s: &str) -> anyhow::Result<Self> {
        let u = url::Url::parse(s)?;
        anyhow::ensure!(
            u.scheme() == "http" || u.scheme() == "https",
            "S3 endpoint must be http(s)"
        );
        let host = u
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("S3 endpoint has no host"))?;
        let authority = match u.port() {
            Some(p) => format!("{host}:{p}"),
            None => host.to_string(),
        };
        Ok(Self {
            scheme: u.scheme().to_string(),
            authority,
            base_path: u.path().trim_end_matches('/').to_string(),
        })
    }
}

/// S3 backend.
pub struct S3Store {
    signer: Signer,
    bucket: String,
    path_style: bool,
    endpoint: Endpoint,
    public: Endpoint,
    http: http::Client,
}

impl std::fmt::Debug for S3Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3Store")
            .field("bucket", &self.bucket)
            .field("endpoint", &self.endpoint)
            .field("public", &self.public)
            .field("path_style", &self.path_style)
            .finish_non_exhaustive()
    }
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn s(k: &str, v: &str) -> (String, String) {
    (k.to_string(), v.to_string())
}

/// First `<tag>…</tag>` in an XML document.
fn xml_tag<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let rest = xml.split_once(&open)?.1;
    Some(rest.split_once(&close)?.0)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl S3Store {
    /// Build from config.
    pub fn new(cfg: &S3Config) -> anyhow::Result<Self> {
        let secret = match &cfg.secret_key_file {
            Some(p) => std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("reading S3 secret_key_file: {e}"))?
                .trim()
                .to_string(),
            None => cfg.secret_key.clone(),
        };
        anyhow::ensure!(!cfg.bucket.is_empty(), "S3 bucket is not set");
        anyhow::ensure!(
            !cfg.access_key.is_empty() && !secret.is_empty(),
            "S3 access_key/secret_key are not set"
        );
        let endpoint = Endpoint::parse(&cfg.endpoint)?;
        let public = match &cfg.public_endpoint {
            Some(p) => Endpoint::parse(p)?,
            None => endpoint.clone(),
        };
        Ok(Self {
            signer: Signer {
                access_key: cfg.access_key.clone(),
                secret_key: secret,
                region: cfg.region.clone(),
                service: "s3".into(),
            },
            bucket: cfg.bucket.clone(),
            path_style: cfg.path_style,
            endpoint,
            public,
            http: http::Client::new()?,
        })
    }

    /// `(host header, encoded path)` for `key` on endpoint `ep`.
    fn target(&self, ep: &Endpoint, key: &str) -> (String, String) {
        if self.path_style {
            (
                ep.authority.clone(),
                format!(
                    "{}/{}/{}",
                    ep.base_path,
                    uri_encode(&self.bucket, false),
                    uri_encode(key, true)
                ),
            )
        } else {
            (
                format!("{}.{}", self.bucket, ep.authority),
                format!("{}/{}", ep.base_path, uri_encode(key, true)),
            )
        }
    }

    /// Presign `method` on `key` for the apps (public endpoint).
    fn presign(
        &self,
        method: &str,
        key: &str,
        query: Vec<(String, String)>,
        mut headers: Vec<(String, String)>,
        ttl: Duration,
    ) -> StorageResult<String> {
        validate_key(key)?;
        let (host, path) = self.target(&self.public, key);
        headers.push(s("host", &host));
        let req = Request {
            method,
            path: &path,
            query,
            headers,
        };
        let qs = self
            .signer
            .presign_query(&req, now_secs(), ttl.as_secs().clamp(1, MAX_PRESIGN));
        Ok(format!("{}://{host}{path}?{qs}", self.public.scheme))
    }

    /// Send a header-signed request to the internal endpoint. Retries
    /// connection errors and 5xx a few times (all calls are idempotent or harmless to repeat).
    async fn call(
        &self,
        op: &str,
        method: &str,
        key: &str,
        query: Vec<(String, String)>,
        extra_headers: Vec<(String, String)>,
        body: Bytes,
    ) -> StorageResult<http::Response> {
        validate_key(key)?;
        let (host, path) = self.target(&self.endpoint, key);
        let payload_hash = sha256_hex(&body);
        let qs = sigv4::canonical_query(&query);
        let url = if qs.is_empty() {
            format!("{}://{host}{path}", self.endpoint.scheme)
        } else {
            format!("{}://{host}{path}?{qs}", self.endpoint.scheme)
        };
        let mut last_err = String::new();
        for attempt in 0..4u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(200 * 2u64.pow(attempt))).await;
            }
            let (_, stamp) = sigv4::amz_dates(now_secs());
            let mut headers = extra_headers.clone();
            headers.push(s("host", &host));
            headers.push(s("x-amz-date", &stamp));
            headers.push(s("x-amz-content-sha256", &payload_hash));
            let req = Request {
                method,
                path: &path,
                query: query.clone(),
                headers: headers.clone(),
            };
            let auth = self.signer.authorization(&req, &payload_hash, &stamp);
            headers.push(s("authorization", &auth));
            match self.http.send(method, &url, &headers, body.clone()).await {
                Ok(r) if r.status >= 500 => {
                    last_err = format!("{op}: HTTP {}{}", r.status, error_code(&r));
                }
                Ok(r) => return Ok(r),
                Err(e) => last_err = format!("{op}: {e}"),
            }
        }
        Err(StorageError::Backend(last_err))
    }

    async fn create_upload(&self, key: &str) -> StorageResult<String> {
        let r = self
            .call(
                "create_multipart",
                "POST",
                key,
                vec![s("uploads", "")],
                vec![],
                Bytes::new(),
            )
            .await?;
        ok_status("create_multipart", &r)?;
        let body = String::from_utf8_lossy(&r.body);
        xml_tag(&body, "UploadId")
            .map(str::to_string)
            .ok_or_else(|| StorageError::Backend("create_multipart: no UploadId".into()))
    }

    async fn complete_upload(
        &self,
        key: &str,
        upload_id: &str,
        etags: &[String],
    ) -> StorageResult<()> {
        let parts: String = etags
            .iter()
            .zip(1..)
            .map(|(e, n)| {
                format!(
                    "<Part><PartNumber>{n}</PartNumber><ETag>{}</ETag></Part>",
                    xml_escape(e)
                )
            })
            .collect();
        let xml = format!("<CompleteMultipartUpload>{parts}</CompleteMultipartUpload>");
        let r = self
            .call(
                "complete_multipart",
                "POST",
                key,
                vec![s("uploadId", upload_id)],
                vec![s("content-type", "application/xml")],
                Bytes::from(xml),
            )
            .await?;
        ok_status("complete_multipart", &r)?;
        // S3 may answer 200 with an <Error> body.
        if String::from_utf8_lossy(&r.body).contains("<Error>") {
            return Err(StorageError::Backend(format!(
                "complete_multipart: {}",
                error_code(&r)
            )));
        }
        Ok(())
    }

    fn copy_source(&self, src: &str) -> String {
        format!(
            "/{}/{}",
            uri_encode(&self.bucket, false),
            uri_encode(src, true)
        )
    }

    async fn copy_multipart(&self, src: &str, dst: &str, size: u64) -> StorageResult<()> {
        let upload_id = self.create_upload(dst).await?;
        let res: StorageResult<()> = async {
            let mut etags = Vec::new();
            let mut start = 0u64;
            let mut n = 1u32;
            while start < size {
                let end = (start + COPY_PART).min(size) - 1;
                let r = self
                    .call(
                        "copy",
                        "PUT",
                        dst,
                        vec![s("partNumber", &n.to_string()), s("uploadId", &upload_id)],
                        vec![
                            s("x-amz-copy-source", &self.copy_source(src)),
                            s("x-amz-copy-source-range", &format!("bytes={start}-{end}")),
                        ],
                        Bytes::new(),
                    )
                    .await?;
                ok_status("copy", &r)?;
                let body = String::from_utf8_lossy(&r.body);
                let etag = xml_tag(&body, "ETag")
                    .ok_or_else(|| StorageError::Backend("copy: no part ETag".into()))?
                    .replace("&quot;", "\"");
                etags.push(etag);
                start = end + 1;
                n += 1;
            }
            self.complete_upload(dst, &upload_id, &etags).await
        }
        .await;
        if res.is_err() {
            let _ = self.abort_multipart(dst, &upload_id).await;
        }
        res
    }
}

fn error_code(r: &http::Response) -> String {
    let body = String::from_utf8_lossy(&r.body);
    xml_tag(&body, "Code")
        .map(|c| format!(" {c}"))
        .unwrap_or_default()
}

fn ok_status(op: &str, r: &http::Response) -> StorageResult<()> {
    match r.status {
        200..=299 => Ok(()),
        404 => Err(StorageError::NotFound),
        st => Err(StorageError::Backend(format!(
            "{op}: HTTP {st}{}",
            error_code(r)
        ))),
    }
}

#[async_trait::async_trait]
impl ObjectStore for S3Store {
    fn name(&self) -> &'static str {
        "s3"
    }

    async fn presign_put(
        &self,
        key: &str,
        ttl: Duration,
        content_length: Option<i64>,
        content_md5: Option<&str>,
    ) -> StorageResult<String> {
        let mut headers = Vec::new();
        if let Some(l) = content_length {
            headers.push(s("content-length", &l.to_string()));
        }
        if let Some(m) = content_md5 {
            headers.push(s("content-md5", m));
        }
        self.presign("PUT", key, vec![], headers, ttl)
    }

    async fn presign_get(&self, key: &str, ttl: Duration) -> StorageResult<String> {
        self.presign("GET", key, vec![], vec![], ttl)
    }

    async fn create_multipart_with(
        &self,
        key: &str,
        parts: &[PartSpec],
        ttl: Duration,
    ) -> StorageResult<MultipartUrls> {
        validate_key(key)?;
        if parts.is_empty() || parts.len() > 10_000 {
            return Err(StorageError::Backend("part count must be 1..=10000".into()));
        }
        let upload_id = self.create_upload(key).await?;
        let mut part_urls = Vec::with_capacity(parts.len());
        for (spec, n) in parts.iter().zip(1u32..) {
            let mut headers = Vec::new();
            if let Some(l) = spec.len {
                headers.push(s("content-length", &l.to_string()));
            }
            if let Some(m) = &spec.md5 {
                headers.push(s("content-md5", m));
            }
            part_urls.push(self.presign(
                "PUT",
                key,
                vec![s("partNumber", &n.to_string()), s("uploadId", &upload_id)],
                headers,
                ttl,
            )?);
        }
        let complete_url =
            self.presign("POST", key, vec![s("uploadId", &upload_id)], vec![], ttl)?;
        Ok(MultipartUrls {
            object_key: key.to_string(),
            part_urls,
            complete_url,
            upload_id,
        })
    }

    async fn abort_multipart(&self, key: &str, upload_id: &str) -> StorageResult<()> {
        let r = self
            .call(
                "abort_multipart",
                "DELETE",
                key,
                vec![s("uploadId", upload_id)],
                vec![],
                Bytes::new(),
            )
            .await?;
        match ok_status("abort_multipart", &r) {
            Ok(()) | Err(StorageError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn head(&self, key: &str) -> StorageResult<Option<u64>> {
        let r = self
            .call("head", "HEAD", key, vec![], vec![], Bytes::new())
            .await?;
        match ok_status("head", &r) {
            Ok(()) => Ok(r
                .headers
                .get("content-length")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse().ok())),
            Err(StorageError::NotFound) => Ok(None),
            Err(e) => Err(e),
        }
    }

    async fn delete(&self, key: &str) -> StorageResult<()> {
        let r = self
            .call("delete", "DELETE", key, vec![], vec![], Bytes::new())
            .await?;
        match ok_status("delete", &r) {
            Ok(()) | Err(StorageError::NotFound) => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn copy(&self, src: &str, dst: &str) -> StorageResult<()> {
        validate_key(src)?;
        let size = self.head(src).await?.ok_or(StorageError::NotFound)?;
        if size > MAX_SINGLE_COPY {
            return self.copy_multipart(src, dst, size).await;
        }
        let r = self
            .call(
                "copy",
                "PUT",
                dst,
                vec![],
                vec![s("x-amz-copy-source", &self.copy_source(src))],
                Bytes::new(),
            )
            .await?;
        ok_status("copy", &r)?;
        // CopyObject can fail after sending 200: the error is in the body.
        if String::from_utf8_lossy(&r.body).contains("<Error>") {
            return Err(StorageError::Backend(format!("copy:{}", error_code(&r))));
        }
        Ok(())
    }

    async fn put_bytes(&self, key: &str, data: Bytes) -> StorageResult<()> {
        let r = self.call("put", "PUT", key, vec![], vec![], data).await?;
        ok_status("put", &r)
    }

    async fn get_bytes(&self, key: &str) -> StorageResult<Bytes> {
        let r = self
            .call("get", "GET", key, vec![], vec![], Bytes::new())
            .await?;
        ok_status("get", &r)?;
        Ok(r.body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(path_style: bool, public: Option<&str>) -> S3Store {
        S3Store::new(&S3Config {
            endpoint: "http://garage:3900".into(),
            public_endpoint: public.map(str::to_string),
            region: "garage".into(),
            bucket: "photos".into(),
            access_key: "GKtest".into(),
            secret_key: "secret".into(),
            secret_key_file: None,
            path_style,
        })
        .expect("store")
    }

    #[tokio::test]
    async fn presigned_put_binds_length_and_md5() {
        let st = store(true, Some("https://s3.example.com"));
        let u = st
            .presign_put(
                "1/a b",
                Duration::from_secs(3600),
                Some(5),
                Some("XUFAKrxLKna5cZ2REBfFkg=="),
            )
            .await
            .expect("presign");
        assert!(
            u.starts_with("https://s3.example.com/photos/1/a%20b?"),
            "{u}"
        );
        assert!(u.contains("X-Amz-SignedHeaders=content-length%3Bcontent-md5%3Bhost"));
        assert!(u.contains("X-Amz-Expires=3600"));
        assert!(u.contains("%2Fgarage%2Fs3%2Faws4_request"));
    }

    #[tokio::test]
    async fn virtual_host_and_ttl_clamp() {
        let st = store(false, None);
        let u = st
            .presign_get("1/x", Duration::from_secs(30 * 24 * 3600))
            .await
            .expect("presign");
        assert!(u.starts_with("http://photos.garage:3900/1/x?"), "{u}");
        assert!(u.contains("X-Amz-Expires=604800"));
        assert!(
            st.presign_get("../x", Duration::from_secs(1))
                .await
                .is_err()
        );
    }
}
