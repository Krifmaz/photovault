//! [`ImmichSource`]: walks an Immich library and yields [`ImportItem`]s.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use reqwest::StatusCode;
use serde_json::{Value, json};

use super::api::{ImmichClient, is_transient, status_of};
use super::dates::{self, NaiveTimes};
use super::model::{Album, Asset, AssetStats, SearchResponse, ServerVersion, User};
use crate::item::{GeoPoint, ImportItem, MediaFile, MediaKind, Source};

/// Options for [`ImmichSource`].
#[derive(Debug, Clone)]
pub struct ImmichOptions {
    /// Also import assets in Immich's Locked Folder, into Ente's hidden
    /// album. Immich only shows locked assets to sessions unlocked with the
    /// PIN code, so with an API key this normally finds nothing; the pass is
    /// then skipped with a warning. Default `false`.
    pub include_locked: bool,
    /// Download Immich's preview image to use as the Ente thumbnail.
    /// When `false` (or the download fails) the uploader makes its own.
    /// Default `true`.
    pub thumbnails: bool,
    /// Assets per search request (1 to 1000). Default 250.
    pub page_size: u32,
    /// Directory for downloaded originals; the system temp dir if `None`.
    /// Needs room for the largest asset (plus its Live Photo video).
    pub temp_dir: Option<PathBuf>,
    /// Retries for a failed request (network error, 5xx, 429). Default 3.
    pub retries: u32,
    /// How to read capture times that have no time zone. Default
    /// [`NaiveTimes::AsImmich`].
    pub naive_times: NaiveTimes,
    /// Read timeout for one HTTP response (a stalled download). Default 5 min.
    pub timeout: Duration,
}

impl Default for ImmichOptions {
    fn default() -> Self {
        Self {
            include_locked: false,
            thumbnails: true,
            page_size: 250,
            temp_dir: None,
            retries: 3,
            naive_times: NaiveTimes::AsImmich,
            timeout: Duration::from_secs(300),
        }
    }
}

/// Which search API the server speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// v3.2+: `filter` object and cursor pagination.
    Filter,
    /// v1.133 to v3.1: top-level `visibility` and page numbers.
    Visibility,
    /// Before v1.133: `isArchived` / `isVisible` and page numbers.
    Legacy,
}

impl Mode {
    fn for_version(v: ServerVersion) -> Self {
        if v.at_least(3, 2) {
            Mode::Filter
        } else if v.at_least(1, 133) {
            Mode::Visibility
        } else {
            Mode::Legacy
        }
    }
}

/// One listing of assets (e.g. "timeline and archive", "locked").
#[derive(Debug, Clone)]
struct Pass {
    /// For logs.
    label: &'static str,
    /// Search fields specific to this pass (merged into the request).
    fields: Value,
    /// A failure with 401/403 skips the pass instead of aborting.
    optional: bool,
}

/// Where a pass is in its pagination.
#[derive(Debug, Clone)]
enum Cursor {
    Start,
    Token(String),
    Page(u32),
    End,
}

/// Everything learned from the server on first use.
#[derive(Debug)]
struct Library {
    mode: Mode,
    version: ServerVersion,
    me: String,
    /// Asset id → names of the albums it is in.
    albums: HashMap<String, Vec<String>>,
}

/// An Immich library as an import [`Source`].
///
/// Lists the user's own assets with `POST /search/metadata` (timeline and
/// archive, optionally the Locked Folder), skips trashed assets, assets of
/// partners, and the hidden motion parts of Live Photos, and downloads each
/// original (plus its Live Photo video and preview) to a temporary directory.
pub struct ImmichSource {
    client: ImmichClient,
    opts: ImmichOptions,
    tmp: tempfile::TempDir,
    lib: Option<Library>,
    passes: VecDeque<Pass>,
    current: Option<(Pass, Cursor)>,
    buffer: VecDeque<Asset>,
    /// Ids already yielded or skipped (passes may overlap).
    seen: HashSet<String>,
    /// Motion-video ids of Live Photos, never imported on their own.
    motion: HashSet<String>,
    failures: Vec<String>,
}

