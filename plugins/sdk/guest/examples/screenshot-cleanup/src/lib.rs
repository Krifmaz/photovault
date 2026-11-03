//! Screenshot cleanup: a Photovault example plugin that finds old screenshots
//! and either collects them in an album or moves them to the trash.
//!
//! # Detection
//!
//! Only still images are considered (screen recordings are left alone). An
//! image is a screenshot when either
//!
//! * a segment of its **device folder** is one of [`SCREENSHOT_FOLDERS`]
//!   (Android's `Pictures/Screenshots`, the iOS "Screenshots" album, ...), or
//! * its **file name** starts with one of [`SCREENSHOT_PREFIXES`] followed by
//!   a non-letter (so `Screenshot_20240101.png` and `Screen Shot 2019-01-01 at
//!   10.00.00.png` match, but `Screenshotting tips.jpg` does not).
//!
//! Both lists are compared case-insensitively and are deliberately short:
//! they cover the defaults of Android, iOS, macOS (old and new naming, Xcode's
//! simulator), Windows, GNOME, and the Spanish and German system languages.
//!
//! # Safety
//!
//! With "Never touch favourites" on (the default) a favourite is never added
//! to the album or trashed. The `scan` button asks for confirmation, and
//! trashed files stay recoverable from the trash, as in Ente.
//!
//! Photovault is based on Ente (ente.com).

#![warn(missing_docs)]

use photovault_plugin_pdk::prelude::*;
use photovault_plugin_pdk::time::{MICROS_PER_DAY, days_between};
use serde::Deserialize;

/// Device folder names (any path segment, case-insensitive) that hold
/// screenshots.
pub const SCREENSHOT_FOLDERS: &[&str] = &[
    "screenshots",          // Android, iOS, Windows (Pictures\Screenshots)
    "screenshot",           // some Android skins
    "screen shots",         // older macOS exports
    "capturas de pantalla", // Spanish
    "bildschirmfotos",      // German
];

/// File name prefixes (case-insensitive, followed by a non-letter) used by
/// screenshot tools.
pub const SCREENSHOT_PREFIXES: &[&str] = &[
    "screenshot",            // Android `Screenshot_…`, macOS 10.14+, Windows 11, GNOME
    "screen shot",           // macOS before 10.14
    "simulator screen shot", // Xcode iOS simulator
    "captura de pantalla",   // Spanish macOS and Windows
    "bildschirmfoto",        // German macOS
];

/// Files trashed or added per host call.
pub const BATCH: usize = 500;

/// How often (in files) the scan reports progress.
pub const PROGRESS_EVERY: u64 = 250;

/// What to do with screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Collect them in an album.
    Album,
    /// Move them to the trash.
    Trash,
}

/// The plugin's settings, as declared in `plugin.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct Settings {
    /// Album or trash.
    pub action: Action,
    /// Album to collect into.
    pub album_name: String,
    /// Minimum age in days.
    pub min_age_days: u32,
    /// Leave favourites alone.
    pub skip_favorites: bool,
}

/// Whether `file` looks like a screenshot (see the crate docs).
pub fn is_screenshot(file: &FileInfo) -> bool {
    if file.file_type != FileType::Image {
        return false;
    }
    let in_folder = file.device_folder.as_deref().is_some_and(|folder| {
        folder
            .split(['/', '\\'])
            .any(|seg| SCREENSHOT_FOLDERS.contains(&seg.trim().to_lowercase().as_str()))
    });
    in_folder || has_screenshot_prefix(&file.title)
}

fn has_screenshot_prefix(name: &str) -> bool {
    let name = name.to_lowercase();
    SCREENSHOT_PREFIXES.iter().any(|p| {
        name.strip_prefix(p)
            .is_some_and(|rest| !rest.chars().next().is_some_and(char::is_alphabetic))
    })
}

/// Whether the plugin should act on `file` now.
pub fn is_candidate(file: &FileInfo, settings: &Settings, now: i64) -> bool {
    is_screenshot(file)
        && !(settings.skip_favorites && file.is_favorite)
        && days_between(file.creation_time, now) >= i64::from(settings.min_age_days)
}

/// Result of a cleanup run.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    /// Files looked at.
    pub scanned: u64,
    /// Screenshots acted on.
    pub moved: u64,
    /// Screenshots skipped because they are favourites.
    pub skipped_favorites: u64,
}

