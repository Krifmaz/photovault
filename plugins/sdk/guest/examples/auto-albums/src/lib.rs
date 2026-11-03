//! Auto albums: a Photovault example plugin that sorts files into albums by
//! user-written rules.
//!
//! The rule language is described in [`rules`]. The plugin needs
//! `read:metadata` and `write:albums`, and runs:
//!
//! * `on_upload`: applies the rules to the new file;
//! * `on_demand` `apply_all`: walks the whole library, reporting progress;
//! * `on_demand` `preview`: the same walk, but only reports what it would do;
//! * `on_schedule` (daily): the same as `apply_all`, to catch files uploaded
//!   from devices where the plugin is not installed.
//!
//! It is idempotent: files already in an album are never re-added, and albums
//! are only created once a file actually matches.
//!
//! Photovault is based on Ente (ente.com).

#![warn(missing_docs)]

pub mod rules;

use photovault_plugin_pdk::prelude::*;
use rules::{Rule, albums_for, parse_rules};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

/// The plugin's settings, as declared in `plugin.toml`.
#[derive(Debug, Deserialize)]
pub struct Settings {
    /// Rule text, one rule per line.
    pub rules: String,
}

/// Files added to the host per `add_to_album` call.
pub const BATCH: usize = 500;

/// How often (in files) the library walk reports progress.
pub const PROGRESS_EVERY: u64 = 250;

/// Whether to change the library or only count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Add files to albums.
    Apply,
    /// Change nothing; report what would be added.
    Preview,
}

/// What a run did (or would do).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Stats {
    /// Files looked at.
    pub scanned: u64,
    /// New additions per album name.
    pub added: BTreeMap<String, u64>,
    /// Distinct files that went into at least one album.
    pub files_changed: u64,
}

impl Stats {
    /// One line for the activity log.
    pub fn summary(&self, mode: Mode) -> String {
        let total: u64 = self.added.values().sum();
        if total == 0 {
            return format!(
                "Everything is already sorted ({} files checked)",
                self.scanned
            );
        }
        let detail = self
            .added
            .iter()
            .map(|(name, n)| format!("{name}: {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        let verb = match mode {
            Mode::Apply => "Added",
            Mode::Preview => "Would add",
        };
        let albums = self.added.len();
        format!(
            "{verb} {} {} to {albums} {} ({detail})",
            self.files_changed,
            plural(self.files_changed, "file", "files"),
            plural(albums as u64, "album", "albums"),
        )
    }
}

fn plural(n: u64, one: &'static str, many: &'static str) -> &'static str {
    if n == 1 { one } else { many }
}

/// Parse the rules from the settings, turning parse errors into one readable
/// error that lists every bad line.
pub fn load_rules(input: &HookInput) -> Result<Vec<Rule>> {
    let settings: Settings = input.settings_as()?;
    parse_rules(&settings.rules).map_err(|errors| {
        let list = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ");
        anyhow!(
            "{} invalid {}: {list}",
            errors.len(),
            plural(errors.len() as u64, "rule", "rules")
        )
    })
}

/// Apply `rules` to `files`.
///
/// Album names are resolved against the user's owned albums once; albums that
/// do not exist yet are created (with `ensure_album`) only when the first file
/// needs to go in. Additions are sent in batches of [`BATCH`].
pub fn apply<I>(rules: &[Rule], files: I, mode: Mode, report_progress: bool) -> Result<Stats>
where
    I: IntoIterator<Item = std::result::Result<FileInfo, HostError>>,
{
    let mut stats = Stats::default();
    if rules.is_empty() {
        return Ok(stats);
    }
    // Owned ordinary albums by name, as `ensure_album` would find them.
    let mut known: BTreeMap<String, AlbumId> = pdk::host::list_albums()?
        .into_iter()
        .filter(|a| a.is_owned && a.kind == AlbumKind::Album)
        .map(|a| (a.name, a.id))
        .collect();
    let mut pending: BTreeMap<String, Vec<FileId>> = BTreeMap::new();
    let mut changed: BTreeSet<FileId> = BTreeSet::new();

    for file in files {
        let file = file?;
        stats.scanned += 1;
        for album in albums_for(rules, &file) {
            if known
                .get(album)
                .is_some_and(|id| file.album_ids.contains(id))
            {
                continue;
            }
            let queue = pending.entry(album.to_owned()).or_default();
            queue.push(file.id);
            *stats.added.entry(album.to_owned()).or_default() += 1;
            changed.insert(file.id);
            if mode == Mode::Apply && queue.len() >= BATCH {
                let ids = std::mem::take(queue);
                flush(album, &ids, &mut known)?;
            }
        }
        if report_progress && stats.scanned.is_multiple_of(PROGRESS_EVERY) {
            pdk::host::progress(stats.scanned, None, Some("Checking files"))?;
        }
    }
    if mode == Mode::Apply {
        for (album, ids) in &pending {
            flush(album, ids, &mut known)?;
        }
    }
    if report_progress {
        pdk::host::progress(stats.scanned, Some(stats.scanned), Some("Done"))?;
    }
    stats.files_changed = changed.len() as u64;
    Ok(stats)
}

