//! A mock Ente server ("museum") for the importer's end-to-end tests.
//!
//! It implements just the endpoints the importer uses, with museum's JSON
//! shapes, plays the SRP server role, hands out its own presigned storage
//! URLs (checking `Content-MD5` and lengths like S3 would), and keeps
//! everything it receives so tests can decrypt it afterwards with the
//! importer's crypto module and assert the plaintext.

#![allow(dead_code, clippy::result_large_err, clippy::collapsible_if)]

pub mod srp;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::Engine as _;
use md5::{Digest as _, Md5};
use photovault_import::crypto;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, MutexGuard};

/// Argon2id parameters for test accounts: libsodium's minimum ops and a
/// small memory limit, so tests stay fast.
pub const TEST_OPS_LIMIT: u64 = 2;
/// See [`TEST_OPS_LIMIT`].
pub const TEST_MEM_LIMIT: u64 = 8 * 1024 * 1024;

/// How the test account logs in.
#[derive(Debug, Clone, Default)]
pub struct AccountOptions {
    /// Account has SRP set up (otherwise `/users/srp/attributes` is 404).
    pub srp: bool,
    /// TOTP code required after the first factor.
    pub totp: Option<String>,
    /// Passkey required (the importer must refuse).
    pub passkey: bool,
    /// Email MFA on (SRP is skipped in favour of an email code).
    pub email_mfa: bool,
}

/// The test account and all its secrets.
#[derive(Debug, Clone)]
pub struct Account {
    /// Email.
    pub email: String,
    /// Password.
    pub password: String,
    /// User id.
    pub user_id: i64,
    /// Master key.
    pub master_key: Vec<u8>,
    /// Public key.
    pub public_key: Vec<u8>,
    /// Secret key.
    pub secret_key: Vec<u8>,
    /// Raw auth token bytes.
    pub token_raw: Vec<u8>,
    /// Login options.
    pub opts: AccountOptions,
    /// SRP identity.
    pub srp_user_id: String,
    /// SRP salt.
    pub srp_salt: Vec<u8>,
    /// SRP verifier.
    pub verifier: Vec<u8>,
    /// KEK salt.
    pub kek_salt: Vec<u8>,
    /// Key attributes as museum returns them.
    pub key_attributes: Value,
}

impl Account {
    /// Create an account the way an Ente app does at sign-up.
    pub fn new(email: &str, password: &str, opts: AccountOptions) -> Self {
        crypto::init();
        let kek_salt = crypto::random_bytes(16);
        let kek = crypto::derive_argon_key(
            password.as_bytes(),
            &kek_salt,
            TEST_OPS_LIMIT,
            TEST_MEM_LIMIT,
        )
        .expect("kek");
        let master_key = crypto::generate_key();
        let (public_key, secret_key) = crypto::box_keypair();
        let (enc_key, key_nonce) = crypto::secretbox_seal(&master_key, &kek).expect("seal");
        let (enc_sk, sk_nonce) = crypto::secretbox_seal(&secret_key, &master_key).expect("seal");
        let login_key = crypto::derive_login_key(&kek).expect("login key");
        let srp_user_id = "4f6a3c1e-2b7d-4e0a-9c55-0d1e2f3a4b5c".to_string();
        let srp_salt = crypto::random_bytes(16);
        let verifier = srp::verifier(&srp_salt, srp_user_id.as_bytes(), &login_key);
        let mut token_raw = b"auth-token".to_vec();
        token_raw.extend(crypto::random_bytes(32));
        let key_attributes = json!({
            "kekSalt": crypto::b64(&kek_salt),
            "encryptedKey": crypto::b64(&enc_key),
            "keyDecryptionNonce": crypto::b64(&key_nonce),
            "publicKey": crypto::b64(&public_key),
            "encryptedSecretKey": crypto::b64(&enc_sk),
            "secretKeyDecryptionNonce": crypto::b64(&sk_nonce),
            "memLimit": TEST_MEM_LIMIT,
            "opsLimit": TEST_OPS_LIMIT,
        });
        Self {
            email: email.into(),
            password: password.into(),
            user_id: 1580559962386438,
            master_key,
            public_key,
            secret_key,
            token_raw,
            opts,
            srp_user_id,
            srp_salt,
            verifier,
            kek_salt,
            key_attributes,
        }
    }

    /// The `X-Auth-Token` value clients must send.
    pub fn token(&self) -> String {
        base64::engine::general_purpose::URL_SAFE.encode(&self.token_raw)
    }

