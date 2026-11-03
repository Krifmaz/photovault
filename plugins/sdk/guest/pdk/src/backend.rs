//! The one place where the PDK talks to its host.
//!
//! On `wasm32` every function here calls the real Extism host functions
//! (`photovault_call` and `photovault_read` in `extism:host/user`, plus
//! Extism's built-in logging and HTTP). On native targets the same functions
//! forward to the [`crate::testing::Host`] installed on the current thread,
//! normally a [`crate::testing::MockHost`], so plugin logic can be unit-tested
//! with `cargo test`.
//!
//! Requests and responses always cross this boundary as JSON bytes, exactly as
//! they do in Wasm, so tests exercise the real wire format.

use crate::abi::HostError;
use crate::http::{HttpRequest, HttpResponse};
use crate::log::Level;

#[cfg(target_arch = "wasm32")]
pub(crate) use wasm::*;

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::*;

/// Shorthand for an [`crate::abi::ErrorCode::Internal`] error.
pub(crate) fn internal(message: impl Into<String>) -> HostError {
    HostError::new(crate::abi::ErrorCode::Internal, message)
}

#[cfg(target_arch = "wasm32")]
#[allow(unsafe_code)]
mod wasm {
    use super::*;
    use extism_pdk::Memory;

    #[link(wasm_import_module = "extism:host/user")]
    unsafe extern "C" {
        #[link_name = "photovault_call"]
        fn photovault_call(request: u64) -> u64;
        #[link_name = "photovault_read"]
        fn photovault_read(request: u64) -> u64;
    }

    /// Hand `bytes` to a host function taking and returning one Extism
    /// memory block, and copy the reply out (freeing both blocks).
    fn exchange(bytes: &[u8], f: unsafe extern "C" fn(u64) -> u64) -> Result<Vec<u8>, HostError> {
        let request = Memory::from_bytes(bytes).map_err(|e| internal(e.to_string()))?;
        // SAFETY: the import takes the offset of a live Extism memory block
        // and returns the offset of a block the host allocated (or 0).
        let reply = unsafe { f(request.offset()) };
        request.free();
        if reply == 0 {
            return Err(internal("host returned no data"));
        }
        let memory = Memory::find(reply).ok_or_else(|| internal("host returned a bad pointer"))?;
        let out = memory.to_vec();
        memory.free();
        Ok(out)
    }

    /// `photovault_call`: JSON request in, JSON response out.
    pub(crate) fn call(request: &[u8]) -> Result<Vec<u8>, HostError> {
        exchange(request, photovault_call)
    }

    /// `photovault_read`: JSON request in, raw bytes out. A failed read makes
    /// the host return an Extism error, which ends the hook call with that
    /// error; it does not come back here.
    pub(crate) fn read(request: &[u8]) -> Result<Vec<u8>, HostError> {
        exchange(request, photovault_read)
    }

    /// Extism's log functions, honouring the host's log level.
    pub(crate) fn log(level: Level, message: &str) {
        let level = level.to_extism();
        // SAFETY: plain host call without arguments.
        let threshold = unsafe { extism_pdk::extism::get_log_level() };
        if threshold == i32::MAX || level.to_int() < threshold {
            return;
        }
        if let Ok(memory) = Memory::from_bytes(message) {
            memory.log(level);
            memory.free();
        }
    }

    /// Extism's `http_request`, limited by the host to the granted hosts.
    pub(crate) fn http(
        request: &HttpRequest,
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, HostError> {
        let response = extism_pdk::http::request(request, body.map(|b| b.to_vec()))
            .map_err(|e| internal(e.to_string()))?;
        Ok(HttpResponse {
            status: response.status_code(),
            headers: response
                .headers()
                .iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
                .collect(),
            body: response.body(),
        })
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use crate::testing::Host;
    use std::cell::RefCell;
    use std::rc::Rc;

    thread_local! {
        static HOST: RefCell<Option<Rc<dyn Host>>> = const { RefCell::new(None) };
    }

    /// Install `host` for the current thread, returning the previous one.
    pub(crate) fn install(host: Option<Rc<dyn Host>>) -> Option<Rc<dyn Host>> {
        HOST.with(|h| h.replace(host))
    }

    fn with<T>(f: impl FnOnce(&dyn Host) -> Result<T, HostError>) -> Result<T, HostError> {
        let host = HOST.with(|h| h.borrow().clone());
        match host {
            Some(host) => f(&*host),
            None => Err(internal(
                "no host installed: call MockHost::install() in your test",
            )),
        }
    }

    /// Forward to [`Host::call`].
    pub(crate) fn call(request: &[u8]) -> Result<Vec<u8>, HostError> {
        with(|h| Ok(h.call(request)))
    }

    /// Forward to [`Host::read`].
    pub(crate) fn read(request: &[u8]) -> Result<Vec<u8>, HostError> {
        with(|h| h.read(request))
    }

    /// Forward to [`Host::log`]; without a host, print to stderr.
    pub(crate) fn log(level: Level, message: &str) {
        let host = HOST.with(|h| h.borrow().clone());
        match host {
            Some(host) => host.log(level, message),
            None => eprintln!("[{level:?}] {message}"),
        }
    }

    /// Forward to [`Host::http`].
    pub(crate) fn http(
        request: &HttpRequest,
        body: Option<&[u8]>,
    ) -> Result<HttpResponse, HostError> {
        with(|h| h.http(request, body))
    }
}
