//! /memory-share (owner) and /public-memory/* (viewers).

mod support;

use photovault_server::db::now_micros;
use serde_json::json;
use support::*;

fn auth(token: &str) -> [(&str, &str); 1] {
    [("X-Auth-Token", token)]
}

#[tokio::test]
async fn memory_share_lifecycle() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (alice, at) = user(db, 1 << 30).await;
    let (bob, bt) = user(db, 1 << 30).await;
    let ca = collection(db, alice).await;
    let f1 = file(&srv, alice, ca, 2000).await;
    let f2 = file(&srv, alice, ca, 2001).await;
    // Bob's file, in Bob's album shared with Alice.
    let cb = collection(db, bob).await;
    let fb = file(&srv, bob, cb, 2002).await;
    sqlx::query(
        "INSERT INTO collection_shares (collection_id, from_user_id, to_user_id, encrypted_key, updation_time) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(cb)
    .bind(bob)
    .bind(alice)
    .bind("k")
    .bind(1i64)
    .execute(db)
    .await
    .unwrap();
    // Bob's private file.
    let cb2 = collection(db, bob).await;
    let fp = file(&srv, bob, cb2, 2003).await;

    let base = format!("{}/memory-share", srv.url);
    let item = |id: i64| json!({"fileID": id, "encryptedKey": format!("mk{id}"), "keyDecryptionNonce": "mn"});
    let body = |files: Vec<serde_json::Value>| {
        json!({"metadataCipher": "mc", "metadataNonce": "mno", "encryptedKey": "sk",
               "keyDecryptionNonce": "skn", "memoryHash": "hash", "files": files})
    };

    // Validation.
    let r = post_json(&base, &[], body(vec![item(f1)])).await;
    assert_eq!(r.status, 401);
    let r = post_json(&base, &auth(&at), body(vec![])).await;
    assert_eq!((r.status, r.json()), (400, json!({})));
    let mut bad_type = body(vec![item(f1)]);
    bad_type["type"] = json!("nope");
    assert_eq!(post_json(&base, &auth(&at), bad_type).await.status, 400);
    assert_eq!(
        post_json(&base, &auth(&at), body(vec![item(f1), item(f1)]))
            .await
            .status,
        400
    );
    assert_eq!(
        post_json(&base, &auth(&at), body(vec![item(fp)]))
            .await
            .status,
        403
    );
    assert_eq!(
        post_json(&base, &auth(&at), body(vec![item(999_999)]))
            .await
            .status,
        400
    );
    let mut neg = item(f1);
    neg["position"] = json!(-1);
    assert_eq!(
        post_json(&base, &auth(&at), body(vec![neg])).await.status,
        400
    );

    let r = post_json(&base, &auth(&at), body(vec![item(f2), item(f1), item(fb)])).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let share = r.json()["memoryShare"].clone();
    let sid = share["id"].as_i64().unwrap();
    let access = share["accessToken"].as_str().unwrap().to_string();
    assert_eq!(access.len(), 10);
    assert_eq!(share["type"], "share");
    assert_eq!(share["memoryHash"], "hash");
    assert_eq!(share["encryptedKey"], "sk");
    assert!(share.get("isDeleted").is_none());
    assert!(share.get("userID").is_none());
    assert_eq!(share["url"], format!("{}/{access}", srv.url));

    let r = get(&base, &auth(&at)).await;
    assert_eq!(r.json()["memoryShares"].as_array().unwrap().len(), 1);
    let r = get(&base, &auth(&bt)).await;
    assert_eq!(r.json(), json!({"memoryShares": []}));
    let r = get(&format!("{base}/{sid}"), &auth(&at)).await;
    assert_eq!(r.json()["memoryShare"]["id"], sid);
    assert_eq!(get(&format!("{base}/{sid}"), &auth(&bt)).await.status, 403);
    assert_eq!(get(&format!("{base}/abc"), &auth(&at)).await.status, 400);
    assert_eq!(get(&format!("{base}/999999"), &auth(&at)).await.status, 404);

    // Public side.
    let pm = format!("{}/public-memory", srv.url);
    let h = [("X-Auth-Access-Token", access.as_str())];
    let r = get(&format!("{pm}/info"), &[]).await;
    assert_eq!(
        (r.status, r.json()["context"].clone()),
        (401, json!("memory_share"))
    );
    let r = get(&format!("{pm}/info"), &[("X-Auth-Access-Token", "BAD")]).await;
    assert_eq!(r.status, 401);
    let r = get(&format!("{pm}/info"), &h).await;
    assert_eq!(r.json()["memoryShare"]["accessToken"], access);
    // Bob's file drops out once it leaves every collection.
    sqlx::query("UPDATE collection_files SET is_deleted = TRUE WHERE file_id = $1")
        .bind(fb)
        .execute(db)
        .await
        .unwrap();
    let r = get(&format!("{pm}/files"), &h).await;
    let files = r.json()["files"].as_array().unwrap().clone();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["file"]["id"], f2);
    assert_eq!(files[0]["position"], 0);
    assert_eq!(files[0]["encryptedKey"], format!("mk{f2}"));
    assert_eq!(files[1]["file"]["id"], f1);
    assert_eq!(files[1]["position"], 1);
    assert!(files[0]["file"].get("magicMetadata").is_none());

    let r = get(&format!("{pm}/files/download/v3/{f1}"), &h).await;
    let url = r.json()["url"].as_str().unwrap().to_string();
    assert_eq!(get(&url, &[]).await.body, b"hello");
    let r = get(&format!("{pm}/files/preview/{f1}"), &h).await;
    assert_eq!(r.status, 307);
    assert_eq!(
        get(&format!("{pm}/files/download/{fb}"), &h).await.status,
        404
    );
    assert_eq!(
        get(&format!("{pm}/files/download/{fp}"), &h).await.status,
        404
    );
    let r = get(&format!("{pm}/files/thumbnail/v3/{fp}"), &h).await;
    assert_eq!(
        (r.status, r.json()["code"].clone()),
        (400, json!("NOT_FOUND"))
    );
    let r = get(&format!("{pm}/file-data?fileID={f1}&type=mldata"), &h).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["message"], "unsupported object type");
    let r = get(
        &format!("{pm}/file-data?fileID={f1}&type=vid_preview&preferNoContent=true"),
        &h,
    )
    .await;
    assert_eq!(r.status, 204);
    let r = get(
        &format!("{pm}/files/data/preview?fileID={f1}&type=img_preview"),
        &h,
    )
    .await;
    assert_eq!(r.status, 404);

    // Expiry after 7 days.
    sqlx::query("UPDATE memory_shares SET created_at = $1 WHERE id = $2")
        .bind(now_micros() - 8 * 24 * 3600 * 1_000_000)
        .bind(sid)
        .execute(db)
        .await
        .unwrap();
    let r = get(&format!("{pm}/info"), &h).await;
    assert_eq!(r.status, 410);
    assert_eq!(
        r.json(),
        json!({"code": "LINK_EXPIRED", "error": "expired token"})
    );
    assert_eq!(get(&format!("{base}/{sid}"), &auth(&at)).await.status, 404);
    assert_eq!(
        get(&base, &auth(&at)).await.json(),
        json!({"memoryShares": []})
    );
    sqlx::query("UPDATE memory_shares SET created_at = $1 WHERE id = $2")
        .bind(now_micros())
        .bind(sid)
        .execute(db)
        .await
        .unwrap();

    // Delete.
    assert_eq!(
        http("DELETE", &format!("{base}/{sid}"), &auth(&bt), b"")
            .await
            .status,
        404
    );
    let r = http("DELETE", &format!("{base}/{sid}"), &auth(&at), b"").await;
    assert_eq!((r.status, r.json()), (200, json!({})));
    assert_eq!(
        http("DELETE", &format!("{base}/{sid}"), &auth(&at), b"")
            .await
            .status,
        404
    );
    let r = get(&format!("{pm}/info"), &h).await;
    assert_eq!(r.status, 410);
    assert_eq!(r.json()["code"], "LINK_DISABLED");

    // Lane type and explicit positions.
    let mut lane = body(vec![
        json!({"fileID": f1, "position": 5, "encryptedKey": "a", "keyDecryptionNonce": "b"}),
    ]);
    lane["type"] = json!("lane");
    let r = post_json(&base, &auth(&at), lane).await;
    assert_eq!(r.json()["memoryShare"]["type"], "lane");
}
