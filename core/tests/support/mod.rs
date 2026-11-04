//! Helpers for the end-to-end tests: a real Photovault server in process,
//! accounts signed up the way the Ente apps do it, and a small proxy that
//! adds the Photovault-only endpoints the base server lacks.
//!
//! Adapted from `importers/tests/real_server.rs` (claude/project-thread-cp9x81).

#![allow(dead_code)]

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use photovault_core::api::Api;
use photovault_core::crypto;
use photovault_core::srp::{SrpClient, verifier};
use photovault_core::{Core, CoreError};
use photovault_server::testing::TestServer;
use reqwest::Method;
use serde_json::{Value, json};
use std::sync::Arc;

/// The email code the test server accepts for every test account.
pub const OTT: &str = "424242";
/// Argon2 limits whose product is the strength the server requires
/// (like the apps' "sensitive" setting), cheaper on memory.
pub const OPS_LIMIT: u64 = 32;
/// See [`OPS_LIMIT`].
pub const MEM_LIMIT: u64 = 128 * 1024 * 1024;

/// Start a server that accepts [`OTT`] for each of `emails`.
pub async fn server(emails: &[&str]) -> TestServer {
    let emails: Vec<String> = emails.iter().map(|e| e.to_string()).collect();
    TestServer::start_with(move |c| {
        for e in emails {
            c.hardcoded_ott.insert(e, OTT.into());
        }
    })
    .await
    .expect("start server")
}

/// A signed-up account and the secrets the test needs to check things.
pub struct Account {
    /// Email.
    pub email: String,
    /// Password.
    pub password: String,
    /// The recovery key as 24 words.
    pub recovery_words: String,
    /// The recovery key as hex.
    pub recovery_hex: String,
    /// Master key.
    pub master_key: Vec<u8>,
    /// Token from sign-up (for setting up 2FA in tests).
    pub token: String,
}

/// Sign up exactly as the apps do: email code, key attributes (with a
/// recovery key), SRP setup.
pub async fn signup(base: &str, email: &str, password: &str) -> Account {
    crypto::init();
    let api = Api::new(base);
    api.json_no_content(
        Method::POST,
        "/users/ott",
        &json!({"email": email, "purpose": "signup"}),
    )
    .await
    .expect("ott");
    let v: Value = api
        .json(
            Method::POST,
            "/users/verify-email",
            &json!({"email": email, "ott": OTT}),
        )
        .await
        .expect("verify-email");
    let token = v["token"].as_str().expect("token").to_string();
    let api = api.with_token(token.clone());

    let kek_salt = crypto::random_bytes(16);
    let kek = crypto::derive_argon_key(password.as_bytes(), &kek_salt, OPS_LIMIT, MEM_LIMIT)
        .expect("kek");
    let master_key = crypto::generate_key();
    let (public_key, secret_key) = crypto::box_keypair();
    let recovery_key = crypto::generate_key();
    let (enc_key, key_nonce) = crypto::secretbox_seal(&master_key, &kek).unwrap();
    let (enc_sk, sk_nonce) = crypto::secretbox_seal(&secret_key, &master_key).unwrap();
    let (mk_rk, mk_rk_nonce) = crypto::secretbox_seal(&master_key, &recovery_key).unwrap();
    let (rk_mk, rk_mk_nonce) = crypto::secretbox_seal(&recovery_key, &master_key).unwrap();
    let ka = json!({
        "kekSalt": crypto::b64(&kek_salt),
        "encryptedKey": crypto::b64(&enc_key),
        "keyDecryptionNonce": crypto::b64(&key_nonce),
        "publicKey": crypto::b64(&public_key),
        "encryptedSecretKey": crypto::b64(&enc_sk),
        "secretKeyDecryptionNonce": crypto::b64(&sk_nonce),
        "memLimit": MEM_LIMIT,
        "opsLimit": OPS_LIMIT,
        "masterKeyEncryptedWithRecoveryKey": crypto::b64(&mk_rk),
        "masterKeyDecryptionNonce": crypto::b64(&mk_rk_nonce),
        "recoveryKeyEncryptedWithMasterKey": crypto::b64(&rk_mk),
        "recoveryKeyDecryptionNonce": crypto::b64(&rk_mk_nonce),
    });
    api.json_no_content(
        Method::PUT,
        "/users/attributes",
        &json!({"keyAttributes": ka}),
    )
    .await
    .expect("attributes");

    let login_key = crypto::derive_login_key(&kek).unwrap();
    // A fresh UUID per account (the server keys SRP attributes by it).
    let r = hex::encode(crypto::random_bytes(16));
    let srp_user_id = &format!(
        "{}-{}-4{}-8{}-{}",
        &r[0..8],
        &r[8..12],
        &r[13..16],
        &r[17..20],
        &r[20..32]
    );
    let srp_salt = crypto::random_bytes(16);
    let ver = verifier(srp_user_id.as_bytes(), &srp_salt, &login_key);
    let mut srp = SrpClient::new(srp_user_id.as_bytes(), &srp_salt, &login_key);
    let v: Value = api
        .json(
            Method::POST,
            "/users/srp/setup",
            &json!({
                "srpUserID": srp_user_id,
                "srpSalt": crypto::b64(&srp_salt),
                "srpVerifier": crypto::b64(&ver),
                "srpA": crypto::b64(&srp.public_a()),
            }),
        )
        .await
        .expect("srp setup");
    let m1 = srp
        .compute_m1(&crypto::unb64(v["srpB"].as_str().unwrap()).unwrap())
        .unwrap();
    let v2: Value = api
        .json(
            Method::POST,
            "/users/srp/complete",
            &json!({"setupID": v["setupID"], "srpM1": crypto::b64(&m1)}),
        )
        .await
        .expect("srp complete");
    srp.verify_m2(&crypto::unb64(v2["srpM2"].as_str().unwrap()).unwrap())
        .unwrap();

    Account {
        email: email.into(),
        password: password.into(),
        recovery_words: crypto::recovery_key_to_mnemonic(&recovery_key).unwrap(),
        recovery_hex: hex::encode(&recovery_key),
        master_key,
        token,
    }
}