impl ImmichSource {
    /// Create a source for the Immich server at `base_url` (web address or
    /// API root), authorised with an API key. Nothing is requested until the
    /// first call to [`Source::total`] or [`Source::next`].
    pub fn new(
        base_url: &str,
        api_key: impl Into<String>,
        opts: ImmichOptions,
    ) -> anyhow::Result<Self> {
        let client = ImmichClient::new(base_url, api_key, opts.timeout)?;
        let dir = opts.temp_dir.clone().unwrap_or_else(std::env::temp_dir);
        let tmp = tempfile::Builder::new()
            .prefix("photovault-immich-")
            .tempdir_in(&dir)
            .with_context(|| format!("creating temp dir in {}", dir.display()))?;
        Ok(Self {
            client,
            opts,
            tmp,
            lib: None,
            passes: VecDeque::new(),
            current: None,
            buffer: VecDeque::new(),
            seen: HashSet::new(),
            motion: HashSet::new(),
            failures: Vec::new(),
        })
    }

    /// The temporary directory downloads go to (removed when the source is
    /// dropped).
    pub fn temp_path(&self) -> &Path {
        self.tmp.path()
    }

    /// Ids of assets that could not be downloaded and were left out. They
    /// are not in the journal, so the next run tries them again.
    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    /// The server version, once known.
    pub fn server_version(&self) -> Option<(u32, u32, u32)> {
        self.lib
            .as_ref()
            .map(|l| (l.version.major, l.version.minor, l.version.patch))
    }

    async fn ensure_init(&mut self) -> anyhow::Result<()> {
        if self.lib.is_some() {
            return Ok(());
        }
        let version = self.server_version_or_latest().await;
        let mode = Mode::for_version(version);
        let me: User = self
            .retry(|c| async move { c.get::<User>("users/me").await })
            .await
            .context("reading the Immich user (does the API key have the user.read permission?)")?;
        tracing::info!(version = %format!("{}.{}.{}", version.major, version.minor, version.patch), ?mode, "connected to Immich");
        let albums = self.album_membership(version, mode).await?;
        self.passes = passes(mode, self.opts.include_locked).into();
        self.lib = Some(Library {
            mode,
            version,
            me: me.id,
            albums,
        });
        Ok(())
    }

    async fn server_version_or_latest(&self) -> ServerVersion {
        for path in ["server/version", "server-info/version"] {
            match self.client.get::<ServerVersion>(path).await {
                Ok(v) if v.major > 0 || v.minor > 0 => return v,
                Ok(_) => {}
                Err(e) => tracing::debug!(error = %format!("{e:#}"), path, "no server version"),
            }
        }
        tracing::warn!("Immich did not report its version; assuming the current API");
        ServerVersion::LATEST
    }

