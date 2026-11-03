//! Encrypting and uploading objects (file contents and thumbnails) the way
//! `web/packages/gallery/services/upload/upload-service.ts` does:
//!
//! * File contents are a secretstream in 4 MiB plaintext chunks, read from
//!   disk as they are encrypted.
//! * Files of fewer than 5 chunks are sent with one `PUT`. Larger files are
//!   sent as a multipart upload whose parts are 5 encrypted chunks each
//!   (`multipartChunksPerPart`); only one part is in memory at a time.
//! * URLs come from the checksum-protected endpoints (`POST
//!   /files/upload-url` with the ciphertext MD5, `POST
//!   /files/multipart-upload-url` with `Content-MD5` on every part, the
//!   web app's "deferred checksum" mode). Ente's hosted server answers the
//!   older `GET /files/upload-urls` with 410 for most accounts, but servers
//!   that predate the new endpoints only have the old ones, so the importer
//!   falls back to them on 404/405.

use super::api::{Api, status_of};
use super::model::*;
use crate::crypto::{self, ENCRYPTION_CHUNK_SIZE, STREAM_ABYTES, StreamEncryptor};
use anyhow::{Context as _, bail};
use base64::Engine as _;
use bytes::Bytes;
use md5::{Digest as _, Md5};
use reqwest::Method;
use std::path::Path;
use tokio::io::AsyncReadExt as _;

/// Encrypted chunks per multipart part (`multipartChunksPerPart`).
pub const MULTIPART_CHUNKS_PER_PART: u64 = 5;

/// Which family of upload-URL endpoints the server speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlMode {
    /// `POST /files/upload-url` and `POST /files/multipart-upload-url`.
    Checksummed,
    /// `GET /files/upload-urls` and `GET /files/multipart-upload-urls`.
    Legacy,
}

/// An uploaded object, ready for `POST /files`.
#[derive(Debug, Clone)]
pub struct UploadedObject {
    /// Object key.
    pub object_key: String,
    /// Base64 secretstream header.
    pub decryption_header: String,
    /// Ciphertext size.
    pub size: u64,
}

impl UploadedObject {
    /// As the `file`/`thumbnail` attributes of `POST /files`.
    pub fn attributes(&self) -> FileAttributes {
        FileAttributes {
            object_key: Some(self.object_key.clone()),
            encrypted_data: None,
            decryption_header: self.decryption_header.clone(),
            size: Some(self.size),
        }
    }
}

/// Base64 MD5, as sent in `Content-MD5`.
pub fn md5_b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(Md5::digest(data))
}

/// Number of secretstream chunks for a plaintext of `size` bytes.
pub fn chunk_count(size: u64) -> u64 {
    size.div_ceil(ENCRYPTION_CHUNK_SIZE as u64)
}

/// Ciphertext size for a plaintext of `size` bytes.
pub fn encrypted_size(size: u64) -> u64 {
    size + chunk_count(size) * STREAM_ABYTES as u64
}

/// Reads a file from disk and yields its secretstream chunks in order.
pub struct EncryptingReader {
    file: tokio::fs::File,
    enc: StreamEncryptor,
    header: Vec<u8>,
    chunks: u64,
    done: u64,
    size: u64,
    buf: Vec<u8>,
}

impl EncryptingReader {
    /// Open `path` for encryption with `key`.
    pub async fn open(path: &Path, key: &[u8]) -> anyhow::Result<Self> {
        let file = tokio::fs::File::open(path)
            .await
            .with_context(|| format!("opening {}", path.display()))?;
        let size = file.metadata().await?.len();
        if size == 0 {
            bail!("{} is empty", path.display());
        }
        let (enc, header) = StreamEncryptor::new(key)?;
        Ok(Self {
            file,
            enc,
            header,
            chunks: chunk_count(size),
            done: 0,
            size,
            buf: vec![0u8; ENCRYPTION_CHUNK_SIZE],
        })
    }

    /// The secretstream header.
    pub fn header(&self) -> &[u8] {
        &self.header
    }

    /// Number of chunks.
    pub fn chunk_count(&self) -> u64 {
        self.chunks
    }

    /// Total ciphertext size.
    pub fn encrypted_size(&self) -> u64 {
        encrypted_size(self.size)
    }

