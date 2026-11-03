//! Shared test helpers for the public-link and cast tests: rows inserted directly in
//! the real library schema (migration `0200_library`), and a tiny HTTP client.

#![allow(dead_code)]

use photovault_server::db::now_micros;
use photovault_server::testing::TestServer;
use sqlx::AnyPool;
use std::collections::HashMap;

/// A started server (real migrations, fresh database).
pub async fn server() -> TestServer {
    TestServer::start().await.expect("server starts")
}

/// Insert a user and a session token; returns (user_id, token).
pub async fn user(db: &AnyPool, quota: i64) -> (i64, String) {
    let tag = hex::encode(photovault_server::crypto::random_bytes(8));
    let (uid,): (i64,) = sqlx::query_as(
        "INSERT INTO users (encrypted_email, email_decryption_nonce, email_hash, creation_time, storage_quota) \
         VALUES ($1, $2, $3, $4, $5) RETURNING user_id",
    )
    .bind("e")
    .bind("n")
    .bind(&tag)
    .bind(now_micros())
    .bind(quota)
    .fetch_one(db)
    .await
    .expect("insert user");
    let token = format!("tok-{tag}");
    sqlx::query(
        "INSERT INTO tokens (token_hash, user_id, app, creation_time, last_used_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(photovault_server::auth::hash_token(&token))
    .bind(uid)
    .bind("photos")
    .bind(now_micros())
    .bind(now_micros())
    .execute(db)
    .await
    .expect("insert token");
    (uid, token)
}

/// Insert a collection; returns its id.
pub async fn collection(db: &AnyPool, owner: i64) -> i64 {
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO collections (owner_id, encrypted_key, key_decryption_nonce, encrypted_name, name_decryption_nonce, \
         type, attributes, updation_time, magic_metadata, pub_magic_metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING collection_id",
    )
    .bind(owner)
    .bind("ck")
    .bind("cn")
    .bind("encname")
    .bind("namenonce")
    .bind("album")
    .bind(r#"{"version":0}"#)
    .bind(1000i64)
    .bind(r#"{"version":1,"count":1,"data":"private","header":"h"}"#)
    .bind(r#"{"version":2,"count":1,"data":"pub","header":"ph"}"#)
    .fetch_one(db)
    .await
    .expect("insert collection");
    id
}

/// Insert a file with both objects (also written to the store) in `collection_id`.
pub async fn file(srv: &TestServer, owner: i64, collection_id: i64, version: i64) -> i64 {
    let db = &srv.state.db.pool;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO files (owner_id, file_decryption_header, thumbnail_decryption_header, metadata_decryption_header, \
         encrypted_metadata, magic_metadata, pub_magic_metadata, info, updation_time) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) RETURNING file_id",
    )
    .bind(owner)
    .bind("fh")
    .bind("th")
    .bind("mh")
    .bind("meta")
    .bind(r#"{"version":1,"count":1,"data":"secret","header":"x"}"#)
    .bind(r#"{"version":1,"count":1,"data":"pubdata","header":"y"}"#)
    .bind(r#"{"fileSize":5,"thumbSize":3}"#)
    .bind(version)
    .fetch_one(db)
    .await
    .expect("insert file");
    for (t, data) in [("file", &b"hello"[..]), ("thumbnail", &b"thm"[..])] {
        let key = format!("{owner}/{t}-{id}");
        sqlx::query(
            "INSERT INTO object_keys (file_id, o_type, object_key, size) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(t)
        .bind(&key)
        .bind(data.len() as i64)
        .execute(db)
        .await
        .expect("insert object");
        srv.state
            .store
            .put_bytes(&key, bytes::Bytes::copy_from_slice(data))
            .await
            .expect("store object");
    }
    add_to_collection(db, id, owner, collection_id, version).await;
    id
}

/// Add an existing file to a collection.
pub async fn add_to_collection(
    db: &AnyPool,
    file_id: i64,
    owner: i64,
    collection_id: i64,
    version: i64,
) {
    sqlx::query(
        "INSERT INTO collection_files (file_id, collection_id, encrypted_key, key_decryption_nonce, updation_time, \
         c_owner_id, f_owner_id, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(file_id)
    .bind(collection_id)
    .bind(format!("ek{file_id}"))
    .bind("kn")
    .bind(version)
    .bind(owner)
    .bind(owner)
    .bind(version)
    .execute(db)
    .await
    .expect("insert collection file");
}

/// Insert an album link; returns (link id, access token).
pub async fn album_link(db: &AnyPool, collection_id: i64, extra: &[(&str, &str)]) -> (i64, String) {
    let token = format!(
        "A{}",
        hex::encode(photovault_server::crypto::random_bytes(5)).to_uppercase()
    );
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO public_collection_tokens (collection_id, access_token, created_at, updated_at) \
         VALUES ($1, $2, $3, $3) RETURNING id",
    )
    .bind(collection_id)
    .bind(&token)
    .bind(now_micros())
    .fetch_one(db)
    .await
    .expect("insert link");
    for (col, val) in extra {
        // Test-only: column names are literals from the test code, values are SQL literals.
        sqlx::query(&format!(
            "UPDATE public_collection_tokens SET {col} = {val} WHERE id = $1"
        ))
        .bind(id)
        .execute(db)
        .await
        .expect("update link");
    }
    (id, token)
}

/// Response from [`http`].
#[derive(Debug)]
pub struct Res {
    /// Status code.
    pub status: u16,
    /// Lower-cased header names.
    pub headers: HashMap<String, String>,
    /// Body.
    pub body: Vec<u8>,
}

impl Res {
    /// Body as JSON.
    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| {
            panic!(
                "not JSON (status {}): {}",
                self.status,
                String::from_utf8_lossy(&self.body)
            )
        })
    }
}

/// Minimal HTTP/1.1 client over TCP (no TLS needed in tests).
pub async fn http(method: &str, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Res {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(url).expect("url");
    let mut s =
        tokio::net::TcpStream::connect((u.host_str().expect("host"), u.port().expect("port")))
            .await
            .expect("connect");
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
    s.write_all(req.as_bytes()).await.expect("write");
    s.write_all(body).await.expect("write body");
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.expect("read");
    let idx = out
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("header end")
        + 4;
    let head = String::from_utf8_lossy(&out[..idx]).to_string();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .expect("status");
    let headers: HashMap<String, String> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let mut body = out[idx..].to_vec();
    if headers.get("transfer-encoding").map(String::as_str) == Some("chunked") {
        let mut decoded = Vec::new();
        let mut rest = &body[..];
        while let Some(p) = rest.windows(2).position(|w| w == b"\r\n") {
            let size =
                usize::from_str_radix(String::from_utf8_lossy(&rest[..p]).trim(), 16).unwrap_or(0);
            if size == 0 {
                break;
            }
            decoded.extend_from_slice(&rest[p + 2..p + 2 + size]);
            rest = &rest[p + 2 + size + 2..];
        }
        body = decoded;
    }
    Res {
        status,
        headers,
        body,
    }
}

/// GET with headers.
pub async fn get(url: &str, headers: &[(&str, &str)]) -> Res {
    http("GET", url, headers, b"").await
}

/// POST a JSON body with headers.
pub async fn post_json(url: &str, headers: &[(&str, &str)], body: serde_json::Value) -> Res {
    let mut h = headers.to_vec();
    h.push(("Content-Type", "application/json"));
    http("POST", url, &h, body.to_string().as_bytes()).await
}
