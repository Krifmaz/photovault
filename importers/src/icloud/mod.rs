//! iCloud Photos export source.
//!
//! Reads the copy of iCloud Photos that Apple delivers through
//! <https://privacy.apple.com> ("Request a copy of your data" → iCloud
//! Photos). The export arrives as one or more zips named
//! `iCloud Photos Part N of M.zip`; each holds a folder
//! `iCloud Photos Part N of M/` with:
//!
//! - `Photos/`: the original files (`IMG_0001.HEIC`, the Live Photo motion
//!   part as `IMG_0001.MOV`, …) and one or more `Photo Details*.csv` files
//!   with the columns `imgName, fileChecksum, favorite, hidden, deleted,
//!   originalCreationDate, viewCount, importDate`. Duplicate names inside one
//!   folder get a `-1`, `-2` … suffix on disk but not in the CSV.
//! - `Albums/<album name>.csv`: one column (`imgName`) listing members.
//! - `Memories/<memory name>.csv`: same shape as albums.
//! - `Shared Albums/<album name>/`: reduced-quality copies plus an
//!   `AlbumInfo.json` (`albumName`, `photos[{name, dateCreated}]`); file
//!   names on disk may be truncated.
//!
//! A Live Photo's still and video can be in different parts. This source
//! reads the zips in place (only the central directory up front; one entry
//! at a time to a temp file when its item is next), so a 500 GB export does
//! not need 500 GB of scratch space.
//!
//! # Mapping
//! | Export                              | Ente                                    |
//! |-------------------------------------|-----------------------------------------|
//! | still + `.MOV` with the same name    | Live Photo (`fileType` 2)               |
//! | `Albums/*.csv`, `Shared Albums/*/`  | albums of the same name                 |
//! | `Memories/*.csv`                     | albums `Memory: <name>` (opt-in)        |
//! | `favorite = yes`                     | favourite                               |
//! | `hidden = yes`                       | Ente's hidden album                     |
//! | `deleted = yes`, `Recently Deleted/` | skipped                                 |
//! | `originalCreationDate`              | creation time (EXIF if it agrees ±60 s, else EXIF, else file time) |
//! | EXIF / QuickTime GPS                 | location                                |
//! | EXIF `ImageDescription`, shared-album caption | description                    |
//!
//! # Edited versions
//! Apple's privacy export ships the original of every asset. If an export
//! (or a folder mixed in from another tool such as iCloud for Windows) also
//! carries Apple's rendered edits, they are named `IMG_E1234.*` next to
//! `IMG_1234.*`. We import the original and skip the edited render by
//! default, because Ente keeps one file per asset and the original is the
//! lossless one; set [`ICloudOptions::include_edited`] to import the edits as
//! separate files too.

mod archive;
mod metadata;

pub use metadata::parse_icloud_date;

