//! Uploads: encrypting and sending objects the way Ente's apps do, and the
//! resumable upload queue.
//!
//! Objects (adapted from `importers/src/ente/upload.rs`, branch
//! `claude/project-thread-cp9x81`, which follows
//! `web/packages/gallery/services/upload/upload-service.ts`):
//!
//! * File contents are a secretstream in 4 MiB plaintext chunks, read from
//!   disk as they are encrypted, under a fresh per-file key.
//! * Files of fewer than 5 chunks go up with one `PUT`; larger ones as a
//!   multipart upload of 5 encrypted chunks per part.
//! * URLs come from `POST /files/upload-url` / `POST /files/multipart-upload-url`
//!   (checksummed), falling back to `GET /files/upload-urls` /
//!   `GET /files/multipart-upload-urls` on servers without them.
//! * The thumbnail is one secretstream message under the file key, the
//!   metadata JSON too; the file key is secretbox'd with the collection key
//!   for `POST /files`.
//!
//! The queue lives in the store (`uploads`): each row's sealed state records
//! the file key and every object already uploaded, so after process death a
//! run continues where it stopped instead of starting over.

use crate::api::Api;
use crate::client::Inner;
use crate::crypto::{self, ENCRYPTION_CHUNK_SIZE, STREAM_ABYTES, StreamEncryptor};
use crate::library::{CollectionRec, decrypt_file};
use crate::media::{self, MediaKind};
use crate::metadata::{
    FILE_TYPE_IMAGE, FILE_TYPE_VIDEO, FileMetadata, encrypt_json, encrypt_magic, public_magic_data,
};
use crate::model::*;
use crate::store::{UP_DONE, UP_FAILED, UP_PENDING, UP_UPLOADING, now_us};
use crate::{CoreError, Result, UploadStatus};
use base64::Engine as _;
use bytes::Bytes;
use md5::{Digest as _, Md5};
use reqwest::Method;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::Path;
use tokio::io::AsyncReadExt as _;

/// Encrypted chunks per multipart part (`multipartChunksPerPart`).
pub const MULTIPART_CHUNKS_PER_PART: u64 = 5;
/// Attempts before a queued file stays failed.
const MAX_ATTEMPTS: i64 = 5;
/// Name the apps give the uncategorized collection.
const UNCATEGORIZED_NAME: &str = "Uncategorized";

/// Which family of upload-URL endpoints the server speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UrlMode {
    Checksummed,
    Legacy,
}

/// An uploaded object, ready for `POST /files`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadedObject {
    /// Object key.
    pub object_key: String,
    /// Base64 secretstream header.
    pub decryption_header: String,
    /// Ciphertext size.
    pub size: u64,
}

