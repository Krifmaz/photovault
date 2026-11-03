//! Sign-in against a mock museum that runs the server side of SRP, using an
//! account created by Ente's own sign-up code (see `crypto_ente_vectors.rs`).

use num_bigint_dig::BigUint;
use photovault_node::api::srp;
use photovault_node::api::{
    ApiClient, ApiError, CLIENT_PACKAGE, LoginFlow, LoginStep, RetryPolicy, Session,
};
use photovault_node::crypto;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn account() -> Value {
    let v: Value =
        serde_json::from_str(include_str!("crypto_vectors/ente_core_vectors.json")).unwrap();
    v["account"].clone()
}

const USER_ID: i64 = 1;
const B_SECRET: [u8; 64] = [0x37; 64];

/// Server-side SRP, mirroring museum/go-srp (and `server/src/legacy/srp.rs`).
mod server_srp {
    use super::*;
    fn int(b: &[u8]) -> BigUint {
        BigUint::from_bytes_be(b)
    }
    pub fn compute_b(verifier: &[u8], secret: &[u8]) -> Vec<u8> {
        let n = srp::n();
        srp::pad(&((srp::k() * int(verifier) + srp::g().modpow(&int(secret), n)) % n))
    }
    pub fn verify_m1(verifier: &[u8], secret: &[u8], a: &[u8], m1: &[u8]) -> Option<Vec<u8>> {
        let n = srp::n();
        let a_int = int(a);
        if a_int == BigUint::from(0u32) || &a_int >= n {
            return None;
        }
        let b_pad = compute_b(verifier, secret);
        let u = int(&srp::h(&[&srp::pad(&a_int), &b_pad]));
        let s = srp::pad(&(a_int * int(verifier).modpow(&u, n)).modpow(&int(secret), n));
        let key = srp::h(&[&s]);
        let expected = srp::h(&[a, &b_pad, &s]);
        (expected == m1).then(|| srp::h(&[a, &expected, &key]))
    }
}

struct CreateSession {
    verifier: Vec<u8>,
    seen_a: Arc<Mutex<Option<Vec<u8>>>>,
}

impl Respond for CreateSession {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        let a = crypto::b64_decode(body["srpA"].as_str().unwrap()).unwrap();
        assert_eq!(a.len(), 512, "A is padded to 512 bytes");
        *self.seen_a.lock().unwrap() = Some(a);
        let b = server_srp::compute_b(&self.verifier, &B_SECRET);
        ResponseTemplate::new(200).set_body_json(json!({
            "sessionID": "4b0f1c5e-0000-4000-8000-000000000001",
            "srpB": crypto::b64_encode(&b),
        }))
    }
}

struct VerifySession {
    verifier: Vec<u8>,
    seen_a: Arc<Mutex<Option<Vec<u8>>>>,
    on_success: Value,
    bad_m2: bool,
}

impl Respond for VerifySession {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["sessionID"], "4b0f1c5e-0000-4000-8000-000000000001");
        let m1 = crypto::b64_decode(body["srpM1"].as_str().unwrap()).unwrap();
        let a = self.seen_a.lock().unwrap().clone().unwrap();
        match server_srp::verify_m1(&self.verifier, &B_SECRET, &a, &m1) {
            Some(mut m2) => {
                if self.bad_m2 {
                    m2[0] ^= 1;
                }
                let mut resp = self.on_success.clone();
                resp["srpM2"] = json!(crypto::b64_encode(&m2));
                ResponseTemplate::new(200).set_body_json(resp)
            }
            None => ResponseTemplate::new(401).set_body_json(json!({"code": "INVALID_PASSWORD"})),
        }
    }
}

fn keys_response(a: &Value) -> Value {
    json!({
        "id": USER_ID,
        "keyAttributes": a["key_attributes"],
        "encryptedToken": a["encrypted_token_b64"],
    })
}

