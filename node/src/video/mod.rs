//! Streamable video (HLS) generation, producing Ente's `vid_preview` file data.
//!
//! A streamable video in Ente is two things:
//!
//! 1. An **object**: one MPEG-TS file holding every HLS segment, each segment
//!    AES-128-CBC encrypted by `ffmpeg` with a random one-time key
//!    (`-hls_flags single_file`, `-hls_key_info_file`). It is uploaded as-is to
//!    a presigned URL from `POST /files/data/preview-upload-url` (or its
//!    multipart sibling) and gets a server-chosen `pv_…` object ID.
//! 2. A **playlist**: the JSON
//!    `{"type":"hls_video","playlist":…,"width":…,"height":…,"size":…}`, where
//!    `playlist` is `ffmpeg`'s `.m3u8` text (which carries the one-time AES key
//!    as a `data:` URI), gzipped and encrypted with the file key as a
//!    libsodium secretstream blob, then registered with
//!    `PUT /files/video-data`.
//!
//! Apps reconstruct the playlist by replacing every segment URI with a signed
//! URL for the object (`web/packages/gallery/utils/hls.ts`), so the server
//! never sees the AES key or any plaintext.
//!
//! This module follows the Ente desktop app (`ffmpeg-worker.ts` and
//! `gallery/services/video.ts`) step by step. The HTTP calls themselves live in
//! [`crate::api`]; this module exposes the pure pieces the worker wires:
//!
//! ```text
//! candidate_eligibility()      skip non-videos, others' files, sv == 1, …
//! generate()                   NotRequired  → set pubMagicMetadata {sv: 1}
//!                              Generated(o) → UploadPlan::for_file(o)
//!   POST preview-upload-url    plan.single_upload_request() / multipart_upload_request()
//!   PUT  object bytes          plan.read_part() per part (+ complete_multipart_xml)
//!   PUT  /files/video-data     video_data_request(file_id, object_id, &o, &file_key)
//! ```

pub mod ffmpeg;

use std::io::Write;
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::sync::CancellationToken;

use crate::crypto::{self, EncryptedBlob, SecretKey};
pub use ffmpeg::{FfmpegConfig, HlsDecisions, VideoProbe};

/// The file-data type of streamable videos.
pub const PREVIEW_TYPE: &str = "vid_preview";

/// The `type` field of the playlist JSON.
pub const PLAYLIST_TYPE: &str = "hls_video";

/// H.264 originals at or below this size are not converted (desktop: "skip
/// H.264 under 10 MB"); the apps then set `sv = 1` on the file.
pub const H264_SKIP_MAX_SIZE: u64 = 10 * 1024 * 1024;

/// Part size for preview uploads: 96 MiB, leaving headroom below Cloudflare's
/// 100 MB request limit (desktop `uploadVideoSegments`).
pub const PREVIEW_PART_SIZE: u64 = 96 * 1024 * 1024;

/// Largest preview object museum accepts (`maxPreviewUploadSize`, 10 GiB).
pub const MAX_PREVIEW_SIZE: u64 = 10 << 30;

/// Largest decompressed playlist JSON the web app will read
/// (`maxHLSMetadataBytes`).
pub const MAX_PLAYLIST_JSON_BYTES: usize = 8 * 1024 * 1024;

/// Largest reconstructed playlist the web app will build
/// (`maxHLSPlaylistBytes`).
pub const MAX_HLS_PLAYLIST_BYTES: usize = 32 * 1024 * 1024;

/// Museum's `smallVideoPreviewSize` (200 MB) used by [`preview_size_allowed`].
const SMALL_VIDEO_PREVIEW_SIZE: u64 = 200_000_000;