impl UploadedObject {
    fn attributes(&self) -> FileAttributes {
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
    size.div_ceil(ENCRYPTION_CHUNK_SIZE as u64).max(1)
}

/// Ciphertext size for a plaintext of `size` bytes.
pub fn encrypted_size(size: u64) -> u64 {
    size + chunk_count(size) * STREAM_ABYTES as u64
}

/// Reads a file from disk and yields its secretstream chunks in order.
struct EncryptingReader {
    file: tokio::fs::File,
    enc: StreamEncryptor,
    header: Vec<u8>,
    chunks: u64,
    done: u64,
    size: u64,
    buf: Vec<u8>,
}

impl EncryptingReader {
    async fn open(path: &Path, key: &[u8]) -> Result<Self> {
        let file = tokio::fs::File::open(path).await?;
        let size = file.metadata().await?.len();
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

    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>> {
        if self.done == self.chunks {
            let mut probe = [0u8; 1];
            if self.file.read(&mut probe).await? != 0 {
                return Err(CoreError::storage("file grew while it was being uploaded"));
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
                return Err(CoreError::storage(
                    "file shrank while it was being uploaded",
                ));
            }
            filled += n;
        }
        self.done += 1;
        let last = self.done == self.chunks;
        Ok(Some(self.enc.push(&self.buf[..want], last)?))
    }

    async fn next_chunks(&mut self, n: u64) -> Result<Vec<u8>> {
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
struct ObjectUploader<'a> {
    api: &'a Api,
    mode: Option<UrlMode>,
    progress: &'a (dyn Fn(u64) + Sync),
}

impl<'a> ObjectUploader<'a> {
    async fn upload_file(&mut self, path: &Path, key: &[u8]) -> Result<UploadedObject> {
        let mut reader = EncryptingReader::open(path, key).await?;
        let header = crypto::b64(&reader.header);
        let size = encrypted_size(reader.size);
        let object_key = if reader.chunks >= MULTIPART_CHUNKS_PER_PART {
            self.put_multipart(&mut reader).await?
        } else {
            let data = reader.next_chunks(MULTIPART_CHUNKS_PER_PART).await?;
            if reader.next_chunk().await?.is_some() {
                return Err(CoreError::storage("unexpected extra chunk"));
            }
            let len = data.len() as u64;
            let k = self.put_single(Bytes::from(data)).await?;
            (self.progress)(len);
            k
        };
        Ok(UploadedObject {
            object_key,
            decryption_header: header,
            size,
        })
    }

    async fn upload_blob(&mut self, ciphertext: Vec<u8>, header: &[u8]) -> Result<UploadedObject> {
        let size = ciphertext.len() as u64;
        let object_key = self.put_single(Bytes::from(ciphertext)).await?;
        Ok(UploadedObject {
            object_key,
            decryption_header: crypto::b64(header),
            size,
        })
    }

    async fn put_single(&mut self, data: Bytes) -> Result<String> {
        let md5 = md5_b64(&data);
        let url = self.single_url(data.len() as u64, &md5).await?;
        self.api
            .send(|| {
                self.api
                    .storage_request(Method::PUT, &url.url)
                    .header("Content-MD5", &md5)
                    .header(reqwest::header::CONTENT_LENGTH, data.len())
                    .body(data.clone())
            })
            .await?;
        Ok(url.object_key)
    }

    async fn single_url(&mut self, content_length: u64, md5: &str) -> Result<UploadUrl> {
        if self.mode != Some(UrlMode::Legacy) {
            match self
                .api
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
                Err(e) if self.mode.is_none() && matches!(e.status(), Some(404 | 405)) => {
                    self.mode = Some(UrlMode::Legacy);
                }
                Err(e) => return Err(e),
            }
        }
        let res: UploadUrlsResponse = self
            .api
            .get_json("/files/upload-urls", &[("count", "1".to_string())])
            .await?;
        res.urls
            .into_iter()
            .next()
            .ok_or_else(|| CoreError::network("server returned no upload URL"))
    }

    async fn multipart_urls(
        &mut self,
        content_length: u64,
        part_length: u64,
        parts: u64,
    ) -> Result<MultipartUploadUrls> {
        if self.mode != Some(UrlMode::Legacy) {
            match self
                .api
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
                Err(e) if self.mode.is_none() && matches!(e.status(), Some(404 | 405)) => {
                    self.mode = Some(UrlMode::Legacy);
                }
                Err(e) => return Err(e),
            }
        }
        let res: MultipartUploadUrlsResponse = self
            .api
            .get_json(
                "/files/multipart-upload-urls",
                &[("count", parts.to_string())],
            )
            .await?;
        Ok(res.urls)
    }

