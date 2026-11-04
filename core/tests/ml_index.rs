//! ML sync and indexing against the real Photovault server, in process:
//! `mldata` written by "another device" (this test, with Ente's format and
//! the file key) is pulled and decrypted by a Core; an index computed by a
//! Core (with a stand-in for the models) is uploaded and read back by
//! another device and another Core; the `faceSearchEnabled` flag round
//! trips through `/remote-store`.

mod support;

use photovault_core::api::Api;
use photovault_core::crypto;
use photovault_core::ml::index::{Analysis, AnalyzeError, Analyzer, ImageInput};
use photovault_core::ml::mldata::{self, Face, RemoteMlData};
use reqwest::Method;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use support::{TestCore, jpeg, server, signup};

const EMAIL: &str = "ml@example.org";
const PASSWORD: &str = "correct horse battery staple";

/// File keys as another device derives them: master key -> collection
/// key -> file key, straight from the API.
async fn file_keys(api: &Api, master_key: &[u8]) -> HashMap<i64, Vec<u8>> {
    let cols: Value = api
        .get_json("/collections/v2", &[("sinceTime", "0".into())])
        .await
        .unwrap();
    let mut out = HashMap::new();
    for c in cols["collections"].as_array().unwrap() {
        let ck = crypto::secretbox_open(
            &crypto::unb64(c["encryptedKey"].as_str().unwrap()).unwrap(),
            &crypto::unb64(c["keyDecryptionNonce"].as_str().unwrap()).unwrap(),
            master_key,
        )
        .unwrap();
        let diff: Value = api
            .get_json(
                "/collections/v2/diff",
                &[
                    ("collectionID", c["id"].to_string()),
                    ("sinceTime", "0".into()),
                ],
            )
            .await
            .unwrap();
        for f in diff["diff"].as_array().unwrap() {
            if f["isDeleted"].as_bool() == Some(true) {
                continue;
            }
            let fk = crypto::secretbox_open(
                &crypto::unb64(f["encryptedKey"].as_str().unwrap()).unwrap(),
                &crypto::unb64(f["keyDecryptionNonce"].as_str().unwrap()).unwrap(),
                &ck,
            )
            .unwrap();
            out.insert(f["id"].as_i64().unwrap(), fk);
        }
    }
    out
}

/// `PUT /files/data` like an Ente app.
async fn put_mldata(api: &Api, file_id: i64, key: &[u8], raw: &Map<String, Value>, last: i64) {
    let (ct, header) = mldata::seal(raw, key).unwrap();
    api.json_no_content(
        Method::PUT,
        "/files/data",
        &json!({
            "fileID": file_id,
            "type": "mldata",
            "encryptedData": crypto::b64(&ct),
            "decryptionHeader": crypto::b64(&header),
            "lastUpdatedAt": last,
        }),
    )
    .await
    .unwrap();
}

/// `POST /files/data/fetch` + decrypt, like an Ente app.
async fn fetch_mldata(api: &Api, file_id: i64, key: &[u8]) -> Option<RemoteMlData> {
    let v: Value = api
        .json(
            Method::POST,
            "/files/data/fetch",
            &json!({"type": "mldata", "fileIDs": [file_id]}),
        )
        .await
        .unwrap();
    let d = v["data"].as_array()?.first()?.clone();
    Some(
        mldata::open(
            &crypto::unb64(d["encryptedData"].as_str().unwrap()).unwrap(),
            &crypto::unb64(d["decryptionHeader"].as_str().unwrap()).unwrap(),
            key,
            d["updatedAt"].as_i64(),
        )
        .unwrap(),
    )
}

fn face(file_id: i64) -> Face {
    Face::from_rust_result(
        format!("{file_id}_10000_20000_50000_70000"),
        [0.1, 0.2, 0.5, 0.7],
        &[[0.2, 0.3], [0.4, 0.3], [0.3, 0.4], [0.25, 0.5], [0.35, 0.5]],
        0.9,
        150.0,
        &[0.25; 192],
    )
}

fn complete_mldata(file_id: i64, client: &str) -> Map<String, Value> {
    let mut f = serde_json::to_value(mldata::RemoteFaceIndex::new(
        640,
        480,
        vec![face(file_id)],
        0,
    ))
    .unwrap();
    f["client"] = json!(client);
    let mut c = serde_json::to_value(mldata::RemoteClipIndex::new(vec![0.5; 512], 0)).unwrap();
    c["client"] = json!(client);
    let mut m = Map::new();
    m.insert("face".into(), f);
    m.insert("clip".into(), c);
    m.insert("futureKey".into(), json!({"keep": true}));
    m
}

/// A stand-in for Ente's models: checks it got real pixels, returns one
/// face and a CLIP embedding, fails for chosen files.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<(i64, bool, bool)>>,
    fail: Mutex<Vec<i64>>,
}