/// Find and handle screenshots according to `settings`.
pub fn clean(settings: &Settings, now: i64) -> Result<Stats> {
    let name = settings.album_name.trim();
    if settings.action == Action::Album && name.is_empty() {
        bail!("set an album name on the settings page");
    }
    let query = FileQuery {
        file_type: Some(FileType::Image),
        // Inclusive age limit: created at or before `now - days`.
        created_before: Some(now - i64::from(settings.min_age_days) * MICROS_PER_DAY + 1),
        ..Default::default()
    };
    // The owned album `ensure_album` would return, if it already exists, so
    // files already in it are skipped without creating anything.
    let existing: Option<AlbumId> = if settings.action == Action::Album {
        pdk::host::list_albums()?
            .into_iter()
            .find(|a| a.is_owned && a.kind == AlbumKind::Album && a.name == name)
            .map(|a| a.id)
    } else {
        None
    };
    let mut found: Vec<FileId> = Vec::new();
    let mut stats = Stats::default();

    // Collect first and act afterwards: trashing while paging would shift the
    // pages under the iterator.
    for file in pdk::host::files(query) {
        let file = file?;
        stats.scanned += 1;
        if stats.scanned.is_multiple_of(PROGRESS_EVERY) {
            pdk::host::progress(stats.scanned, None, Some("Looking for screenshots"))?;
        }
        if is_screenshot(&file) && settings.skip_favorites && file.is_favorite {
            stats.skipped_favorites += 1;
            continue;
        }
        if !is_candidate(&file, settings, now) {
            continue;
        }
        if existing.is_some_and(|id| file.album_ids.contains(&id)) {
            continue;
        }
        found.push(file.id);
    }
    stats.moved = found.len() as u64;

    if !found.is_empty() {
        match settings.action {
            Action::Trash => {
                for chunk in found.chunks(BATCH) {
                    pdk::host::trash_files(chunk)?;
                }
            }
            Action::Album => {
                let id = match existing {
                    Some(id) => id,
                    None => {
                        pdk::host::ensure_album(name)
                            .with_context(|| format!("could not create album `{name}`"))?
                            .id
                    }
                };
                for chunk in found.chunks(BATCH) {
                    pdk::host::add_to_album(id, chunk)?;
                }
            }
        }
    }
    pdk::host::progress(stats.scanned, Some(stats.scanned), Some("Done"))?;
    Ok(stats)
}

/// One line for the activity log.
pub fn summary(settings: &Settings, stats: &Stats) -> String {
    let noun = |n: u64| if n == 1 { "screenshot" } else { "screenshots" };
    let mut s = if stats.moved == 0 {
        format!(
            "No new screenshots older than {} days",
            settings.min_age_days
        )
    } else {
        match settings.action {
            Action::Album => format!(
                "Added {} {} to {}",
                stats.moved,
                noun(stats.moved),
                settings.album_name.trim()
            ),
            Action::Trash => format!("Moved {} {} to the trash", stats.moved, noun(stats.moved)),
        }
    };
    if stats.skipped_favorites > 0 {
        s.push_str(&format!(
            " (kept {} favourite {})",
            stats.skipped_favorites,
            noun(stats.skipped_favorites)
        ));
    }
    s
}

fn run(input: &HookInput) -> Result<HookOutput> {
    let settings: Settings = input.settings_as()?;
    let stats = clean(&settings, input.now)?;
    Ok(HookOutput {
        summary: Some(summary(&settings, &stats)),
        changed: stats.moved,
    })
}

/// `on_demand`: the `scan` button.
pub fn handle_demand(input: &HookInput) -> Result<HookOutput> {
    match input.action() {
        Some("scan") => run(input),
        other => bail!("unknown action {other:?}"),
    }
}

