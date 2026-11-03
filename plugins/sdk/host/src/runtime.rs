//! Running a verified plugin in a sandbox.
//!
//! [`PluginInstance::load`] turns a verified [`Package`] into an Extism plugin
//! with:
//!
//! * a memory cap (the manifest's `limits.memory_mb`, clamped to
//!   [`HostLimits::max_memory_mb`]),
//! * a wall-clock timeout per hook call (the manifest's `limits.timeout_ms`,
//!   clamped to [`HostLimits::max_timeout`]),
//! * an optional fuel (instruction) budget per call,
//! * WASI with no preopened directories, no environment and no arguments,
//!   and no Extism `allowed_paths`, so the plugin has no file system,
//! * outbound HTTP only to the **currently granted** network hosts, with no
//!   redirects and a response size cap,
//! * the two Photovault host functions ([`abi::HOST_CALL`], [`abi::HOST_READ`]).
//!
//! Every host call asks the [`PolicySource`] for the plugin's *current*
//! policy, so disabling the plugin or revoking a permission takes effect on
//! the very next call, even inside a hook that is already running.

use crate::fsutil::{self, prepare_export_target, sanitize_export_path};
use crate::kv::KvStore;
use crate::library::Library;
use crate::package::Package;
use extism::{CurrentPlugin, Function, PTR, PluginBuilder, UserData, Val, ValType};
use photovault_plugin_manifest::abi::{
    self, ErrorCode, ExportResult, FileId, HookEvent, HookInput, HookOutput, HostError, ReadKind,
    ReadRequest, Request, Response, Runner,
};
use photovault_plugin_manifest::settings::{Values, effective_values};
use photovault_plugin_manifest::{Manifest, Permission, PermissionSet, ReadScope};
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// The Extism namespace of the built-in HTTP functions the host overrides.
const EXTISM_ENV: &str = "extism:host/env";

/// Host-side maxima and caps. Manifest limits are clamped to these.
#[derive(Debug, Clone)]
pub struct HostLimits {
    /// Largest memory a plugin may use, in MiB.
    pub max_memory_mb: u32,
    /// Longest wall-clock time of one hook call.
    pub max_timeout: Duration,
    /// Fuel (roughly, Wasm instructions) per hook call; `None` disables fuel
    /// metering. The wall-clock timeout applies either way.
    pub fuel_per_call: Option<u64>,
    /// Largest [`abi::FileQuery::limit`]; larger requests are capped.
    pub max_page_size: u32,
    /// Largest thumbnail or original handed to a plugin, in bytes.
    pub max_read_bytes: u64,
    /// Largest `photovault_call` request, in bytes.
    pub max_request_bytes: u64,
    /// Largest number of file ids or tags in one request.
    pub max_batch: usize,
    /// Largest key/value store per plugin (serialised JSON), in bytes.
    pub max_kv_bytes: u64,
    /// Longest key/value key, in bytes.
    pub max_kv_key_bytes: usize,
    /// Largest HTTP response body, in bytes.
    pub max_http_response_bytes: u64,
    /// Largest Extism variable store, in bytes.
    pub max_var_bytes: u64,
    /// Largest hook output, in bytes.
    pub max_output_bytes: usize,
    /// Use the `HTTPS_PROXY`/`HTTP_PROXY`/`NO_PROXY` environment for plugin
    /// HTTP requests.
    pub http_use_env_proxy: bool,
}

impl Default for HostLimits {
    fn default() -> Self {
        Self {
            max_memory_mb: 256,
            max_timeout: Duration::from_secs(60),
            fuel_per_call: Some(500_000_000_000),
            max_page_size: 500,
            max_read_bytes: 256 * 1024 * 1024,
            max_request_bytes: 1024 * 1024,
            max_batch: 1000,
            max_kv_bytes: 1024 * 1024,
            max_kv_key_bytes: 128,
            max_http_response_bytes: 16 * 1024 * 1024,
            max_var_bytes: 1024 * 1024,
            max_output_bytes: 64 * 1024,
            http_use_env_proxy: true,
        }
    }
}