    fn authorization(&self, srp_m2: Option<&[u8]>) -> Value {
        let enc_token = crypto::box_seal(&self.token_raw, &self.public_key).expect("seal token");
        let mut v = json!({
            "id": self.user_id,
            "keyAttributes": self.key_attributes,
            "encryptedToken": crypto::b64(&enc_token),
            "accountsUrl": "https://accounts.example",
            "twoFactorSessionID": "",
            "passkeySessionID": "",
        });
        if let Some(m2) = srp_m2 {
            v["srpM2"] = json!(crypto::b64(m2));
        }
        v
    }
}

/// A file as the mock stores it.
#[derive(Debug, Clone)]
pub struct StoredFile {
    /// File id.
    pub id: i64,
    /// Owner.
    pub owner_id: i64,
    /// `file` attributes as received.
    pub file: Value,
    /// `thumbnail` attributes as received.
    pub thumbnail: Value,
    /// `metadata` attributes as received.
    pub metadata: Value,
    /// Public magic metadata as stored.
    pub pub_magic: Option<Value>,
    /// Private magic metadata as stored.
    pub magic: Option<Value>,
}

/// A file's membership in a collection.
#[derive(Debug, Clone)]
pub struct Membership {
    /// Collection id.
    pub collection_id: i64,
    /// File id.
    pub file_id: i64,
    /// File key under the collection key.
    pub encrypted_key: String,
    /// Its nonce.
    pub nonce: String,
    /// Last change.
    pub updation_time: i64,
}

/// What the mock knows.
#[derive(Debug)]
pub struct MockState {
    /// The account.
    pub account: Account,
    /// Base URL of the mock.
    pub base: String,
    /// Pending SRP sessions: id → (server secret b, client A).
    pub srp_sessions: HashMap<String, (Vec<u8>, Vec<u8>)>,
    /// Pending email code.
    pub ott: Option<String>,
    /// Collections as museum returns them.
    pub collections: Vec<Value>,
    /// Files by id.
    pub files: BTreeMap<i64, StoredFile>,
    /// File ↔ collection memberships.
    pub memberships: Vec<Membership>,
    /// Uploaded objects by key.
    pub objects: HashMap<String, Vec<u8>>,
    /// Issued single upload URLs: key → expected (length, md5) if checksummed.
    pub issued: HashMap<String, Option<(usize, String)>>,
    /// In-progress multipart uploads: key → parts.
    pub multipart: HashMap<String, BTreeMap<u32, Vec<u8>>>,
    /// Every request, as "METHOD /path".
    pub requests: Vec<String>,
    /// Answer the next N requests to a path with 503.
    pub fail_next: HashMap<String, u32>,
    /// Pretend to be an old server without `POST /files/upload-url`.
    pub legacy_only: bool,
    /// Files per diff page.
    pub diff_page_size: usize,
    next_id: i64,
    clock: i64,
}

impl MockState {
    fn tick(&mut self) -> i64 {
        self.clock += 1;
        self.clock
    }

    fn next_id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }
}

/// A running mock server.
pub struct MockServer {
    /// Base URL, e.g. `http://127.0.0.1:1234`.
    pub url: url::Url,
    state: Arc<Mutex<MockState>>,
}

type Shared = State<Arc<Mutex<MockState>>>;

fn err(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"code": "ERR", "message": msg}))).into_response()
}

