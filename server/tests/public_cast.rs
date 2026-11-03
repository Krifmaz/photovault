//! /cast/*: device pairing and cast-token access to one album.

mod support;

use serde_json::json;
use support::*;

#[tokio::test]
async fn cast_pairing_and_access() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (alice, at) = user(db, 1 << 30).await;
    let (_bob, bt) = user(db, 1 << 30).await;
    let cid = collection(db, alice).await;
    let f1 = file(&srv, alice, cid, 2000).await;
    let other = collection(db, alice).await;
    let f_other = file(&srv, alice, other, 2001).await;
    let auth_a = [("X-Auth-Token", at.as_str())];
    let cast = format!("{}/cast", srv.url);

    // Register (public; with and without trailing slash).
    let r = post_json(&format!("{cast}/device-info"), &[], json!({})).await;
    assert_eq!(r.status, 400);
    let r = post_json(
        &format!("{cast}/device-info"),
        &[],
        json!({"publicKey": "pk", "pqPublicKey": "dG9vc2hvcnQ="}),
    )
    .await;
    assert_eq!(r.status, 400);
    let pq = photovault_server::crypto::b64(&[7u8; 1216]);
    let r = post_json(
        &format!("{cast}/device-info/"),
        &[("User-Agent", "Mozilla/5.0 (SMART-TV; Linux; Tizen 6.0)")],
        json!({"publicKey": "pk", "pqPublicKey": pq}),
    )
    .await;
    assert_eq!(r.status, 200);
    let code = r.json()["deviceCode"].as_str().unwrap().to_string();
    assert_eq!(code.len(), 6);
    assert!(code.chars().next().unwrap().is_ascii_uppercase());

    // Device info needs auth; codes are case-insensitive.
    assert_eq!(
        get(&format!("{cast}/device-info/{code}"), &[]).await.status,
        401
    );
    let r = get(
        &format!("{cast}/device-info/{}", code.to_lowercase()),
        &auth_a,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json(), json!({"publicKey": "pk", "pqPublicKey": pq}));
    let r = get(&format!("{cast}/device-info/ZZZZZZ"), &auth_a).await;
    assert_eq!(r.status, 404);
    assert_eq!(
        r.json(),
        json!({"code": "NOT_FOUND", "message": "code not found"})
    );

    // Before pairing the TV polls and sees null.
    let r = get(&format!("{cast}/cast-data/{code}"), &[]).await;
    assert_eq!(r.json(), json!({"encCastData": null}));
    assert_eq!(
        get(&format!("{cast}/cast-data/NOCODE"), &[]).await.status,
        404
    );

    // Pair.
    let pair = json!({"collectionID": cid, "castToken": "cast-secret", "encPayload": "payload", "deviceCode": code});
    let r = post_json(&format!("{cast}/cast-data"), &[], pair.clone()).await;
    assert_eq!(r.status, 401);
    let mut big = pair.clone();
    big["encPayload"] = json!("x".repeat(3000));
    assert_eq!(
        post_json(&format!("{cast}/cast-data"), &auth_a, big)
            .await
            .status,
        400
    );
    let r = post_json(&format!("{cast}/cast-data/"), &auth_a, pair.clone()).await;
    assert_eq!(r.status, 200);
    let device_id = r.json()["deviceID"].as_str().unwrap().to_string();
    assert_eq!(device_id.len(), 36);
    let r = post_json(&format!("{cast}/cast-data"), &auth_a, pair.clone()).await;
    assert_eq!(
        (r.status, r.json()["code"].clone()),
        (409, json!("CONFLICT"))
    );
    let r = get(&format!("{cast}/cast-data/{code}"), &[]).await;
    assert_eq!(r.json(), json!({"encCastData": "payload"}));

    let r = get(&format!("{cast}/device-info"), &auth_a).await;
    let devices = r.json()["devices"].as_array().unwrap().clone();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["deviceID"], device_id);
    assert_eq!(devices[0]["collectionID"], cid);
    assert_eq!(devices[0]["deviceName"], "Tv");
    assert!(devices[0]["lastUsedAt"].is_i64());

    // Cast-authenticated endpoints.
    let r = get(&format!("{cast}/info"), &[]).await;
    assert_eq!(
        (r.status, r.json()),
        (401, json!({"error": "cast access token missing"}))
    );
    let r = get(&format!("{cast}/info"), &[("X-Cast-Access-Token", "nope")]).await;
    assert_eq!(
        (r.status, r.json()),
        (401, json!({"error": "invalid token"}))
    );
    let ch = [("X-Cast-Access-Token", "cast-secret")];
    let r = get(&format!("{cast}/info"), &ch).await;
    assert_eq!(r.status, 200);
    let c = r.json()["collection"].clone();
    assert_eq!(c["id"], cid);
    assert_eq!(c["publicURLs"], json!([]));
    assert!(c.get("magicMetadata").is_none());
    let r = get(
        &format!("{cast}/diff?sinceTime=0&castToken=cast-secret"),
        &[],
    )
    .await;
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["hasMore"], false);
    assert_eq!(v["diff"][0]["id"], f1);
    assert!(v["diff"][0].get("magicMetadata").is_none());
    assert_eq!(get(&format!("{cast}/diff"), &ch).await.status, 500);
    let r = get(&format!("{cast}/files/download/v3/{f1}"), &ch).await;
    let url = r.json()["url"].as_str().unwrap().to_string();
    assert_eq!(get(&url, &[]).await.body, b"hello");
    let r = get(&format!("{cast}/files/thumbnail/v3/{f1}"), &ch).await;
    assert_eq!(r.status, 200);
    let r = get(
        &format!("{cast}/files/preview/{f1}?castToken=cast-secret"),
        &[],
    )
    .await;
    assert_eq!(r.status, 307);
    assert_eq!(get(&r.headers["location"], &[]).await.body, b"thm");
    let r = get(&format!("{cast}/files/download/{f1}"), &ch).await;
    assert_eq!(r.status, 307);
    assert_eq!(
        get(&format!("{cast}/files/download/v3/{f_other}"), &ch)
            .await
            .status,
        403
    );

    // Deleting the device (bad id → 400, other user's → no-op) revokes the token.
    let r = http(
        "DELETE",
        &format!("{cast}/device-info/not-a-uuid"),
        &auth_a,
        b"",
    )
    .await;
    assert_eq!(r.status, 400);
    let r = http(
        "DELETE",
        &format!("{cast}/device-info/{device_id}"),
        &[("X-Auth-Token", bt.as_str())],
        b"",
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(get(&format!("{cast}/info"), &ch).await.status, 200);
    let r = http(
        "DELETE",
        &format!("{cast}/device-info/{}", device_id.to_uppercase()),
        &auth_a,
        b"",
    )
    .await;
    assert_eq!((r.status, r.json()), (200, json!({})));
    assert_eq!(get(&format!("{cast}/info"), &ch).await.status, 401);
}

