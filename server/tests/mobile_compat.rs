//! Compatibility checks for the Photos mobile app (Flutter + its Rust core), derived
//! from a static audit of ente-io/ente at b2ed3dc (see `docs/src/mobile-api-audit.md`).
//!
//! Each test sends requests shaped exactly like the app's (Dio `enteDio` headers,
//! body field names, query parameters, body-less POSTs) and asserts only what the app
//! reads back (field names, JSON types, null vs missing, status codes). Tests that
//! currently fail would be `#[ignore = "mobile-compat: …"]` so the suite stays green (none are now); run
//! them with `cargo test --test mobile_compat -- --ignored`.

use photovault_server::crypto::{b64, random_bytes, secretbox};
use photovault_server::db::now_micros;
use photovault_server::testing::TestServer;
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------
// Tiny HTTP client (no TLS) and an app-like API client.
// ---------------------------------------------------------------------------------

struct Resp {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    fn header(&self, name: &str) -> Option<String> {
        let name = name.to_ascii_lowercase();
        self.head.split("\r\n").find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim().to_ascii_lowercase() == name).then(|| v.trim().to_string())
        })
    }
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).to_string()
    }
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(i) = b.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let n = usize::from_str_radix(String::from_utf8_lossy(&b[..i]).trim(), 16).unwrap_or(0);
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&b[i + 2..i + 2 + n]);
        b = &b[i + 2 + n + 2..];
    }
}

/// One HTTP/1.1 request. `body: None` sends no body and no Content-Length (like Dio
/// for a `post(path)` without `data`).
async fn http(method: &str, url: &str, headers: &[(&str, &str)], body: Option<&[u8]>) -> Resp {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(url).unwrap();
    let mut s = tokio::net::TcpStream::connect((u.host_str().unwrap(), u.port().unwrap()))
        .await
        .unwrap();
    let pq = match u.query() {
        Some(q) => format!("{}?{}", u.path(), q),
        None => u.path().to_string(),
    };
    let mut req = format!("{method} {pq} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n");
    if let Some(b) = body {
        req.push_str(&format!("Content-Length: {}\r\n", b.len()));
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    s.write_all(req.as_bytes()).await.unwrap();
    if let Some(b) = body {
        s.write_all(b).await.unwrap();
    }
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let idx = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    let head = String::from_utf8_lossy(&out[..idx]).to_string();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let raw = out[idx..].to_vec();
    let body = if head.to_lowercase().contains("transfer-encoding: chunked") {
        dechunk(&raw)
    } else {
        raw
    };
    Resp { status, head, body }
}

/// A signed-in Photos app: the headers `NetworkClient` + `EnteRequestInterceptor` add.
struct App {
    base: String,
    token: Option<String>,
    id: i64,
    email: String,
}

impl App {
    fn anon(srv: &TestServer) -> Self {
        App {
            base: srv.url.clone(),
            token: None,
            id: 0,
            email: String::new(),
        }
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
        extra: &[(&str, &str)],
    ) -> Resp {
        let rid = format!("droid-{}", hex::encode(random_bytes(6)));
        let mut h: Vec<(&str, &str)> = vec![
            ("User-Agent", "Dart/3.10 (dart:io)"),
            ("X-Client-Version", "1.2.3"),
            ("X-Client-Package", "io.ente.photos"),
            ("x-request-id", &rid),
        ];
        if let Some(t) = &self.token {
            h.push(("X-Auth-Token", t));
        }
        let bytes = body.map(|b| b.to_string().into_bytes());
        if bytes.is_some() {
            h.push(("Content-Type", "application/json"));
        }
        h.extend_from_slice(extra);
        http(
            method,
            &format!("{}{}", self.base, path),
            &h,
            bytes.as_deref(),
        )
        .await
    }
    async fn get(&self, path: &str) -> Resp {
        self.send("GET", path, None, &[]).await
    }
    async fn post(&self, path: &str, body: Value) -> Resp {
        self.send("POST", path, Some(body), &[]).await
    }
    async fn put(&self, path: &str, body: Value) -> Resp {
        self.send("PUT", path, Some(body), &[]).await
    }
    async fn delete(&self, path: &str) -> Resp {
        self.send("DELETE", path, None, &[]).await
    }
    /// `_enteDio.post(path)` with no `data`.
    async fn post_empty(&self, path: &str) -> Resp {
        self.send("POST", path, None, &[]).await
    }
    async fn ok(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let r = self.send(method, path, body, &[]).await;
        assert!(
            (200..300).contains(&r.status),
            "{method} {path}: {} {}",
            r.status,
            r.text()
        );
        r.json()
    }
}

fn rb64(n: usize) -> String {
    b64(&random_bytes(n))
}

fn md5_b64(data: &[u8]) -> String {
    use base64::Engine;
    use md5::Digest;
    base64::engine::general_purpose::STANDARD.encode(md5::Md5::digest(data))
}

/// A user with key attributes and a `photos` session token.
async fn user(srv: &TestServer, email: &str) -> App {
    let st = &srv.state;
    let (ct, nonce) = secretbox(email.as_bytes(), &st.keys.encryption).unwrap();
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO users (encrypted_email, email_decryption_nonce, email_hash, creation_time, storage_quota) \
         VALUES ($1, $2, $3, $4, $5) RETURNING user_id",
    )
    .bind(b64(&ct))
    .bind(b64(&nonce))
    .bind(photovault_server::library::users::email_hash(st, email))
    .bind(now_micros())
    .bind(10_i64 * 1024 * 1024 * 1024)
    .fetch_one(&st.db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO key_attributes (user_id, kek_salt, encrypted_key, key_decryption_nonce, public_key, \
         encrypted_secret_key, secret_key_decryption_nonce, mem_limit, ops_limit, \
         master_key_encrypted_with_recovery_key, master_key_decryption_nonce, \
         recovery_key_encrypted_with_master_key, recovery_key_decryption_nonce, creation_time) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 1)",
    )
    .bind(id)
    .bind(rb64(16))
    .bind(rb64(48))
    .bind(rb64(24))
    .bind(rb64(32))
    .bind(rb64(48))
    .bind(rb64(24))
    .bind(268_435_456_i64)
    .bind(4_i64)
    .bind(rb64(48))
    .bind(rb64(24))
    .bind(rb64(48))
    .bind(rb64(24))
    .execute(&st.db.pool)
    .await
    .unwrap();
    let token = photovault_server::crypto::b64url(&random_bytes(32));
    sqlx::query(
        "INSERT INTO tokens (token_hash, user_id, app, creation_time, last_used_at, ip, user_agent) \
         VALUES ($1, $2, 'photos', $3, $3, '127.0.0.1', 'Dart/3.10 (dart:io)')",
    )
    .bind(photovault_server::auth::hash_token(&token))
    .bind(id)
    .bind(now_micros())
    .execute(&st.db.pool)
    .await
    .unwrap();
    App {
        base: srv.url.clone(),
        token: Some(token),
        id,
        email: email.to_string(),
    }
}

/// Another `photos` session for an existing user.
async fn user_token(srv: &TestServer, id: i64) -> String {
    let token = photovault_server::crypto::b64url(&random_bytes(32));
    sqlx::query(
        "INSERT INTO tokens (token_hash, user_id, app, creation_time, last_used_at, ip, user_agent) \
         VALUES ($1, $2, 'photos', $3, $3, '10.0.0.2', 'Dart/3.10 (dart:io)')",
    )
    .bind(photovault_server::auth::hash_token(&token))
    .bind(id)
    .bind(now_micros())
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    token
}

/// `CollectionsGateway.createCollection` → `response.data["collection"]["id"]`.
async fn album(app: &App) -> i64 {
    let v = app
        .ok(
            "POST",
            "/collections",
            Some(json!({
                "encryptedKey": rb64(48),
                "keyDecryptionNonce": rb64(24),
                "encryptedName": rb64(20),
                "nameDecryptionNonce": rb64(24),
                "type": "album",
                "attributes": {"version": 0},
                "magicMetadata": null,
            })),
        )
        .await;
    v["collection"]["id"].as_i64().expect("collection.id")
}

/// `FileUploadGateway.getUploadUrl` (v2: length + MD5 bound) then the `_s3Dio.put`.
async fn upload_object(app: &App, data: &[u8]) -> String {
    let md5 = md5_b64(data);
    let u = app
        .ok(
            "POST",
            "/files/upload-url",
            Some(json!({"contentLength": data.len(), "contentMD5": md5})),
        )
        .await;
    let url = u["url"].as_str().expect("url").to_string();
    let key = u["objectKey"].as_str().expect("objectKey").to_string();
    let len = data.len().to_string();
    let r = http(
        "PUT",
        &url,
        &[
            ("Content-Length", &len),
            ("Content-Type", "application/octet-stream"),
            ("Content-MD5", &md5),
        ],
        Some(data),
    )
    .await;
    assert_eq!(r.status, 200, "PUT object: {}", r.text());
    key
}