impl MockServer {
    /// Start a mock for `account`.
    pub async fn start(account: Account) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        let base = format!("http://{addr}");
        let state = Arc::new(Mutex::new(MockState {
            account,
            base: base.clone(),
            srp_sessions: HashMap::new(),
            ott: None,
            collections: Vec::new(),
            files: BTreeMap::new(),
            memberships: Vec::new(),
            objects: HashMap::new(),
            issued: HashMap::new(),
            multipart: HashMap::new(),
            requests: Vec::new(),
            fail_next: HashMap::new(),
            legacy_only: false,
            diff_page_size: 1000,
            next_id: 1000,
            clock: 1_700_000_000_000_000,
        }));
        let app = Router::new()
            .route("/users/srp/attributes", get(srp_attributes))
            .route("/users/srp/create-session", post(srp_create))
            .route("/users/srp/verify-session", post(srp_verify))
            .route("/users/two-factor/verify", post(two_factor_verify))
            .route("/users/ott", post(send_ott))
            .route("/users/verify-email", post(verify_email))
            .route("/collections/v2", get(list_collections))
            .route("/collections", post(create_collection))
            .route("/collections/v2/diff", get(collection_diff))
            .route("/collections/add-files", post(add_files))
            .route("/files/upload-url", post(upload_url_v2))
            .route("/files/multipart-upload-url", post(multipart_url_v2))
            .route("/files/upload-urls", get(upload_urls_legacy))
            .route("/files/multipart-upload-urls", get(multipart_urls_legacy))
            .route("/files", post(create_file))
            .route("/files/magic-metadata", put(update_magic))
            .route("/storage/{key}", put(put_object))
            .route("/storage/{key}/parts/{n}", put(put_part))
            .route("/storage/{key}/complete", post(complete_multipart))
            .layer(axum::extract::DefaultBodyLimit::disable())
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                record_and_fail,
            ))
            .with_state(state.clone());
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve");
        });
        Self {
            url: url::Url::parse(&base).expect("url"),
            state,
        }
    }

    /// Lock the state.
    pub fn state(&self) -> MutexGuard<'_, MockState> {
        self.state.lock().expect("lock")
    }

    /// Decrypt a stored collection: `(key, name, private magic metadata)`.
    pub fn decrypt_collection(&self, id: i64) -> (Vec<u8>, String, Option<Value>) {
        let s = self.state();
        let c = s
            .collections
            .iter()
            .find(|c| c["id"] == id)
            .expect("collection")
            .clone();
        let master = s.account.master_key.clone();
        drop(s);
        let key = crypto::secretbox_open(
            &crypto::unb64(c["encryptedKey"].as_str().unwrap()).unwrap(),
            &crypto::unb64(c["keyDecryptionNonce"].as_str().unwrap()).unwrap(),
            &master,
        )
        .expect("collection key");
        let name = String::from_utf8(
            crypto::secretbox_open(
                &crypto::unb64(c["encryptedName"].as_str().unwrap()).unwrap(),
                &crypto::unb64(c["nameDecryptionNonce"].as_str().unwrap()).unwrap(),
                &key,
            )
            .expect("name"),
        )
        .unwrap();
        let magic = c.get("magicMetadata").filter(|m| !m.is_null()).map(|m| {
            decrypt_json(
                m["data"].as_str().unwrap(),
                m["header"].as_str().unwrap(),
                &key,
            )
        });
        (key, name, magic)
    }

    /// All collections, decrypted: `(id, type, name, magic)`.
    pub fn collections(&self) -> Vec<(i64, String, String, Option<Value>)> {
        let ids: Vec<(i64, String)> = self
            .state()
            .collections
            .iter()
            .map(|c| {
                (
                    c["id"].as_i64().unwrap(),
                    c["type"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        ids.into_iter()
            .map(|(id, kind)| {
                let (_, name, magic) = self.decrypt_collection(id);
                (id, kind, name, magic)
            })
            .collect()
    }

    /// The id of the collection named `name`.
    pub fn collection_id(&self, name: &str) -> i64 {
        self.collections()
            .into_iter()
            .find(|c| c.2 == name)
            .unwrap_or_else(|| panic!("no collection {name:?}"))
            .0
    }

    /// Decrypt everything about file `id`, checking that its key decrypts
    /// to the same value under every collection it is in.
    pub fn decrypt_file(&self, id: i64) -> DecryptedFile {
        let (file, members) = {
            let s = self.state();
            let f = s.files.get(&id).expect("file").clone();
            let m: Vec<Membership> = s
                .memberships
                .iter()
                .filter(|m| m.file_id == id)
                .cloned()
                .collect();
            (f, m)
        };
        let mut key: Option<Vec<u8>> = None;
        let mut collections = Vec::new();
        for m in &members {
            let (ckey, name, _) = self.decrypt_collection(m.collection_id);
            let k = crypto::secretbox_open(
                &crypto::unb64(&m.encrypted_key).unwrap(),
                &crypto::unb64(&m.nonce).unwrap(),
                &ckey,
            )
            .expect("file key");
            if let Some(prev) = &key {
                assert_eq!(prev, &k, "file key differs between collections");
            }
            key = Some(k);
            collections.push(name);
        }
        collections.sort();
        let key = key.expect("file in no collection");
        let metadata = decrypt_json(
            file.metadata["encryptedData"].as_str().unwrap(),
            file.metadata["decryptionHeader"].as_str().unwrap(),
            &key,
        );
        let mm = |v: &Option<Value>| {
            v.as_ref().map(|m| {
                (
                    m["version"].as_i64().unwrap(),
                    m["count"].as_i64().unwrap_or(0),
                    decrypt_json(
                        m["data"].as_str().unwrap(),
                        m["header"].as_str().unwrap(),
                        &key,
                    ),
                )
            })
        };
        let (content_ct, thumb_ct) = {
            let s = self.state();
            (
                s.objects[file.file["objectKey"].as_str().unwrap()].clone(),
                s.objects[file.thumbnail["objectKey"].as_str().unwrap()].clone(),
            )
        };
        assert_eq!(
            file.file["size"].as_u64().unwrap() as usize,
            content_ct.len(),
            "file size"
        );
        assert_eq!(
            file.thumbnail["size"].as_u64().unwrap() as usize,
            thumb_ct.len(),
            "thumbnail size"
        );
        let content = crypto::decrypt_stream(
            &content_ct,
            &crypto::unb64(file.file["decryptionHeader"].as_str().unwrap()).unwrap(),
            &key,
        )
        .expect("file contents");
        let thumbnail = crypto::decrypt_blob(
            &thumb_ct,
            &crypto::unb64(file.thumbnail["decryptionHeader"].as_str().unwrap()).unwrap(),
            &key,
        )
        .expect("thumbnail");
        DecryptedFile {
            id,
            metadata,
            pub_magic: mm(&file.pub_magic),
            magic: mm(&file.magic),
            content,
            thumbnail,
            collections,
            encrypted_size: content_ct.len(),
        }
    }

    /// Ids of all files.
    pub fn file_ids(&self) -> Vec<i64> {
        self.state().files.keys().copied().collect()
    }

    /// How many requests matched `method_path` (e.g. `"POST /files"`).
    pub fn count(&self, method_path: &str) -> usize {
        self.state()
            .requests
            .iter()
            .filter(|r| r.as_str() == method_path)
            .count()
    }
}

/// Plaintext of a stored file. Magic metadata is `(version, count, data)`.
#[derive(Debug, Clone)]
pub struct DecryptedFile {
    /// File id.
    pub id: i64,
    /// Decrypted metadata JSON.
    pub metadata: Value,
    /// Decrypted public magic metadata.
    pub pub_magic: Option<(i64, i64, Value)>,
    /// Decrypted private magic metadata.
    pub magic: Option<(i64, i64, Value)>,
    /// Decrypted file contents.
    pub content: Vec<u8>,
    /// Decrypted thumbnail.
    pub thumbnail: Vec<u8>,
    /// Names of the collections the file is in, sorted.
    pub collections: Vec<String>,
    /// Ciphertext length of the contents.
    pub encrypted_size: usize,
}

/// Decrypt `encryptMetadataJSON` output.
pub fn decrypt_json(data: &str, header: &str, key: &[u8]) -> Value {
    let plain = crypto::decrypt_blob(
        &crypto::unb64(data).unwrap(),
        &crypto::unb64(header).unwrap(),
        key,
    )
    .expect("metadata");
    serde_json::from_slice(&plain).expect("json")
}

fn md5_b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(Md5::digest(data))
}

async fn record_and_fail(
    State(st): Shared,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let key = format!("{} {}", req.method(), req.uri().path());
    {
        let mut s = st.lock().unwrap();
        s.requests.push(key.clone());
        if let Some(n) = s.fail_next.get_mut(&key) {
            if *n > 0 {
                *n -= 1;
                return err(StatusCode::SERVICE_UNAVAILABLE, "injected failure");
            }
        }
    }
    next.run(req).await
}

fn authed(s: &MockState, headers: &HeaderMap) -> Result<(), Response> {
    let ok = headers
        .get("X-Auth-Token")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|t| t == s.account.token());
    let pkg = headers
        .get("X-Client-Package")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|p| !p.is_empty());
    if ok && pkg {
        Ok(())
    } else {
        Err(err(StatusCode::UNAUTHORIZED, "bad token"))
    }
}

