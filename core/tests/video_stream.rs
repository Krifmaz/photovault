//! Streamable videos end to end against the real Photovault server: upload a
//! video through core, attach an Ente-format `vid_preview` the way Ente's
//! clients do (see `core/src/video/playlist.rs` for the format and the Ente
//! source it follows), sync, and play it through core's loopback server.
//!
//! The preview is a real one: `tests/vectors/ente_hls/` holds ffmpeg's output
//! for Ente's own command line (`generate.sh`), and `segments.txt` the
//! SHA-256 of every segment decrypted independently with OpenSSL.

mod support;

use photovault_core::api::Api;
use photovault_core::{FileKind, LoginStep, crypto};
use reqwest::Method;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::io::Write as _;
use support::{TestCore, server, signup};

const M3U8: &str = include_str!("vectors/ente_hls/output.m3u8");
const TS: &[u8] = include_bytes!("vectors/ente_hls/output.ts");
const MP4: &[u8] = include_bytes!("vectors/ente_hls/src.mp4");
const SEGMENTS: &str = include_str!("vectors/ente_hls/segments.txt");

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .tls_backend_preconfigured(photovault_core::api::tls_config())
        .build()
        .unwrap()
}

/// The file key of `file_id`, opened from the server's records with the
/// master key, as any Ente client does.
async fn file_key(api: &Api, master_key: &[u8], file_id: i64) -> Vec<u8> {
    let cols: Value = api
        .get_json("/collections/v2", &[("sinceTime", "0".into())])
        .await
        .unwrap();
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
            if f["id"].as_i64() == Some(file_id) {
                return crypto::secretbox_open(
                    &crypto::unb64(f["encryptedKey"].as_str().unwrap()).unwrap(),
                    &crypto::unb64(f["keyDecryptionNonce"].as_str().unwrap()).unwrap(),
                    &ck,
                )
                .unwrap();
            }
        }
    }
    panic!("file {file_id} not found");
}

