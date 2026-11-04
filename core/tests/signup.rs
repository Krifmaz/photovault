//! Create account through `Core` (contract v1.2, "signup") against the real
//! Photovault server in process, then prove the account is an ordinary Ente
//! account: other devices sign in with SRP, a hand-rolled SRP login (no
//! `Core`) gets key attributes that open with Ente's algorithm, and the
//! recovery key the user was shown recovers the account.

mod support;

use photovault_core::api::Api;
use photovault_core::crypto;
use photovault_core::srp::SrpClient;
use photovault_core::{CoreError, LoginStep, fetch_server_info};
use photovault_server::testing::TestServer;
use reqwest::Method;
use serde_json::{Value, json};
use support::{OTT, TestCore, assert_err, jpeg, server};

fn done(step: LoginStep) -> photovault_core::Account {
    match step {
        LoginStep::Done { account } => account,
        other => panic!("expected Done, got {other:?}"),
    }
}

fn b64(v: &Value) -> Vec<u8> {
    crypto::unb64(v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))).unwrap()
}

/// SRP login exactly as Ente's clients do it, without `Core`: returns the
/// server's authorization answer (key attributes, sealed token).
async fn raw_srp_login(base: &str, email: &str, password: &str) -> Value {
    crypto::init();
    let api = Api::new(base);
    let a: Value = api
        .get_json("/users/srp/attributes", &[("email", email.to_string())])
        .await
        .expect("srp attributes");
    let a = &a["attributes"];
    let kek = crypto::derive_argon_key(
        password.as_bytes(),
        &b64(&a["kekSalt"]),
        a["opsLimit"].as_u64().unwrap(),
        a["memLimit"].as_u64().unwrap(),
    )
    .unwrap();
    let login_key = crypto::derive_login_key(&kek).unwrap();
    let id = a["srpUserID"].as_str().unwrap();
    let mut srp = SrpClient::new(id.as_bytes(), &b64(&a["srpSalt"]), &login_key);
    let s: Value = api
        .json(
            Method::POST,
            "/users/srp/create-session",
            &json!({"srpUserID": id, "srpA": crypto::b64(&srp.public_a())}),
        )
        .await
        .expect("create-session");
    let m1 = srp.compute_m1(&b64(&s["srpB"])).unwrap();
    let auth: Value = api
        .json(
            Method::POST,
            "/users/srp/verify-session",
            &json!({"srpUserID": id, "sessionID": s["sessionID"], "srpM1": crypto::b64(&m1)}),
        )
        .await
        .expect("verify-session");
    srp.verify_m2(&b64(&auth["srpM2"])).unwrap();
    auth
}

/// Open key attributes the way Ente's apps do (`decryptAndStoreAttributes`,
/// the same steps as `ente_vectors.rs` checks against Ente's own vectors):
/// password → Argon2id KEK → master key → secret key → sealed token, and the
/// recovery key both ways. Returns the master key.
fn open_like_ente(ka: &Value, encrypted_token: &Value, password: &str, words: &str) -> Vec<u8> {
    let ops = ka["opsLimit"].as_u64().unwrap();
    let mem = ka["memLimit"].as_u64().unwrap();
    let kek =
        crypto::derive_argon_key(password.as_bytes(), &b64(&ka["kekSalt"]), ops, mem).unwrap();
    let master = crypto::secretbox_open(
        &b64(&ka["encryptedKey"]),
        &b64(&ka["keyDecryptionNonce"]),
        &kek,
    )
    .expect("master key opens with the password");
    assert_eq!(master.len(), 32);
    let sk = crypto::secretbox_open(
        &b64(&ka["encryptedSecretKey"]),
        &b64(&ka["secretKeyDecryptionNonce"]),
        &master,
    )
    .expect("secret key opens with the master key");
    let pk = b64(&ka["publicKey"]);
    assert_eq!(crypto::public_key_from_secret(&sk).unwrap(), pk);
    let token = crypto::box_seal_open(&b64(encrypted_token), &pk, &sk).expect("sealed token");
    assert!(!token.is_empty());
    let rk = crypto::parse_recovery_key(words).unwrap();
    assert_eq!(crypto::recovery_key_to_mnemonic(&rk).unwrap(), words);
    let m2 = crypto::secretbox_open(
        &b64(&ka["masterKeyEncryptedWithRecoveryKey"]),
        &b64(&ka["masterKeyDecryptionNonce"]),
        &rk,
    )
    .expect("master key opens with the recovery key");
    assert_eq!(m2, master);
    let rk2 = crypto::secretbox_open(
        &b64(&ka["recoveryKeyEncryptedWithMasterKey"]),
        &b64(&ka["recoveryKeyDecryptionNonce"]),
        &master,
    )
    .expect("recovery key opens with the master key");
    assert_eq!(rk2, rk);
    master
}

