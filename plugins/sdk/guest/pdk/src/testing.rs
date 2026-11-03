//! Native unit testing of plugin logic (not available in Wasm).
//!
//! Install a [`MockHost`] on the test thread, fill it with files and albums,
//! run your hook through [`run_hook`], then assert on what the plugin did:
//!
//! ```
//! use photovault_plugin_pdk::prelude::*;
//! use photovault_plugin_pdk::testing::{MockHost, file, on_demand};
//!
//! fn tag_all(_: &HookInput) -> Result<HookOutput> {
//!     let mut changed = 0;
//!     for f in pdk::host::files(Default::default()) {
//!         pdk::host::add_tags(f?.id, &["seen"])?;
//!         changed += 1;
//!     }
//!     Ok(HookOutput { summary: None, changed })
//! }
//!
//! let host = MockHost::new();
//! host.install();
//! host.add_file(file(1, "IMG_1.jpg"));
//! let out = photovault_plugin_pdk::testing::run_hook(tag_all, &on_demand("run")).unwrap();
//! assert_eq!(out.changed, 1);
//! assert_eq!(host.file(1).unwrap().tags, ["seen"]);
//! ```
//!
//! The mock speaks the real JSON wire format, enforces the granted
//! [`PermissionSet`] (all scopes by default; see [`MockHost::grant_only`] and
//! [`MockHost::deny`]), paginates [`Request::ListFiles`] with a configurable
//! page size, and records every mutating request.

use crate::Error;
use crate::abi::*;
use crate::hook::{HookFn, run_json};
use crate::http::{HttpRequest, HttpResponse, host_matches, url_host};
use crate::log::Level;
use crate::{Manifest, Permission, PermissionSet, ReadScope, WriteScope, settings};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// What the PDK needs from a host. [`MockHost`] implements it; implement it
/// yourself for unusual tests and install it with [`install`].
pub trait Host {
    /// `photovault_call`: a JSON [`Request`] in, a JSON [`Response`] out.
    fn call(&self, request: &[u8]) -> Vec<u8>;
    /// `photovault_read`: a JSON [`ReadRequest`] in, raw bytes out.
    fn read(&self, request: &[u8]) -> Result<Vec<u8>, HostError>;
    /// One log line.
    fn log(&self, level: Level, message: &str);
    /// One HTTP exchange.
    fn http(&self, request: &HttpRequest, body: Option<&[u8]>) -> Result<HttpResponse, HostError>;
}

/// Install `host` for the current thread (each test runs on its own thread,
/// so tests do not see each other's hosts). Returns the previous host.
pub fn install(host: Rc<dyn Host>) -> Option<Rc<dyn Host>> {
    crate::backend::install(Some(host))
}

/// Remove the current thread's host.
pub fn uninstall() -> Option<Rc<dyn Host>> {
    crate::backend::install(None)
}