fn flush(album: &str, ids: &[FileId], known: &mut BTreeMap<String, AlbumId>) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let id = match known.get(album) {
        Some(id) => *id,
        None => {
            let created = pdk::host::ensure_album(album)
                .with_context(|| format!("could not create album `{album}`"))?;
            pdk::info!("using album `{album}` ({})", created.id);
            known.insert(album.to_owned(), created.id);
            created.id
        }
    };
    pdk::host::add_to_album(id, ids).with_context(|| format!("could not add files to `{album}`"))
}

fn output(stats: &Stats, mode: Mode) -> HookOutput {
    HookOutput {
        summary: Some(stats.summary(mode)),
        changed: match mode {
            Mode::Apply => stats.files_changed,
            Mode::Preview => 0,
        },
    }
}

/// `on_upload`: sort the new file.
pub fn handle_upload(input: &HookInput) -> Result<HookOutput> {
    let rules = load_rules(input)?;
    if rules.is_empty() {
        return Ok(HookOutput::default());
    }
    let Some(file) = input.uploaded_file()? else {
        return Ok(HookOutput::default());
    };
    let stats = apply(&rules, [Ok(file)], Mode::Apply, false)?;
    Ok(output(&stats, Mode::Apply))
}

/// Walk the whole library.
fn walk(input: &HookInput, mode: Mode) -> Result<HookOutput> {
    let rules = load_rules(input)?;
    if rules.is_empty() {
        return Ok(HookOutput {
            summary: Some("No rules yet: add some on the settings page".into()),
            changed: 0,
        });
    }
    let stats = apply(&rules, pdk::host::files(FileQuery::default()), mode, true)?;
    let mut out = output(&stats, mode);
    if mode == Mode::Preview {
        let n = rules.len() as u64;
        out.summary = Some(format!(
            "{} {} OK. {}",
            n,
            plural(n, "rule", "rules"),
            stats.summary(mode)
        ));
    }
    Ok(out)
}

/// `on_demand`: `apply_all` or `preview`.
pub fn handle_demand(input: &HookInput) -> Result<HookOutput> {
    match input.action() {
        Some("apply_all") => walk(input, Mode::Apply),
        Some("preview") => walk(input, Mode::Preview),
        other => bail!("unknown action {other:?}"),
    }
}

/// `on_schedule`: same as `apply_all`.
pub fn handle_schedule(input: &HookInput) -> Result<HookOutput> {
    walk(input, Mode::Apply)
}

