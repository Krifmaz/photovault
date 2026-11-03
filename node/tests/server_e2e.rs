//! End to end against the real Photovault server (not a mock).
//!
//! Starts `photovault-server` in-process ([`photovault_server::testing`]) on
//! SQLite and local object storage in a temp dir, then:
//!
//! 1. signs up a new account through the real API as the apps do — email
//!    code, key attributes made with the node's libsodium crypto (Argon2id
//!    KEK, sealed master and secret keys, recovery key), then
//!    `/users/srp/setup` + `/users/srp/complete` with the node's SRP client;
//! 2. creates an album and uploads, the way the Ente apps do (per-file key
//!    sealed with the collection key, streamed-encrypted file and thumbnail
//!    PUT to the server's own presigned URLs, encrypted metadata): an image,
//!    a non-H.264 video (needs a streamable preview) and a small H.264 video
//!    (needs none);
//! 3. signs the node in with [`EnteBackend::login`] (scripted prompts), builds
//!    its services with [`Backend::services`] and runs [`Worker::run_once`]
//!    with a fake ML indexer and the real ffmpeg transcoder;
//! 4. checks through the real API that `mldata` decrypts to the indexes,
//!    the status diff lists them, the preview object behind
//!    `/files/data/preview` decrypts with the playlist's key and plays
//!    (ffprobe), and the small H.264 video got `sv = 1`.
//!
//! A second pass must upload nothing. The test is skipped when `ffmpeg` or
//! `ffprobe` is missing.

use std::collections::HashMap;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use async_trait::async_trait;
use photovault_node::api::srp::{self, SrpClient};
use photovault_node::backend::EnteBackend;
use photovault_node::cli::{Backend, LoginRequest, Prompts};
use photovault_node::config::{Config, GpuProvider};
use photovault_node::crypto::{self, EncryptedBlob, SecretKey};
use photovault_node::ml::mldata;
use photovault_node::schedule::{self, GpuJobs};
use photovault_node::state::StateStore;
use photovault_node::video::PlaylistJson;
use photovault_node::worker::{
    LibraryFile, MlIndexOutput, MlIndexer, MlRun, MlVersions, Worker, WorkerOptions,
};
use photovault_server::testing::TestServer;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

const EMAIL: &str = "node-e2e@example.org";
const OTT: &str = "424242";