async fn mount_srp(server: &MockServer, email_mfa: bool, on_success: Value, bad_m2: bool) {
    let a = account();
    let verifier = crypto::b64_decode(a["srp_verifier_b64"].as_str().unwrap()).unwrap();
    let attrs = &a["key_attributes"];
    Mock::given(method("GET"))
        .and(path("/users/srp/attributes"))
        .and(query_param("email", "me@example.org"))
        .and(header("X-Client-Package", CLIENT_PACKAGE))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"attributes": {
                "srpUserID": a["srp_user_id"], "srpSalt": a["srp_salt_b64"],
                "memLimit": attrs["memLimit"], "opsLimit": attrs["opsLimit"],
                "kekSalt": attrs["kekSalt"], "isEmailMFAEnabled": email_mfa,
            }})),
        )
        .mount(server)
        .await;
    let seen_a = Arc::new(Mutex::new(None));
    Mock::given(method("POST"))
        .and(path("/users/srp/create-session"))
        .and(body_partial_json(json!({"srpUserID": a["srp_user_id"]})))
        .respond_with(CreateSession {
            verifier: verifier.clone(),
            seen_a: seen_a.clone(),
        })
        .mount(server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/srp/verify-session"))
        .respond_with(VerifySession {
            verifier,
            seen_a,
            on_success,
            bad_m2,
        })
        .mount(server)
        .await;
}

fn client(server: &MockServer) -> ApiClient {
    ApiClient::new(&server.uri(), "0.1.0")
        .unwrap()
        .with_retry(RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        })
}

fn password() -> String {
    account()["password"].as_str().unwrap().to_owned()
}

fn check_session(s: &Session, server: &MockServer) {
    let a = account();
    assert_eq!(s.user_id, USER_ID);
    assert_eq!(s.email, "me@example.org");
    assert_eq!(s.token, a["token_b64url"].as_str().unwrap());
    assert_eq!(s.master_key.to_b64(), a["master_key_b64"].as_str().unwrap());
    assert_eq!(s.secret_key.to_b64(), a["secret_key_b64"].as_str().unwrap());
    assert_eq!(
        s.public_key,
        a["key_attributes"]["publicKey"].as_str().unwrap()
    );
    assert!(s.endpoint.starts_with(&server.uri()));
    // Round-trips through serde (the lead persists it as JSON).
    let json = serde_json::to_string(s).unwrap();
    let back: Session = serde_json::from_str(&json).unwrap();
    assert_eq!(back.token, s.token);
    assert_eq!(back.master_key, s.master_key);
    assert!(!format!("{s:?}").contains(&s.token));
}