photovault_plugin_pdk::photovault_plugin! {
    on_upload: handle_upload,
    on_demand: handle_demand,
    on_schedule: handle_schedule,
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use photovault_plugin_pdk::testing::*;
    use photovault_plugin_pdk::{Permission, WriteScope};
    use serde_json::json;

    const MANIFEST: &str = include_str!("../plugin.toml");

    fn setup() -> MockHost {
        let host = MockHost::new().install();
        host.grant_manifest(MANIFEST);
        host
    }

    fn with_rules(input: HookInput, rules: &str) -> HookInput {
        input.with_settings(settings_from_manifest(MANIFEST, json!({ "rules": rules })))
    }

    fn library(host: &MockHost) {
        let mut a = file(1, "dog_1.jpg");
        a.device_folder = Some("Camera".into());
        let mut b = file(2, "Screenshot_2024.png");
        b.device_folder = Some("Screenshots".into());
        let mut c = file(3, "clip.mp4");
        c.file_type = FileType::Video;
        c.is_favorite = true;
        host.add_file(a).add_file(b).add_file(c);
    }

    const RULES: &str = "\
# sort things
Dogs: name=dog*
Screens: folder=screenshots
Favourite videos: type=video and favorite
";

    #[test]
    fn manifest_is_valid() {
        let m = photovault_plugin_pdk::Manifest::from_toml(MANIFEST).unwrap();
        assert_eq!(m.plugin.id, "dev.photovault.auto-albums");
    }

    #[test]
    fn apply_all_is_idempotent() {
        let host = setup();
        host.set_page_size(2);
        library(&host);
        let out = run_hook(handle_demand, &with_rules(on_demand("apply_all"), RULES)).unwrap();
        assert_eq!(out.changed, 3);
        assert_eq!(
            out.summary.as_deref(),
            Some("Added 3 files to 3 albums (Dogs: 1, Favourite videos: 1, Screens: 1)")
        );
        let dogs = host.album_named("Dogs").unwrap();
        assert_eq!(host.album_files(dogs.id), [1]);
        assert_eq!(
            host.album_files(host.album_named("Screens").unwrap().id),
            [2]
        );
        assert_eq!(
            host.album_files(host.album_named("Favourite videos").unwrap().id),
            [3]
        );
        let last = host.progress().pop().unwrap();
        assert_eq!((last.done, last.total), (3, Some(3)));

        // Second run changes nothing and sends no writes.
        host.clear_records();
        let out = run_hook(handle_demand, &with_rules(on_demand("apply_all"), RULES)).unwrap();
        assert_eq!(out.changed, 0);
        assert!(host.writes().is_empty(), "{:?}", host.writes());
        assert_eq!(
            out.summary.as_deref(),
            Some("Everything is already sorted (3 files checked)")
        );
    }

    #[test]
    fn uses_existing_album_and_skips_members() {
        let host = setup();
        library(&host);
        let dogs = host.add_named_album("Dogs");
        let mut d = host.file(1).unwrap();
        d.album_ids.push(dogs);
        host.add_file(d);
        host.add_file(file(4, "dog_2.jpg"));
        run_hook(
            handle_schedule,
            &with_rules(on_schedule(None), "Dogs: name=dog*"),
        )
        .unwrap();
        assert_eq!(host.album_files(dogs), [1, 4]);
        // No new album was created and only file 4 was sent.
        assert!(
            host.writes()
                .iter()
                .all(|w| !matches!(w, photovault_plugin_pdk::abi::Request::EnsureAlbum { .. }))
        );
    }

    #[test]
    fn upload_sorts_one_file() {
        let host = setup();
        library(&host);
        let new = file(9, "dog_9.jpg");
        host.add_file(new.clone());
        let out = run_hook(handle_upload, &with_rules(on_upload(&new), RULES)).unwrap();
        assert_eq!(out.changed, 1);
        assert_eq!(host.album_files(host.album_named("Dogs").unwrap().id), [9]);
        // Only the uploaded file was touched.
        assert!(host.album_named("Screens").is_none());
    }

    #[test]
    fn preview_changes_nothing() {
        let host = setup();
        library(&host);
        let out = run_hook(handle_demand, &with_rules(on_demand("preview"), RULES)).unwrap();
        assert_eq!(out.changed, 0);
        assert_eq!(
            out.summary.as_deref(),
            Some(
                "3 rules OK. Would add 3 files to 3 albums (Dogs: 1, Favourite videos: 1, Screens: 1)"
            )
        );
        assert!(host.writes().is_empty());
        assert!(host.album_named("Dogs").is_none());
    }

    #[test]
    fn parse_errors_are_reported() {
        let host = setup();
        library(&host);
        let err = run_hook(
            handle_demand,
            &with_rules(on_demand("apply_all"), "Dogs name=dog\nX: month=13"),
        )
        .unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.starts_with("2 invalid rules: line 1:"), "{msg}");
        assert!(msg.contains("line 2: month `13`"), "{msg}");
        assert!(host.writes().is_empty());
    }

    #[test]
    fn empty_rules_do_nothing() {
        let host = setup();
        library(&host);
        let out = run_hook(handle_demand, &with_rules(on_demand("apply_all"), "# none")).unwrap();
        assert_eq!(out.changed, 0);
        let new = file(9, "dog_9.jpg");
        host.add_file(new.clone());
        let out = run_hook(handle_upload, &with_rules(on_upload(&new), "")).unwrap();
        assert_eq!(out, HookOutput::default());
    }

    #[test]
    fn missing_permission_fails_cleanly() {
        let host = setup();
        library(&host);
        host.deny(Permission::Write(WriteScope::Albums));
        let err = run_hook(handle_demand, &with_rules(on_demand("apply_all"), RULES)).unwrap_err();
        let host_err = err.root_cause().downcast_ref::<HostError>().unwrap();
        assert_eq!(host_err.code, ErrorCode::PermissionDenied);
    }

    #[test]
    fn batches_large_runs() {
        let host = setup();
        for i in 0..(BATCH as i64 + 10) {
            host.add_file(file(i + 1, "dog.jpg"));
        }
        run_hook(
            handle_demand,
            &with_rules(on_demand("apply_all"), "Dogs: name=dog*"),
        )
        .unwrap();
        let adds = host
            .writes()
            .into_iter()
            .filter(|w| matches!(w, photovault_plugin_pdk::abi::Request::AddToAlbum { .. }))
            .count();
        assert_eq!(adds, 2);
        assert_eq!(
            host.album_files(host.album_named("Dogs").unwrap().id).len(),
            BATCH + 10
        );
    }

    #[test]
    fn unknown_action() {
        setup();
        assert!(run_hook(handle_demand, &with_rules(on_demand("nope"), RULES)).is_err());
    }
}