    /// The next encrypted chunk, or `None` at the end.
    pub async fn next_chunk(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        if self.done == self.chunks {
            let mut probe = [0u8; 1];
            if self.file.read(&mut probe).await? != 0 {
                bail!("file grew while it was being encrypted");
            }
            return Ok(None);
        }
        let want = if self.done + 1 == self.chunks {
            (self.size - self.done * ENCRYPTION_CHUNK_SIZE as u64) as usize
        } else {
            ENCRYPTION_CHUNK_SIZE
        };
        let mut filled = 0;
        while filled < want {
            let n = self.file.read(&mut self.buf[filled..want]).await?;
            if n == 0 {
                bail!("file shrank while it was being encrypted");
            }
            filled += n;
        }
        self.done += 1;
        let last = self.done == self.chunks;
        Ok(Some(self.enc.push(&self.buf[..want], last)?))
    }

    /// Up to `n` chunks concatenated (empty at the end).
    pub async fn next_chunks(&mut self, n: u64) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::new();
        for _ in 0..n {
            match self.next_chunk().await? {
                Some(c) => out.extend_from_slice(&c),
                None => break,
            }
        }
        Ok(out)
    }
}

/// Uploads objects, remembering which URL endpoints the server has.
#[derive(Debug, Default)]
pub struct ObjectUploader {
    mode: Option<UrlMode>,
}

impl ObjectUploader {
    /// A new uploader; the URL mode is probed on first use.
    pub fn new() -> Self {
        Self::default()
    }

    /// Encrypt the file at `path` with `key` and upload it.
    pub async fn upload_file(
        &mut self,
        api: &Api,
        path: &Path,
        key: &[u8],
    ) -> anyhow::Result<UploadedObject> {
        let mut reader = EncryptingReader::open(path, key).await?;
        let header = crypto::b64(reader.header());
        let size = reader.encrypted_size();
        let object_key = if reader.chunk_count() >= MULTIPART_CHUNKS_PER_PART {
            self.put_multipart(api, &mut reader).await?
        } else {
            let data = reader.next_chunks(MULTIPART_CHUNKS_PER_PART).await?;
            if reader.next_chunk().await?.is_some() {
                bail!("unexpected extra chunk");
            }
            self.put_single(api, Bytes::from(data)).await?
        };
        Ok(UploadedObject {
            object_key,
            decryption_header: header,
            size,
        })
    }

    /// Upload an already encrypted blob (a thumbnail) with one `PUT`.
    pub async fn upload_blob(
        &mut self,
        api: &Api,
        ciphertext: Vec<u8>,
        header: &[u8],
    ) -> anyhow::Result<UploadedObject> {
        let size = ciphertext.len() as u64;
        let object_key = self.put_single(api, Bytes::from(ciphertext)).await?;
        Ok(UploadedObject {
            object_key,
            decryption_header: crypto::b64(header),
            size,
        })
    }

    async fn put_single(&mut self, api: &Api, data: Bytes) -> anyhow::Result<String> {
        let md5 = md5_b64(&data);
        let url = self.single_url(api, data.len() as u64, &md5).await?;
        api.send(&Method::PUT, "object upload", || {
            api.storage_request(Method::PUT, &url.url)
                .header("Content-MD5", &md5)
                .header(reqwest::header::CONTENT_LENGTH, data.len())
                .body(data.clone())
        })
        .await
        .context("uploading object")?;
        Ok(url.object_key)
    }

    async fn single_url(
        &mut self,
        api: &Api,
        content_length: u64,
        md5: &str,
    ) -> anyhow::Result<UploadUrl> {
        if self.mode != Some(UrlMode::Legacy) {
            match api
                .json::<_, UploadUrl>(
                    Method::POST,
                    "/files/upload-url",
                    &UploadUrlRequest {
                        content_length,
                        content_md5: md5.to_string(),
                    },
                )
                .await
            {
                Ok(u) => {
                    self.mode = Some(UrlMode::Checksummed);
                    return Ok(u);
                }
                Err(e) if self.mode.is_none() && matches!(status_of(&e), Some(404 | 405)) => {
                    tracing::info!(
                        "server has no checksummed upload URLs; using the legacy endpoints"
                    );
                    self.mode = Some(UrlMode::Legacy);
                }
                Err(e) => return Err(e.context("getting an upload URL")),
            }
        }
        let res: UploadUrlsResponse = api
            .get_json(
                "/files/upload-urls",
                &[("count", "1".to_string()), ("ts", now_ms())],
            )
            .await
            .context("getting an upload URL")?;
        res.urls
            .into_iter()
            .next()
            .context("server returned no upload URL")
    }

