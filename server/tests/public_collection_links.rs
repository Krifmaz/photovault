//! /public-collection/*: link auth (expiry, disabled, password JWT, device limit),
//! info, diff, downloads, file data, and public "collect" uploads.

mod support;

use serde_json::json;
use support::*;

#[tokio::test]
async fn info_diff_and_downloads() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (owner, _) = user(db, 1 << 30).await;
    let cid = collection(db, owner).await;
    let f1 = file(&srv, owner, cid, 2000).await;
    let f2 = file(&srv, owner, cid, 3000).await;
    // A removed entry is scrubbed in the diff.
    sqlx::query("UPDATE collection_files SET is_deleted = TRUE WHERE file_id = $1")
        .bind(f2)
        .execute(db)
        .await
        .unwrap();
    let (_, token) = album_link(db, cid, &[]).await;
    let h = [("X-Auth-Access-Token", token.as_str())];

    // Missing / unknown tokens.
    let r = get(&format!("{}/public-collection/info", srv.url), &[]).await;
    assert_eq!(r.status, 401);
    assert_eq!(
        r.json(),
        json!({"error": "missing accessToken", "context": "album_link"})
    );
    let r = get(
        &format!("{}/public-collection/info", srv.url),
        &[("X-Auth-Access-Token", "NOPE")],
    )
    .await;
    assert_eq!(
        (r.status, r.json()),
        (401, json!({"error": "invalid token"}))
    );

    let r = get(&format!("{}/public-collection/info", srv.url), &h).await;
    assert_eq!(r.status, 200);
    assert!(r.headers.contains_key("x-link-device-token"));
    let v = r.json();
    assert_eq!(v["referralCode"], "");
    let c = &v["collection"];
    assert_eq!(c["id"], cid);
    assert_eq!(
        c["owner"],
        json!({"id": owner, "email": "", "name": "", "role": ""})
    );
    assert_eq!(c["encryptedName"], "encname");
    assert_eq!(c["sharees"], serde_json::Value::Null);
    assert!(c.get("magicMetadata").is_none());
    assert_eq!(c["pubMagicMetadata"]["data"], "pub");
    assert_eq!(c["attributes"], json!({"version": 0}));
    assert_eq!(
        c["publicURLs"],
        json!([{"url": "", "deviceLimit": 0, "validTill": 0, "enableDownload": true,
                "enableCollect": false, "enableComment": false, "passwordEnabled": false,
                "enableJoin": true}])
    );

    // Query-string token works too.
    let r = get(
        &format!(
            "{}/public-collection/diff?sinceTime=0&accessToken={token}",
            srv.url
        ),
        &[],
    )
    .await;
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["hasMore"], false);
    let diff = v["diff"].as_array().unwrap();
    assert_eq!(diff.len(), 2);
    assert_eq!(diff[0]["id"], f1);
    assert_eq!(diff[0]["encryptedKey"], format!("ek{f1}"));
    assert_eq!(
        diff[0]["metadata"],
        json!({"encryptedData": "meta", "decryptionHeader": "mh", "size": 0})
    );
    assert_eq!(diff[0]["pubMagicMetadata"]["data"], "pubdata");
    assert!(diff[0].get("magicMetadata").is_none());
    assert_eq!(diff[0]["info"], json!({"fileSize": 5, "thumbSize": 3}));
    assert_eq!(diff[0]["collectionOwnerID"], owner);
    assert_eq!(diff[1]["isDeleted"], true);
    assert_eq!(diff[1]["metadata"]["encryptedData"], "-");
    assert_eq!(diff[1]["encryptedKey"], "");
    let r = get(
        &format!("{}/public-collection/diff?sinceTime=2000", srv.url),
        &h,
    )
    .await;
    assert_eq!(r.json()["diff"].as_array().unwrap().len(), 1);
    let r = get(
        &format!("{}/public-collection/diff?sinceTime=x", srv.url),
        &h,
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "BAD_REQUEST");

    // v3 URL then fetch; legacy redirect.
    let r = get(
        &format!("{}/public-collection/files/download/v3/{f1}", srv.url),
        &h,
    )
    .await;
    assert_eq!(r.status, 200);
    let url = r.json()["url"].as_str().unwrap().to_string();
    assert_eq!(get(&url, &[]).await.body, b"hello");
    let r = get(
        &format!("{}/public-collection/files/thumbnail/v3/{f1}", srv.url),
        &h,
    )
    .await;
    let url = r.json()["url"].as_str().unwrap().to_string();
    assert_eq!(get(&url, &[]).await.body, b"thm");
    let r = get(
        &format!("{}/public-collection/files/preview/{f1}", srv.url),
        &h,
    )
    .await;
    assert_eq!(r.status, 307);
    assert_eq!(get(&r.headers["location"], &[]).await.body, b"thm");
    // Files outside the album (or removed) are refused.
    let r = get(
        &format!("{}/public-collection/files/download/v3/{f2}", srv.url),
        &h,
    )
    .await;
    assert_eq!((r.status, r.json()), (403, json!({})));
    let r = get(
        &format!("{}/public-collection/files/download/abc", srv.url),
        &h,
    )
    .await;
    assert_eq!(r.status, 400);
    // A member file with no object: v3 says 400 NOT_FOUND.
    sqlx::query("UPDATE object_keys SET is_deleted = TRUE WHERE file_id = $1")
        .bind(f1)
        .execute(db)
        .await
        .unwrap();
    let r = get(
        &format!("{}/public-collection/files/download/v3/{f1}", srv.url),
        &h,
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "NOT_FOUND");
    let r = get(
        &format!("{}/public-collection/files/download/{f1}", srv.url),
        &h,
    )
    .await;
    assert_eq!(r.status, 404);
}