/// Errors from streamable video generation.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    /// The caller cancelled (for example because the machine became busy).
    #[error("cancelled")]
    Cancelled,
    /// `ffmpeg` or `ffprobe` could not be started.
    #[error("could not run {program:?}: {source}")]
    Spawn {
        /// The program we tried to run.
        program: PathBuf,
        /// Why it failed.
        source: std::io::Error,
    },
    /// `ffprobe` failed or printed something we could not read.
    #[error("probe failed: {0}")]
    Probe(String),
    /// The file has no video stream.
    #[error("no video stream")]
    NoVideoStream,
    /// `ffmpeg` exited unsuccessfully.
    #[error("ffmpeg exited with {status}: {stderr_tail}")]
    Ffmpeg {
        /// Exit status.
        status: String,
        /// The end of `ffmpeg`'s stderr.
        stderr_tail: String,
    },
    /// The output dimensions could not be read from `ffmpeg`'s stderr.
    #[error("unable to detect output video dimensions")]
    Dimensions,
    /// `ffmpeg` produced a playlist the Ente apps would reject.
    #[error("invalid playlist: {0}")]
    InvalidPlaylist(&'static str),
    /// The generated stream is empty or larger than museum accepts.
    #[error("preview size {0} must be between 1 byte and 10 GiB")]
    Size(u64),
    /// Encryption failed.
    #[error(transparent)]
    Crypto(#[from] crypto::CryptoError),
    /// File system error.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// JSON (de)serialisation failed.
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, VideoError>;

/// What the worker knows about a library file before downloading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoCandidate {
    /// Whether the file's `metadata.fileType` is video (live photos are not).
    pub is_video: bool,
    /// The file's owner.
    pub owner_id: i64,
    /// The signed-in user.
    pub user_id: i64,
    /// Whether the file is in the trash.
    pub trashed: bool,
    /// `pubMagicMetadata.sv`, the "stream version" flag; `Some(1)` means a
    /// client already decided no stream is needed.
    pub stream_version: Option<i64>,
    /// Whether `vid_preview` data already exists for the file.
    pub has_preview: bool,
}

/// Why a file is or is not processed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    /// Download and run [`generate`].
    Eligible,
    /// Not a video.
    NotVideo,
    /// Owned by someone else (only owners may write preview data).
    NotOwner,
    /// In the trash.
    Trashed,
    /// `pubMagicMetadata.sv == 1`: stable "not required" marker.
    MarkedNotRequired,
    /// Another client already uploaded a stream.
    AlreadyProcessed,
}

/// The web/desktop app's candidate filter (`streamCandidateFiles`) plus its
/// "already processed" check (`processQueueItem`).
///
/// The mobile app additionally skips files over 500 MB or 60 s in its
/// *automatic* queue to save phone bandwidth and battery; the desktop app has
/// no such limit and neither does the node.
pub fn candidate_eligibility(c: &VideoCandidate) -> Eligibility {
    if !c.is_video {
        Eligibility::NotVideo
    } else if c.owner_id != c.user_id {
        Eligibility::NotOwner
    } else if c.trashed {
        Eligibility::Trashed
    } else if c.stream_version == Some(1) {
        Eligibility::MarkedNotRequired
    } else if c.has_preview {
        Eligibility::AlreadyProcessed
    } else {
        Eligibility::Eligible
    }
}

/// Whether a stream is needed for a downloaded original (desktop: H.264
/// originals of at most 10 MiB play fine as they are; HEVC is never skipped).
pub fn stream_required(probe: &VideoProbe, original_size: u64) -> bool {
    !(probe.is_h264() && original_size <= H264_SKIP_MAX_SIZE)
}

/// A generated, encrypted HLS stream waiting to be uploaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HlsOutput {
    /// `ffmpeg`'s `.m3u8` text, including the AES key as a `data:` URI. This
    /// is the `playlist` of the playlist JSON; it never leaves the node
    /// unencrypted.
    pub playlist_template: String,
    /// The AES-128 encrypted MPEG-TS object (`output.ts`).
    pub encrypted_video_path: PathBuf,
    /// Output video width.
    pub width: u32,
    /// Output video height.
    pub height: u32,
    /// Size of [`encrypted_video_path`](Self::encrypted_video_path) in bytes.
    pub size: u64,
}

/// Result of [`generate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GenerateOutcome {
    /// No stream is needed. Like the apps, the worker should set
    /// `pubMagicMetadata.sv = 1` on the file so no client tries again.
    NotRequired,
    /// A stream was generated.
    Generated(HlsOutput),
}

