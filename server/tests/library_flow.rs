//! End-to-end flow for files, collections, sharing and trash over HTTP, on SQLite by
//! default or Postgres with `PHOTOVAULT_TEST_DATABASE_URL`.

use photovault_server::crypto::{b64, random_bytes, secretbox};
use photovault_server::testing::TestServer;
use serde_json::{Value, json};
use std::time::Duration;

struct Resp {
    status: u16,
    headers: String,
    body: Vec<u8>,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

fn dechunk(mut b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(i) = b.windows(2).position(|w| w == b"\r\n") else {
            return out;
        };
        let n = usize::from_str_radix(std::str::from_utf8(&b[..i]).unwrap().trim(), 16).unwrap();
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&b[i + 2..i + 2 + n]);
        b = &b[i + 2 + n + 2..];
    }
}

/// Tiny HTTP/1.1 client over TCP (no TLS stack needed).
async fn http(method: &str, url: &str, headers: &[(&str, &str)], body: Vec<u8>) -> Resp {
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
    let raw = out[idx..].to_vec();
    let body = if head.to_lowercase().contains("transfer-encoding: chunked") {
        dechunk(&raw)
    } else {
        raw
    };
    Resp {
        status,
        headers: head,
        body,
    }
}

struct Client {
    base: String,
    token: String,
}

impl Client {
    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Resp {
        let b = body.map(|v| v.to_string().into_bytes()).unwrap_or_default();
        http(
            method,
            &format!("{}{}", self.base, path),
            &[
                ("X-Auth-Token", &self.token),
                ("Content-Type", "application/json"),
            ],
            b,
        )
        .await
    }
    async fn ok(&self, method: &str, path: &str, body: Option<Value>) -> Value {
        let r = self.call(method, path, body).await;
        assert_eq!(
            r.status,
            200,
            "{method} {path}: {}",
            String::from_utf8_lossy(&r.body)
        );
        r.json()
    }
}

fn rb64(n: usize) -> String {
    b64(&random_bytes(n))
}

async fn exec(srv: &TestServer, sql: &str) {
    sqlx::query(sql).execute(&srv.state.db.pool).await.unwrap();
}

/// Insert a user with key attributes and a session token; returns a client for it.
async fn add_user(srv: &TestServer, id: i64, email: &str) -> Client {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "error".into()))
        .with_test_writer()
        .try_init();
    let (ct, nonce) = secretbox(email.as_bytes(), &srv.state.keys.encryption).unwrap();
    sqlx::query(
        "INSERT INTO users (user_id, encrypted_email, email_decryption_nonce, email_hash, creation_time, storage_quota)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(id)
    .bind(b64(&ct))
    .bind(b64(&nonce))
    .bind(photovault_server::library::users::email_hash(&srv.state, email))
    .bind(1i64)
    .bind(10i64 * 1024 * 1024 * 1024)
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    exec(
        srv,
        "CREATE TABLE IF NOT EXISTS key_attributes (user_id BIGINT PRIMARY KEY, kek_salt TEXT NOT NULL,
            kek_hash_bytes TEXT NOT NULL, encrypted_key TEXT NOT NULL, key_decryption_nonce TEXT NOT NULL,
            public_key TEXT NOT NULL, encrypted_secret_key TEXT NOT NULL, secret_key_decryption_nonce TEXT NOT NULL)",
    )
    .await;
    sqlx::query(
        "INSERT INTO key_attributes (user_id, kek_salt, kek_hash_bytes, encrypted_key, key_decryption_nonce,
            public_key, encrypted_secret_key, secret_key_decryption_nonce)
         VALUES ($1, 'x', 'x', 'x', 'x', 'x', 'x', 'x')",
    )
    .bind(id)
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    let token = format!("token-{id}-{}", hex::encode(random_bytes(8)));
    sqlx::query(
        "INSERT INTO tokens (token_hash, user_id, app, creation_time, last_used_at) VALUES ($1, $2, 'photos', 1, 1)",
    )
    .bind(photovault_server::auth::hash_token(&token))
    .bind(id)
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    Client {
        base: srv.url.clone(),
        token,
    }
}