async fn srp_attributes(State(st): Shared, Query(q): Query<HashMap<String, String>>) -> Response {
    let s = st.lock().unwrap();
    let a = &s.account;
    if q.get("email") != Some(&a.email) || !a.opts.srp {
        return err(StatusCode::NOT_FOUND, "no srp attributes");
    }
    Json(json!({"attributes": {
        "srpUserID": a.srp_user_id,
        "srpSalt": crypto::b64(&a.srp_salt),
        "memLimit": TEST_MEM_LIMIT,
        "opsLimit": TEST_OPS_LIMIT,
        "kekSalt": crypto::b64(&a.kek_salt),
        "isEmailMFAEnabled": a.opts.email_mfa,
    }}))
    .into_response()
}

async fn srp_create(State(st): Shared, Json(body): Json<Value>) -> Response {
    let mut s = st.lock().unwrap();
    if body["srpUserID"] != s.account.srp_user_id.as_str() {
        return err(StatusCode::NOT_FOUND, "unknown srp user");
    }
    let a = crypto::unb64(body["srpA"].as_str().unwrap_or_default()).unwrap_or_default();
    let secret = crypto::random_bytes(32);
    let b = srp::compute_b(&s.account.verifier, &secret);
    let id = format!("0b8a0c5e-0000-4000-8000-{:012}", s.srp_sessions.len());
    s.srp_sessions.insert(id.clone(), (secret, a));
    Json(json!({"sessionID": id, "srpB": crypto::b64(&b)})).into_response()
}