#[tokio::test]
async fn sharee_cast_and_revoke_all() {
    let srv = server().await;
    let db = &srv.state.db.pool;
    let (alice, _) = user(db, 1 << 30).await;
    let (bob, bt) = user(db, 1 << 30).await;
    let cid = collection(db, alice).await;
    file(&srv, alice, cid, 2000).await;
    sqlx::query(
        "INSERT INTO collection_shares (collection_id, from_user_id, to_user_id, encrypted_key, updation_time) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(cid)
    .bind(alice)
    .bind(bob)
    .bind("k")
    .bind(1i64)
    .execute(db)
    .await
    .unwrap();
    let cast = format!("{}/cast", srv.url);
    let auth_b = [("X-Auth-Token", bt.as_str())];
    let mut tokens = Vec::new();
    for i in 0..2 {
        let r = post_json(
            &format!("{cast}/device-info"),
            &[],
            json!({"publicKey": "pk"}),
        )
        .await;
        let code = r.json()["deviceCode"].as_str().unwrap().to_string();
        let token = format!("tok{i}");
        let r = post_json(
            &format!("{cast}/cast-data"),
            &auth_b,
            json!({"collectionID": cid, "castToken": token, "encPayload": "p", "deviceCode": code}),
        )
        .await;
        assert_eq!(r.status, 200);
        tokens.push(token);
    }
    for t in &tokens {
        assert_eq!(
            get(&format!("{cast}/info"), &[("X-Cast-Access-Token", t)])
                .await
                .status,
            200
        );
    }
    // Unsharing cuts the sharee's casts off.
    sqlx::query("UPDATE collection_shares SET is_deleted = TRUE WHERE to_user_id = $1")
        .bind(bob)
        .execute(db)
        .await
        .unwrap();
    assert_eq!(
        get(
            &format!("{cast}/info"),
            &[("X-Cast-Access-Token", tokens[0].as_str())]
        )
        .await
        .status,
        401
    );
    sqlx::query("UPDATE collection_shares SET is_deleted = FALSE WHERE to_user_id = $1")
        .bind(bob)
        .execute(db)
        .await
        .unwrap();
    let r = http(
        "DELETE",
        &format!("{cast}/revoke-all-tokens/"),
        &auth_b,
        b"",
    )
    .await;
    assert_eq!((r.status, r.json()), (200, json!({})));
    for t in &tokens {
        assert_eq!(
            get(&format!("{cast}/info"), &[("X-Cast-Access-Token", t)])
                .await
                .status,
            401
        );
    }
    let r = http("DELETE", &format!("{cast}/revoke-all-tokens"), &auth_b, b"").await;
    assert_eq!(r.status, 200);
    assert_eq!(
        get(&format!("{cast}/device-info"), &auth_b).await.json(),
        json!({"devices": []})
    );
}
