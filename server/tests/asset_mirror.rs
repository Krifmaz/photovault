//! `/photovault/models/*` and `/photovault/music/*`: cache-through mirror of
//! Ente's model and music CDNs, against a fake upstream on localhost.

use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use photovault_server::testing::TestServer;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const TRACK: &[u8] = b"ID3 not really an mp3, but bytes are bytes 0123456789";

struct Upstream {
    url: String,
    hits: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

async fn upstream() -> Upstream {
    let hits = Arc::new(AtomicUsize::new(0));
    let h = hits.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let base = url.clone();
    let app = Router::new()
        .route(
            "/music/tracks.json",
            get(move || {
                let base = base.clone();
                async move {
                    axum::Json(serde_json::json!({"tracks": [
                        {"id": "calm", "url": format!("{base}/music/t/calm.mp3"), "title": "Calm"},
                        {"id": "ext", "url": "https://elsewhere.example/x.mp3"}
                    ]}))
                }
            }),
        )
        .route(
            "/music/t/calm.mp3",
            get(move || {
                h.fetch_add(1, Ordering::SeqCst);
                async { TRACK }
            }),
        )
        .route(
            "/models/cls_fixed_v1.onnx",
            get(|| async { "not the real model" }),
        );
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Upstream { url, hits, task }
}

async fn start(up: &Upstream) -> TestServer {
    let models = format!("{}/models", up.url);
    let music = format!("{}/music", up.url);
    TestServer::start_with(move |c| {
        c.mdns = false;
        c.models_upstream = models;
        c.music_upstream = music;
    })
    .await
    .unwrap()
}

fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::new()
}

#[tokio::test]
async fn music_manifest_and_tracks_are_mirrored() {
    let up = upstream().await;
    let srv = start(&up).await;
    let http = client();

    // Manifest: our track rewritten to this server, the foreign one dropped.
    let r = http
        .get(format!("{}/photovault/music/tracks.json", srv.url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: serde_json::Value = serde_json::from_slice(&r.bytes().await.unwrap()).unwrap();
    let tracks = v["tracks"].as_array().unwrap();
    assert_eq!(tracks.len(), 1, "{v}");
    let track_url = tracks[0]["url"].as_str().unwrap().to_string();
    assert_eq!(
        track_url,
        format!("{}/photovault/music/t/calm.mp3", srv.url)
    );
    assert_eq!(tracks[0]["title"], "Calm");

    // First download goes upstream, the second is served from the cache.
    let r = http.get(&track_url).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let etag = r.headers()["etag"].to_str().unwrap().to_string();
    assert_eq!(etag, format!("\"{}\"", hex::encode(Sha256::digest(TRACK))));
    assert_eq!(r.bytes().await.unwrap().as_ref(), TRACK);
    let r = http.get(&track_url).send().await.unwrap();
    assert_eq!(r.bytes().await.unwrap().as_ref(), TRACK);
    assert_eq!(up.hits.load(Ordering::SeqCst), 1);

    // Range, If-Range and If-None-Match.
    let r = http
        .get(&track_url)
        .header("range", "bytes=4-6")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT.as_u16());
    assert_eq!(r.bytes().await.unwrap().as_ref(), &TRACK[4..=6]);
    let r = http
        .get(&track_url)
        .header("range", "bytes=4-6")
        .header("if-range", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 206);
    let r = http
        .get(&track_url)
        .header("range", "bytes=4-6")
        .header("if-range", "\"other\"")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.bytes().await.unwrap().len(), TRACK.len());
    let r = http
        .get(&track_url)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 304);
    let r = http.head(&track_url).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["content-length"],
        TRACK.len().to_string().as_str()
    );

    // Paths the manifest does not list are refused.
    for p in ["t/other.mp3", "../config/keys.toml", "t/calm.mp3.sha256"] {
        let r = http
            .get(format!("{}/photovault/music/{p}", srv.url))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 404, "{p}");
    }

    // Upstream gone: the cache still serves.
    up.task.abort();
    let r = http.get(&track_url).send().await.unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.bytes().await.unwrap().as_ref(), TRACK);
}

#[tokio::test]
async fn models_are_allow_listed_and_verified() {
    let up = upstream().await;
    let srv = start(&up).await;
    let http = client();

    let r = http
        .get(format!("{}/photovault/models/evil.onnx", srv.url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    let r = http
        .get(format!("{}/photovault/models/../keys.toml", srv.url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);

    // The fake upstream's bytes do not match the pinned size/hash: refused, nothing cached.
    let r = http
        .get(format!("{}/photovault/models/cls_fixed_v1.onnx", srv.url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 502);
    let cached = srv
        .state
        .config
        .config_dir
        .join("assets/models/cls_fixed_v1.onnx");
    assert!(!cached.exists());

    // A file already in the cache is served with the pinned hash as its ETag.
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(&cached, b"cached model bytes").unwrap();
    let r = http
        .get(format!("{}/photovault/models/cls_fixed_v1.onnx", srv.url))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()["etag"],
        "\"378d52a73263828d08d4dda37f1d0aac2e36cbd486fdc6351bf309d515610654\""
    );
    assert!(
        r.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable")
    );
}

#[tokio::test]
async fn info_advertises_the_mirror_and_it_can_be_turned_off() {
    let up = upstream().await;
    let srv = start(&up).await;
    let http = client();
    let v: serde_json::Value = http
        .get(format!("{}/photovault/info", srv.url))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .map(|b| serde_json::from_slice(&b).unwrap())
        .unwrap();
    let features = v["features"].as_array().unwrap();
    assert!(features.iter().any(|f| f == "models"), "{v}");
    assert!(features.iter().any(|f| f == "music"), "{v}");
    assert_eq!(v["modelsUrl"], format!("{}/photovault/models", srv.url));
    assert_eq!(
        v["musicManifestUrl"],
        format!("{}/photovault/music/tracks.json", srv.url)
    );

    let off = TestServer::start_with(|c| {
        c.mdns = false;
        c.asset_mirror = false;
    })
    .await
    .unwrap();
    let v: serde_json::Value = http
        .get(format!("{}/photovault/info", off.url))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .map(|b| serde_json::from_slice(&b).unwrap())
        .unwrap();
    assert!(
        !v["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "models")
    );
    assert!(v["modelsUrl"].is_null());
    let r = http
        .get(format!("{}/photovault/models/cls_fixed_v1.onnx", off.url))
        .send()
        .await
        .unwrap();
    assert_ne!(r.status(), 200);
}
