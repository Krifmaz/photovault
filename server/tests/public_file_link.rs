//! /file-link/*: single-file public links.

mod support;

use photovault_server::db::now_micros;
use serde_json::json;
use support::*;

async fn file_link(
    srv: &photovault_server::testing::TestServer,
    file_id: i64,
    owner: i64,
    sets: &str,
) -> String {
    let token = format!(
        "F{}",
        hex::encode(photovault_server::crypto::random_bytes(5)).to_uppercase()
    );
    sqlx::query(
        "INSERT INTO public_file_tokens (id, app, file_id, owner_id, access_token, created_at, updated_at, \
         encrypted_file_key, encrypted_file_key_nonce, kdf_nonce, kdf_mem_limit, kdf_ops_limit) \
         VALUES ($1, 'photos', $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(format!("fl_{token}"))
    .bind(file_id)
    .bind(owner)
    .bind(&token)
    .bind(now_micros())
    .bind(now_micros())
    .bind("efk")
    .bind("efkn")
    .bind("kdfn")
    .bind(64i64)
    .bind(2i64)
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    if !sets.is_empty() {
        sqlx::query(&format!(
            "UPDATE public_file_tokens SET {sets} WHERE access_token = $1"
        ))
        .bind(&token)
        .execute(&srv.state.db.pool)
        .await
        .unwrap();
    }
    token
}

#[tokio::test]
async fn file_link_flow() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (owner, _) = user(db, 1 << 30).await;
    let cid = collection(db, owner).await;
    let fid = file(&srv, owner, cid, 2000).await;
    let token = file_link(&srv, fid, owner, "device_limit = 1").await;
    let h = [
        ("X-Auth-Access-Token", token.as_str()),
        ("User-Agent", "ua-1"),
    ];

    let r = get(&format!("{}/file-link/info", srv.url), &[]).await;
    assert_eq!(r.status, 401);
    assert_eq!(r.json()["context"], "file_link");

    let r = get(&format!("{}/file-link/info", srv.url), &h).await;
    assert_eq!(r.status, 200);
    assert!(r.headers.contains_key("x-link-device-token"));
    let v = r.json();
    assert_eq!(
        v["link"],
        json!({"encryptedFileKey": "efk", "encryptedFileKeyNonce": "efkn", "kdfNonce": "kdfn",
               "kdfMemLimit": 64, "kdfOpsLimit": 2})
    );
    assert_eq!(v["file"]["id"], fid);
    assert_eq!(v["file"]["ownerID"], owner);
    assert_eq!(v["file"]["collectionID"], 0);
    assert_eq!(v["file"]["metadata"]["encryptedData"], "meta");
    assert_eq!(v["file"]["pubMagicMetadata"]["data"], "pubdata");
    assert!(v["file"].get("magicMetadata").is_none());

    // Device limit 1: a second device is refused on /info only.
    let h2 = [
        ("X-Auth-Access-Token", token.as_str()),
        ("User-Agent", "ua-2"),
    ];
    let r = get(&format!("{}/file-link/info", srv.url), &h2).await;
    assert_eq!(
        (r.status, r.json()["code"].clone()),
        (403, json!("LINK_DEVICE_LIMIT_EXCEEDED"))
    );
    let r = get(&format!("{}/file-link/file/v3", srv.url), &h2).await;
    assert_eq!(r.status, 200);
    let url = r.json()["url"].as_str().unwrap().to_string();
    assert_eq!(get(&url, &[]).await.body, b"hello");
    let r = get(&format!("{}/file-link/thumbnail/v3", srv.url), &h).await;
    let url = r.json()["url"].as_str().unwrap().to_string();
    assert_eq!(get(&url, &[]).await.body, b"thm");
    let r = get(&format!("{}/file-link/thumbnail", srv.url), &h).await;
    assert_eq!(r.status, 307);
    let r = get(&format!("{}/file-link/file", srv.url), &h).await;
    assert_eq!(r.status, 307);
    assert_eq!(get(&r.headers["location"], &[]).await.body, b"hello");

    let r = get(&format!("{}/file-link/pass-info", srv.url), &h).await;
    assert_eq!(
        r.json(),
        json!({"nonce": null, "opsLimit": null, "memLimit": null})
    );
    let r = post_json(
        &format!("{}/file-link/verify-password", srv.url),
        &h,
        json!({"passHash": "x"}),
    )
    .await;
    assert_eq!(r.status, 400);

    // Object gone → 400 NOT_FOUND on v3.
    sqlx::query("UPDATE object_keys SET is_deleted = TRUE WHERE file_id = $1 AND o_type = 'file'")
        .bind(fid)
        .execute(db)
        .await
        .unwrap();
    let r = get(&format!("{}/file-link/file/v3", srv.url), &h).await;
    assert_eq!(
        (r.status, r.json()["code"].clone()),
        (400, json!("NOT_FOUND"))
    );

    // Disabled link.
    sqlx::query("UPDATE public_file_tokens SET is_disabled = TRUE WHERE access_token = $1")
        .bind(&token)
        .execute(db)
        .await
        .unwrap();
    let r = get(&format!("{}/file-link/info", srv.url), &h).await;
    assert_eq!(r.status, 410);
    assert_eq!(
        r.json(),
        json!({"code": "LINK_DISABLED", "error": "disabled token"})
    );
}

#[tokio::test]
async fn password_and_expiry() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (owner, _) = user(db, 1 << 30).await;
    let cid = collection(db, owner).await;
    let fid = file(&srv, owner, cid, 2000).await;
    let token = file_link(
        &srv,
        fid,
        owner,
        "pw_hash = 'ph', pw_nonce = 'pn', mem_limit = 7, ops_limit = 3",
    )
    .await;
    let h = [("X-Auth-Access-Token", token.as_str())];
    let r = get(&format!("{}/file-link/info", srv.url), &h).await;
    assert_eq!(r.status, 401);
    assert_eq!(
        r.json(),
        json!({"error": {"code": "PASS_PROTECTED_RESOURCE", "message": "This resource is password protected"}})
    );
    let r = get(&format!("{}/file-link/pass-info", srv.url), &h).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.json(),
        json!({"nonce": "pn", "opsLimit": 3, "memLimit": 7})
    );
    let r = post_json(
        &format!("{}/file-link/verify-password", srv.url),
        &h,
        json!({"passHash": "bad"}),
    )
    .await;
    assert_eq!(r.status, 401);
    let r = post_json(
        &format!("{}/file-link/verify-password", srv.url),
        &h,
        json!({"passHash": "ph"}),
    )
    .await;
    assert_eq!(r.status, 200);
    let jwt = r.json()["jwtToken"].as_str().unwrap().to_string();
    let hj = [
        ("X-Auth-Access-Token", token.as_str()),
        ("X-Auth-Access-Token-JWT", jwt.as_str()),
    ];
    assert_eq!(
        get(&format!("{}/file-link/info", srv.url), &hj)
            .await
            .status,
        200
    );
    let r = get(
        &format!("{}/file-link/info?accessTokenJWT=bogus", srv.url),
        &h,
    )
    .await;
    assert_eq!((r.status, r.json()), (401, json!({"error": {}})));

    let fid2 = file(&srv, owner, cid, 2001).await;
    let expired = file_link(&srv, fid2, owner, "valid_till = 5").await;
    let r = get(
        &format!("{}/file-link/info", srv.url),
        &[("X-Auth-Access-Token", expired.as_str())],
    )
    .await;
    assert_eq!(r.status, 410);
    assert_eq!(r.json()["code"], "LINK_EXPIRED");
}
