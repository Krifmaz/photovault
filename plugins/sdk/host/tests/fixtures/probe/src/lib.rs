//! Probe plugin for the host runtime's integration tests.
//!
//! It speaks the raw ABI (JSON in, JSON out) instead of the guest SDK so tests
//! can send any request, including malformed ones. The on-demand action
//! decides what to do; its argument comes from the `arg` setting.

#![allow(unsafe_code)]

use extism_pdk::{FnResult, Memory, plugin_fn};
use serde_json::{Value, json};

#[link(wasm_import_module = "extism:host/user")]
unsafe extern "C" {
    fn photovault_call(request: u64) -> u64;
    fn photovault_read(request: u64) -> u64;
}

fn exchange(bytes: &[u8], f: unsafe extern "C" fn(u64) -> u64) -> Vec<u8> {
    let req = Memory::from_bytes(bytes).expect("alloc");
    // SAFETY: the host takes a live memory block and returns a new one.
    let reply = unsafe { f(req.offset()) };
    req.free();
    let mem = Memory::find(reply).expect("host returned a bad pointer");
    let out = mem.to_vec();
    mem.free();
    out
}

fn call(request: &str) -> String {
    String::from_utf8_lossy(&exchange(request.as_bytes(), photovault_call)).into_owned()
}

fn output(summary: String) -> FnResult<Vec<u8>> {
    Ok(serde_json::to_vec(
        &json!({"summary": summary, "changed": 1}),
    )?)
}

/// Echo the hook input.
#[plugin_fn]
pub fn on_upload(input: Vec<u8>) -> FnResult<Vec<u8>> {
    output(String::from_utf8_lossy(&input).into_owned())
}

/// Echo the hook input.
#[plugin_fn]
pub fn on_schedule(input: Vec<u8>) -> FnResult<Vec<u8>> {
    output(String::from_utf8_lossy(&input).into_owned())
}

/// Do whatever the action says.
#[plugin_fn]
pub fn on_demand(input: Vec<u8>) -> FnResult<Vec<u8>> {
    let input: Value = serde_json::from_slice(&input)?;
    let action = input["event"]["action"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let arg = input["settings"]["arg"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    match action.as_str() {
        "call" => output(call(&arg)),
        "progress_then_call" => {
            call(r#"{"method":"progress","params":{"done":1,"total":2,"message":"half"}}"#);
            output(call(&arg))
        }
        "read" => {
            let bytes = exchange(arg.as_bytes(), photovault_read);
            output(format!(
                "{}:{}",
                bytes.len(),
                String::from_utf8_lossy(&bytes)
            ))
        }
        "loop" => {
            let mut x: u64 = 0;
            loop {
                x = std::hint::black_box(x.wrapping_add(1));
            }
        }
        "alloc" => {
            let mib: usize = arg.parse().unwrap_or(64);
            let mut v: Vec<u8> = Vec::new();
            v.resize(mib * 1024 * 1024, 7);
            let sum: u64 = v.iter().step_by(4096).map(|&b| b as u64).sum();
            output(format!("allocated {mib} MiB ({sum})"))
        }
        "http" => {
            let req = extism_pdk::HttpRequest::new(&arg);
            let res = extism_pdk::http::request::<()>(&req, None)?;
            output(format!(
                "{}:{}",
                res.status_code(),
                String::from_utf8_lossy(&res.body())
            ))
        }
        "settings" => output(input["settings"].to_string()),
        "bad_output" => Ok(b"this is not json".to_vec()),
        other => output(format!("unknown action {other}")),
    }
}
