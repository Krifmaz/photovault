//! /user-entity/*: keys, entity CRUD, diff semantics, smart album restore,
//! library share optimistic concurrency, validation.

#[path = "legacy_support/mod.rs"]
mod support;

use photovault_server::testing::TestServer;
use serde_json::{Value, json};
use support::*;

fn key(kind: &str, ek: &str) -> Value {
    json!({"type": kind, "encryptedKey": ek, "header": "key-header"})
}

#[tokio::test]
async fn user_entity_keys_crud_and_diff() {
    let srv = TestServer::start().await.unwrap();
    let api = Api::new(&srv);
    let u = create_user(&srv, "ue@example.com", PHOTOS).await;
    let other = create_user(&srv, "ue2@example.com", PHOTOS).await;

    // The Auth app may not use these endpoints.
    let auth = User {
        id: u.id,
        email: u.email.clone(),
        token: add_token(&srv, u.id, AUTH).await,
        package: AUTH,
    };
    assert_eq!(
        api.get(&auth, "/user-entity/key?type=person").await.status,
        403
    );

    // Keys.
    let r = api.get(&u, "/user-entity/key?type=person").await;
    assert_eq!((r.status, r.json()), (404, json!({})));
    assert_eq!(api.get(&u, "/user-entity/key").await.status, 400);
    let r = api.post(&u, "/user-entity/key", key("bogus", "k")).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "BAD_REQUEST");
    assert_eq!(
        api.post(&u, "/user-entity/key", json!({"type": "person"}))
            .await
            .status,
        400
    );
    assert_eq!(
        api.post(&u, "/user-entity/key", key("person", "k1"))
            .await
            .status,
        200
    );
    // Same key again is idempotent; a different one conflicts.
    assert_eq!(
        api.post(&u, "/user-entity/key", key("person", "k1"))
            .await
            .status,
        200
    );
    let r = api.post(&u, "/user-entity/key", key("person", "k2")).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["code"], "ALREADY_EXISTS");
    let r = api.get(&u, "/user-entity/key?type=person").await;
    assert_eq!(r.status, 200);
    let k = r.json();
    assert_eq!(k["userID"], u.id);
    assert_eq!(k["type"], "person");
    assert_eq!(k["encryptedKey"], "k1");
    assert_eq!(k["header"], "key-header");
    assert!(k["createdAt"].as_i64().unwrap() > 0);
    // Ensure returns the existing key, or creates one.
    let r = api
        .post(&u, "/user-entity/key/ensure", key("person", "k9"))
        .await;
    assert_eq!(r.status, 200);
    assert!(
        r.headers
            .to_ascii_lowercase()
            .contains("cache-control: no-store")
    );
    assert_eq!(r.json()["encryptedKey"], "k1");
    let r = api
        .post(&u, "/user-entity/key/ensure", key("location", "lk"))
        .await;
    assert_eq!(r.json()["encryptedKey"], "lk");
    assert_eq!(
        api.get(&other, "/user-entity/key?type=person").await.status,
        404
    );

    // Entities.
    let body = json!({"type": "person", "encryptedData": "d1", "header": "h1"});
    let r = api.post(&u, "/user-entity/entity", body).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let e1 = r.json();
    let id1 = e1["id"].as_str().unwrap().to_string();
    assert!(id1.starts_with("person_"));
    assert_eq!(e1["userID"], u.id);
    assert_eq!(e1["encryptedData"], "d1");
    assert_eq!(e1["isDeleted"], false);
    assert_eq!(e1["createdAt"], e1["updatedAt"]);
    let r = api
        .post(
            &u,
            "/user-entity/entity",
            json!({"type": "person", "encryptedData": "d2", "header": "h2"}),
        )
        .await;
    let id2 = r.json()["id"].as_str().unwrap().to_string();
    let r = api
        .post(
            &u,
            "/user-entity/entity",
            json!({"type": "nope", "encryptedData": "d", "header": "h"}),
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(
        api.post(
            &u,
            "/user-entity/entity",
            json!({"type": "person", "header": "h"})
        )
        .await
        .status,
        400
    );

    let r = api
        .put(
            &u,
            "/user-entity/entity",
            json!({"id": id1, "type": "person", "encryptedData": "d1b", "header": "h1b"}),
        )
        .await;
    assert_eq!(r.status, 200);
    let up = r.json();
    assert_eq!(up["encryptedData"], "d1b");
    assert!(up["updatedAt"].as_i64().unwrap() > e1["updatedAt"].as_i64().unwrap());
    // Wrong type for the id.
    let r = api
        .put(
            &u,
            "/user-entity/entity",
            json!({"id": id1, "type": "location", "encryptedData": "x", "header": "y"}),
        )
        .await;
    assert_eq!(r.status, 400);
    // Someone else's id.
    let r = api
        .put(
            &other,
            "/user-entity/entity",
            json!({"id": id1, "type": "person", "encryptedData": "x", "header": "y"}),
        )
        .await;
    assert_eq!(r.status, 404);

    let r = api
        .delete(&u, &format!("/user-entity/entity?id={id2}"))
        .await;
    assert_eq!(r.status, 200);
    let r = api
        .put(
            &u,
            "/user-entity/entity",
            json!({"id": id2, "type": "person", "encryptedData": "x", "header": "y"}),
        )
        .await;
    assert_eq!(r.status, 400);

    // Diff: everything since 0, in updated_at order, deleted rows with null data.
    let r = api
        .get(
            &u,
            "/user-entity/entity/diff?type=person&sinceTime=0&limit=10",
        )
        .await;
    assert_eq!(r.status, 200);
    let diff = r.json()["diff"].as_array().unwrap().clone();
    assert_eq!(diff.len(), 2);
    assert_eq!(diff[0]["id"], id1.as_str());
    assert_eq!(diff[1]["id"], id2.as_str());
    assert_eq!(diff[1]["isDeleted"], true);
    assert_eq!(diff[1]["encryptedData"], Value::Null);
    assert_eq!(diff[1]["header"], Value::Null);
    let since = diff[0]["updatedAt"].as_i64().unwrap();
    let r = api
        .get(
            &u,
            &format!("/user-entity/entity/diff?type=person&sinceTime={since}&limit=10"),
        )
        .await;
    let diff = r.json()["diff"].as_array().unwrap().clone();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0]["id"], id2.as_str());
    let r = api
        .get(
            &u,
            "/user-entity/entity/diff?type=person&sinceTime=0&limit=1",
        )
        .await;
    assert_eq!(r.json()["diff"].as_array().unwrap().len(), 1);
    let r = api
        .get(
            &u,
            "/user-entity/entity/diff?type=location&sinceTime=0&limit=10",
        )
        .await;
    assert_eq!(r.json(), json!({"diff": []}));
    let r = api
        .get(
            &other,
            "/user-entity/entity/diff?type=person&sinceTime=0&limit=10",
        )
        .await;
    assert_eq!(r.json(), json!({"diff": []}));

    // Limit validation.
    let r = api
        .get(
            &u,
            "/user-entity/entity/diff?type=person&sinceTime=0&limit=0",
        )
        .await;
    assert_eq!((r.status, r.json()), (400, json!({})));
    let r = api
        .get(
            &u,
            "/user-entity/entity/diff?type=person&sinceTime=0&limit=5001",
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "BAD_REQUEST");
    let r = api
        .get(&u, "/user-entity/entity/diff?type=person&limit=10")
        .await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn user_entity_smart_album_and_library_share() {
    let srv = TestServer::start().await.unwrap();
    let api = Api::new(&srv);
    let u = create_user(&srv, "sa@example.com", PHOTOS).await;
    let friend = create_user(&srv, "friend@example.com", PHOTOS).await;
    // Entities reference their type's key (museum's foreign key).
    for kind in ["smart_album", "library_share"] {
        assert_eq!(
            api.post(&u, "/user-entity/key", key(kind, "k"))
                .await
                .status,
            200
        );
    }

    // Smart albums need a caller-chosen id `sa_<uid>_…`.
    let r = api
        .post(
            &u,
            "/user-entity/entity",
            json!({"type": "smart_album", "encryptedData": "d", "header": "h"}),
        )
        .await;
    assert_eq!(r.status, 400);
    let bad_id = format!("sa_{}_x", friend.id);
    let r = api
        .post(
            &u,
            "/user-entity/entity",
            json!({"type": "smart_album", "id": bad_id, "encryptedData": "d", "header": "h"}),
        )
        .await;
    assert_eq!(r.status, 400);
    let sa = format!("sa_{}_album1", u.id);
    let body = json!({"type": "smart_album", "id": sa, "encryptedData": "d1", "header": "h1"});
    let r = api.post(&u, "/user-entity/entity", body.clone()).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["id"], sa.as_str());
    let r = api.post(&u, "/user-entity/entity", body.clone()).await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["code"], "ALREADY_EXISTS");
    // Deleted smart albums can be restored by re-creating them.
    assert_eq!(
        api.delete(&u, &format!("/user-entity/entity?id={sa}"))
            .await
            .status,
        200
    );
    let r = api
        .post(
            &u,
            "/user-entity/entity",
            json!({"type": "smart_album", "id": sa, "encryptedData": "d2", "header": "h2"}),
        )
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["isDeleted"], false);
    assert_eq!(r.json()["encryptedData"], "d2");

    // Library shares: `ls_<owner>_<recipient>`, updates need expectedUpdatedAt.
    for bad in [
        format!("ls_{}_{}", u.id, u.id),
        format!("ls_{}_{}", friend.id, u.id),
        format!("ls_{}_0", u.id),
        format!("ls_{}_{}_x", u.id, friend.id),
        format!("ls_{}_0{}", u.id, friend.id),
    ] {
        let r = api
            .post(
                &u,
                "/user-entity/entity",
                json!({"type": "library_share", "id": bad, "encryptedData": "d", "header": "h"}),
            )
            .await;
        assert_eq!(r.status, 400, "{bad}");
    }
    let ls = format!("ls_{}_{}", u.id, friend.id);
    let r = api
        .post(
            &u,
            "/user-entity/entity",
            json!({"type": "library_share", "id": ls, "encryptedData": "d1", "header": "h1"}),
        )
        .await;
    assert_eq!(r.status, 200);
    let v1 = r.json()["updatedAt"].as_i64().unwrap();
    let r = api
        .put(
            &u,
            "/user-entity/entity",
            json!({"id": ls, "type": "library_share", "encryptedData": "d2", "header": "h2"}),
        )
        .await;
    assert_eq!(r.status, 400);
    let r = api
        .put(
            &u,
            "/user-entity/entity",
            json!({"id": ls, "type": "library_share", "encryptedData": "d2", "header": "h2", "expectedUpdatedAt": v1}),
        )
        .await;
    assert_eq!(r.status, 200);
    let v2 = r.json()["updatedAt"].as_i64().unwrap();
    // A stale version with different content is a version mismatch.
    let r = api
        .put(
            &u,
            "/user-entity/entity",
            json!({"id": ls, "type": "library_share", "encryptedData": "d3", "header": "h3", "expectedUpdatedAt": v1}),
        )
        .await;
    assert_eq!((r.status, r.json()), (409, json!({})));
    // Replaying the already-applied update is accepted.
    let r = api
        .put(
            &u,
            "/user-entity/entity",
            json!({"id": ls, "type": "library_share", "encryptedData": "d2", "header": "h2", "expectedUpdatedAt": v1}),
        )
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["updatedAt"], v2);
}