/// Generate an encrypted HLS stream for `input` inside `workdir`, as the Ente
/// desktop app does.
///
/// `workdir` is created (mode 0700) if needed and should be unique to this
/// file; on success it holds `output.m3u8` and `output.ts`, which the caller
/// deletes after uploading. The one-time key and temporary files are removed
/// before returning. Cancelling `cancel` kills `ffmpeg`/`ffprobe` and returns
/// [`VideoError::Cancelled`].
pub async fn generate(
    cfg: &FfmpegConfig,
    input: &Path,
    workdir: &Path,
    cancel: &CancellationToken,
) -> Result<GenerateOutcome> {
    let probe = ffmpeg::probe(cfg, input, cancel).await?;
    if !probe.has_video {
        return Err(VideoError::NoVideoStream);
    }
    let original_size = tokio::fs::metadata(input).await?.len();
    if !stream_required(&probe, original_size) {
        return Ok(GenerateOutcome::NotRequired);
    }
    let decisions = HlsDecisions::for_probe(&probe);
    tracing::debug!(?probe, ?decisions, "generating HLS stream");

    create_private_dir(workdir).await?;
    let playlist_path = workdir.join("output.m3u8");
    let video_path = workdir.join("output.ts");
    let stderr_path = workdir.join("stderr.txt");
    let key_path = workdir.join("output.m3u8.key");
    let key_info_path = workdir.join("output.m3u8.key-info");

    let result = async {
        let key: [u8; 16] = rand::random();
        write_private(&key_path, &key).await?;
        // ffmpeg key-info: the URI written to the playlist, then the local key path.
        let key_info = format!(
            "{}\n{}",
            key_uri(&key),
            key_path.to_str().ok_or(VideoError::InvalidPlaylist(
                "work directory path is not UTF-8"
            ))?
        );
        write_private(&key_info_path, key_info.as_bytes()).await?;

        let args = ffmpeg::hls_args(input, &decisions, &key_info_path, &playlist_path);
        ffmpeg::run_ffmpeg(cfg, &args, &stderr_path, cancel).await?;

        let playlist_template = tokio::fs::read_to_string(&playlist_path).await?;
        if playlist_template.contains("\r\n") {
            return Err(VideoError::InvalidPlaylist("unexpected CRLF newlines"));
        }
        validate_playlist_template(&playlist_template)?;
        let stderr = tokio::fs::read_to_string(&stderr_path).await?;
        let (width, height) =
            ffmpeg::parse_output_dimensions(&stderr).ok_or(VideoError::Dimensions)?;
        let size = tokio::fs::metadata(&video_path).await?.len();
        if size == 0 || size > MAX_PREVIEW_SIZE {
            return Err(VideoError::Size(size));
        }
        Ok(HlsOutput {
            playlist_template,
            encrypted_video_path: video_path.clone(),
            width,
            height,
            size,
        })
    }
    .await;

    let tmp = {
        let mut p = video_path.clone().into_os_string();
        p.push(".tmp");
        PathBuf::from(p)
    };
    let mut leftovers = vec![stderr_path, key_info_path, key_path, tmp];
    if result.is_err() {
        leftovers.extend([playlist_path, video_path]);
    }
    for p in leftovers {
        let _ = tokio::fs::remove_file(p).await;
    }
    result.map(GenerateOutcome::Generated)
}

/// The `data:` URI under which the AES key appears in the playlist.
pub fn key_uri(key: &[u8; 16]) -> String {
    format!("data:text/plain;base64,{}", B64.encode(key))
}

async fn create_private_dir(dir: &Path) -> Result<()> {
    let mut b = tokio::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    b.mode(0o700);
    b.create(dir).await?;
    Ok(())
}

async fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    let mut o = tokio::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    o.mode(0o600);
    let mut f = o.open(path).await?;
    tokio::io::AsyncWriteExt::write_all(&mut f, data).await?;
    tokio::io::AsyncWriteExt::flush(&mut f).await?;
    Ok(())
}

