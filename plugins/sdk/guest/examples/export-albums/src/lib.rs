//! Export albums: a Photovault example plugin that keeps decrypted copies of
//! chosen albums in a folder the user picked.
//!
//! The plugin never sees file bytes or real paths. It asks the host to copy
//! each file with `export_file(file_id, "Album/2024/IMG_0001.HEIC")`; the
//! host decrypts the file and writes it under the export folder the user chose
//! for this plugin. The sandbox has no file system access at all.
//!
//! # Paths
//!
//! `<album>/<file>`, `<album>/<year>/<file>` or `<album>/<year-month>/<file>`,
//! with the year and month of the capture time (UTC). Every component is
//! cleaned with [`sanitize_component`]. Names that collide (two albums or two
//! files that clean to the same name, compared case-insensitively because
//! macOS and Windows file systems are) get ` (2)`, ` (3)`, ... before the
//! extension, in album-id and file-id order, so the result is deterministic.
//!
//! # Incremental runs
//!
//! What was exported is remembered in the plugin's key/value storage
//! ([`STATE_INDEX`] and one [`album_key`] per album), so each run only copies
//! new files and, with "Update changed files" on, files whose modification
//! time changed. An album keeps its folder even if it is renamed, and an
//! exported file keeps its path even if the layout changes. Files removed from
//! an album are not deleted from the export folder.
//!
//! Photovault is based on Ente (ente.com).

#![warn(missing_docs)]

use photovault_plugin_pdk::path::{sanitize_component, split_extension};
use photovault_plugin_pdk::prelude::*;
use photovault_plugin_pdk::time::date_from_micros;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// KV key of the album → folder index.
pub const STATE_INDEX: &str = "albums";

/// KV key of one album's export record.
pub fn album_key(album_id: AlbumId) -> String {
    format!("album:{album_id}")
}

/// Save the album record after this many copies, so a run cut short by the
/// time limit does not repeat work.
pub const SAVE_EVERY: u64 = 100;

/// Folder layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Layout {
    /// `album/file`.
    #[serde(rename = "album/file")]
    Flat,
    /// `album/year/file`.
    #[serde(rename = "album/year/file")]
    Year,
    /// `album/year-month/file`.
    #[serde(rename = "album/year-month/file")]
    YearMonth,
}

/// The plugin's settings, as declared in `plugin.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    /// Album ids to export.
    pub albums: Vec<AlbumId>,
    /// Folder layout.
    pub layout: Layout,
    /// Re-copy files that changed.
    pub overwrite: bool,
}

/// One exported file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Path inside the album folder.
    #[serde(rename = "p")]
    pub path: String,
    /// The file's modification time when it was copied.
    #[serde(rename = "m")]
    pub modified: i64,
}

/// What was exported from one album.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumState {
    /// Exported files by id.
    pub files: BTreeMap<FileId, Entry>,
}

/// Folder (one sanitised component) of each album ever exported.
pub type Index = BTreeMap<AlbumId, String>;

/// Add ` (n)` before the extension.
fn numbered(name: &str, n: u32) -> String {
    let (stem, ext) = split_extension(name);
    format!("{stem} ({n}){ext}")
}

/// The first of `name`, `name (2)`, `name (3)`, ... whose lowercase form is
/// not in `taken`, built by `make`.
fn first_free(taken: &BTreeSet<String>, mut make: impl FnMut(Option<u32>) -> String) -> String {
    let first = make(None);
    if !taken.contains(&first.to_lowercase()) {
        return first;
    }
    (2..)
        .map(|n| make(Some(n)))
        .find(|c| !taken.contains(&c.to_lowercase()))
        .expect("an unbounded range always has a free name")
}

/// The folder for `album`, reusing the recorded one or assigning a new,
/// unique one.
pub fn album_folder(index: &mut Index, album: &AlbumInfo) -> String {
    if let Some(f) = index.get(&album.id) {
        return f.clone();
    }
    let taken: BTreeSet<String> = index.values().map(|f| f.to_lowercase()).collect();
    let base = sanitize_component(&album.name);
    let folder = first_free(&taken, |n| match n {
        None => base.clone(),
        Some(n) => format!("{base} ({n})"),
    });
    index.insert(album.id, folder.clone());
    folder
}

/// The sub-folder for `file` under `layout`, if any.
pub fn subdir(layout: Layout, file: &FileInfo) -> Option<String> {
    let d = date_from_micros(file.creation_time);
    match layout {
        Layout::Flat => None,
        Layout::Year => Some(format!("{:04}", d.year)),
        Layout::YearMonth => Some(format!("{:04}-{:02}", d.year, d.month)),
    }
}