/// `on_schedule`: daily cleanup.
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
    const DAY: i64 = MICROS_PER_DAY;

    fn setup() -> MockHost {
        let host = MockHost::new().install();
        host.grant_manifest(MANIFEST);
        host
    }

    fn input(overrides: serde_json::Value) -> HookInput {
        on_demand("scan").with_settings(settings_from_manifest(MANIFEST, overrides))
    }

    fn shot(id: FileId, title: &str, age_days: i64) -> FileInfo {
        FileInfo {
            creation_time: DEFAULT_NOW - age_days * DAY,
            ..file(id, title)
        }
    }

    #[test]
    fn manifest_is_valid() {
        let m = photovault_plugin_pdk::Manifest::from_toml(MANIFEST).unwrap();
        assert_eq!(m.plugin.id, "dev.photovault.screenshot-cleanup");
        assert!(m.hooks.on_demand[0].confirm);
    }

    #[test]
    fn detection() {
        for name in [
            "Screenshot_20240101-101010.png",
            "Screenshot 2024-01-01 at 10.00.00.png",
            "Screenshot from 2024-01-01 10-00-00.png",
            "Screen Shot 2019-05-01 at 9.41.12 AM.png",
            "Simulator Screen Shot - iPhone 15 - 2024-01-01 at 10.00.00.png",
            "Captura de pantalla 2024-01-01 100000.png",
            "Bildschirmfoto 2024-01-01 um 10.00.00.png",
            "screenshot.png",
        ] {
            assert!(is_screenshot(&file(1, name)), "{name}");
        }
        for name in [
            "IMG_0001.HEIC",
            "Screenshotting tips.jpg",
            "my screenshot.png",
        ] {
            assert!(!is_screenshot(&file(1, name)), "{name}");
        }
        let mut f = file(1, "IMG_0001.PNG");
        f.device_folder = Some("Pictures/Screenshots".into());
        assert!(is_screenshot(&f));
        f.device_folder = Some("Camera".into());
        assert!(!is_screenshot(&f));
        // Screen recordings are not screenshots.
        let mut v = file(1, "Screenshot_1.mp4");
        v.file_type = FileType::Video;
        assert!(!is_screenshot(&v));
    }

    fn library(host: &MockHost) {
        host.add_file(shot(1, "Screenshot_old.png", 40));
        host.add_file(shot(2, "Screenshot_new.png", 5));
        host.add_file(FileInfo {
            is_favorite: true,
            ..shot(3, "Screenshot_fav.png", 100)
        });
        host.add_file(shot(4, "IMG_1.jpg", 400));
        host.add_file(FileInfo {
            device_folder: Some("Screenshots".into()),
            ..shot(5, "IMG_2.PNG", 30)
        });
    }

    #[test]
    fn collects_into_album_idempotently() {
        let host = setup();
        library(&host);
        let out = run_hook(handle_demand, &input(json!({}))).unwrap();
        assert_eq!(out.changed, 2);
        assert_eq!(
            out.summary.as_deref(),
            Some("Added 2 screenshots to Screenshots (kept 1 favourite screenshot)")
        );
        let album = host.album_named("Screenshots").unwrap();
        assert_eq!(host.album_files(album.id), [1, 5]);
        assert!(host.trashed().is_empty());

        host.clear_records();
        let out = run_hook(handle_schedule, &input(json!({}))).unwrap();
        assert_eq!(out.changed, 0);
        assert!(host.writes().is_empty());
    }

    #[test]
    fn trashes_but_never_favourites() {
        let host = setup();
        library(&host);
        let out = run_hook(
            handle_demand,
            &input(json!({"action": "trash", "min_age_days": 0})),
        )
        .unwrap();
        assert_eq!(host.trashed(), [1, 2, 5]);
        assert_eq!(out.changed, 3);
        assert!(host.file(3).unwrap().album_ids.is_empty());
    }

    #[test]
    fn favourites_when_allowed() {
        let host = setup();
        library(&host);
        run_hook(
            handle_demand,
            &input(json!({"action": "trash", "skip_favorites": false})),
        )
        .unwrap();
        assert_eq!(host.trashed(), [1, 3, 5]);
    }

    #[test]
    fn custom_album_and_nothing_found() {
        let host = setup();
        host.add_file(shot(1, "IMG_1.jpg", 400));
        let out = run_hook(handle_demand, &input(json!({"album_name": "Old shots"}))).unwrap();
        assert_eq!(
            out.summary.as_deref(),
            Some("No new screenshots older than 30 days")
        );
        // No empty album was created.
        assert!(host.album_named("Old shots").is_none());
        assert!(host.writes().is_empty());
    }

    #[test]
    fn empty_album_name_is_an_error() {
        setup();
        assert!(run_hook(handle_demand, &input(json!({"album_name": "  "}))).is_err());
    }

    #[test]
    fn batches_and_progress() {
        let host = setup();
        host.set_page_size(100);
        for i in 0..(BATCH as i64 + 1) {
            host.add_file(shot(i + 1, "Screenshot_x.png", 31));
        }
        let out = run_hook(handle_demand, &input(json!({"action": "trash"}))).unwrap();
        assert_eq!(out.changed, BATCH as u64 + 1);
        assert_eq!(host.writes().len(), 2);
        assert!(host.progress().len() >= 3);
    }
}
