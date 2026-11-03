//! Shared helpers for the users & accounts integration tests: a tiny HTTP client and an
//! app-side implementation of signup / SRP login using the clients' formulas.
#![allow(dead_code)]

use photovault_server::crypto;
use photovault_server::testing::TestServer;
use photovault_server::users::srp::client as srp;
use serde_json::{Value, json};

/// Raw HTTP/1.1 over TCP (no TLS stack needed).
pub async fn http(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&Value>,
) -> (u16, Value) {
    let (code, bytes) = http_raw(
        method,
        url,
        headers,
        body.map(|b| b.to_string().into_bytes()),
    )
    .await;
    let v = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()))
    };
    (code, v)
}

/// Raw request returning the body bytes.
pub async fn http_raw(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<Vec<u8>>,
) -> (u16, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(url).unwrap();
    let mut s = tokio::net::TcpStream::connect((u.host_str().unwrap(), u.port().unwrap()))
        .await
        .unwrap();
    let pq = match u.query() {
        Some(q) => format!("{}?{}", u.path(), q),
        None => u.path().to_string(),
    };
    let body = body.unwrap_or_default();
    let mut req = format!(
        "{method} {pq} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    s.write_all(&body).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).to_string();
    let code = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let idx = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&out[..idx]).to_lowercase();
    let mut payload = out[idx..].to_vec();
    if head.contains("transfer-encoding: chunked") {
        payload = dechunk(&payload);
    }
    (code, payload)
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(p) = b.windows(2).position(|w| w == b"\r\n") {
        let n = usize::from_str_radix(std::str::from_utf8(&b[..p]).unwrap().trim(), 16).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&b[p + 2..p + 2 + n]);
        b = &b[p + 2 + n + 2..];
    }
    out
}

/// A client bound to one server, optionally authenticated.
#[derive(Clone)]
pub struct Client {
    pub base: String,
    pub token: Option<String>,
    pub package: Option<String>,
}

impl Client {
    pub fn new(srv: &TestServer) -> Self {
        Client {
            base: srv.url.clone(),
            token: None,
            package: None,
        }
    }
    pub fn with_token(&self, t: &str) -> Self {
        let mut c = self.clone();
        c.token = Some(t.to_string());
        c
    }
    pub async fn req(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut h: Vec<(&str, &str)> =
            vec![("User-Agent", "Mozilla/5.0 (X11; Linux x86_64) Chrome/120")];
        if let Some(t) = &self.token {
            h.push(("X-Auth-Token", t));
        }
        if let Some(p) = &self.package {
            h.push(("X-Client-Package", p));
        }
        http(method, &format!("{}{}", self.base, path), &h, body.as_ref()).await
    }
    pub async fn get(&self, path: &str) -> (u16, Value) {
        self.req("GET", path, None).await
    }
    pub async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.req("POST", path, Some(body)).await
    }
    pub async fn put(&self, path: &str, body: Value) -> (u16, Value) {
        self.req("PUT", path, Some(body)).await
    }
    pub async fn delete(&self, path: &str, body: Option<Value>) -> (u16, Value) {
        self.req("DELETE", path, body).await
    }
}

/// Route server logs to the test output.
pub fn init_logs() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_env_filter("warn")
        .try_init();
}

/// What the app keeps after signup.
pub struct Account {
    pub email: String,
    pub id: i64,
    pub public_key: Vec<u8>,
    pub secret_key: Vec<u8>,
    pub srp_user_id: String,
    pub srp_salt: Vec<u8>,
    pub login_key: Vec<u8>,
    pub key_attributes: Value,
    pub token: String,
}

pub const OTT: &str = "123456";

/// Start a server where `emails` get the hard-coded OTT.
pub async fn server(emails: &[&str]) -> TestServer {
    init_logs();
    let emails: Vec<String> = emails.iter().map(|e| e.to_string()).collect();
    TestServer::start_with(move |c| {
        for e in emails {
            c.hardcoded_ott.insert(e, OTT.to_string());
        }
    })
    .await
    .unwrap()
}

fn rb64(n: usize) -> String {
    crypto::b64(&crypto::random_bytes(n))
}

/// Key attributes as the apps generate them (random ciphertexts of the right sizes).
pub fn key_attributes(public_key: &[u8]) -> Value {
    json!({
        "kekSalt": rb64(16),
        "encryptedKey": rb64(48),
        "keyDecryptionNonce": rb64(24),
        "publicKey": crypto::b64(public_key),
        "encryptedSecretKey": rb64(48),
        "secretKeyDecryptionNonce": rb64(24),
        "memLimit": 268435456,
        "opsLimit": 16,
        "masterKeyEncryptedWithRecoveryKey": rb64(48),
        "masterKeyDecryptionNonce": rb64(24),
        "recoveryKeyEncryptedWithMasterKey": rb64(48),
        "recoveryKeyDecryptionNonce": rb64(24),
    })
}

