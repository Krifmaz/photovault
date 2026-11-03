//! /authenticator/*: Auth-app-only key and entity endpoints.

#[path = "legacy_support/mod.rs"]
mod support;

use photovault_server::entities::ids::new_uuid;
use photovault_server::testing::TestServer;
use serde_json::{Value, json};
use support::*;

#[tokio::test]
async fn authenticator_key_entities_and_diff() {
    let srv = TestServer::start().await.unwrap();
    let api = Api::new(&srv);
    let u = create_user(&srv, "totp@example.com", AUTH).await;
    let other = create_user(&srv, "totp2@example.com", AUTH).await;
    let photos = User {
        token: add_token(&srv, u.id, PHOTOS).await,
        package: PHOTOS,
        ..u.clone()
    };

    // Only the Auth app.
    let r = api.get(&photos, "/authenticator/key").await;
    assert_eq!(r.status, 403);
    assert_eq!(r.json(), json!({"error": "invalid app for endpoint"}));
    let r = api
        .post(
            &photos,
            "/authenticator/key",
            json!({"encryptedKey": "k", "header": "h"}),
        )
        .await;
    assert_eq!(r.status, 403);

    // Key.
    let r = api.get(&u, "/authenticator/key").await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["code"], "NOT_FOUND");
    let key = json!({"encryptedKey": "k1", "header": "h1"});
    // The account must have key attributes first.
    assert_eq!(
        api.post(&u, "/authenticator/key", key.clone()).await.status,
        404
    );
    set_key_attributes(&srv, u.id).await;
    set_key_attributes(&srv, other.id).await;
    assert_eq!(
        api.post(&u, "/authenticator/key", json!({"header": "h"}))
            .await
            .status,
        400
    );
    // Entities need the key.
    let r = api
        .post(
            &u,
            "/authenticator/entity",
            json!({"encryptedData": "d", "header": "h"}),
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "AUTH_KEY_NOT_CREATED");
    assert_eq!(
        api.post(&u, "/authenticator/key", key.clone()).await.status,
        200
    );
    let r = api.post(&u, "/authenticator/key", key).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["code"], "CONFLICT");
    let r = api.get(&u, "/authenticator/key").await;
    assert_eq!(r.status, 200);
    let k = r.json();
    assert_eq!(k["userID"], u.id);
    assert_eq!(k["encryptedKey"], "k1");
    assert_eq!(k["header"], "h1");
    assert!(k["createdAt"].as_i64().unwrap() > 0);
    assert_eq!(api.get(&other, "/authenticator/key").await.status, 404);

    // Entities.
    let r = api
        .post(
            &u,
            "/authenticator/entity",
            json!({"encryptedData": "d1", "header": "h1"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let e1 = r.json();
    let id1 = e1["id"].as_str().unwrap().to_string();
    assert_eq!(id1.len(), 36);
    assert_eq!(e1["isDeleted"], false);
    assert_eq!(e1["encryptedData"], "d1");
    let r = api
        .post(
            &u,
            "/authenticator/entity",
            json!({"encryptedData": "d2", "header": "h2"}),
        )
        .await;
    let id2 = r.json()["id"].as_str().unwrap().to_string();
    assert_eq!(
        api.post(&u, "/authenticator/entity", json!({"header": "h"}))
            .await
            .status,
        400
    );

    let r = api
        .put(
            &u,
            "/authenticator/entity",
            json!({"id": id1, "encryptedData": "d1b", "header": "h1b"}),
        )
        .await;
    // museum answers an update with an empty 200.
    assert_eq!((r.status, r.body.len()), (200, 0));
    let r = api
        .put(
            &u,
            "/authenticator/entity",
            json!({"id": "not-a-uuid", "encryptedData": "x", "header": "y"}),
        )
        .await;
    assert_eq!(r.status, 400);
    let r = api
        .put(
            &u,
            "/authenticator/entity",
            json!({"id": new_uuid(), "encryptedData": "x", "header": "y"}),
        )
        .await;
    assert_eq!(r.status, 404);
    let r = api
        .put(
            &other,
            "/authenticator/entity",
            json!({"id": id1, "encryptedData": "x", "header": "y"}),
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "AUTH_KEY_NOT_CREATED");

    assert_eq!(
        api.delete(&u, "/authenticator/entity?id=bad").await.status,
        400
    );
    assert_eq!(
        api.delete(&u, &format!("/authenticator/entity?id={id2}"))
            .await
            .status,
        200
    );
    let r = api
        .put(
            &u,
            "/authenticator/entity",
            json!({"id": id2, "encryptedData": "x", "header": "y"}),
        )
        .await;
    assert_eq!(r.status, 400);

    // Diff.
    let r = api
        .get(&u, "/authenticator/entity/diff?sinceTime=0&limit=10")
        .await;
    assert_eq!(r.status, 200);
    let body = r.json();
    assert!(body["timestamp"].as_i64().unwrap() > 0);
    let diff = body["diff"].as_array().unwrap().clone();
    assert_eq!(diff.len(), 2);
    assert_eq!(diff[0]["id"], id1.as_str());
    assert_eq!(diff[0]["encryptedData"], "d1b");
    assert_eq!(diff[0]["header"], "h1b");
    assert_eq!(diff[1]["id"], id2.as_str());
    assert_eq!(diff[1]["isDeleted"], true);
    assert_eq!(diff[1]["encryptedData"], Value::Null);
    let since = diff[0]["updatedAt"].as_i64().unwrap();
    let r = api
        .get(
            &u,
            &format!("/authenticator/entity/diff?sinceTime={since}&limit=10"),
        )
        .await;
    assert_eq!(r.json()["diff"].as_array().unwrap().len(), 1);
    let r = api
        .get(&u, "/authenticator/entity/diff?sinceTime=0&limit=1")
        .await;
    assert_eq!(r.json()["diff"].as_array().unwrap().len(), 1);
    let r = api
        .get(&other, "/authenticator/entity/diff?sinceTime=0&limit=10")
        .await;
    assert_eq!(r.json()["diff"], json!([]));
    let r = api
        .get(&u, "/authenticator/entity/diff?sinceTime=0&limit=0")
        .await;
    assert_eq!((r.status, r.json()), (400, json!({})));
    let r = api
        .get(&u, "/authenticator/entity/diff?sinceTime=0&limit=6000")
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "BAD_REQUEST");
    assert_eq!(
        api.get(&photos, "/authenticator/entity/diff?sinceTime=0&limit=10")
            .await
            .status,
        403
    );
}