/// Whether a playlist line is a tag the web app accepts (the `allowedTag`
/// regex in `web/packages/gallery/utils/hls.ts`).
pub fn is_allowed_tag(line: &str) -> bool {
    fn digits(s: &str) -> bool {
        !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
    }
    let Some(rest) = line.strip_prefix('#') else {
        return false;
    };
    if rest == "EXTM3U" || rest == "EXT-X-ENDLIST" {
        return true;
    }
    if let Some(inf) = rest.strip_prefix("EXTINF:") {
        let Some(num) = inf.strip_suffix(',') else {
            return false;
        };
        return match num.split_once('.') {
            Some((i, f)) => digits(i) && digits(f),
            None => digits(num),
        };
    }
    for prefix in [
        "EXT-X-VERSION:",
        "EXT-X-TARGETDURATION:",
        "EXT-X-MEDIA-SEQUENCE:",
    ] {
        if let Some(n) = rest.strip_prefix(prefix) {
            return digits(n);
        }
    }
    if let Some(br) = rest.strip_prefix("EXT-X-BYTERANGE:") {
        return match br.split_once('@') {
            Some((n, o)) => digits(n) && digits(o),
            None => digits(br),
        };
    }
    if let Some(key) = rest.strip_prefix("EXT-X-KEY:METHOD=AES-128,URI=\"data:text/plain;base64,") {
        let Some((b64, iv)) = key.split_once("\",IV=0x") else {
            return false;
        };
        return b64.len() == 24
            && b64.ends_with("==")
            && b64[..22]
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
            && iv.len() == 32
            && iv.bytes().all(|b| b.is_ascii_hexdigit());
    }
    false
}

/// Rebuild a playable playlist from a template, exactly as the web app's
/// `reconstructHLSPlaylist`: every URI line becomes `segment_url`, comments
/// are dropped, and any unexpected tag rejects the whole playlist.
pub fn reconstruct_playlist(template: &str, segment_url: &str) -> Option<String> {
    if segment_url.contains(['\r', '\n']) {
        return None;
    }
    let mut out: Vec<&str> = Vec::new();
    let mut bytes = 0usize;
    let (mut has_key, mut has_segment) = (false, false);
    for raw in template.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() || (line.starts_with('#') && !line.starts_with("#EXT")) {
            continue;
        }
        let line = if !line.starts_with('#') {
            has_segment = true;
            segment_url
        } else if is_allowed_tag(line) {
            has_key |= line.starts_with("#EXT-X-KEY:");
            line
        } else {
            return None;
        };
        bytes += line.len() + 1;
        if bytes > MAX_HLS_PLAYLIST_BYTES {
            return None;
        }
        out.push(line);
    }
    if !has_key
        || !has_segment
        || out.first() != Some(&"#EXTM3U")
        || out.last() != Some(&"#EXT-X-ENDLIST")
    {
        return None;
    }
    out.push("");
    Some(out.join("\n"))
}

/// Check that the web app would accept `template`.
pub fn validate_playlist_template(template: &str) -> Result<()> {
    reconstruct_playlist(template, "https://example.invalid/segment")
        .map(|_| ())
        .ok_or(VideoError::InvalidPlaylist(
            "rejected by the apps' playlist rules",
        ))
}

/// The playlist JSON the apps store (encrypted) in `vid_preview` metadata.
///
/// Field order matches the web app's `JSON.stringify` output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaylistJson {
    /// Always [`PLAYLIST_TYPE`].
    #[serde(rename = "type")]
    pub kind: String,
    /// The `.m3u8` template.
    pub playlist: String,
    /// Video width.
    pub width: u32,
    /// Video height.
    pub height: u32,
    /// Size of the encrypted video object.
    pub size: u64,
}

impl PlaylistJson {
    /// The playlist JSON for a generated stream.
    pub fn for_output(out: &HlsOutput) -> Self {
        Self {
            kind: PLAYLIST_TYPE.to_owned(),
            playlist: out.playlist_template.clone(),
            width: out.width,
            height: out.height,
            size: out.size,
        }
    }