/// Run a hook the way the Wasm export does: `input` is serialised to JSON,
/// decoded, passed to `f`, and the output goes through JSON again. Errors
/// from `f` are returned unchanged so tests can inspect or downcast them.
pub fn run_hook(f: HookFn, input: &HookInput) -> Result<HookOutput, Error> {
    let bytes = serde_json::to_vec(input)?;
    let out = run_json(f, &bytes)?;
    Ok(serde_json::from_slice(&out)?)
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// 2024-01-01T00:00:00Z in microseconds, the default `now` of the builders.
pub const DEFAULT_NOW: i64 = 1_704_067_200_000_000;

/// A plain image with this id and title, created at [`DEFAULT_NOW`] minus one
/// day, in no album. Adjust fields with struct update syntax.
pub fn file(id: FileId, title: &str) -> FileInfo {
    FileInfo {
        id,
        album_ids: vec![],
        title: title.to_owned(),
        file_type: FileType::Image,
        creation_time: DEFAULT_NOW - crate::time::MICROS_PER_DAY,
        modification_time: DEFAULT_NOW - crate::time::MICROS_PER_DAY,
        latitude: None,
        longitude: None,
        width: None,
        height: None,
        size: Some(1000),
        device_folder: None,
        caption: None,
        tags: vec![],
        is_favorite: false,
        is_archived: false,
    }
}

fn input(event: HookEvent) -> HookInput {
    HookInput {
        event,
        settings: settings::Values::new(),
        now: DEFAULT_NOW,
        runner: Runner::Test,
    }
}

/// An on-demand input for `action`, with empty settings.
pub fn on_demand(action: &str) -> HookInput {
    input(HookEvent::OnDemand {
        action: action.to_owned(),
    })
}

/// A schedule input, with empty settings.
pub fn on_schedule(last_run: Option<i64>) -> HookInput {
    input(HookEvent::Schedule { last_run })
}

/// An upload input for `file`, carrying the metadata as the host does when
/// `read:metadata` is granted.
pub fn on_upload(file: &FileInfo) -> HookInput {
    input(HookEvent::Upload {
        file_id: file.id,
        file: Some(file.clone()),
    })
}

/// The settings the host would pass: the defaults from `plugin_toml` (which
/// must be a valid manifest) overlaid with `overrides` (a JSON object).
/// Invalid overrides panic, so tests cannot use values the app would reject.
pub fn settings_from_manifest(plugin_toml: &str, overrides: Value) -> settings::Values {
    let manifest = Manifest::from_toml(plugin_toml).expect("plugin.toml is valid");
    let overrides = overrides.as_object().cloned().unwrap_or_default();
    settings::validate_values(&manifest.settings, &overrides).expect("valid setting overrides");
    settings::effective_values(&manifest.settings, &overrides)
}

/// Extension to attach settings or a clock to a [`HookInput`] fluently.
pub trait HookInputBuilder {
    /// Replace the settings.
    fn with_settings(self, settings: settings::Values) -> Self;
    /// Replace `now`.
    fn at(self, now: i64) -> Self;
}

impl HookInputBuilder for HookInput {
    fn with_settings(mut self, settings: settings::Values) -> Self {
        self.settings = settings;
        self
    }
    fn at(mut self, now: i64) -> Self {
        self.now = now;
        self
    }
}

// ---------------------------------------------------------------------------
// MockHost
// ---------------------------------------------------------------------------

/// One progress report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressReport {
    /// Items done.
    pub done: u64,
    /// Items in total.
    pub total: Option<u64>,
    /// Status text.
    pub message: Option<String>,
}

/// One file the mock "exported".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exported {
    /// The file.
    pub file_id: FileId,
    /// Bytes written.
    pub bytes: u64,
}

#[derive(Debug)]
struct State {
    granted: PermissionSet,
    files: BTreeMap<FileId, FileInfo>,
    trashed: BTreeSet<FileId>,
    albums: BTreeMap<AlbumId, AlbumInfo>,
    next_album_id: AlbumId,
    page_size: u32,
    kv: BTreeMap<String, Value>,
    kv_limit: usize,
    thumbnails: BTreeMap<FileId, Vec<u8>>,
    originals: BTreeMap<FileId, Vec<u8>>,
    exports: BTreeMap<String, Exported>,
    http: BTreeMap<String, HttpResponse>,
    failures: BTreeMap<String, HostError>,
    writes: Vec<Request>,
    progress: Vec<ProgressReport>,
    logs: Vec<(Level, String)>,
    http_log: Vec<String>,
}

/// An in-memory host for native tests. Cloning gives another handle to the
/// same state.
#[derive(Debug, Clone)]
pub struct MockHost(Rc<RefCell<State>>);

impl Default for MockHost {
    fn default() -> Self {
        Self::new()
    }
}

fn all_permissions() -> PermissionSet {
    PermissionSet {
        read: [
            ReadScope::Metadata,
            ReadScope::Thumbnails,
            ReadScope::Originals,
        ]
        .into(),
        write: [
            WriteScope::Tags,
            WriteScope::Albums,
            WriteScope::Trash,
            WriteScope::Export,
        ]
        .into(),
        network: BTreeSet::new(),
    }
}