    /// Run `f` with retries on transient errors.
    async fn retry<T, F, Fut>(&self, f: F) -> anyhow::Result<T>
    where
        F: Fn(ImmichClient) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        let mut attempt = 0;
        loop {
            match f(self.client.clone()).await {
                Ok(v) => return Ok(v),
                Err(e) if attempt < self.opts.retries && is_transient(&e) => {
                    attempt += 1;
                    let wait = Duration::from_millis(500 << attempt.min(6));
                    tracing::warn!(error = %format!("{e:#}"), attempt, "Immich request failed; retrying");
                    tokio::time::sleep(wait).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Map every asset to the names of the albums it is in: albums the user
    /// owns and albums shared with the user.
    async fn album_membership(
        &self,
        version: ServerVersion,
        mode: Mode,
    ) -> anyhow::Result<HashMap<String, Vec<String>>> {
        let mut albums: Vec<Album> = self
            .retry(|c| async move { c.get::<Vec<Album>>("albums").await })
            .await
            .context("listing Immich albums (does the API key have the album.read permission?)")?;
        if !version.at_least(3, 0) {
            // Before v3, `GET /albums` lists owned albums only.
            match self
                .retry(|c| async move { c.get::<Vec<Album>>("albums?shared=true").await })
                .await
            {
                Ok(shared) => {
                    let known: HashSet<String> = albums.iter().map(|a| a.id.clone()).collect();
                    albums.extend(shared.into_iter().filter(|a| !known.contains(&a.id)));
                }
                Err(e) => tracing::warn!(error = %format!("{e:#}"), "could not list shared albums"),
            }
        }
        let mut map: HashMap<String, Vec<String>> = HashMap::new();
        for album in &albums {
            let name = match album.album_name.trim() {
                "" => "Untitled".to_owned(),
                n => n.to_owned(),
            };
            let ids = self
                .album_asset_ids(album, version, mode)
                .await
                .with_context(|| format!("listing album {name:?}"))?;
            tracing::debug!(album = %name, assets = ids.len(), "album");
            for id in ids {
                let names = map.entry(id).or_default();
                if !names.contains(&name) {
                    names.push(name.clone());
                }
            }
        }
        tracing::info!(albums = albums.len(), "read Immich albums");
        Ok(map)
    }

    async fn album_asset_ids(
        &self,
        album: &Album,
        version: ServerVersion,
        mode: Mode,
    ) -> anyhow::Result<Vec<String>> {
        if !version.at_least(3, 0) {
            let path = format!("albums/{}", album.id);
            let full: Album = self
                .retry(|c| {
                    let p = path.clone();
                    async move { c.get::<Album>(&p).await }
                })
                .await?;
            return Ok(full
                .assets
                .unwrap_or_default()
                .into_iter()
                .map(|a| a.id)
                .collect());
        }
        let mut ids = Vec::new();
        let mut cursor = Cursor::Start;
        loop {
            let mut body = match mode {
                Mode::Filter => json!({ "filter": { "albumIds": { "any": [album.id] } } }),
                _ => json!({ "albumIds": [album.id] }),
            };
            body["size"] = json!(1000);
            if !apply_cursor(&mut body, &cursor) {
                break;
            }
            let page: SearchResponse = self
                .retry(|c| {
                    let b = body.clone();
                    async move { c.post::<SearchResponse>("search/metadata", &b).await }
                })
                .await?;
            let n = page.assets.items.len();
            ids.extend(page.assets.items.into_iter().map(|a| a.id));
            cursor = next_cursor(
                mode,
                &cursor,
                page.assets.next_cursor,
                page.assets.next_page,
                n,
            );
        }
        Ok(ids)
    }

    /// Fetch the next page of the current pass into the buffer. Returns
    /// `false` when every pass is exhausted.
    async fn fill(&mut self) -> anyhow::Result<bool> {
        let mode = self.lib.as_ref().map_or(Mode::Filter, |l| l.mode);
        loop {
            let (pass, cursor) = match self.current.take() {
                Some((p, Cursor::End)) => {
                    tracing::debug!(pass = p.label, "pass finished");
                    continue;
                }
                Some(c) => c,
                None => match self.passes.pop_front() {
                    Some(p) => (p, Cursor::Start),
                    None => return Ok(false),
                },
            };
            let mut body = pass.fields.clone();
            body["size"] = json!(self.opts.page_size.clamp(1, 1000));
            body["withExif"] = json!(true);
            body["withPeople"] = json!(true);
            match mode {
                Mode::Filter => {
                    body["orderBy"] = json!({ "field": "fileCreatedAt", "direction": "asc" })
                }
                _ => body["order"] = json!("asc"),
            }
            apply_cursor(&mut body, &cursor);
            let res = self
                .retry(|c| {
                    let b = body.clone();
                    async move { c.post::<SearchResponse>("search/metadata", &b).await }
                })
                .await;
            let page = match res {
                Ok(p) => p,
                Err(e)
                    if pass.optional
                        && matches!(
                            status_of(&e),
                            Some(StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN)
                        ) =>
                {
                    tracing::warn!(pass = pass.label, error = %format!("{e:#}"), "Immich refused this listing; skipping it");
                    continue;
                }
                Err(e) => {
                    return Err(e.context(format!("searching Immich assets ({})", pass.label)));
                }
            };
            let n = page.assets.items.len();
            let next = next_cursor(
                mode,
                &cursor,
                page.assets.next_cursor,
                page.assets.next_page,
                n,
            );
            self.buffer.extend(page.assets.items);
            self.current = Some((pass, next));
            if n > 0 {
                return Ok(true);
            }
        }
    }

    /// Why an asset is not imported, or `None` if it is.
    fn exclusion(&self, a: &Asset) -> Option<&'static str> {
        let me = self.lib.as_ref().map(|l| l.me.as_str());
        if a.is_trashed {
            Some("trashed")
        } else if a.visibility.as_deref() == Some("hidden") || a.is_visible == Some(false) {
            Some("hidden (Live Photo motion part)")
        } else if a.visibility.as_deref() == Some("locked") && !self.opts.include_locked {
            Some("locked")
        } else if matches!((me, a.owner_id.as_deref()), (Some(m), Some(o)) if m != o) {
            Some("owned by another user")
        } else if a.kind != "IMAGE" && a.kind != "VIDEO" {
            Some("not an image or video")
        } else if self.seen.contains(&a.id) {
            Some("already listed")
        } else if a.kind == "VIDEO" && self.motion.contains(&a.id) {
            Some("Live Photo motion part")
        } else {
            None
        }
    }

    /// Download one asset and build its item.
    async fn fetch(&self, a: &Asset) -> anyhow::Result<ImportItem> {
        let dir = self.tmp.path().join(&a.id);
        let orig_dir = dir.join("original");
        tokio::fs::create_dir_all(&orig_dir).await?;
        let name = file_name(&a.original_file_name, &a.id);
        let primary = MediaFile {
            path: orig_dir.join(&name),
            file_name: name.clone(),
        };
        self.download(&format!("assets/{}/original", a.id), &primary.path)
            .await
            .context("downloading original")?;

        let mut kind = if a.kind == "VIDEO" {
            MediaKind::Video
        } else {
            MediaKind::Image
        };
        let mut live_video = None;
        if let (MediaKind::Image, Some(vid)) = (
            kind,
            a.live_photo_video_id.as_deref().filter(|v| !v.is_empty()),
        ) {
            live_video = self.fetch_motion(vid, &name, &dir).await?;
            if live_video.is_some() {
                kind = MediaKind::LivePhoto;
            }
        }

        let thumbnail = if self.opts.thumbnails {
            self.fetch_preview(&a.id, &dir).await
        } else {
            None
        };

        let lib = self.lib.as_ref();
        let mut item = ImportItem::new(a.id.clone(), kind, primary);
        item.live_video = live_video;
        item.thumbnail = thumbnail;
        item.creation_time_us = dates::creation_time_us(a, self.opts.naive_times);
        item.modification_time_us = dates::modification_time_us(a);
        item.location = location(a);
        item.favorite = a.is_favorite;
        item.archived = match a.visibility.as_deref() {
            Some(v) => v == "archive",
            None => a.is_archived.unwrap_or(false),
        };
        item.hidden = a.visibility.as_deref() == Some("locked");
        item.description = a
            .exif_info
            .as_ref()
            .and_then(|e| e.description.as_deref())
            .map(str::trim)
            .filter(|d| !d.is_empty())
            .map(str::to_owned);
        item.albums = lib
            .and_then(|l| l.albums.get(&a.id))
            .cloned()
            .unwrap_or_default();
        item.people = people(a);
        item.dimensions = dimensions(a);
        Ok(item)
    }

    /// Download the motion video of a Live Photo. Returns `None` when the
    /// video is gone, or when it is the video Immich extracted from an
    /// Android motion photo (the still already embeds it; Ente reads it
    /// from there).
    async fn fetch_motion(
        &self,
        vid: &str,
        still_name: &str,
        dir: &Path,
    ) -> anyhow::Result<Option<MediaFile>> {
        let path = format!("assets/{vid}");
        let meta = match self
            .retry(|c| {
                let p = path.clone();
                async move { c.get::<Asset>(&p).await }
            })
            .await
        {
            Ok(m) => Some(m),
            Err(e) if status_of(&e) == Some(StatusCode::NOT_FOUND) => {
                tracing::warn!(
                    video = vid,
                    "Live Photo video is missing in Immich; importing the still only"
                );
                return Ok(None);
            }
            Err(e) => {
                tracing::warn!(video = vid, error = %format!("{e:#}"), "could not read Live Photo video details");
                None
            }
        };
        if let Some(p) = meta.as_ref().and_then(|m| m.original_path.as_deref())
            && is_extracted_motion(p)
        {
            tracing::debug!(
                video = vid,
                "motion photo with embedded video; importing the still as is"
            );
            return Ok(None);
        }
        let stem = Path::new(still_name)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(vid);
        let fallback = format!("{stem}.mov");
        let mut name = meta.as_ref().map_or(fallback.clone(), |m| {
            file_name(&m.original_file_name, &fallback)
        });
        if name == still_name {
            name = format!("motion-{name}");
        }
        let motion_dir = dir.join("motion");
        tokio::fs::create_dir_all(&motion_dir).await?;
        let file = MediaFile {
            path: motion_dir.join(&name),
            file_name: name,
        };
        match self
            .download(&format!("assets/{vid}/original"), &file.path)
            .await
        {
            Ok(()) => Ok(Some(file)),
            Err(e) if status_of(&e) == Some(StatusCode::NOT_FOUND) => {
                tracing::warn!(
                    video = vid,
                    "Live Photo video is missing in Immich; importing the still only"
                );
                Ok(None)
            }
            Err(e) => Err(e.context("downloading Live Photo video")),
        }
    }

    /// Download Immich's preview image. Failures only cost the thumbnail.
    async fn fetch_preview(&self, id: &str, dir: &Path) -> Option<PathBuf> {
        let tmp = dir.join("preview.download");
        let res = self
            .retry(|c| {
                let t = tmp.clone();
                let id = id.to_owned();
                async move {
                    c.download(&format!("assets/{id}/thumbnail?size=preview"), &t)
                        .await
                }
            })
            .await;
        match res {
            Ok(ct) => {
                let ext = match ct
                    .as_deref()
                    .map(|c| c.split(';').next().unwrap_or("").trim())
                {
                    Some("image/webp") => "webp",
                    Some("image/png") => "png",
                    _ => "jpg",
                };
                let dest = dir.join(format!("preview.{ext}"));
                tokio::fs::rename(&tmp, &dest).await.ok()?;
                Some(dest)
            }
            Err(e) => {
                tracing::warn!(asset = id, error = %format!("{e:#}"), "no Immich preview; a thumbnail will be generated");
                let _ = tokio::fs::remove_file(&tmp).await;
                None
            }
        }
    }

    async fn download(&self, path: &str, dest: &Path) -> anyhow::Result<()> {
        self.retry(|c| {
            let p = path.to_owned();
            let d = dest.to_owned();
            async move { c.download(&p, &d).await.map(|_| ()) }
        })
        .await
    }
}

#[async_trait::async_trait]
impl Source for ImmichSource {
    fn name(&self) -> &'static str {
        "immich"
    }

    async fn total(&mut self) -> anyhow::Result<Option<u64>> {
        self.ensure_init().await?;
        if self.lib.as_ref().is_some_and(|l| l.mode == Mode::Legacy) {
            return Ok(None);
        }
        let mut vis = vec!["timeline", "archive"];
        if self.opts.include_locked {
            vis.push("locked");
        }
        let mut total = 0;
        for v in vis {
            let path = format!("assets/statistics?visibility={v}&isTrashed=false");
            match self.client.get::<AssetStats>(&path).await {
                Ok(s) => total += s.total,
                Err(e) if v == "locked" => {
                    tracing::debug!(error = %format!("{e:#}"), "no locked statistics")
                }
                Err(e) => {
                    tracing::debug!(error = %format!("{e:#}"), "no asset statistics");
                    return Ok(None);
                }
            }
        }
        Ok(Some(total))
    }

    async fn next(
        &mut self,
        skip: &(dyn for<'a> Fn(&'a str) -> bool + Sync),
    ) -> anyhow::Result<Option<ImportItem>> {
        self.ensure_init().await?;
        loop {
            let Some(asset) = self.buffer.pop_front() else {
                if self.fill().await? {
                    continue;
                }
                return Ok(None);
            };
            if let Some(why) = self.exclusion(&asset) {
                tracing::debug!(asset = %asset.id, file = %asset.original_file_name, reason = why, "not imported");
                continue;
            }
            self.seen.insert(asset.id.clone());
            if let Some(v) = asset
                .live_photo_video_id
                .as_deref()
                .filter(|v| !v.is_empty())
            {
                self.motion.insert(v.to_owned());
            }
            if skip(&asset.id) {
                continue;
            }
            match self.fetch(&asset).await {
                Ok(item) => return Ok(Some(item)),
                Err(e) => {
                    tracing::error!(asset = %asset.id, file = %asset.original_file_name, error = %format!("{e:#}"), "could not download asset; it will be retried on the next run");
                    let _ = tokio::fs::remove_dir_all(self.tmp.path().join(&asset.id)).await;
                    self.failures.push(asset.id.clone());
                }
            }
        }
    }

