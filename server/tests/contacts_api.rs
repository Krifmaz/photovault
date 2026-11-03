//! /contacts/* and /attachments/*: eligibility, CRUD, diff, profile pictures
//! through signed URLs, and background cleanup of replaced objects.

#[path = "legacy_support/mod.rs"]
mod support;

use photovault_server::crypto::b64;
use photovault_server::routes::contacts::{attachment_object_key, cleanup_attachments};
use photovault_server::testing::TestServer;
use serde_json::{Value, json};
use support::*;

fn card(contact_user_id: i64, data: &[u8]) -> Value {
    json!({"contactUserID": contact_user_id, "encryptedKey": b64(b"card-key"), "encryptedData": b64(data)})
}

async fn upload_picture(api: &Api<'_>, u: &User, contact_id: &str, bytes: &[u8]) -> String {
    use base64::Engine;
    use md5::Digest;
    let md5 = base64::engine::general_purpose::STANDARD.encode(md5::Md5::digest(bytes));
    let r = api
        .post(
            u,
            &format!("/contacts/{contact_id}/profile-picture/upload-url"),
            json!({"contentLength": bytes.len(), "contentMD5": md5}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let j = r.json();
    let attachment_id = j["attachmentID"].as_str().unwrap().to_string();
    assert!(attachment_id.starts_with("ua_"));
    let put = raw(
        "PUT",
        j["url"].as_str().unwrap(),
        &[("Content-MD5", md5.as_str())],
        bytes.to_vec(),
    )
    .await;
    assert!(put.status == 200 || put.status == 204, "PUT {}", put.status);
    attachment_id
}

#[tokio::test]
async fn contacts_crud_diff_and_profile_pictures() {
    let srv = TestServer::start().await.unwrap();
    let api = Api::new(&srv);
    let u = create_user(&srv, "card-owner@example.com", PHOTOS).await;
    let friend = create_user(&srv, "Friend@Example.com", PHOTOS).await;
    let stranger = create_user(&srv, "stranger@example.com", PHOTOS).await;
    for x in [&u, &friend, &stranger] {
        set_key_attributes(&srv, x.id).await;
    }

    // Validation and eligibility.
    let r = api
        .post(
            &u,
            "/contacts",
            json!({"contactUserID": friend.id, "encryptedData": b64(b"d")}),
        )
        .await;
    assert_eq!((r.status, r.json()), (400, json!({})));
    let r = api
        .post(
            &u,
            "/contacts",
            json!({"contactUserID": friend.id, "encryptedKey": "%%%", "encryptedData": b64(b"d")}),
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(api.post(&u, "/contacts", card(-4, b"d")).await.status, 400);
    assert_eq!(
        api.post(&u, "/contacts", card(987_654_321, b"d"))
            .await
            .status,
        400
    );
    let r = api.post(&u, "/contacts", card(friend.id, b"d")).await;
    assert_eq!(r.status, 400, "no relationship yet");
    assert_eq!(r.json()["code"], "BAD_REQUEST");
    // A trusted-contact invite makes them eligible.
    let r = api
        .post(
            &u,
            "/emergency-contacts/add",
            json!({"email": friend.email, "encryptedKey": "ek"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));

    let r = api.post(&u, "/contacts", card(friend.id, b"data-1")).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let c = r.json();
    let id = c["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("ct_"));
    assert_eq!(c["userID"], u.id);
    assert_eq!(c["contactUserID"], friend.id);
    assert_eq!(c["email"], friend.email.as_str());
    assert_eq!(c["encryptedKey"], b64(b"card-key"));
    assert_eq!(c["encryptedData"], b64(b"data-1"));
    assert_eq!(c["isDeleted"], false);
    assert_eq!(c["profilePictureAttachmentID"], Value::Null);
    // One live card per contact user.
    assert_eq!(
        api.post(&u, "/contacts", card(friend.id, b"again"))
            .await
            .status,
        400
    );

    let r = api.get(&u, &format!("/contacts/{id}")).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["id"], id.as_str());
    assert_eq!(api.get(&u, "/contacts/bogus").await.status, 400);
    let r = api.get(&u, "/contacts/ct_missing").await;
    assert_eq!(r.status, 404);
    assert_eq!(r.json()["code"], "NOT_FOUND");
    assert_eq!(
        api.get(&stranger, &format!("/contacts/{id}")).await.status,
        404
    );

    let r = api
        .put(
            &u,
            &format!("/contacts/{id}"),
            json!({"contactUserID": friend.id, "encryptedData": b64(b"data-2")}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let updated = r.json();
    assert_eq!(updated["encryptedData"], b64(b"data-2"));
    assert!(updated["updatedAt"].as_i64().unwrap() > c["updatedAt"].as_i64().unwrap());
    let r = api
        .put(
            &u,
            &format!("/contacts/{id}"),
            json!({"contactUserID": stranger.id, "encryptedData": b64(b"x")}),
        )
        .await;
    assert_eq!(r.status, 400, "cannot retarget to an ineligible user");

    // Profile picture: upload-url → PUT → commit → URL → replace → delete.
    let pic1 = upload_picture(&api, &u, &id, b"picture-one").await;
    let path = format!("/contacts/{id}/profile-picture");
    let r = api
        .put(&u, &path, json!({"attachmentID": pic1, "size": 3}))
        .await;
    assert_eq!(r.status, 500, "size mismatch");
    let r = api
        .put(&u, &path, json!({"attachmentID": pic1, "size": 11}))
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["profilePictureAttachmentID"], pic1.as_str());
    let r = api.get(&u, &path).await;
    assert_eq!(r.status, 200);
    let got = raw("GET", r.json()["url"].as_str().unwrap(), &[], vec![]).await;
    assert_eq!(
        (got.status, got.body.as_slice()),
        (200, &b"picture-one"[..])
    );
    let r = api
        .get(&u, &format!("/attachments/profile_picture/{pic1}"))
        .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        api.get(&u, &format!("/attachments/cover/{pic1}"))
            .await
            .status,
        400
    );
    assert_eq!(
        api.get(&stranger, &format!("/attachments/profile_picture/{pic1}"))
            .await
            .status,
        404
    );
    let r = api
        .post(
            &u,
            "/attachments/profile_picture/upload-url",
            json!({"contentLength": 600 * 1024, "contentMD5": "x"}),
        )
        .await;
    assert_eq!(r.status, 400, "too large");

    let pic2 = upload_picture(&api, &u, &id, b"picture-two!").await;
    let r = api
        .put(
            &u,
            &format!("/contacts/{id}/attachments/profile_picture"),
            json!({"attachmentID": pic2, "size": 12}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["profilePictureAttachmentID"], pic2.as_str());
    assert_eq!(
        api.get(&u, &format!("/attachments/profile_picture/{pic1}"))
            .await
            .status,
        404
    );
    // Committing a staged upload twice fails.
    let r = api
        .put(&u, &path, json!({"attachmentID": pic2, "size": 12}))
        .await;
    assert_eq!(r.status, 400);

    let key1 = attachment_object_key(u.id, "profile_picture", &pic1);
    let key2 = attachment_object_key(u.id, "profile_picture", &pic2);
    assert_eq!(
        key1,
        format!("{}/user-attachments/profile_picture/{pic1}", u.id)
    );
    assert!(srv.state.store.head(&key1).await.unwrap().is_some());
    cleanup_attachments(&srv.state).await.unwrap();
    assert!(
        srv.state.store.head(&key1).await.unwrap().is_none(),
        "replaced object removed"
    );
    assert!(srv.state.store.head(&key2).await.unwrap().is_some());

    let r = api.delete(&u, &path).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["profilePictureAttachmentID"], Value::Null);
    let r = api.get(&u, &path).await;
    assert_eq!(r.status, 404);
    cleanup_attachments(&srv.state).await.unwrap();
    assert!(srv.state.store.head(&key2).await.unwrap().is_none());

    // An abandoned upload is removed once its stage expires.
    let pic3 = upload_picture(&api, &u, &id, b"abandoned").await;
    let key3 = attachment_object_key(u.id, "profile_picture", &pic3);
    sqlx::query("UPDATE user_attachment_uploads SET expires_at = 1")
        .execute(&srv.state.db.pool)
        .await
        .unwrap();
    cleanup_attachments(&srv.state).await.unwrap();
    assert!(srv.state.store.head(&key3).await.unwrap().is_none());

    // Delete, diff.
    let second = {
        let r = api
            .post(&friend, "/contacts", card(u.id, b"friend-card"))
            .await;
        assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
        r.json()
    };
    assert_eq!(second["email"], u.email.as_str());
    let r = api.delete(&u, &format!("/contacts/{id}")).await;
    assert_eq!(r.status, 200);
    let r = api.get(&u, "/contacts/diff?sinceTime=0&limit=10").await;
    assert_eq!(r.status, 200);
    let diff = r.json()["diff"].as_array().unwrap().clone();
    assert_eq!(diff.len(), 1);
    assert_eq!(diff[0]["id"], id.as_str());
    assert_eq!(diff[0]["isDeleted"], true);
    assert_eq!(diff[0]["encryptedData"], Value::Null);
    assert_eq!(diff[0]["encryptedKey"], Value::Null);
    let since = diff[0]["updatedAt"].as_i64().unwrap();
    let r = api
        .get(&u, &format!("/contacts/diff?sinceTime={since}&limit=10"))
        .await;
    assert_eq!(r.json(), json!({"diff": []}));
    let r = api.get(&u, "/contacts/diff?sinceTime=0&limit=0").await;
    assert_eq!((r.status, r.json()), (400, json!({})));
    assert_eq!(
        api.get(&u, "/contacts/diff?sinceTime=0&limit=9000")
            .await
            .status,
        400
    );
    // As in museum, the (user, contact user) pair stays taken by the deleted
    // card; updating that card revives it.
    assert_eq!(
        api.post(&u, "/contacts", card(friend.id, b"fresh"))
            .await
            .status,
        400
    );
    let r = api
        .put(
            &u,
            &format!("/contacts/{id}"),
            json!({"contactUserID": friend.id, "encryptedData": b64(b"z")}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["isDeleted"], false);
    assert_eq!(r.json()["encryptedData"], b64(b"z"));
    assert_eq!(r.json()["email"], friend.email.as_str());
}