async fn create_album(c: &Client) -> i64 {
    let v = c
        .ok(
            "POST",
            "/collections",
            Some(json!({
                "encryptedKey": rb64(48),
                "keyDecryptionNonce": rb64(24),
                "encryptedName": rb64(20),
                "nameDecryptionNonce": rb64(24),
                "type": "album",
                "attributes": {},
            })),
        )
        .await;
    v["collection"]["id"].as_i64().unwrap()
}

async fn put_object(url: &str, data: &[u8]) {
    let r = http("PUT", url, &[], data.to_vec()).await;
    assert_eq!(r.status, 200, "PUT object");
}

/// Upload two objects through `/files/upload-urls` and create a file from them.
async fn upload_file(c: &Client, cid: i64, size: usize) -> Value {
    let urls = c.ok("GET", "/files/upload-urls?count=2", None).await;
    let urls = urls["urls"].as_array().unwrap().clone();
    assert_eq!(urls.len(), 2);
    put_object(urls[0]["url"].as_str().unwrap(), &vec![7u8; size]).await;
    put_object(urls[1]["url"].as_str().unwrap(), b"thumb").await;
    c.ok(
        "POST",
        "/files",
        Some(file_body(
            cid,
            urls[0]["objectKey"].as_str().unwrap(),
            urls[1]["objectKey"].as_str().unwrap(),
        )),
    )
    .await
}

fn file_body(cid: i64, file_key: &str, thumb_key: &str) -> Value {
    json!({
        "collectionID": cid,
        "encryptedKey": rb64(48),
        "keyDecryptionNonce": rb64(24),
        "file": {"objectKey": file_key, "decryptionHeader": rb64(24)},
        "thumbnail": {"objectKey": thumb_key, "decryptionHeader": rb64(24)},
        "metadata": {"encryptedData": rb64(64), "decryptionHeader": rb64(24)},
    })
}

fn item(id: i64) -> Value {
    json!({"id": id, "encryptedKey": rb64(48), "keyDecryptionNonce": rb64(24)})
}

async fn diff(c: &Client, cid: i64, since: i64) -> (Vec<Value>, bool) {
    let v = c
        .ok(
            "GET",
            &format!("/collections/v2/diff?collectionID={cid}&sinceTime={since}"),
            None,
        )
        .await;
    (
        v["diff"].as_array().unwrap().clone(),
        v["hasMore"].as_bool().unwrap(),
    )
}

fn find(d: &[Value], id: i64) -> Option<Value> {
    d.iter().find(|f| f["id"].as_i64() == Some(id)).cloned()
}

async fn trash_diff(c: &Client) -> Vec<Value> {
    c.ok("GET", "/trash/v2/diff?sinceTime=0", None).await["diff"]
        .as_array()
        .unwrap()
        .clone()
}