async fn srp_verify(State(st): Shared, Json(body): Json<Value>) -> Response {
    let s = st.lock().unwrap();
    if s.account.opts.email_mfa {
        return err(StatusCode::UNAUTHORIZED, "EMAIL_MFA_ENABLED");
    }
    let Some((secret, a)) = s
        .srp_sessions
        .get(body["sessionID"].as_str().unwrap_or_default())
        .cloned()
    else {
        return err(StatusCode::NOT_FOUND, "no session");
    };
    let m1 = crypto::unb64(body["srpM1"].as_str().unwrap_or_default()).unwrap_or_default();
    let Some(m2) = srp::verify_m1(&s.account.verifier, &secret, &a, &m1) else {
        return err(StatusCode::UNAUTHORIZED, "wrong password");
    };
    second_factor_or_done(&s.account, Some(&m2))
}

fn second_factor_or_done(a: &Account, m2: Option<&[u8]>) -> Response {
    if a.opts.passkey {
        return Json(json!({
            "id": a.user_id,
            "passkeySessionID": "pk-session",
            "accountsUrl": "https://accounts.example",
            "twoFactorSessionID": "",
        }))
        .into_response();
    }
    if a.opts.totp.is_some() {
        return Json(json!({
            "id": a.user_id,
            "twoFactorSessionID": "tf-session",
            "passkeySessionID": "",
            "accountsUrl": "https://accounts.example",
        }))
        .into_response();
    }
    Json(a.authorization(m2)).into_response()
}

async fn two_factor_verify(State(st): Shared, Json(body): Json<Value>) -> Response {
    let s = st.lock().unwrap();
    if body["sessionID"] != "tf-session" {
        return err(StatusCode::NOT_FOUND, "no session");
    }
    if s.account.opts.totp.as_deref() != body["code"].as_str() {
        return err(StatusCode::UNAUTHORIZED, "wrong code");
    }
    Json(s.account.authorization(None)).into_response()
}

async fn send_ott(State(st): Shared, Json(body): Json<Value>) -> Response {
    let mut s = st.lock().unwrap();
    if body["email"] != s.account.email.as_str() || body["purpose"] != "login" {
        return err(StatusCode::BAD_REQUEST, "bad ott request");
    }
    s.ott = Some("424242".into());
    StatusCode::OK.into_response()
}

async fn verify_email(State(st): Shared, Json(body): Json<Value>) -> Response {
    let s = st.lock().unwrap();
    if body["email"] != s.account.email.as_str() || s.ott.as_deref() != body["ott"].as_str() {
        return err(StatusCode::UNAUTHORIZED, "wrong code");
    }
    second_factor_or_done(&s.account, None)
}

async fn list_collections(State(st): Shared, headers: HeaderMap) -> Response {
    let s = st.lock().unwrap();
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    Json(json!({"collections": s.collections})).into_response()
}

