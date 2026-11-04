//! Streamable videos: Ente's `vid_preview` file data, played through a
//! loopback HTTP server inside core.
//!
//! * Which files have a stream is learnt the way Ente's apps learn it: the
//!   file data status diff (`POST /files/data/status-diff {lastUpdatedAt}`,
//!   mobile `FileDataService._syncFDStatusInternal`, web
//!   `syncUpdatedFileDataFileIDs`), run after every library sync and kept in
//!   the `video_streams` table. The diff lists the account's own files only,
//!   so, like Ente's video widget (`isPreviewLoadable = true` for files of
//!   others), a shared video is checked on demand by [`Core::stream_playlist`].
//!   `TimelineItem::has_stream` is true for videos with a known stream whose
//!   public magic metadata does not say `sv: 1` (web: `"skip"`).
//! * The playlist format and the segment encryption are described in
//!   [`playlist`]. [`Core::stream_playlist`] fetches and decrypts the playlist
//!   with the file key, validates it like Ente, and registers it with the
//!   [`loopback`] server, which decrypts segments on demand. The app plays
//!   the returned `http://127.0.0.1:<port>/<token>/<stream>/index.m3u8`.

pub mod loopback;
pub mod playlist;

use crate::api::Api;
use crate::client::Inner;
use crate::library::FileRec;
use crate::{Core, CoreError, Result, crypto, run};
use reqwest::Method;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

/// Ente's file data type of a streamable video.
pub const VID_PREVIEW: &str = "vid_preview";
/// kv key of the status-diff cursor for `vid_preview` (plain).
const KV_FD_SINCE: &str = "video_fd_since";

/// A playable stream of a video, as [`Core::video_stream`] returns it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VideoStream {
    /// `http://127.0.0.1:<port>/<token>/<stream>/index.m3u8`: an HLS playlist
    /// served by core, whose segments core fetches and decrypts on demand.
    pub playlist_url: String,
    /// Stream width in pixels, 0 if unknown.
    pub width: u32,
    /// Stream height in pixels, 0 if unknown.
    pub height: u32,
    /// Sum of the segment durations.
    pub duration_ms: u64,
    /// Bytes of the encrypted segment object.
    pub size_bytes: u64,
}

/// One `filedata.FDStatus` of the diff.
#[derive(Debug, Deserialize)]
struct FdStatus {
    #[serde(rename = "fileID")]
    file_id: i64,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "isDeleted", default)]
    is_deleted: bool,
    #[serde(rename = "objectID", default)]
    object_id: Option<String>,
    #[serde(default)]
    size: i64,
    #[serde(rename = "updatedAt")]
    updated_at: i64,
}

#[derive(Debug, Deserialize)]
struct StatusDiff {
    #[serde(default)]
    diff: Option<Vec<FdStatus>>,
}

#[derive(Debug, Deserialize)]
struct FetchOne {
    data: FileDataEntity,
}

#[derive(Debug, Deserialize)]
struct FileDataEntity {
    #[serde(rename = "encryptedData")]
    encrypted_data: String,
    #[serde(rename = "decryptionHeader")]
    decryption_header: String,
}

#[derive(Debug, Deserialize)]
struct PreviewUrl {
    url: String,
}