    async fn put_multipart(&mut self, reader: &mut EncryptingReader) -> Result<String> {
        let total = encrypted_size(reader.size);
        let parts = reader.chunks.div_ceil(MULTIPART_CHUNKS_PER_PART);
        let part_length =
            total.min(MULTIPART_CHUNKS_PER_PART * (ENCRYPTION_CHUNK_SIZE + STREAM_ABYTES) as u64);
        let urls = self.multipart_urls(total, part_length, parts).await?;
        if urls.part_urls.len() as u64 != parts {
            return Err(CoreError::network(format!(
                "server returned {} part URLs for {parts} parts",
                urls.part_urls.len()
            )));
        }
        let mut etags = Vec::with_capacity(urls.part_urls.len());
        for part_url in &urls.part_urls {
            let data = Bytes::from(reader.next_chunks(MULTIPART_CHUNKS_PER_PART).await?);
            if data.is_empty() {
                return Err(CoreError::storage(
                    "file shrank while it was being uploaded",
                ));
            }
            let md5 = md5_b64(&data);
            let res = self
                .api
                .send(|| {
                    self.api
                        .storage_request(Method::PUT, part_url)
                        .header("Content-MD5", &md5)
                        .header(reqwest::header::CONTENT_LENGTH, data.len())
                        .body(data.clone())
                })
                .await?;
            let etag = res
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
                .ok_or_else(|| CoreError::network("no ETag in part upload response"))?;
            etags.push(etag);
            (self.progress)(data.len() as u64);
        }
        if reader.next_chunk().await?.is_some() {
            return Err(CoreError::storage("file grew while it was being uploaded"));
        }
        let body = Bytes::from(complete_multipart_body(&etags));
        self.api
            .send(|| {
                self.api
                    .storage_request(Method::POST, &urls.complete_url)
                    .header(reqwest::header::CONTENT_TYPE, "text/xml")
                    .body(body.clone())
            })
            .await?;
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

/// The sealed state of a queued upload.
#[derive(Debug, Default, Serialize, Deserialize)]
struct UpState {
    path: String,
    /// Per-file key (fresh random), set once the file has been prepared.
    file_key: Option<Vec<u8>>,
    metadata: Option<PreparedMeta>,
    file_obj: Option<UploadedObject>,
    thumb_obj: Option<UploadedObject>,
    file_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PreparedMeta {
    file_type: i64,
    title: String,
    creation_time: i64,
    modification_time: i64,
    hash: String,
    latitude: Option<f64>,
    longitude: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
}

impl PreparedMeta {
    fn kind(&self) -> MediaKind {
        if self.file_type == FILE_TYPE_VIDEO {
            MediaKind::Video
        } else {
            MediaKind::Image
        }
    }
}

/// Hash, type, EXIF of a local file (blocking).
fn prepare(path: &Path) -> Result<PreparedMeta> {
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(CoreError::storage(format!(
            "{} is not a file",
            path.display()
        )));
    }
    let kind = media::detect_kind(path).unwrap_or(MediaKind::Image);
    let hash = media::hash_file(path)?;
    let info = media::read_embedded_info(path);
    let mtime = meta
        .modified()
        .ok()
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_micros() as i64)
        .unwrap_or_else(now_us);
    Ok(PreparedMeta {
        file_type: match kind {
            MediaKind::Image => FILE_TYPE_IMAGE,
            MediaKind::Video => FILE_TYPE_VIDEO,
        },
        title: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into()),
        creation_time: info.creation_time_us.unwrap_or(mtime),
        modification_time: mtime,
        hash,
        latitude: info.location.map(|l| l.latitude),
        longitude: info.location.map(|l| l.longitude),
        width: info.dimensions.map(|d| d.0),
        height: info.dimensions.map(|d| d.1),
    })
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| CoreError::storage(format!("worker failed: {e}")))?
}

/// What happened to one queued file.
enum Outcome {
    Done,
    /// Stop the run (offline, signed out); the row stays queued.
    Stop(CoreError),
    Failed(CoreError),
}

impl Inner {
    pub fn enqueue_upload(&self, path: &str, collection_id: Option<i64>) -> Result<()> {
        self.session()?;
        let meta =
            std::fs::metadata(path).map_err(|e| CoreError::storage(format!("{path}: {e}")))?;
        if !meta.is_file() {
            return Err(CoreError::storage(format!("{path} is not a file")));
        }
        let st = self.store();
        let sealed = st.seal(&UpState {
            path: path.to_string(),
            ..Default::default()
        })?;
        st.conn().execute(
            "INSERT INTO uploads (state, collection_id, bytes_total, sealed, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![UP_PENDING, collection_id, meta.len() as i64, sealed, now_us()],
        )?;
        Ok(())
    }