async fn create_collection(
    State(st): Shared,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut s = st.lock().unwrap();
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    for k in [
        "encryptedKey",
        "keyDecryptionNonce",
        "encryptedName",
        "nameDecryptionNonce",
        "type",
    ] {
        if !body[k].is_string() {
            return err(StatusCode::BAD_REQUEST, k);
        }
    }
    let kind = body["type"].as_str().unwrap();
    if !["album", "folder", "favorites", "uncategorized"].contains(&kind) {
        return err(StatusCode::BAD_REQUEST, "type");
    }
    let id = s.next_id();
    let t = s.tick();
    let mut c = json!({
        "id": id,
        "owner": {"id": s.account.user_id, "email": s.account.email, "name": "", "role": "OWNER"},
        "encryptedKey": body["encryptedKey"],
        "keyDecryptionNonce": body["keyDecryptionNonce"],
        "name": "",
        "encryptedName": body["encryptedName"],
        "nameDecryptionNonce": body["nameDecryptionNonce"],
        "type": kind,
        "attributes": {"version": 0},
        "sharees": [],
        "publicURLs": [],
        "updationTime": t,
        "app": "photos",
    });
    if !body["magicMetadata"].is_null() {
        c["magicMetadata"] = body["magicMetadata"].clone();
    }
    s.collections.push(c.clone());
    Json(json!({"collection": c})).into_response()
}

fn file_json(s: &MockState, m: &Membership) -> Value {
    let f = &s.files[&m.file_id];
    let mut v = json!({
        "id": f.id,
        "ownerID": f.owner_id,
        "collectionID": m.collection_id,
        "encryptedKey": m.encrypted_key,
        "keyDecryptionNonce": m.nonce,
        "file": {"decryptionHeader": f.file["decryptionHeader"]},
        "thumbnail": {"decryptionHeader": f.thumbnail["decryptionHeader"]},
        "metadata": f.metadata,
        "isDeleted": false,
        "updationTime": m.updation_time,
    });
    if let Some(mm) = &f.magic {
        v["magicMetadata"] = mm.clone();
    }
    if let Some(mm) = &f.pub_magic {
        v["pubMagicMetadata"] = mm.clone();
    }
    v
}

async fn collection_diff(
    State(st): Shared,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let s = st.lock().unwrap();
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    let cid: i64 = q
        .get("collectionID")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let since: i64 = q.get("sinceTime").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut ms: Vec<&Membership> = s
        .memberships
        .iter()
        .filter(|m| m.collection_id == cid && m.updation_time > since)
        .collect();
    ms.sort_by_key(|m| m.updation_time);
    let has_more = ms.len() > s.diff_page_size;
    let diff: Vec<Value> = ms
        .into_iter()
        .take(s.diff_page_size)
        .map(|m| file_json(&s, m))
        .collect();
    Json(json!({"diff": diff, "hasMore": has_more})).into_response()
}

async fn add_files(State(st): Shared, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let mut s = st.lock().unwrap();
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    let cid = body["collectionID"].as_i64().unwrap_or(0);
    if !s.collections.iter().any(|c| c["id"] == cid) {
        return err(StatusCode::NOT_FOUND, "collection");
    }
    for f in body["files"].as_array().cloned().unwrap_or_default() {
        let fid = f["id"].as_i64().unwrap_or(0);
        if !s.files.contains_key(&fid) {
            return err(StatusCode::BAD_REQUEST, "file");
        }
        if s.memberships
            .iter()
            .any(|m| m.file_id == fid && m.collection_id == cid)
        {
            continue;
        }
        let t = s.tick();
        s.memberships.push(Membership {
            collection_id: cid,
            file_id: fid,
            encrypted_key: f["encryptedKey"].as_str().unwrap_or_default().into(),
            nonce: f["keyDecryptionNonce"].as_str().unwrap_or_default().into(),
            updation_time: t,
        });
    }
    StatusCode::OK.into_response()
}

fn new_object_key(s: &mut MockState) -> String {
    let n = s.next_id();
    format!("{}-obj-{n}", s.account.user_id)
}

async fn upload_url_v2(State(st): Shared, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let mut s = st.lock().unwrap();
    if s.legacy_only {
        return err(StatusCode::NOT_FOUND, "404 page not found");
    }
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    let len = body["contentLength"].as_u64().unwrap_or(0) as usize;
    let md5 = body["contentMD5"].as_str().unwrap_or_default().to_string();
    if len == 0
        || base64::engine::general_purpose::STANDARD
            .decode(&md5)
            .map(|d| d.len())
            != Ok(16)
    {
        return err(StatusCode::BAD_REQUEST, "contentLength/contentMD5");
    }
    let key = new_object_key(&mut s);
    s.issued.insert(key.clone(), Some((len, md5)));
    Json(json!({"objectKey": key, "url": format!("{}/storage/{key}?sig=x", s.base)}))
        .into_response()
}