/// A new, unique path (inside the album folder) for `file`, given the lowercase
/// paths already used in this album.
pub fn file_path(layout: Layout, file: &FileInfo, taken: &BTreeSet<String>) -> String {
    let name = if file.title.trim().is_empty() {
        format!("file-{}", file.id)
    } else {
        sanitize_component(&file.title)
    };
    let dir = subdir(layout, file);
    first_free(taken, |n| {
        let name = match n {
            None => name.clone(),
            Some(n) => numbered(&name, n),
        };
        match &dir {
            Some(d) => format!("{d}/{name}"),
            None => name,
        }
    })
}

/// Counts for one run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    /// Albums exported.
    pub albums: u64,
    /// Files copied.
    pub written: u64,
    /// Files already up to date.
    pub unchanged: u64,
    /// Files found already present in the export folder.
    pub existing: u64,
    /// Bytes copied.
    pub bytes: u64,
    /// Selected albums that no longer exist.
    pub missing_albums: u64,
}

/// Export every selected album.
pub fn export(settings: &Settings) -> Result<Stats> {
    let mut stats = Stats::default();
    if settings.albums.is_empty() {
        return Ok(stats);
    }
    let all: BTreeMap<AlbumId, AlbumInfo> = pdk::host::list_albums()?
        .into_iter()
        .map(|a| (a.id, a))
        .collect();
    let mut selected: Vec<&AlbumInfo> = Vec::new();
    for id in settings.albums.iter().copied().collect::<BTreeSet<_>>() {
        match all.get(&id) {
            Some(a) => selected.push(a),
            None => {
                pdk::warn!("album {id} no longer exists; skipping it");
                stats.missing_albums += 1;
            }
        }
    }
    let total: Option<u64> = selected.iter().map(|a| a.file_count).sum();
    let mut index: Index = pdk::host::kv_get(STATE_INDEX)?.unwrap_or_default();
    let mut done = 0u64;

    for album in selected {
        let before = index.len();
        let folder = album_folder(&mut index, album);
        if index.len() != before {
            pdk::host::kv_set(STATE_INDEX, &index).context("could not save export state")?;
        }
        export_album(settings, album, &folder, &mut stats, &mut done, total)?;
        stats.albums += 1;
    }
    pdk::host::progress(done, Some(done), Some("Done"))?;
    Ok(stats)
}

fn export_album(
    settings: &Settings,
    album: &AlbumInfo,
    folder: &str,
    stats: &mut Stats,
    done: &mut u64,
    total: Option<u64>,
) -> Result<()> {
    let key = album_key(album.id);
    let mut state: AlbumState = pdk::host::kv_get(&key)?.unwrap_or_default();
    let mut taken: BTreeSet<String> = state
        .files
        .values()
        .map(|e| e.path.to_lowercase())
        .collect();
    let mut dirty = 0u64;
    let save =
        |state: &AlbumState| pdk::host::kv_set(&key, state).context("could not save export state");

    let query = FileQuery {
        album_id: Some(album.id),
        ..Default::default()
    };
    // Sort by id so name collisions resolve the same way on every device.
    let mut files: Vec<FileInfo> = pdk::host::files(query).collect::<Result<_, _>>()?;
    files.sort_by_key(|f| f.id);

    for file in files {
        *done += 1;
        let (path, overwrite) = match state.files.get(&file.id) {
            Some(e) if e.modified == file.modification_time || !settings.overwrite => {
                stats.unchanged += 1;
                continue;
            }
            Some(e) => (e.path.clone(), true),
            None => {
                let p = file_path(settings.layout, &file, &taken);
                taken.insert(p.to_lowercase());
                (p, false)
            }
        };
        let result = pdk::host::export_file(file.id, &format!("{folder}/{path}"), overwrite)
            .with_context(|| format!("could not export {}", file.title))?;
        if result.written {
            stats.written += 1;
            stats.bytes += result.bytes;
        } else {
            // Something is already there (an earlier run whose record was
            // lost, or the user's own copy); leave it and remember it.
            stats.existing += 1;
        }
        state.files.insert(
            file.id,
            Entry {
                path,
                modified: file.modification_time,
            },
        );
        dirty += 1;
        if dirty.is_multiple_of(SAVE_EVERY) {
            save(&state)?;
            pdk::host::progress(*done, total, Some(&format!("Exporting {}", album.name)))?;
        }
    }
    if dirty > 0 {
        save(&state)?;
    }
    Ok(())
}

