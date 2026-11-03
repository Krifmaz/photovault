//! End to end: the real backend adapters ([`photovault_node::backend`])
//! driven by [`Worker::run_once`] against a mock museum.
//!
//! The library mixes the Ente-encrypted fixture account (collections 10 and
//! 20 from `crypto_vectors/ente_core_vectors.json`, encrypted by Ente's own
//! Rust core) with an owned collection 30 encrypted here the same way,
//! holding: a non-H.264 video (gets a streamable preview), a small H.264
//! video (needs none: `sv = 1` is set through public magic metadata), a live
//! photo (indexed from the still image inside its zip), and a video that
//! already has a preview on the server.
//!
//! ML runs through a fake indexer (the real one needs ONNX Runtime); video
//! uses the real `ffmpeg`. The test is skipped when `ffmpeg` is missing.
//! Everything uploaded is decrypted with the file keys and checked: `mldata`
//! must be gzipped JSON that keeps the keys already on the server, and the
//! `vid_preview` playlist must match the uploaded object.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use md5::Digest;
use photovault_node::api::{ApiClient, RetryPolicy, Session};
use photovault_node::backend::{EnteTranscoder, Shared};
use photovault_node::config::Config;
use photovault_node::crypto::{self, EncryptedBlob, SecretKey};
use photovault_node::ml::mldata;
use photovault_node::schedule::{self, GpuJobs};
use photovault_node::state::StateStore;
use photovault_node::video::{FfmpegConfig, PlaylistJson};
use photovault_node::worker::{
    LibraryFile, MlIndexOutput, MlIndexer, MlRun, MlVersions, Worker, WorkerOptions,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wiremock::matchers::{body_partial_json, method, path, query_param};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

const OBJECT_ID: &str = "pv_ABCDEFGHIJKLMNOPQRSTUV";

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

fn account() -> Value {
    let v: Value =
        serde_json::from_str(include_str!("crypto_vectors/ente_core_vectors.json")).unwrap();
    v["account"].clone()
}

fn session(server: &MockServer) -> Session {
    let a = account();
    Session {
        endpoint: server.uri(),
        email: "me@example.org".into(),
        user_id: 1,
        token: "dG9rZW4=".into(),
        master_key: SecretKey::from_b64(a["master_key_b64"].as_str().unwrap()).unwrap(),
        public_key: a["key_attributes"]["publicKey"].as_str().unwrap().into(),
        secret_key: SecretKey::from_b64(a["secret_key_b64"].as_str().unwrap()).unwrap(),
    }
}

fn b64(b: &[u8]) -> String {
    crypto::b64_encode(b)
}

/// One file of collection 30, encrypted as the Ente apps do.
struct TestFile {
    id: i64,
    key: SecretKey,
    entry: Value,
    original: Vec<u8>,
    thumbnail: Vec<u8>,
}

fn encrypt_file(
    id: i64,
    collection_key: &SecretKey,
    metadata: Value,
    original: &[u8],
    thumbnail: &[u8],
) -> TestFile {
    let key = SecretKey::generate();
    let (ek, nonce) = crypto::secretbox_seal(key.as_bytes(), collection_key).unwrap();
    let md = crypto::encrypt_blob(&serde_json::to_vec(&metadata).unwrap(), &key).unwrap();
    let orig = crypto::encrypt_file(original, &key).unwrap();
    let thumb = crypto::encrypt_file(thumbnail, &key).unwrap();
    let entry = json!({
        "id": id,
        "ownerID": 1,
        "collectionID": 30,
        "encryptedKey": b64(&ek),
        "keyDecryptionNonce": b64(&nonce),
        "file": {"decryptionHeader": b64(&orig.decryption_header)},
        "thumbnail": {"decryptionHeader": b64(&thumb.decryption_header)},
        "metadata": {"encryptedData": b64(&md.encrypted_data),
                     "decryptionHeader": b64(&md.decryption_header)},
        "info": {"fileSize": original.len(), "thumbSize": thumbnail.len()},
        "isDeleted": false,
        "updationTime": 2000 + id,
    });
    TestFile {
        id,
        key,
        entry,
        original: orig.encrypted_data,
        thumbnail: thumb.encrypted_data,
    }
}

fn live_photo_zip(image: &[u8], video: &[u8]) -> Vec<u8> {
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let o = zip::write::SimpleFileOptions::default();
        w.start_file("image.jpg", o).unwrap();
        w.write_all(image).unwrap();
        w.start_file("video.mov", o).unwrap();
        w.write_all(video).unwrap();
        w.finish().unwrap();
    }
    buf.into_inner()
}

