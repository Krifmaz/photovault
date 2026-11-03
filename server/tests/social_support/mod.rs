//! Test-only fixtures for the social tests: the foreign library tables (owned by the
//! files/collections workstream, created here with `IF NOT EXISTS` in museum's shape),
//! users/tokens, a tiny HTTP client, and a stand-in `/public-collection` middleware.
#![allow(dead_code)]

use axum::Router;
use photovault_server::social::public::{PublicCollectionCtx, public_router};
use photovault_server::state::AppState;
use photovault_server::testing::TestServer;
use serde_json::Value;

/// museum's collections / collection_shares / collection_files, translated to BIGINT/TEXT/BOOLEAN.
pub async fn create_library_tables(st: &AppState) {
    for sql in [
        "CREATE TABLE IF NOT EXISTS collections (
            collection_id BIGINT PRIMARY KEY,
            owner_id BIGINT NOT NULL,
            encrypted_key TEXT NOT NULL DEFAULT '',
            key_decryption_nonce TEXT NOT NULL DEFAULT '',
            name TEXT,
            encrypted_name TEXT,
            name_decryption_nonce TEXT,
            type TEXT NOT NULL DEFAULT 'album',
            attributes TEXT NOT NULL DEFAULT '{}',
            updation_time BIGINT NOT NULL DEFAULT 0,
            is_deleted BOOLEAN DEFAULT FALSE,
            magic_metadata TEXT,
            pub_magic_metadata TEXT,
            app TEXT DEFAULT 'photos')",
        "CREATE TABLE IF NOT EXISTS collection_shares (
            collection_id BIGINT NOT NULL,
            from_user_id BIGINT NOT NULL,
            to_user_id BIGINT NOT NULL,
            encrypted_key TEXT NOT NULL DEFAULT '',
            updation_time BIGINT NOT NULL DEFAULT 0,
            is_deleted BOOLEAN DEFAULT FALSE,
            role_type TEXT DEFAULT 'VIEWER',
            shared_at BIGINT,
            UNIQUE (collection_id, from_user_id, to_user_id))",
        "CREATE TABLE IF NOT EXISTS collection_files (
            file_id BIGINT NOT NULL,
            collection_id BIGINT NOT NULL,
            encrypted_key TEXT NOT NULL DEFAULT '',
            key_decryption_nonce TEXT NOT NULL DEFAULT '',
            is_deleted BOOLEAN DEFAULT FALSE,
            updation_time BIGINT NOT NULL DEFAULT 0,
            c_owner_id BIGINT,
            f_owner_id BIGINT,
            created_at BIGINT,
            action_user BIGINT,
            action TEXT,
            UNIQUE (collection_id, file_id))",
    ] {
        sqlx::query(sql).execute(&st.db.pool).await.unwrap();
    }
}

/// Start a server with the library tables in place.
pub async fn start() -> TestServer {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("warn")
        .with_test_writer()
        .try_init();
    let srv = TestServer::start().await.unwrap();
    create_library_tables(&srv.state).await;
    srv
}

/// Insert a user with an encrypted email and a Photos token; returns (user_id, token).
pub async fn user(st: &AppState, email: &str) -> (i64, String) {
    let (ct, nonce) =
        photovault_server::crypto::secretbox(email.as_bytes(), &st.keys.encryption).unwrap();
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO users (encrypted_email, email_decryption_nonce, email_hash, creation_time, storage_quota) \
         VALUES ($1, $2, $3, 0, 0) RETURNING user_id",
    )
    .bind(photovault_server::crypto::b64(&ct))
    .bind(photovault_server::crypto::b64(&nonce))
    .bind(email)
    .fetch_one(&st.db.pool)
    .await
    .unwrap();
    let token = format!(
        "tok-{}",
        hex::encode(photovault_server::crypto::random_bytes(8))
    );
    sqlx::query(
        "INSERT INTO tokens (token_hash, user_id, app, creation_time, last_used_at) VALUES ($1, $2, 'photos', 0, 0)",
    )
    .bind(photovault_server::auth::hash_token(&token))
    .bind(id)
    .execute(&st.db.pool)
    .await
    .unwrap();
    (id, token)
}

/// Insert a collection.
pub async fn collection(st: &AppState, id: i64, owner: i64, app: &str) {
    sqlx::query("INSERT INTO collections (collection_id, owner_id, app) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(owner)
        .bind(app)
        .execute(&st.db.pool)
        .await
        .unwrap();
}

/// Share a collection.
pub async fn share(st: &AppState, cid: i64, from: i64, to: i64, role: &str) {
    sqlx::query(
        "INSERT INTO collection_shares (collection_id, from_user_id, to_user_id, role_type) VALUES ($1, $2, $3, $4)",
    )
    .bind(cid)
    .bind(from)
    .bind(to)
    .bind(role)
    .execute(&st.db.pool)
    .await
    .unwrap();
}

/// Put a file in a collection.
pub async fn add_file(st: &AppState, cid: i64, fid: i64, owner: i64) {
    sqlx::query(
        "INSERT INTO collection_files (file_id, collection_id, f_owner_id, c_owner_id) VALUES ($1, $2, $3, $3)",
    )
    .bind(fid)
    .bind(cid)
    .bind(owner)
    .execute(&st.db.pool)
    .await
    .unwrap();
}

/// Minimal HTTP/1.1 client over TCP. Returns (status, JSON body or Null).
pub async fn http(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(url).unwrap();
    let mut s = tokio::net::TcpStream::connect((u.host_str().unwrap(), u.port().unwrap()))
        .await
        .unwrap();
    let pq = match u.query() {
        Some(q) => format!("{}?{}", u.path(), q),
        None => u.path().to_string(),
    };
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!(
        "{method} {pq} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    s.write_all(body.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).to_string();
    let code = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let idx = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let v = serde_json::from_slice(&out[idx..]).unwrap_or(Value::Null);
    (code, v)
}

/// Serve `public_router()` under `/public-collection` behind a stand-in middleware that
/// reads `X-Test-Collection: <collection_id>:<owner_id>:<enable 0|1>`.
pub async fn start_public(st: &AppState) -> String {
    async fn mw(
        mut req: axum::extract::Request,
        next: axum::middleware::Next,
    ) -> axum::response::Response {
        let ctx = req
            .headers()
            .get("X-Test-Collection")
            .and_then(|v| v.to_str().ok())
            .map(|v| {
                v.split(':')
                    .filter_map(|x| x.parse().ok())
                    .collect::<Vec<i64>>()
            });
        if let Some(p) = ctx.filter(|p| p.len() == 3) {
            req.extensions_mut().insert(PublicCollectionCtx {
                id: 1,
                collection_id: p[0],
                owner_id: p[1],
                enable_comment: p[2] == 1,
                ip: String::new(),
                user_agent: String::new(),
            });
        }
        next.run(req).await
    }
    let app: Router = Router::new()
        .nest(
            "/public-collection",
            public_router().route_layer(axum::middleware::from_fn(mw)),
        )
        .with_state(st.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    url
}

/// A base64 string of `n` random bytes (a stand-in ciphertext).
pub fn cipher(n: usize) -> String {
    photovault_server::crypto::b64(&photovault_server::crypto::random_bytes(n))
}

/// A 156-char reaction cipher (116 bytes).
pub fn reaction_cipher() -> String {
    cipher(116)
}