/// `FileUploadGateway.createFile` with the app's exact body.
async fn upload_file(app: &App, cid: i64, data: &[u8]) -> Value {
    let fk = upload_object(app, data).await;
    let tk = upload_object(app, b"thumbnail").await;
    app.ok(
        "POST",
        "/files",
        Some(json!({
            "collectionID": cid,
            "encryptedKey": rb64(48),
            "keyDecryptionNonce": rb64(24),
            "file": {"objectKey": fk, "decryptionHeader": rb64(24), "size": data.len()},
            "thumbnail": {"objectKey": tk, "decryptionHeader": rb64(24), "size": 9},
            "metadata": {"encryptedData": rb64(64), "decryptionHeader": rb64(24)},
        })),
    )
    .await
}

fn is_int(v: &Value) -> bool {
    v.as_i64().is_some()
}
fn is_str(v: &Value) -> bool {
    v.as_str().is_some()
}

// ---------------------------------------------------------------------------------
// Endpoint check / custom server.
// ---------------------------------------------------------------------------------

/// Developer settings `_ping` (`response.data['message'] == 'pong'`) and the Rust core's
/// `Api::ping` (`PingResponse { message: String, id: String }`, both required).
#[tokio::test]
async fn ping_has_message_and_id() {
    let srv = TestServer::start().await.unwrap();
    let r = App::anon(&srv).get("/ping").await;
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["message"], "pong");
    assert!(is_str(&v["id"]), "ping.id must be a string: {v}");
}

// ---------------------------------------------------------------------------------
// Body-less POSTs (Dio sends no body and no Content-Type when `data` is null).
// ---------------------------------------------------------------------------------

#[tokio::test]
async fn logout_without_body() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "logout@example.org").await;
    let r = app.post_empty("/users/logout").await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert_eq!(app.get("/users/details/v2").await.status, 401);
}

#[tokio::test]
async fn two_factor_disable_and_status_without_body() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "tfa@example.org").await;
    let v = app.ok("GET", "/users/two-factor/status", None).await;
    assert_eq!(v["status"], false);
    // museum's disable handler binds nothing; a body-less POST must not be a bind error.
    let r = app.post_empty("/users/two-factor/disable").await;
    assert!(
        r.status != 415 && r.status != 422,
        "{} {}",
        r.status,
        r.text()
    );
}

#[tokio::test]
async fn family_create_and_leave_without_body() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "fam@example.org").await;
    let r = app.post_empty("/family/create").await;
    assert!(
        r.status != 415 && r.status != 422 && !(r.status == 400 && r.json()["code"].is_null()),
        "{} {}",
        r.status,
        r.text()
    );
}

#[tokio::test]
async fn leave_collection_without_body() {
    let srv = TestServer::start().await.unwrap();
    let owner = user(&srv, "owner-leave@example.org").await;
    let viewer = user(&srv, "viewer-leave@example.org").await;
    let cid = album(&owner).await;
    let r = owner
        .post(
            "/collections/share",
            json!({"collectionID": cid, "email": viewer.email, "encryptedKey": rb64(80), "role": "VIEWER"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    assert!(r.json()["sharees"].is_array());
    let r = viewer
        .post_empty(&format!("/collections/leave/{cid}"))
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
}

#[tokio::test]
async fn referral_claim_code_in_query_without_body() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "ref@example.org").await;
    let r = app
        .post_empty("/storage-bonus/referral-claim?code=NOPE12")
        .await;
    // An API error with a code (unknown code), never a body/extractor rejection.
    assert!(
        r.status != 415 && r.status != 422,
        "{} {}",
        r.status,
        r.text()
    );
}

// ---------------------------------------------------------------------------------
// Users / account.
// ---------------------------------------------------------------------------------

/// `UserDetails.fromMap`: `email` String, `usage` int, `subscription` map (typed
/// `productID`/`paymentProvider`/`price`/`period` Strings, `storage`/`expiryTime` ints),
/// `familyData` null or map, `profileData`/`bonusData` nullable maps.
#[tokio::test]
async fn user_details_v2_shape() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "details@example.org").await;
    for q in ["?memoryCount=true", "?memoryCount=false"] {
        let v = app.ok("GET", &format!("/users/details/v2{q}"), None).await;
        assert_eq!(v["email"], "details@example.org");
        assert!(is_int(&v["usage"]), "{v}");
        let s = &v["subscription"];
        assert!(s.is_object(), "{v}");
        for k in ["productID", "paymentProvider", "price", "period"] {
            assert!(is_str(&s[k]), "subscription.{k}: {s}");
        }
        for k in ["storage", "expiryTime"] {
            assert!(is_int(&s[k]), "subscription.{k}: {s}");
        }
        assert!(v["familyData"].is_null() || v["familyData"]["members"].is_array());
        assert!(v["profileData"].is_null() || v["profileData"].is_object());
        assert!(v["bonusData"].is_null() || v["bonusData"]["storageBonuses"].is_array());
    }
}

/// `Sessions.fromMap`: `sessions` non-null; each `token`, `isCurrent`, `creationTime`,
/// `ip`, `ua`, `prettyUA`, `lastUsedTime`. `terminateSession` sends `?token=`.
#[tokio::test]
async fn sessions_list_and_terminate() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "sessions@example.org").await;
    // A second device's session (the app only offers to end other sessions; museum
    // refuses the current one with "use logout").
    let _other = user_token(&srv, app.id).await;
    let v = app.ok("GET", "/users/sessions", None).await;
    let list = v["sessions"].as_array().expect("sessions");
    assert_eq!(list.len(), 2);
    let s = list
        .iter()
        .find(|s| s["isCurrent"] == false)
        .expect("other session");
    for k in ["token", "ip", "ua", "prettyUA"] {
        assert!(is_str(&s[k]), "sessions[].{k}: {s}");
    }
    assert!(s["isCurrent"].is_boolean(), "{s}");
    assert!(
        is_int(&s["creationTime"]) && is_int(&s["lastUsedTime"]),
        "{s}"
    );
    let tok = s["token"].as_str().unwrap().to_string();
    let r = app.delete(&format!("/users/session?token={tok}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

/// `UsersGateway.getPublicKey` (404 → null) and `getPublicKeys` (array aligned with
/// `emails`, every entry cast `as String`; a 404 makes the app fall back to single lookups).
#[tokio::test]
async fn public_key_lookups() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "pk-a@example.org").await;
    let _b = user(&srv, "pk-b@example.org").await;
    let v = app
        .ok("GET", "/users/public-key?email=pk-b@example.org", None)
        .await;
    assert!(is_str(&v["publicKey"]));
    assert_eq!(
        app.get("/users/public-key?email=nobody@example.org")
            .await
            .status,
        404
    );
    let v = app
        .ok(
            "POST",
            "/users/public-keys",
            Some(json!({"emails": ["pk-b@example.org", "pk-a@example.org"]})),
        )
        .await;
    let keys = v["publicKeys"].as_array().unwrap();
    assert_eq!(keys.len(), 2);
    assert!(keys.iter().all(is_str));
    let r = app
        .post(
            "/users/public-keys",
            json!({"emails": ["pk-b@example.org", "nobody@example.org"]}),
        )
        .await;
    assert!(r.status == 404 || r.status == 200, "{}", r.status);
}

/// The Rust core maps `GET /users/srp/attributes` 404 → "SRP not configured".
#[tokio::test]
async fn srp_attributes_unknown_email_is_404() {
    let srv = TestServer::start().await.unwrap();
    let r = App::anon(&srv)
        .get("/users/srp/attributes?email=nobody@example.org")
        .await;
    assert_eq!(r.status, 404);
}

/// `send_otp` classifies `404 USER_NOT_REGISTERED` for a login OTT to an unknown email
/// (the core also sends `"mobile": true` only when set).
#[tokio::test]
async fn ott_login_unknown_email() {
    let srv = TestServer::start().await.unwrap();
    let r = App::anon(&srv)
        .post(
            "/users/ott",
            json!({"email": "nobody@example.org", "purpose": "login", "mobile": true}),
        )
        .await;
    // museum answers 200 for privacy unless configured otherwise; the core only needs
    // a coded error when it is an error.
    if r.status != 200 {
        assert_eq!(r.status, 404, "{}", r.text());
        assert_eq!(r.json()["code"], "USER_NOT_REGISTERED");
    }
}

/// `check_passkey_status`: 400 = keep polling, 404/410 = expired. An unknown session
/// must not answer 400 or the app polls forever.
#[tokio::test]
async fn passkey_get_token_unknown_session_is_not_400() {
    let srv = TestServer::start().await.unwrap();
    let r = App::anon(&srv)
        .get("/users/two-factor/passkeys/get-token?sessionID=00000000-0000-0000-0000-000000000000")
        .await;
    assert!(
        r.status == 404 || r.status == 410,
        "{} {}",
        r.status,
        r.text()
    );
}

