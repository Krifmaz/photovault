//! Shared helpers for the legacy / entity / contacts integration tests:
//! a tiny HTTP client, users with tokens, and test-only stand-ins for the
//! users workstream's `key_attributes` / SRP tables (museum column names).

#![allow(dead_code)]

use photovault_server::db::now_micros;
use photovault_server::testing::TestServer;
use serde_json::Value;

/// A response.
pub struct Resp {
    pub status: u16,
    pub headers: String,
    pub body: Vec<u8>,
}

impl Resp {
    pub fn json(&self) -> Value {
        if self.body.is_empty() {
            return Value::Null;
        }
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| panic!("not json: {}", String::from_utf8_lossy(&self.body)))
    }
}

/// Raw HTTP/1.1 request over TCP.
pub async fn raw(method: &str, url: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Resp {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(url).unwrap();
    let mut s = tokio::net::TcpStream::connect((u.host_str().unwrap(), u.port().unwrap()))
        .await
        .unwrap();
    let pq = match u.query() {
        Some(q) => format!("{}?{}", u.path(), q),
        None => u.path().to_string(),
    };
    let mut req = format!(
        "{method} {pq} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n",
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
    let idx = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&out[..idx]).to_string();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let mut body = out[idx..].to_vec();
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        body = dechunk(&body);
    }
    Resp {
        status,
        headers: head,
        body,
    }
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(pos) = b.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let n = usize::from_str_radix(String::from_utf8_lossy(&b[..pos]).trim(), 16).unwrap_or(0);
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&b[pos + 2..pos + 2 + n]);
        b = &b[pos + 4 + n..];
    }
}

/// A test user with a session token.
#[derive(Clone)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub token: String,
    /// `X-Client-Package` sent with requests.
    pub package: &'static str,
}

/// Client bound to a server.
pub struct Api<'a> {
    pub srv: &'a TestServer,
}

impl<'a> Api<'a> {
    pub fn new(srv: &'a TestServer) -> Self {
        Self { srv }
    }

    pub async fn call(
        &self,
        user: Option<&User>,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Resp {
        let mut headers: Vec<(&str, &str)> = vec![("Content-Type", "application/json")];
        if let Some(u) = user {
            headers.push(("X-Auth-Token", &u.token));
            headers.push(("X-Client-Package", u.package));
        }
        let bytes = body
            .map(|b| serde_json::to_vec(&b).unwrap())
            .unwrap_or_default();
        raw(
            method,
            &format!("{}{}", self.srv.url, path),
            &headers,
            bytes,
        )
        .await
    }

    pub async fn get(&self, u: &User, path: &str) -> Resp {
        self.call(Some(u), "GET", path, None).await
    }
    pub async fn post(&self, u: &User, path: &str, body: Value) -> Resp {
        self.call(Some(u), "POST", path, Some(body)).await
    }
    pub async fn put(&self, u: &User, path: &str, body: Value) -> Resp {
        self.call(Some(u), "PUT", path, Some(body)).await
    }
    pub async fn delete(&self, u: &User, path: &str) -> Resp {
        self.call(Some(u), "DELETE", path, None).await
    }
    pub async fn public_post(&self, path: &str, body: Value) -> Resp {
        self.call(None, "POST", path, Some(body)).await
    }
}

/// Insert a user the way the users workstream stores them (encrypted email,
/// hex generichash lookup hash) plus a session token for `package`.
pub async fn create_user(srv: &TestServer, email: &str, package: &'static str) -> User {
    let st = &srv.state;
    let (ct, nonce) =
        photovault_server::crypto::secretbox(email.as_bytes(), &st.keys.encryption).unwrap();
    let hash = photovault_server::legacy::users::email_hash(st, email);
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO users(encrypted_email, email_decryption_nonce, email_hash, creation_time, storage_quota) \
         VALUES ($1, $2, $3, $4, $5) RETURNING user_id",
    )
    .bind(photovault_server::crypto::b64(&ct))
    .bind(photovault_server::crypto::b64(&nonce))
    .bind(hash)
    .bind(now_micros())
    .bind(1_i64 << 40)
    .fetch_one(&st.db.pool)
    .await
    .unwrap();
    let token = add_token(srv, id, package).await;
    User {
        id,
        email: email.to_string(),
        token,
        package,
    }
}

/// Another session for a user.
pub async fn add_token(srv: &TestServer, user_id: i64, package: &str) -> String {
    let token = photovault_server::crypto::b64url(&photovault_server::crypto::random_bytes(32));
    let app = if package.starts_with("io.ente.auth") {
        "auth"
    } else {
        "photos"
    };
    sqlx::query(
        "INSERT INTO tokens(token_hash, user_id, app, creation_time, last_used_at) VALUES ($1, $2, $3, $4, $4)",
    )
    .bind(photovault_server::auth::hash_token(&token))
    .bind(user_id)
    .bind(app)
    .bind(now_micros())
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    token
}