fn err(code: ErrorCode, message: impl Into<String>) -> HostError {
    HostError::new(code, message)
}

impl MockHost {
    /// An empty library with every read and write scope granted (no network
    /// hosts) and a page size of 500.
    pub fn new() -> Self {
        Self(Rc::new(RefCell::new(State {
            granted: all_permissions(),
            files: BTreeMap::new(),
            trashed: BTreeSet::new(),
            albums: BTreeMap::new(),
            next_album_id: 1000,
            page_size: 500,
            kv: BTreeMap::new(),
            kv_limit: 1 << 20,
            thumbnails: BTreeMap::new(),
            originals: BTreeMap::new(),
            exports: BTreeMap::new(),
            http: BTreeMap::new(),
            failures: BTreeMap::new(),
            writes: Vec::new(),
            progress: Vec::new(),
            logs: Vec::new(),
            http_log: Vec::new(),
        })))
    }

    /// Install this mock as the current thread's host and return it.
    pub fn install(&self) -> Self {
        install(Rc::new(self.clone()));
        self.clone()
    }

    // ----- permissions -------------------------------------------------

    /// Grant exactly `set` (for example a plugin's manifest permissions).
    pub fn grant_only(&self, set: PermissionSet) -> &Self {
        self.0.borrow_mut().granted = set;
        self
    }

    /// Grant exactly what `plugin_toml` asks for.
    pub fn grant_manifest(&self, plugin_toml: &str) -> &Self {
        let m = Manifest::from_toml(plugin_toml).expect("plugin.toml is valid");
        self.grant_only(m.permissions)
    }

    /// Revoke one permission.
    pub fn deny(&self, permission: Permission) -> &Self {
        self.0.borrow_mut().granted.remove(&permission);
        self
    }

    /// Make every request with this `method` name (e.g. `"add_to_album"`)
    /// fail with `error`.
    pub fn fail_on(&self, method: &str, error: HostError) -> &Self {
        self.0
            .borrow_mut()
            .failures
            .insert(method.to_owned(), error);
        self
    }

    // ----- library -------------------------------------------------------

    /// Add or replace a file. Albums it names are created (owned, named
    /// `Album <id>`) if missing.
    pub fn add_file(&self, file: FileInfo) -> &Self {
        let mut s = self.0.borrow_mut();
        for &a in &file.album_ids {
            s.albums.entry(a).or_insert_with(|| AlbumInfo {
                id: a,
                name: format!("Album {a}"),
                kind: AlbumKind::Album,
                is_owned: true,
                file_count: None,
            });
        }
        s.files.insert(file.id, file);
        Self::recount(&mut s);
        drop(s);
        self
    }

    /// Add or replace an album and return its id.
    pub fn add_album(&self, album: AlbumInfo) -> AlbumId {
        let mut s = self.0.borrow_mut();
        let id = album.id;
        s.next_album_id = s.next_album_id.max(id + 1);
        s.albums.insert(id, album);
        Self::recount(&mut s);
        id
    }

    /// Add an owned ordinary album called `name` and return its id.
    pub fn add_named_album(&self, name: &str) -> AlbumId {
        let id = self.0.borrow().next_album_id;
        self.add_album(AlbumInfo {
            id,
            name: name.to_owned(),
            kind: AlbumKind::Album,
            is_owned: true,
            file_count: None,
        })
    }

    /// Set the thumbnail bytes `read_thumbnail` returns.
    pub fn set_thumbnail(&self, file_id: FileId, bytes: Vec<u8>) -> &Self {
        self.0.borrow_mut().thumbnails.insert(file_id, bytes);
        self
    }

    /// Set the original bytes `read_original` returns (and `export_file`
    /// counts).
    pub fn set_original(&self, file_id: FileId, bytes: Vec<u8>) -> &Self {
        self.0.borrow_mut().originals.insert(file_id, bytes);
        self
    }

