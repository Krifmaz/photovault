//! Legacy kits: owner CRUD, proof-of-possession challenge, recovery sessions,
//! notice period (shortened via DB edits), password reset, blocking, expiry.

#[path = "legacy_support/mod.rs"]
mod support;

use photovault_server::db::now_micros;
use photovault_server::entities::ids::new_uuid;
use photovault_server::testing::TestServer;
use serde_json::{Value, json};
use support::*;

struct Kit {
    id: String,
    pk: Vec<u8>,
    sk: Vec<u8>,
}

fn kit_body(kit: &Kit, notice: i64) -> Value {
    json!({
        "id": kit.id,
        "variant": 1,
        "noticePeriodInHours": notice,
        "encryptedRecoveryBlob": "recovery-blob",
        "authPublicKey": photovault_server::crypto::b64(&kit.pk),
        "encryptedOwnerBlob": "owner-blob",
    })
}

fn new_kit() -> Kit {
    let (pk, sk) = photovault_server::crypto::box_keypair();
    Kit {
        id: new_uuid(),
        pk,
        sk,
    }
}

/// Fetch a challenge and open it with the kit secret, like the recovery page.
async fn solve(api: &Api<'_>, kit: &Kit) -> String {
    let r = api
        .public_post("/legacy-kits/recovery/challenge", json!({"kitID": kit.id}))
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let c = r.json();
    assert_eq!(c["kitID"], kit.id.as_str());
    assert!(c["expiresAt"].as_i64().unwrap() > now_micros());
    let sealed =
        photovault_server::crypto::unb64(c["encryptedChallenge"].as_str().unwrap()).unwrap();
    let opened = photovault_server::crypto::box_seal_open(&sealed, &kit.pk, &kit.sk).unwrap();
    let s = String::from_utf8(opened).unwrap();
    assert!(s.starts_with(&format!("legacy-kit-open:v1\n{}\n", kit.id)));
    s
}