#[tokio::test]
async fn library_flow() {
    let srv = TestServer::start().await.unwrap();
    let alice = add_user(&srv, 1, "alice@example.org").await;
    let bob = add_user(&srv, 2, "bob@example.org").await;

    // Unauthenticated and public endpoints.
    assert_eq!(
        http(
            "GET",
            &format!("{}/files/upload-urls?count=1", srv.url),
            &[],
            vec![]
        )
        .await
        .status,
        401
    );
    assert_eq!(
        http("GET", &format!("{}/files/count", srv.url), &[], vec![])
            .await
            .status,
        200
    );
    let r = alice.call("GET", "/files/upload-eligibility", None).await;
    assert_eq!((r.status, r.body.len()), (200, 0));

    // Album 1 with a file uploaded through single-part URLs.
    let album1 = create_album(&alice).await;
    let f1 = upload_file(&alice, album1, 11).await;
    let f1_id = f1["id"].as_i64().unwrap();
    assert_eq!(f1["file"]["size"], 11);
    assert_eq!(f1["thumbnail"]["size"], 5);
    assert_eq!(f1["ownerID"], 1);

    // Second file: multipart main object, v2 (length + MD5 bound) thumbnail URL.
    let mp = alice
        .ok("GET", "/files/multipart-upload-urls?count=2", None)
        .await;
    let mp = &mp["urls"];
    let parts = mp["partURLs"].as_array().unwrap();
    assert_eq!(parts.len(), 2);
    put_object(parts[0].as_str().unwrap(), b"hello ").await;
    put_object(parts[1].as_str().unwrap(), b"world").await;
    let xml = "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber></Part><Part><PartNumber>2</PartNumber></Part></CompleteMultipartUpload>";
    let r = http(
        "POST",
        mp["completeURL"].as_str().unwrap(),
        &[],
        xml.as_bytes().to_vec(),
    )
    .await;
    assert_eq!(r.status, 200);
    let md5 = b64(&random_bytes(16));
    let thumb = alice
        .ok(
            "POST",
            "/files/upload-url",
            Some(json!({"contentLength": 4, "contentMD5": md5})),
        )
        .await;
    put_object(thumb["url"].as_str().unwrap(), b"tiny").await;
    let f2 = alice
        .ok(
            "POST",
            "/files",
            Some(file_body(
                album1,
                mp["objectKey"].as_str().unwrap(),
                thumb["objectKey"].as_str().unwrap(),
            )),
        )
        .await;
    let f2_id = f2["id"].as_i64().unwrap();
    assert_eq!(f2["file"]["size"], 11);
    // Multipart v2 URL request validation.
    let r = alice
        .call(
            "POST",
            "/files/multipart-upload-url",
            Some(json!({"contentLength": 100, "partLength": 10})),
        )
        .await;
    assert_eq!(r.status, 400, "parts below 5 MiB are refused");
    let v = alice
        .ok(
            "POST",
            "/files/multipart-upload-url",
            Some(json!({"contentLength": 6 * 1024 * 1024, "partLength": 5 * 1024 * 1024})),
        )
        .await;
    assert_eq!(v["partURLs"].as_array().unwrap().len(), 2);

    // A staged upload can only be used once.
    let r = alice
        .call(
            "POST",
            "/files",
            Some(file_body(
                album1,
                mp["objectKey"].as_str().unwrap(),
                thumb["objectKey"].as_str().unwrap(),
            )),
        )
        .await;
    assert_ne!(r.status, 200);

    // Diff ordering and sinceTime.
    let (d, more) = diff(&alice, album1, 0).await;
    assert!(!more);
    assert_eq!(d.len(), 2);
    assert_eq!(d[0]["id"].as_i64(), Some(f1_id));
    let t1 = d[0]["updationTime"].as_i64().unwrap();
    let (d, _) = diff(&alice, album1, t1).await;
    assert_eq!(d.len(), 1);
    assert_eq!(d[0]["id"].as_i64(), Some(f2_id));

    // Download URLs.
    let r = alice
        .call("GET", &format!("/files/download/{f1_id}"), None)
        .await;
    assert_eq!(r.status, 307);
    assert!(r.headers.to_lowercase().contains("location:"));
    let v = alice
        .ok("GET", &format!("/files/download/v2/{f1_id}"), None)
        .await;
    let got = http("GET", v["url"].as_str().unwrap(), &[], vec![]).await;
    assert_eq!(got.body, vec![7u8; 11]);
    let r = alice.call("GET", "/files/download/v3/999999", None).await;
    assert_eq!(
        r.status, 400,
        "v3 reports a missing object as 400 NOT_FOUND"
    );
    assert_eq!(r.json()["code"], "NOT_FOUND");
    let v = alice
        .ok(
            "POST",
            "/files/info",
            Some(json!({"fileIDs": [f1_id, f2_id]})),
        )
        .await;
    assert_eq!(v["filesInfo"][1]["fileInfo"]["fileSize"], 11);
    let v = alice
        .ok(
            "POST",
            "/files/size",
            Some(json!({"fileIDs": [f1_id, f2_id]})),
        )
        .await;
    assert_eq!(v["size"], 11 + 11, "file objects only, like museum");

    // Album 2: add f1, move f2 from album 1.
    let album2 = create_album(&alice).await;
    alice
        .ok(
            "POST",
            "/collections/add-files",
            Some(json!({"collectionID": album2, "files": [item(f1_id)]})),
        )
        .await;
    alice
        .ok(
            "POST",
            "/collections/move-files",
            Some(json!({"fromCollectionID": album1, "toCollectionID": album2, "files": [item(f2_id)]})),
        )
        .await;
    let (d, _) = diff(&alice, album2, 0).await;
    assert_eq!(d.len(), 2);
    let (d, _) = diff(&alice, album1, 0).await;
    let moved = find(&d, f2_id).unwrap();
    assert_eq!(moved["isDeleted"], true);
    assert_eq!(moved["metadata"]["encryptedData"], "-");

    // Share album 2 with Bob as collaborator.
    let v = alice
        .ok(
            "POST",
            "/collections/share",
            Some(json!({"collectionID": album2, "email": "bob@example.org",
                "encryptedKey": rb64(80), "role": "COLLABORATOR"})),
        )
        .await;
    assert_eq!(v["sharees"][0]["email"], "bob@example.org");
    assert_eq!(v["sharees"][0]["role"], "COLLABORATOR");
    let v = bob.ok("GET", "/collections/v2?sinceTime=0", None).await;
    let shared = v["collections"].as_array().unwrap();
    assert_eq!(shared.len(), 1);
    assert_eq!(shared[0]["id"].as_i64(), Some(album2));
    assert_eq!(shared[0]["owner"]["id"], 1);
    let v = bob.ok("GET", "/collections/v3?sinceTime=0", None).await;
    assert_eq!(v["owned"].as_array().unwrap().len(), 0);
    assert_eq!(v["shared"].as_array().unwrap().len(), 1);
    let (d, _) = diff(&bob, album2, 0).await;
    assert_eq!(d.len(), 2);
    assert!(d.iter().all(|f| f.get("magicMetadata").is_none()));
    let v = bob
        .ok("GET", &format!("/files/download/v2/{f1_id}"), None)
        .await;
    assert!(v["url"].is_string());
    let v = bob
        .ok(
            "GET",
            &format!("/collections/sharees?collectionID={album2}"),
            None,
        )
        .await;
    assert_eq!(v["sharees"].as_array().unwrap().len(), 1);

    // Bob adds his own file; Alice (owner) removes it from her album.
    let bob_album = create_album(&bob).await;
    let f3 = upload_file(&bob, bob_album, 3).await;
    let f3_id = f3["id"].as_i64().unwrap();
    bob.ok(
        "POST",
        "/collections/add-files",
        Some(json!({"collectionID": album2, "files": [item(f3_id)]})),
    )
    .await;
    let (d, _) = diff(&alice, album2, 0).await;
    assert_eq!(find(&d, f3_id).unwrap()["ownerID"], 2);
    let r = alice
        .call(
            "POST",
            "/collections/v3/remove-files",
            Some(json!({"collectionID": album2, "fileIDs": [f1_id]})),
        )
        .await;
    assert_eq!(r.status, 400, "owner can not remove own files with v3");
    alice
        .ok(
            "POST",
            "/collections/v3/remove-files",
            Some(json!({"collectionID": album2, "fileIDs": [f3_id]})),
        )
        .await;
    let (d, _) = diff(&bob, album2, 0).await;
    assert_eq!(find(&d, f3_id).unwrap()["isDeleted"], true);
    // Bob can not trash Alice's file.
    let r = bob
        .call(
            "POST",
            "/files/trash",
            Some(json!({"items": [{"fileID": f1_id, "collectionID": album2}]})),
        )
        .await;
    assert_eq!(r.status, 403);
    let v = bob
        .ok("GET", "/collection-actions/pending-remove", None)
        .await;
    assert_eq!(v["actions"].as_array().unwrap().len(), 0);
    assert_eq!(v["hasMore"], false);

    // Trash f2 (in album 2) and check both views.
    let (_, _) = diff(&bob, album2, 0).await;
    let r = alice
        .call(
            "POST",
            "/files/trash",
            Some(json!({"items": [{"fileID": f2_id, "collectionID": album2}]})),
        )
        .await;
    assert_eq!((r.status, r.body.len()), (200, 0));
    let t = trash_diff(&alice).await;
    assert_eq!(t.len(), 1);
    assert_eq!(t[0]["file"]["id"].as_i64(), Some(f2_id));
    assert_eq!(t[0]["isDeleted"], false);
    assert_eq!(t[0]["isRestored"], false);
    assert!(t[0]["file"]["collectionOwnerID"].is_null());
    let delete_by = t[0]["deleteBy"].as_i64().unwrap();
    let created = t[0]["createdAt"].as_i64().unwrap();
    assert_eq!(delete_by - created, 30 * 24 * 3600 * 1_000_000);
    let (d, _) = diff(&bob, album2, 0).await;
    assert_eq!(find(&d, f2_id).unwrap()["isDeleted"], true);

    // Restore into album 1.
    alice
        .ok(
            "POST",
            "/collections/restore-files",
            Some(json!({"collectionID": album1, "files": [item(f2_id)]})),
        )
        .await;
    let t = trash_diff(&alice).await;
    assert_eq!(t[0]["isRestored"], true);
    let (d, _) = diff(&alice, album1, 0).await;
    assert_eq!(find(&d, f2_id).unwrap()["isDeleted"], false);

    // Trash both, permanently delete f1, empty the rest.
    alice
        .ok(
            "POST",
            "/files/trash",
            Some(json!({"items": [
                {"fileID": f1_id, "collectionID": album2},
                {"fileID": f2_id, "collectionID": album1}]})),
        )
        .await;
    let usage_before = usage(&srv, 1).await;
    assert_eq!(usage_before, 31);
    alice
        .ok("POST", "/trash/delete", Some(json!({"fileIDs": [f1_id]})))
        .await;
    assert_eq!(usage(&srv, 1).await, 15);
    let t = trash_diff(&alice).await;
    let last = t
        .iter()
        .map(|e| e["updatedAt"].as_i64().unwrap())
        .max()
        .unwrap();
    assert_eq!(
        t.iter().find(|e| e["file"]["id"] == f1_id).unwrap()["isDeleted"],
        true
    );
    alice
        .ok("POST", "/trash/empty", Some(json!({"lastUpdatedAt": last})))
        .await;
    let mut done = false;
    for _ in 0..100 {
        let t = trash_diff(&alice).await;
        if t.iter().all(|e| e["isDeleted"] == true) {
            done = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(done, "empty trash processed");
    assert_eq!(usage(&srv, 1).await, 0);
    let key = mp["objectKey"].as_str().unwrap().to_string();
    assert!(srv.state.store.head(&key).await.unwrap().is_some());
    photovault_server::library::tasks::run_once(&srv.state).await;
    assert!(srv.state.store.head(&key).await.unwrap().is_none());

    // Deleting a collection: Bob's album with his file in it, files go to trash.
    let r = bob
        .call(
            "DELETE",
            &format!("/collections/v3/{bob_album}?collectionID={bob_album}&keepFiles=false"),
            None,
        )
        .await;
    assert_eq!(r.status, 200);
    photovault_server::library::tasks::run_once(&srv.state).await;
    let t = trash_diff(&bob).await;
    assert_eq!(t.len(), 1);
    assert_eq!(t[0]["file"]["id"].as_i64(), Some(f3_id));
    let v = bob.ok("GET", "/collections/v2?sinceTime=0", None).await;
    let own = v["collections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"].as_i64() == Some(bob_album))
        .cloned()
        .unwrap();
    assert_eq!(own["isDeleted"], true);
}

async fn usage(srv: &TestServer, uid: i64) -> i64 {
    let r: Option<(i64,)> = sqlx::query_as("SELECT storage_consumed FROM usage WHERE user_id = $1")
        .bind(uid)
        .fetch_optional(&srv.state.db.pool)
        .await
        .unwrap();
    r.map(|r| r.0).unwrap_or(0)
}

/// The diff never splits one `updationTime` across pages, and returns the whole group when
/// the group alone is larger than the page.
#[tokio::test]
async fn diff_paging_edges() {
    let srv = TestServer::start().await.unwrap();
    let alice = add_user(&srv, 1, "alice@example.org").await;
    let album = create_album(&alice).await;

    // 2499 rows with distinct versions 1..=2499, then 3 rows sharing version 5000.
    let mut versions: Vec<i64> = (1..=2499).collect();
    versions.extend([5000, 5000, 5000]);
    insert_rows(&srv, album, 1, &versions).await;
    let (d, more) = diff(&alice, album, 0).await;
    assert!(more);
    assert_eq!(d.len(), 2499);
    let (d, more) = diff(&alice, album, 2499).await;
    assert!(!more);
    assert_eq!(d.len(), 3);

    // A second album whose 2600 rows all share one version.
    let album2 = create_album(&alice).await;
    insert_rows(&srv, album2, 10_000, &[7000; 2600]).await;
    let (d, more) = diff(&alice, album2, 0).await;
    assert!(more);
    assert_eq!(d.len(), 2600);
    let (d, more) = diff(&alice, album2, 7000).await;
    assert!(!more);
    assert!(d.is_empty());

    // Exactly `limit` rows: no more pages.
    let album3 = create_album(&alice).await;
    let v: Vec<i64> = (1..=2500).collect();
    insert_rows(&srv, album3, 20_000, &v).await;
    let (d, more) = diff(&alice, album3, 0).await;
    assert!(!more);
    assert_eq!(d.len(), 2500);
}

async fn insert_rows(srv: &TestServer, cid: i64, first_id: i64, versions: &[i64]) {
    let mut tx = srv.state.db.pool.begin().await.unwrap();
    for (chunk_no, chunk) in versions.chunks(200).enumerate() {
        let base = first_id + (chunk_no * 200) as i64;
        let mut files = Vec::new();
        let mut cfs = Vec::new();
        for (i, v) in chunk.iter().enumerate() {
            let id = base + i as i64;
            files.push(format!("({id}, 1, 'photos', 'm', 'h', 'h', 'h', {v})"));
            cfs.push(format!("({cid}, {id}, 'k', 'n', FALSE, {v}, 1, 1, {v})"));
        }
        sqlx::query(&format!(
            "INSERT INTO files (file_id, owner_id, app, encrypted_metadata, file_decryption_header,
                thumbnail_decryption_header, metadata_decryption_header, updation_time) VALUES {}",
            files.join(", ")
        ))
        .execute(&mut *tx)
        .await
        .unwrap();
        sqlx::query(&format!(
            "INSERT INTO collection_files (collection_id, file_id, encrypted_key, key_decryption_nonce,
                is_deleted, updation_time, c_owner_id, f_owner_id, created_at) VALUES {}",
            cfs.join(", ")
        ))
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
}