    /// Set the `ListFiles` page size (the host caps requested limits to it).
    pub fn set_page_size(&self, n: u32) -> &Self {
        self.0.borrow_mut().page_size = n.max(1);
        self
    }

    /// Set the total KV size limit in bytes (default 1 MiB).
    pub fn set_kv_limit(&self, bytes: usize) -> &Self {
        self.0.borrow_mut().kv_limit = bytes;
        self
    }

    /// Answer `GET`/`POST`/... to exactly `url` with `response`. The request's
    /// host must still be granted.
    pub fn route(&self, url: &str, response: HttpResponse) -> &Self {
        self.0.borrow_mut().http.insert(url.to_owned(), response);
        self
    }

    /// Pretend a file already exists in the export folder at `path`.
    pub fn put_export(&self, path: &str, file_id: FileId) -> &Self {
        self.0
            .borrow_mut()
            .exports
            .insert(path.to_owned(), Exported { file_id, bytes: 0 });
        self
    }

    /// Store a KV value directly.
    pub fn put_kv(&self, key: &str, value: Value) -> &Self {
        self.0.borrow_mut().kv.insert(key.to_owned(), value);
        self
    }

    // ----- inspection -------------------------------------------------

    /// A file's current metadata (also for trashed files).
    pub fn file(&self, id: FileId) -> Option<FileInfo> {
        self.0.borrow().files.get(&id).cloned()
    }

    /// An album by id.
    pub fn album(&self, id: AlbumId) -> Option<AlbumInfo> {
        self.0.borrow().albums.get(&id).cloned()
    }

    /// The owned album called `name`, if any.
    pub fn album_named(&self, name: &str) -> Option<AlbumInfo> {
        self.0
            .borrow()
            .albums
            .values()
            .find(|a| a.is_owned && a.name == name)
            .cloned()
    }

    /// Ids of the (untrashed) files in an album, sorted.
    pub fn album_files(&self, id: AlbumId) -> Vec<FileId> {
        let s = self.0.borrow();
        s.files
            .values()
            .filter(|f| !s.trashed.contains(&f.id) && f.album_ids.contains(&id))
            .map(|f| f.id)
            .collect()
    }

    /// Ids of trashed files, sorted.
    pub fn trashed(&self) -> Vec<FileId> {
        self.0.borrow().trashed.iter().copied().collect()
    }

    /// Everything in the export folder, by path.
    pub fn exports(&self) -> BTreeMap<String, Exported> {
        self.0.borrow().exports.clone()
    }

    /// A KV value.
    pub fn kv(&self, key: &str) -> Option<Value> {
        self.0.borrow().kv.get(key).cloned()
    }

    /// Every successful mutating request so far (tags, albums, trash, export,
    /// kv_set), in order.
    pub fn writes(&self) -> Vec<Request> {
        self.0.borrow().writes.clone()
    }

    /// Forget recorded writes, progress, logs and HTTP requests.
    pub fn clear_records(&self) {
        let mut s = self.0.borrow_mut();
        s.writes.clear();
        s.progress.clear();
        s.logs.clear();
        s.http_log.clear();
    }

    /// Every progress report so far.
    pub fn progress(&self) -> Vec<ProgressReport> {
        self.0.borrow().progress.clone()
    }

    /// Every log line so far.
    pub fn logs(&self) -> Vec<(Level, String)> {
        self.0.borrow().logs.clone()
    }

    /// URLs of every HTTP request so far, allowed or not.
    pub fn http_requests(&self) -> Vec<String> {
        self.0.borrow().http_log.clone()
    }

    // ----- request handling -------------------------------------------

    fn recount(s: &mut State) {
        let mut counts: BTreeMap<AlbumId, u64> = BTreeMap::new();
        for f in s.files.values().filter(|f| !s.trashed.contains(&f.id)) {
            for a in &f.album_ids {
                *counts.entry(*a).or_default() += 1;
            }
        }
        for a in s.albums.values_mut() {
            a.file_count = Some(counts.get(&a.id).copied().unwrap_or(0));
        }
    }