    async fn done(&mut self, item: &ImportItem) -> anyhow::Result<()> {
        let dir = self.tmp.path().join(&item.source_id);
        match tokio::fs::remove_dir_all(&dir).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", dir.display())),
        }
    }
}

/// The listings to run for a server mode.
fn passes(mode: Mode, include_locked: bool) -> Vec<Pass> {
    let main = |label, fields| Pass {
        label,
        fields,
        optional: false,
    };
    let mut out = match mode {
        Mode::Filter => vec![main(
            "timeline and archive",
            json!({ "filter": { "visibility": { "in": ["timeline", "archive"] }, "trashedAt": { "eq": null } } }),
        )],
        Mode::Visibility => vec![
            main(
                "timeline",
                json!({ "visibility": "timeline", "withDeleted": false }),
            ),
            main(
                "archive",
                json!({ "visibility": "archive", "withDeleted": false }),
            ),
        ],
        Mode::Legacy => vec![
            main(
                "timeline",
                json!({ "isArchived": false, "isVisible": true, "withDeleted": false }),
            ),
            main(
                "archive",
                json!({ "isArchived": true, "isVisible": true, "withDeleted": false }),
            ),
        ],
    };
    if include_locked && mode != Mode::Legacy {
        let fields = match mode {
            Mode::Filter => {
                json!({ "filter": { "visibility": { "eq": "locked" }, "trashedAt": { "eq": null } } })
            }
            _ => json!({ "visibility": "locked", "withDeleted": false }),
        };
        out.push(Pass {
            label: "locked",
            fields,
            optional: true,
        });
    }
    out
}