    /// Serialise and gzip (the web app's `encodePlaylistJSON`).
    pub fn to_gzip(&self) -> Result<Vec<u8>> {
        let json = serde_json::to_vec(self)?;
        if json.len() > MAX_PLAYLIST_JSON_BYTES {
            return Err(VideoError::InvalidPlaylist("playlist JSON exceeds 8 MiB"));
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(&json)?;
        Ok(gz.finish()?)
    }

    /// Gunzip and parse, refusing more than [`MAX_PLAYLIST_JSON_BYTES`] of JSON.
    pub fn from_gzip(data: &[u8]) -> Result<Self> {
        use std::io::Read;
        let mut json = Vec::new();
        flate2::read::GzDecoder::new(data)
            .take(MAX_PLAYLIST_JSON_BYTES as u64 + 1)
            .read_to_end(&mut json)?;
        if json.len() > MAX_PLAYLIST_JSON_BYTES {
            return Err(VideoError::InvalidPlaylist("playlist JSON exceeds 8 MiB"));
        }
        Ok(serde_json::from_slice(&json)?)
    }

    /// Gzip and encrypt with the file key (`encryptBlob(gzip(json), file.key)`).
    pub fn encrypt(&self, file_key: &SecretKey) -> Result<EncryptedBlob> {
        Ok(crypto::encrypt_blob(&self.to_gzip()?, file_key)?)
    }

    /// Inverse of [`encrypt`](Self::encrypt).
    pub fn decrypt(blob: &EncryptedBlob, file_key: &SecretKey) -> Result<Self> {
        let gz = crypto::decrypt_blob(&blob.encrypted_data, &blob.decryption_header, file_key)?;
        Self::from_gzip(&gz)
    }
}

/// Body of `PUT /files/video-data` (museum `VidPreviewRequest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoDataRequest {
    /// The file.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// The `pv_…` object ID from the upload URL response.
    #[serde(rename = "objectID")]
    pub object_id: String,
    /// Size of the uploaded object.
    #[serde(rename = "objectSize")]
    pub object_size: u64,
    /// Base64 encrypted, gzipped playlist JSON.
    pub playlist: String,
    /// Base64 secretstream header for `playlist`.
    #[serde(rename = "playlistHeader")]
    pub playlist_header: String,
}

/// Build the `PUT /files/video-data` body for an uploaded stream.
pub fn video_data_request(
    file_id: i64,
    object_id: &str,
    out: &HlsOutput,
    file_key: &SecretKey,
) -> Result<VideoDataRequest> {
    let blob = PlaylistJson::for_output(out).encrypt(file_key)?;
    Ok(VideoDataRequest {
        file_id,
        object_id: object_id.to_owned(),
        object_size: out.size,
        playlist: B64.encode(&blob.encrypted_data),
        playlist_header: B64.encode(&blob.decryption_header),
    })
}

/// Whether `id` is a valid preview object ID (museum: `^pv_[0-9A-Za-z]{22}$`).
pub fn is_valid_object_id(id: &str) -> bool {
    id.strip_prefix("pv_")
        .is_some_and(|r| r.len() == 22 && r.bytes().all(|b| b.is_ascii_alphanumeric()))
}

/// Museum's `VidPreviewRequest.ValidateSize`: whether a preview of
/// `object_size` bytes is accepted for an original of `original_size` bytes.
/// Check this before uploading to avoid wasting bandwidth.
pub fn preview_size_allowed(object_size: u64, original_size: u64) -> bool {
    if original_size == 0 {
        // Museum skips the check when it does not know the original size.
        return true;
    }
    if object_size == 0 {
        return false;
    }
    if original_size > (i64::MAX as u64) / 3 || object_size <= 3 * original_size {
        return true;
    }
    object_size < SMALL_VIDEO_PREVIEW_SIZE && object_size / 4 < original_size
}

/// One part of a preview upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPart {
    /// 1-based part number.
    pub number: u32,
    /// Byte offset in the object.
    pub offset: u64,
    /// Length in bytes.
    pub len: u64,
    /// Base64 MD5 of the part, sent as `Content-MD5`.
    pub md5_b64: String,
}

/// How to upload a generated stream (desktop `uploadVideoSegments`, upload
/// V2 with MD5s): a single PUT when it fits in one 96 MiB part, otherwise an
/// S3 multipart upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadPlan {
    /// The object on disk.
    pub path: PathBuf,
    /// Total size.
    pub size: u64,
    /// Part size ([`PREVIEW_PART_SIZE`]).
    pub part_size: u64,
    /// Parts in order.
    pub parts: Vec<UploadPart>,
}