/// What the user currently allows a plugin to do. Looked up on every host
/// call.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Policy {
    /// Whether the plugin is enabled. A disabled plugin's host calls fail.
    pub enabled: bool,
    /// The granted permissions (the runtime additionally intersects them with
    /// what the manifest requests).
    pub granted: PermissionSet,
    /// Stored settings values (merged with defaults before each hook).
    pub settings: Values,
    /// The folder `ExportFile` writes into, chosen by the user.
    pub export_root: Option<PathBuf>,
    /// Last successful scheduled run, µs since the epoch.
    pub last_run: Option<i64>,
}

/// Where the runtime reads a plugin's current [`Policy`] from. The
/// [`crate::registry::Registry`] implements it; [`SharedPolicy`] is a simple
/// in-memory one.
pub trait PolicySource: Send + Sync {
    /// The current policy of `plugin_id`, or `None` if it is not installed.
    fn policy(&self, plugin_id: &str) -> Option<Policy>;
}

/// A [`PolicySource`] holding one policy in memory, returned for any plugin
/// id. Useful for embedding a single plugin and for tests.
#[derive(Debug, Default)]
pub struct SharedPolicy(RwLock<Policy>);

impl SharedPolicy {
    /// Wrap a policy.
    pub fn new(policy: Policy) -> Self {
        Self(RwLock::new(policy))
    }

    /// Change the policy in place; takes effect on the next host call.
    pub fn update(&self, f: impl FnOnce(&mut Policy)) {
        f(&mut self.0.write().unwrap_or_else(|e| e.into_inner()));
    }
}

impl PolicySource for SharedPolicy {
    fn policy(&self, _plugin_id: &str) -> Option<Policy> {
        Some(self.0.read().unwrap_or_else(|e| e.into_inner()).clone())
    }
}

/// A progress report from a plugin ([`Request::Progress`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressUpdate {
    /// Items done.
    pub done: u64,
    /// Items in total, if known.
    pub total: Option<u64>,
    /// Short status text.
    pub message: Option<String>,
}

/// Receives progress reports: `(plugin_id, update)`.
pub type ProgressFn = Arc<dyn Fn(&str, &ProgressUpdate) + Send + Sync>;

/// Everything a plugin instance needs from its embedder.
#[derive(Clone)]
pub struct InstanceEnv {
    /// The user's library.
    pub library: Arc<dyn Library>,
    /// Where the current policy comes from.
    pub policy: Arc<dyn PolicySource>,
    /// The plugin's key/value file.
    pub kv_path: PathBuf,
    /// Host limits.
    pub limits: HostLimits,
    /// Where the plugin runs, passed to hooks.
    pub runner: Runner,
    /// Progress callback.
    pub progress: Option<ProgressFn>,
}