/// `verify_totp`: 404/410 for an unknown session (not 400/401).
#[tokio::test]
async fn totp_verify_unknown_session() {
    let srv = TestServer::start().await.unwrap();
    let r = App::anon(&srv)
        .post(
            "/users/two-factor/verify",
            json!({"sessionID": "nope", "code": "123456"}),
        )
        .await;
    assert!(
        r.status == 404 || r.status == 410,
        "{} {}",
        r.status,
        r.text()
    );
}

/// `get_session_validity` (`hasSetKeys` bool, `keyAttributes` with u64/u32 limits).
#[tokio::test]
async fn session_validity_shape() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "validity@example.org").await;
    let v = app.ok("GET", "/users/session-validity/v2", None).await;
    assert_eq!(v["hasSetKeys"], true);
    let k = &v["keyAttributes"];
    for f in [
        "kekSalt",
        "encryptedKey",
        "keyDecryptionNonce",
        "publicKey",
        "encryptedSecretKey",
        "secretKeyDecryptionNonce",
    ] {
        assert!(is_str(&k[f]), "keyAttributes.{f}: {k}");
    }
    assert!(k["memLimit"].as_u64().is_some() && k["opsLimit"].as_u64().is_some());
}

/// `UsersGateway.getPaymentToken`, `updateEmailMFA`, the accounts token.
#[tokio::test]
async fn payment_token_email_mfa_accounts_token() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "tokens@example.org").await;
    let v = app.ok("GET", "/users/payment-token", None).await;
    assert!(is_str(&v["paymentToken"]), "{v}");
    let r = app
        .put("/users/email-mfa", json!({"isEnabled": true}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = app.ok("GET", "/users/accounts-token", None).await;
    assert!(
        is_str(&v["accountsUrl"]) && is_str(&v["accountsToken"]),
        "{v}"
    );
}

// ---------------------------------------------------------------------------------
// Upload, sync diff loops, download.
// ---------------------------------------------------------------------------------

/// `FileUploadGateway.validateUploadEligibility`, single-part v2 upload URLs,
/// `createFile`, then `DiffFetcher.getEncryptedFilesDiff` until `hasMore` is false.
#[tokio::test]
async fn upload_then_collection_diff_loop() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "upload@example.org").await;
    assert_eq!(app.get("/files/upload-eligibility").await.status, 200);
    let cid = album(&app).await;
    let f = upload_file(&app, cid, b"encrypted-file-bytes").await;
    let fid = f["id"].as_i64().expect("file.id");

    let mut since = 0;
    let mut seen = Vec::new();
    for _ in 0..5 {
        let v = app
            .ok(
                "GET",
                &format!("/collections/v2/diff?collectionID={cid}&sinceTime={since}"),
                None,
            )
            .await;
        let diff = v["diff"].as_array().expect("diff").clone();
        let has_more = v["hasMore"].as_bool().expect("hasMore bool");
        for item in &diff {
            assert!(is_int(&item["id"]) && is_int(&item["collectionID"]));
            assert!(is_int(&item["updationTime"]) && item["isDeleted"].is_boolean());
            if item["isDeleted"] == false {
                assert!(is_int(&item["ownerID"]));
                assert!(is_str(&item["encryptedKey"]) && is_str(&item["keyDecryptionNonce"]));
                assert!(is_str(&item["file"]["decryptionHeader"]));
                assert!(is_str(&item["thumbnail"]["decryptionHeader"]));
                assert!(is_str(&item["metadata"]["decryptionHeader"]));
                assert!(is_str(&item["metadata"]["encryptedData"]));
                assert!(item["info"].is_null() || is_int(&item["info"]["fileSize"]));
            }
            since = since.max(item["updationTime"].as_i64().unwrap());
            seen.push(item["id"].as_i64().unwrap());
        }
        if !has_more {
            break;
        }
    }
    assert_eq!(seen, vec![fid]);
    // Re-polling from the last updationTime returns nothing (no infinite loop).
    let v = app
        .ok(
            "GET",
            &format!("/collections/v2/diff?collectionID={cid}&sinceTime={since}"),
            None,
        )
        .await;
    assert_eq!(v["diff"].as_array().unwrap().len(), 0);
    assert_eq!(v["hasMore"], false);

    // `CollectionsGateway.getAll(sinceTime, source)` → `collections`.
    let v = app
        .ok("GET", "/collections/v2?sinceTime=0&source=sync", None)
        .await;
    let cols = v["collections"].as_array().expect("collections");
    let c = cols.iter().find(|c| c["id"] == cid).expect("album listed");
    assert!(
        // `isDeleted` is `omitempty` in museum; the app reads `map['isDeleted'] ?? false`.
        is_int(&c["owner"]["id"])
            && is_int(&c["updationTime"])
            && (c["isDeleted"].is_null() || c["isDeleted"].is_boolean()),
        "{c}"
    );

    // `FilesGateway.getFilesInfo` / `getFilesSize`.
    let v = app
        .ok("POST", "/files/info", Some(json!({"fileIDs": [fid]})))
        .await;
    assert_eq!(v["filesInfo"][0]["id"], fid);
    assert!(is_int(&v["filesInfo"][0]["fileInfo"]["fileSize"]), "{v}");
    let v = app
        .ok("POST", "/files/size", Some(json!({"fileIDs": [fid]})))
        .await;
    assert!(is_int(&v["size"]));
}

/// `MultiPartUploader`: `POST /files/multipart-upload-url` (20 MiB parts, part MD5s),
/// part PUTs with `Content-MD5` + octet-stream (ETag read from the `etag` header),
/// then `POST completeURL` with `text/xml` and the quotes stripped from every ETag.
#[tokio::test]
async fn multipart_upload_like_the_app() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "multipart@example.org").await;
    let cid = album(&app).await;
    let part_len: usize = 20 * 1024 * 1024;
    let data: Vec<u8> = (0..part_len + 1000).map(|i| (i % 251) as u8).collect();
    let parts: Vec<&[u8]> = data.chunks(part_len).collect();
    let md5s: Vec<String> = parts.iter().map(|p| md5_b64(p)).collect();
    let v = app
        .ok(
            "POST",
            "/files/multipart-upload-url",
            Some(json!({"contentLength": data.len(), "partLength": part_len, "partMd5s": md5s})),
        )
        .await;
    let raw = if v.get("urls").is_some() {
        &v["urls"]
    } else {
        &v
    };
    let object_key = raw["objectKey"].as_str().expect("objectKey").to_string();
    let part_urls: Vec<String> = raw["partURLs"]
        .as_array()
        .expect("partURLs")
        .iter()
        .map(|u| u.as_str().unwrap().to_string())
        .collect();
    let complete = raw["completeURL"]
        .as_str()
        .expect("completeURL")
        .to_string();
    assert_eq!(part_urls.len(), 2);
    let mut etags = Vec::new();
    for (i, url) in part_urls.iter().enumerate() {
        let len = parts[i].len().to_string();
        let r = http(
            "PUT",
            url,
            &[
                ("Content-Length", &len),
                ("Content-Type", "application/octet-stream"),
                ("Content-MD5", &md5s[i]),
            ],
            Some(parts[i]),
        )
        .await;
        assert_eq!(r.status, 200, "part {i}: {}", r.text());
        let etag = r.header("etag").expect("ETag header on part PUT");
        assert!(!etag.is_empty());
        etags.push(etag);
    }
    let mut xml = String::from("<?xml version=\"1.0\"?>\n<CompleteMultipartUpload>\n");
    for (i, e) in etags.iter().enumerate() {
        xml.push_str(&format!(
            "    <Part>\n        <PartNumber>{}</PartNumber>\n        <ETag>{}</ETag>\n    </Part>\n",
            i + 1,
            e
        ));
    }
    xml.push_str("</CompleteMultipartUpload>");
    let xml = xml.replace('"', "").replace("&quot;", "");
    let r = http(
        "POST",
        &complete,
        &[("Content-Type", "text/xml")],
        Some(xml.as_bytes()),
    )
    .await;
    assert_eq!(r.status, 200, "complete: {}", r.text());

    let tk = upload_object(&app, b"thumb").await;
    let f = app
        .ok(
            "POST",
            "/files",
            Some(json!({
                "collectionID": cid,
                "encryptedKey": rb64(48),
                "keyDecryptionNonce": rb64(24),
                "file": {"objectKey": object_key, "decryptionHeader": rb64(24), "size": data.len()},
                "thumbnail": {"objectKey": tk, "decryptionHeader": rb64(24), "size": 5},
                "metadata": {"encryptedData": rb64(64), "decryptionHeader": rb64(24)},
            })),
        )
        .await;
    assert!(is_int(&f["id"]));
}