#[tokio::test]
async fn link_states_password_and_device_limit() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (owner, _) = user(db, 1 << 30).await;

    // Disabled and expired links.
    let cid = collection(db, owner).await;
    let (_, disabled) = album_link(db, cid, &[("is_disabled", "TRUE")]).await;
    let r = get(
        &format!("{}/public-collection/info", srv.url),
        &[("X-Auth-Access-Token", disabled.as_str())],
    )
    .await;
    assert_eq!(r.status, 410);
    assert_eq!(
        r.json(),
        json!({"code": "LINK_DISABLED", "error": "disabled token"})
    );
    let cid2 = collection(db, owner).await;
    let (_, expired) = album_link(db, cid2, &[("valid_till", "1")]).await;
    let r = get(
        &format!("{}/public-collection/info", srv.url),
        &[("X-Auth-Access-Token", expired.as_str())],
    )
    .await;
    assert_eq!(
        (r.status, r.json()["code"].clone()),
        (410, json!("LINK_EXPIRED"))
    );

    // Password-protected link: info is allowed, diff needs the JWT.
    let cid3 = collection(db, owner).await;
    file(&srv, owner, cid3, 2000).await;
    let (_, pw) = album_link(
        db,
        cid3,
        &[
            ("pw_hash", "'hash123'"),
            ("pw_nonce", "'nonce'"),
            ("mem_limit", "64"),
            ("ops_limit", "2"),
        ],
    )
    .await;
    let h = [("X-Auth-Access-Token", pw.as_str())];
    let r = get(&format!("{}/public-collection/info", srv.url), &h).await;
    assert_eq!(r.status, 200);
    let u = &r.json()["collection"]["publicURLs"][0];
    assert_eq!(u["passwordEnabled"], true);
    assert_eq!(u["nonce"], "nonce");
    assert_eq!(u["memLimit"], 64);
    assert_eq!(u["opsLimit"], 2);
    let r = get(
        &format!("{}/public-collection/diff?sinceTime=0", srv.url),
        &h,
    )
    .await;
    assert_eq!((r.status, r.json()), (401, json!({"error": {}})));
    let r = post_json(
        &format!("{}/public-collection/verify-password", srv.url),
        &h,
        json!({"passHash": "wrong"}),
    )
    .await;
    assert_eq!(r.status, 401);
    let r = post_json(
        &format!("{}/public-collection/verify-password", srv.url),
        &h,
        json!({}),
    )
    .await;
    assert_eq!(r.status, 400);
    let r = post_json(
        &format!("{}/public-collection/verify-password", srv.url),
        &h,
        json!({"passHash": "hash123"}),
    )
    .await;
    assert_eq!(r.status, 200);
    let jwt = r.json()["jwtToken"].as_str().unwrap().to_string();
    let hj = [
        ("X-Auth-Access-Token", pw.as_str()),
        ("X-Auth-Access-Token-JWT", jwt.as_str()),
    ];
    let r = get(
        &format!("{}/public-collection/diff?sinceTime=0", srv.url),
        &hj,
    )
    .await;
    assert_eq!(r.status, 200);
    let r = get(
        &format!(
            "{}/public-collection/diff?sinceTime=0&accessTokenJWT=garbage",
            srv.url
        ),
        &h,
    )
    .await;
    assert_eq!(r.status, 401);
    // Changing the password invalidates old JWTs.
    sqlx::query("UPDATE public_collection_tokens SET pw_hash = 'other' WHERE access_token = $1")
        .bind(&pw)
        .execute(db)
        .await
        .unwrap();
    let r = get(
        &format!("{}/public-collection/diff?sinceTime=0", srv.url),
        &hj,
    )
    .await;
    assert_eq!(r.status, 401);
    // A link without a password refuses verify-password.
    let cid4 = collection(db, owner).await;
    let (_, open) = album_link(db, cid4, &[("device_limit", "2")]).await;
    let r = post_json(
        &format!("{}/public-collection/verify-password", srv.url),
        &[("X-Auth-Access-Token", open.as_str())],
        json!({"passHash": "x"}),
    )
    .await;
    assert_eq!(r.status, 400);

    // Device limit 2: two devices admitted, a third refused, known ones (or a valid
    // device token) still let in.
    let info = |ua: &'static str, dt: Option<String>| {
        let url = format!("{}/public-collection/info", srv.url);
        let open = open.clone();
        async move {
            let mut h = vec![("X-Auth-Access-Token", open.as_str()), ("User-Agent", ua)];
            if let Some(t) = &dt {
                h.push(("X-Auth-Link-Device-Token", t.as_str()));
            }
            get(&url, &h).await
        }
    };
    let r1 = info("dev-a", None).await;
    assert_eq!(r1.status, 200);
    let token_a = r1.headers["x-link-device-token"].clone();
    assert_eq!(info("dev-b", None).await.status, 200);
    let r = info("dev-c", None).await;
    assert_eq!(r.status, 403);
    assert_eq!(
        r.json(),
        json!({"code": "LINK_DEVICE_LIMIT_EXCEEDED", "message": "Public link device limit reached"})
    );
    assert_eq!(info("dev-a", None).await.status, 200);
    // Device token from dev-a lets a "new" device through without counting it,
    // and a fresh token isn't needed (no header back).
    let r = info("dev-d", Some(token_a)).await;
    assert_eq!(r.status, 200);
    assert!(!r.headers.contains_key("x-link-device-token"));
    // Downloads don't do device admission.
    let r = get(
        &format!("{}/public-collection/files/download/v3/1", srv.url),
        &[
            ("X-Auth-Access-Token", open.as_str()),
            ("User-Agent", "dev-z"),
        ],
    )
    .await;
    assert_eq!(r.status, 403);
}