/// A fresh Core with its own data and cache directories.
pub struct TestCore {
    /// The core.
    pub core: Arc<Core>,
    /// Keeps the directories alive.
    pub dir: tempfile::TempDir,
}

impl TestCore {
    /// A new Core.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::new(
            dir.path().join("data").display().to_string(),
            dir.path().join("cache").display().to_string(),
        )
        .expect("core");
        Self { core, dir }
    }

    /// A Core with `base` saved and active.
    pub async fn connected(base: &str) -> Self {
        let c = Self::new();
        c.core.add_server(base.into()).await.expect("add server");
        c
    }

    /// The data directory.
    pub fn data_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("data")
    }

    /// The cache directory.
    pub fn cache_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("cache")
    }
}

/// A small real JPEG.
pub fn jpeg(w: u32, h: u32, seed: u8) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| {
        image::Rgb([
            (x as u8).wrapping_mul(3).wrapping_add(seed),
            (y as u8).wrapping_mul(5),
            seed,
        ])
    });
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .unwrap();
    out.into_inner()
}

/// Assert `r` is `Err(e)`.
pub fn assert_err<T: std::fmt::Debug>(r: Result<T, CoreError>, e: CoreError) {
    match r {
        Err(got) => assert_eq!(got, e),
        Ok(v) => panic!("expected {e:?}, got Ok({v:?})"),
    }
}

// ---- a proxy with the Photovault-only endpoints ----

/// What the proxy answers on `/photovault/*`.
#[derive(Clone)]
pub struct ProxyConfig {
    /// `/photovault/info` body.
    pub info: Value,
    /// `/photovault/usage` body (`None` = 404).
    pub usage: Option<Value>,
}

#[derive(Clone)]
struct ProxyState {
    upstream: String,
    cfg: Arc<std::sync::Mutex<ProxyConfig>>,
    client: reqwest::Client,
}

/// A listening proxy in front of the real server.
pub struct Proxy {
    /// URLs of its listeners.
    pub urls: Vec<String>,
    cfg: Arc<std::sync::Mutex<ProxyConfig>>,
}

impl Proxy {
    /// Start a proxy for `upstream` with `listeners` listening sockets
    /// (to play a public and a home address of the same server).
    /// `make` builds the config from the listener URLs.
    pub async fn start(
        upstream: &str,
        listeners: usize,
        make: impl FnOnce(&[String]) -> ProxyConfig,
    ) -> Self {
        let mut ls = Vec::new();
        let mut urls = Vec::new();
        for _ in 0..listeners {
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            urls.push(format!("http://{}", l.local_addr().unwrap()));
            ls.push(l);
        }
        let cfg = Arc::new(std::sync::Mutex::new(make(&urls)));
        let client = reqwest::Client::builder()
            .tls_backend_preconfigured(photovault_core::api::tls_config())
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let state = ProxyState {
            upstream: upstream.trim_end_matches('/').into(),
            cfg: cfg.clone(),
            client,
        };
        let app = Router::new()
            .route("/photovault/info", get(info))
            .route("/photovault/usage", get(usage))
            .fallback(forward)
            .with_state(state);
        for l in ls {
            let app = app.clone();
            tokio::spawn(async move {
                let _ = axum::serve(l, app).await;
            });
        }
        Self { urls, cfg }
    }

    /// Change what it answers.
    pub fn set(&self, f: impl FnOnce(&mut ProxyConfig)) {
        f(&mut self.cfg.lock().unwrap());
    }
}

async fn info(State(s): State<ProxyState>) -> Json<Value> {
    Json(s.cfg.lock().unwrap().info.clone())
}

async fn usage(State(s): State<ProxyState>, headers: HeaderMap) -> Response {
    if headers.get("X-Auth-Token").is_none() {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match s.cfg.lock().unwrap().usage.clone() {
        Some(u) => Json(u).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn forward(State(s): State<ProxyState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let path = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let body: Bytes = axum::body::to_bytes(body, usize::MAX).await.unwrap();
    let mut rb = s
        .client
        .request(parts.method.clone(), format!("{}{path}", s.upstream))
        .body(body);
    for (k, v) in parts.headers.iter() {
        if k != "host" && k != "content-length" {
            rb = rb.header(k, v);
        }
    }
    let res = match rb.send().await {
        Ok(r) => r,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let mut out = Response::builder().status(res.status().as_u16());
    for (k, v) in res.headers().iter() {
        if k != "content-length" && k != "transfer-encoding" {
            out = out.header(k, v);
        }
    }
    let bytes = res.bytes().await.unwrap_or_default();
    out.body(Body::from(bytes)).unwrap()
}

/// `/photovault/info` JSON.
pub fn info_json(name: &str, public_url: &str, home_url: Option<&str>) -> Value {
    json!({
        "product": "photovault",
        "name": name,
        "version": "0.1.0",
        "publicUrl": public_url,
        "homeUrl": home_url,
        "signupsAllowed": true,
    })
}
