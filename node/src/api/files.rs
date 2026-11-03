//! Library and file-data endpoints: collections, diffs, downloads, ML data,
//! video previews and presigned uploads.
//!
//! All payloads are ciphertext; see [`crate::library`] for decryption.

use super::models::{
    CollectionDiff, CollectionsResponse, FileDataStatus, FilesDataResponse, PreviewUploadUrl,
    PutFileDataRequest, PutVideoDataRequest, RemoteCollection, RemoteFileData, StatusDiffResponse,
};
use super::{ApiClient, ApiError, Result};
use crate::crypto::{self, EncryptedBlob};
use reqwest::{Method, Response, StatusCode};
use serde::Serialize;

/// Kinds of derived file data the server stores (museum `ObjectType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileDataType {
    /// Face and CLIP indexes (`mldata`): gzipped JSON in a blob.
    MlData,
    /// Streamable video (`vid_preview`): encrypted HLS segments + playlist.
    VideoPreview,
    /// Image preview (`img_preview`).
    ImagePreview,
}

impl FileDataType {
    /// The wire name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MlData => "mldata",
            Self::VideoPreview => "vid_preview",
            Self::ImagePreview => "img_preview",
        }
    }
}

impl std::fmt::Display for FileDataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// An in-progress download of an encrypted object.
#[derive(Debug)]
pub struct Download {
    resp: Response,
}

impl Download {
    /// `Content-Length`, if the server sent it.
    pub fn content_length(&self) -> Option<u64> {
        self.resp.content_length()
    }

    /// The next piece of the body, or `None` at the end.
    pub async fn chunk(&mut self) -> Result<Option<bytes::Bytes>> {
        self.resp.chunk().await.map_err(ApiError::Network)
    }

    /// The whole body.
    pub async fn bytes(self) -> Result<Vec<u8>> {
        Ok(self.resp.bytes().await.map_err(ApiError::Network)?.to_vec())
    }
}

/// Museum's maximum `fileIDs` per `POST /files/data/fetch`.
pub const MAX_FETCH_FILE_IDS: usize = 200;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FetchFilesData<'a> {
    #[serde(rename = "fileIDs")]
    file_ids: &'a [i64],
    #[serde(rename = "type")]
    kind: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StatusDiffRequest {
    last_updated_at: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreviewUploadV2<'a> {
    #[serde(rename = "fileID")]
    file_id: i64,
    #[serde(rename = "type")]
    kind: &'a str,
    content_length: i64,
    #[serde(rename = "contentMD5")]
    content_md5: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MultipartPreviewUpload<'a> {
    #[serde(rename = "fileID")]
    file_id: i64,
    #[serde(rename = "type")]
    kind: &'a str,
    content_length: i64,
    part_length: i64,
    #[serde(rename = "partMd5s")]
    part_md5s: &'a [String],
}

#[derive(serde::Deserialize)]
struct FileDataEnvelope {
    data: RemoteFileData,
}

impl ApiClient {
    /// `GET /collections/v2?sinceTime=…`: collections changed since `since_time`
    /// (0 for all), including deleted ones (`isDeleted`).
    pub async fn get_collections(&self, since_time: i64) -> Result<Vec<RemoteCollection>> {
        let r: CollectionsResponse = self
            .get_json("/collections/v2", &[("sinceTime", since_time)])
            .await?;
        Ok(r.collections)
    }

    /// `GET /collections/v2/diff?collectionID=…&sinceTime=…`: one page of file
    /// changes in a collection. Entries with `isDeleted` were removed (or trashed).
    pub async fn get_collection_diff(
        &self,
        collection_id: i64,
        since_time: i64,
    ) -> Result<CollectionDiff> {
        self.get_json(
            "/collections/v2/diff",
            &[("collectionID", collection_id), ("sinceTime", since_time)],
        )
        .await
    }

    /// Start downloading the encrypted original (`GET /files/download/{id}`).
    pub async fn download_file(&self, file_id: i64) -> Result<Download> {
        self.download(&format!("/files/download/{file_id}")).await
    }

    /// Start downloading the encrypted thumbnail (`GET /files/preview/{id}`).
    pub async fn download_thumbnail(&self, file_id: i64) -> Result<Download> {
        self.download(&format!("/files/preview/{file_id}")).await
    }