/// `GET /files/data/preview?fileID&type=vid_preview`: the presigned URL of
/// the segment object, `None` when there is none (404).
pub(crate) async fn preview_url(api: &Api, file_id: i64) -> Result<Option<String>> {
    let r: Result<PreviewUrl> = api
        .get_json(
            "/files/data/preview",
            &[
                ("fileID", file_id.to_string()),
                ("type", VID_PREVIEW.to_string()),
            ],
        )
        .await;
    match r {
        Ok(p) => Ok(Some(p.url)),
        Err(CoreError::Server { status: 404, .. }) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `GET /files/data/fetch?fileID&type=vid_preview&preferNoContent=true`:
/// `None` on 204 (or 404 from older servers), like web `fetchFileData`.
async fn fetch_playlist_data(api: &Api, file_id: i64) -> Result<Option<FileDataEntity>> {
    let res = api
        .get(
            "/files/data/fetch",
            &[
                ("fileID", file_id.to_string()),
                ("type", VID_PREVIEW.to_string()),
                ("preferNoContent", "true".to_string()),
            ],
        )
        .await;
    let res = match res {
        Ok(r) => r,
        Err(CoreError::Server { status: 404, .. }) => return Ok(None),
        Err(e) => return Err(e),
    };
    if res.status() == reqwest::StatusCode::NO_CONTENT {
        return Ok(None);
    }
    let bytes = res.bytes().await.map_err(CoreError::network)?;
    let one: FetchOne = serde_json::from_slice(&bytes)
        .map_err(|e| CoreError::protocol(format!("bad file data answer: {e}")))?;
    Ok(Some(one.data))
}

/// Whether Ente's clients would play a stream of this file at all.
fn eligible(f: &FileRec) -> bool {
    let video = f.metadata.get("fileType").and_then(Value::as_i64)
        == Some(crate::metadata::FILE_TYPE_VIDEO);
    // `sv: 1` in public magic metadata: "skip video preview" (web `"skip"`).
    let skip = f
        .pub_magic
        .get("sv")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
        == Some(1);
    video && !skip
}

impl Inner {
    /// Pull the account's `vid_preview` status changes into `video_streams`.
    pub(crate) async fn sync_stream_status(&self, api: &Api) -> Result<()> {
        loop {
            let since: i64 = self
                .store()
                .get_plain(KV_FD_SINCE)?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            let d: StatusDiff = api
                .json(
                    Method::POST,
                    "/files/data/status-diff",
                    &json!({ "lastUpdatedAt": since }),
                )
                .await?;
            let rows = d.diff.unwrap_or_default();
            if rows.is_empty() {
                // Mark the status as known, even for an account without streams.
                self.store().put_plain(KV_FD_SINCE, &since.to_string())?;
                return Ok(());
            }
            let mut max = since;
            {
                let mut st = self.store();
                let tx = st.conn_mut().transaction()?;
                for r in &rows {
                    max = max.max(r.updated_at);
                    if r.kind != VID_PREVIEW {
                        continue;
                    }
                    if r.is_deleted {
                        tx.execute(
                            "DELETE FROM video_streams WHERE file_id = ?1",
                            params![r.file_id],
                        )?;
                    } else {
                        tx.execute(
                            "INSERT INTO video_streams (file_id, object_id, size, updated_at) \
                             VALUES (?1, ?2, ?3, ?4) ON CONFLICT(file_id) DO UPDATE SET \
                             object_id = excluded.object_id, size = excluded.size, \
                             updated_at = excluded.updated_at",
                            params![r.file_id, r.object_id, r.size, r.updated_at],
                        )?;
                    }
                }
                tx.execute(
                    "INSERT INTO kv (k, v) VALUES (?1, ?2) ON CONFLICT(k) DO UPDATE SET v = excluded.v",
                    params![KV_FD_SINCE, max.to_string().into_bytes()],
                )?;
                tx.commit()?;
            }
            if max == since {
                // A page that does not move the cursor would repeat forever.
                return Ok(());
            }
        }
    }

    /// Ids of files with a known stream (read under the store lock by the
    /// index rebuild).
    pub(crate) fn known_streams(conn: &rusqlite::Connection) -> Result<HashSet<i64>> {
        let mut q = conn.prepare("SELECT file_id FROM video_streams")?;
        let ids = q
            .query_map([], |r| r.get::<_, i64>(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Whether the index should show a stream badge for `f`.
    pub(crate) fn has_stream(f: &FileRec, known: &HashSet<i64>) -> bool {
        known.contains(&f.id) && eligible(f)
    }

    /// Fetch, decrypt and validate the stream of `file_id` and register it
    /// with the loopback server.
    pub(crate) async fn video_stream(&self, file_id: i64) -> Result<Option<VideoStream>> {
        let s = self.session()?;
        let f = self.file_rec(file_id)?;
        if !eligible(&f) {
            return Ok(None);
        }
        // Our own file, and the status diff ran: it knows whether there is one.
        if f.owner_id == s.account.user_id {
            let st = self.store();
            if st.get_plain(KV_FD_SINCE)?.is_some() {
                let known: i64 = st.conn().query_row(
                    "SELECT COUNT(*) FROM video_streams WHERE file_id = ?1",
                    params![file_id],
                    |r| r.get(0),
                )?;
                if known == 0 {
                    return Ok(None);
                }
            }
        }
        let api = self.api()?;
        let Some(data) = fetch_playlist_data(&api, file_id).await? else {
            return Ok(None);
        };
        let ct = crypto::unb64(&data.encrypted_data)?;
        let header = crypto::unb64(&data.decryption_header)?;
        let mut gz = crypto::decrypt_blob(&ct, &header, &f.key)?;
        let parsed = crate::offload(|| playlist::parse_playlist_json(&gz));
        crypto::wipe(&mut gz);
        let pj = parsed?;
        if pj.kind != playlist::HLS_VIDEO {
            return Ok(None);
        }
        // An invalid template is ignored (web: "Ignoring invalid HLS
        // playlist"): the app falls back to the original.
        let Ok(preview) = playlist::parse_template(&pj.playlist) else {
            return Ok(None);
        };
        let Some(url) = preview_url(&api, file_id).await? else {
            return Ok(None);
        };
        let dim = |v: Option<f64>| {
            v.filter(|v| *v > 0.0)
                .map_or(0, |v| v.min(u32::MAX as f64) as u32)
        };
        let duration_ms = preview.duration_ms();
        let size_bytes = pj.size.filter(|v| *v > 0.0).map_or(0, |v| v as u64);
        let playlist_url = loopback::server()?.add(preview, loopback::Source { api, file_id, url });
        Ok(Some(VideoStream {
            playlist_url,
            width: dim(pj.width),
            height: dim(pj.height),
            duration_ms,
            size_bytes,
        }))
    }

    /// `stream_playlist` of contract v1: the playlist URL of [`Self::video_stream`].
    pub(crate) async fn stream_playlist_url(&self, file_id: i64) -> Result<Option<String>> {
        Ok(self.video_stream(file_id).await?.map(|v| v.playlist_url))
    }
}

#[uniffi::export]
impl Core {
    /// The stream of a video with its size and duration, or `None` when it
    /// has none (then play the original from `download`). Like
    /// `stream_playlist`, which returns just `playlist_url`.
    pub async fn video_stream(&self, file_id: i64) -> Result<Option<VideoStream>, CoreError> {
        let inner = self.inner.clone();
        run(async move { inner.video_stream(file_id).await }).await
    }

    /// End every stream of the loopback server (call when the viewer closes):
    /// their URLs stop working and their keys and decrypted segments are
    /// dropped.
    pub fn stop_streams(&self) {
        if let Ok(s) = loopback::server() {
            s.stop_all();
        }
    }

    /// Re-read the account's stream status now (also part of every `sync`).
    /// Returns the number of files with a known stream.
    pub async fn sync_streams(&self) -> Result<u64, CoreError> {
        let inner = self.inner.clone();
        run(async move {
            let api = inner.api()?;
            inner.sync_stream_status(&api).await?;
            inner.rebuild_index()?;
            let st = inner.store();
            Ok(Inner::known_streams(st.conn())?.len() as u64)
        })
        .await
    }
}