impl Analyzer for Fake {
    fn analyze(
        &self,
        file_id: i64,
        input: ImageInput,
        run_faces: bool,
        run_clip: bool,
    ) -> Result<Analysis, AnalyzeError> {
        self.calls
            .lock()
            .unwrap()
            .push((file_id, run_faces, run_clip));
        if self.fail.lock().unwrap().contains(&file_id) {
            return Err(AnalyzeError::Image("cannot decode".into()));
        }
        let bytes = match input {
            ImageInput::Path(p) => {
                // A decrypted temporary file in the cache directory.
                assert!(p.to_string_lossy().contains("ml-tmp"), "{p:?}");
                std::fs::read(p).unwrap()
            }
            ImageInput::Bytes(b) => b,
        };
        let img =
            image::load_from_memory(&bytes).map_err(|e| AnalyzeError::Image(e.to_string()))?;
        Ok(Analysis {
            width: img.width(),
            height: img.height(),
            faces: run_faces.then(|| vec![face(file_id)]),
            clip: run_clip.then(|| vec![0.125; 512]),
            flags: mldata::flags::for_rust_result(false, false),
        })
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mldata_sync_indexing_and_the_enabled_flag() {
    let srv = server(&[EMAIL]).await;
    let acc = signup(&srv.url, EMAIL, PASSWORD).await;
    let raw_api = Api::new(&srv.url).with_token(acc.token.clone());

    // Device A uploads three photos.
    let a = TestCore::connected(&srv.url).await;
    a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap();
    for (i, name) in ["one.jpg", "two.jpg", "three.jpg"].iter().enumerate() {
        let p = a.dir.path().join(name);
        std::fs::write(&p, jpeg(320 + 16 * i as u32, 240, i as u8)).unwrap();
        a.core
            .enqueue_upload(p.display().to_string(), None)
            .unwrap();
    }
    let st = a.core.run_uploads().await.unwrap();
    assert_eq!(st.failed, 0, "{st:?}");
    a.core.sync(None).await.unwrap();
    let ids: HashMap<String, i64> = a
        .core
        .timeline_page(0, 10)
        .into_iter()
        .map(|i| (i.title, i.id))
        .collect();
    let (one, two, three) = (ids["one.jpg"], ids["two.jpg"], ids["three.jpg"]);
    let keys = file_keys(&raw_api, &acc.master_key).await;
    assert_eq!(keys.len(), 3);

    // Device B, a second Core.
    let b = TestCore::connected(&srv.url).await;
    b.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap();
    b.core.sync(None).await.unwrap();

    // ML is off by default: nothing is pulled.
    let s = b.core.sync_ml(None).await.unwrap();
    assert!(!s.enabled, "{s:?}");
    assert_eq!(s.inference_available, cfg!(feature = "ml"));
    assert_eq!(s.files_indexed, 0);

    // "Another device" (an Ente app) uploads complete mldata for `one`
    // and only a CLIP index for `two`.
    put_mldata(
        &raw_api,
        one,
        &keys[&one],
        &complete_mldata(one, "io.ente.photos/1.0"),
        0,
    )
    .await;
    let mut clip_only = complete_mldata(two, "io.ente.photos/1.0");
    clip_only.remove("face");
    put_mldata(&raw_api, two, &keys[&two], &clip_only, 0).await;

    // A turns ML on; B sees the flag on its next sync and pulls the data.
    let s = a.core.set_ml_enabled(true).await.unwrap();
    assert!(s.enabled);
    let flag: Value = raw_api
        .get_json(
            "/remote-store",
            &[
                ("key", "faceSearchEnabled".into()),
                ("defaultValue", "false".into()),
            ],
        )
        .await
        .unwrap();
    assert_eq!(flag["value"], "true");
    let seen = Arc::new(Mutex::new(Vec::new()));
    struct L(Arc<Mutex<Vec<photovault_core::ml::MlStatus>>>);
    impl photovault_core::ml::MlListener for L {
        fn on_status(&self, s: photovault_core::ml::MlStatus) {
            self.0.lock().unwrap().push(s);
        }
    }
    let s = b
        .core
        .sync_ml(Some(Arc::new(L(seen.clone()))))
        .await
        .unwrap();
    assert!(s.enabled, "{s:?}");
    assert_eq!(s.files_total, 3, "{s:?}");
    assert_eq!(s.files_indexed, 1, "{s:?}");
    // `two` (face index missing) and `three` (nothing) are B's to index.
    assert_eq!(s.files_pending, 2, "{s:?}");
    assert!(!s.running);
    assert!(!seen.lock().unwrap().is_empty());
    assert_eq!(b.core.ml_status(), s);
    // A second sync is a no-op.
    assert_eq!(b.core.sync_ml(None).await.unwrap(), s);

    // Nothing in B's store is plaintext.
    let db = std::fs::read_dir(b.data_dir())
        .unwrap()
        .flatten()
        .map(|e| std::fs::read(e.path()).unwrap_or_default())
        .collect::<Vec<_>>()
        .concat();
    let text = String::from_utf8_lossy(&db);
    assert!(!text.contains("io.ente.photos"));
    assert!(!text.contains("futureKey"));

    // B indexes with a stand-in for the models; `three` fails once.
    let fake = Arc::new(Fake::default());
    fake.fail.lock().unwrap().push(three);
    let s = b.core.run_ml_indexing_with(10, fake.clone()).await.unwrap();
    {
        let calls = fake.calls.lock().unwrap();
        // Only the face model runs for `two` (its CLIP index is current).
        assert!(calls.contains(&(two, true, false)), "{calls:?}");
        assert!(calls.contains(&(three, true, true)), "{calls:?}");
        assert!(!calls.iter().any(|c| c.0 == one), "{calls:?}");
    }
    assert_eq!(s.files_indexed, 2, "{s:?}");
    assert_eq!(s.files_pending, 1, "{s:?}");
    assert_eq!(s.last_error, None);
    assert!(!b.cache_dir().join("ml-tmp").exists());

    // The upload is Ente's format: merged into the existing object
    // (unknown keys and the other app's CLIP index kept), readable by an
    // Ente app with the file key.
    let d = fetch_mldata(&raw_api, two, &keys[&two]).await.unwrap();
    assert!(d.plan().is_done(), "{:?}", d.raw);
    assert_eq!(d.raw["futureKey"], json!({"keep": true}));
    let parsed = d.parsed.as_ref().unwrap();
    let f = parsed.face.as_ref().unwrap();
    assert!(
        f.client.starts_with("app.photovault.native/"),
        "{}",
        f.client
    );
    assert_eq!(f.flags, mldata::flags::RUNTIME_RUST);
    assert_eq!((f.width, f.height), (336.0, 240.0));
    assert_eq!(f.faces, vec![face(two)]);
    let c = parsed.clip.as_ref().unwrap();
    assert_eq!(c.client, "io.ente.photos/1.0");
    assert_eq!(c.embedding, vec![0.5; 512]);
    assert!(fetch_mldata(&raw_api, three, &keys[&three]).await.is_none());

    // `three` is retried until it has failed three times, then left alone.
    for _ in 0..2 {
        let s = b.core.run_ml_indexing_with(10, fake.clone()).await.unwrap();
        assert_eq!(s.files_indexed, 2, "{s:?}");
    }
    assert_eq!(b.core.ml_status().files_pending, 0);
    let before = fake.calls.lock().unwrap().len();
    b.core.run_ml_indexing_with(10, fake.clone()).await.unwrap();
    assert_eq!(fake.calls.lock().unwrap().len(), before);
    let tries = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.0 == three)
        .count();
    assert_eq!(tries, 3);

    // A pulls B's index (and the other app's data) through the status diff.
    let s = a.core.sync_ml(None).await.unwrap();
    assert_eq!(s.files_indexed, 2, "{s:?}");
    // A indexes nothing B already did; `three` is still A's to try.
    let fake_a = Arc::new(Fake::default());
    let s = a
        .core
        .run_ml_indexing_with(10, fake_a.clone())
        .await
        .unwrap();
    assert_eq!(*fake_a.calls.lock().unwrap(), vec![(three, true, true)]);
    assert_eq!(s.files_indexed, 3, "{s:?}");
    assert_eq!(s.files_pending, 0, "{s:?}");
    let d = fetch_mldata(&raw_api, three, &keys[&three]).await.unwrap();
    assert!(d.plan().is_done());
    assert_eq!(d.parsed.unwrap().clip.unwrap().embedding, vec![0.125; 512]);

    // An update from another device replaces B's copy.
    let mut newer = complete_mldata(one, "io.ente.photos/2.0");
    newer.insert("face".into(), {
        let mut f =
            serde_json::to_value(mldata::RemoteFaceIndex::new(640, 480, vec![], 0)).unwrap();
        f["client"] = json!("io.ente.photos/2.0");
        f
    });
    let last = fetch_mldata(&raw_api, one, &keys[&one])
        .await
        .unwrap()
        .updated_at
        .unwrap();
    put_mldata(&raw_api, one, &keys[&one], &newer, last).await;
    let s = b.core.sync_ml(None).await.unwrap();
    assert_eq!(s.files_indexed, 3, "{s:?}");

    // Turning ML off is seen by the other Core too.
    let s = b.core.set_ml_enabled(false).await.unwrap();
    assert!(!s.enabled);
    let s = a.core.sync_ml(None).await.unwrap();
    assert!(!s.enabled);
    assert!(!a.core.ml_status().enabled);
    // While off, indexing does nothing.
    let s = a
        .core
        .run_ml_indexing_with(10, fake_a.clone())
        .await
        .unwrap();
    assert!(!s.running);
    assert_eq!(fake_a.calls.lock().unwrap().len(), 1);
}