/// One line for the activity log.
pub fn summary(stats: &Stats) -> String {
    let files = |n: u64| if n == 1 { "file" } else { "files" };
    let albums = |n: u64| if n == 1 { "album" } else { "albums" };
    let mut s = format!(
        "Exported {} {} from {} {}",
        stats.written,
        files(stats.written),
        stats.albums,
        albums(stats.albums)
    );
    let mut extra = Vec::new();
    if stats.unchanged > 0 {
        extra.push(format!("{} up to date", stats.unchanged));
    }
    if stats.existing > 0 {
        extra.push(format!("{} already in the folder", stats.existing));
    }
    if stats.missing_albums > 0 {
        extra.push(format!(
            "{} missing {}",
            stats.missing_albums,
            albums(stats.missing_albums)
        ));
    }
    if !extra.is_empty() {
        s.push_str(&format!(" ({})", extra.join(", ")));
    }
    s
}

fn run(input: &HookInput) -> Result<HookOutput> {
    let settings: Settings = input.settings_as()?;
    if settings.albums.is_empty() {
        return Ok(HookOutput {
            summary: Some("Pick albums to export on the settings page".into()),
            changed: 0,
        });
    }
    let stats = export(&settings)?;
    Ok(HookOutput {
        summary: Some(summary(&stats)),
        changed: stats.written,
    })
}

/// `on_demand`: the `export_now` button.
pub fn handle_demand(input: &HookInput) -> Result<HookOutput> {
    match input.action() {
        Some("export_now") => run(input),
        other => bail!("unknown action {other:?}"),
    }
}

/// `on_schedule`: daily export.
pub fn handle_schedule(input: &HookInput) -> Result<HookOutput> {
    run(input)
}