/// `DownloadManager._resolveDownloadUrl`: v3 JSON `url` (or 404 → legacy), the legacy
/// `/files/download/{id}` with `followRedirects: false` must be a 3xx with `Location`,
/// and chunked downloads send `Range` to the signed URL.
#[tokio::test]
async fn download_urls_and_ranges() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "download@example.org").await;
    let cid = album(&app).await;
    let data = b"0123456789abcdefghij";
    let f = upload_file(&app, cid, data).await;
    let fid = f["id"].as_i64().unwrap();

    let v = app
        .ok("GET", &format!("/files/download/v3/{fid}"), None)
        .await;
    let url = v["url"].as_str().expect("v3 url").to_string();
    let r = http("GET", &url, &[("Range", "bytes=5-9")], None).await;
    assert_eq!(r.status, 206, "Range on signed URL");
    assert_eq!(r.body, b"56789");

    let v = app
        .ok("GET", &format!("/files/thumbnail/v3/{fid}"), None)
        .await;
    assert!(is_str(&v["url"]));

    let r = app.get(&format!("/files/download/{fid}")).await;
    assert!(
        (300..400).contains(&r.status),
        "legacy download must redirect, got {}",
        r.status
    );
    assert!(r.header("location").is_some_and(|l| !l.is_empty()));
}

