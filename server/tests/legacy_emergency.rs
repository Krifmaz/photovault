//! Trusted contacts: invite/accept, the full recovery flow with the notice
//! period shortened via direct DB time edits, reject/approve/stop, revocation.

#[path = "legacy_support/mod.rs"]
mod support;

use photovault_server::db::now_micros;
use photovault_server::testing::TestServer;
use serde_json::json;
use support::*;

async fn recovery_status(srv: &TestServer, id: &str) -> String {
    let (s,): (String,) = sqlx::query_as("SELECT status FROM emergency_recovery WHERE id = $1")
        .bind(id)
        .fetch_one(&srv.state.db.pool)
        .await
        .unwrap();
    s
}

#[tokio::test]
async fn trusted_contact_full_recovery_flow() {
    let srv = TestServer::start().await.unwrap();
    let api = Api::new(&srv);
    let owner = create_user(&srv, "Owner@Example.com", PHOTOS).await;
    let trusted = create_user(&srv, "trusted@example.com", PHOTOS).await;
    let other = create_user(&srv, "other@example.com", PHOTOS).await;
    set_key_attributes(&srv, owner.id).await;
    let owner_second_session = add_token(&srv, owner.id, PHOTOS).await;
    assert_eq!(live_tokens(&srv, owner.id).await, 2);

    // Unknown email → 404; self → 400.
    let r = api
        .post(
            &owner,
            "/emergency-contacts/add",
            json!({"email": "nobody@example.com", "encryptedKey": "ek"}),
        )
        .await;
    assert_eq!(r.status, 404);
    let r = api
        .post(
            &owner,
            "/emergency-contacts/add",
            json!({"email": "owner@example.com", "encryptedKey": "ek"}),
        )
        .await;
    assert_eq!(r.status, 400);
    // Missing encryptedKey → 400 {}.
    let r = api
        .post(
            &owner,
            "/emergency-contacts/add",
            json!({"email": trusted.email}),
        )
        .await;
    assert_eq!(r.status, 400);

    // Email lookup is case/whitespace-insensitive.
    let r = api
        .post(
            &owner,
            "/emergency-contacts/add",
            json!({"email": "  TRUSTED@example.com ", "encryptedKey": "sealed-recovery-key", "recoveryNoticeInDays": 7}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert!(r.body.is_empty());

    let info = api.get(&owner, "/emergency-contacts/info").await.json();
    assert_eq!(info["contacts"].as_array().unwrap().len(), 1);
    assert_eq!(info["contacts"][0]["state"], "INVITED");
    assert_eq!(info["contacts"][0]["recoveryNoticeInDays"], 7);
    assert_eq!(info["contacts"][0]["user"]["email"], "Owner@Example.com");
    assert_eq!(info["contacts"][0]["emergencyContact"]["id"], trusted.id);
    assert_eq!(info["recoverSessions"], json!([]));
    assert_eq!(info["othersEmergencyContact"], json!([]));
    assert_eq!(info["othersRecoverySession"], json!([]));
    let tinfo = api.get(&trusted, "/emergency-contacts/info").await.json();
    assert_eq!(tinfo["othersEmergencyContact"][0]["user"]["id"], owner.id);

    // State machine validation.
    let upd =
        |state: &str| json!({"userID": owner.id, "emergencyContactID": trusted.id, "state": state});
    let r = api
        .post(&owner, "/emergency-contacts/update", upd("ACCEPTED"))
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "BAD_REQUEST");
    let r = api
        .post(&other, "/emergency-contacts/update", upd("ACCEPTED"))
        .await;
    assert_eq!(r.status, 403);
    let r = api
        .post(
            &trusted,
            "/emergency-contacts/update",
            json!({"userID": owner.id}),
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "BAD_REQUEST");

    // Recovery cannot start before acceptance.
    let ident = json!({"userID": owner.id, "emergencyContactID": trusted.id});
    let r = api
        .post(
            &trusted,
            "/emergency-contacts/start-recovery",
            ident.clone(),
        )
        .await;
    assert_eq!(r.status, 404);

    let r = api
        .post(&trusted, "/emergency-contacts/update", upd("ACCEPTED"))
        .await;
    assert_eq!((r.status, r.json()), (200, json!({})));
    let info = api.get(&owner, "/emergency-contacts/info").await.json();
    assert_eq!(info["contacts"][0]["state"], "ACCEPTED");

    // Notice period can be changed while no recovery is active.
    let r = api
        .post(
            &owner,
            "/emergency-contacts/update-recovery-notice",
            json!({"emergencyContactID": trusted.id, "recoveryNoticeInDays": 61}),
        )
        .await;
    assert_eq!(r.status, 400);
    let r = api
        .post(
            &owner,
            "/emergency-contacts/update-recovery-notice",
            json!({"emergencyContactID": trusted.id, "recoveryNoticeInDays": 14}),
        )
        .await;
    assert_eq!(r.status, 200);

    // Only the trusted contact starts a recovery.
    let r = api
        .post(&owner, "/emergency-contacts/start-recovery", ident.clone())
        .await;
    assert_eq!(r.status, 403);
    let r = api
        .post(
            &trusted,
            "/emergency-contacts/start-recovery",
            ident.clone(),
        )
        .await;
    assert_eq!((r.status, r.json()), (200, json!({})));
    // Starting again is a no-op.
    let r = api
        .post(
            &trusted,
            "/emergency-contacts/start-recovery",
            ident.clone(),
        )
        .await;
    assert_eq!(r.status, 200);

    let info = api.get(&owner, "/emergency-contacts/info").await.json();
    let sessions = info["recoverSessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["status"], "WAITING");
    let wait = sessions[0]["waitTill"].as_i64().unwrap();
    assert!(
        wait > 13 * 86_400_000_000 && wait <= 14 * 86_400_000_000,
        "{wait}"
    );
    let session_id = sessions[0]["id"].as_str().unwrap().to_string();
    let tinfo = api.get(&trusted, "/emergency-contacts/info").await.json();
    assert_eq!(tinfo["othersRecoverySession"][0]["id"], session_id.as_str());

    let r = api
        .post(
            &owner,
            "/emergency-contacts/update-recovery-notice",
            json!({"emergencyContactID": trusted.id, "recoveryNoticeInDays": 3}),
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "ACTIVE_RECOVERY_SESSION");

    // Too early: recovery info is refused; others cannot see it at all.
    let path = format!("/emergency-contacts/recovery-info/{session_id}");
    let r = api.get(&trusted, &path).await;
    assert_eq!(r.status, 400);
    assert_eq!(r.json()["code"], "BAD_REQUEST");
    assert_eq!(api.get(&other, &path).await.status, 403);
    assert_eq!(
        api.get(&trusted, "/emergency-contacts/recovery-info/not-a-uuid")
            .await
            .status,
        400
    );

    // The background reminder job leaves an unfinished wait alone…
    photovault_server::legacy::emergency::run_reminders_once(&srv.state)
        .await
        .unwrap();
    assert_eq!(recovery_status(&srv, &session_id).await, "WAITING");

    // …and promotes it once the notice period is over (shortened via the DB).
    sqlx::query(
        "UPDATE emergency_recovery SET wait_till = $1, next_reminder_at = $2 WHERE id = $3",
    )
    .bind(now_micros() - 1_000)
    .bind(now_micros() - 2_000)
    .bind(&session_id)
    .execute(&srv.state.db.pool)
    .await
    .unwrap();
    photovault_server::legacy::emergency::run_reminders_once(&srv.state)
        .await
        .unwrap();
    assert_eq!(recovery_status(&srv, &session_id).await, "READY");
    let info = api.get(&owner, "/emergency-contacts/info").await.json();
    assert_eq!(info["recoverSessions"][0]["status"], "READY");
    assert_eq!(info["recoverSessions"][0]["waitTill"], 0);

    let r = api.get(&trusted, &path).await;
    assert_eq!(r.status, 200);
    let ri = r.json();
    assert_eq!(ri["encryptedKey"], "sealed-recovery-key");
    assert_eq!(ri["userKeyAttr"]["memLimit"], 268_435_456_i64);
    assert!(ri["userKeyAttr"]["publicKey"].as_str().unwrap().len() == 44);

    // Password reset on behalf of the owner.
    let srp = SrpClient::new(b"new-login-subkey");
    let r = api
        .post(
            &trusted,
            "/emergency-contacts/init-change-password",
            json!({"recoveryID": session_id, "setupSRPRequest": srp.setup_request()}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let setup = r.json();
    assert!(setup["setupID"].is_string());
    let (update, expected_m2) = srp.update_request(&setup);

    // A wrong proof is rejected (and uses up an attempt).
    let mut bad = update.clone();
    bad["srpM1"] = json!(photovault_server::crypto::b64(&[0u8; 32]));
    let r = api
        .post(
            &trusted,
            "/emergency-contacts/change-password",
            json!({"recoveryID": session_id, "updateSrpAndKeysRequest": bad}),
        )
        .await;
    assert_eq!(r.status, 401);
    // Bad KDF parameters are rejected before SRP verification.
    let mut weak = update.clone();
    weak["updatedKeyAttr"]["opsLimit"] = json!(1);
    let r = api
        .post(
            &trusted,
            "/emergency-contacts/change-password",
            json!({"recoveryID": session_id, "updateSrpAndKeysRequest": weak}),
        )
        .await;
    assert_eq!(r.status, 400);
    // The owner cannot use the trusted-contact endpoint.
    let r = api
        .post(
            &owner,
            "/emergency-contacts/change-password",
            json!({"recoveryID": session_id, "updateSrpAndKeysRequest": update}),
        )
        .await;
    assert_eq!(r.status, 403);

    let r = api
        .post(
            &trusted,
            "/emergency-contacts/change-password",
            json!({"recoveryID": session_id, "updateSrpAndKeysRequest": update}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let resp = r.json();
    assert_eq!(resp["srpM2"], expected_m2.as_str());
    assert_eq!(resp["setupID"], setup["setupID"]);

    // New SRP + key attributes stored, every owner session revoked, session closed.
    let (srp_user_id, verifier): (String, String) =
        sqlx::query_as("SELECT srp_user_id, verifier FROM srp_auth WHERE user_id = $1")
            .bind(owner.id)
            .fetch_one(&srv.state.db.pool)
            .await
            .unwrap();
    assert_eq!(srp_user_id, srp.srp_user_id);
    assert_eq!(verifier, photovault_server::crypto::b64(&srp.verifier));
    let (kek_salt, mem): (String, i64) =
        sqlx::query_as("SELECT kek_salt, mem_limit FROM key_attributes WHERE user_id = $1")
            .bind(owner.id)
            .fetch_one(&srv.state.db.pool)
            .await
            .unwrap();
    assert_eq!(
        kek_salt,
        update["updatedKeyAttr"]["kekSalt"].as_str().unwrap()
    );
    assert_eq!(mem, 1_073_741_824);
    assert_eq!(live_tokens(&srv, owner.id).await, 0);
    let _ = owner_second_session;
    assert_eq!(live_tokens(&srv, trusted.id).await, 1);
    assert_eq!(recovery_status(&srv, &session_id).await, "RECOVERED");
    assert_eq!(
        api.get(&owner, "/emergency-contacts/info").await.status,
        401
    );

    // The completed session cannot be reused.
    assert_eq!(api.get(&trusted, &path).await.status, 400);
}

#[tokio::test]
async fn reject_approve_stop_and_revoke() {
    let srv = TestServer::start().await.unwrap();
    let api = Api::new(&srv);
    let owner = create_user(&srv, "a@example.com", PHOTOS).await;
    let trusted = create_user(&srv, "b@example.com", PHOTOS).await;
    set_key_attributes(&srv, owner.id).await;

    // A notice period of one day is too short to start a recovery.
    api.post(
        &owner,
        "/emergency-contacts/add",
        json!({"email": trusted.email, "encryptedKey": "k", "recoveryNoticeInDays": 1}),
    )
    .await;
    let upd =
        |state: &str| json!({"userID": owner.id, "emergencyContactID": trusted.id, "state": state});
    assert_eq!(
        api.post(&trusted, "/emergency-contacts/update", upd("ACCEPTED"))
            .await
            .status,
        200
    );
    let ident = json!({"userID": owner.id, "emergencyContactID": trusted.id});
    let r = api
        .post(
            &trusted,
            "/emergency-contacts/start-recovery",
            ident.clone(),
        )
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(
        api.post(
            &owner,
            "/emergency-contacts/update-recovery-notice",
            json!({"emergencyContactID": trusted.id, "recoveryNoticeInDays": 30})
        )
        .await
        .status,
        200
    );

    let session = |info: serde_json::Value| {
        info["recoverSessions"][0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };

    // Reject: owner only; afterwards stop/reject conflict.
    assert_eq!(
        api.post(
            &trusted,
            "/emergency-contacts/start-recovery",
            ident.clone()
        )
        .await
        .status,
        200
    );
    let id = session(api.get(&owner, "/emergency-contacts/info").await.json());
    let rid = json!({"id": id, "userID": owner.id, "emergencyContactID": trusted.id});
    assert_eq!(
        api.post(&trusted, "/emergency-contacts/reject-recovery", rid.clone())
            .await
            .status,
        403
    );
    let r = api
        .post(&owner, "/emergency-contacts/reject-recovery", rid.clone())
        .await;
    assert_eq!((r.status, r.json()), (200, json!({})));
    assert_eq!(recovery_status(&srv, &id).await, "REJECTED");
    let r = api
        .post(&trusted, "/emergency-contacts/stop-recovery", rid.clone())
        .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.json()["code"], "CONFLICT");
    // Mismatched identifiers are refused.
    let wrong = json!({"id": id, "userID": owner.id, "emergencyContactID": owner.id + 1000});
    assert_eq!(
        api.post(&owner, "/emergency-contacts/reject-recovery", wrong)
            .await
            .status,
        403
    );

    // Approve: the owner skips the notice period.
    assert_eq!(
        api.post(
            &trusted,
            "/emergency-contacts/start-recovery",
            ident.clone()
        )
        .await
        .status,
        200
    );
    let id2 = session(api.get(&owner, "/emergency-contacts/info").await.json());
    assert_ne!(id, id2);
    let rid2 = json!({"id": id2, "userID": owner.id, "emergencyContactID": trusted.id});
    assert_eq!(
        api.post(
            &trusted,
            "/emergency-contacts/approve-recovery",
            rid2.clone()
        )
        .await
        .status,
        403
    );
    assert_eq!(
        api.post(&owner, "/emergency-contacts/approve-recovery", rid2.clone())
            .await
            .status,
        200
    );
    assert_eq!(recovery_status(&srv, &id2).await, "READY");
    assert_eq!(
        api.get(
            &trusted,
            &format!("/emergency-contacts/recovery-info/{id2}")
        )
        .await
        .status,
        200
    );
    // Stop: the trusted contact only.
    assert_eq!(
        api.post(&owner, "/emergency-contacts/stop-recovery", rid2.clone())
            .await
            .status,
        403
    );
    assert_eq!(
        api.post(&trusted, "/emergency-contacts/stop-recovery", rid2.clone())
            .await
            .status,
        200
    );
    assert_eq!(recovery_status(&srv, &id2).await, "STOPPED");
    assert_eq!(
        api.get(
            &trusted,
            &format!("/emergency-contacts/recovery-info/{id2}")
        )
        .await
        .status,
        400
    );

    // Revoking the contact rejects an active recovery and drops the key.
    assert_eq!(
        api.post(
            &trusted,
            "/emergency-contacts/start-recovery",
            ident.clone()
        )
        .await
        .status,
        200
    );
    let id3 = session(api.get(&owner, "/emergency-contacts/info").await.json());
    assert_eq!(
        api.post(&owner, "/emergency-contacts/update", upd("REVOKED"))
            .await
            .status,
        200
    );
    assert_eq!(recovery_status(&srv, &id3).await, "REJECTED");
    let (key,): (Option<String>,) = sqlx::query_as(
        "SELECT encrypted_key FROM emergency_contact WHERE user_id = $1 AND emergency_contact_id = $2",
    )
    .bind(owner.id)
    .bind(trusted.id)
    .fetch_one(&srv.state.db.pool)
    .await
    .unwrap();
    assert_eq!(key, None);
    let info = api.get(&owner, "/emergency-contacts/info").await.json();
    assert_eq!(info["contacts"], json!([]));
    assert_eq!(
        api.post(
            &trusted,
            "/emergency-contacts/start-recovery",
            ident.clone()
        )
        .await
        .status,
        404
    );

    // A revoked contact can be invited again; leaving works from the contact side.
    let r = api
        .post(
            &owner,
            "/emergency-contacts/add",
            json!({"email": trusted.email, "encryptedKey": "k2"}),
        )
        .await;
    assert_eq!(r.status, 200);
    let info = api.get(&owner, "/emergency-contacts/info").await.json();
    assert_eq!(info["contacts"][0]["state"], "INVITED");
    assert_eq!(info["contacts"][0]["recoveryNoticeInDays"], 30);
    assert_eq!(
        api.post(
            &trusted,
            "/emergency-contacts/update",
            upd("CONTACT_DENIED")
        )
        .await
        .status,
        200
    );
    let info = api.get(&owner, "/emergency-contacts/info").await.json();
    assert_eq!(info["contacts"], json!([]));

    // Account deletion cleanup helper for the users workstream.
    api.post(
        &owner,
        "/emergency-contacts/add",
        json!({"email": trusted.email, "encryptedKey": "k3"}),
    )
    .await;
    photovault_server::legacy::emergency::handle_account_deletion(&srv.state, owner.id)
        .await
        .unwrap();
    let (state,): (String,) = sqlx::query_as(
        "SELECT state FROM emergency_contact WHERE user_id = $1 AND emergency_contact_id = $2",
    )
    .bind(owner.id)
    .bind(trusted.id)
    .fetch_one(&srv.state.db.pool)
    .await
    .unwrap();
    assert_eq!(state, "REVOKED");

    // Unauthenticated calls are refused.
    let r = api
        .call(None, "GET", "/emergency-contacts/info", None)
        .await;
    assert_eq!(r.status, 401);
}