#[tokio::test]
async fn collect_uploads_into_owner_album() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (owner, _) = user(db, 100).await;
    let cid = collection(db, owner).await;
    let (_, no_collect) = album_link(db, cid, &[]).await;
    let r = post_json(
        &format!("{}/public-collection/upload-url", srv.url),
        &[("X-Auth-Access-Token", no_collect.as_str())],
        json!({"contentLength": 5, "contentMD5": "XUFAKrxLKna5cZ2REBfFkg=="}),
    )
    .await;
    assert_eq!(r.status, 405);
    assert_eq!(r.json()["code"], "PUBLIC_COLLECT_DISABLED");

    sqlx::query(
        "UPDATE public_collection_tokens SET enable_collect = TRUE WHERE access_token = $1",
    )
    .bind(&no_collect)
    .execute(db)
    .await
    .unwrap();
    let h = [("X-Auth-Access-Token", no_collect.as_str())];
    // Bad bodies.
    let r = post_json(
        &format!("{}/public-collection/upload-url", srv.url),
        &h,
        json!({}),
    )
    .await;
    assert_eq!((r.status, r.json()), (400, json!({})));
    let r = post_json(
        &format!("{}/public-collection/upload-url", srv.url),
        &h,
        json!({"contentLength": 5, "contentMD5": "bm90LW1kNQ=="}),
    )
    .await;
    assert_eq!(r.status, 400);
    // Over the owner's quota (100 bytes).
    let r = post_json(
        &format!("{}/public-collection/upload-url", srv.url),
        &h,
        json!({"contentLength": 500, "contentMD5": "XUFAKrxLKna5cZ2REBfFkg=="}),
    )
    .await;
    assert_eq!(r.status, 426);

    let mut keys = Vec::new();
    for body in [&b"hello"[..], &b"thm"[..]] {
        // Any 16-byte base64 digest passes validation; the local store doesn't verify it.
        let md5 = "XUFAKrxLKna5cZ2REBfFkg==".to_string();
        let r = post_json(
            &format!("{}/public-collection/upload-url", srv.url),
            &h,
            json!({"contentLength": body.len(), "contentMD5": md5}),
        )
        .await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        let v = r.json();
        let key = v["objectKey"].as_str().unwrap().to_string();
        assert!(key.starts_with(&format!("{owner}/")));
        let put = http(
            "PUT",
            v["url"].as_str().unwrap(),
            &[("Content-MD5", md5.as_str())],
            body,
        )
        .await;
        assert_eq!(put.status, 200);
        keys.push(key);
    }

    // Multipart needs part checksums.
    let r = post_json(
        &format!("{}/public-collection/multipart-upload-url", srv.url),
        &h,
        json!({"contentLength": 10, "partLength": 10}),
    )
    .await;
    assert_eq!(r.status, 400);
    let r = post_json(
        &format!("{}/public-collection/multipart-upload-url", srv.url),
        &h,
        json!({"contentLength": 10, "partLength": 10, "partMd5s": ["XUFAKrxLKna5cZ2REBfFkg=="]}),
    )
    .await;
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["partURLs"].as_array().unwrap().len(), 1);
    assert!(v["completeURL"].is_string());

    let new_file = json!({
        "collectionID": cid,
        "encryptedKey": "ek", "keyDecryptionNonce": "kn",
        "file": {"objectKey": keys[0], "decryptionHeader": "fh"},
        "thumbnail": {"objectKey": keys[1], "decryptionHeader": "th"},
        "metadata": {"encryptedData": "md", "decryptionHeader": "mh"},
        "pubMagicMetadata": {"version": 1, "count": 1, "data": "d", "header": "h"},
        "updationTime": 1,
    });
    let mut wrong = new_file.clone();
    wrong["collectionID"] = json!(cid + 100);
    let r = post_json(&format!("{}/public-collection/file", srv.url), &h, wrong).await;
    assert_eq!(r.status, 400);
    assert_eq!(
        r.json()["message"],
        "can only update to associated collection"
    );
    let mut foreign = new_file.clone();
    foreign["file"]["objectKey"] = json!("999/x");
    let r = post_json(&format!("{}/public-collection/file", srv.url), &h, foreign).await;
    assert_eq!(r.status, 400);

    let r = post_json(
        &format!("{}/public-collection/file", srv.url),
        &h,
        new_file.clone(),
    )
    .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let f = r.json();
    assert_eq!(f["ownerID"], owner);
    assert_eq!(f["collectionID"], cid);
    assert_eq!(f["file"]["size"], 5);
    assert_eq!(f["thumbnail"]["size"], 3);
    assert_eq!(f["info"], json!({"fileSize": 5, "thumbSize": 3}));
    let fid = f["id"].as_i64().unwrap();
    // Charged to the owner.
    let (used,): (i64,) = sqlx::query_as("SELECT storage_consumed FROM usage WHERE user_id = $1")
        .bind(owner)
        .fetch_one(db)
        .await
        .unwrap();
    assert_eq!(used, 8);
    // Reusing the objects is refused.
    let r = post_json(&format!("{}/public-collection/file", srv.url), &h, new_file).await;
    assert_eq!(r.status, 400);
    // Visible through the link.
    let r = get(
        &format!("{}/public-collection/diff?sinceTime=0", srv.url),
        &h,
    )
    .await;
    let diff = r.json()["diff"].as_array().unwrap().clone();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0]["id"], fid);
    let r = get(
        &format!("{}/public-collection/files/download/v3/{fid}", srv.url),
        &h,
    )
    .await;
    let url = r.json()["url"].as_str().unwrap().to_string();
    assert_eq!(get(&url, &[]).await.body, b"hello");
}