async fn upload_urls_legacy(
    State(st): Shared,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let mut s = st.lock().unwrap();
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    let count: usize = q.get("count").and_then(|v| v.parse().ok()).unwrap_or(0);
    let mut urls = Vec::new();
    for _ in 0..count {
        let key = new_object_key(&mut s);
        s.issued.insert(key.clone(), None);
        urls.push(json!({"objectKey": key, "url": format!("{}/storage/{key}?sig=x", s.base)}));
    }
    Json(json!({"urls": urls})).into_response()
}

fn multipart_json(s: &mut MockState, parts: usize) -> Value {
    let key = new_object_key(s);
    s.multipart.insert(key.clone(), BTreeMap::new());
    let part_urls: Vec<String> = (1..=parts)
        .map(|n| format!("{}/storage/{key}/parts/{n}?sig=x", s.base))
        .collect();
    json!({
        "objectKey": key,
        "partURLs": part_urls,
        "completeURL": format!("{}/storage/{key}/complete?sig=x", s.base),
    })
}

async fn multipart_url_v2(
    State(st): Shared,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let mut s = st.lock().unwrap();
    if s.legacy_only {
        return err(StatusCode::NOT_FOUND, "404 page not found");
    }
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    let len = body["contentLength"].as_u64().unwrap_or(0);
    let part = body["partLength"].as_u64().unwrap_or(0);
    // Museum's ValidateMultipartPartLength.
    if len == 0 || part == 0 || (len > part && part < 5 << 20) {
        return err(StatusCode::BAD_REQUEST, "partLength");
    }
    let parts = len.div_ceil(part) as usize;
    Json(multipart_json(&mut s, parts)).into_response()
}

async fn multipart_urls_legacy(
    State(st): Shared,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    let mut s = st.lock().unwrap();
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    let count: usize = q.get("count").and_then(|v| v.parse().ok()).unwrap_or(0);
    Json(json!({"urls": multipart_json(&mut s, count)})).into_response()
}

fn check_md5(headers: &HeaderMap, body: &[u8]) -> Result<String, Response> {
    let got = md5_b64(body);
    match headers.get("Content-MD5").and_then(|v| v.to_str().ok()) {
        Some(m) if m == got => Ok(got),
        Some(_) => Err(err(StatusCode::BAD_REQUEST, "BadDigest")),
        None => Ok(got),
    }
}

async fn put_object(
    State(st): Shared,
    Path(key): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut s = st.lock().unwrap();
    if headers.contains_key("X-Auth-Token") {
        return err(StatusCode::BAD_REQUEST, "token leaked to storage");
    }
    let Some(expect) = s.issued.remove(&key) else {
        return err(StatusCode::FORBIDDEN, "no such upload URL");
    };
    let md5 = match check_md5(&headers, &body) {
        Ok(m) => m,
        Err(r) => return r,
    };
    if let Some((len, want)) = expect {
        if len != body.len() || want != md5 {
            return err(
                StatusCode::BAD_REQUEST,
                "length or checksum differs from the signed one",
            );
        }
    }
    s.objects.insert(key, body.to_vec());
    StatusCode::OK.into_response()
}

async fn put_part(
    State(st): Shared,
    Path((key, n)): Path<(String, u32)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut s = st.lock().unwrap();
    if headers.contains_key("X-Auth-Token") {
        return err(StatusCode::BAD_REQUEST, "token leaked to storage");
    }
    if let Err(r) = check_md5(&headers, &body) {
        return r;
    }
    let etag = format!("\"{}\"", hex::encode(Md5::digest(&body)));
    let Some(parts) = s.multipart.get_mut(&key) else {
        return err(StatusCode::NOT_FOUND, "NoSuchUpload");
    };
    parts.insert(n, body.to_vec());
    ([("ETag", etag)], StatusCode::OK).into_response()
}