/// Put the pagination field into a search body. Returns `false` at the end.
fn apply_cursor(body: &mut Value, cursor: &Cursor) -> bool {
    match cursor {
        Cursor::Start => true,
        Cursor::Token(t) => {
            body["cursor"] = json!(t);
            true
        }
        Cursor::Page(p) => {
            body["page"] = json!(p);
            true
        }
        Cursor::End => false,
    }
}

/// Work out the next page from a search response.
fn next_cursor(
    mode: Mode,
    current: &Cursor,
    token: Option<String>,
    next_page: Option<String>,
    items: usize,
) -> Cursor {
    if items == 0 {
        return Cursor::End;
    }
    match mode {
        Mode::Filter => match token.filter(|t| !t.is_empty()) {
            Some(t) => Cursor::Token(t),
            None => Cursor::End,
        },
        _ => match next_page.and_then(|p| p.trim().parse::<u32>().ok()) {
            Some(p) if !matches!(current, Cursor::Page(c) if *c >= p) => Cursor::Page(p),
            _ => Cursor::End,
        },
    }
}

/// A safe local file name from Immich's `originalFileName`.
fn file_name(original: &str, fallback: &str) -> String {
    let base = original.rsplit(['/', '\\']).next().unwrap_or("");
    let clean: String = base.chars().filter(|c| !c.is_control()).collect();
    let clean = clean.trim();
    if clean.is_empty() || clean == "." || clean == ".." {
        fallback.to_owned()
    } else {
        clean.to_owned()
    }
}