/// Records what it was asked to index; returns fixed indexes.
#[derive(Default)]
struct FakeIndexer {
    seen: Mutex<HashMap<i64, (Vec<u8>, MlRun)>>,
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
        let bytes = std::fs::read(image)?;
        self.seen.lock().unwrap().insert(file.id, (bytes, run));
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

fn body_json(r: &Request) -> Value {
    serde_json::from_slice(&r.body).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn worker_pass_with_real_adapters() {
    if !have_ffmpeg() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    let s = session(&server);
    let a = account();
    let lib = &a["library"];

    // --- collection 30 (owned) and its files -------------------------------
    let ckey = SecretKey::generate();
    let (cek, cnonce) = crypto::secretbox_seal(ckey.as_bytes(), &s.master_key).unwrap();
    let col30 = json!({"id": 30, "owner": {"id": 1, "email": "me@example.org"},
        "encryptedKey": b64(&cek), "keyDecryptionNonce": b64(&cnonce),
        "type": "album", "sharees": [], "updationTime": 1000});

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
    let f300 = encrypt_file(300, &ckey, video_md("a.mp4"), &mpeg4, b"THUMB300");
    // Small H.264 with existing public magic metadata (kept on update).
    let mut f500 = encrypt_file(500, &ckey, video_md("b.mp4"), &h264, b"THUMB500");
    let pmm = crypto::encrypt_blob(br#"{"w":320,"h":240}"#, &f500.key).unwrap();
    f500.entry["pubMagicMetadata"] = json!({"version": 4, "count": 2,
        "data": b64(&pmm.encrypted_data), "header": b64(&pmm.decryption_header)});
    let still = b"\xFF\xD8STILL-IMAGE\xFF\xD9".to_vec();
    let f400 = encrypt_file(
        400,
        &ckey,
        json!({"title": "IMG_4.HEIC", "fileType": 2, "creationTime": 1}),
        &live_photo_zip(&still, b"MOV"),
        b"THUMB400",
    );
    // A video another client already made streamable.
    let f600 = encrypt_file(600, &ckey, video_md("c.mp4"), &h264, b"THUMB600");
    let ours = [&f300, &f400, &f500, &f600];

    // --- museum ------------------------------------------------------------
    let mut collections = lib["collections"].as_array().unwrap().clone();
    collections.push(col30);
    Mock::given(method("GET"))
        .and(path("/collections/v2"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({ "collections": collections })),
        )
        .mount(&server)
        .await;
    let mut diffs = lib["diffs"].clone();
    diffs["30"] = Value::Array(ours.iter().map(|f| f.entry.clone()).collect());
    for cid in ["10", "20", "30"] {
        Mock::given(method("GET"))
            .and(path("/collections/v2/diff"))
            .and(query_param("collectionID", cid))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"diff": diffs[cid], "hasMore": false})),
            )
            .mount(&server)
            .await;
    }
    // Downloads (fixture file 100 and ours).
    let mut blobs: Vec<(i64, Vec<u8>, Vec<u8>)> = vec![(
        100,
        crypto::b64_decode(lib["original_100_b64"].as_str().unwrap()).unwrap(),
        crypto::b64_decode(lib["thumbnail_100_b64"].as_str().unwrap()).unwrap(),
    )];
    blobs.extend(
        ours.iter()
            .map(|f| (f.id, f.original.clone(), f.thumbnail.clone())),
    );
    for (id, orig, thumb) in blobs {
        Mock::given(method("GET"))
            .and(path(format!("/files/download/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(orig))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/files/preview/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(thumb))
            .mount(&server)
            .await;
    }
    // Remote mldata for file 100: a current CLIP index and a key from a
    // newer client, both of which must survive the upload.
    let key100 =
        SecretKey::from_slice(&hex::decode(lib["file_keys_hex"]["100"].as_str().unwrap()).unwrap())
            .unwrap();
    let remote100 = json!({"clip": {"version": 1, "client": "web/1", "embedding": [0.75]},
                           "futureKey": {"a": [1, 2]}});
    let sealed = mldata::seal(remote100.as_object().unwrap(), &key100).unwrap();
    Mock::given(method("POST"))
        .and(path("/files/data/fetch"))
        .and(body_partial_json(json!({"type": "mldata"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{
            "fileID": 100, "type": "mldata", "updatedAt": 1234,
            "encryptedData": b64(&sealed.encrypted_data),
            "decryptionHeader": b64(&sealed.decryption_header)}]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/files/data/status-diff"))
        .and(body_partial_json(json!({"lastUpdatedAt": 0})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"diff": [
            {"fileID": 600, "userID": 1, "type": "vid_preview", "isDeleted": false,
             "objectID": "pv_x", "size": 10, "updatedAt": 50},
            {"fileID": 100, "userID": 1, "type": "mldata", "isDeleted": false,
             "size": 10, "updatedAt": 40}]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/files/data/status-diff"))
        .and(body_partial_json(json!({"lastUpdatedAt": 50})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"diff": []})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/files/data"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/files/data/preview-upload-url"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "objectID": OBJECT_ID, "url": format!("{}/storage/upload", server.uri())})))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/storage/upload"))
        .respond_with(ResponseTemplate::new(200).insert_header("ETag", "\"e1\""))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/files/video-data"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path("/files/public-magic-metadata"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    // --- run one pass with the real adapters --------------------------------
    let client = ApiClient::for_session(&s, "0.1.0")
        .unwrap()
        .with_retry(RetryPolicy {
            max_attempts: 2,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(2),
        });
    let ctx = Shared::new(s.clone(), client, 0);
    let indexer = Arc::new(FakeIndexer::default());
    let mut services = ctx.services();
    services.ml = Some(indexer.clone());
    services.video = Some(Arc::new(EnteTranscoder::new(
        ctx.clone(),
        FfmpegConfig::default(),
    )));
    let cfg = Config {
        data_dir: tmp.path().join("data"),
        ..Default::default()
    };
    let mut opts = WorkerOptions::from_config(&cfg);
    opts.save_every = Duration::ZERO;
    let store = StateStore::load(cfg.state_path()).unwrap();
    let (_gate_tx, gate) = schedule::always_run();
    let mut worker = Worker::new(services, opts, store, gate, GpuJobs::default());
    let summary = worker.run_once(CancellationToken::new()).await.unwrap();

    // Owned files only: 100 + ours (fixture file 200 belongs to user 2).
    assert_eq!(summary.files, 5, "{summary:?}");
    assert_eq!(summary.failed, 0, "{summary:?}");
    // ML for 5 files; video for 300 (preview) and 500 (not needed); 600 remote.
    assert_eq!(summary.completed, 7, "{summary:?}");
    assert_eq!(summary.already_remote, 1, "{summary:?}");

    // --- what the indexer saw ---------------------------------------------
    {
        let seen = indexer.seen.lock().unwrap();
        assert_eq!(seen.len(), 5);
        let (img100, run100) = &seen[&100];
        assert_eq!(
            hex::encode(sha2::Sha256::digest(img100)),
            lib["original_100_plain_sha256"].as_str().unwrap()
        );
        assert!(run100.faces && !run100.clip, "CLIP already on server");
        assert_eq!(seen[&400].0, still, "live photo: still image from the zip");
        assert_eq!(
            seen[&300].0, b"THUMB300",
            "videos are indexed from the thumbnail"
        );
    }

    // --- uploads -------------------------------------------------------------
    let reqs = server.received_requests().await.unwrap();
    let keys: HashMap<i64, &SecretKey> = std::iter::once((100, &key100))
        .chain(ours.iter().map(|f| (f.id, &f.key)))
        .collect();
    let ml_puts: Vec<Value> = reqs
        .iter()
        .filter(|r| r.method.as_str() == "PUT" && r.url.path() == "/files/data")
        .map(body_json)
        .collect();
    assert_eq!(ml_puts.len(), 5);
    for b in &ml_puts {
        assert_eq!(b["type"], "mldata");
        let id = b["fileID"].as_i64().unwrap();
        let m = mldata::open(
            &crypto::b64_decode(b["encryptedData"].as_str().unwrap()).unwrap(),
            &crypto::b64_decode(b["decryptionHeader"].as_str().unwrap()).unwrap(),
            keys[&id],
            None,
        )
        .unwrap();
        let got = Value::Object(m.raw);
        if id == 100 {
            assert_eq!(b["lastUpdatedAt"], 1234);
            assert_eq!(
                got,
                json!({"clip": remote100["clip"], "futureKey": remote100["futureKey"],
                       "face": face_index(100)})
            );
        } else {
            assert!(b.get("lastUpdatedAt").is_none_or(Value::is_null));
            assert_eq!(got, json!({"face": face_index(id), "clip": clip_index()}));
        }
    }

    // Streamable video for 300: object uploaded with its MD5, playlist
    // registered and decryptable with the file key.
    let uploads: Vec<&Request> = reqs
        .iter()
        .filter(|r| r.url.path() == "/storage/upload")
        .collect();
    assert_eq!(uploads.len(), 1);
    let object = &uploads[0].body;
    assert!(!uploads[0].headers.contains_key("X-Auth-Token"));
    assert_eq!(
        uploads[0].headers["Content-MD5"].to_str().unwrap(),
        b64(&md5::Md5::digest(object))
    );
    let url_req = reqs
        .iter()
        .find(|r| r.url.path() == "/files/data/preview-upload-url")
        .map(body_json)
        .unwrap();
    assert_eq!(url_req["fileID"], 300);
    assert_eq!(url_req["type"], "vid_preview");
    assert_eq!(url_req["contentLength"], object.len());
    let vd: Vec<Value> = reqs
        .iter()
        .filter(|r| r.url.path() == "/files/video-data")
        .map(body_json)
        .collect();
    assert_eq!(vd.len(), 1);
    assert_eq!(vd[0]["fileID"], 300);
    assert_eq!(vd[0]["objectID"], OBJECT_ID);
    assert_eq!(vd[0]["objectSize"], object.len());
    let pl = PlaylistJson::decrypt(
        &EncryptedBlob {
            encrypted_data: crypto::b64_decode(vd[0]["playlist"].as_str().unwrap()).unwrap(),
            decryption_header: crypto::b64_decode(vd[0]["playlistHeader"].as_str().unwrap())
                .unwrap(),
        },
        &f300.key,
    )
    .unwrap();
    assert_eq!(pl.kind, "hls_video");
    assert_eq!(pl.size, object.len() as u64);
    assert!(pl.width > 0 && pl.height > 0);
    assert!(pl.playlist.contains("#EXT-X-KEY:METHOD=AES-128"));
    // The object is AES ciphertext, not a plain MPEG-TS (sync byte 0x47 every 188 bytes).
    assert!(!(object[0] == 0x47 && object.get(188) == Some(&0x47)));

    // Small H.264 (500): no preview, `sv = 1` merged into its magic metadata.
    let mm: Vec<Value> = reqs
        .iter()
        .filter(|r| r.url.path() == "/files/public-magic-metadata")
        .map(body_json)
        .collect();
    assert_eq!(mm.len(), 1);
    let item = &mm[0]["metadataList"][0];
    assert_eq!(item["id"], 500);
    assert_eq!(item["magicMetadata"]["version"], 4);
    assert_eq!(item["magicMetadata"]["count"], 3);
    let plain = crypto::decrypt_blob(
        &crypto::b64_decode(item["magicMetadata"]["data"].as_str().unwrap()).unwrap(),
        &crypto::b64_decode(item["magicMetadata"]["header"].as_str().unwrap()).unwrap(),
        &f500.key,
    )
    .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&plain).unwrap(),
        json!({"w": 320, "h": 240, "sv": 1})
    );

    // Plaintext temp files are gone.
    let leftovers: Vec<_> = std::fs::read_dir(cfg.tmp_dir())
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");

    // A second pass finds everything done and uploads nothing more.
    let before = server.received_requests().await.unwrap().len();
    let summary = worker.run_once(CancellationToken::new()).await.unwrap();
    assert_eq!(summary.completed, 0, "{summary:?}");
    assert_eq!(summary.failed, 0, "{summary:?}");
    let after = server.received_requests().await.unwrap();
    assert!(
        after[before..].iter().all(|r| r.method.as_str() != "PUT"),
        "second pass uploaded again"
    );
}