/// Test-only stand-ins for tables the users workstream owns, with museum's
/// column names. `IF NOT EXISTS`, so real migrations take precedence.
pub async fn ensure_user_auth_tables(srv: &TestServer) {
    let stmts = [
        "CREATE TABLE IF NOT EXISTS key_attributes (
            user_id BIGINT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
            kek_salt TEXT NOT NULL,
            encrypted_key TEXT NOT NULL,
            key_decryption_nonce TEXT NOT NULL,
            public_key TEXT NOT NULL,
            encrypted_secret_key TEXT NOT NULL,
            secret_key_decryption_nonce TEXT NOT NULL,
            mem_limit BIGINT NOT NULL,
            ops_limit BIGINT NOT NULL,
            master_key_encrypted_with_recovery_key TEXT,
            master_key_decryption_nonce TEXT,
            recovery_key_encrypted_with_master_key TEXT,
            recovery_key_decryption_nonce TEXT,
            created_at BIGINT NOT NULL DEFAULT 0
        )",
        "CREATE TABLE IF NOT EXISTS srp_auth (
            user_id BIGINT PRIMARY KEY REFERENCES users(user_id) ON DELETE CASCADE,
            srp_user_id TEXT NOT NULL UNIQUE,
            salt TEXT NOT NULL,
            verifier TEXT NOT NULL,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS temp_srp_setup (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            srp_user_id TEXT NOT NULL,
            user_id BIGINT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE,
            salt TEXT NOT NULL,
            verifier TEXT NOT NULL,
            created_at BIGINT NOT NULL
        )",
        "CREATE TABLE IF NOT EXISTS srp_sessions (
            id TEXT PRIMARY KEY,
            srp_user_id TEXT NOT NULL,
            server_key TEXT NOT NULL,
            srp_a TEXT NOT NULL,
            has_verified BOOLEAN NOT NULL DEFAULT FALSE,
            attempt_count BIGINT NOT NULL DEFAULT 0,
            is_fake BOOLEAN NOT NULL DEFAULT FALSE,
            created_at BIGINT NOT NULL,
            updated_at BIGINT NOT NULL
        )",
    ];
    for s in stmts {
        sqlx::query(s).execute(&srv.state.db.pool).await.unwrap();
    }
}

/// Insert key attributes for a user (museum columns); returns the `kek_salt` used.
pub async fn set_key_attributes(srv: &TestServer, user_id: i64) {
    let b = |n: usize| photovault_server::crypto::b64(&photovault_server::crypto::random_bytes(n));
    sqlx::query(
        "INSERT INTO key_attributes(user_id, kek_salt, encrypted_key, key_decryption_nonce, public_key, encrypted_secret_key, \
         secret_key_decryption_nonce, mem_limit, ops_limit, master_key_encrypted_with_recovery_key, master_key_decryption_nonce, \
         recovery_key_encrypted_with_master_key, recovery_key_decryption_nonce, creation_time) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 1)",
    )
    .bind(user_id)
    .bind(b(16))
    .bind(b(48))
    .bind(b(24))
    .bind(b(32))
    .bind(b(48))
    .bind(b(24))
    .bind(268_435_456_i64)
    .bind(16_i64)
    .bind(b(48))
    .bind(b(24))
    .bind(b(48))
    .bind(b(24))
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
}

/// Plays the app during a recovery password reset: returns the two request
/// bodies' inner objects and the expected M2.
pub struct SrpClient {
    pub srp_user_id: String,
    pub salt: Vec<u8>,
    pub verifier: Vec<u8>,
    client: photovault_server::legacy::srp::client::Client,
}

impl SrpClient {
    pub fn new(password: &[u8]) -> Self {
        let srp_user_id = photovault_server::entities::ids::new_uuid();
        let salt = photovault_server::crypto::random_bytes(16);
        let verifier = photovault_server::legacy::srp::client::verifier(
            &salt,
            srp_user_id.as_bytes(),
            password,
        );
        let client = photovault_server::legacy::srp::client::Client::new(
            &salt,
            srp_user_id.as_bytes(),
            password,
            &photovault_server::crypto::random_bytes(32),
        );
        Self {
            srp_user_id,
            salt,
            verifier,
            client,
        }
    }

    /// `setupSRPRequest` body.
    pub fn setup_request(&self) -> Value {
        serde_json::json!({
            "srpUserID": self.srp_user_id,
            "srpSalt": photovault_server::crypto::b64(&self.salt),
            "srpVerifier": photovault_server::crypto::b64(&self.verifier),
            "srpA": photovault_server::crypto::b64(&self.client.a()),
        })
    }

    /// `updateSrpAndKeysRequest` body for a setup response; returns (body, expected M2 b64).
    pub fn update_request(&self, setup_resp: &Value) -> (Value, String) {
        let b = photovault_server::crypto::unb64(setup_resp["srpB"].as_str().unwrap()).unwrap();
        let (m1, m2) = self.client.proofs(&b);
        let r =
            |n: usize| photovault_server::crypto::b64(&photovault_server::crypto::random_bytes(n));
        (
            serde_json::json!({
                "setupID": setup_resp["setupID"],
                "srpM1": photovault_server::crypto::b64(&m1),
                "updatedKeyAttr": {
                    "kekSalt": r(16),
                    "encryptedKey": r(48),
                    "keyDecryptionNonce": r(24),
                    "memLimit": 1_073_741_824_i64,
                    "opsLimit": 4,
                },
            }),
            photovault_server::crypto::b64(&m2),
        )
    }
}

/// Count of live tokens of a user.
pub async fn live_tokens(srv: &TestServer, user_id: i64) -> i64 {
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM tokens WHERE user_id = $1 AND is_deleted = FALSE")
            .bind(user_id)
            .fetch_one(&srv.state.db.pool)
            .await
            .unwrap();
    n
}

pub const PHOTOS: &str = "io.ente.photos";
pub const AUTH: &str = "io.ente.auth";