/// `TrashGateway`: trash items, `/trash/v2/diff` (`diff`, `hasMore`), delete, empty.
#[tokio::test]
async fn trash_flow() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "trash@example.org").await;
    let cid = album(&app).await;
    let f = upload_file(&app, cid, b"to-be-trashed").await;
    let fid = f["id"].as_i64().unwrap();
    let r = app
        .post(
            "/files/trash",
            json!({"items": [{"fileID": fid, "collectionID": cid}]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = app.ok("GET", "/trash/v2/diff?sinceTime=0", None).await;
    let diff = v["diff"].as_array().unwrap();
    assert!(v["hasMore"].is_boolean());
    let t = diff
        .iter()
        .find(|t| t["file"]["id"] == fid)
        .expect("trashed");
    assert!(is_int(&t["updatedAt"]) && t["isDeleted"].is_boolean() && t["isRestored"].is_boolean());
    assert!(is_int(&t["deleteBy"]));
    let last = t["updatedAt"].as_i64().unwrap();
    let r = app
        .post("/trash/empty", json!({"lastUpdatedAt": last}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
}

/// `CollectionsGateway.deleteCollection`: both ids in the query.
#[tokio::test]
async fn delete_collection_v3_query() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "delcol@example.org").await;
    let cid = album(&app).await;
    let r = app
        .delete(&format!(
            "/collections/v3/{cid}?keepFiles=false&collectionID={cid}"
        ))
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
}

// ---------------------------------------------------------------------------------
// ML file data and streamable video previews.
// ---------------------------------------------------------------------------------

/// `FileDataService`: PUT mldata, batch fetch (`data`, and the app casts
/// `pendingIndexFileIDs`/`errFileIDs` `as List`, so they must be arrays, never null),
/// single fetch with `preferNoContent=true` (200/204/404), and the status-diff loop that
/// stops only when a page is empty.
#[tokio::test]
async fn ml_file_data_and_status_diff_loop() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "ml@example.org").await;
    let cid = album(&app).await;
    let f1 = upload_file(&app, cid, b"one").await["id"].as_i64().unwrap();
    let f2 = upload_file(&app, cid, b"two").await["id"].as_i64().unwrap();

    // Nothing indexed yet: lists present and empty or listing pending ids.
    let v = app
        .ok(
            "POST",
            "/files/data/fetch",
            Some(json!({"fileIDs": [f1, f2], "type": "mldata"})),
        )
        .await;
    assert!(v["data"].is_array(), "{v}");
    assert!(v["pendingIndexFileIDs"].is_array(), "{v}");
    assert!(v["errFileIDs"].is_array(), "{v}");

    let r = app
        .put(
            "/files/data",
            json!({"fileID": f1, "type": "mldata", "encryptedData": rb64(100), "decryptionHeader": rb64(24)}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = app
        .ok(
            "POST",
            "/files/data/fetch",
            Some(json!({"fileIDs": [f1, f2], "type": "mldata"})),
        )
        .await;
    let d = &v["data"][0];
    assert_eq!(d["fileID"], f1);
    assert!(
        is_str(&d["encryptedData"]) && is_str(&d["decryptionHeader"]),
        "{v}"
    );
    assert!(v["pendingIndexFileIDs"].is_array() && v["errFileIDs"].is_array());

    let r = app
        .get(&format!(
            "/files/data/fetch?fileID={f1}&type=mldata&preferNoContent=true"
        ))
        .await;
    assert_eq!(r.status, 200);
    assert!(is_str(&r.json()["data"]["encryptedData"]));
    let r = app
        .get(&format!(
            "/files/data/fetch?fileID={f2}&type=mldata&preferNoContent=true"
        ))
        .await;
    assert!(r.status == 204 || r.status == 404, "{}", r.status);

    let mut last = 0_i64;
    let mut pages = 0;
    loop {
        pages += 1;
        assert!(pages < 5, "status-diff never returned an empty page");
        let v = app
            .ok(
                "POST",
                "/files/data/status-diff",
                Some(json!({"lastUpdatedAt": last})),
            )
            .await;
        let diff = v["diff"].as_array().cloned().unwrap_or_default();
        if diff.is_empty() {
            break;
        }
        for e in &diff {
            assert!(is_int(&e["fileID"]) && is_int(&e["userID"]) && is_str(&e["type"]));
            assert!(is_int(&e["size"]), "FDStatus.size is a required int: {e}");
            last = last.max(e["updatedAt"].as_i64().expect("updatedAt"));
        }
    }
}

/// `uploadVideoPreview` (v1 `GET …/preview-upload-url` → PUT without Content-MD5, and
/// v2 `POST` with length + MD5), `putVideoData`, `getPreview`.
#[tokio::test]
async fn video_preview_upload_like_the_app() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "video@example.org").await;
    let cid = album(&app).await;
    let fid = upload_file(&app, cid, b"a-video").await["id"]
        .as_i64()
        .unwrap();
    let preview = b"encrypted-hls-preview-bytes";

    // v1
    let v = app
        .ok(
            "GET",
            &format!("/files/data/preview-upload-url?fileID={fid}&type=vid_preview"),
            None,
        )
        .await;
    let url = v["url"].as_str().expect("url").to_string();
    let object_id = v["objectID"].as_str().expect("objectID").to_string();
    let len = preview.len().to_string();
    let r = http("PUT", &url, &[("Content-Length", &len)], Some(preview)).await;
    assert_eq!(r.status, 200, "v1 preview PUT: {}", r.text());

    // v2
    let md5 = md5_b64(preview);
    let v = app
        .ok(
            "POST",
            "/files/data/preview-upload-url",
            Some(json!({"fileID": fid, "type": "vid_preview", "contentLength": preview.len(), "contentMD5": md5})),
        )
        .await;
    let url2 = v["url"].as_str().expect("url").to_string();
    let object_id2 = v["objectID"].as_str().expect("objectID").to_string();
    let r = http(
        "PUT",
        &url2,
        &[("Content-Length", &len), ("Content-MD5", &md5)],
        Some(preview),
    )
    .await;
    assert_eq!(r.status, 200, "v2 preview PUT: {}", r.text());
    let _ = object_id;

    let r = app
        .put(
            "/files/video-data",
            json!({
                "fileID": fid,
                "objectID": object_id2,
                "objectSize": preview.len(),
                "playlist": rb64(64),
                "playlistHeader": rb64(24),
            }),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = app
        .ok(
            "GET",
            &format!("/files/data/preview?fileID={fid}&type=vid_preview"),
            None,
        )
        .await;
    assert!(is_str(&v["url"]));
    let r = app
        .get(&format!(
            "/files/data/fetch?fileID={fid}&type=vid_preview&preferNoContent=true"
        ))
        .await;
    assert_eq!(r.status, 200);
    assert!(is_str(&r.json()["data"]["encryptedData"]));
}

// ---------------------------------------------------------------------------------
// User entities (people / cgroups / smart albums / library sharing).
// ---------------------------------------------------------------------------------

#[tokio::test]
async fn user_entity_flow() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "entity@example.org").await;
    let other = user(&srv, "entity-peer@example.org").await;
    assert_eq!(app.get("/user-entity/key?type=cgroup").await.status, 404);
    let r = app
        .post(
            "/user-entity/key",
            json!({"type": "cgroup", "encryptedKey": rb64(48), "header": rb64(24)}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let k = app.ok("GET", "/user-entity/key?type=cgroup", None).await;
    assert!(is_str(&k["encryptedKey"]) && is_str(&k["header"]) && k["type"] == "cgroup");

    let e = app
        .ok(
            "POST",
            "/user-entity/entity",
            Some(json!({"encryptedData": rb64(40), "header": rb64(24), "type": "cgroup"})),
        )
        .await;
    let id = e["id"].as_str().expect("id").to_string();
    assert!(is_int(&e["userID"]) && e["isDeleted"] == false);
    assert!(is_int(&e["createdAt"]) && is_int(&e["updatedAt"]));
    let v = app
        .ok(
            "GET",
            "/user-entity/entity/diff?sinceTime=0&limit=500&type=cgroup",
            None,
        )
        .await;
    assert!(
        v["diff"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d["id"] == id.as_str())
    );
    let u = app
        .ok(
            "PUT",
            "/user-entity/entity",
            Some(
                json!({"id": id, "encryptedData": rb64(40), "header": rb64(24), "type": "cgroup"}),
            ),
        )
        .await;
    assert!(is_int(&u["updatedAt"]));

    // Library sharing: custom id `ls_<owner>_<recipient>` on create, then optimistic
    // updates with `expectedUpdatedAt` (409 when stale).
    app.ok(
        "POST",
        "/user-entity/key",
        Some(json!({"type": "library_share", "encryptedKey": rb64(48), "header": rb64(24)})),
    )
    .await;
    let lid = format!("ls_{}_{}", app.id, other.id);
    let e = app
        .ok(
            "POST",
            "/user-entity/entity",
            Some(json!({"encryptedData": rb64(40), "id": lid, "header": rb64(24), "type": "library_share"})),
        )
        .await;
    assert_eq!(e["id"], lid.as_str());
    let at = e["updatedAt"].as_i64().unwrap();
    let u = app
        .ok(
            "PUT",
            "/user-entity/entity",
            Some(json!({"id": lid, "encryptedData": rb64(40), "header": rb64(24), "type": "library_share", "expectedUpdatedAt": at})),
        )
        .await;
    assert!(u["updatedAt"].as_i64().unwrap() > at);
    let r = app
        .put(
            "/user-entity/entity",
            json!({"id": lid, "encryptedData": rb64(40), "header": rb64(24), "type": "library_share", "expectedUpdatedAt": at}),
        )
        .await;
    assert_eq!(r.status, 409);

    let r = app.delete(&format!("/user-entity/entity?id={id}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

// ---------------------------------------------------------------------------------
// Legacy: trusted contacts and legacy kits (served by the Rust core, strict serde).
// ---------------------------------------------------------------------------------

/// `ente_legacy::add_contact` serializes `recoveryNoticeInDays: Option<i32>` as an
/// explicit `null` when unset; `info` needs all four arrays (non-null `Vec`s) and an
/// integer `recoveryNoticeInDays` on every record.
#[tokio::test]
async fn legacy_contacts_core_shapes() {
    let srv = TestServer::start().await.unwrap();
    let owner = user(&srv, "legacy-owner@example.org").await;
    let trusted = user(&srv, "legacy-trusted@example.org").await;
    let r = owner
        .post(
            "/emergency-contacts/add",
            json!({"email": trusted.email, "encryptedKey": rb64(80), "recoveryNoticeInDays": null}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());

    let v = owner.ok("GET", "/emergency-contacts/info", None).await;
    for k in [
        "contacts",
        "recoverSessions",
        "othersEmergencyContact",
        "othersRecoverySession",
    ] {
        assert!(v[k].is_array(), "info.{k} must be an array: {v}");
    }
    let c = &v["contacts"][0];
    assert!(is_int(&c["user"]["id"]) && is_str(&c["user"]["email"]));
    assert_eq!(c["emergencyContact"]["id"], trusted.id);
    assert_eq!(c["state"], "INVITED");
    assert!(is_int(&c["recoveryNoticeInDays"]));

    let r = trusted
        .post(
            "/emergency-contacts/update",
            json!({"userID": owner.id, "emergencyContactID": trusted.id, "state": "ACCEPTED"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = owner
        .post(
            "/emergency-contacts/update-recovery-notice",
            json!({"emergencyContactID": trusted.id, "recoveryNoticeInDays": 7}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());

    let r = trusted
        .post(
            "/emergency-contacts/start-recovery",
            json!({"userID": owner.id, "emergencyContactID": trusted.id}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = trusted.ok("GET", "/emergency-contacts/info", None).await;
    let s = &v["othersRecoverySession"][0];
    assert!(is_str(&s["id"]) && is_int(&s["waitTill"]) && is_int(&s["createdAt"]));
    assert!(["INITIATED", "WAITING", "READY"].contains(&s["status"].as_str().unwrap()));

    // Changing the notice while a session is active → coded ACTIVE_RECOVERY_SESSION.
    let r = owner
        .post(
            "/emergency-contacts/update-recovery-notice",
            json!({"emergencyContactID": trusted.id, "recoveryNoticeInDays": 14}),
        )
        .await;
    assert_eq!(r.json()["code"], "ACTIVE_RECOVERY_SESSION", "{}", r.text());
}

/// `ente_legacy` kit client: create (variant `1`), list (`kits`), download-content,
/// owner recovery-session (`session` nullable, `initiators` a non-null array),
/// update notice, block, delete.
#[tokio::test]
async fn legacy_kits_core_shapes() {
    let srv = TestServer::start().await.unwrap();
    let owner = user(&srv, "kit-owner@example.org").await;
    let kit_id = uuid::Uuid::new_v4().to_string();
    let auth_pk = b64(&random_bytes(32));
    let v = owner
        .ok(
            "POST",
            "/legacy-kits",
            Some(json!({
                "id": kit_id,
                "variant": 1,
                "noticePeriodInHours": 168,
                "encryptedRecoveryBlob": rb64(72),
                "authPublicKey": auth_pk,
                "encryptedOwnerBlob": rb64(200),
            })),
        )
        .await;
    assert_eq!(v["id"], kit_id.as_str());
    assert_eq!(v["variant"], 1);
    assert_eq!(v["noticePeriodInHours"], 168);
    assert!(
        is_str(&v["legacyUrl"]) && is_str(&v["encryptedOwnerBlob"]),
        "{v}"
    );
    assert!(is_int(&v["createdAt"]) && is_int(&v["updatedAt"]));

    let v = owner.ok("GET", "/legacy-kits", None).await;
    assert_eq!(v["kits"][0]["id"], kit_id.as_str());
    assert!(is_str(&v["kits"][0]["legacyUrl"]));

    let v = owner
        .ok(
            "GET",
            &format!("/legacy-kits/{kit_id}/download-content"),
            None,
        )
        .await;
    assert_eq!(v["id"], kit_id.as_str());
    assert_eq!(v["variant"], 1);
    assert!(is_str(&v["encryptedOwnerBlob"]));

    let v = owner
        .ok(
            "GET",
            &format!("/legacy-kits/{kit_id}/recovery-session"),
            None,
        )
        .await;
    assert!(
        v["initiators"].is_array(),
        "initiators must be an array: {v}"
    );

    let r = owner
        .post(
            "/legacy-kits/update-recovery-notice",
            json!({"kitID": kit_id, "noticePeriodInHours": 24}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = owner
        .post("/legacy-kits/block-recovery", json!({"kitID": kit_id}))
        .await;
    // museum: no active session → 404 (the app only offers "block" during a session).
    assert_eq!(r.status, 404, "{}", r.text());
    let r = owner.delete(&format!("/legacy-kits/{kit_id}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

// ---------------------------------------------------------------------------------
// Family.
// ---------------------------------------------------------------------------------

/// `UsersGateway.updateFamilyMemberStorage` sends `"storageLimit": null` to clear it;
/// the member ids it passes are the `familyData.members[].id` strings.
#[tokio::test]
async fn family_modify_storage_null_limit() {
    let srv = TestServer::start().await.unwrap();
    let admin = user(&srv, "fam-admin@example.org").await;
    let r = admin
        .post(
            "/family/modify-storage",
            json!({"id": uuid::Uuid::new_v4().to_string(), "storageLimit": null}),
        )
        .await;
    // Not a member: museum's 404, never a JSON-binding rejection (400) of the null.
    assert!(
        r.status == 200 || r.status == 404,
        "{} {}",
        r.status,
        r.text()
    );
}

// ---------------------------------------------------------------------------------
// Cast.
// ---------------------------------------------------------------------------------

/// `CastGateway`: device lookup by code, publish payload with a client `castToken`
/// (optional `deviceID` back), list sessions (`devices[]` with `collectionID`,
/// `deviceIP`, `deviceID`, `lastUsedAt` µs int, `deviceName`), revoke one, revoke all.
#[tokio::test]
async fn cast_pairing_and_sessions() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "cast@example.org").await;
    let cid = album(&app).await;
    let tv = App::anon(&srv);
    let v = tv
        .post("/cast/device-info", json!({"publicKey": rb64(32)}))
        .await
        .json();
    let code = v["deviceCode"].as_str().expect("deviceCode").to_string();

    assert_eq!(app.get("/cast/device-info/NOPE00").await.status, 404);
    let v = app
        .ok("GET", &format!("/cast/device-info/{code}"), None)
        .await;
    assert!(is_str(&v["publicKey"]));

    let cast_token = photovault_server::crypto::b64url(&random_bytes(24));
    let r = app
        .post(
            "/cast/cast-data",
            json!({"deviceCode": code, "encPayload": rb64(120), "collectionID": cid, "castToken": cast_token}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());

    let v = app.ok("GET", "/cast/device-info", None).await;
    let devices = v["devices"].as_array().expect("devices");
    assert_eq!(devices.len(), 1, "{v}");
    let d = &devices[0];
    assert_eq!(d["collectionID"], cid);
    assert!(is_str(&d["deviceIP"]) && is_str(&d["deviceID"]), "{d}");
    assert!(is_int(&d["lastUsedAt"]), "{d}");
    assert!(d["deviceName"].is_null() || is_str(&d["deviceName"]));
    let device_id = d["deviceID"].as_str().unwrap().to_string();
    let r = app.delete(&format!("/cast/device-info/{device_id}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = app.delete("/cast/revoke-all-tokens").await;
    assert_eq!(r.status, 200, "{}", r.text());
}

// ---------------------------------------------------------------------------------
// Memories share.
// ---------------------------------------------------------------------------------

/// `MemoryShareService`: create (`type` share/lane, `memoryHash`, `files[]` with
/// `position`), `MemoryShare.fromJson` (`id` int, `encryptedKey`, `keyDecryptionNonce`,
/// `accessToken`, `createdAt` int, `url` Strings required), list, public files via
/// `X-Auth-Access-Token`, delete.
#[tokio::test]
async fn memory_share_like_the_app() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "memories@example.org").await;
    let cid = album(&app).await;
    let fid = upload_file(&app, cid, b"memory").await["id"]
        .as_i64()
        .unwrap();
    for kind in ["share", "lane"] {
        let v = app
            .ok(
                "POST",
                "/memory-share",
                Some(json!({
                    "type": kind,
                    "metadataCipher": rb64(40),
                    "metadataNonce": rb64(24),
                    "encryptedKey": rb64(48),
                    "keyDecryptionNonce": rb64(24),
                    "memoryHash": format!("hash-{kind}"),
                    "files": [{"fileID": fid, "position": 0, "encryptedKey": rb64(48), "keyDecryptionNonce": rb64(24)}],
                })),
            )
            .await;
        let m = &v["memoryShare"];
        assert!(is_int(&m["id"]) && is_int(&m["createdAt"]), "{m}");
        for k in ["encryptedKey", "keyDecryptionNonce", "accessToken", "url"] {
            assert!(is_str(&m[k]), "memoryShare.{k}: {m}");
        }
        assert_eq!(m["type"], kind);
        let token = m["accessToken"].as_str().unwrap().to_string();
        let r = App::anon(&srv)
            .send(
                "GET",
                "/public-memory/files",
                None,
                &[("X-Auth-Access-Token", &token)],
            )
            .await;
        assert_eq!(r.status, 200, "{}", r.text());
        assert_eq!(r.json()["files"][0]["file"]["id"], fid, "{}", r.text());
        let id = m["id"].as_i64().unwrap();
        let v = app.ok("GET", "/memory-share", None).await;
        assert!(
            v["memoryShares"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["id"] == id)
        );
        let r = app.delete(&format!("/memory-share/{id}")).await;
        assert_eq!(r.status, 200, "{}", r.text());
    }
}

// ---------------------------------------------------------------------------------
// Sharing, public links, misc.
// ---------------------------------------------------------------------------------

/// `CollectionShareGateway`: create/update link (`result`), sharees, delete link.
#[tokio::test]
async fn public_link_management() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "links@example.org").await;
    let cid = album(&app).await;
    let v = app
        .ok(
            "POST",
            "/collections/share-url",
            Some(json!({"collectionID": cid})),
        )
        .await;
    let res = &v["result"];
    assert!(
        is_str(&res["url"]) && is_int(&res["deviceLimit"]) && is_int(&res["validTill"]),
        "{v}"
    );
    assert!(res["enableDownload"].is_boolean() && res["passwordEnabled"].is_boolean());
    let v = app
        .ok(
            "PUT",
            "/collections/share-url",
            Some(json!({"collectionID": cid, "enableCollect": true})),
        )
        .await;
    assert_eq!(v["result"]["enableCollect"], true);
    let v = app
        .ok(
            "GET",
            &format!("/collections/sharees?collectionID={cid}"),
            None,
        )
        .await;
    assert!(v["sharees"].is_array());
    let r = app.delete(&format!("/collections/share-url/{cid}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

/// `PushGateway.registerToken` always sends `apnsToken` and `notificationKey`, as
/// explicit `null` on Android. museum's Go `string`/`[]byte` fields decode `null` as
/// the zero value; a serde `String` rejects it, so the app gets a 400.
#[tokio::test]
async fn push_token_with_nulls() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "push@example.org").await;
    let r = app
        .post(
            "/push/token",
            json!({"fcmToken": "fcm-abc", "platform": "android", "apnsToken": null, "notificationKey": null}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
}

/// `FlagService` (`RemoteFlags.fromMap`), billing (`BillingPlans`, `Subscription`),
/// storage bonus views.
#[tokio::test]
async fn flags_billing_and_bonus_views() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "views@example.org").await;
    let v = app.ok("GET", "/remote-store/feature-flags", None).await;
    assert!(v.is_object());
    let v = app.ok("GET", "/billing/user-plans", None).await;
    assert!(v["plans"].is_array(), "{v}");
    let v = app.ok("GET", "/billing/subscription", None).await;
    assert!(is_str(&v["subscription"]["productID"]), "{v}");
    let v = app.ok("GET", "/storage-bonus/referral-view", None).await;
    assert!(v["planInfo"].is_object() && is_str(&v["code"]), "{v}");
    let v = app.ok("GET", "/storage-bonus/details", None).await;
    assert!(
        v["bonuses"].is_array() || v["referralStats"].is_array(),
        "{v}"
    );
}

/// `CollectionsGateway`: delete suggestions and rejections (collection actions).
#[tokio::test]
async fn collection_actions_views() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "actions@example.org").await;
    let v = app
        .ok("GET", "/collection-actions/pending-remove", None)
        .await;
    assert!(v["actions"].is_array(), "{v}");
    let v = app
        .ok("GET", "/collection-actions/delete-suggestions", None)
        .await;
    assert!(v["actions"].is_array(), "{v}");
    let r = app
        .post(
            "/collection-actions/reject-delete-suggestions",
            json!({"fileIDs": [1]}),
        )
        .await;
    assert!(
        r.status == 200 || r.json()["code"].is_string(),
        "{}",
        r.text()
    );
}

// ---------------------------------------------------------------------------------
// Comments / reactions (client-generated ids), account deletion, 2FA views, contacts.
// ---------------------------------------------------------------------------------

/// `SocialGateway`: the app generates `cmt_`/`rct_` + 21-char ids and expects the same
/// `id` back; diff items need `id` String, `collectionID`/`userID`/`createdAt`/
/// `updatedAt` ints.
#[tokio::test]
async fn social_client_ids_and_diffs() {
    let srv = TestServer::start().await.unwrap();
    let owner = user(&srv, "social-owner@example.org").await;
    let viewer = user(&srv, "social-viewer@example.org").await;
    let cid = album(&owner).await;
    let r = owner
        .post(
            "/collections/share",
            json!({"collectionID": cid, "email": viewer.email, "encryptedKey": rb64(80), "role": "VIEWER"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let fid = upload_file(&owner, cid, b"social").await["id"]
        .as_i64()
        .unwrap();
    let cmt = "cmt_0123456789ABCDEFGHIJK";
    let v = viewer
        .ok(
            "POST",
            "/comments",
            Some(json!({"id": cmt, "collectionID": cid, "cipher": rb64(40), "nonce": rb64(24), "fileID": fid})),
        )
        .await;
    assert_eq!(v["id"], cmt);
    let rct = "rct_0123456789ABCDEFGHIJK";
    let v = owner
        .ok(
            "PUT",
            "/reactions",
            Some(json!({"id": rct, "collectionID": cid, "cipher": rb64(116), "nonce": rb64(24), "fileID": fid, "commentID": cmt})),
        )
        .await;
    assert_eq!(v["id"], rct);
    let r = viewer
        .put(
            &format!("/comments/{cmt}"),
            json!({"cipher": rb64(40), "nonce": rb64(24)}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());

    let v = owner
        .ok(
            "GET",
            &format!("/social/diff?collectionID={cid}&limit=100"),
            None,
        )
        .await;
    let c = &v["comments"][0];
    assert_eq!(c["id"], cmt);
    assert!(is_int(&c["collectionID"]) && is_int(&c["userID"]), "{c}");
    assert!(is_int(&c["createdAt"]) && is_int(&c["updatedAt"]), "{c}");
    let rr = &v["reactions"][0];
    assert_eq!(rr["id"], rct);
    assert!(is_int(&rr["userID"]) && is_int(&rr["createdAt"]), "{rr}");
    let v = owner
        .ok(
            "GET",
            &format!("/comments/diff?collectionID={cid}&sinceTime=0&limit=50"),
            None,
        )
        .await;
    assert!(v["comments"].is_array());
    let v = owner
        .ok(
            "GET",
            &format!("/reactions/diff?collectionID={cid}&sinceTime=0"),
            None,
        )
        .await;
    assert!(v["reactions"].is_array());
    let v = owner
        .ok("GET", "/comments-reactions/updated-at", None)
        .await;
    let u = v["updates"].as_array().expect("updates");
    assert!(u.iter().any(|u| u["collectionID"] == cid), "{v}");
    let v = owner
        .ok(
            "GET",
            &format!("/social/anon-profiles?collectionID={cid}"),
            None,
        )
        .await;
    assert!(v.is_object());
    let r = owner.delete(&format!("/reactions/{rct}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = viewer.delete(&format!("/comments/{cmt}")).await;
    assert_eq!(r.status, 200, "{}", r.text());
}

/// `ente_accounts::deletion`: summary counts (required i64s) and the challenge
/// (`encryptedChallenge`, sealed to the user's public key).
#[tokio::test]
async fn account_deletion_views() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "deletion@example.org").await;
    let v = app.ok("GET", "/users/deletion-summary", None).await;
    for k in [
        "photosAndVideosCount",
        "authenticatorCodesCount",
        "lockerRecordsCount",
    ] {
        assert!(is_int(&v[k]), "{k}: {v}");
    }
}

/// `setup_two_factor` (`secretCode`, `qrCode`), `get_two_factor_recovery_status`
/// (`isPasskeyRecoveryEnabled`), passkey list.
#[tokio::test]
async fn two_factor_views() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "twofa-views@example.org").await;
    let v = app
        .ok("POST", "/users/two-factor/setup", Some(json!({})))
        .await;
    assert!(is_str(&v["secretCode"]) && is_str(&v["qrCode"]), "{v}");
    let v = app
        .ok("GET", "/users/two-factor/recovery-status", None)
        .await;
    assert!(v["isPasskeyRecoveryEnabled"].is_boolean(), "{v}");
}

/// `ente_contacts`: the root key is `user-entity` type `contact`; a second create must
/// be 409 (the core then re-reads it), and `/contacts/diff` returns `diff`.
#[tokio::test]
async fn contacts_root_key_conflict_and_diff() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "contacts-core@example.org").await;
    let body = json!({"type": "contact", "encryptedKey": rb64(48), "header": rb64(24)});
    assert_eq!(app.post("/user-entity/key", body.clone()).await.status, 200);
    // Same key again is idempotent (museum); a different one is 409.
    assert_eq!(app.post("/user-entity/key", body).await.status, 200);
    let other = json!({"type": "contact", "encryptedKey": rb64(48), "header": rb64(24)});
    assert_eq!(app.post("/user-entity/key", other).await.status, 409);
    let v = app.ok("GET", "/user-entity/key?type=contact", None).await;
    assert!(is_str(&v["encryptedKey"]) && is_str(&v["header"]));
    let v = app
        .ok("GET", "/contacts/diff?sinceTime=0&limit=500", None)
        .await;
    assert!(v["diff"].is_array(), "{v}");
}

// ---------------------------------------------------------------------------------
// File edits, magic metadata (409 on stale version), copy, remove, duplicates.
// ---------------------------------------------------------------------------------

fn mmd(version: i64) -> Value {
    json!({"version": version, "count": 1, "data": rb64(40), "header": rb64(24)})
}

/// `FileMagicService` retries on 409; `UploadTransport` reads `id`, `updationTime`,
/// `ownerID` from `POST /files` and `id`, `updationTime` from `PUT /files/update`.
#[tokio::test]
async fn file_edits_and_magic_metadata() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "edits@example.org").await;
    let cid = album(&app).await;
    let cid2 = album(&app).await;
    let f = upload_file(&app, cid, b"original").await;
    let fid = f["id"].as_i64().unwrap();
    assert!(is_int(&f["updationTime"]) && f["ownerID"] == app.id);

    let r = app
        .put(
            "/files/magic-metadata",
            json!({"metadataList": [{"id": fid, "magicMetadata": mmd(1)}]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = app
        .put(
            "/files/magic-metadata",
            json!({"metadataList": [{"id": fid, "magicMetadata": mmd(1)}]}),
        )
        .await;
    assert_eq!(r.status, 409, "stale version: {}", r.text());
    let r = app
        .put(
            "/files/public-magic-metadata",
            json!({"metadataList": [{"id": fid, "magicMetadata": mmd(1)}]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = app
        .put(
            "/collections/magic-metadata",
            json!({"id": cid, "magicMetadata": mmd(1)}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = app
        .put(
            "/collections/public-magic-metadata",
            json!({"id": cid, "magicMetadata": mmd(1)}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());

    // Edited file re-upload.
    let fk = upload_object(&app, b"edited-bytes").await;
    let tk = upload_object(&app, b"thumb2").await;
    let u = app
        .ok(
            "PUT",
            "/files/update",
            Some(json!({
                "id": fid,
                "file": {"objectKey": fk, "decryptionHeader": rb64(24), "size": 12},
                "thumbnail": {"objectKey": tk, "decryptionHeader": rb64(24), "size": 6},
                "metadata": {"encryptedData": rb64(64), "decryptionHeader": rb64(24)},
            })),
        )
        .await;
    assert_eq!(u["id"], fid);
    assert!(is_int(&u["updationTime"]));

    // Add to a second album, copy, remove, rename.
    let item = json!({"id": fid, "encryptedKey": rb64(48), "keyDecryptionNonce": rb64(24)});
    let r = app
        .post(
            "/collections/add-files",
            json!({"collectionID": cid2, "files": [item.clone()]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    // museum's v3 remove-files refuses the owner's own files (the app moves or trashes
    // those and only "removes" other people's files).
    let r = app
        .post(
            "/collections/v3/remove-files",
            json!({"collectionID": cid2, "fileIDs": [fid]}),
        )
        .await;
    assert_eq!(r.status, 400, "{}", r.text());
    let r = app
        .post(
            "/collections/rename",
            json!({"collectionID": cid, "encryptedName": rb64(20), "nameDecryptionNonce": rb64(24)}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = app.ok("GET", &format!("/collections/{cid}"), None).await;
    assert_eq!(v["collection"]["id"], cid);

    let v = app.ok("GET", "/files/duplicates", None).await;
    assert!(
        v["duplicates"].is_array() || v["duplicates"].is_null(),
        "{v}"
    );
}

/// `CollectionFilesGateway.copyFiles` (a sharee copies an owner's file into their own
/// album) reads `oldToNewFileIDMap` with string keys.
#[tokio::test]
async fn copy_shared_file_into_own_album() {
    let srv = TestServer::start().await.unwrap();
    let owner = user(&srv, "copy-owner@example.org").await;
    let viewer = user(&srv, "copy-viewer@example.org").await;
    let cid = album(&owner).await;
    let fid = upload_file(&owner, cid, b"copy-me").await["id"]
        .as_i64()
        .unwrap();
    let r = owner
        .post(
            "/collections/share",
            json!({"collectionID": cid, "email": viewer.email, "encryptedKey": rb64(80), "role": "VIEWER"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let dst = album(&viewer).await;
    let v = viewer
        .ok(
            "POST",
            "/files/copy",
            Some(json!({
                "dstCollectionID": dst,
                "srcCollectionID": cid,
                "files": [{"id": fid, "encryptedKey": rb64(48), "keyDecryptionNonce": rb64(24)}],
            })),
        )
        .await;
    let map = v["oldToNewFileIDMap"]
        .as_object()
        .expect("oldToNewFileIDMap");
    assert!(is_int(&map[&fid.to_string()]), "{v}");
}

/// `StorageBonusGateway.changeCode`: code in the query *and* the body.
#[tokio::test]
async fn storage_bonus_change_code() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "bonus-code@example.org").await;
    let r = app
        .post(
            "/storage-bonus/change-code?code=MYCODE42",
            json!({"code": "MYCODE42"}),
        )
        .await;
    assert!(
        r.status == 200 || r.json()["code"].is_string(),
        "{} {}",
        r.status,
        r.text()
    );
    if r.status == 200 {
        let v = app.ok("GET", "/storage-bonus/referral-view", None).await;
        assert_eq!(v["code"], "MYCODE42");
    }
}

/// `UserDetails.fromMap` → `FamilyData.fromMap` / `FamilyMember.fromMap`: `members[]`
/// with `id` String, `email`, `usage` int, `isAdmin` bool, `status`, `storageLimit`;
/// `storage`/`expiryTime` ints.
#[tokio::test]
async fn family_data_shape_after_create() {
    let srv = TestServer::start().await.unwrap();
    let admin = user(&srv, "family-admin@example.org").await;
    let r = admin.post_empty("/family/create").await;
    if r.status != 200 {
        // museum: a free plan cannot host a family (402); nothing more to check.
        assert_eq!(r.status, 402, "{}", r.text());
        return;
    }
    let v = admin
        .ok("GET", "/users/details/v2?memoryCount=true", None)
        .await;
    let fd = &v["familyData"];
    let m = &fd["members"][0];
    assert!(is_str(&m["id"]) && is_str(&m["email"]), "{fd}");
    assert!(is_int(&m["usage"]) && m["isAdmin"].is_boolean(), "{fd}");
    assert!(is_int(&fd["storage"]) && is_int(&fd["expiryTime"]), "{fd}");
    let v = admin.ok("GET", "/family/members", None).await;
    assert!(v["members"].is_array(), "{v}");
}

/// Library sharing: `shareBulk`/`unshareBulk` (`source` manual/automatic) parse
/// `results[]` of `{collectionID, status}` with snake_case statuses; `shareBatch`
/// returns `sharees`.
#[tokio::test]
async fn bulk_and_batch_sharing() {
    let srv = TestServer::start().await.unwrap();
    let owner = user(&srv, "bulk-owner@example.org").await;
    let peer = user(&srv, "bulk-peer@example.org").await;
    let peer2 = user(&srv, "bulk-peer2@example.org").await;
    let c1 = album(&owner).await;
    let c2 = album(&owner).await;
    let v = owner
        .ok(
            "POST",
            "/collections/share/bulk",
            Some(json!({
                "recipientUserID": peer.id,
                "recipientEmail": peer.email,
                "source": "manual",
                "collections": [
                    {"collectionID": c1, "encryptedKey": rb64(80), "role": "VIEWER"},
                    {"collectionID": c2, "encryptedKey": rb64(80), "role": "VIEWER"},
                ],
            })),
        )
        .await;
    let results = v["results"].as_array().expect("results");
    assert_eq!(results.len(), 2, "{v}");
    let known = [
        "shared",
        "already_shared",
        "unshared",
        "already_unshared",
        "not_shared",
        "blocked_previous_removal",
        "failed",
    ];
    for r in results {
        assert!(is_int(&r["collectionID"]), "{r}");
        assert!(known.contains(&r["status"].as_str().unwrap_or("")), "{r}");
    }
    assert!(results.iter().all(|r| r["status"] == "shared"), "{v}");
    let v = owner
        .ok(
            "POST",
            "/collections/unshare/bulk",
            Some(
                json!({"recipientUserID": peer.id, "source": "manual", "collectionIDs": [c1, c2]}),
            ),
        )
        .await;
    assert!(
        v["results"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["status"] == "unshared"),
        "{v}"
    );

    let v = owner
        .ok(
            "POST",
            "/collections/share/batch",
            Some(json!({"shares": [
                {"collectionID": c1, "email": peer2.email, "encryptedKey": rb64(80), "role": "COLLABORATOR"},
            ]})),
        )
        .await;
    assert!(
        v["sharees"]
            .as_array()
            .unwrap()
            .iter()
            .any(|u| u["id"] == peer2.id),
        "{v}"
    );
}

/// Collaboration: a collaborator uploads into the owner's album; the owner suggests
/// deleting it (`suggestDelete`); the collaborator sees it in
/// `/collection-actions/delete-suggestions` and rejects it; the sharee updates its own
/// `sharee-magic-metadata`.
#[tokio::test]
async fn collaborator_suggest_delete_and_sharee_metadata() {
    let srv = TestServer::start().await.unwrap();
    let owner = user(&srv, "collab-owner@example.org").await;
    let collab = user(&srv, "collab-peer@example.org").await;
    let cid = album(&owner).await;
    let r = owner
        .post(
            "/collections/share",
            json!({"collectionID": cid, "email": collab.email, "encryptedKey": rb64(80), "role": "COLLABORATOR"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    // The app uploads into the collaborator's own collection, then adds to the album.
    let own = album(&collab).await;
    let fid = upload_file(&collab, own, b"collab-file").await["id"]
        .as_i64()
        .unwrap();
    let r = collab
        .post(
            "/collections/add-files",
            json!({"collectionID": cid, "files": [{"id": fid, "encryptedKey": rb64(48), "keyDecryptionNonce": rb64(24)}]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = owner
        .post(
            "/collections/suggest-delete",
            json!({"collectionID": cid, "fileIDs": [fid]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = collab
        .ok("GET", "/collection-actions/delete-suggestions", None)
        .await;
    let a = v["actions"].as_array().unwrap();
    assert!(a.iter().any(|a| a["fileID"] == fid), "{v}");
    let r = collab
        .post(
            "/collection-actions/reject-delete-suggestions",
            json!({"fileIDs": [fid]}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = collab
        .put(
            "/collections/sharee-magic-metadata",
            json!({"id": cid, "magicMetadata": mmd(1)}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
}

/// `CollectionsGateway.joinViaLink` with the public-link headers the app builds
/// (`X-Auth-Access-Token`), after `createShareUrl(enableJoin: true)`.
#[tokio::test]
async fn join_album_via_link() {
    let srv = TestServer::start().await.unwrap();
    let owner = user(&srv, "join-owner@example.org").await;
    let joiner = user(&srv, "join-peer@example.org").await;
    let cid = album(&owner).await;
    let v = owner
        .ok(
            "POST",
            "/collections/share-url",
            Some(json!({"collectionID": cid, "enableCollect": false, "enableJoin": true, "enableComment": true})),
        )
        .await;
    let url = v["result"]["url"].as_str().unwrap().to_string();
    let token = url::Url::parse(&url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "t")
        .map(|(_, v)| v.to_string())
        .expect("t= in link url");
    let r = App::anon(&srv)
        .send(
            "GET",
            "/public-collection/info",
            None,
            &[("X-Auth-Access-Token", &token)],
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = joiner
        .send(
            "POST",
            "/collections/join-link",
            Some(json!({"collectionID": cid, "encryptedKey": rb64(80)})),
            &[("X-Auth-Access-Token", &token)],
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = joiner
        .ok("GET", "/collections/v2?sinceTime=0&source=sync", None)
        .await;
    assert!(
        v["collections"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == cid)
    );
}

/// `UsersGateway.setRecoveryKey` and core `configure_passkey_recovery`; legacy
/// `/files/preview/{id}` thumbnails (non-worker path).
#[tokio::test]
async fn recovery_key_passkey_recovery_and_legacy_thumbnail() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "recovery@example.org").await;
    // The app sets the recovery key only when it is missing (museum answers 500 when it
    // is already set), so clear it first.
    sqlx::query(
        "UPDATE key_attributes SET master_key_encrypted_with_recovery_key = '', master_key_decryption_nonce = '', \
         recovery_key_encrypted_with_master_key = '', recovery_key_decryption_nonce = '' WHERE user_id = $1",
    )
    .bind(app.id)
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    let r = app
        .put(
            "/users/recovery-key",
            json!({
                "masterKeyEncryptedWithRecoveryKey": rb64(48),
                "masterKeyDecryptionNonce": rb64(24),
                "recoveryKeyEncryptedWithMasterKey": rb64(48),
                "recoveryKeyDecryptionNonce": rb64(24),
            }),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let r = app
        .post(
            "/users/two-factor/passkeys/configure-recovery",
            json!({"secret": uuid::Uuid::new_v4().to_string(), "userSecretCipher": rb64(48), "userSecretNonce": rb64(24)}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text());
    let v = app
        .ok("GET", "/users/two-factor/recovery-status", None)
        .await;
    assert_eq!(v["isPasskeyRecoveryEnabled"], true);

    let cid = album(&app).await;
    let fid = upload_file(&app, cid, b"thumbnailed").await["id"]
        .as_i64()
        .unwrap();
    let r = app.get(&format!("/files/preview/{fid}")).await;
    assert!(
        r.status == 200 || (300..400).contains(&r.status),
        "{} {}",
        r.status,
        r.text()
    );
}

/// `UsersGateway.leaveFamilyPlan` when not in a family: an API error, not a crash.
#[tokio::test]
async fn leave_family_when_not_member() {
    let srv = TestServer::start().await.unwrap();
    let app = user(&srv, "nofamily@example.org").await;
    let r = app.delete("/family/leave").await;
    assert!(r.status < 500, "{} {}", r.status, r.text());
}
