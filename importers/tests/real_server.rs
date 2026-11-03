//! End to end against the real Photovault server (in process, SQLite and
//! local disk): sign up like an Ente app, import a synthetic iCloud export,
//! then read everything back through the server's API and decrypt it.

mod support;

use photovault_import::crypto;
use photovault_import::ente::{Credentials, PeopleMode, RetryPolicy, UploadOptions, Uploader};
use photovault_import::icloud::{ICloudOptions, ICloudSource};
use photovault_import::run::{Summary, run};
use photovault_import::srp::SrpClient;
use photovault_import::state::Journal;
use photovault_server::testing::TestServer;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::{Account, AccountOptions};

const EMAIL: &str = "carol@example.org";
const PASSWORD: &str = "real server password";
const OTT: &str = "424242";
/// Argon2 limits whose product is the strength the server requires
/// (4 × 1 GiB, like the apps' "sensitive" setting), but cheap on memory.
const OPS_LIMIT: u64 = 32;
const MEM_LIMIT: u64 = 128 * 1024 * 1024;
const HEADER: &str =
    "imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate\n";

/// The importer uploads originals byte for byte, so the photo contents need
/// not be real JPEGs (thumbnails fall back to a placeholder).
fn write_export(root: &Path) -> PathBuf {
    let part = root.join("iCloud Photos Part 1 of 1");
    std::fs::create_dir_all(part.join("Photos")).unwrap();
    std::fs::create_dir_all(part.join("Albums")).unwrap();
    // Large enough for a multipart upload (more than 16 MiB of ciphertext).
    let big: Vec<u8> = (0..(17 * 1024 * 1024u32))
        .map(|i| (i * 31 % 251) as u8)
        .collect();
    std::fs::write(part.join("Photos/IMG_0200.JPG"), &big).unwrap();
    std::fs::write(part.join("Photos/IMG_0201.JPG"), b"small photo bytes").unwrap();
    std::fs::write(
        part.join("Photos/Photo Details.csv"),
        format!(
            "{HEADER}IMG_0200.JPG,CK1,yes,no,no,\"Monday May 1,2023 9:30 AM GMT\",0,\"Monday May 1,2023 9:31 AM GMT\"\n\
             IMG_0201.JPG,CK2,no,no,no,\"Tuesday May 2,2023 10:00 AM GMT\",0,\"Tuesday May 2,2023 10:01 AM GMT\"\n"
        ),
    )
    .unwrap();
    std::fs::write(part.join("Albums/Paris.csv"), "imgName\nIMG_0200.JPG\n").unwrap();
    part
}

struct Http {
    base: String,
    c: reqwest::Client,
    token: Option<String>,
}

impl Http {
    async fn call(&self, m: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let mut r = self.c.request(m, format!("{}{path}", self.base));
        if let Some(t) = &self.token {
            r = r.header("X-Auth-Token", t);
        }
        if let Some(b) = body {
            r = r.json(&b);
        }
        let res = r.send().await.unwrap();
        let status = res.status();
        let text = res.text().await.unwrap();
        assert!(status.is_success(), "{path}: {status} {text}");
        serde_json::from_str(&text).unwrap_or(Value::Null)
    }
}

/// Sign up exactly as the apps do: email code, key attributes, SRP setup.
async fn signup(base: &str, acc: &Account) {
    let mut h = Http {
        base: base.into(),
        c: reqwest::Client::new(),
        token: None,
    };
    h.call(
        reqwest::Method::POST,
        "/users/ott",
        Some(json!({"email": acc.email, "purpose": "signup"})),
    )
    .await;
    let v = h
        .call(
            reqwest::Method::POST,
            "/users/verify-email",
            Some(json!({"email": acc.email, "ott": OTT})),
        )
        .await;
    h.token = Some(v["token"].as_str().expect("token").to_string());
    let mut ka = acc.key_attributes.clone();
    let r = |n| crypto::b64(&crypto::random_bytes(n));
    ka["masterKeyEncryptedWithRecoveryKey"] = r(48).into();
    ka["masterKeyDecryptionNonce"] = r(24).into();
    ka["recoveryKeyEncryptedWithMasterKey"] = r(48).into();
    ka["recoveryKeyDecryptionNonce"] = r(24).into();
    h.call(
        reqwest::Method::PUT,
        "/users/attributes",
        Some(json!({"keyAttributes": ka})),
    )
    .await;
    let kek =
        crypto::derive_argon_key(acc.password.as_bytes(), &acc.kek_salt, OPS_LIMIT, MEM_LIMIT)
            .unwrap();
    let login_key = crypto::derive_login_key(&kek).unwrap();
    let mut srp = SrpClient::new(acc.srp_user_id.as_bytes(), &acc.srp_salt, &login_key);
    let v = h
        .call(
            reqwest::Method::POST,
            "/users/srp/setup",
            Some(json!({
                "srpUserID": acc.srp_user_id,
                "srpSalt": crypto::b64(&acc.srp_salt),
                "srpVerifier": crypto::b64(&acc.verifier),
                "srpA": crypto::b64(&srp.public_a()),
            })),
        )
        .await;
    let m1 = srp
        .compute_m1(&crypto::unb64(v["srpB"].as_str().unwrap()).unwrap())
        .unwrap();
    let v2 = h
        .call(
            reqwest::Method::POST,
            "/users/srp/complete",
            Some(json!({"setupID": v["setupID"], "srpM1": crypto::b64(&m1)})),
        )
        .await;
    srp.verify_m2(&crypto::unb64(v2["srpM2"].as_str().unwrap()).unwrap())
        .unwrap();
}