#[tokio::test]
async fn srp_login_round_trip() {
    let server = MockServer::start().await;
    mount_srp(&server, false, keys_response(&account()), false).await;
    let c = client(&server);
    let (mut flow, step) = LoginFlow::start(&c, "me@example.org").await.unwrap();
    assert!(matches!(step, LoginStep::Password));
    let LoginStep::Complete(session) = flow.submit_password(&c, &password()).await.unwrap() else {
        panic!("expected completion");
    };
    check_session(&session, &server);

    // Logout sends the token.
    Mock::given(method("POST"))
        .and(path("/users/logout"))
        .and(header("X-Auth-Token", session.token.as_str()))
        .and(header("X-Client-Package", CLIENT_PACKAGE))
        .and(header("X-Client-Version", "0.1.0"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    ApiClient::for_session(&session, "0.1.0")
        .unwrap()
        .logout()
        .await
        .unwrap();
}

#[tokio::test]
async fn wrong_password_is_reported() {
    let server = MockServer::start().await;
    mount_srp(&server, false, keys_response(&account()), false).await;
    let c = client(&server);
    let (mut flow, _) = LoginFlow::start(&c, "me@example.org").await.unwrap();
    let err = flow
        .submit_password(&c, "wrong password")
        .await
        .unwrap_err();
    assert!(matches!(err, ApiError::IncorrectPassword), "{err}");
}

#[tokio::test]
async fn bad_server_proof_is_rejected() {
    let server = MockServer::start().await;
    mount_srp(&server, false, keys_response(&account()), true).await;
    let c = client(&server);
    let (mut flow, _) = LoginFlow::start(&c, "me@example.org").await.unwrap();
    let err = flow.submit_password(&c, &password()).await.unwrap_err();
    assert!(matches!(err, ApiError::Protocol(_)), "{err}");
}

#[tokio::test]
async fn totp_second_factor() {
    let server = MockServer::start().await;
    mount_srp(
        &server,
        false,
        json!({"id": USER_ID, "twoFactorSessionIDV2": "tfa-1"}),
        false,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/users/two-factor/verify"))
        .and(body_partial_json(
            json!({"sessionID": "tfa-1", "code": "123456"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(keys_response(&account())))
        .expect(1)
        .mount(&server)
        .await;
    let c = client(&server);
    let (mut flow, _) = LoginFlow::start(&c, "me@example.org").await.unwrap();
    let step = flow.submit_password(&c, &password()).await.unwrap();
    assert!(matches!(step, LoginStep::Totp));
    let LoginStep::Complete(session) = flow.submit_code(&c, " 123456 ").await.unwrap() else {
        panic!("expected completion");
    };
    check_session(&session, &server);
}

#[tokio::test]
async fn passkey_only_account_errors_clearly() {
    let server = MockServer::start().await;
    mount_srp(
        &server,
        false,
        json!({"id": USER_ID, "passkeySessionID": "pk-1", "accountsUrl": "https://accounts.example.org"}),
        false,
    )
    .await;
    let c = client(&server);
    let (mut flow, _) = LoginFlow::start(&c, "me@example.org").await.unwrap();
    let err = flow.submit_password(&c, &password()).await.unwrap_err();
    assert!(matches!(err, ApiError::PasskeyRequired));
    assert!(err.to_string().contains("passkey"));
}

#[tokio::test]
async fn email_code_then_password() {
    let server = MockServer::start().await;
    mount_srp(&server, true, keys_response(&account()), false).await;
    Mock::given(method("POST"))
        .and(path("/users/ott"))
        .and(body_partial_json(
            json!({"email": "me@example.org", "purpose": "login"}),
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/verify-email"))
        .and(body_partial_json(
            json!({"email": "me@example.org", "ott": "424242"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(keys_response(&account())))
        .mount(&server)
        .await;
    let c = client(&server);
    let (mut flow, step) = LoginFlow::start(&c, "me@example.org").await.unwrap();
    assert!(matches!(step, LoginStep::EmailCode));
    assert!(matches!(
        flow.submit_code(&c, "424242").await.unwrap(),
        LoginStep::Password
    ));
    // A wrong password at this point does not lose the state.
    assert!(matches!(
        flow.submit_password(&c, "wrong").await,
        Err(ApiError::IncorrectPassword)
    ));
}

#[tokio::test]
async fn email_code_then_correct_password() {
    let server = MockServer::start().await;
    // No SRP configured: 404 on attributes means email code first.
    Mock::given(method("GET"))
        .and(path("/users/srp/attributes"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"code": "NOT_FOUND"})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/ott"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/users/verify-email"))
        .respond_with(ResponseTemplate::new(200).set_body_json(keys_response(&account())))
        .mount(&server)
        .await;
    let c = client(&server);
    let (mut flow, step) = LoginFlow::start(&c, "me@example.org").await.unwrap();
    assert!(matches!(step, LoginStep::EmailCode));
    assert!(matches!(
        flow.submit_code(&c, "1").await.unwrap(),
        LoginStep::Password
    ));
    let LoginStep::Complete(session) = flow.submit_password(&c, &password()).await.unwrap() else {
        panic!("expected completion");
    };
    check_session(&session, &server);
}