/// Attach a `vid_preview` exactly like Ente's mobile app
/// (`video_preview_service.dart` `_reportVideoPreview` and
/// `filedata/preview_upload.dart` `uploadVideoPreview`).
async fn attach_preview(api: &Api, file_id: i64, key: &[u8]) {
    let up: Value = api
        .get_json(
            "/files/data/preview-upload-url",
            &[
                ("fileID", file_id.to_string()),
                ("type", "vid_preview".into()),
            ],
        )
        .await
        .unwrap();
    let object_id = up["objectID"].as_str().unwrap().to_string();
    let url = up["url"].as_str().unwrap().to_string();
    let body = bytes::Bytes::from_static(TS);
    api.send(|| api.storage_request(Method::PUT, &url).body(body.clone()))
        .await
        .unwrap();
    // gzipAndEncryptJson: gzip(JSON) as one secretstream message under the file key.
    let playlist = json!({
        "playlist": M3U8,
        "type": "hls_video",
        "width": 160,
        "height": 120,
        "size": TS.len(),
    });
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(playlist.to_string().as_bytes()).unwrap();
    let (ct, header) = crypto::encrypt_blob(&gz.finish().unwrap(), key).unwrap();
    api.json_no_content(
        Method::PUT,
        "/files/video-data",
        &json!({
            "fileID": file_id,
            "objectID": object_id,
            "objectSize": TS.len(),
            "playlist": crypto::b64(&ct),
            "playlistHeader": crypto::b64(&header),
        }),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn streamable_video_plays_through_the_loopback_server() {
    const EMAIL: &str = "vera@example.org";
    const PASSWORD: &str = "a video password";
    let srv = server(&[EMAIL]).await;
    let acc = signup(&srv.url, EMAIL, PASSWORD).await;
    let api = Api::new(&srv.url).with_token(acc.token.clone());

    let a = TestCore::connected(&srv.url).await;
    assert!(matches!(
        a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap(),
        LoginStep::Done { .. }
    ));
    let path = a.dir.path().join("clip.mp4");
    std::fs::write(&path, MP4).unwrap();
    a.core
        .enqueue_upload(path.display().to_string(), None)
        .unwrap();
    let st = a.core.run_uploads().await.unwrap();
    assert_eq!(st.failed, 0, "{st:?}");
    a.core.sync(None).await.unwrap();
    let item = a.core.timeline_page(0, 10).remove(0);
    assert_eq!(item.kind, FileKind::Video);
    assert!(!item.has_stream);
    // Our own video and the status diff ran: no stream, and no request needed.
    assert_eq!(a.core.stream_playlist(item.id).await.unwrap(), None);
    assert_eq!(a.core.video_stream(item.id).await.unwrap(), None);

    // Another client (Ente's app, the compute node) generates the stream.
    let key = file_key(&api, &acc.master_key, item.id).await;
    attach_preview(&api, item.id, &key).await;

    // The status diff after the next sync marks it.
    a.core.sync(None).await.unwrap();
    let item = a.core.timeline_page(0, 10).remove(0);
    assert!(item.has_stream, "{item:?}");

    let vs = a
        .core
        .video_stream(item.id)
        .await
        .unwrap()
        .expect("a stream");
    assert!(
        vs.playlist_url.starts_with("http://127.0.0.1:"),
        "{}",
        vs.playlist_url
    );
    assert!(vs.playlist_url.ends_with("/index.m3u8"));
    assert_eq!((vs.width, vs.height), (160, 120));
    assert_eq!(vs.duration_ms, 4000);
    assert_eq!(vs.size_bytes, TS.len() as u64);

    let client = http();
    let res = client.get(&vs.playlist_url).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(
        res.headers()["content-type"],
        "application/vnd.apple.mpegurl"
    );
    let m3u8 = res.text().await.unwrap();
    // No key and no storage URL reach the player.
    assert!(!m3u8.contains("KEY") && !m3u8.contains("base64") && !m3u8.contains("://"));
    let segs: Vec<&str> = m3u8.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(segs, ["0.ts", "1.ts", "2.ts", "3.ts"]);

    // Every segment, fetched with a range request and decrypted by core,
    // equals OpenSSL's decryption of the same bytes.
    let base = vs.playlist_url.trim_end_matches("index.m3u8").to_string();
    for (i, line) in SEGMENTS.lines().enumerate() {
        let expect: Vec<&str> = line.split(' ').collect();
        let res = client.get(format!("{base}{i}.ts")).send().await.unwrap();
        assert_eq!(res.status(), 200);
        assert_eq!(res.headers()["content-type"], "video/mp2t");
        let body = res.bytes().await.unwrap();
        assert_eq!(body.len().to_string(), expect[2]);
        assert_eq!(hex::encode(Sha256::digest(&body)), expect[3]);
        assert!(body.chunks(188).all(|p| p[0] == 0x47));
    }
    // A byte range of a segment (players resume with Range).
    let res = client
        .get(format!("{base}1.ts"))
        .header("Range", "bytes=188-375")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), 206);
    assert_eq!(res.headers()["content-range"], "bytes 188-375/15228");
    let part = res.bytes().await.unwrap();
    assert_eq!(part.len(), 188);
    assert_eq!(part[0], 0x47);
    assert_eq!(
        client
            .get(format!("{base}4.ts"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );

    // The token and the stream id are both required.
    let (head, tail) = vs.playlist_url.split_at("http://127.0.0.1:".len());
    let port_end = tail.find('/').unwrap();
    let bad_token = format!(
        "{head}{}/{}/{}",
        &tail[..port_end],
        "0".repeat(64),
        tail[port_end + 1..].split_once('/').unwrap().1
    );
    assert_eq!(client.get(&bad_token).send().await.unwrap().status(), 404);
    let bad_stream = format!("{base}../{}/index.m3u8", "0".repeat(32));
    assert_eq!(client.get(&bad_stream).send().await.unwrap().status(), 404);

    // stream_playlist (contract v1) gives a fresh stream of the same video.
    let url = a.core.stream_playlist(item.id).await.unwrap().unwrap();
    assert_ne!(url, vs.playlist_url);
    assert_eq!(client.get(&url).send().await.unwrap().status(), 200);

    // Closing the viewer ends every stream.
    a.core.stop_streams();
    assert_eq!(client.get(&url).send().await.unwrap().status(), 404);
    assert_eq!(
        client.get(&vs.playlist_url).send().await.unwrap().status(),
        404
    );

    // A second device learns it from its own sync.
    let b = TestCore::connected(&srv.url).await;
    b.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap();
    b.core.sync(None).await.unwrap();
    let bi = b.core.timeline_page(0, 10).remove(0);
    assert!(bi.has_stream);
    let url = b.core.stream_playlist(bi.id).await.unwrap().unwrap();
    let seg = client
        .get(url.replace("index.m3u8", "0.ts"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&seg)),
        SEGMENTS.lines().next().unwrap().split(' ').nth(3).unwrap()
    );
    assert_eq!(b.core.sync_streams().await.unwrap(), 1);

    // Nothing decrypted was written to disk: no MPEG-TS packets in the
    // caches (the original download cache holds ciphertext only).
    for dir in [a.cache_dir(), a.data_dir(), b.cache_dir(), b.data_dir()] {
        for e in walk(&dir) {
            let bytes = std::fs::read(&e).unwrap();
            let ts_like = bytes.len() >= 376 && bytes[0] == 0x47 && bytes[188] == 0x47;
            assert!(!ts_like, "decrypted video at {}", e.display());
        }
    }
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}