/// Ente's required strength for account keys (museum `ExpectedKDFStrength`),
/// never below 128 MiB.
fn assert_sensitive(ka: &Value) {
    let ops = ka["opsLimit"].as_u64().unwrap();
    let mem = ka["memLimit"].as_u64().unwrap();
    assert_eq!(ops * mem, 4 * 1024 * 1024 * 1024, "{ka}");
    assert!(mem >= 128 * 1024 * 1024, "{ka}");
}

/// The checker itself against key attributes and a sealed token made by
/// Ente's own client code (`tests/vectors/keys.json`, `sealed_token.json`).
#[test]
fn checker_opens_entes_own_attributes() {
    let keys: Value = serde_json::from_str(include_str!("vectors/keys.json")).unwrap();
    let token: Value = serde_json::from_str(include_str!("vectors/sealed_token.json")).unwrap();
    assert_eq!(keys["keyAttributes"]["publicKey"], token["publicKey"]);
    let master = open_like_ente(
        &keys["keyAttributes"],
        &token["encryptedToken"],
        keys["password"].as_str().unwrap(),
        keys["plaintext"]["recoveryKeyMnemonic"].as_str().unwrap(),
    );
    assert_eq!(master, b64(&keys["plaintext"]["masterKey"]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_account_then_sign_in_everywhere() {
    const EMAIL: &str = "nora@example.org";
    const PASSWORD: &str = "a long and quite secret passphrase";
    const NEW_PASSWORD: &str = "another passphrase, after recovery";
    let srv = server(&[EMAIL]).await;
    assert!(
        fetch_server_info(srv.url.clone())
            .await
            .unwrap()
            .signups_allowed
    );

    // ---- sign up on device A ----
    let a = TestCore::connected(&srv.url).await;
    a.core.signup_send_code(EMAIL.into()).await.unwrap();
    assert_err(
        a.core
            .signup_verify_code(EMAIL.into(), "000000".into())
            .await,
        CoreError::IncorrectCode,
    );
    // Nothing to prepare before the code is verified.
    assert!(a.core.signup_finish().await.is_err());
    a.core
        .signup_verify_code(EMAIL.into(), format!(" {OTT} "))
        .await
        .unwrap();
    let words = a.core.signup_prepare(PASSWORD.into(), true).await.unwrap();
    assert_eq!(words.split(' ').count(), 24, "{words}");
    // Same password again (the user went back): the same keys.
    assert_eq!(
        a.core.signup_prepare(PASSWORD.into(), true).await.unwrap(),
        words
    );
    // Not signed in until the user confirmed the recovery key.
    assert_eq!(a.core.current_account(), None);
    let account = a.core.signup_finish().await.unwrap();
    assert_eq!(account.email, EMAIL);
    assert_eq!(account.server_url, srv.url);
    assert!(account.user_id > 0);
    assert_eq!(a.core.current_account(), Some(account.clone()));
    assert_eq!(a.core.recovery_key_words().unwrap(), words);
    // The session works: upload a photo.
    let path = a.dir.path().join("first.jpg");
    std::fs::write(&path, jpeg(64, 48, 3)).unwrap();
    a.core
        .enqueue_upload(path.display().to_string(), None)
        .unwrap();
    let st = a.core.run_uploads().await.unwrap();
    assert_eq!((st.pending, st.failed), (0, 0), "{st:?}");
    a.core.sync(None).await.unwrap();
    assert_eq!(a.core.timeline_count(), 1);
    // The session survives a restart of the core.
    drop(a.core);
    let a2 = photovault_core::Core::new(
        a.dir.path().join("data").display().to_string(),
        a.dir.path().join("cache").display().to_string(),
    )
    .unwrap();
    assert_eq!(a2.current_account(), Some(account.clone()));

    // The email is taken now.
    let x = TestCore::connected(&srv.url).await;
    match x.core.signup_send_code(EMAIL.into()).await {
        Err(CoreError::Server { status: 409, .. }) => {}
        other => panic!("expected 409, got {other:?}"),
    }

    // ---- device B signs in with SRP and sees the photo ----
    let b = TestCore::connected(&srv.url).await;
    assert_err(
        b.core.login(EMAIL.into(), "wrong".into()).await,
        CoreError::IncorrectPassword,
    );
    let got = done(b.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap());
    assert_eq!(got.user_id, account.user_id);
    b.core.sync(None).await.unwrap();
    assert_eq!(b.core.timeline_count(), 1);
    let id = b.core.timeline_page(0, 1)[0].id;
    let thumb = b.core.thumbnail(id).await.unwrap();
    assert_eq!(&thumb[..2], &[0xFF, 0xD8]);
    assert_eq!(b.core.recovery_key_words().unwrap(), words);

    // ---- a plain SRP login (no Core) and Ente's decryption steps ----
    let auth = raw_srp_login(&srv.url, EMAIL, PASSWORD).await;
    assert_eq!(auth["id"].as_i64(), Some(account.user_id));
    let ka = &auth["keyAttributes"];
    // Every field Ente's KeyAttributes has, under Ente's names.
    for f in [
        "kekSalt",
        "encryptedKey",
        "keyDecryptionNonce",
        "publicKey",
        "encryptedSecretKey",
        "secretKeyDecryptionNonce",
        "memLimit",
        "opsLimit",
        "masterKeyEncryptedWithRecoveryKey",
        "masterKeyDecryptionNonce",
        "recoveryKeyEncryptedWithMasterKey",
        "recoveryKeyDecryptionNonce",
    ] {
        assert!(!ka[f].is_null(), "missing {f}: {ka}");
    }
    // `low_memory` start: 256 MiB / 16 ops (enough memory here, no fallback).
    assert_eq!(ka["memLimit"].as_u64(), Some(256 * 1024 * 1024));
    assert_eq!(ka["opsLimit"].as_u64(), Some(16));
    assert_sensitive(ka);
    open_like_ente(ka, &auth["encryptedToken"], PASSWORD, &words);

    // ---- the recovery key the user saved recovers the account ----
    let c = TestCore::connected(&srv.url).await;
    c.core.send_email_ott(EMAIL.into()).await.unwrap();
    assert_eq!(
        c.core
            .verify_email_ott(EMAIL.into(), OTT.into(), String::new())
            .await
            .unwrap(),
        LoginStep::NeedsRecoveryKey
    );
    c.core
        .recover_with_key(words.clone(), NEW_PASSWORD.into())
        .await
        .unwrap();
    c.core.sync(None).await.unwrap();
    assert_eq!(c.core.timeline_count(), 1);
    let d = TestCore::connected(&srv.url).await;
    assert_err(
        d.core.login(EMAIL.into(), PASSWORD.into()).await,
        CoreError::IncorrectPassword,
    );
    done(
        d.core
            .login(EMAIL.into(), NEW_PASSWORD.into())
            .await
            .unwrap(),
    );
    let auth = raw_srp_login(&srv.url, EMAIL, NEW_PASSWORD).await;
    assert_sensitive(&auth["keyAttributes"]);
    open_like_ente(
        &auth["keyAttributes"],
        &auth["encryptedToken"],
        NEW_PASSWORD,
        &words,
    );
}

/// Keys from the full-memory start (1 GiB / 4 ops, as on a capable phone).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sensitive_limits() {
    const EMAIL: &str = "omar@example.org";
    const PASSWORD: &str = "omar's own long passphrase";
    let srv = server(&[EMAIL]).await;
    let a = TestCore::connected(&srv.url).await;
    a.core.signup_send_code(EMAIL.into()).await.unwrap();
    a.core
        .signup_verify_code(EMAIL.into(), OTT.into())
        .await
        .unwrap();
    let words = a.core.signup_prepare(PASSWORD.into(), false).await.unwrap();
    a.core.signup_finish().await.unwrap();
    let auth = raw_srp_login(&srv.url, EMAIL, PASSWORD).await;
    let ka = &auth["keyAttributes"];
    // 1 GiB / 4 ops, unless this machine could not spare 1 GiB (then the
    // halving fallback, still 4 GiB of work).
    let (ops, mem) = (
        ka["opsLimit"].as_u64().unwrap(),
        ka["memLimit"].as_u64().unwrap(),
    );
    assert!(
        matches!(
            (ops, mem),
            (4, 1073741824) | (8, 536870912) | (16, 268435456)
        ),
        "{ka}"
    );
    assert_sensitive(ka);
    open_like_ente(ka, &auth["encryptedToken"], PASSWORD, &words);
    done(
        TestCore::connected(&srv.url)
            .await
            .core
            .login(EMAIL.into(), PASSWORD.into())
            .await
            .unwrap(),
    );
}

/// A server that does not accept new accounts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn signups_closed() {
    const EMAIL: &str = "pia@example.org";
    let srv = TestServer::start_with(|c| {
        c.allow_signups = false;
        c.hardcoded_ott.insert(EMAIL.into(), OTT.into());
    })
    .await
    .unwrap();
    let info = fetch_server_info(srv.url.clone()).await.unwrap();
    assert!(!info.signups_allowed, "{info:?}");
    let a = TestCore::connected(&srv.url).await;
    match a.core.signup_send_code(EMAIL.into()).await {
        Err(CoreError::Server { status: 403, .. }) => {}
        other => panic!("expected 403, got {other:?}"),
    }
    // The server refuses too, even if the app skipped the check.
    let api = Api::new(&srv.url);
    let r = api
        .json_no_content(
            Method::POST,
            "/users/ott",
            &json!({"email": EMAIL, "purpose": "signup"}),
        )
        .await;
    assert_eq!(r.unwrap_err().status(), Some(403));
    // No code was sent, so none verifies.
    assert!(
        a.core
            .signup_verify_code(EMAIL.into(), OTT.into())
            .await
            .is_err()
    );
    assert_eq!(a.core.current_account(), None);
}