    fn method_name(req: &Request) -> String {
        serde_json::to_value(req)
            .ok()
            .and_then(|v| v.get("method").and_then(Value::as_str).map(str::to_owned))
            .unwrap_or_default()
    }

    /// Handle one request, recording it in `writes` if it is a successful
    /// mutation.
    fn handle(&self, req: Request) -> Result<Value, HostError> {
        let is_write = !matches!(
            req,
            Request::ListFiles(_)
                | Request::GetFile { .. }
                | Request::ListAlbums
                | Request::KvGet { .. }
                | Request::Progress { .. }
        );
        let result = self.apply(req.clone());
        if is_write && result.is_ok() {
            self.0.borrow_mut().writes.push(req);
        }
        result
    }

    fn apply(&self, req: Request) -> Result<Value, HostError> {
        let mut s = self.0.borrow_mut();
        if let Some(p) = req.required_permission()
            && !s.granted.contains(&p)
        {
            return Err(err(
                ErrorCode::PermissionDenied,
                format!("{p} is not granted"),
            ));
        }
        if let Some(e) = s.failures.get(&Self::method_name(&req)) {
            return Err(e.clone());
        }
        let live = |s: &State, id: FileId| s.files.contains_key(&id) && !s.trashed.contains(&id);
        let owned_album = |s: &State, id: AlbumId| -> Result<(), HostError> {
            match s.albums.get(&id) {
                None => Err(err(ErrorCode::NotFound, format!("album {id} not found"))),
                Some(a)
                    if !a.is_owned || a.kind != AlbumKind::Album && a.kind != AlbumKind::Folder =>
                {
                    Err(err(
                        ErrorCode::InvalidRequest,
                        format!("album {id} is not owned"),
                    ))
                }
                Some(_) => Ok(()),
            }
        };
        let check_files = |s: &State, ids: &[FileId]| -> Result<(), HostError> {
            match ids.iter().find(|&&id| !live(s, id)) {
                Some(id) => Err(err(ErrorCode::NotFound, format!("file {id} not found"))),
                None => Ok(()),
            }
        };
        match req {
            Request::ListFiles(q) => {
                let limit = q.limit.unwrap_or(s.page_size).clamp(1, s.page_size) as usize;
                let start: usize = match &q.cursor {
                    None => 0,
                    Some(c) => c
                        .parse()
                        .map_err(|_| err(ErrorCode::InvalidRequest, "bad cursor"))?,
                };
                let matching: Vec<&FileInfo> = s
                    .files
                    .values()
                    .filter(|f| !s.trashed.contains(&f.id))
                    .filter(|f| q.album_id.is_none_or(|a| f.album_ids.contains(&a)))
                    .filter(|f| q.created_after.is_none_or(|t| f.creation_time >= t))
                    .filter(|f| q.created_before.is_none_or(|t| f.creation_time < t))
                    .filter(|f| q.file_type.is_none_or(|t| f.file_type == t))
                    .collect();
                let files: Vec<FileInfo> = matching
                    .iter()
                    .skip(start)
                    .take(limit)
                    .map(|f| (*f).clone())
                    .collect();
                let end = start + files.len();
                let next_cursor = (end < matching.len()).then(|| end.to_string());
                Ok(to_json(&FilePage { files, next_cursor }))
            }
            Request::GetFile { file_id } => {
                let f = live(&s, file_id).then(|| s.files[&file_id].clone());
                Ok(to_json(&f))
            }
            Request::ListAlbums => Ok(to_json(&s.albums.values().cloned().collect::<Vec<_>>())),
            Request::AddTags { file_id, tags } => {
                check_files(&s, &[file_id])?;
                let f = s.files.get_mut(&file_id).expect("checked");
                for t in tags {
                    if !f.tags.contains(&t) {
                        f.tags.push(t);
                    }
                }
                Ok(Value::Null)
            }
            Request::EnsureAlbum { name } => {
                if name.trim().is_empty() {
                    return Err(err(ErrorCode::InvalidRequest, "album name is empty"));
                }
                if let Some(a) = s
                    .albums
                    .values()
                    .find(|a| a.is_owned && a.kind == AlbumKind::Album && a.name == name)
                {
                    return Ok(to_json(a));
                }
                let id = s.next_album_id;
                s.next_album_id += 1;
                let album = AlbumInfo {
                    id,
                    name,
                    kind: AlbumKind::Album,
                    is_owned: true,
                    file_count: Some(0),
                };
                s.albums.insert(id, album.clone());
                Ok(to_json(&album))
            }
            Request::AddToAlbum { album_id, file_ids } => {
                owned_album(&s, album_id)?;
                check_files(&s, &file_ids)?;
                for id in file_ids {
                    let f = s.files.get_mut(&id).expect("checked");
                    if !f.album_ids.contains(&album_id) {
                        f.album_ids.push(album_id);
                    }
                }
                Self::recount(&mut s);
                Ok(Value::Null)
            }
            Request::RemoveFromAlbum { album_id, file_ids } => {
                owned_album(&s, album_id)?;
                for id in file_ids {
                    if let Some(f) = s.files.get_mut(&id) {
                        f.album_ids.retain(|a| *a != album_id);
                    }
                }
                Self::recount(&mut s);
                Ok(Value::Null)
            }
            Request::TrashFiles { file_ids } => {
                check_files(&s, &file_ids)?;
                s.trashed.extend(file_ids);
                Self::recount(&mut s);
                Ok(Value::Null)
            }
            Request::ExportFile {
                file_id,
                path,
                overwrite,
            } => {
                check_files(&s, &[file_id])?;
                if !crate::path::is_safe_relative_path(&path) {
                    return Err(err(
                        ErrorCode::InvalidRequest,
                        format!("unsafe export path `{path}`"),
                    ));
                }
                if s.exports.contains_key(&path) && !overwrite {
                    return Ok(to_json(&ExportResult {
                        written: false,
                        bytes: 0,
                    }));
                }
                let bytes = s
                    .originals
                    .get(&file_id)
                    .map(|b| b.len() as u64)
                    .or(s.files[&file_id].size)
                    .unwrap_or(0);
                s.exports.insert(path, Exported { file_id, bytes });
                Ok(to_json(&ExportResult {
                    written: true,
                    bytes,
                }))
            }
            Request::KvGet { key } => {
                check_key(&key)?;
                Ok(to_json(&s.kv.get(&key)))
            }
            Request::KvSet { key, value } => {
                check_key(&key)?;
                if value.is_null() {
                    s.kv.remove(&key);
                    return Ok(Value::Null);
                }
                let mut next = s.kv.clone();
                next.insert(key, value);
                let size: usize = next
                    .iter()
                    .map(|(k, v)| k.len() + v.to_string().len())
                    .sum();
                if size > s.kv_limit {
                    return Err(err(ErrorCode::LimitExceeded, "plugin storage is full"));
                }
                s.kv = next;
                Ok(Value::Null)
            }
            Request::Progress {
                done,
                total,
                message,
            } => {
                s.progress.push(ProgressReport {
                    done,
                    total,
                    message,
                });
                Ok(Value::Null)
            }
        }
    }
}