photovault_plugin_pdk::photovault_plugin! {
    on_demand: handle_demand,
    on_schedule: handle_schedule,
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use photovault_plugin_pdk::testing::*;
    use serde_json::json;

    const MANIFEST: &str = include_str!("../plugin.toml");
    const JULY_2023: i64 = 1_688_169_600_000_000; // 2023-07-01

    fn setup() -> MockHost {
        let host = MockHost::new().install();
        host.grant_manifest(MANIFEST);
        host
    }

    fn input(overrides: serde_json::Value) -> HookInput {
        on_demand("export_now").with_settings(settings_from_manifest(MANIFEST, overrides))
    }

    fn album(host: &MockHost, name: &str) -> AlbumId {
        host.add_named_album(name)
    }

    fn put(host: &MockHost, id: FileId, title: &str, albums: &[AlbumId]) {
        host.add_file(FileInfo {
            album_ids: albums.to_vec(),
            creation_time: JULY_2023,
            ..file(id, title)
        });
    }

    fn paths(host: &MockHost) -> Vec<String> {
        host.exports().into_keys().collect()
    }

    #[test]
    fn manifest_is_valid() {
        let m = photovault_plugin_pdk::Manifest::from_toml(MANIFEST).unwrap();
        assert_eq!(m.plugin.id, "dev.photovault.export-albums");
    }

    #[test]
    fn nothing_selected() {
        setup();
        let out = run_hook(handle_demand, &input(json!({}))).unwrap();
        assert_eq!(
            out.summary.as_deref(),
            Some("Pick albums to export on the settings page")
        );
    }

    #[test]
    fn exports_with_layouts_and_dedupes() {
        let host = setup();
        let a = album(&host, "Trip: Rome/2023");
        let b = album(&host, "trip_ rome_2023"); // cleans to the same folder
        let c = album(&host, "Not selected");
        put(&host, 1, "IMG_1.jpg", &[a]);
        put(&host, 2, "img_1.JPG", &[a]); // collides case-insensitively
        put(&host, 3, "../evil.jpg", &[a, b]);
        put(&host, 4, "", &[b]);
        put(&host, 5, "x.jpg", &[c]);

        let out = run_hook(
            handle_demand,
            &input(json!({"albums": [b, a], "layout": "album/year-month/file"})),
        )
        .unwrap();
        assert_eq!(out.changed, 5);
        assert_eq!(
            paths(&host),
            [
                "Trip_ Rome_2023/2023-07/IMG_1.jpg",
                "Trip_ Rome_2023/2023-07/__evil.jpg",
                "Trip_ Rome_2023/2023-07/img_1 (2).JPG",
                "trip_ rome_2023 (2)/2023-07/__evil.jpg",
                "trip_ rome_2023 (2)/2023-07/file-4",
            ]
        );
        assert_eq!(
            host.exports()["Trip_ Rome_2023/2023-07/IMG_1.jpg"].file_id,
            1
        );
        assert_eq!(
            out.summary.as_deref(),
            Some("Exported 5 files from 2 albums")
        );
    }

    #[test]
    fn reruns_are_incremental() {
        let host = setup();
        let a = album(&host, "A");
        put(&host, 1, "1.jpg", &[a]);
        put(&host, 2, "2.jpg", &[a]);
        let settings = json!({"albums": [a], "layout": "album/year/file"});
        run_hook(handle_demand, &input(settings.clone())).unwrap();
        assert_eq!(paths(&host), ["A/2023/1.jpg", "A/2023/2.jpg"]);

        // Nothing new: no exports at all.
        host.clear_records();
        let out = run_hook(handle_schedule, &input(settings.clone())).unwrap();
        assert_eq!(out.changed, 0);
        assert_eq!(
            out.summary.as_deref(),
            Some("Exported 0 files from 1 album (2 up to date)")
        );
        assert!(host.writes().is_empty(), "{:?}", host.writes());

        // A new file, and an edited one (ignored without overwrite).
        put(&host, 3, "3.jpg", &[a]);
        let mut edited = host.file(1).unwrap();
        edited.modification_time += 1;
        host.add_file(edited);
        let out = run_hook(handle_demand, &input(settings.clone())).unwrap();
        assert_eq!(out.changed, 1);

        // With overwrite the edited file is copied again, to the same path,
        // even after the layout changed.
        let out = run_hook(
            handle_demand,
            &input(json!({"albums": [a], "layout": "album/file", "overwrite": true})),
        )
        .unwrap();
        assert_eq!(out.changed, 1);
        assert_eq!(
            paths(&host),
            ["A/2023/1.jpg", "A/2023/2.jpg", "A/2023/3.jpg"]
        );
        let last = host.writes().pop().unwrap();
        assert!(matches!(
            last,
            photovault_plugin_pdk::abi::Request::KvSet { .. }
        ));
    }

    #[test]
    fn renamed_album_keeps_its_folder() {
        let host = setup();
        let a = album(&host, "Old");
        put(&host, 1, "1.jpg", &[a]);
        run_hook(handle_demand, &input(json!({"albums": [a]}))).unwrap();
        let mut info = host.album(a).unwrap();
        info.name = "New".into();
        host.add_album(info);
        put(&host, 2, "2.jpg", &[a]);
        run_hook(handle_demand, &input(json!({"albums": [a]}))).unwrap();
        assert_eq!(paths(&host), ["Old/1.jpg", "Old/2.jpg"]);
    }

    #[test]
    fn existing_files_are_kept_and_remembered() {
        let host = setup();
        let a = album(&host, "A");
        put(&host, 1, "1.jpg", &[a]);
        host.put_export("A/1.jpg", 99);
        let out = run_hook(handle_demand, &input(json!({"albums": [a]}))).unwrap();
        assert_eq!(out.changed, 0);
        assert_eq!(host.exports()["A/1.jpg"].file_id, 99);
        assert!(out.summary.unwrap().contains("1 already in the folder"));
        let state: AlbumState = serde_json::from_value(host.kv(&album_key(a)).unwrap()).unwrap();
        assert_eq!(state.files[&1].path, "1.jpg");
    }

    #[test]
    fn missing_album_and_permission() {
        let host = setup();
        let out = run_hook(handle_demand, &input(json!({"albums": [12345]}))).unwrap();
        assert!(out.summary.unwrap().contains("1 missing album"));
        assert!(host.logs().iter().any(|(_, m)| m.contains("12345")));

        let a = album(&host, "A");
        put(&host, 1, "1.jpg", &[a]);
        host.deny(photovault_plugin_pdk::Permission::Write(
            photovault_plugin_pdk::WriteScope::Export,
        ));
        let err = run_hook(handle_demand, &input(json!({"albums": [a]}))).unwrap_err();
        assert_eq!(
            err.root_cause().downcast_ref::<HostError>().unwrap().code,
            ErrorCode::PermissionDenied
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(numbered("a.jpg", 2), "a (2).jpg");
        assert_eq!(numbered("README", 3), "README (3)");
        let f = FileInfo {
            creation_time: JULY_2023,
            ..file(1, "a.jpg")
        };
        assert_eq!(subdir(Layout::Year, &f).as_deref(), Some("2023"));
        assert_eq!(subdir(Layout::YearMonth, &f).as_deref(), Some("2023-07"));
        assert_eq!(subdir(Layout::Flat, &f), None);
    }
}