/// Body of `POST /files/data/preview-upload-url` (museum
/// `PreviewUploadRequest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewUploadRequest {
    /// The file.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Always [`PREVIEW_TYPE`].
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Object size.
    #[serde(rename = "contentLength")]
    pub content_length: u64,
    /// Base64 MD5 of the object.
    #[serde(rename = "contentMD5")]
    pub content_md5: String,
}

/// Body of `POST /files/data/multipart-preview-upload-url` (museum
/// `MultipartPreviewUploadRequest`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MultipartPreviewUploadRequest {
    /// The file.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Always [`PREVIEW_TYPE`].
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// Object size.
    #[serde(rename = "contentLength")]
    pub content_length: u64,
    /// Part size.
    #[serde(rename = "partLength")]
    pub part_length: u64,
    /// Base64 MD5 of each part.
    #[serde(rename = "partMd5s")]
    pub part_md5s: Vec<String>,
}

/// Response of either upload-URL endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PreviewUploadUrlResponse {
    /// The `pv_…` object ID to pass to `PUT /files/video-data`.
    #[serde(rename = "objectID")]
    pub object_id: String,
    /// Single-part upload URL.
    #[serde(default)]
    pub url: Option<String>,
    /// Multipart part URLs, in order.
    #[serde(default, rename = "partURLs")]
    pub part_urls: Option<Vec<String>>,
    /// Multipart completion URL (POST the XML from [`complete_multipart_xml`]).
    #[serde(default, rename = "completeURL")]
    pub complete_url: Option<String>,
}

impl UploadPlan {
    /// Plan the upload of `out`, computing each part's MD5.
    pub async fn for_file(out: &HlsOutput) -> Result<Self> {
        Self::for_path(&out.encrypted_video_path, out.size, PREVIEW_PART_SIZE).await
    }

    /// Plan the upload of `size` bytes at `path` in parts of `part_size`.
    pub async fn for_path(path: &Path, size: u64, part_size: u64) -> Result<Self> {
        if size == 0 || size > MAX_PREVIEW_SIZE {
            return Err(VideoError::Size(size));
        }
        let mut f = tokio::fs::File::open(path).await?;
        let mut parts = Vec::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut offset = 0;
        while offset < size {
            let len = part_size.min(size - offset);
            let mut h = Md5::new();
            let mut left = len;
            while left > 0 {
                let n = (left as usize).min(buf.len());
                f.read_exact(&mut buf[..n]).await?;
                h.update(&buf[..n]);
                left -= n as u64;
            }
            parts.push(UploadPart {
                number: parts.len() as u32 + 1,
                offset,
                len,
                md5_b64: B64.encode(h.finalize()),
            });
            offset += len;
        }
        Ok(Self {
            path: path.to_owned(),
            size,
            part_size,
            parts,
        })
    }

    /// Whether this needs the multipart endpoint.
    pub fn is_multipart(&self) -> bool {
        self.parts.len() > 1
    }

    /// Request body for a single-part upload URL (when not multipart).
    pub fn single_upload_request(&self, file_id: i64) -> PreviewUploadRequest {
        PreviewUploadRequest {
            file_id,
            kind: PREVIEW_TYPE,
            content_length: self.size,
            content_md5: self.parts[0].md5_b64.clone(),
        }
    }

    /// Request body for a multipart upload URL.
    pub fn multipart_upload_request(&self, file_id: i64) -> MultipartPreviewUploadRequest {
        MultipartPreviewUploadRequest {
            file_id,
            kind: PREVIEW_TYPE,
            content_length: self.size,
            part_length: self.part_size,
            part_md5s: self.parts.iter().map(|p| p.md5_b64.clone()).collect(),
        }
    }

    /// Query parameters for the legacy `GET /files/data/preview-upload-url`
    /// (desktop `getFilePreviewDataUploadURL`, used when upload V2 is off).
    pub fn legacy_query(&self, file_id: i64, now_ms: i64) -> Vec<(&'static str, String)> {
        let mut q = vec![
            ("fileID", file_id.to_string()),
            ("type", PREVIEW_TYPE.to_owned()),
            ("ts", now_ms.to_string()),
        ];
        if self.is_multipart() {
            q.push(("isMultiPart", "true".to_owned()));
            q.push(("count", self.parts.len().to_string()));
        }
        q
    }