fn have_ffmpeg() -> bool {
    let ok = |p: &str| {
        Command::new(p)
            .arg("-version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    };
    let have = ok("ffmpeg") && ok("ffprobe");
    if !have {
        eprintln!("skipping: ffmpeg/ffprobe not installed");
    }
    have
}

fn make_clip(out: &Path, codec_args: &[&str]) -> Vec<u8> {
    let mut args = vec![
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "lavfi",
        "-i",
        "testsrc=size=320x240:rate=30:duration=2",
    ];
    args.extend_from_slice(codec_args);
    let st = Command::new("ffmpeg")
        .args(&args)
        .arg(out)
        .status()
        .unwrap();
    assert!(st.success(), "making clip failed");
    std::fs::read(out).unwrap()
}

/// Ente's account KDF limits as the apps use them when 1 GiB is not
/// available (`deriveSensitiveKey` halves memory and doubles passes):
/// museum only accepts `memLimit * opsLimit == 4 GiB` and `memLimit >= 128 MiB`.
/// (The fixture account in `crypto_vectors/` uses 64 MiB x 2, which museum
/// would reject at `PUT /users/attributes`.)
const MEM_LIMIT: u64 = 268_435_456;
const OPS_LIMIT: u32 = 16;
const PASSWORD: &str = "correct horse battery staple";

fn b64(b: &[u8]) -> String {
    crypto::b64_encode(b)
}

fn unb64(v: &Value) -> Vec<u8> {
    crypto::b64_decode(v.as_str().unwrap_or_else(|| panic!("not a string: {v}"))).unwrap()
}

/// A thin JSON client for the real server.
struct Api {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
}

impl Api {
    fn new(base: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base: base.to_owned(),
            token: None,
        }
    }

    async fn call(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let mut rb = self
            .http
            .request(method.clone(), format!("{}{path}", self.base));
        if let Some(t) = &self.token {
            rb = rb.header("X-Auth-Token", t);
        }
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        let r = rb.send().await.unwrap();
        let status = r.status().as_u16();
        let bytes = r.bytes().await.unwrap();
        let v = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, v)
    }

    async fn ok(&self, method: reqwest::Method, path: &str, body: Option<Value>) -> Value {
        let (s, v) = self.call(method.clone(), path, body).await;
        assert_eq!(s, 200, "{method} {path}: {v}");
        v
    }

    async fn get(&self, path: &str) -> Value {
        self.ok(reqwest::Method::GET, path, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Value {
        self.ok(reqwest::Method::POST, path, Some(body)).await
    }

    async fn put(&self, path: &str, body: Value) -> Value {
        self.ok(reqwest::Method::PUT, path, Some(body)).await
    }
}

/// Signup as the apps do it: email code, key attributes generated with the
/// node's libsodium crypto (Ente's scheme), SRP setup. Returns the user id
/// and the master key.
async fn sign_up(api: &mut Api) -> (i64, SecretKey) {
    api.post("/users/ott", json!({"email": EMAIL, "purpose": "signup"}))
        .await;
    let v = api
        .post("/users/verify-email", json!({"email": EMAIL, "ott": OTT}))
        .await;
    api.token = Some(v["token"].as_str().expect("token for a new user").into());
    let user_id = v["id"].as_i64().unwrap();

    let master_key = SecretKey::generate();
    let recovery_key = SecretKey::generate();
    let kek_salt = crypto::random_bytes(16);
    let kek = crypto::derive_key(
        PASSWORD,
        &kek_salt,
        crypto::KdfParams {
            mem_limit: MEM_LIMIT,
            ops_limit: OPS_LIMIT,
        },
    )
    .unwrap();
    let (public_key, secret_key) = crypto::box_keypair();
    let (ek, ek_nonce) = crypto::secretbox_seal(master_key.as_bytes(), &kek).unwrap();
    let (esk, esk_nonce) = crypto::secretbox_seal(secret_key.as_bytes(), &master_key).unwrap();
    let (mkr, mkr_nonce) = crypto::secretbox_seal(master_key.as_bytes(), &recovery_key).unwrap();
    let (rkm, rkm_nonce) = crypto::secretbox_seal(recovery_key.as_bytes(), &master_key).unwrap();
    api.put(
        "/users/attributes",
        json!({"keyAttributes": {
            "kekSalt": b64(&kek_salt),
            "encryptedKey": b64(&ek), "keyDecryptionNonce": b64(&ek_nonce),
            "publicKey": b64(&public_key),
            "encryptedSecretKey": b64(&esk), "secretKeyDecryptionNonce": b64(&esk_nonce),
            "memLimit": MEM_LIMIT, "opsLimit": OPS_LIMIT,
            "masterKeyEncryptedWithRecoveryKey": b64(&mkr), "masterKeyDecryptionNonce": b64(&mkr_nonce),
            "recoveryKeyEncryptedWithMasterKey": b64(&rkm), "recoveryKeyDecryptionNonce": b64(&rkm_nonce),
        }}),
    )
    .await;

    // SRP: identity is a fresh UUID, password the login sub-key of the KEK.
    let identity = format!(
        "{}-{}-{}-{}-{}",
        hex::encode(crypto::random_bytes(4)),
        hex::encode(crypto::random_bytes(2)),
        hex::encode(crypto::random_bytes(2)),
        hex::encode(crypto::random_bytes(2)),
        hex::encode(crypto::random_bytes(6))
    );
    let salt = crypto::random_bytes(16);
    let login_key = crypto::derive_login_key(&kek).unwrap();
    let mut srp = SrpClient::new(&identity, &salt, &login_key);
    let setup = api
        .post(
            "/users/srp/setup",
            json!({"srpUserID": identity, "srpSalt": b64(&salt),
                   "srpVerifier": b64(&srp::verifier(&salt, identity.as_bytes(), &login_key)),
                   "srpA": b64(srp.public_a())}),
        )
        .await;
    let m1 = srp.compute_m1(&unb64(&setup["srpB"])).unwrap();
    let done = api
        .post(
            "/users/srp/complete",
            json!({"setupID": setup["setupID"], "srpM1": b64(&m1)}),
        )
        .await;
    srp.verify_m2(&unb64(&done["srpM2"])).unwrap();
    (user_id, master_key)
}

/// Upload `data` to a fresh presigned URL; returns the object key.
async fn upload_object(api: &Api, data: &[u8]) -> String {
    let v = api.get("/files/upload-urls?count=1").await;
    let u = &v["urls"][0];
    let r = api
        .http
        .put(u["url"].as_str().unwrap())
        .header(
            "Content-MD5",
            photovault_node::api::files::content_md5(data),
        )
        .body(data.to_vec())
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "object PUT: {}", r.status());
    u["objectKey"].as_str().unwrap().to_owned()
}

/// One uploaded file and its key.
struct Uploaded {
    id: i64,
    key: SecretKey,
}

/// Encrypts and uploads a file into collection `cid` like the Ente apps.
async fn upload_file(
    api: &Api,
    cid: i64,
    ckey: &SecretKey,
    metadata: Value,
    original: &[u8],
    thumbnail: &[u8],
) -> Uploaded {
    let key = SecretKey::generate();
    let (ek, nonce) = crypto::secretbox_seal(key.as_bytes(), ckey).unwrap();
    let md = crypto::encrypt_blob(&serde_json::to_vec(&metadata).unwrap(), &key).unwrap();
    let orig = crypto::encrypt_file(original, &key).unwrap();
    let thumb = crypto::encrypt_file(thumbnail, &key).unwrap();
    let fkey = upload_object(api, &orig.encrypted_data).await;
    let tkey = upload_object(api, &thumb.encrypted_data).await;
    let v = api
        .post(
            "/files",
            json!({
                "collectionID": cid,
                "encryptedKey": b64(&ek),
                "keyDecryptionNonce": b64(&nonce),
                "file": {"objectKey": fkey, "decryptionHeader": b64(&orig.decryption_header),
                         "size": orig.encrypted_data.len()},
                "thumbnail": {"objectKey": tkey, "decryptionHeader": b64(&thumb.decryption_header),
                              "size": thumb.encrypted_data.len()},
                "metadata": {"encryptedData": b64(&md.encrypted_data),
                             "decryptionHeader": b64(&md.decryption_header)},
            }),
        )
        .await;
    Uploaded {
        id: v["id"].as_i64().unwrap(),
        key,
    }
}

/// Answers the sign-in prompts: the fixture password and the email code.
struct Scripted {
    password: String,
    asked: Mutex<Vec<String>>,
}

impl Prompts for Scripted {
    fn ask(&self, question: &str) -> anyhow::Result<String> {
        self.asked.lock().unwrap().push(question.to_owned());
        Ok(OTT.to_owned())
    }
    fn ask_secret(&self, question: &str) -> anyhow::Result<String> {
        self.asked.lock().unwrap().push(question.to_owned());
        Ok(self.password.clone())
    }
    fn info(&self, _message: &str) {}
}

/// Returns fixed indexes and records what it was asked to index.
#[derive(Default)]
struct FakeIndexer {
    seen: Mutex<HashMap<i64, Vec<u8>>>,
}

#[async_trait]
impl MlIndexer for FakeIndexer {
    fn versions(&self) -> MlVersions {
        MlVersions { face: 1, clip: 1 }
    }
    async fn index(
        &self,
        file: &LibraryFile,
        image: &Path,
        run: MlRun,
    ) -> anyhow::Result<MlIndexOutput> {
        self.seen
            .lock()
            .unwrap()
            .insert(file.id, std::fs::read(image)?);
        Ok(MlIndexOutput {
            face: run.faces.then(|| face_index(file.id)),
            clip: run.clip.then(clip_index),
        })
    }
}

fn face_index(id: i64) -> Value {
    json!({"version": 1, "client": "test/1", "width": 320, "height": 240,
           "faces": [{"faceID": format!("{id}_0"), "score": 0.9, "blur": 10.5,
                      "embedding": [0.1, 0.2],
                      "detection": {"box": {"x": 0.1, "y": 0.1, "width": 0.2, "height": 0.2},
                                    "landmarks": []}}]})
}

fn clip_index() -> Value {
    json!({"version": 1, "client": "test/1", "embedding": [0.5, -0.25]})
}

/// Decrypts an HLS object with the playlist template's AES-128 key and IV,
/// segment by segment as a player does, and returns the plaintext MPEG-TS.
fn decrypt_hls(template: &str, object: &[u8]) -> Vec<u8> {
    let mut key: Option<[u8; 16]> = None;
    let mut iv: Option<[u8; 16]> = None;
    let mut ranges = Vec::new();
    for line in template.lines() {
        if let Some(k) =
            line.strip_prefix("#EXT-X-KEY:METHOD=AES-128,URI=\"data:text/plain;base64,")
        {
            let (kb64, ivhex) = k.split_once("\",IV=0x").expect("IV in key line");
            key = Some(crypto::b64_decode(kb64).unwrap().try_into().unwrap());
            iv = Some(hex::decode(ivhex).unwrap().try_into().unwrap());
        } else if let Some(br) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            let (n, o) = br.split_once('@').unwrap();
            ranges.push((o.parse::<usize>().unwrap(), n.parse::<usize>().unwrap()));
        }
    }
    let (key, iv) = (key.expect("key"), iv.expect("iv"));
    assert!(!ranges.is_empty());
    let mut next = 0;
    let mut plain = Vec::new();
    for (off, len) in ranges {
        assert_eq!(off, next, "segments tile the object");
        next = off + len;
        let seg = cbc::Decryptor::<aes::Aes128>::new(&key.into(), &iv.into())
            .decrypt_padded_vec_mut::<Pkcs7>(&object[off..next])
            .expect("segment decrypts with the playlist key");
        assert_eq!(seg[0], 0x47, "MPEG-TS");
        plain.extend(seg);
    }
    assert_eq!(next, object.len());
    plain
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn node_against_real_server() {
    if !have_ffmpeg() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .with_test_writer()
        .try_init();
    let tmp = tempfile::tempdir().unwrap();
    let srv = TestServer::start_with(|c| {
        c.hardcoded_ott.insert(EMAIL.into(), OTT.into());
    })
    .await
    .unwrap();
    // --- account, album, uploads (as the apps do) ---------------------------
    let mut api = Api::new(&srv.url);
    let (user_id, master_key) = sign_up(&mut api).await;

    let ckey = SecretKey::generate();
    let (cek, cnonce) = crypto::secretbox_seal(ckey.as_bytes(), &master_key).unwrap();
    let (cname, cname_nonce) = crypto::secretbox_seal(b"Node test", &ckey).unwrap();
    let col = api
        .post(
            "/collections",
            json!({"encryptedKey": b64(&cek), "keyDecryptionNonce": b64(&cnonce),
                   "encryptedName": b64(&cname), "nameDecryptionNonce": b64(&cname_nonce),
                   "type": "album", "attributes": {}}),
        )
        .await;
    let cid = col["collection"]["id"].as_i64().unwrap();

    let image = b"\xFF\xD8\xFF\xE0JPEG-STAND-IN\xFF\xD9".to_vec();
    let mpeg4 = make_clip(
        &tmp.path().join("a.mp4"),
        &["-c:v", "mpeg4", "-q:v", "2", "-pix_fmt", "yuv420p"],
    );
    let h264 = make_clip(
        &tmp.path().join("b.mp4"),
        &["-c:v", "libx264", "-pix_fmt", "yuv420p"],
    );
    let video_md = |t: &str| {
        json!({"title": t, "fileType": 1, "duration": 2,
               "creationTime": 1_700_000_000_000_000_i64})
    };
    let img = upload_file(
        &api,
        cid,
        &ckey,
        json!({"title": "IMG_1.JPG", "fileType": 0, "creationTime": 1_700_000_000_000_000_i64}),
        &image,
        b"THUMB-IMG",
    )
    .await;
    let vid = upload_file(&api, cid, &ckey, video_md("a.mp4"), &mpeg4, b"THUMB-VID").await;
    let small = upload_file(&api, cid, &ckey, video_md("b.mp4"), &h264, b"THUMB-SMALL").await;
    let keys: HashMap<i64, &SecretKey> = [&img, &vid, &small]
        .iter()
        .map(|f| (f.id, &f.key))
        .collect();

    // --- sign the node in and run one pass -----------------------------------
    let cfg = Config {
        server_url: srv.url.clone(),
        data_dir: tmp.path().join("node"),
        ..Default::default()
    };
    let prompts = Scripted {
        password: PASSWORD.into(),
        asked: Mutex::default(),
    };
    let backend = EnteBackend::new();
    let summary = backend
        .login(
            &cfg,
            &LoginRequest {
                server_url: srv.url.clone(),
                email: EMAIL.into(),
            },
            &prompts,
        )
        .await
        .expect("node signs in with SRP against the real server");
    assert_eq!(summary.user_id, user_id);
    assert_eq!(*prompts.asked.lock().unwrap(), ["Password: "]);

    // Real services; the ML indexer is replaced by the fake (the real one
    // needs ONNX Runtime), so build without ML tasks and add it.
    let mut services_cfg = cfg.clone();
    services_cfg.tasks.faces = false;
    services_cfg.tasks.clip = false;
    let mut services = backend
        .services(&services_cfg, GpuProvider::Cpu)
        .await
        .unwrap();
    assert!(services.video.is_some());
    let indexer = Arc::new(FakeIndexer::default());
    services.ml = Some(indexer.clone());
    let mut opts = WorkerOptions::from_config(&cfg);
    opts.save_every = Duration::ZERO;
    let store = StateStore::load(cfg.state_path()).unwrap();
    let (_gate_tx, gate) = schedule::always_run();
    let mut worker = Worker::new(services, opts, store, gate, GpuJobs::default());
    let pass = worker.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(pass.files, 3, "{pass:?}");
    assert_eq!(pass.failed, 0, "{pass:?}");
    // ML for 3 files; video for the MPEG-4 (preview) and the H.264 (sv = 1).
    assert_eq!(pass.completed, 5, "{pass:?}");
    {
        let seen = indexer.seen.lock().unwrap();
        assert_eq!(seen[&img.id], image, "image indexed from the original");
        assert_eq!(
            seen[&vid.id], b"THUMB-VID",
            "video indexed from the thumbnail"
        );
    }

    // --- verify through the real API ------------------------------------------
    let ids: Vec<i64> = keys.keys().copied().collect();
    let v = api
        .post(
            "/files/data/fetch",
            json!({"fileIDs": ids, "type": "mldata"}),
        )
        .await;
    let data = v["data"].as_array().unwrap();
    assert_eq!(data.len(), 3, "{v}");
    for d in data {
        let id = d["fileID"].as_i64().unwrap();
        let m = mldata::open(
            &unb64(&d["encryptedData"]),
            &unb64(&d["decryptionHeader"]),
            keys[&id],
            None,
        )
        .unwrap();
        assert_eq!(
            Value::Object(m.raw),
            json!({"face": face_index(id), "clip": clip_index()})
        );
    }

    let v = api
        .post("/files/data/status-diff", json!({"lastUpdatedAt": 0}))
        .await;
    let diff = v["diff"].as_array().unwrap();
    let mut listed: Vec<(i64, String)> = diff
        .iter()
        .map(|e| {
            (
                e["fileID"].as_i64().unwrap(),
                e["type"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    listed.sort();
    let mut want: Vec<(i64, String)> = ids.iter().map(|i| (*i, "mldata".into())).collect();
    want.push((vid.id, "vid_preview".into()));
    want.sort();
    assert_eq!(listed, want, "{v}");

    // Streamable video: playlist from file data, object from the preview URL.
    let v = api
        .get(&format!(
            "/files/data/fetch?fileID={}&type=vid_preview",
            vid.id
        ))
        .await;
    let pl = PlaylistJson::decrypt(
        &EncryptedBlob {
            encrypted_data: unb64(&v["data"]["encryptedData"]),
            decryption_header: unb64(&v["data"]["decryptionHeader"]),
        },
        &vid.key,
    )
    .unwrap();
    assert_eq!(pl.kind, "hls_video");
    assert_eq!((pl.width, pl.height), (320, 240));
    let v = api
        .get(&format!(
            "/files/data/preview?fileID={}&type=vid_preview",
            vid.id
        ))
        .await;
    let object = reqwest::get(v["url"].as_str().unwrap())
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(object.len() as u64, pl.size);
    let ts = decrypt_hls(&pl.playlist, &object);
    let ts_path = tmp.path().join("preview.ts");
    std::fs::write(&ts_path, ts).unwrap();
    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries"])
        .args(["stream=codec_name,width,height", "-of", "json"])
        .arg(&ts_path)
        .output()
        .unwrap();
    assert!(probe.status.success());
    let probe: Value = serde_json::from_slice(&probe.stdout).unwrap();
    assert_eq!(probe["streams"][0]["codec_name"], "h264", "{probe}");
    // And it decodes end to end.
    let st = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&ts_path)
        .args(["-f", "null", "-"])
        .status()
        .unwrap();
    assert!(st.success(), "preview plays");
    let (s, _) = api
        .call(
            reqwest::Method::GET,
            &format!(
                "/files/data/fetch?fileID={}&type=vid_preview&preferNoContent=true",
                small.id
            ),
            None,
        )
        .await;
    assert_eq!(s, 204, "small H.264 got no preview");

    // Small H.264: `sv = 1` in its public magic metadata.
    let v = api
        .get(&format!(
            "/collections/v2/diff?collectionID={cid}&sinceTime=0"
        ))
        .await;
    let entry = v["diff"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"].as_i64() == Some(small.id))
        .unwrap()
        .clone();
    let pmm = &entry["pubMagicMetadata"];
    assert_eq!(pmm["count"], 1, "{entry}");
    let plain =
        crypto::decrypt_blob(&unb64(&pmm["data"]), &unb64(&pmm["header"]), &small.key).unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&plain).unwrap(),
        json!({"sv": 1})
    );

    // --- a second pass uploads nothing -----------------------------------------
    let pass = worker.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(pass.completed, 0, "{pass:?}");
    assert_eq!(pass.failed, 0, "{pass:?}");
    let v = api
        .post("/files/data/status-diff", json!({"lastUpdatedAt": 0}))
        .await;
    let after: Vec<i64> = v["diff"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["updatedAt"].as_i64().unwrap())
        .collect();
    let before: Vec<i64> = diff
        .iter()
        .map(|e| e["updatedAt"].as_i64().unwrap())
        .collect();
    assert_eq!(after, before, "second pass changed file data");
    let v = api
        .get(&format!(
            "/collections/v2/diff?collectionID={cid}&sinceTime=0"
        ))
        .await;
    let again = v["diff"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"].as_i64() == Some(small.id))
        .unwrap()
        .clone();
    assert_eq!(again["updationTime"], entry["updationTime"]);
}