use crate::item::{ImportItem, MediaFile, MediaKind, Source};
use crate::media;
use archive::{Container, Entry};
use metadata::PhotoRow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Largest CSV/JSON metadata file we read (they are small; this guards
/// against a mislabelled huge file).
const MAX_METADATA_BYTES: u64 = 256 * 1024 * 1024;

/// Two halves of a Live Photo must be dated within this of each other
/// (Ente's threshold in `areLivePhotoAssets`).
const LIVE_PHOTO_MAX_SKEW_US: i64 = 24 * 60 * 60 * 1_000_000;

/// EXIF replaces the CSV date (minute precision) when both agree this well.
const EXIF_AGREEMENT_US: i64 = 60 * 1_000_000;

/// Options for [`ICloudSource::open`].
#[derive(Debug, Clone)]
pub struct ICloudOptions {
    /// Pair stills and videos with the same name into Live Photos (default `true`).
    pub live_photos: bool,
    /// Import `Shared Albums/*` as albums (default `true`). Shared album
    /// copies are reduced-quality duplicates of library items, so turn this
    /// off if you only want originals.
    pub shared_albums: bool,
    /// Turn each `Memories/*.csv` into an album `Memory: <name>` (default `false`).
    pub memories_as_albums: bool,
    /// Also import Apple's edited renders (`IMG_E1234.*`) as separate files
    /// (default `false`: originals only).
    pub include_edited: bool,
    /// Where to put extracted temp files (default: the system temp dir).
    pub temp_dir: Option<PathBuf>,
}

impl Default for ICloudOptions {
    fn default() -> Self {
        Self {
            live_photos: true,
            shared_albums: true,
            memories_as_albums: false,
            include_edited: false,
            temp_dir: None,
        }
    }
}

/// Counts from planning, for logs and tests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ICloudStats {
    /// Media files found.
    pub media_files: usize,
    /// Rows read from `Photo Details` CSVs.
    pub detail_rows: usize,
    /// Files skipped because they are in Recently Deleted.
    pub deleted_skipped: usize,
    /// Edited renders skipped.
    pub edited_skipped: usize,
    /// Live Photos formed.
    pub live_photos: usize,
    /// Media files with no `Photo Details` row.
    pub without_details: usize,
    /// Empty (zero-byte) media files skipped, as the Ente apps do.
    pub empty_skipped: usize,
}

/// One item as planned from the export's listing and CSVs.
#[derive(Debug, Clone)]
struct Planned {
    source_id: String,
    kind: MediaKind,
    primary: usize,
    primary_name: String,
    video: Option<usize>,
    video_name: Option<String>,
    row: Option<PhotoRow>,
    video_row: Option<PhotoRow>,
    albums: Vec<String>,
    shared_date_us: Option<i64>,
    caption: Option<String>,
}

/// A [`Source`] over an iCloud Photos export (zips and/or extracted folders).
pub struct ICloudSource {
    containers: Vec<Container>,
    entries: Vec<Entry>,
    planned: std::collections::VecDeque<Planned>,
    total: usize,
    stats: ICloudStats,
    temp_root: tempfile::TempDir,
    temp_dirs: HashMap<String, PathBuf>,
    seq: u64,
}

impl ICloudSource {
    /// Open an export given as any mix of `iCloud Photos Part N of M.zip`
    /// files and directories (an extracted part, a folder holding several
    /// extracted parts, or a folder of the zips' contents merged).
    ///
    /// Reads the zips' central directories and all metadata CSV/JSON files,
    /// and plans every item; no media is extracted yet.
    pub fn open(inputs: &[PathBuf], options: ICloudOptions) -> anyhow::Result<Self> {
        anyhow::ensure!(!inputs.is_empty(), "no iCloud export given");
        let (containers, entries) = archive::open_all(inputs)?;
        let (planned, stats) = plan(&containers, &entries, &options)?;
        tracing::info!(?stats, items = planned.len(), "iCloud export indexed");
        let mut tmp = tempfile::Builder::new();
        tmp.prefix("photovault-icloud-");
        let temp_root = match &options.temp_dir {
            Some(d) => tmp.tempdir_in(d)?,
            None => tmp.tempdir()?,
        };
        Ok(Self {
            containers,
            entries,
            total: planned.len(),
            planned: planned.into(),
            stats,
            temp_root,
            temp_dirs: HashMap::new(),
            seq: 0,
        })
    }

    /// What planning found.
    pub fn stats(&self) -> &ICloudStats {
        &self.stats
    }
}