    /// GET an object endpoint. Museum answers `307` to a presigned storage URL,
    /// which is fetched without the auth token; servers may also stream directly.
    async fn download(&self, path: &str) -> Result<Download> {
        let resp = self
            .send(path, true, || self.request_untimed(Method::GET, path))
            .await?;
        if !resp.status().is_redirection() {
            return Ok(Download { resp });
        }
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| ApiError::Protocol(format!("redirect without Location from {path}")))?;
        let target = resp
            .url()
            .join(location)
            .map_err(|e| ApiError::Protocol(format!("bad redirect from {path}: {e}")))?;
        let resp = self
            .send(path, true, || Ok(self.storage.get(target.clone())))
            .await?;
        Ok(Download { resp })
    }

    /// `POST /files/data/fetch`: data of `kind` for up to 200 files.
    pub async fn fetch_files_data(
        &self,
        kind: FileDataType,
        file_ids: &[i64],
    ) -> Result<FilesDataResponse> {
        if file_ids.len() > MAX_FETCH_FILE_IDS {
            return Err(ApiError::InvalidInput(format!(
                "at most {MAX_FETCH_FILE_IDS} file ids per fetch"
            )));
        }
        if file_ids.is_empty() {
            return Ok(FilesDataResponse::default());
        }
        self.send_json(
            Method::POST,
            "/files/data/fetch",
            &FetchFilesData {
                file_ids,
                kind: kind.as_str(),
            },
            true,
        )
        .await
    }

    /// `GET /files/data/fetch?fileID=…&type=…`: one file's data, `None` if absent.
    pub async fn fetch_file_data(
        &self,
        kind: FileDataType,
        file_id: i64,
    ) -> Result<Option<RemoteFileData>> {
        let path = "/files/data/fetch";
        let query = [
            ("fileID", file_id.to_string()),
            ("type", kind.as_str().to_owned()),
            ("preferNoContent", "true".to_owned()),
        ];
        let resp = match self
            .send(path, true, || {
                Ok(self.request(Method::GET, path)?.query(&query))
            })
            .await
        {
            Ok(r) => r,
            Err(ApiError::Http { status: 404, .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        if resp.status() == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let env: FileDataEnvelope = super::parse_json(path, resp).await?;
        Ok(Some(env.data))
    }

    /// `PUT /files/data`: store an encrypted blob (e.g. `mldata`) for a file.
    ///
    /// `last_updated_at` is the `updatedAt` of the remote record this replaces
    /// (0 if there was none), so the server rejects writes over newer data.
    pub async fn put_file_data(
        &self,
        file_id: i64,
        kind: FileDataType,
        blob: &EncryptedBlob,
        last_updated_at: Option<i64>,
        version: Option<i32>,
    ) -> Result<()> {
        let body = PutFileDataRequest {
            file_id,
            kind: kind.as_str().to_owned(),
            encrypted_data: crypto::b64_encode(&blob.encrypted_data),
            decryption_header: crypto::b64_encode(&blob.decryption_header),
            version,
            last_updated_at,
        };
        self.send_json_no_content(Method::PUT, "/files/data", &body, true)
            .await
    }

    /// `POST /files/data/status-diff`: one page of file-data changes after
    /// `last_updated_at`. Call again with the largest `updatedAt` until empty.
    pub async fn file_data_status_diff(&self, last_updated_at: i64) -> Result<Vec<FileDataStatus>> {
        let r: StatusDiffResponse = self
            .send_json(
                Method::POST,
                "/files/data/status-diff",
                &StatusDiffRequest { last_updated_at },
                true,
            )
            .await?;
        Ok(r.diff.unwrap_or_default())
    }

    /// `GET /files/data/preview-upload-url`: where to upload a preview object.
    /// `parts` > 1 asks for a multipart upload with that many parts.
    pub async fn preview_upload_url(
        &self,
        file_id: i64,
        kind: FileDataType,
        parts: Option<u32>,
    ) -> Result<PreviewUploadUrl> {
        let mut query = vec![
            ("fileID", file_id.to_string()),
            ("type", kind.as_str().to_owned()),
        ];
        if let Some(n) = parts.filter(|n| *n > 1) {
            query.push(("isMultiPart", "true".to_owned()));
            query.push(("count", n.to_string()));
        }
        self.get_json("/files/data/preview-upload-url", &query)
            .await
    }

    /// `POST /files/data/preview-upload-url`: single-part upload URL bound to
    /// the object's length and base64 MD5 (see [`content_md5`]).
    pub async fn preview_upload_url_v2(
        &self,
        file_id: i64,
        kind: FileDataType,
        content_length: i64,
        content_md5: &str,
    ) -> Result<PreviewUploadUrl> {
        self.send_json(
            Method::POST,
            "/files/data/preview-upload-url",
            &PreviewUploadV2 {
                file_id,
                kind: kind.as_str(),
                content_length,
                content_md5,
            },
            true,
        )
        .await
    }

    /// `POST /files/data/multipart-preview-upload-url`: part URLs bound to
    /// each part's base64 MD5 (parts are `part_length` bytes, the last shorter).
    pub async fn multipart_preview_upload_url(
        &self,
        file_id: i64,
        kind: FileDataType,
        content_length: i64,
        part_length: i64,
        part_md5s: &[String],
    ) -> Result<PreviewUploadUrl> {
        self.send_json(
            Method::POST,
            "/files/data/multipart-preview-upload-url",
            &MultipartPreviewUpload {
                file_id,
                kind: kind.as_str(),
                content_length,
                part_length,
                part_md5s,
            },
            true,
        )
        .await
    }

    /// `PUT /files/video-data`: register an uploaded streamable video
    /// (segments object `object_id` of `object_size` bytes, and its encrypted playlist).
    pub async fn put_video_data(
        &self,
        file_id: i64,
        object_id: &str,
        object_size: i64,
        playlist: &EncryptedBlob,
        version: Option<i32>,
    ) -> Result<()> {
        let body = PutVideoDataRequest {
            file_id,
            object_id: object_id.to_owned(),
            object_size,
            playlist: crypto::b64_encode(&playlist.encrypted_data),
            playlist_header: crypto::b64_encode(&playlist.decryption_header),
            version,
        };
        self.send_json_no_content(Method::PUT, "/files/video-data", &body, true)
            .await
    }

    /// PUT `body` to a presigned storage URL (no auth headers). Returns the
    /// `ETag` if the storage sent one (needed for multipart completion).
    pub async fn put_presigned(
        &self,
        url: &str,
        body: bytes::Bytes,
        content_md5: Option<&str>,
    ) -> Result<Option<String>> {
        let target =
            url::Url::parse(url).map_err(|e| ApiError::InvalidInput(format!("upload URL: {e}")))?;
        let path = target.path().to_owned();
        let resp = self
            .send(&path, true, || {
                let mut rb = self.storage.put(target.clone()).body(body.clone());
                if let Some(md5) = content_md5 {
                    rb = rb.header("Content-MD5", md5);
                }
                Ok(rb)
            })
            .await?;
        Ok(resp
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned))
    }

    /// Complete an S3-style multipart upload: POST the
    /// `<CompleteMultipartUpload>` XML with each part's ETag, in order.
    pub async fn complete_multipart(&self, complete_url: &str, etags: &[String]) -> Result<()> {
        let target = url::Url::parse(complete_url)
            .map_err(|e| ApiError::InvalidInput(format!("completion URL: {e}")))?;
        let mut xml = String::from("<CompleteMultipartUpload>");
        for (i, etag) in etags.iter().enumerate() {
            xml.push_str(&format!(
                "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
                i + 1,
                xml_escape(etag)
            ));
        }
        xml.push_str("</CompleteMultipartUpload>");
        let path = target.path().to_owned();
        self.send(&path, true, || {
            Ok(self
                .storage
                .post(target.clone())
                .header(reqwest::header::CONTENT_TYPE, "text/xml")
                .body(xml.clone()))
        })
        .await?;
        Ok(())
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Base64 MD5 of `data`, as `Content-MD5` and museum's `contentMD5`/`partMd5s` expect.
pub fn content_md5(data: &[u8]) -> String {
    use md5::{Digest, Md5};
    crypto::b64_encode(&Md5::digest(data))
}