#[tokio::test]
async fn legacy_kit_owner_and_recovery_flow() {
    let srv = TestServer::start().await.unwrap();
    let api = Api::new(&srv);
    let owner = create_user(&srv, "owner@example.com", PHOTOS).await;
    let stranger = create_user(&srv, "stranger@example.com", PHOTOS).await;
    set_key_attributes(&srv, owner.id).await;
    let kit = new_kit();

    // Validation.
    let mut bad = kit_body(&kit, 48);
    let r = api.post(&owner, "/legacy-kits", bad.clone()).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "BAD_REQUEST");
    bad = kit_body(&kit, 24);
    bad["authPublicKey"] = json!(photovault_server::crypto::b64(&[0u8; 32]));
    let r = api.post(&owner, "/legacy-kits", bad).await;
    assert_eq!(r.status, 400);
    bad = kit_body(&kit, 24);
    bad["variant"] = json!(2);
    assert_eq!(api.post(&owner, "/legacy-kits", bad).await.status, 400);
    bad = kit_body(&kit, 24);
    bad.as_object_mut().unwrap().remove("noticePeriodInHours");
    let r = api.post(&owner, "/legacy-kits", bad).await;
    assert_eq!((r.status, r.json()), (400, json!({})));

    let r = api.post(&owner, "/legacy-kits", kit_body(&kit, 24)).await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let created = r.json();
    assert_eq!(created["id"], kit.id.as_str());
    assert_eq!(created["variant"], 1);
    assert_eq!(created["noticePeriodInHours"], 24);
    assert_eq!(created["encryptedOwnerBlob"], "owner-blob");
    assert!(created["legacyUrl"].as_str().unwrap().starts_with("http"));
    assert!(created.get("activeRecoverySession").is_none());

    let list = api.get(&owner, "/legacy-kits").await.json();
    assert_eq!(list["kits"].as_array().unwrap().len(), 1);
    assert_eq!(
        api.get(&stranger, "/legacy-kits").await.json(),
        json!({"kits": []})
    );
    for p in ["download", "download-content"] {
        let d = api
            .get(&owner, &format!("/legacy-kits/{}/{p}", kit.id))
            .await
            .json();
        assert_eq!(
            d,
            json!({"id": kit.id, "variant": 1, "encryptedOwnerBlob": "owner-blob"})
        );
        assert_eq!(
            api.get(&stranger, &format!("/legacy-kits/{}/{p}", kit.id))
                .await
                .status,
            404
        );
    }
    let r = api.get(&owner, "/legacy-kits/not-a-uuid/download").await;
    assert_eq!(r.status, 400);
    let s = api
        .get(&owner, &format!("/legacy-kits/{}/recovery-session", kit.id))
        .await
        .json();
    assert_eq!(s, json!({"session": null, "initiators": []}));

    // Notice period update.
    let r = api
        .post(
            &owner,
            "/legacy-kits/update-recovery-notice",
            json!({"kitID": kit.id, "noticePeriodInHours": 168}),
        )
        .await;
    assert_eq!(r.status, 200);
    let r = api
        .post(
            &stranger,
            "/legacy-kits/update-recovery-notice",
            json!({"kitID": kit.id, "noticePeriodInHours": 168}),
        )
        .await;
    assert_eq!(r.status, 404);

    // Challenge for an unknown kit → 404; a wrong answer → 400.
    let r = api
        .public_post(
            "/legacy-kits/recovery/challenge",
            json!({"kitID": new_uuid()}),
        )
        .await;
    assert_eq!(r.status, 404);
    let _ = solve(&api, &kit).await;
    let r = api
        .public_post(
            "/legacy-kits/recovery/open",
            json!({"kitID": kit.id, "challenge": "nope"}),
        )
        .await;
    assert_eq!(r.status, 400);
    // Used part indexes must fit the variant.
    let ch = solve(&api, &kit).await;
    let r = api
        .public_post(
            "/legacy-kits/recovery/open",
            json!({"kitID": kit.id, "challenge": ch, "usedPartIndexes": [1, 1]}),
        )
        .await;
    assert_eq!(r.status, 400);

    // Open a recovery session.
    let r = api
        .call(
            None,
            "POST",
            "/legacy-kits/recovery/open",
            Some(json!({"kitID": kit.id, "challenge": ch, "usedPartIndexes": [1, 3], "email": "heir@example.com"})),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let opened = r.json();
    let session_id = opened["session"]["id"].as_str().unwrap().to_string();
    let token = opened["sessionToken"].as_str().unwrap().to_string();
    assert_eq!(opened["session"]["status"], "WAITING");
    assert_eq!(opened["session"]["kitID"], kit.id.as_str());
    let wait = opened["session"]["waitTill"].as_i64().unwrap();
    assert!(wait > 167 * 3_600_000_000 && wait <= 168 * 3_600_000_000);
    // The challenge is single-use.
    let r = api
        .public_post(
            "/legacy-kits/recovery/open",
            json!({"kitID": kit.id, "challenge": ch}),
        )
        .await;
    assert_eq!(r.status, 400);

    // A second browser resumes the same session with its own token.
    let ch2 = solve(&api, &kit).await;
    let r = api
        .public_post(
            "/legacy-kits/recovery/open",
            json!({"kitID": kit.id, "challenge": ch2}),
        )
        .await;
    let resumed = r.json();
    assert_eq!(resumed["session"]["id"], session_id.as_str());
    let token2 = resumed["sessionToken"].as_str().unwrap().to_string();
    assert_ne!(token, token2);

    // Owner sees the session and its initiators; notice cannot change now.
    let s = api
        .get(&owner, &format!("/legacy-kits/{}/recovery-session", kit.id))
        .await
        .json();
    assert_eq!(s["session"]["id"], session_id.as_str());
    assert_eq!(s["initiators"].as_array().unwrap().len(), 2);
    assert_eq!(s["initiators"][0]["usedPartIndexes"], json!([1, 3]));
    assert!(s["initiators"][1].get("usedPartIndexes").is_none());
    let list = api.get(&owner, "/legacy-kits").await.json();
    assert_eq!(
        list["kits"][0]["activeRecoverySession"]["id"],
        session_id.as_str()
    );
    let r = api
        .post(
            &owner,
            "/legacy-kits/update-recovery-notice",
            json!({"kitID": kit.id, "noticePeriodInHours": 24}),
        )
        .await;
    assert_eq!(r.json()["code"], "ACTIVE_RECOVERY_SESSION");

    // Session lookups.
    let sess = json!({"sessionID": session_id, "sessionToken": token});
    let r = api
        .public_post("/legacy-kits/recovery/session", sess.clone())
        .await;
    assert_eq!(r.json()["status"], "WAITING");
    let r = api
        .public_post(
            "/legacy-kits/recovery/session",
            json!({"sessionID": session_id, "sessionToken": "wrong"}),
        )
        .await;
    assert_eq!(r.status, 404);
    let r = api
        .public_post("/legacy-kits/recovery/info", sess.clone())
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["message"], "legacy kit recovery is not ready");

    // Notice period over (via DB) → READY on next use.
    sqlx::query("UPDATE legacy_kit_recovery_session SET wait_till = $1 WHERE id = $2")
        .bind(now_micros() - 1)
        .bind(&session_id)
        .execute(&srv.state.db.pool)
        .await
        .unwrap();
    let r = api
        .public_post("/legacy-kits/recovery/session", sess.clone())
        .await;
    assert_eq!(r.json()["status"], "READY");
    assert_eq!(r.json()["waitTill"], 0);
    let info = api
        .public_post("/legacy-kits/recovery/info", sess.clone())
        .await
        .json();
    assert_eq!(info["encryptedRecoveryBlob"], "recovery-blob");
    assert_eq!(info["userKeyAttr"]["opsLimit"], 16);

    // Password reset.
    let srp = SrpClient::new(b"heir-chosen-password");
    let r = api
        .public_post(
            "/legacy-kits/recovery/init-change-password",
            json!({"sessionID": session_id, "sessionToken": token, "setupSRPRequest": srp.setup_request()}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let setup = r.json();
    let (update, m2) = srp.update_request(&setup);
    // A token for another session cannot complete it.
    let r = api
        .public_post(
            "/legacy-kits/recovery/change-password",
            json!({"sessionID": session_id, "sessionToken": "bogus", "updateSrpAndKeysRequest": update}),
        )
        .await;
    assert_eq!(r.status, 404);
    let r = api
        .public_post(
            "/legacy-kits/recovery/change-password",
            json!({"sessionID": session_id, "sessionToken": token2, "updateSrpAndKeysRequest": update}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["srpM2"], m2.as_str());
    assert_eq!(live_tokens(&srv, owner.id).await, 0);
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM legacy_kit_recovery_session WHERE id = $1")
            .bind(&session_id)
            .fetch_one(&srv.state.db.pool)
            .await
            .unwrap();
    assert_eq!(status, "RECOVERED");
    let (srp_user,): (String,) =
        sqlx::query_as("SELECT srp_user_id FROM srp_auth WHERE user_id = $1")
            .bind(owner.id)
            .fetch_one(&srv.state.db.pool)
            .await
            .unwrap();
    assert_eq!(srp_user, srp.srp_user_id);
    let r = api
        .public_post("/legacy-kits/recovery/info", sess.clone())
        .await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn legacy_kit_block_delete_limit_and_expiry() {
    let srv = TestServer::start().await.unwrap();
    let api = Api::new(&srv);
    let owner = create_user(&srv, "o@example.com", PHOTOS).await;
    set_key_attributes(&srv, owner.id).await;

    // Immediate kit: session is READY at once.
    let kit = new_kit();
    assert_eq!(
        api.post(&owner, "/legacy-kits", kit_body(&kit, 0))
            .await
            .status,
        200
    );
    let ch = solve(&api, &kit).await;
    let opened = api
        .public_post(
            "/legacy-kits/recovery/open",
            json!({"kitID": kit.id, "challenge": ch}),
        )
        .await
        .json();
    assert_eq!(opened["session"]["status"], "READY");
    let sid = opened["session"]["id"].as_str().unwrap().to_string();
    let token = opened["sessionToken"].as_str().unwrap().to_string();
    let sess = json!({"sessionID": sid, "sessionToken": token});
    assert_eq!(
        api.public_post("/legacy-kits/recovery/info", sess.clone())
            .await
            .status,
        200
    );

    // Owner blocks it; a second block finds nothing.
    assert_eq!(
        api.post(
            &owner,
            "/legacy-kits/block-recovery",
            json!({"kitID": kit.id})
        )
        .await
        .status,
        200
    );
    assert_eq!(
        api.post(
            &owner,
            "/legacy-kits/block-recovery",
            json!({"kitID": kit.id})
        )
        .await
        .status,
        404
    );
    let r = api
        .public_post("/legacy-kits/recovery/session", sess.clone())
        .await;
    assert_eq!(r.json()["status"], "BLOCKED");
    assert_eq!(
        api.public_post("/legacy-kits/recovery/info", sess.clone())
            .await
            .status,
        400
    );

    // The kit stays usable: a new session can be opened; tokens expire after 24h.
    let ch = solve(&api, &kit).await;
    let opened = api
        .public_post(
            "/legacy-kits/recovery/open",
            json!({"kitID": kit.id, "challenge": ch}),
        )
        .await
        .json();
    let sid2 = opened["session"]["id"].as_str().unwrap().to_string();
    assert_ne!(sid, sid2);
    sqlx::query(
        "UPDATE legacy_kit_recovery_session_token SET created_at = $1 WHERE session_id = $2",
    )
    .bind(now_micros() - 25 * 3_600_000_000)
    .bind(&sid2)
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    let r = api
        .public_post(
            "/legacy-kits/recovery/session",
            json!({"sessionID": sid2, "sessionToken": opened["sessionToken"]}),
        )
        .await;
    assert_eq!(r.status, 410);
    assert_eq!(r.json()["code"], "SESSION_EXPIRED");

    // Deleting blocks the active session and hides the kit.
    assert_eq!(
        api.delete(&owner, &format!("/legacy-kits/{}", kit.id))
            .await
            .status,
        200
    );
    assert_eq!(
        api.delete(&owner, &format!("/legacy-kits/{}", kit.id))
            .await
            .status,
        404
    );
    assert_eq!(
        api.get(&owner, &format!("/legacy-kits/{}/download", kit.id))
            .await
            .status,
        404
    );
    assert_eq!(
        api.public_post("/legacy-kits/recovery/challenge", json!({"kitID": kit.id}))
            .await
            .status,
        404
    );
    let (status,): (String,) =
        sqlx::query_as("SELECT status FROM legacy_kit_recovery_session WHERE id = $1")
            .bind(&sid2)
            .fetch_one(&srv.state.db.pool)
            .await
            .unwrap();
    assert_eq!(status, "BLOCKED");

    // At most five active kits.
    for _ in 0..5 {
        assert_eq!(
            api.post(&owner, "/legacy-kits", kit_body(&new_kit(), 720))
                .await
                .status,
            200
        );
    }
    let r = api
        .post(&owner, "/legacy-kits", kit_body(&new_kit(), 720))
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["message"], "legacy kit limit reached");
    let list = api.get(&owner, "/legacy-kits").await.json();
    let kits = list["kits"].as_array().unwrap();
    assert_eq!(kits.len(), 5);
    assert!(kits[0]["createdAt"].as_i64() >= kits[4]["createdAt"].as_i64());
}