fn check_key(key: &str) -> Result<(), HostError> {
    if key.is_empty() || key.len() > 128 {
        Err(err(ErrorCode::InvalidRequest, "kv key must be 1-128 bytes"))
    } else {
        Ok(())
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("mock values serialise")
}

impl Host for MockHost {
    fn call(&self, request: &[u8]) -> Vec<u8> {
        let response = match serde_json::from_slice::<Request>(request) {
            Ok(req) => match self.handle(req) {
                Ok(v) => Response::Ok(v),
                Err(e) => Response::Err(e),
            },
            Err(e) => Response::Err(err(ErrorCode::InvalidRequest, e.to_string())),
        };
        serde_json::to_vec(&response).expect("responses serialise")
    }

    fn read(&self, request: &[u8]) -> Result<Vec<u8>, HostError> {
        let req: ReadRequest = serde_json::from_slice(request)
            .map_err(|e| err(ErrorCode::InvalidRequest, e.to_string()))?;
        let s = self.0.borrow();
        let (scope, store) = match req.kind {
            ReadKind::Thumbnail => (ReadScope::Thumbnails, &s.thumbnails),
            ReadKind::Original => (ReadScope::Originals, &s.originals),
        };
        if !s.granted.can_read(scope) {
            return Err(err(
                ErrorCode::PermissionDenied,
                format!("{} is not granted", Permission::Read(scope)),
            ));
        }
        if s.trashed.contains(&req.file_id) {
            return Err(err(ErrorCode::NotFound, "file is trashed"));
        }
        store.get(&req.file_id).cloned().ok_or_else(|| {
            err(
                ErrorCode::NotFound,
                format!("file {} not found", req.file_id),
            )
        })
    }

    fn log(&self, level: Level, message: &str) {
        self.0.borrow_mut().logs.push((level, message.to_owned()));
    }

    fn http(&self, request: &HttpRequest, _body: Option<&[u8]>) -> Result<HttpResponse, HostError> {
        let mut s = self.0.borrow_mut();
        s.http_log.push(request.url.clone());
        let host = url_host(&request.url)
            .ok_or_else(|| err(ErrorCode::InvalidRequest, "not an http(s) URL"))?;
        if !s.granted.network.iter().any(|p| host_matches(p, &host)) {
            return Err(err(
                ErrorCode::PermissionDenied,
                format!("network:{host} is not granted"),
            ));
        }
        s.http.get(&request.url).cloned().ok_or_else(|| {
            err(
                ErrorCode::NotFound,
                format!("no mock route for {}", request.url),
            )
        })
    }
}

/// A JSON object, for [`settings_from_manifest`] overrides.
pub fn values(v: Value) -> settings::Values {
    v.as_object().cloned().unwrap_or_default()
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::host;
    use serde_json::json;

    #[test]
    fn pagination_walks_every_page() {
        let h = MockHost::new().install();
        h.set_page_size(3);
        for i in 1..=10 {
            h.add_file(file(i, &format!("{i}.jpg")));
        }
        let ids: Vec<_> = host::files(FileQuery::default())
            .map(|f| f.unwrap().id)
            .collect();
        assert_eq!(ids, (1..=10).collect::<Vec<_>>());
        let page = host::list_files(&FileQuery::default()).unwrap();
        assert_eq!(page.files.len(), 3);
        assert_eq!(page.next_cursor.as_deref(), Some("3"));
    }

    #[test]
    fn albums_and_permissions() {
        let h = MockHost::new().install();
        h.add_file(file(1, "a.jpg"));
        let a = host::ensure_album("X").unwrap();
        assert_eq!(host::ensure_album("X").unwrap().id, a.id);
        host::add_to_album(a.id, &[1]).unwrap();
        host::add_to_album(a.id, &[1]).unwrap();
        assert_eq!(h.album_files(a.id), [1]);
        assert_eq!(h.album(a.id).unwrap().file_count, Some(1));
        assert_eq!(
            host::add_to_album(a.id, &[2]).unwrap_err().code,
            ErrorCode::NotFound
        );
        h.deny(Permission::Write(WriteScope::Albums));
        assert_eq!(
            host::ensure_album("Y").unwrap_err().code,
            ErrorCode::PermissionDenied
        );
        h.deny(Permission::Read(ReadScope::Metadata));
        assert_eq!(
            host::files(FileQuery::default())
                .next()
                .unwrap()
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        // Empty lists never reach the host.
        assert!(host::add_to_album(a.id, &[]).is_ok());
    }

    #[test]
    fn kv_trash_export_read() {
        let h = MockHost::new().install();
        h.add_file(file(1, "a.jpg")).add_file(file(2, "b.jpg"));
        h.set_original(1, vec![1, 2, 3]).set_thumbnail(1, vec![9]);

        host::kv_set("k", &vec![1, 2]).unwrap();
        assert_eq!(host::kv_get::<Vec<i32>>("k").unwrap(), Some(vec![1, 2]));
        host::kv_delete("k").unwrap();
        assert_eq!(host::kv_get::<Vec<i32>>("k").unwrap(), None);
        assert!(host::kv_set(&"x".repeat(129), &1).is_err());
        h.set_kv_limit(10);
        assert_eq!(
            host::kv_set("big", &"y".repeat(20)).unwrap_err().code,
            ErrorCode::LimitExceeded
        );

        let r = host::export_file(1, "A/a.jpg", false).unwrap();
        assert_eq!(
            r,
            ExportResult {
                written: true,
                bytes: 3
            }
        );
        let r = host::export_file(1, "A/a.jpg", false).unwrap();
        assert!(!r.written);
        assert!(host::export_file(1, "../a.jpg", false).is_err());

        assert_eq!(host::read_original(1).unwrap(), [1, 2, 3]);
        assert_eq!(host::read_thumbnail(1).unwrap(), [9]);

        host::trash_files(&[2]).unwrap();
        assert_eq!(h.trashed(), [2]);
        assert_eq!(host::get_file(2).unwrap(), None);
        assert_eq!(host::files(FileQuery::default()).count(), 1);

        h.deny(Permission::Read(ReadScope::Originals));
        assert_eq!(
            host::read_original(1).unwrap_err().code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(h.writes().len(), 5);
    }

    #[test]
    fn http_respects_granted_hosts() {
        let h = MockHost::new().install();
        let ok = HttpResponse {
            status: 200,
            body: br#"{"a":1}"#.to_vec(),
            ..Default::default()
        };
        h.route("https://api.example.com/x", ok);
        let req = HttpRequest::new("https://api.example.com/x");
        assert_eq!(
            crate::http::request(&req, None).unwrap_err().code,
            ErrorCode::PermissionDenied
        );
        h.grant_only(PermissionSet {
            network: ["*.example.com".to_owned()].into(),
            ..Default::default()
        });
        let res = crate::http::request(&req, None).unwrap();
        assert_eq!(res.json::<Value>().unwrap(), json!({"a": 1}));
        assert_eq!(h.http_requests().len(), 2);
    }

    #[test]
    fn logs_and_progress_and_failures() {
        let h = MockHost::new().install();
        crate::info!("hello {}", 1);
        host::progress(1, Some(2), Some("half")).unwrap();
        assert_eq!(h.logs(), [(Level::Info, "hello 1".to_owned())]);
        assert_eq!(h.progress()[0].total, Some(2));
        h.fail_on("list_albums", err(ErrorCode::Internal, "boom"));
        assert_eq!(host::list_albums().unwrap_err().message, "boom");
    }

    #[test]
    fn no_host_is_an_error() {
        uninstall();
        assert_eq!(host::list_albums().unwrap_err().code, ErrorCode::Internal);
    }

    #[test]
    fn hook_round_trip_and_settings() {
        fn hook(input: &HookInput) -> Result<HookOutput, Error> {
            use crate::HookInputExt;
            let f = input.uploaded_file()?.expect("file");
            Ok(HookOutput {
                summary: Some(f.title),
                changed: 1,
            })
        }
        MockHost::new().install();
        let out = run_hook(hook, &on_upload(&file(5, "x.jpg"))).unwrap();
        assert_eq!(out.summary.as_deref(), Some("x.jpg"));

        let toml = r#"
            [plugin]
            id = "dev.example.t"
            name = "T"
            version = "0.1.0"
            api_version = 1
            [[settings]]
            key = "n"
            label = "N"
            type = "number"
            default = 3
        "#;
        let v = settings_from_manifest(toml, json!({}));
        assert_eq!(v["n"], json!(3.0));
        let v = settings_from_manifest(toml, json!({"n": 4}));
        assert_eq!(v["n"], json!(4));
    }
}