    async fn multipart_urls(
        &mut self,
        api: &Api,
        content_length: u64,
        part_length: u64,
        parts: u64,
    ) -> anyhow::Result<MultipartUploadUrls> {
        if self.mode != Some(UrlMode::Legacy) {
            match api
                .json::<_, MultipartUploadUrls>(
                    Method::POST,
                    "/files/multipart-upload-url",
                    &MultipartUploadUrlRequest {
                        content_length,
                        part_length,
                    },
                )
                .await
            {
                Ok(u) => {
                    self.mode = Some(UrlMode::Checksummed);
                    return Ok(u);
                }
                Err(e) if self.mode.is_none() && matches!(status_of(&e), Some(404 | 405)) => {
                    tracing::info!(
                        "server has no checksummed upload URLs; using the legacy endpoints"
                    );
                    self.mode = Some(UrlMode::Legacy);
                }
                Err(e) => return Err(e.context("getting multipart upload URLs")),
            }
        }
        let res: MultipartUploadUrlsResponse = api
            .get_json(
                "/files/multipart-upload-urls",
                &[("count", parts.to_string()), ("ts", now_ms())],
            )
            .await
            .context("getting multipart upload URLs")?;
        Ok(res.urls)
    }

    async fn put_multipart(
        &mut self,
        api: &Api,
        reader: &mut EncryptingReader,
    ) -> anyhow::Result<String> {
        let total = reader.encrypted_size();
        let parts = reader.chunk_count().div_ceil(MULTIPART_CHUNKS_PER_PART);
        let part_length =
            total.min(MULTIPART_CHUNKS_PER_PART * (ENCRYPTION_CHUNK_SIZE + STREAM_ABYTES) as u64);
        let urls = self.multipart_urls(api, total, part_length, parts).await?;
        if urls.part_urls.len() as u64 != parts {
            bail!(
                "server returned {} part URLs for {parts} parts",
                urls.part_urls.len()
            );
        }
        let mut etags = Vec::with_capacity(urls.part_urls.len());
        for (i, part_url) in urls.part_urls.iter().enumerate() {
            let data = Bytes::from(reader.next_chunks(MULTIPART_CHUNKS_PER_PART).await?);
            if data.is_empty() {
                bail!("ran out of data at part {}", i + 1);
            }
            let md5 = md5_b64(&data);
            let res = api
                .send(&Method::PUT, "multipart part upload", || {
                    api.storage_request(Method::PUT, part_url)
                        .header("Content-MD5", &md5)
                        .header(reqwest::header::CONTENT_LENGTH, data.len())
                        .body(data.clone())
                })
                .await
                .with_context(|| format!("uploading part {}", i + 1))?;
            let etag = res
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .context("ETag header not present in part upload response")?;
            etags.push(etag);
        }
        if reader.next_chunk().await?.is_some() {
            bail!("more chunks than expected");
        }
        let body = Bytes::from(complete_multipart_body(&etags));
        api.send(&Method::POST, "multipart complete", || {
            api.storage_request(Method::POST, &urls.complete_url)
                .header(reqwest::header::CONTENT_TYPE, "text/xml")
                .body(body.clone())
        })
        .await
        .context("completing multipart upload")?;
        Ok(urls.object_key)
    }
}

/// The S3 `CompleteMultipartUpload` XML the web app sends.
pub fn complete_multipart_body(etags: &[String]) -> String {
    let parts: Vec<String> = etags
        .iter()
        .enumerate()
        .map(|(i, e)| {
            format!(
                "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
                i + 1,
                e
            )
        })
        .collect();
    format!(
        "<CompleteMultipartUpload>\n{}\n</CompleteMultipartUpload>",
        parts.join("\n")
    )
}

fn now_ms() -> String {
    chrono::Utc::now().timestamp_millis().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        let c = ENCRYPTION_CHUNK_SIZE as u64;
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(c), 1);
        assert_eq!(chunk_count(c + 1), 2);
        assert_eq!(encrypted_size(c + 1), c + 1 + 34);
    }

    #[test]
    fn md5_matches_rfc1321() {
        assert_eq!(md5_b64(b""), "1B2M2Y8AsgTpgAmY7PhCfg==");
    }

    #[test]
    fn completion_xml() {
        assert_eq!(
            complete_multipart_body(&["\"a\"".into(), "\"b\"".into()]),
            "<CompleteMultipartUpload>\n<Part><PartNumber>1</PartNumber><ETag>\"a\"</ETag></Part>\n<Part><PartNumber>2</PartNumber><ETag>\"b\"</ETag></Part>\n</CompleteMultipartUpload>"
        );
    }
}