/// True for the path Immich gives the video it extracts from an Android
/// motion photo (`.../encoded-video/<user>/<xx>/<yy>/<uuid>-MP.mp4`).
fn is_extracted_motion(path: &str) -> bool {
    path.contains("encoded-video") && path.ends_with("-MP.mp4")
}

/// GPS position, if valid. `0,0` is treated as missing.
fn location(a: &Asset) -> Option<GeoPoint> {
    let e = a.exif_info.as_ref()?;
    let (lat, lon) = (e.latitude?, e.longitude?);
    let valid = lat.is_finite()
        && lon.is_finite()
        && lat.abs() <= 90.0
        && lon.abs() <= 180.0
        && (lat != 0.0 || lon != 0.0);
    valid.then_some(GeoPoint {
        latitude: lat,
        longitude: lon,
    })
}

/// Named, visible people, in Immich's order without duplicates.
fn people(a: &Asset) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in a.people.iter().flatten() {
        let name = p.name.trim();
        if !p.is_hidden && !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_owned());
        }
    }
    out
}

/// Displayed pixel size: the asset's own (already oriented) size, else the
/// EXIF size with width and height swapped for rotated orientations.
fn dimensions(a: &Asset) -> Option<(u32, u32)> {
    let px = |v: Option<f64>| {
        v.filter(|x| x.is_finite() && *x >= 1.0 && *x <= f64::from(u32::MAX))
            .map(|x| x.round() as u32)
    };
    if let (Some(w), Some(h)) = (px(a.width), px(a.height)) {
        return Some((w, h));
    }
    let e = a.exif_info.as_ref()?;
    let (w, h) = (px(e.exif_image_width)?, px(e.exif_image_height)?);
    let orientation = match &e.orientation {
        Some(Value::String(s)) => s.trim().parse::<u32>().ok(),
        Some(Value::Number(n)) => n.as_u64().map(|n| n as u32),
        _ => None,
    };
    Some(if matches!(orientation, Some(5..=8)) {
        (h, w)
    } else {
        (w, h)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(v: Value) -> Asset {
        serde_json::from_value(v).expect("asset")
    }

    #[test]
    fn modes() {
        let v = |major, minor| ServerVersion {
            major,
            minor,
            patch: 0,
        };
        assert_eq!(Mode::for_version(v(3, 4)), Mode::Filter);
        assert_eq!(Mode::for_version(v(3, 2)), Mode::Filter);
        assert_eq!(Mode::for_version(v(3, 1)), Mode::Visibility);
        assert_eq!(Mode::for_version(v(2, 5)), Mode::Visibility);
        assert_eq!(Mode::for_version(v(1, 133)), Mode::Visibility);
        assert_eq!(Mode::for_version(v(1, 132)), Mode::Legacy);
    }

    #[test]
    fn page_numbers() {
        assert!(matches!(
            next_cursor(Mode::Visibility, &Cursor::Start, None, Some("2".into()), 5),
            Cursor::Page(2)
        ));
        assert!(matches!(
            next_cursor(Mode::Visibility, &Cursor::Page(2), None, None, 5),
            Cursor::End
        ));
        assert!(matches!(
            next_cursor(
                Mode::Visibility,
                &Cursor::Page(2),
                None,
                Some("2".into()),
                5
            ),
            Cursor::End
        ));
        assert!(
            matches!(next_cursor(Mode::Filter, &Cursor::Start, Some("abc".into()), None, 5), Cursor::Token(t) if t == "abc")
        );
        assert!(matches!(
            next_cursor(Mode::Filter, &Cursor::Start, Some("abc".into()), None, 0),
            Cursor::End
        ));
    }

    #[test]
    fn names() {
        assert_eq!(file_name("IMG_1.HEIC", "x"), "IMG_1.HEIC");
        assert_eq!(file_name("../../etc/passwd", "x"), "passwd");
        assert_eq!(file_name("a\\b\\c.jpg", "x"), "c.jpg");
        assert_eq!(file_name("..", "x"), "x");
        assert_eq!(file_name("", "x"), "x");
        assert!(is_extracted_motion(
            "/data/encoded-video/u/ab/cd/0f0e-MP.mp4"
        ));
        assert!(!is_extracted_motion("/data/library/u/2024/IMG_1.MOV"));
    }

    #[test]
    fn dims_and_location() {
        let a = asset(
            json!({"id": "a", "width": 3024, "height": 4032, "exifInfo": {"exifImageWidth": 4032, "exifImageHeight": 3024}}),
        );
        assert_eq!(dimensions(&a), Some((3024, 4032)));
        let a = asset(
            json!({"id": "a", "exifInfo": {"exifImageWidth": 4032, "exifImageHeight": 3024, "orientation": "6"}}),
        );
        assert_eq!(dimensions(&a), Some((3024, 4032)));
        let a = asset(
            json!({"id": "a", "exifInfo": {"exifImageWidth": 4032, "exifImageHeight": 3024, "orientation": 1}}),
        );
        assert_eq!(dimensions(&a), Some((4032, 3024)));
        let a = asset(json!({"id": "a", "exifInfo": {"latitude": 0.0, "longitude": 0.0}}));
        assert_eq!(location(&a), None);
        let a = asset(json!({"id": "a", "exifInfo": {"latitude": 91.0, "longitude": 0.5}}));
        assert_eq!(location(&a), None);
        let a = asset(json!({"id": "a", "exifInfo": {"latitude": 52.5, "longitude": 13.4}}));
        assert_eq!(
            location(&a),
            Some(GeoPoint {
                latitude: 52.5,
                longitude: 13.4
            })
        );
    }

    #[test]
    fn people_names_only() {
        let a = asset(json!({"id": "a", "people": [
            {"id": "1", "name": "Ada", "isHidden": false},
            {"id": "2", "name": "", "isHidden": false},
            {"id": "3", "name": "Secret", "isHidden": true},
            {"id": "4", "name": " Ada ", "isHidden": false},
            {"id": "5", "name": "Bob", "isHidden": false}
        ]}));
        assert_eq!(people(&a), vec!["Ada", "Bob"]);
    }
}