    /// Check that the server's response has the URLs this plan needs.
    pub fn check_response(&self, r: &PreviewUploadUrlResponse) -> Result<()> {
        let ok = if self.is_multipart() {
            r.part_urls
                .as_ref()
                .is_some_and(|u| u.len() == self.parts.len())
                && r.complete_url.is_some()
        } else {
            r.url.is_some()
                || (r.part_urls.as_ref().is_some_and(|u| u.len() == 1) && r.complete_url.is_some())
        };
        if ok && is_valid_object_id(&r.object_id) {
            Ok(())
        } else {
            Err(VideoError::InvalidPlaylist("malformed upload URLs"))
        }
    }

    /// Read one part's bytes (at most 96 MiB) to PUT.
    pub async fn read_part(&self, part: &UploadPart) -> Result<Vec<u8>> {
        let mut f = tokio::fs::File::open(&self.path).await?;
        f.seek(std::io::SeekFrom::Start(part.offset)).await?;
        let mut buf = vec![0u8; part.len as usize];
        f.read_exact(&mut buf).await?;
        Ok(buf)
    }
}

/// The S3 `CompleteMultipartUpload` body from each part's `ETag` header, in
/// part order (desktop `uploadVideoSegmentsMultipart`; ETags are used verbatim,
/// quotes included). POST it with `Content-Type: text/xml`.
pub fn complete_multipart_xml(etags: &[String]) -> String {
    let mut s = String::from("<CompleteMultipartUpload>");
    for (i, e) in etags.iter().enumerate() {
        s.push_str(&format!(
            "<Part><PartNumber>{}</PartNumber><ETag>{}</ETag></Part>",
            i + 1,
            e
        ));
    }
    s.push_str("</CompleteMultipartUpload>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEMPLATE: &str = "#EXTM3U\n#EXT-X-VERSION:4\n#EXT-X-TARGETDURATION:8\n#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-KEY:METHOD=AES-128,URI=\"data:text/plain;base64,Vpk0K5bsiSp4UpYzWC8Lvg==\",IV=0x00000000000000000000000000000000\n#EXTINF:8.333333,\n#EXT-X-BYTERANGE:153792@0\noutput.ts\n#EXTINF:8.333333,\n#EXT-X-BYTERANGE:154176@153792\noutput.ts\n#EXT-X-ENDLIST\n";

    #[test]
    fn reconstructs_like_hls_ts() {
        let out = reconstruct_playlist(TEMPLATE, "https://s/x?sig=1").unwrap();
        assert_eq!(out, TEMPLATE.replace("output.ts", "https://s/x?sig=1"));
        assert!(reconstruct_playlist(TEMPLATE, "a\nb").is_none());
        // Unknown tags are rejected; comments are dropped.
        let bad = TEMPLATE.replace("#EXT-X-VERSION:4", "#EXT-X-PLAYLIST-TYPE:VOD");
        assert!(reconstruct_playlist(&bad, "u").is_none());
        let commented = TEMPLATE.replace("#EXT-X-VERSION:4", "# hi");
        assert!(reconstruct_playlist(&commented, "u").is_some());
        // Keys must be 16-byte data URIs with explicit IVs.
        let nokey = TEMPLATE.replace(",IV=0x00000000000000000000000000000000", "");
        assert!(reconstruct_playlist(&nokey, "u").is_none());
        assert!(!is_allowed_tag("#EXTINF:8.,"));
        assert!(is_allowed_tag("#EXTINF:8,"));
        assert!(is_allowed_tag("#EXT-X-BYTERANGE:5"));
    }

    #[test]
    fn eligibility() {
        let base = VideoCandidate {
            is_video: true,
            owner_id: 1,
            user_id: 1,
            trashed: false,
            stream_version: None,
            has_preview: false,
        };
        assert_eq!(candidate_eligibility(&base), Eligibility::Eligible);
        let c = |f: fn(&mut VideoCandidate)| {
            let mut c = base.clone();
            f(&mut c);
            candidate_eligibility(&c)
        };
        assert_eq!(c(|c| c.is_video = false), Eligibility::NotVideo);
        assert_eq!(c(|c| c.owner_id = 2), Eligibility::NotOwner);
        assert_eq!(c(|c| c.trashed = true), Eligibility::Trashed);
        assert_eq!(
            c(|c| c.stream_version = Some(1)),
            Eligibility::MarkedNotRequired
        );
        assert_eq!(c(|c| c.stream_version = Some(0)), Eligibility::Eligible);
        assert_eq!(c(|c| c.has_preview = true), Eligibility::AlreadyProcessed);

        let h264 = VideoProbe {
            codec_name: Some("h264".into()),
            has_video: true,
            ..Default::default()
        };
        assert!(!stream_required(&h264, H264_SKIP_MAX_SIZE));
        assert!(stream_required(&h264, H264_SKIP_MAX_SIZE + 1));
        let hevc = VideoProbe {
            codec_name: Some("hevc".into()),
            ..h264
        };
        assert!(stream_required(&hevc, 1));
    }

    #[test]
    fn museum_rules() {
        assert!(is_valid_object_id("pv_0123456789abcdefABCDEF"));
        assert!(!is_valid_object_id("pv_0123456789abcdefABCDE"));
        assert!(!is_valid_object_id("xx_0123456789abcdefABCDEF"));
        assert!(preview_size_allowed(300, 100));
        assert!(preview_size_allowed(399, 100));
        assert!(!preview_size_allowed(400, 100));
        assert!(!preview_size_allowed(SMALL_VIDEO_PREVIEW_SIZE, 60_000_000));
        assert!(preview_size_allowed(5, 0));
    }

    #[test]
    fn playlist_json_shape_and_gzip() {
        let out = HlsOutput {
            playlist_template: TEMPLATE.into(),
            encrypted_video_path: "/x".into(),
            width: 1280,
            height: 720,
            size: 307968,
        };
        let pj = PlaylistJson::for_output(&out);
        let json = serde_json::to_string(&pj).unwrap();
        assert!(json.starts_with(r##"{"type":"hls_video","playlist":"#EXTM3U"##));
        assert!(json.ends_with(r#","width":1280,"height":720,"size":307968}"#));
        assert_eq!(PlaylistJson::from_gzip(&pj.to_gzip().unwrap()).unwrap(), pj);
    }

    #[test]
    fn multipart_xml() {
        assert_eq!(
            complete_multipart_xml(&["\"a\"".into(), "\"b\"".into()]),
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"a\"</ETag></Part><Part><PartNumber>2</PartNumber><ETag>\"b\"</ETag></Part></CompleteMultipartUpload>"
        );
    }

    #[tokio::test]
    async fn upload_plan_parts() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("o.ts");
        std::fs::write(&p, vec![7u8; 2500]).unwrap();
        let plan = UploadPlan::for_path(&p, 2500, 1000).await.unwrap();
        assert!(plan.is_multipart());
        let lens: Vec<u64> = plan.parts.iter().map(|p| p.len).collect();
        assert_eq!(lens, [1000, 1000, 500]);
        assert_eq!(plan.parts[2].offset, 2000);
        assert_eq!(
            plan.parts[2].md5_b64,
            B64.encode(Md5::digest(vec![7u8; 500]))
        );
        assert_eq!(
            plan.read_part(&plan.parts[2]).await.unwrap(),
            vec![7u8; 500]
        );
        let req = serde_json::to_value(plan.multipart_upload_request(9)).unwrap();
        assert_eq!(req["partMd5s"].as_array().unwrap().len(), 3);
        assert_eq!(req["type"], "vid_preview");
        let q = plan.legacy_query(9, 1);
        assert!(q.contains(&("count", "3".into())));

        let single = UploadPlan::for_path(&p, 2500, PREVIEW_PART_SIZE)
            .await
            .unwrap();
        assert!(!single.is_multipart());
        let req = serde_json::to_value(single.single_upload_request(9)).unwrap();
        assert_eq!(req["contentLength"], 2500);
        assert!(req["contentMD5"].is_string());
    }
}