#[tokio::test]
async fn file_data_and_previews() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (owner, _) = user(db, 1 << 30).await;
    let cid = collection(db, owner).await;
    let fid = file(&srv, owner, cid, 2000).await;
    let other_cid = collection(db, owner).await;
    let outside = file(&srv, owner, other_cid, 2000).await;
    let (_, token) = album_link(db, cid, &[]).await;
    let h = [("X-Auth-Access-Token", token.as_str())];
    let base = format!("{}/public-collection/files/data", srv.url);

    let r = get(&format!("{base}/fetch?type=vid_preview"), &h).await;
    assert_eq!(
        (r.status, r.json()["code"].clone()),
        (400, json!("BAD_REQUEST"))
    );
    let r = get(&format!("{base}/fetch?fileID={fid}&type=mldata"), &h).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["message"], "unsupported object type mldata");
    let r = get(&format!("{base}/fetch?fileID={fid}&type=vid_preview"), &h).await;
    assert_eq!(
        (r.status, r.json()["code"].clone()),
        (404, json!("NOT_FOUND"))
    );
    let r = get(
        &format!("{base}/fetch?fileID={fid}&type=vid_preview&preferNoContent=true"),
        &h,
    )
    .await;
    assert_eq!(r.status, 204);
    let r = get(
        &format!("{base}/fetch?fileID={outside}&type=vid_preview"),
        &h,
    )
    .await;
    assert_eq!(r.status, 403);

    sqlx::query(
        "INSERT INTO file_data (file_id, user_id, data_type, size, obj_id, updated_at) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(fid)
    .bind(owner)
    .bind("vid_preview")
    .bind(10i64)
    .bind("pv_1")
    .bind(4242i64)
    .execute(db)
    .await
    .unwrap();
    let prefix = format!("{owner}/file-data/{fid}/vid_preview/pv_1");
    srv.state
        .store
        .put_bytes(
            &format!("{prefix}_playlist"),
            bytes::Bytes::from(r#"{"v":1,"encryptedData":"ENC","header":"HDR","client":"web"}"#),
        )
        .await
        .unwrap();
    srv.state
        .store
        .put_bytes(&prefix, bytes::Bytes::from_static(b"video"))
        .await
        .unwrap();
    let r = get(&format!("{base}/fetch?fileID={fid}&type=vid_preview"), &h).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.json(),
        json!({"data": {"fileID": fid, "type": "vid_preview", "encryptedData": "ENC",
                        "decryptionHeader": "HDR", "updatedAt": 4242}})
    );
    let r = get(&format!("{base}/preview?fileID={fid}&type=vid_preview"), &h).await;
    assert_eq!(r.status, 200);
    let url = r.json()["url"].as_str().unwrap().to_string();
    assert_eq!(get(&url, &[]).await.body, b"video");
    let r = get(&format!("{base}/preview?fileID={fid}&type=img_preview"), &h).await;
    assert_eq!(r.status, 404);
    let r = get(&format!("{base}/preview?fileID={fid}"), &h).await;
    assert_eq!(r.status, 400);
}