async fn import(base: &str, export: &Path, journal: &Path) -> Summary {
    let no_prompt = || -> anyhow::Result<String> { panic!("unexpected prompt") };
    let mut uploader = Uploader::connect_with_retry(
        Credentials {
            endpoint: url::Url::parse(base).unwrap(),
            email: EMAIL.into(),
            password: PASSWORD.into(),
        },
        UploadOptions {
            default_album: "Imported from iCloud".into(),
            people: PeopleMode::Albums,
            people_album_prefix: String::new(),
            dry_run: false,
        },
        &no_prompt,
        &no_prompt,
        RetryPolicy {
            attempts: 2,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        },
    )
    .await
    .expect("login against the real server");
    let mut source = ICloudSource::open(&[export.to_path_buf()], ICloudOptions::default()).unwrap();
    let mut journal = Journal::open(journal, "icloud").unwrap();
    run(&mut source, &mut uploader, &mut journal).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn import_into_real_photovault_server() {
    photovault_import::tls::install_provider();
    let server = TestServer::start_with(|c| {
        c.hardcoded_ott.insert(EMAIL.into(), OTT.into());
    })
    .await
    .unwrap();
    let acc = Account::with_kdf(
        EMAIL,
        PASSWORD,
        AccountOptions {
            srp: true,
            ..Default::default()
        },
        OPS_LIMIT,
        MEM_LIMIT,
    );
    signup(&server.url, &acc).await;

    let dir = tempfile::tempdir().unwrap();
    let export = write_export(dir.path());
    let s = import(&server.url, &export, &dir.path().join("j.jsonl")).await;
    assert_eq!((s.uploaded, s.failed), (2, 0), "{s:?}");

    // Read back through the API with an independent token from a fresh login.
    let token = {
        // Email-code login, as the apps can do.
        let h = Http {
            base: server.url.clone(),
            c: reqwest::Client::new(),
            token: None,
        };
        h.call(
            reqwest::Method::POST,
            "/users/ott",
            Some(json!({"email": EMAIL, "purpose": "login"})),
        )
        .await;
        let v = h
            .call(
                reqwest::Method::POST,
                "/users/verify-email",
                Some(json!({"email": EMAIL, "ott": OTT})),
            )
            .await;
        let sealed = crypto::unb64(v["encryptedToken"].as_str().unwrap()).unwrap();
        let raw = crypto::box_seal_open(&sealed, &acc.public_key, &acc.secret_key).unwrap();
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE, raw)
    };
    let h = Http {
        base: server.url.clone(),
        c: reqwest::Client::new(),
        token: Some(token),
    };
    let cols = h
        .call(reqwest::Method::GET, "/collections/v2?sinceTime=0", None)
        .await;
    let mut names = std::collections::BTreeMap::new();
    for c in cols["collections"].as_array().unwrap() {
        let key = crypto::secretbox_open(
            &crypto::unb64(c["encryptedKey"].as_str().unwrap()).unwrap(),
            &crypto::unb64(c["keyDecryptionNonce"].as_str().unwrap()).unwrap(),
            &acc.master_key,
        )
        .unwrap();
        let name = String::from_utf8(
            crypto::secretbox_open(
                &crypto::unb64(c["encryptedName"].as_str().unwrap()).unwrap(),
                &crypto::unb64(c["nameDecryptionNonce"].as_str().unwrap()).unwrap(),
                &key,
            )
            .unwrap(),
        )
        .unwrap();
        names.insert(name, (c["id"].as_i64().unwrap(), key));
    }
    for want in ["Paris", "Favorites", "Imported from iCloud"] {
        assert!(
            names.contains_key(want),
            "{want} missing: {:?}",
            names.keys()
        );
    }

    // Download and decrypt the multipart-uploaded original from the Paris album.
    let (paris_id, paris_key) = &names["Paris"];
    let diff = h
        .call(
            reqwest::Method::GET,
            &format!("/collections/v2/diff?collectionID={paris_id}&sinceTime=0"),
            None,
        )
        .await;
    let f = &diff["diff"].as_array().unwrap()[0];
    let file_key = crypto::secretbox_open(
        &crypto::unb64(f["encryptedKey"].as_str().unwrap()).unwrap(),
        &crypto::unb64(f["keyDecryptionNonce"].as_str().unwrap()).unwrap(),
        paris_key,
    )
    .unwrap();
    let meta = crypto::decrypt_blob(
        &crypto::unb64(f["metadata"]["encryptedData"].as_str().unwrap()).unwrap(),
        &crypto::unb64(f["metadata"]["decryptionHeader"].as_str().unwrap()).unwrap(),
        &file_key,
    )
    .unwrap();
    let meta: Value = serde_json::from_slice(&meta).unwrap();
    assert_eq!(meta["title"], "IMG_0200.JPG");
    assert_eq!(meta["creationTime"], 1_682_933_400_000_000i64);
    let res =
        h.c.get(format!("{}/files/download/{}", server.url, f["id"]))
            .header("X-Auth-Token", h.token.as_deref().unwrap())
            .send()
            .await
            .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    let ct = res.bytes().await.unwrap();
    let plain = crypto::decrypt_stream(
        &ct,
        &crypto::unb64(f["file"]["decryptionHeader"].as_str().unwrap()).unwrap(),
        &file_key,
    )
    .unwrap();
    assert_eq!(
        plain,
        std::fs::read(export.join("Photos/IMG_0200.JPG")).unwrap()
    );

    // A fresh journal still uploads nothing: dedup works through the server.
    let s = import(&server.url, &export, &dir.path().join("j2.jsonl")).await;
    assert_eq!((s.uploaded, s.duplicates), (0, 2), "{s:?}");
}