/// Open an `encryptedToken` like the apps: unseal, then URL-safe base64 the bytes.
pub fn open_token(acc_pk: &[u8], acc_sk: &[u8], encrypted: &str) -> String {
    let raw = crypto::box_seal_open(&crypto::unb64(encrypted).unwrap(), acc_pk, acc_sk).unwrap();
    crypto::b64url(&raw)
}

/// Full signup: OTT → verify-email → key attributes → SRP setup/complete.
pub async fn signup(c: &Client, email: &str) -> Account {
    let (s, _) = c
        .post("/users/ott", json!({"email": email, "purpose": "signup"}))
        .await;
    assert_eq!(s, 200);
    let (s, v) = c
        .post("/users/verify-email", json!({"email": email, "ott": OTT}))
        .await;
    assert_eq!(s, 200, "{v}");
    let token = v["token"]
        .as_str()
        .expect("plain token for new user")
        .to_string();
    assert!(v.get("keyAttributes").is_none());
    let id = v["id"].as_i64().unwrap();
    let authed = c.with_token(&token);
    let (pk, sk) = crypto::box_keypair();
    let ka = key_attributes(&pk);
    let (s, v) = authed
        .put("/users/attributes", json!({"keyAttributes": ka}))
        .await;
    assert_eq!(s, 200, "{v}");

    let srp_user_id = uuid::Uuid::new_v4().to_string();
    let salt = crypto::random_bytes(16);
    let login_key = crypto::random_bytes(16);
    setup_srp(&authed, &srp_user_id, &salt, &login_key).await;
    Account {
        email: email.into(),
        id,
        public_key: pk,
        secret_key: sk,
        srp_user_id,
        srp_salt: salt,
        login_key,
        key_attributes: ka,
        token,
    }
}

/// `/users/srp/setup` + `/users/srp/complete` as the apps do it.
pub async fn setup_srp(c: &Client, srp_user_id: &str, salt: &[u8], login_key: &[u8]) {
    let verifier = srp::verifier(srp_user_id.as_bytes(), login_key, salt);
    let a = crypto::random_bytes(32);
    let (s, v) = c
        .post(
            "/users/srp/setup",
            json!({
                "srpUserID": srp_user_id,
                "srpSalt": crypto::b64(salt),
                "srpVerifier": crypto::b64(&verifier),
                "srpA": crypto::b64(&srp::public_a(&a)),
            }),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let b = crypto::unb64(v["srpB"].as_str().unwrap()).unwrap();
    assert_eq!(b.len(), 512);
    let (m1, m2) = srp::proofs(srp_user_id.as_bytes(), login_key, salt, &a, &b).unwrap();
    let (s, v2) = c
        .post(
            "/users/srp/complete",
            json!({"setupID": v["setupID"], "srpM1": crypto::b64(&m1)}),
        )
        .await;
    assert_eq!(s, 200, "{v2}");
    assert_eq!(v2["srpM2"].as_str().unwrap(), crypto::b64(&m2));
}

/// SRP login (attributes → create-session → verify-session). Returns the response JSON.
pub async fn srp_login(c: &Client, acc: &Account) -> (u16, Value) {
    srp_login_with(c, &acc.email, &acc.login_key).await
}

/// SRP login with an explicit password-derived key.
pub async fn srp_login_with(c: &Client, email: &str, login_key: &[u8]) -> (u16, Value) {
    let (s, attrs) = c.get(&format!("/users/srp/attributes?email={email}")).await;
    assert_eq!(s, 200, "{attrs}");
    let attrs = &attrs["attributes"];
    let srp_user_id = attrs["srpUserID"].as_str().unwrap().to_string();
    let salt = crypto::unb64(attrs["srpSalt"].as_str().unwrap()).unwrap();
    let a = crypto::random_bytes(32);
    let (s, sess) = c
        .post(
            "/users/srp/create-session",
            json!({"srpUserID": srp_user_id, "srpA": crypto::b64(&srp::public_a(&a))}),
        )
        .await;
    if s != 200 {
        return (s, sess);
    }
    let b = crypto::unb64(sess["srpB"].as_str().unwrap()).unwrap();
    let (m1, m2) = srp::proofs(srp_user_id.as_bytes(), login_key, &salt, &a, &b).unwrap();
    let (s, v) = c
        .post(
            "/users/srp/verify-session",
            json!({"sessionID": sess["sessionID"], "srpUserID": srp_user_id, "srpM1": crypto::b64(&m1)}),
        )
        .await;
    if s == 200 {
        assert_eq!(v["srpM2"].as_str().unwrap(), crypto::b64(&m2));
    }
    (s, v)
}
