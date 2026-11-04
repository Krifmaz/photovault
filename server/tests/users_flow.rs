//! End-to-end account flow as the apps drive it: signup → key attributes → SRP setup →
//! SRP login → token use → TOTP 2FA (verify, recover/remove) → sessions → logout → delete →
//! self-service recovery. Runs on SQLite, or Postgres with `PHOTOVAULT_TEST_DATABASE_URL`.

mod users_common;

use photovault_server::crypto;
use photovault_server::users::totp;
use serde_json::json;
use users_common::*;

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test]
async fn full_account_flow() {
    let email = "alice@example.org";
    let srv = server(&[email]).await;
    let c = Client::new(&srv);

    // Login before signup is refused with museum's codes.
    let (s, v) = c
        .post("/users/ott", json!({"email": email, "purpose": "login"}))
        .await;
    assert_eq!((s, v["code"].as_str()), (404, Some("USER_NOT_REGISTERED")));
    let (s, _) = c
        .post("/users/ott", json!({"email": "", "purpose": "signup"}))
        .await;
    assert_eq!(s, 400);

    let acc = signup(&c, email).await;
    // Signup-incomplete → complete: signup OTT is now USER_ALREADY_REGISTERED.
    let (s, v) = c
        .post("/users/ott", json!({"email": email, "purpose": "signup"}))
        .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (409, Some("USER_ALREADY_REGISTERED"))
    );
    // Key attributes cannot be replaced.
    let (s, _) = c
        .with_token(&acc.token)
        .put(
            "/users/attributes",
            json!({"keyAttributes": acc.key_attributes}),
        )
        .await;
    assert_eq!(s, 403);

    // SRP attributes.
    let (s, v) = c.get(&format!("/users/srp/attributes?email={email}")).await;
    assert_eq!(s, 200);
    assert_eq!(v["attributes"]["srpUserID"], acc.srp_user_id);
    assert_eq!(v["attributes"]["memLimit"], 268435456);
    assert_eq!(v["attributes"]["kekSalt"], acc.key_attributes["kekSalt"]);
    assert_eq!(v["attributes"]["isEmailMFAEnabled"], false);
    let (s, _) = c
        .get("/users/srp/attributes?email=nobody@example.org")
        .await;
    assert_eq!(s, 404);

    // SRP login with a wrong password fails, the right one yields a sealed token.
    let (s, _) = srp_login_with(&c, email, b"wrong-password-k").await;
    assert_eq!(s, 401);
    let (s, v) = srp_login(&c, &acc).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["id"], acc.id);
    assert_eq!(
        v["keyAttributes"]["publicKey"],
        acc.key_attributes["publicKey"]
    );
    assert_eq!(v["twoFactorSessionID"], "");
    assert_eq!(v["passkeySessionID"], "");
    assert!(v.get("token").is_none());
    let t1 = open_token(
        &acc.public_key,
        &acc.secret_key,
        v["encryptedToken"].as_str().unwrap(),
    );
    let u1 = c.with_token(&t1);
    let (s, v) = u1.get("/users/session-validity/v2").await;
    assert_eq!(s, 200);
    assert_eq!(v["hasSetKeys"], true);
    assert_eq!(
        v["keyAttributes"]["encryptedKey"],
        acc.key_attributes["encryptedKey"]
    );

    // Email OTT login also works and returns the same shape.
    let (s, _) = c
        .post("/users/ott", json!({"email": email, "purpose": "login"}))
        .await;
    assert_eq!(s, 200);
    let (s, v) = c
        .post(
            "/users/verify-email",
            json!({"email": email, "ott": "000000"}),
        )
        .await;
    assert_eq!(s, 401, "{v}");
    let (s, v) = c
        .post("/users/verify-email", json!({"email": email, "ott": OTT}))
        .await;
    assert_eq!(s, 200, "{v}");
    let t_ott = open_token(
        &acc.public_key,
        &acc.secret_key,
        v["encryptedToken"].as_str().unwrap(),
    );

    // Details.
    let (s, v) = u1.get("/users/details/v2").await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["email"], email);
    assert_eq!(v["usage"], 0);
    assert_eq!(v["subscription"]["productID"], "free");
    // Real numbers: no quota by default, so the limit is used + free disk space.
    let storage = v["subscription"]["storage"].as_i64().unwrap();
    assert!(
        storage > 0 && storage != photovault_server::capacity::UNLIMITED,
        "{v}"
    );
    assert_eq!(v["profileData"]["canDisableEmailMFA"], true);
    assert_eq!(v["profileData"]["isTwoFactorEnabled"], false);
    assert_eq!(v["bonusData"]["storageBonuses"], json!([]));
    assert_eq!(v["fileCount"], 0);

    // Public keys.
    let (s, v) = u1.get(&format!("/users/public-key?email={email}")).await;
    assert_eq!(
        (s, v["publicKey"].as_str()),
        (200, acc.key_attributes["publicKey"].as_str())
    );
    let (s, _) = u1.get("/users/public-key?email=nobody@example.org").await;
    assert_eq!(s, 404);
    let (s, v) = u1
        .post("/users/public-keys", json!({"emails": [email]}))
        .await;
    assert_eq!(s, 200);
    assert_eq!(v["publicKeys"][0], acc.key_attributes["publicKey"]);

    // TOTP: setup → enable.
    let (s, v) = u1.post("/users/two-factor/setup", json!({})).await;
    assert_eq!(s, 200, "{v}");
    let secret = v["secretCode"].as_str().unwrap().to_string();
    assert!(!v["qrCode"].as_str().unwrap().is_empty());
    let (s, _) = u1
        .post(
            "/users/two-factor/enable",
            json!({"code": "000000", "encryptedTwoFactorSecret": crypto::b64(&[1u8; 40]), "twoFactorSecretDecryptionNonce": crypto::b64(&[2u8; 24])}),
        )
        .await;
    assert_eq!(s, 401);
    let code = totp::code_at(&secret, now_secs()).unwrap();
    let recovery_ct = crypto::b64(&[3u8; 40]);
    let recovery_nonce = crypto::b64(&[4u8; 24]);
    let (s, v) = u1
        .post(
            "/users/two-factor/enable",
            json!({"code": code, "encryptedTwoFactorSecret": recovery_ct, "twoFactorSecretDecryptionNonce": recovery_nonce}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (_, v) = u1.get("/users/two-factor/status").await;
    assert_eq!(v["status"], true);

    // SRP login now needs the second factor.
    let (s, v) = srp_login(&c, &acc).await;
    assert_eq!(s, 200);
    assert!(v.get("encryptedToken").is_none());
    let sid = v["twoFactorSessionID"].as_str().unwrap().to_string();
    assert!(!sid.is_empty());
    let (s, _) = c
        .post(
            "/users/two-factor/verify",
            json!({"sessionID": sid, "code": "000000"}),
        )
        .await;
    assert_eq!(s, 401);
    let (s, v) = c
        .post(
            "/users/two-factor/verify",
            json!({"sessionID": sid, "code": code}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let t2 = open_token(
        &acc.public_key,
        &acc.secret_key,
        v["encryptedToken"].as_str().unwrap(),
    );
    assert_eq!(
        c.with_token(&t2).get("/users/two-factor/status").await.0,
        200
    );
    // The session is consumed, and the code cannot be replayed.
    let (s, _) = c
        .post(
            "/users/two-factor/verify",
            json!({"sessionID": sid, "code": code}),
        )
        .await;
    assert_eq!(s, 404);
    let (_, v) = srp_login(&c, &acc).await;
    let sid2 = v["twoFactorSessionID"].as_str().unwrap().to_string();
    let (s, _) = c
        .post(
            "/users/two-factor/verify",
            json!({"sessionID": sid2, "code": code}),
        )
        .await;
    assert_eq!(s, 401, "replayed TOTP must be refused");

    // Lost authenticator: recover → remove with the decrypted secret.
    let (s, v) = c
        .get(&format!(
            "/users/two-factor/recover?sessionID={sid2}&twoFactorType=totp"
        ))
        .await;
    assert_eq!(s, 200);
    assert_eq!(v["encryptedSecret"], recovery_ct);
    assert_eq!(v["secretDecryptionNonce"], recovery_nonce);
    let (s, _) = c
        .post(
            "/users/two-factor/remove",
            json!({"sessionID": sid2, "secret": "WRONG", "twoFactorType": "totp"}),
        )
        .await;
    assert_eq!(s, 403);
    let (s, _) = c
        .post(
            "/users/two-factor/remove",
            json!({"sessionID": sid2, "secret": secret, "proof": "x"}),
        )
        .await;
    assert_eq!(s, 400);
    let (s, v) = c
        .post(
            "/users/two-factor/remove",
            json!({"sessionID": sid2, "secret": secret, "twoFactorType": "totp"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["encryptedToken"].is_string());
    let (_, v) = u1.get("/users/two-factor/status").await;
    assert_eq!(v["status"], false);

    // Sessions: list, terminate another, refuse terminating the current one.
    let (s, v) = u1.get("/users/sessions").await;
    assert_eq!(s, 200);
    let sessions = v["sessions"].as_array().unwrap().clone();
    assert!(sessions.len() >= 4, "{v}");
    let current: Vec<_> = sessions.iter().filter(|s| s["isCurrent"] == true).collect();
    assert_eq!(current.len(), 1);
    assert!(current[0]["token"].as_str().unwrap().starts_with("th:"));
    assert_eq!(current[0]["prettyUA"], "Chrome, Linux");
    let (s, v) = u1
        .delete(
            &format!(
                "/users/session?token={}",
                current[0]["token"].as_str().unwrap()
            ),
            None,
        )
        .await;
    assert_eq!((s, v["code"].as_str()), (400, Some("BAD_REQUEST")));
    let ott_session = format!("th:{}", {
        use sha2::Digest;
        photovault_server::users::webauthn::encode_b64url(&sha2::Sha256::digest(t_ott.as_bytes()))
    });
    let (s, _) = u1
        .delete(&format!("/users/session?token={ott_session}"), None)
        .await;
    assert_eq!(s, 200);
    assert_eq!(
        c.with_token(&t_ott)
            .get("/users/session-validity/v2")
            .await
            .0,
        401
    );

    // JWTs for the web apps.
    let (s, v) = u1.get("/users/families-token").await;
    assert_eq!(s, 200);
    assert!(v["familiesToken"].as_str().unwrap().contains('.'));
    let (s, v) = u1.get("/users/accounts-token").await;
    assert_eq!(s, 200);
    assert_eq!(v["accountsUrl"], srv.state.config.accounts.accounts_url);
    let (s, v) = u1.get("/users/payment-token").await;
    assert_eq!(s, 200);
    assert!(v["paymentToken"].is_string());

    // Logout.
    let (s, v) = c.with_token(&t2).post("/users/logout", json!({})).await;
    assert_eq!((s, v), (200, json!({})));
    assert_eq!(c.with_token(&t2).get("/users/details/v2").await.0, 401);

    // Delete: challenge sealed to the user's key, then delete with the JWT inside.
    let (s, v) = u1.get("/users/delete-challenge").await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["allowDelete"], true);
    assert_eq!(v["apps"], json!(["photos"]));
    let opened = crypto::box_seal_open(
        &crypto::unb64(v["encryptedChallenge"].as_str().unwrap()).unwrap(),
        &acc.public_key,
        &acc.secret_key,
    )
    .unwrap();
    let challenge = String::from_utf8(opened).unwrap();
    assert!(challenge.starts_with("delete-jwt"));
    let jwt = challenge.trim_start_matches("delete-jwt");
    let (s, _) = u1
        .delete(
            "/users/delete",
            Some(json!({"challenge": "bogus.jwt.value"})),
        )
        .await;
    assert_ne!(s, 200);
    let (s, v) = u1
        .delete(
            "/users/delete",
            Some(json!({"challenge": jwt, "reason": "testing"})),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(
        v,
        json!({"isSubscriptionCancelled": true, "userID": acc.id})
    );
    assert_eq!(u1.get("/users/details/v2").await.0, 401);
    let (s, v) = c
        .post("/users/ott", json!({"email": email, "purpose": "login"}))
        .await;
    assert_eq!((s, v["code"].as_str()), (404, Some("USER_NOT_REGISTERED")));

    // Self-service recovery with the emailed link.
    let link = photovault_server::users::lifecycle::recovery_link(&srv.state, acc.id, email);
    let token =
        url::form_urlencoded::parse(link.split("recoveryToken=").nth(1).unwrap().as_bytes())
            .next()
            .map(|(k, _)| k.into_owned())
            .unwrap();
    let (s, v) = c
        .post("/users/recover-account/validate", json!({"token": token}))
        .await;
    assert_eq!((s, v["status"].as_str()), (200, Some("ready")), "{v}");
    let (s, v) = c
        .post("/users/recover-account", json!({"token": token}))
        .await;
    assert_eq!((s, v["status"].as_str()), (200, Some("recovered")), "{v}");
    let (s, v) = c
        .post("/users/recover-account/validate", json!({"token": "x.y.z"}))
        .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (400, Some("ACCOUNT_RECOVERY_INVALID_LINK"))
    );
    let (s, v) = srp_login(&c, &acc).await;
    assert_eq!(s, 200, "{v}");
}

#[tokio::test]
async fn signup_disabled_and_email_mfa() {
    let email = "bob@example.org";
    let srv = photovault_server::testing::TestServer::start_with(|c| {
        c.allow_signups = false;
        c.hardcoded_ott.insert("bob@example.org".into(), OTT.into());
    })
    .await
    .unwrap();
    let c = Client::new(&srv);
    let (s, _) = c
        .post("/users/ott", json!({"email": email, "purpose": "signup"}))
        .await;
    assert_eq!(s, 403);
    // An admin-created account can still log in.
    photovault_server::users::create_user(&srv.state, email, None)
        .await
        .unwrap();
    let (s, v) = c
        .post("/users/ott", json!({"email": email, "purpose": "login"}))
        .await;
    assert_eq!(
        (s, v["code"].as_str()),
        (404, Some("USER_SIGNUP_INCOMPLETE"))
    );
    let (s, _) = c.post("/users/ott", json!({"email": email})).await;
    assert_eq!(s, 200);
    let (s, v) = c
        .post("/users/verify-email", json!({"email": email, "ott": OTT}))
        .await;
    assert_eq!(s, 200, "{v}");
    let token = v["token"].as_str().unwrap().to_string();
    let u = c.with_token(&token);
    let (pk, _sk) = crypto::box_keypair();
    let (s, _) = u
        .put(
            "/users/attributes",
            json!({"keyAttributes": key_attributes(&pk)}),
        )
        .await;
    assert_eq!(s, 200);
    // Email MFA cannot be turned off before SRP exists.
    let (s, v) = u.put("/users/email-mfa", json!({"isEnabled": false})).await;
    assert_eq!((s, v["code"].as_str()), (409, Some("CONFLICT")));
    let srp_id = uuid::Uuid::new_v4().to_string();
    let salt = crypto::random_bytes(16);
    let key = crypto::random_bytes(16);
    setup_srp(&u, &srp_id, &salt, &key).await;
    let (s, _) = u.put("/users/email-mfa", json!({"isEnabled": true})).await;
    assert_eq!(s, 200);
    // SRP login is refused while email MFA is on.
    let (s, v) = srp_login_with(&c, email, &key).await;
    assert_eq!((s, v["code"].as_str()), (409, Some("EMAIL_MFA_ENABLED")));
    let (s, v) = c.get(&format!("/users/srp/attributes?email={email}")).await;
    assert_eq!(
        (s, v["attributes"]["isEmailMFAEnabled"].as_bool()),
        (200, Some(true))
    );
}

#[tokio::test]
async fn srp_password_change_logs_out_others() {
    let email = "carol@example.org";
    let srv = server(&[email]).await;
    let c = Client::new(&srv);
    let acc = signup(&c, email).await;
    let (_, v) = srp_login(&c, &acc).await;
    let other = open_token(
        &acc.public_key,
        &acc.secret_key,
        v["encryptedToken"].as_str().unwrap(),
    );
    let me = c.with_token(&acc.token);

    // Change password: new SRP verifier + new KEK-wrapped key.
    let new_key = crypto::random_bytes(16);
    let new_salt = crypto::random_bytes(16);
    let verifier = photovault_server::users::srp::client::verifier(
        acc.srp_user_id.as_bytes(),
        &new_key,
        &new_salt,
    );
    let a = crypto::random_bytes(32);
    let (s, v) = me
        .post(
            "/users/srp/setup",
            json!({
                "srpUserID": acc.srp_user_id,
                "srpSalt": crypto::b64(&new_salt),
                "srpVerifier": crypto::b64(&verifier),
                "srpA": crypto::b64(&photovault_server::users::srp::client::public_a(&a)),
            }),
        )
        .await;
    assert_eq!(s, 200);
    let b = crypto::unb64(v["srpB"].as_str().unwrap()).unwrap();
    let (m1, _) = photovault_server::users::srp::client::proofs(
        acc.srp_user_id.as_bytes(),
        &new_key,
        &new_salt,
        &a,
        &b,
    )
    .unwrap();
    let new_kek_salt = crypto::b64(&crypto::random_bytes(16));
    let (s, v) = me
        .post(
            "/users/srp/update",
            json!({
                "setupID": v["setupID"],
                "srpM1": crypto::b64(&m1),
                "updatedKeyAttr": {
                    "kekSalt": new_kek_salt,
                    "encryptedKey": crypto::b64(&crypto::random_bytes(48)),
                    "keyDecryptionNonce": crypto::b64(&crypto::random_bytes(24)),
                    "memLimit": 1073741824,
                    "opsLimit": 4,
                }
            }),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["srpM2"].is_string());
    assert_eq!(c.with_token(&other).get("/users/details/v2").await.0, 401);
    assert_eq!(me.get("/users/details/v2").await.0, 200);
    let (s, _) = srp_login(&c, &acc).await;
    assert_eq!(s, 401, "old password");
    let (s, v) = srp_login_with(&c, email, &new_key).await;
    assert_eq!(s, 200);
    assert_eq!(v["keyAttributes"]["kekSalt"], new_kek_salt);
    assert_eq!(v["keyAttributes"]["memLimit"], 1073741824);
}

/// The web/desktop apps' Rust client (`accounts::models::UpdateSrpAndKeysRequest`, serde
/// camelCase) sends `setupId`, not `setupID`; museum accepts it because Go's JSON decoder
/// matches field names case-insensitively. Found by the e2e `change_password` test.
#[tokio::test]
async fn srp_update_accepts_rust_client_body() {
    let email = "dave@example.org";
    let srv = server(&[email]).await;
    let c = Client::new(&srv);
    let acc = signup(&c, email).await;
    let (_, v) = srp_login(&c, &acc).await;
    let other = open_token(
        &acc.public_key,
        &acc.secret_key,
        v["encryptedToken"].as_str().unwrap(),
    );
    let me = c.with_token(&acc.token);
    let new_key = crypto::random_bytes(16);
    let new_salt = crypto::random_bytes(16);
    let verifier = photovault_server::users::srp::client::verifier(
        acc.srp_user_id.as_bytes(),
        &new_key,
        &new_salt,
    );
    let a = crypto::random_bytes(32);
    let (s, v) = me
        .post(
            "/users/srp/setup",
            json!({
                "srpUserID": acc.srp_user_id,
                "srpSalt": crypto::b64(&new_salt),
                "srpVerifier": crypto::b64(&verifier),
                "srpA": crypto::b64(&photovault_server::users::srp::client::public_a(&a)),
            }),
        )
        .await;
    assert_eq!(s, 200);
    let setup_id = v["setupID"].clone();
    let b = crypto::unb64(v["srpB"].as_str().unwrap()).unwrap();
    let (m1, _) = photovault_server::users::srp::client::proofs(
        acc.srp_user_id.as_bytes(),
        &new_key,
        &new_salt,
        &a,
        &b,
    )
    .unwrap();
    // Exactly the field names the client serializes.
    let (s, v) = me
        .post(
            "/users/srp/update",
            json!({
                "setupId": setup_id,
                "srpM1": crypto::b64(&m1),
                "updatedKeyAttr": {
                    "kekSalt": crypto::b64(&crypto::random_bytes(16)),
                    "encryptedKey": crypto::b64(&crypto::random_bytes(48)),
                    "keyDecryptionNonce": crypto::b64(&crypto::random_bytes(24)),
                    "opsLimit": 4,
                    "memLimit": 1073741824u64,
                },
                "logOutOtherDevices": false,
            }),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["setupID"], setup_id);
    assert!(v["srpM2"].is_string());
    // logOutOtherDevices=false keeps the other session.
    assert_eq!(c.with_token(&other).get("/users/details/v2").await.0, 200);
    let (s, _) = srp_login_with(&c, email, &new_key).await;
    assert_eq!(s, 200);
}