/// File data, public album links, file links and collection metadata endpoints.
#[tokio::test]
async fn file_data_and_links() {
    let srv = TestServer::start().await.unwrap();
    let alice = add_user(&srv, 1, "alice@example.org").await;
    let album = create_album(&alice).await;
    let f = upload_file(&alice, album, 9).await;
    let fid = f["id"].as_i64().unwrap();

    // ML data round trip and status diff.
    let v = alice
        .ok(
            "POST",
            "/files/data/status-diff",
            Some(json!({"lastUpdatedAt": 0})),
        )
        .await;
    assert!(v["diff"].is_null());
    let r = alice
        .call(
            "PUT",
            "/files/data",
            Some(json!({"fileID": fid, "type": "mldata", "encryptedData": "abc", "decryptionHeader": "hdr"})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let v = alice
        .ok(
            "POST",
            "/files/data/fetch",
            Some(json!({"fileIDs": [fid, fid + 100], "type": "mldata"})),
        )
        .await;
    assert_eq!(v["data"][0]["encryptedData"], "abc");
    assert_eq!(v["pendingIndexFileIDs"], json!([fid + 100]));
    let v = alice
        .ok(
            "GET",
            &format!("/files/data/fetch?fileID={fid}&type=mldata"),
            None,
        )
        .await;
    assert_eq!(v["data"]["decryptionHeader"], "hdr");
    let updated = v["data"]["updatedAt"].as_i64().unwrap();
    let r = alice
        .call(
            "PUT",
            "/files/data",
            Some(
                json!({"fileID": fid, "type": "mldata", "encryptedData": "x",
                "decryptionHeader": "y", "lastUpdatedAt": updated - 1}),
            ),
        )
        .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["code"], "INVALID_VERSION");
    let v = alice
        .ok(
            "POST",
            "/files/data/status-diff",
            Some(json!({"lastUpdatedAt": 0})),
        )
        .await;
    assert_eq!(v["diff"][0]["type"], "mldata");
    let r = alice
        .call(
            "GET",
            &format!("/files/data/fetch?fileID={fid}&type=vid_preview&preferNoContent=true"),
            None,
        )
        .await;
    assert_eq!(r.status, 204);

    // Video preview: upload URL, upload, register, read back.
    let v = alice
        .ok(
            "GET",
            &format!("/files/data/preview-upload-url?fileID={fid}&type=vid_preview"),
            None,
        )
        .await;
    let obj = v["objectID"].as_str().unwrap().to_string();
    assert!(obj.starts_with("pv_"));
    put_object(v["url"].as_str().unwrap(), b"preview").await;
    alice
        .ok(
            "PUT",
            "/files/video-data",
            Some(json!({"fileID": fid, "objectID": obj, "objectSize": 7,
                "playlist": "pl", "playlistHeader": "ph"})),
        )
        .await;
    let v = alice
        .ok(
            "GET",
            &format!("/files/data/preview?fileID={fid}&type=vid_preview"),
            None,
        )
        .await;
    let got = http("GET", v["url"].as_str().unwrap(), &[], vec![]).await;
    assert_eq!(got.body, b"preview");

    // Public album link: create, update, list via the collection, disable.
    let v = alice
        .ok(
            "POST",
            "/collections/share-url",
            Some(json!({"collectionID": album})),
        )
        .await;
    assert!(v["result"]["url"].is_string());
    assert_eq!(v["result"]["enableJoin"], true);
    let v = alice
        .ok(
            "PUT",
            "/collections/share-url",
            Some(json!({"collectionID": album, "enableDownload": false})),
        )
        .await;
    assert_eq!(v["result"]["enableDownload"], false);
    let v = alice
        .ok("GET", &format!("/collections/{album}"), None)
        .await;
    assert_eq!(v["collection"]["publicURLs"].as_array().unwrap().len(), 1);
    let r = alice
        .call("DELETE", &format!("/collections/share-url/{album}"), None)
        .await;
    assert_eq!(r.status, 200);
    let v = alice
        .ok("GET", &format!("/collections/{album}"), None)
        .await;
    assert_eq!(v["collection"]["publicURLs"].as_array().unwrap().len(), 0);

    // File link.
    let v = alice
        .ok(
            "POST",
            "/files/share-url",
            Some(json!({"fileID": fid, "app": "photos"})),
        )
        .await;
    assert!(v["linkID"].as_str().unwrap().starts_with("pft_"));
    let v = alice
        .ok("GET", "/files/share-urls/?sinceTime=0", None)
        .await;
    assert_eq!(v["diff"].as_array().unwrap().len(), 1);
    let r = alice
        .call("GET", "/files/share-url?sinceTime=x", None)
        .await;
    assert_eq!(r.status, 400);
    alice
        .ok("DELETE", &format!("/files/share-url/{fid}"), None)
        .await;

    // Rename and magic metadata (versions are checked and bumped).
    alice
        .ok(
            "POST",
            "/collections/rename",
            Some(json!({"collectionID": album, "encryptedName": "bmV3",
                "nameDecryptionNonce": rb64(24)})),
        )
        .await;
    let mm = json!({"version": 1, "count": 1, "data": "ZA==", "header": "aA=="});
    alice
        .ok(
            "PUT",
            "/collections/magic-metadata",
            Some(json!({"id": album, "magicMetadata": mm})),
        )
        .await;
    let v = alice
        .ok("GET", &format!("/collections/{album}"), None)
        .await;
    assert_eq!(v["collection"]["magicMetadata"]["version"], 2);
    alice
        .ok(
            "PUT",
            "/files/magic-metadata",
            Some(json!({"metadataList": [{"id": fid, "magicMetadata": mm}]})),
        )
        .await;
    let (d, _) = diff(&alice, album, 0).await;
    assert_eq!(d[0]["magicMetadata"]["version"], 2);
    let r = alice
        .call(
            "PUT",
            "/files/magic-metadata",
            Some(json!({"metadataList": [{"id": fid, "magicMetadata":
                {"version": 5, "count": 1, "data": "ZA==", "header": "aA=="}}]})),
        )
        .await;
    assert_eq!(r.status, 409, "file metadata versions are checked");
}