async fn complete_multipart(
    State(st): Shared,
    Path(key): Path<String>,
    headers: HeaderMap,
    body: String,
) -> Response {
    let mut s = st.lock().unwrap();
    if headers.get("Content-Type").and_then(|v| v.to_str().ok()) != Some("text/xml") {
        return err(StatusCode::BAD_REQUEST, "content type");
    }
    let Some(parts) = s.multipart.remove(&key) else {
        return err(StatusCode::NOT_FOUND, "NoSuchUpload");
    };
    // Minimal parse of <Part><PartNumber>n</PartNumber><ETag>e</ETag></Part>.
    let mut out = Vec::new();
    for (i, seg) in body.split("<Part>").skip(1).enumerate() {
        let num: u32 = seg
            .split("<PartNumber>")
            .nth(1)
            .and_then(|r| r.split("</PartNumber>").next())
            .and_then(|n| n.parse().ok())
            .unwrap_or(0);
        let etag = seg
            .split("<ETag>")
            .nth(1)
            .and_then(|r| r.split("</ETag>").next())
            .unwrap_or_default();
        let Some(data) = parts.get(&num) else {
            return err(StatusCode::BAD_REQUEST, "InvalidPart");
        };
        if num as usize != i + 1 || etag != format!("\"{}\"", hex::encode(Md5::digest(data))) {
            return err(StatusCode::BAD_REQUEST, "InvalidPartOrder");
        }
        out.extend_from_slice(data);
    }
    if out.is_empty() || parts.len() != body.matches("<Part>").count() {
        return err(StatusCode::BAD_REQUEST, "parts");
    }
    s.objects.insert(key, out);
    StatusCode::OK.into_response()
}

async fn create_file(State(st): Shared, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let mut s = st.lock().unwrap();
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    let cid = body["collectionID"].as_i64().unwrap_or(0);
    if !s.collections.iter().any(|c| c["id"] == cid) {
        return err(StatusCode::NOT_FOUND, "collection");
    }
    for part in ["file", "thumbnail"] {
        let key = body[part]["objectKey"].as_str().unwrap_or_default();
        let Some(obj) = s.objects.get(key) else {
            return err(StatusCode::BAD_REQUEST, "object not uploaded");
        };
        if body[part]["size"].as_u64() != Some(obj.len() as u64)
            || !body[part]["decryptionHeader"].is_string()
        {
            return err(StatusCode::BAD_REQUEST, "size/header");
        }
    }
    if !body["metadata"]["encryptedData"].is_string()
        || !body["metadata"]["decryptionHeader"].is_string()
        || !body["encryptedKey"].is_string()
        || !body["keyDecryptionNonce"].is_string()
    {
        return err(StatusCode::BAD_REQUEST, "fields");
    }
    let id = s.next_id();
    let t = s.tick();
    let owner = s.account.user_id;
    let nonnull = |v: &Value| (!v.is_null()).then(|| v.clone());
    s.files.insert(
        id,
        StoredFile {
            id,
            owner_id: owner,
            file: body["file"].clone(),
            thumbnail: body["thumbnail"].clone(),
            metadata: body["metadata"].clone(),
            pub_magic: nonnull(&body["pubMagicMetadata"]),
            magic: nonnull(&body["magicMetadata"]),
        },
    );
    let m = Membership {
        collection_id: cid,
        file_id: id,
        encrypted_key: body["encryptedKey"].as_str().unwrap().into(),
        nonce: body["keyDecryptionNonce"].as_str().unwrap().into(),
        updation_time: t,
    };
    let resp = file_json(&s, &m);
    s.memberships.push(m);
    Json(resp).into_response()
}

async fn update_magic(State(st): Shared, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let mut s = st.lock().unwrap();
    if let Err(r) = authed(&s, &headers) {
        return r;
    }
    for u in body["metadataList"].as_array().cloned().unwrap_or_default() {
        let id = u["id"].as_i64().unwrap_or(0);
        let mut mm = u["magicMetadata"].clone();
        let Some(f) = s.files.get(&id) else {
            return err(StatusCode::NOT_FOUND, "file");
        };
        // Museum's validateUpdateMetadataRequest.
        if let Some(existing) = &f.magic {
            let diff = existing["count"].as_i64().unwrap_or(0) - mm["count"].as_i64().unwrap_or(0);
            if existing["version"] != mm["version"] || diff > 2 {
                return err(StatusCode::CONFLICT, "VERSION_MISMATCH");
            }
        }
        mm["version"] = json!(mm["version"].as_i64().unwrap_or(0) + 1);
        s.files.get_mut(&id).unwrap().magic = Some(mm);
        let t = s.tick();
        for m in s.memberships.iter_mut().filter(|m| m.file_id == id) {
            m.updation_time = t;
        }
    }
    StatusCode::OK.into_response()
}

/// Unused-import guard for helper types some test files do not need.
pub fn _unused(_: Method, _: Uri) {}