#[async_trait::async_trait]
impl Source for ICloudSource {
    fn name(&self) -> &'static str {
        "icloud"
    }

    async fn total(&mut self) -> anyhow::Result<Option<u64>> {
        Ok(Some(self.total as u64))
    }

    async fn next(
        &mut self,
        skip: &(dyn for<'a> Fn(&'a str) -> bool + Sync),
    ) -> anyhow::Result<Option<ImportItem>> {
        while let Some(p) = self.planned.pop_front() {
            if skip(&p.source_id) {
                continue;
            }
            self.seq += 1;
            let dest = self.temp_root.path().join(self.seq.to_string());
            let primary = self.entries[p.primary].clone();
            let video = p.video.map(|v| self.entries[v].clone());
            let c_primary = self.containers[primary.container].clone();
            let c_video = video.as_ref().map(|v| self.containers[v.container].clone());
            let dest2 = dest.clone();
            let p2 = p.clone();
            let built = tokio::task::spawn_blocking(move || {
                build_item(
                    &p2,
                    &primary,
                    &c_primary,
                    video.as_ref().zip(c_video.as_ref()),
                    &dest2,
                )
            })
            .await?;
            match built {
                Ok(item) => {
                    self.temp_dirs.insert(item.source_id.clone(), dest);
                    return Ok(Some(item));
                }
                Err(e) => {
                    // A damaged entry (e.g. a truncated part) must not stop the
                    // whole import; it is retried on the next run.
                    let _ = std::fs::remove_dir_all(&dest);
                    tracing::error!(source_id = %p.source_id, file = %p.primary_name,
                        error = %format!("{e:#}"), "cannot read item from iCloud export; skipping");
                }
            }
        }
        Ok(None)
    }

    async fn done(&mut self, item: &ImportItem) -> anyhow::Result<()> {
        if let Some(dir) = self.temp_dirs.remove(&item.source_id) {
            match tokio::fs::remove_dir_all(&dir).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

/// Extract (if needed) and read one planned item. Blocking.
fn build_item(
    p: &Planned,
    primary: &Entry,
    c_primary: &Container,
    video: Option<(&Entry, &Container)>,
    dest: &Path,
) -> anyhow::Result<ImportItem> {
    let (primary_path, _) = archive::materialize(c_primary, primary, &dest.join("p"))?;
    let mut item = ImportItem::new(
        p.source_id.clone(),
        p.kind,
        MediaFile {
            path: primary_path.clone(),
            file_name: p.primary_name.clone(),
        },
    );
    let info = media::read_embedded_info(&primary_path);
    let mut video_info = None;
    if let Some((ve, vc)) = video {
        let (vpath, _) = archive::materialize(vc, ve, &dest.join("v"))?;
        video_info = Some(media::read_embedded_info(&vpath));
        item.live_video = Some(MediaFile {
            path: vpath,
            file_name: p
                .video_name
                .clone()
                .unwrap_or_else(|| ve.file_name().to_string()),
        });
    }

    let csv_date = p
        .row
        .as_ref()
        .and_then(|r| r.original_creation_us)
        .or_else(|| p.video_row.as_ref().and_then(|r| r.original_creation_us));
    let exif_date = info.creation_time_us;
    item.creation_time_us = match (csv_date, exif_date) {
        (Some(c), Some(e)) if (c - e).abs() < EXIF_AGREEMENT_US => Some(e),
        (Some(c), _) => Some(c),
        (None, Some(e)) => Some(e),
        (None, None) => video_info
            .as_ref()
            .and_then(|v| v.creation_time_us)
            .or(p.shared_date_us)
            .or(primary.mtime_us),
    };
    item.modification_time_us = primary.mtime_us.or(item.creation_time_us);
    item.location = info
        .location
        .or_else(|| video_info.as_ref().and_then(|v| v.location));
    item.dimensions = info.dimensions;
    let rows = [p.row.as_ref(), p.video_row.as_ref()];
    item.favorite = rows.iter().flatten().any(|r| r.favorite);
    item.hidden = rows.iter().flatten().any(|r| r.hidden);
    item.albums = p.albums.clone();
    item.description = p.caption.clone().or_else(|| {
        (p.kind != MediaKind::Video)
            .then(|| media::read_description(&primary_path))
            .flatten()
    });
    Ok(item)
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

fn has_component(logical: &str, name: &str) -> bool {
    logical.split('/').any(|c| c.eq_ignore_ascii_case(name))
}

/// For `Shared Albums/<album>/…/file`, the album folder name.
fn shared_album_folder(logical: &str) -> Option<&str> {
    let parts: Vec<&str> = logical.split('/').collect();
    let i = parts.iter().position(|c| {
        c.eq_ignore_ascii_case("Shared Albums") || c.eq_ignore_ascii_case("iCloud Shared Albums")
    })?;
    (parts.len() > i + 2).then(|| parts[i + 1])
}

fn file_stem(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

fn file_ext(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[i + 1..],
        _ => "",
    }
}

/// `IMG_0001-2.JPG` → `Some(("IMG_0001.JPG", 2))`: Apple's on-disk
/// de-duplication suffix.
fn dedup_suffix(name: &str) -> Option<(String, usize)> {
    let stem = file_stem(name);
    let (base, n) = stem.rsplit_once('-')?;
    if base.is_empty() || n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let n: usize = n.parse().ok()?;
    let ext = file_ext(name);
    Some((
        if ext.is_empty() {
            base.to_string()
        } else {
            format!("{base}.{ext}")
        },
        n,
    ))
}

/// Ente's `removePotentialLivePhotoSuffix`: the name both halves share.
fn live_photo_key(name: &str, kind: MediaKind) -> String {
    let mut stem = file_stem(name).to_string();
    for suffix in ["_3", "_HVEC", "_HEVC"] {
        if stem.len() > suffix.len() && stem.to_ascii_uppercase().ends_with(suffix) {
            stem.truncate(stem.len() - suffix.len());
            return stem.to_ascii_lowercase();
        }
    }
    if kind == MediaKind::Video {
        // `IMG_1.HEIC.MOV` pairs with `IMG_1.HEIC`.
        let inner_ext = file_ext(&stem).to_ascii_lowercase();
        if !inner_ext.is_empty()
            && media::detect_kind(Path::new(&format!("x.{inner_ext}"))) == Some(MediaKind::Image)
        {
            stem = file_stem(&stem).to_string();
        }
    }
    stem.to_ascii_lowercase()
}

/// `IMG_E1234` → `IMG_1234`: Apple's name for an edited render.
fn edited_original_stem(stem: &str) -> Option<String> {
    let lower = stem.to_ascii_lowercase();
    let i = lower.rfind("_e")?;
    let digits = &stem[i + 2..];
    (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()))
        .then(|| format!("{}_{}", &stem[..i], digits).to_ascii_lowercase())
}

/// Live Photo grouping key: (logical folder, from a shared album, pruned name).
type LiveKey = (String, bool, String);

struct MediaEntry {
    idx: usize,
    kind: MediaKind,
    name: String,
    row: Option<PhotoRow>,
    albums: Vec<String>,
    shared_date_us: Option<i64>,
    caption: Option<String>,
    shared: bool,
}

fn plan(
    containers: &[Container],
    entries: &[Entry],
    opts: &ICloudOptions,
) -> anyhow::Result<(Vec<Planned>, ICloudStats)> {
    let mut stats = ICloudStats::default();

    // 1. Classify.
    let mut details = Vec::new();
    let mut album_csvs = Vec::new();
    let mut memory_csvs = Vec::new();
    let mut album_infos: HashMap<String, usize> = HashMap::new();
    let mut media_idx = Vec::new();
    for (i, e) in entries.iter().enumerate() {
        let name = e.file_name();
        if has_component(&e.logical, "Recently Deleted") {
            stats.deleted_skipped += 1;
            continue;
        }
        let lower = name.to_ascii_lowercase();
        if let Some(folder) = shared_album_folder(&e.logical)
            && lower == "albuminfo.json"
        {
            album_infos.insert(format!("{}:{folder}", e.container), i);
            continue;
        }
        if lower.ends_with(".csv") {
            let norm = lower.replace(['_', '-'], " ");
            if norm.starts_with("photo details") {
                details.push(i);
            } else if has_component(e.logical_dir(), "Albums") {
                album_csvs.push((i, file_stem(name).to_string()));
            } else if has_component(e.logical_dir(), "Memories") {
                memory_csvs.push((i, file_stem(name).to_string()));
            }
            continue;
        }
        if let Some(kind) = media::detect_kind(Path::new(name)) {
            // Ente never uploads empty files (and Apple's export has a few
            // placeholders for assets it could not fetch).
            if e.size == 0 {
                stats.empty_skipped += 1;
                tracing::warn!(file = %e.inner, "skipping empty file");
                continue;
            }
            media_idx.push((i, kind));
        }
    }
    stats.media_files = media_idx.len();

    // 2. Photo Details rows, per physical folder, plus a global name index.
    let mut by_scope: HashMap<String, Vec<PhotoRow>> = HashMap::new();
    for &i in &details {
        let data = archive::read_small(containers, &entries[i], MAX_METADATA_BYTES)?;
        let rows = metadata::parse_photo_details(&data).unwrap_or_else(|e| {
            tracing::warn!(file = %entries[i].inner, error = %e, "unreadable Photo Details CSV");
            Vec::new()
        });
        stats.detail_rows += rows.len();
        by_scope
            .entry(entries[i].physical_dir())
            .or_default()
            .extend(rows);
    }
    let mut global: HashMap<String, Vec<PhotoRow>> = HashMap::new();
    for rows in by_scope.values() {
        for r in rows {
            global
                .entry(r.img_name.to_lowercase())
                .or_default()
                .push(r.clone());
        }
    }

    // 3. Album and memory membership, by file name.
    let mut membership: HashMap<String, Vec<String>> = HashMap::new();
    let mut add_members = |i: usize, album: String| -> anyhow::Result<()> {
        let data = archive::read_small(containers, &entries[i], MAX_METADATA_BYTES)?;
        let names = metadata::parse_member_list(&data).unwrap_or_default();
        for n in names {
            let v = membership.entry(n.to_lowercase()).or_default();
            if !v.contains(&album) {
                v.push(album.clone());
            }
        }
        Ok(())
    };
    for (i, name) in album_csvs {
        add_members(i, name)?;
    }
    if opts.memories_as_albums {
        for (i, name) in memory_csvs {
            add_members(i, format!("Memory: {name}"))?;
        }
    }

    // 4. Shared album info.
    let mut shared_infos: HashMap<String, metadata::SharedAlbumInfo> = HashMap::new();
    for (key, i) in album_infos {
        let data = archive::read_small(containers, &entries[i], MAX_METADATA_BYTES)?;
        match metadata::parse_album_info(&data) {
            Ok(info) => {
                shared_infos.insert(key, info);
            }
            Err(e) => {
                tracing::warn!(file = %entries[i].inner, error = %e, "unreadable AlbumInfo.json")
            }
        }
    }

    // 5. Match media to rows.
    let mut media_entries = Vec::new();
    for (i, kind) in media_idx {
        let e = &entries[i];
        let disk_name = e.file_name().to_string();
        if let Some(folder) = shared_album_folder(&e.logical) {
            if !opts.shared_albums {
                continue;
            }
            let info = shared_infos.get(&format!("{}:{folder}", e.container));
            let album = info
                .and_then(|i| i.name.clone())
                .unwrap_or_else(|| folder.to_string());
            let photo = info.and_then(|i| match_shared_photo(&i.photos, &disk_name));
            media_entries.push(MediaEntry {
                idx: i,
                kind,
                name: photo.map(|p| p.name.clone()).unwrap_or(disk_name),
                row: None,
                albums: vec![album],
                shared_date_us: photo.and_then(|p| p.date_us),
                caption: photo.and_then(|p| p.caption.clone()),
                shared: true,
            });
            continue;
        }
        let row = match_row(by_scope.get(&e.physical_dir()), &global, &disk_name);
        if row.is_none() {
            stats.without_details += 1;
        }
        if row.as_ref().is_some_and(|r| r.deleted) {
            stats.deleted_skipped += 1;
            continue;
        }
        let name = row
            .as_ref()
            .map(|r| r.img_name.clone())
            .unwrap_or(disk_name);
        let albums = membership
            .get(&name.to_lowercase())
            .cloned()
            .unwrap_or_default();
        media_entries.push(MediaEntry {
            idx: i,
            kind,
            name,
            row,
            albums,
            shared_date_us: None,
            caption: None,
            shared: false,
        });
    }

    // 6. Edited renders.
    if !opts.include_edited {
        let stems: HashSet<(String, String)> = media_entries
            .iter()
            .map(|m| {
                (
                    entries[m.idx].logical_dir().to_lowercase(),
                    file_stem(&m.name).to_lowercase(),
                )
            })
            .collect();
        let before = media_entries.len();
        media_entries.retain(|m| {
            let dir = entries[m.idx].logical_dir().to_lowercase();
            !edited_original_stem(file_stem(&m.name))
                .is_some_and(|orig| stems.contains(&(dir, orig)))
        });
        stats.edited_skipped = before - media_entries.len();
    }

    // 7. Live Photos.
    let mut pair_of: HashMap<usize, usize> = HashMap::new(); // image pos -> video pos
    let mut paired_video: HashSet<usize> = HashSet::new();
    if opts.live_photos {
        let mut groups: HashMap<LiveKey, (Vec<usize>, Vec<usize>)> = HashMap::new();
        for (pos, m) in media_entries.iter().enumerate() {
            let key = (
                entries[m.idx].logical_dir().to_lowercase(),
                m.shared,
                live_photo_key(&m.name, m.kind),
            );
            let g = groups.entry(key).or_default();
            match m.kind {
                MediaKind::Video => g.1.push(pos),
                _ => g.0.push(pos),
            }
        }
        for (_, (imgs, vids)) in groups {
            if let ([img], [vid]) = (&imgs[..], &vids[..]) {
                let date = |m: &MediaEntry| {
                    m.row
                        .as_ref()
                        .and_then(|r| r.original_creation_us)
                        .or(m.shared_date_us)
                };
                let close = match (date(&media_entries[*img]), date(&media_entries[*vid])) {
                    (Some(a), Some(b)) => (a - b).abs() < LIVE_PHOTO_MAX_SKEW_US,
                    _ => true,
                };
                if close {
                    pair_of.insert(*img, *vid);
                    paired_video.insert(*vid);
                }
            }
        }
        stats.live_photos = pair_of.len();
    }

    // 8. Planned items, in export order.
    let mut used_ids = HashSet::new();
    let mut planned = Vec::new();
    for (pos, m) in media_entries.iter().enumerate() {
        if paired_video.contains(&pos) {
            continue;
        }
        let e = &entries[m.idx];
        let video = pair_of.get(&pos).map(|&v| &media_entries[v]);
        let mut id = if m.shared {
            format!("shared:{}", e.logical)
        } else if let Some(ck) = m.row.as_ref().and_then(|r| r.checksum.clone()) {
            format!("ck:{ck}")
        } else {
            format!("path:{}", e.logical)
        };
        if used_ids.contains(&id) {
            id = format!("path:{}", e.inner);
            let base = id.clone();
            let mut n = 2;
            while used_ids.contains(&id) {
                id = format!("{base}#{n}");
                n += 1;
            }
        }
        used_ids.insert(id.clone());
        let mut albums = m.albums.clone();
        if let Some(v) = video {
            for a in &v.albums {
                if !albums.contains(a) {
                    albums.push(a.clone());
                }
            }
        }
        planned.push(Planned {
            source_id: id,
            kind: if video.is_some() {
                MediaKind::LivePhoto
            } else {
                m.kind
            },
            primary: m.idx,
            primary_name: m.name.clone(),
            video: video.map(|v| v.idx),
            video_name: video.map(|v| v.name.clone()),
            row: m.row.clone(),
            video_row: video.and_then(|v| v.row.clone()),
            albums,
            shared_date_us: m.shared_date_us,
            caption: m.caption.clone(),
        });
    }
    Ok((planned, stats))
}

/// Find the `Photo Details` row for a file on disk: by exact name in its own
/// folder (the n-th `-n` duplicate maps to the (n+1)-th row of that name),
/// then by name anywhere in the export if that is unambiguous.
fn match_row(
    scope: Option<&Vec<PhotoRow>>,
    global: &HashMap<String, Vec<PhotoRow>>,
    disk_name: &str,
) -> Option<PhotoRow> {
    if let Some(rows) = scope {
        let nth = |name: &str, n: usize| {
            rows.iter()
                .filter(|r| r.img_name == name)
                .nth(n)
                .or_else(|| {
                    rows.iter()
                        .filter(|r| r.img_name.eq_ignore_ascii_case(name))
                        .nth(n)
                })
                .cloned()
        };
        if let Some(r) = nth(disk_name, 0) {
            return Some(r);
        }
        if let Some((base, n)) = dedup_suffix(disk_name)
            && let Some(r) = nth(&base, n)
        {
            return Some(r);
        }
    }
    match global.get(&disk_name.to_lowercase()).map(Vec::as_slice) {
        Some([r]) => Some(r.clone()),
        _ => None,
    }
}

/// Match a shared-album file to its `AlbumInfo.json` entry: exact name, or
/// the full name whose stem starts with the (truncated) stem on disk.
fn match_shared_photo<'a>(
    photos: &'a [metadata::SharedPhoto],
    disk_name: &str,
) -> Option<&'a metadata::SharedPhoto> {
    if let Some(p) = photos
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(disk_name))
    {
        return Some(p);
    }
    let (name, _) = dedup_suffix(disk_name).unwrap_or((disk_name.to_string(), 0));
    let stem = file_stem(&name).to_lowercase();
    let ext = file_ext(&name).to_lowercase();
    let mut candidates = photos.iter().filter(|p| {
        file_ext(&p.name).eq_ignore_ascii_case(&ext)
            && file_stem(&p.name).to_lowercase().starts_with(&stem)
    });
    match (candidates.next(), candidates.next()) {
        (Some(p), None) => Some(p),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            dedup_suffix("IMG_0001-1.JPG"),
            Some(("IMG_0001.JPG".into(), 1))
        );
        assert_eq!(dedup_suffix("IMG_0001.JPG"), None);
        assert_eq!(
            live_photo_key("IMG_0001.HEIC", MediaKind::Image),
            "img_0001"
        );
        assert_eq!(live_photo_key("IMG_0001.MOV", MediaKind::Video), "img_0001");
        assert_eq!(
            live_photo_key("IMG_0001.HEIC.MOV", MediaKind::Video),
            "img_0001"
        );
        assert_eq!(
            live_photo_key("IMG_0001_HVEC.MOV", MediaKind::Video),
            "img_0001"
        );
        assert_eq!(
            live_photo_key("IMG_0001_3.MOV", MediaKind::Video),
            "img_0001"
        );
        assert_eq!(
            edited_original_stem("IMG_E1234").as_deref(),
            Some("img_1234")
        );
        assert_eq!(edited_original_stem("IMG_1234"), None);
        assert_eq!(
            shared_album_folder("Shared Albums/Family/IMG_1.JPG"),
            Some("Family")
        );
        assert_eq!(shared_album_folder("Shared Albums/AlbumInfo.json"), None);
    }
}