/// Why a hook could not run or failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    /// The plugin is not installed (any more).
    #[error("plugin is not installed")]
    NotInstalled,
    /// The plugin is disabled.
    #[error("plugin is disabled")]
    Disabled,
    /// The manifest does not declare this hook.
    #[error("the plugin does not declare the `{0}` hook")]
    HookNotDeclared(&'static str),
    /// The module does not export a declared hook.
    #[error("the plugin does not export `{0}`")]
    HookNotExported(&'static str),
    /// The on-demand action is not in the manifest.
    #[error("unknown on-demand action `{0}`")]
    UnknownAction(String),
    /// The module could not be compiled or instantiated.
    #[error("could not load plugin: {0}")]
    Load(String),
    /// The hook ran longer than its timeout.
    #[error("plugin timed out")]
    Timeout,
    /// The hook used up its fuel budget.
    #[error("plugin ran out of fuel")]
    OutOfFuel,
    /// The hook trapped or returned an error.
    #[error("plugin failed: {0}")]
    Failed(String),
    /// The hook returned output that is not a valid `HookOutput`.
    #[error("plugin returned invalid output: {0}")]
    BadOutput(String),
    /// A host-side operation needed to build the hook input failed.
    #[error("host error: {0}")]
    Host(#[from] HostError),
}

/// The current time in µs since the Unix epoch.
pub fn now_micros() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Host context
// ---------------------------------------------------------------------------

struct Ctx {
    plugin_id: String,
    requested: PermissionSet,
    library: Arc<dyn Library>,
    policy: Arc<dyn PolicySource>,
    kv: KvStore,
    limits: HostLimits,
    progress: Option<ProgressFn>,
    agent: ureq::Agent,
    http_status: u16,
    http_headers: BTreeMap<String, String>,
}

fn err(code: ErrorCode, msg: impl Into<String>) -> HostError {
    HostError::new(code, msg)
}

fn to_value<T: serde::Serialize>(v: T) -> Result<Value, HostError> {
    serde_json::to_value(v).map_err(|e| err(ErrorCode::Internal, e.to_string()))
}

impl Ctx {
    /// The granted permissions in force right now, or an error if the plugin
    /// is disabled or removed.
    fn current_grants(&self) -> Result<(PermissionSet, Policy), HostError> {
        let policy = self
            .policy
            .policy(&self.plugin_id)
            .ok_or_else(|| err(ErrorCode::PermissionDenied, "the plugin has been removed"))?;
        if !policy.enabled {
            return Err(err(ErrorCode::PermissionDenied, "the plugin is disabled"));
        }
        Ok((policy.granted.intersection(&self.requested), policy))
    }

    fn require(&self, p: &Permission) -> Result<Policy, HostError> {
        let (granted, policy) = self.current_grants()?;
        if !granted.contains(p) {
            return Err(err(
                ErrorCode::PermissionDenied,
                format!("permission `{p}` is not granted"),
            ));
        }
        Ok(policy)
    }

    fn check_batch(&self, n: usize) -> Result<(), HostError> {
        if n > self.limits.max_batch {
            return Err(err(
                ErrorCode::LimitExceeded,
                format!("at most {} items per request", self.limits.max_batch),
            ));
        }
        Ok(())
    }

    fn owned_album(&self, album_id: abi::AlbumId) -> Result<(), HostError> {
        match self
            .library
            .list_albums()?
            .into_iter()
            .find(|a| a.id == album_id)
        {
            Some(a) if a.is_owned => Ok(()),
            Some(_) => Err(err(
                ErrorCode::PermissionDenied,
                "plugins may only change albums the user owns",
            )),
            None => Err(err(ErrorCode::NotFound, "album not found")),
        }
    }

    fn handle_call(&self, bytes: &[u8]) -> Response {
        let req: Request = match serde_json::from_slice(bytes) {
            Ok(r) => r,
            Err(e) => return Response::Err(err(ErrorCode::InvalidRequest, e.to_string())),
        };
        let policy = match req.required_permission() {
            Some(p) => self.require(&p),
            None => self.current_grants().map(|(_, p)| p),
        };
        let result = policy.and_then(|policy| self.dispatch(req, policy));
        match result {
            Ok(v) => Response::Ok(v),
            Err(e) => Response::Err(e),
        }
    }

    fn dispatch(&self, req: Request, policy: Policy) -> Result<Value, HostError> {
        let lib = &*self.library;
        match req {
            Request::ListFiles(mut q) => {
                let max = self.limits.max_page_size;
                q.limit = Some(q.limit.unwrap_or(max).clamp(1, max));
                let mut page = lib.list_files(&q)?;
                page.files.truncate(max as usize);
                to_value(page)
            }
            Request::GetFile { file_id } => to_value(lib.get_file(file_id)?),
            Request::ListAlbums => to_value(lib.list_albums()?),
            Request::AddTags { file_id, tags } => {
                self.check_batch(tags.len())?;
                if tags.iter().any(|t| {
                    t.trim().is_empty()
                        || t.chars().count() > 128
                        || t.chars().any(char::is_control)
                }) {
                    return Err(err(
                        ErrorCode::InvalidRequest,
                        "tags must be 1-128 characters without control characters",
                    ));
                }
                lib.get_file(file_id)?
                    .ok_or_else(|| err(ErrorCode::NotFound, "file not found"))?;
                lib.add_tags(file_id, &tags)?;
                Ok(Value::Null)
            }
            Request::EnsureAlbum { name } => {
                if name.trim().is_empty()
                    || name.chars().count() > 256
                    || name.chars().any(char::is_control)
                {
                    return Err(err(
                        ErrorCode::InvalidRequest,
                        "album names must be 1-256 characters without control characters",
                    ));
                }
                to_value(lib.ensure_album(&name)?)
            }
            Request::AddToAlbum { album_id, file_ids } => {
                self.check_batch(file_ids.len())?;
                self.owned_album(album_id)?;
                lib.add_to_album(album_id, &file_ids)?;
                Ok(Value::Null)
            }
            Request::RemoveFromAlbum { album_id, file_ids } => {
                self.check_batch(file_ids.len())?;
                self.owned_album(album_id)?;
                lib.remove_from_album(album_id, &file_ids)?;
                Ok(Value::Null)
            }
            Request::TrashFiles { file_ids } => {
                self.check_batch(file_ids.len())?;
                lib.trash_files(&file_ids)?;
                Ok(Value::Null)
            }
            Request::ExportFile {
                file_id,
                path,
                overwrite,
            } => to_value(self.export(&policy, file_id, &path, overwrite)?),
            Request::KvGet { key } => to_value(self.kv.get(&key)?),
            Request::KvSet { key, value } => {
                self.kv.set(&key, value)?;
                Ok(Value::Null)
            }
            Request::Progress {
                done,
                total,
                message,
            } => {
                if let Some(cb) = &self.progress {
                    let message = message.map(|m| m.chars().take(256).collect());
                    cb(
                        &self.plugin_id,
                        &ProgressUpdate {
                            done,
                            total,
                            message,
                        },
                    );
                }
                Ok(Value::Null)
            }
        }
    }

    fn export(
        &self,
        policy: &Policy,
        file_id: FileId,
        path: &str,
        overwrite: bool,
    ) -> Result<ExportResult, HostError> {
        let components = sanitize_export_path(path)
            .map_err(|e| err(ErrorCode::InvalidRequest, e.to_string()))?;
        let root = policy.export_root.as_ref().ok_or_else(|| {
            err(
                ErrorCode::InvalidRequest,
                "no export folder has been chosen for this plugin",
            )
        })?;
        self.library
            .get_file(file_id)?
            .ok_or_else(|| err(ErrorCode::NotFound, "file not found"))?;
        let (target, exists) = prepare_export_target(root, &components)
            .map_err(|e| err(ErrorCode::InvalidRequest, e.to_string()))?;
        if exists && !overwrite {
            return Ok(ExportResult {
                written: false,
                bytes: 0,
            });
        }
        let mut lib_err = None;
        let res = fsutil::write_atomic_with(&target, |f| {
            self.library.write_original(file_id, f).map_err(|e| {
                lib_err = Some(e);
                std::io::Error::other("library error")
            })
        });
        match (res, lib_err) {
            (Ok(bytes), _) => Ok(ExportResult {
                written: true,
                bytes,
            }),
            (Err(_), Some(e)) => Err(e),
            (Err(e), None) => Err(err(ErrorCode::Internal, format!("export failed: {e}"))),
        }
    }

    fn handle_read(&self, bytes: &[u8]) -> Result<Vec<u8>, HostError> {
        let req: ReadRequest = serde_json::from_slice(bytes)
            .map_err(|e| err(ErrorCode::InvalidRequest, e.to_string()))?;
        let scope = match req.kind {
            ReadKind::Thumbnail => ReadScope::Thumbnails,
            ReadKind::Original => ReadScope::Originals,
        };
        self.require(&Permission::Read(scope))?;
        let max = self.limits.max_read_bytes;
        let too_big = || {
            err(
                ErrorCode::LimitExceeded,
                format!("files larger than {max} bytes cannot be read by plugins"),
            )
        };
        let data = match req.kind {
            ReadKind::Thumbnail => self.library.thumbnail(req.file_id)?,
            ReadKind::Original => {
                if let Some(info) = self.library.get_file(req.file_id)?
                    && info.size.is_some_and(|s| s > max)
                {
                    return Err(too_big());
                }
                self.library.original(req.file_id)?
            }
        };
        if data.len() as u64 > max {
            return Err(too_big());
        }
        Ok(data)
    }

    fn host_allowed(&self, host: &str) -> Result<bool, HostError> {
        let (granted, _) = self.current_grants()?;
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        Ok(granted.network.iter().any(|p| match p.strip_prefix("*.") {
            Some(suffix) => host
                .strip_suffix(suffix)
                .is_some_and(|rest| rest.len() > 1 && rest.ends_with('.')),
            None => *p == host,
        }))
    }
}

type HostResult = Result<(), extism::Error>;

fn read_input(
    plugin: &mut CurrentPlugin,
    v: &Val,
    max: u64,
) -> Result<Result<Vec<u8>, HostError>, extism::Error> {
    let handle = plugin
        .memory_from_val(v)
        .ok_or_else(|| extism::Error::msg("invalid memory handle"))?;
    if handle.len() as u64 > max {
        return Ok(Err(err(
            ErrorCode::LimitExceeded,
            format!("requests are limited to {max} bytes"),
        )));
    }
    Ok(Ok(plugin.memory_bytes(handle)?.to_vec()))
}

fn with_ctx<T>(ud: &UserData<Ctx>, f: impl FnOnce(&mut Ctx) -> T) -> Result<T, extism::Error> {
    let arc = ud.get()?;
    let mut guard = arc.lock().unwrap_or_else(|e| e.into_inner());
    Ok(f(&mut guard))
}

fn host_call(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    ud: UserData<Ctx>,
) -> HostResult {
    let max = with_ctx(&ud, |c| c.limits.max_request_bytes)?;
    let response = match read_input(plugin, &inputs[0], max)? {
        Ok(bytes) => with_ctx(&ud, |c| c.handle_call(&bytes))?,
        Err(e) => Response::Err(e),
    };
    let json = serde_json::to_vec(&response)?;
    let handle = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

fn host_read(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    ud: UserData<Ctx>,
) -> HostResult {
    let max = with_ctx(&ud, |c| c.limits.max_request_bytes)?;
    let result = match read_input(plugin, &inputs[0], max)? {
        Ok(bytes) => with_ctx(&ud, |c| c.handle_read(&bytes))?,
        Err(e) => Err(e),
    };
    match result {
        Ok(data) => {
            let handle = plugin.memory_new(&data)?;
            outputs[0] = plugin.memory_to_val(handle);
            Ok(())
        }
        Err(e) => Err(extism::Error::msg(serde_json::to_string(&e)?)),
    }
}

/// The JSON `extism_pdk::http::request` sends.
#[derive(Deserialize)]
struct HttpRequestJson {
    url: String,
    #[serde(default)]
    headers: BTreeMap<String, String>,
    #[serde(default)]
    method: Option<String>,
}

/// Replacement for Extism's `http_request`: checks the *current* grants on
/// every request, never follows redirects and caps the response size.
fn host_http_request(
    plugin: &mut CurrentPlugin,
    inputs: &[Val],
    outputs: &mut [Val],
    ud: UserData<Ctx>,
) -> HostResult {
    outputs[0] = Val::I64(0);
    let req_offset = inputs[0]
        .i64()
        .ok_or_else(|| extism::Error::msg("bad argument"))? as u64;
    let body_offset = inputs[1]
        .i64()
        .ok_or_else(|| extism::Error::msg("bad argument"))? as u64;
    let req_handle = plugin
        .memory_handle(req_offset)
        .ok_or_else(|| extism::Error::msg("invalid HTTP request handle"))?;
    let req: HttpRequestJson = serde_json::from_slice(plugin.memory_bytes(req_handle)?)?;
    plugin.memory_free(req_handle)?;
    let body = match plugin.memory_handle(body_offset) {
        Some(h) if body_offset != 0 => {
            let b = plugin.memory_bytes(h)?.to_vec();
            plugin.memory_free(h)?;
            Some(b)
        }
        _ => None,
    };
    let timeout = plugin.time_remaining();

    let uri: ureq::http::Uri = req
        .url
        .parse()
        .map_err(|e| extism::Error::msg(format!("Invalid URL: {e}")))?;
    let scheme_ok = matches!(uri.scheme_str(), Some("http") | Some("https"));
    let host = uri.host().unwrap_or_default().to_owned();

    let (agent, max, allowed) = with_ctx(&ud, |c| {
        c.http_status = 0;
        c.http_headers.clear();
        (
            c.agent.clone(),
            c.limits.max_http_response_bytes,
            c.host_allowed(&host),
        )
    })?;
    match allowed {
        Ok(true) if scheme_ok => {}
        _ => {
            return Err(extism::Error::msg(format!(
                "HTTP request to {} is not allowed",
                req.url
            )));
        }
    }

    let mut builder = ureq::http::Request::builder()
        .method(
            req.method
                .as_deref()
                .unwrap_or("GET")
                .to_ascii_uppercase()
                .as_str(),
        )
        .uri(uri);
    for (k, v) in &req.headers {
        builder = builder.header(k, v);
    }
    let result = match body {
        Some(b) => {
            let r = agent
                .configure_request(builder.body(b)?)
                .timeout_global(timeout)
                .build();
            agent.run(r)
        }
        None => {
            let r = agent
                .configure_request(builder.body(())?)
                .timeout_global(timeout)
                .build();
            agent.run(r)
        }
    };
    let response = result.map_err(|e| extism::Error::msg(format!("HTTP request failed: {e}")))?;
    let status = response.status().as_u16();
    let headers: BTreeMap<String, String> = response
        .headers()
        .iter()
        .filter_map(|(k, v)| Some((k.as_str().to_owned(), v.to_str().ok()?.to_owned())))
        .collect();
    let mut buf = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(max + 1)
        .read_to_end(&mut buf)?;
    if buf.len() as u64 > max {
        return Err(extism::Error::msg(format!(
            "HTTP response exceeds the maximum of {max} bytes"
        )));
    }
    with_ctx(&ud, |c| {
        c.http_status = status;
        c.http_headers = headers;
    })?;
    let handle = plugin.memory_new(&buf)?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

fn host_http_status(
    _: &mut CurrentPlugin,
    _: &[Val],
    outputs: &mut [Val],
    ud: UserData<Ctx>,
) -> HostResult {
    outputs[0] = Val::I32(with_ctx(&ud, |c| c.http_status)? as i32);
    Ok(())
}

fn host_http_headers(
    plugin: &mut CurrentPlugin,
    _: &[Val],
    outputs: &mut [Val],
    ud: UserData<Ctx>,
) -> HostResult {
    let json = with_ctx(&ud, |c| serde_json::to_string(&c.http_headers))??;
    let handle = plugin.memory_new(&json)?;
    outputs[0] = plugin.memory_to_val(handle);
    Ok(())
}

// ---------------------------------------------------------------------------
// Instances
// ---------------------------------------------------------------------------

/// A loaded, sandboxed plugin, ready to run hooks.
pub struct PluginInstance {
    plugin: extism::Plugin,
    manifest: Manifest,
    policy: Arc<dyn PolicySource>,
    library: Arc<dyn Library>,
    runner: Runner,
    timeout: Duration,
    max_output: usize,
}

impl std::fmt::Debug for PluginInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginInstance")
            .field("id", &self.manifest.plugin.id)
            .field("version", &self.manifest.plugin.version)
            .finish_non_exhaustive()
    }
}

impl PluginInstance {
    /// Compile and instantiate a verified package in the sandbox.
    pub fn load(package: &Package, env: InstanceEnv) -> Result<Self, RuntimeError> {
        let manifest = package.manifest().clone();
        let limits = env.limits.clone();
        let memory_mb = manifest.limits.memory_mb.min(limits.max_memory_mb).max(1);
        let timeout = Duration::from_millis(manifest.limits.timeout_ms).min(limits.max_timeout);

        // The built-in HTTP function is replaced below; this list only
        // matters as a second line of defence.
        let initial_hosts: Vec<String> = env
            .policy
            .policy(&manifest.plugin.id)
            .map(|p| {
                p.granted
                    .network
                    .intersection(&manifest.permissions.network)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();

        let mut ext_manifest = extism::Manifest::new([extism::Wasm::data(package.wasm().to_vec())])
            .with_memory_max(memory_mb.saturating_mul(16))
            .with_timeout(timeout)
            .with_allowed_hosts(initial_hosts.into_iter());
        ext_manifest.memory.max_http_response_bytes = Some(limits.max_http_response_bytes);
        ext_manifest.memory.max_var_bytes = Some(limits.max_var_bytes);
        debug_assert!(ext_manifest.allowed_paths.is_none());

        let proxy = if limits.http_use_env_proxy {
            ureq::Proxy::try_from_env()
        } else {
            None
        };
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            .proxy(proxy)
            .build()
            .into();

        let ctx = UserData::new(Ctx {
            plugin_id: manifest.plugin.id.clone(),
            requested: manifest.permissions.clone(),
            library: env.library.clone(),
            policy: env.policy.clone(),
            kv: KvStore::new(
                env.kv_path.clone(),
                limits.max_kv_bytes,
                limits.max_kv_key_bytes,
            ),
            limits: limits.clone(),
            progress: env.progress.clone(),
            agent,
            http_status: 0,
            http_headers: BTreeMap::new(),
        });

        let functions = [
            Function::new(abi::HOST_CALL, [PTR], [PTR], ctx.clone(), host_call),
            Function::new(abi::HOST_READ, [PTR], [PTR], ctx.clone(), host_read),
            Function::new(
                "http_request",
                [ValType::I64, ValType::I64],
                [ValType::I64],
                ctx.clone(),
                host_http_request,
            )
            .with_namespace(EXTISM_ENV),
            Function::new(
                "http_status_code",
                [],
                [ValType::I32],
                ctx.clone(),
                host_http_status,
            )
            .with_namespace(EXTISM_ENV),
            Function::new("http_headers", [], [ValType::I64], ctx, host_http_headers)
                .with_namespace(EXTISM_ENV),
        ];

        let mut builder = PluginBuilder::new(ext_manifest)
            .with_wasi(true)
            .with_cache_disabled()
            .with_functions(functions);
        if let Some(fuel) = limits.fuel_per_call {
            builder = builder.with_fuel_limit(fuel);
        }
        let plugin = builder
            .build()
            .map_err(|e| RuntimeError::Load(format!("{e:#}")))?;
        Ok(Self {
            plugin,
            manifest,
            policy: env.policy,
            library: env.library,
            runner: env.runner,
            timeout,
            max_output: limits.max_output_bytes,
        })
    }

    /// The plugin's manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The plugin id.
    pub fn id(&self) -> &str {
        &self.manifest.plugin.id
    }

    /// The effective timeout of one hook call.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// A handle that cancels the running hook from another thread.
    pub fn cancel_handle(&self) -> extism::CancelHandle {
        self.plugin.cancel_handle()
    }

    fn policy(&self) -> Result<Policy, RuntimeError> {
        let p = self
            .policy
            .policy(&self.manifest.plugin.id)
            .ok_or(RuntimeError::NotInstalled)?;
        if !p.enabled {
            return Err(RuntimeError::Disabled);
        }
        Ok(p)
    }

    fn input(&self, event: HookEvent, policy: &Policy) -> HookInput {
        HookInput {
            event,
            settings: effective_values(&self.manifest.settings, &policy.settings),
            now: now_micros(),
            runner: self.runner,
        }
    }

    fn invoke(
        &mut self,
        export: &'static str,
        input: &HookInput,
    ) -> Result<HookOutput, RuntimeError> {
        if !self.plugin.function_exists(export) {
            return Err(RuntimeError::HookNotExported(export));
        }
        let bytes = serde_json::to_vec(input).map_err(|e| RuntimeError::Failed(e.to_string()))?;
        let started = Instant::now();
        let result = self.plugin.call::<&[u8], &[u8]>(export, &bytes);
        let out = match result {
            Ok(out) => out,
            Err(e) => {
                let msg = format!("{e:#}");
                if msg.contains("out of fuel") {
                    return Err(RuntimeError::OutOfFuel);
                }
                if msg == "timeout"
                    || (msg.contains("timeout") && started.elapsed() >= self.timeout)
                {
                    return Err(RuntimeError::Timeout);
                }
                return Err(RuntimeError::Failed(msg));
            }
        };
        if out.len() > self.max_output {
            return Err(RuntimeError::BadOutput(format!(
                "output is larger than {} bytes",
                self.max_output
            )));
        }
        if out.iter().all(u8::is_ascii_whitespace) {
            return Ok(HookOutput::default());
        }
        serde_json::from_slice(out).map_err(|e| RuntimeError::BadOutput(e.to_string()))
    }

    /// Run `on_upload` for a newly uploaded file. The hook input includes the
    /// file's metadata only if `read:metadata` is currently granted.
    pub fn run_upload(&mut self, file_id: FileId) -> Result<HookOutput, RuntimeError> {
        if !self.manifest.hooks.on_upload {
            return Err(RuntimeError::HookNotDeclared(abi::EXPORT_ON_UPLOAD));
        }
        let policy = self.policy()?;
        let metadata = Permission::Read(ReadScope::Metadata);
        let file = if policy.granted.contains(&metadata)
            && self.manifest.permissions.contains(&metadata)
        {
            self.library.get_file(file_id)?
        } else {
            None
        };
        let input = self.input(HookEvent::Upload { file_id, file }, &policy);
        self.invoke(abi::EXPORT_ON_UPLOAD, &input)
    }

    /// Run `on_schedule`. `last_run` comes from the policy.
    pub fn run_schedule(&mut self) -> Result<HookOutput, RuntimeError> {
        if self.manifest.hooks.schedule.is_none() {
            return Err(RuntimeError::HookNotDeclared(abi::EXPORT_ON_SCHEDULE));
        }
        let policy = self.policy()?;
        let input = self.input(
            HookEvent::Schedule {
                last_run: policy.last_run,
            },
            &policy,
        );
        self.invoke(abi::EXPORT_ON_SCHEDULE, &input)
    }

    /// Run `on_demand` with one of the manifest's action ids.
    pub fn run_on_demand(&mut self, action: &str) -> Result<HookOutput, RuntimeError> {
        if self.manifest.hooks.on_demand.is_empty() {
            return Err(RuntimeError::HookNotDeclared(abi::EXPORT_ON_DEMAND));
        }
        if !self.manifest.hooks.on_demand.iter().any(|a| a.id == action) {
            return Err(RuntimeError::UnknownAction(action.to_owned()));
        }
        let policy = self.policy()?;
        let input = self.input(
            HookEvent::OnDemand {
                action: action.to_owned(),
            },
            &policy,
        );
        self.invoke(abi::EXPORT_ON_DEMAND, &input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_matching() {
        let policy = Arc::new(SharedPolicy::new(Policy {
            enabled: true,
            granted: PermissionSet {
                network: ["api.example.com".into(), "*.cdn.example.org".into()].into(),
                ..Default::default()
            },
            ..Default::default()
        }));
        let requested = policy.policy("x").unwrap().granted;
        let ctx = Ctx {
            plugin_id: "a.b".into(),
            requested,
            library: Arc::new(NoLibrary),
            policy: policy.clone(),
            kv: KvStore::new("/nonexistent/kv.json", 10, 10),
            limits: HostLimits::default(),
            progress: None,
            agent: ureq::Agent::new_with_defaults(),
            http_status: 0,
            http_headers: BTreeMap::new(),
        };
        assert!(ctx.host_allowed("api.example.com").unwrap());
        assert!(ctx.host_allowed("API.example.com.").unwrap());
        assert!(!ctx.host_allowed("evil-api.example.com").unwrap());
        assert!(!ctx.host_allowed("example.com").unwrap());
        assert!(ctx.host_allowed("a.cdn.example.org").unwrap());
        assert!(ctx.host_allowed("a.b.cdn.example.org").unwrap());
        assert!(!ctx.host_allowed("cdn.example.org").unwrap());
        assert!(!ctx.host_allowed("xcdn.example.org").unwrap());
        policy.update(|p| p.enabled = false);
        assert!(ctx.host_allowed("api.example.com").is_err());
    }

    struct NoLibrary;
    impl Library for NoLibrary {
        fn list_files(&self, _: &abi::FileQuery) -> Result<abi::FilePage, HostError> {
            unimplemented!()
        }
        fn get_file(&self, _: FileId) -> Result<Option<abi::FileInfo>, HostError> {
            unimplemented!()
        }
        fn list_albums(&self) -> Result<Vec<abi::AlbumInfo>, HostError> {
            unimplemented!()
        }
        fn thumbnail(&self, _: FileId) -> Result<Vec<u8>, HostError> {
            unimplemented!()
        }
        fn original(&self, _: FileId) -> Result<Vec<u8>, HostError> {
            unimplemented!()
        }
        fn add_tags(&self, _: FileId, _: &[String]) -> Result<(), HostError> {
            unimplemented!()
        }
        fn ensure_album(&self, _: &str) -> Result<abi::AlbumInfo, HostError> {
            unimplemented!()
        }
        fn add_to_album(&self, _: abi::AlbumId, _: &[FileId]) -> Result<(), HostError> {
            unimplemented!()
        }
        fn remove_from_album(&self, _: abi::AlbumId, _: &[FileId]) -> Result<(), HostError> {
            unimplemented!()
        }
        fn trash_files(&self, _: &[FileId]) -> Result<(), HostError> {
            unimplemented!()
        }
    }
}