    pub fn upload_status(&self) -> UploadStatus {
        let live = {
            let l = self.live_upload.lock().unwrap_or_else(|e| e.into_inner());
            (l.row, l.bytes_done)
        };
        let st = self.store();
        let mut s = UploadStatus::default();
        let Ok(mut q) = st
            .conn()
            .prepare("SELECT id, state, bytes_total FROM uploads")
        else {
            return s;
        };
        let Ok(rows) = q.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        }) else {
            return s;
        };
        for (id, state, total) in rows.flatten() {
            let total = total.max(0) as u64;
            s.bytes_total += total;
            match state.as_str() {
                UP_PENDING => s.pending += 1,
                UP_UPLOADING => {
                    s.uploading += 1;
                    if live.0 == Some(id) {
                        s.bytes_done += live.1.min(total);
                    }
                }
                UP_FAILED => s.failed += 1,
                _ => s.bytes_done += total,
            }
        }
        s
    }

    pub async fn run_uploads(&self) -> Result<UploadStatus> {
        let _guard = self.upload_lock.lock().await;
        self.session()?;
        let mut changed = false;
        let mut stop = None;
        let mut skip: Vec<i64> = Vec::new();
        loop {
            let next: Option<(i64, Option<i64>, Vec<u8>)> = {
                let st = self.store();
                let mut q = st.conn().prepare(
                    "SELECT id, collection_id, sealed FROM uploads \
                     WHERE state IN (?1, ?2) OR (state = ?3 AND attempts < ?4) ORDER BY id",
                )?;
                let rows: Vec<(i64, Option<i64>, Vec<u8>)> = q
                    .query_map(
                        params![UP_PENDING, UP_UPLOADING, UP_FAILED, MAX_ATTEMPTS],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                    )?
                    .collect::<rusqlite::Result<_>>()?;
                rows.into_iter().find(|r| !skip.contains(&r.0))
            };
            let Some((row, collection_id, sealed)) = next else {
                break;
            };
            skip.push(row);
            self.store().conn().execute(
                "UPDATE uploads SET state = ?1 WHERE id = ?2",
                params![UP_UPLOADING, row],
            )?;
            *self.live_upload.lock().unwrap_or_else(|e| e.into_inner()) =
                crate::client::LiveUpload {
                    row: Some(row),
                    bytes_done: 0,
                };
            let outcome = self.upload_one(row, collection_id, &sealed).await;
            *self.live_upload.lock().unwrap_or_else(|e| e.into_inner()) = Default::default();
            match outcome {
                Outcome::Done => {
                    changed = true;
                    self.store().conn().execute(
                        "UPDATE uploads SET state = ?1, error = NULL WHERE id = ?2",
                        params![UP_DONE, row],
                    )?;
                }
                Outcome::Failed(e) => {
                    self.store().conn().execute(
                        "UPDATE uploads SET state = ?1, attempts = attempts + 1, error = ?2 WHERE id = ?3",
                        params![UP_FAILED, e.to_string(), row],
                    )?;
                }
                Outcome::Stop(e) => {
                    self.store().conn().execute(
                        "UPDATE uploads SET state = ?1, error = ?2 WHERE id = ?3",
                        params![UP_PENDING, e.to_string(), row],
                    )?;
                    stop = Some(e);
                    break;
                }
            }
        }
        if changed {
            self.rebuild_index()?;
        }
        let status = self.upload_status();
        if status.pending == 0 && status.uploading == 0 {
            self.store()
                .conn()
                .execute("DELETE FROM uploads WHERE state = ?1", params![UP_DONE])?;
        }
        match stop {
            Some(e @ (CoreError::Unauthorized | CoreError::NotLoggedIn)) => Err(e),
            _ => Ok(status),
        }
    }

    fn save_up_state(&self, row: i64, s: &UpState) -> Result<()> {
        let st = self.store();
        let sealed = st.seal(s)?;
        st.conn().execute(
            "UPDATE uploads SET sealed = ?1 WHERE id = ?2",
            params![sealed, row],
        )?;
        Ok(())
    }

    async fn upload_one(&self, row: i64, collection_id: Option<i64>, sealed: &[u8]) -> Outcome {
        let classify = |e: CoreError| match e {
            CoreError::Unauthorized
            | CoreError::NotLoggedIn
            | CoreError::Unreachable { .. }
            | CoreError::Network { .. }
            | CoreError::Cancelled => Outcome::Stop(e),
            CoreError::Server { status, .. } if status == 429 || status >= 500 => Outcome::Stop(e),
            e => Outcome::Failed(e),
        };
        let state: UpState = match self.store().open_sealed(sealed) {
            Ok(s) => s,
            Err(e) => return Outcome::Failed(e),
        };
        match self.upload_steps(row, collection_id, state).await {
            Ok(()) => Outcome::Done,
            Err(e) => classify(e),
        }
    }

    async fn upload_steps(
        &self,
        row: i64,
        collection_id: Option<i64>,
        mut state: UpState,
    ) -> Result<()> {
        let api = self.api()?;
        // 1. Prepare once: metadata and the per-file key are persisted.
        if state.metadata.is_none() || state.file_key.is_none() {
            let path = state.path.clone();
            let meta = blocking(move || prepare(Path::new(&path))).await?;
            state.metadata = Some(meta);
            state.file_key = Some(crypto::generate_key());
            state.file_obj = None;
            state.thumb_obj = None;
            self.save_up_state(row, &state)?;
        }
        let meta = state
            .metadata
            .clone()
            .ok_or_else(|| CoreError::storage("upload state without metadata"))?;
        let file_key = state.file_key.clone().unwrap_or_default();

        // 2. Target collection.
        let target = match collection_id {
            Some(id) => self
                .collection_rec(id)?
                .ok_or_else(|| CoreError::storage(format!("unknown collection {id}")))?,
            None => self.uncategorized(&api).await?,
        };
        if !target.owned {
            return Err(CoreError::storage(
                "cannot upload into a collection shared with you",
            ));
        }

        // 3. Already in the library (same content hash)? Then only add it.
        let existing = self.index().hashes.get(&meta.hash).cloned();
        if let Some((file_id, cols)) = existing {
            if !cols.contains(&target.id) {
                self.add_to_collection(&api, file_id, &target).await?;
            }
            state.file_id = Some(file_id);
            self.save_up_state(row, &state)?;
            return Ok(());
        }

        // 4. Objects (skipped when already uploaded before a restart).
        let resumed = state.file_obj.is_some() || state.thumb_obj.is_some();
        let live = &self.live_upload;
        let progress = |n: u64| {
            let mut l = live.lock().unwrap_or_else(|e| e.into_inner());
            l.bytes_done += n;
        };
        let mut up = ObjectUploader {
            api: &api,
            mode: None,
            progress: &progress,
        };
        let path = Path::new(&state.path).to_path_buf();
        if state.file_obj.is_none() {
            state.file_obj = Some(up.upload_file(&path, &file_key).await?);
            self.save_up_state(row, &state)?;
        } else if let Ok(m) = std::fs::metadata(&path) {
            progress(m.len());
        }
        if state.thumb_obj.is_none() {
            let (p, kind) = (path.clone(), meta.kind());
            let thumb = blocking(move || Ok(media::make_thumbnail(&p, kind))).await?;
            let (ct, header) = crypto::encrypt_blob(&thumb, &file_key)?;
            state.thumb_obj = Some(up.upload_blob(ct, &header).await?);
            self.save_up_state(row, &state)?;
        }

        // 5. Create the file.
        match self
            .create_file(&api, &target, &meta, &file_key, &state)
            .await
        {
            Ok(id) => {
                state.file_id = Some(id);
                self.save_up_state(row, &state)?;
                Ok(())
            }
            // Objects from an earlier run may have expired: start them over once.
            Err(CoreError::Server { status, .. })
                if resumed && (400..500).contains(&status) && status != 401 && status != 429 =>
            {
                state.file_obj = None;
                state.thumb_obj = None;
                self.save_up_state(row, &state)?;
                Box::pin(self.upload_steps(row, collection_id, state)).await
            }
            Err(e) => Err(e),
        }
    }

    async fn create_file(
        &self,
        api: &Api,
        target: &CollectionRec,
        meta: &PreparedMeta,
        file_key: &[u8],
        state: &UpState,
    ) -> Result<i64> {
        let (Some(file), Some(thumb)) = (&state.file_obj, &state.thumb_obj) else {
            return Err(CoreError::storage("objects missing"));
        };
        let md = FileMetadata {
            file_type: meta.file_type,
            title: meta.title.clone(),
            creation_time: meta.creation_time,
            modification_time: meta.modification_time,
            hash: Some(meta.hash.clone()),
            duration: None,
            latitude: meta.latitude,
            longitude: meta.longitude,
        };
        let (md_data, md_header) = encrypt_json(&md, file_key)?;
        let dims = match (meta.width, meta.height) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        };
        let pub_magic = public_magic_data(dims, None);
        let pub_magic_metadata = if pub_magic.is_empty() {
            None
        } else {
            Some(encrypt_magic(&pub_magic, 1, file_key)?)
        };
        let (enc_key, nonce) = crypto::secretbox_seal(file_key, &target.key)?;
        let res: RemoteFile = api
            .json(
                Method::POST,
                "/files",
                &CreateFileRequest {
                    collection_id: target.id,
                    encrypted_key: crypto::b64(&enc_key),
                    key_decryption_nonce: crypto::b64(&nonce),
                    file: file.attributes(),
                    thumbnail: thumb.attributes(),
                    metadata: FileAttributes {
                        object_key: None,
                        encrypted_data: Some(md_data),
                        decryption_header: md_header,
                        size: None,
                    },
                    pub_magic_metadata,
                },
            )
            .await?;
        // Show it right away; the next sync brings the server's view.
        if let Ok(rec) = decrypt_file(&res, &target.key) {
            self.save_file(&rec, target.id)?;
        }
        Ok(res.id)
    }

    async fn add_to_collection(
        &self,
        api: &Api,
        file_id: i64,
        target: &CollectionRec,
    ) -> Result<()> {
        let f = self.file_rec(file_id)?;
        let (enc_key, nonce) = crypto::secretbox_seal(&f.key, &target.key)?;
        api.json_no_content(
            Method::POST,
            "/collections/add-files",
            &AddFilesRequest {
                collection_id: target.id,
                files: vec![CollectionFileItem {
                    id: file_id,
                    encrypted_key: crypto::b64(&enc_key),
                    key_decryption_nonce: crypto::b64(&nonce),
                }],
            },
        )
        .await?;
        self.save_file(&f, target.id)
    }

    /// The user's uncategorized collection, created as the apps do if missing.
    async fn uncategorized(&self, api: &Api) -> Result<CollectionRec> {
        if let Some(c) = self
            .all_collections()?
            .into_iter()
            .find(|c| c.owned && c.kind == "uncategorized")
        {
            return Ok(c);
        }
        let s = self.session()?;
        let key = crypto::generate_key();
        let (enc_key, key_nonce) = crypto::secretbox_seal(&key, &s.master_key)?;
        let (enc_name, name_nonce) = crypto::secretbox_seal(UNCATEGORIZED_NAME.as_bytes(), &key)?;
        let res: CollectionResponse = api
            .json(
                Method::POST,
                "/collections",
                &CreateCollectionRequest {
                    encrypted_key: crypto::b64(&enc_key),
                    key_decryption_nonce: crypto::b64(&key_nonce),
                    encrypted_name: crypto::b64(&enc_name),
                    name_decryption_nonce: crypto::b64(&name_nonce),
                    kind: "uncategorized".into(),
                    magic_metadata: None,
                },
            )
            .await?;
        let rec = crate::library::decrypt_collection(&res.collection, &s)?;
        self.save_collection(&rec)?;
        Ok(rec)
    }
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
        assert_eq!(encrypted_size(0), 17);
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
