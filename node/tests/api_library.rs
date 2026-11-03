//! Library sync, downloads and file-data endpoints against a mock museum,
//! using collections and files encrypted by Ente's own Rust core.

use photovault_node::api::{ApiClient, ApiError, FileDataType, RetryPolicy, Session};
use photovault_node::crypto::{self, SecretKey};
use photovault_node::library::{self, FileType, Library, SyncState};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
use wiremock::matchers::{
    body_json, body_partial_json, body_string, header, method, path, query_param,
};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

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

fn client(s: &Session) -> ApiClient {
    ApiClient::for_session(s, "0.1.0")
        .unwrap()
        .with_retry(RetryPolicy {
            max_attempts: 3,
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
        })
}

async fn mount_library(server: &MockServer) {
    let lib = &account()["library"];
    Mock::given(method("GET"))
        .and(path("/collections/v2"))
        .and(query_param("sinceTime", "0"))
        .and(header("X-Auth-Token", "dG9rZW4="))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"collections": lib["collections"]})),
        )
        .mount(server)
        .await;
    for cid in ["10", "20"] {
        Mock::given(method("GET"))
            .and(path("/collections/v2/diff"))
            .and(query_param("collectionID", cid))
            .and(query_param("sinceTime", "0"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"diff": lib["diffs"][cid], "hasMore": false})),
            )
            .mount(server)
            .await;
    }
}

#[tokio::test]
async fn sync_decrypts_ente_encrypted_library() {
    let server = MockServer::start().await;
    mount_library(&server).await;
    let s = session(&server);
    let c = client(&s);
    let mut state = SyncState::default();
    let report = library::sync(&c, &mut state).await.unwrap();
    assert_eq!(report.updated, vec![100, 200]);
    assert!(report.removed.is_empty());
    assert_eq!(state.collections_since, 2000);
    assert_eq!(state.collection_since[&10], 1500);
    assert_eq!(state.collection_since[&20], 2500);

    let lib = Library::open(&s, &state);
    let mut cols = lib.collections();
    cols.sort_by_key(|c| c.id);
    assert_eq!(cols[0].name.as_deref(), Some("Holidays"));
    assert!(cols[0].owned);
    assert_eq!(
        cols[1].name.as_deref(),
        Some("Family"),
        "shared key opened via sealed box"
    );
    assert!(!cols[1].owned);

    let fixture = &account()["library"];
    let files = lib.files();
    assert_eq!(files.len(), 2);
    for f in &files {
        let id = f.id.to_string();
        assert_eq!(
            hex::encode(f.key.as_bytes()),
            fixture["file_keys_hex"][&id].as_str().unwrap()
        );
        assert_eq!(f.metadata.raw, fixture["metadata"][&id]);
    }
    let img = lib.file(100).unwrap().unwrap();
    assert_eq!(img.file_type, FileType::Image);
    assert_eq!(img.metadata.title, "IMG_0001.JPG");
    assert_eq!(img.metadata.creation_time, 1_700_000_000_000_000);
    assert_eq!(img.pub_magic_metadata, Some(json!({"w": 4032, "h": 3024})));
    assert_eq!(img.file_size, Some(5000));
    let vid = lib.file(200).unwrap().unwrap();
    assert_eq!(vid.file_type, FileType::Video);
    assert_eq!(vid.owner_id, 2);
    assert_eq!(
        lib.owned_files().iter().map(|f| f.id).collect::<Vec<_>>(),
        vec![100]
    );

    // The state is ciphertext only and round-trips through serde.
    let json = serde_json::to_string(&state).unwrap();
    assert!(!json.contains("IMG_0001"));
    assert_eq!(serde_json::from_str::<SyncState>(&json).unwrap(), state);

    // Downloads: original through a 307 to storage (no token sent there),
    // thumbnail streamed directly.
    Mock::given(method("GET"))
        .and(path("/files/download/100"))
        .respond_with(ResponseTemplate::new(307).insert_header(
            "Location",
            format!("{}/storage/orig-100?sig=x", server.uri()),
        ))
        .mount(&server)
        .await;
    let orig = crypto::b64_decode(fixture["original_100_b64"].as_str().unwrap()).unwrap();
    Mock::given(method("GET"))
        .and(path("/storage/orig-100"))
        .and(query_param("sig", "x"))
        .and(|r: &Request| !r.headers.contains_key("X-Auth-Token"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(orig))
        .expect(2)
        .mount(&server)
        .await;
    let thumb = crypto::b64_decode(fixture["thumbnail_100_b64"].as_str().unwrap()).unwrap();
    Mock::given(method("GET"))
        .and(path("/files/preview/100"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(thumb))
        .mount(&server)
        .await;
    let o = library::download_original(&c, &img).await.unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&o)),
        fixture["original_100_plain_sha256"].as_str().unwrap()
    );
    let t = library::download_thumbnail(&c, &img).await.unwrap();
    assert_eq!(
        hex::encode(Sha256::digest(&t)),
        fixture["thumbnail_100_plain_sha256"].as_str().unwrap()
    );
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("orig");
    assert_eq!(
        library::download_original_to(&c, &img, &p).await.unwrap(),
        5000
    );
    assert_eq!(std::fs::read(&p).unwrap(), o);
}

#[tokio::test]
async fn incremental_sync_handles_removal_and_deleted_collections() {
    let server = MockServer::start().await;
    mount_library(&server).await;
    let s = session(&server);
    let c = client(&s);
    let mut state = SyncState::default();
    library::sync(&c, &mut state).await.unwrap();

    // Next round: file 100 trashed (deleted entry), collection 20 deleted.
    server.reset().await;
    let lib = &account()["library"];
    let mut col20 = lib["collections"][1].clone();
    col20["isDeleted"] = json!(true);
    col20["updationTime"] = json!(3000);
    Mock::given(method("GET"))
        .and(path("/collections/v2"))
        .and(query_param("sinceTime", "2000"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"collections": [col20]})))
        .mount(&server)
        .await;
    let mut gone = lib["diffs"]["10"][0].clone();
    gone["isDeleted"] = json!(true);
    gone["updationTime"] = json!(3100);
    gone["metadata"] = json!({"decryptionHeader": ""});
    Mock::given(method("GET"))
        .and(path("/collections/v2/diff"))
        .and(query_param("collectionID", "10"))
        .and(query_param("sinceTime", "1500"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"diff": [gone], "hasMore": false})),
        )
        .mount(&server)
        .await;
    let report = library::sync(&c, &mut state).await.unwrap();
    assert!(report.updated.is_empty());
    assert_eq!(report.removed, vec![100, 200]);
    assert!(state.files.is_empty());
    assert_eq!(state.collection_since[&10], 3100);
    assert!(!state.collections.contains_key(&20));
}

#[tokio::test]
async fn paging_dedup_and_retry() {
    let server = MockServer::start().await;
    let lib = &account()["library"];
    let s = session(&server);
    // The same file 100 appears in collection 10 and in a second owned
    // collection 11 (same file key, wrapped with 11's key).
    let mut col11 = lib["collections"][0].clone();
    col11["id"] = json!(11);
    Mock::given(method("GET"))
        .and(path("/collections/v2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"collections": [lib["collections"][0], col11]})),
        )
        .mount(&server)
        .await;
    let mut f_in_11 = lib["diffs"]["10"][0].clone();
    f_in_11["collectionID"] = json!(11);
    f_in_11["updationTime"] = json!(1600);
    // Collection 10: first a 503 (retried), then two pages.
    Mock::given(method("GET"))
        .and(path("/collections/v2/diff"))
        .and(query_param("collectionID", "10"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/collections/v2/diff"))
        .and(query_param("collectionID", "10"))
        .and(query_param("sinceTime", "0"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"diff": lib["diffs"]["10"], "hasMore": true})),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/collections/v2/diff"))
        .and(query_param("collectionID", "10"))
        .and(query_param("sinceTime", "1500"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"diff": [], "hasMore": false})),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/collections/v2/diff"))
        .and(query_param("collectionID", "11"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"diff": [f_in_11], "hasMore": false})),
        )
        .mount(&server)
        .await;
    let c = client(&s);
    let mut state = SyncState::default();
    let report = library::sync(&c, &mut state).await.unwrap();
    assert_eq!(report.updated, vec![100]);
    let l = Library::open(&s, &state);
    assert_eq!(l.len(), 1);
    let f = l.file(100).unwrap().unwrap();
    assert_eq!(f.collection_ids, vec![10, 11]);
    assert_eq!(f.collection_id, 10);
    assert_eq!(f.updation_time, 1600);
}

#[tokio::test]
async fn errors_are_typed_and_4xx_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/collections/v2"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/collections/v2/diff"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"code": "BAD_REQUEST", "message": "nope"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let s = session(&server);
    let c = client(&s);
    assert!(matches!(
        c.get_collections(0).await,
        Err(ApiError::Unauthorized)
    ));
    match c.get_collection_diff(1, 0).await {
        Err(ApiError::Http {
            status: 400,
            code,
            message,
            ..
        }) => {
            assert_eq!(code, "BAD_REQUEST");
            assert_eq!(message, "nope");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn file_data_endpoints() {
    let server = MockServer::start().await;
    let s = session(&server);
    let c = client(&s);
    let key = SecretKey([0x71; 32]);
    let blob = crypto::encrypt_blob(b"{\"faces\":[]}", &key).unwrap();

    Mock::given(method("POST"))
        .and(path("/files/data/fetch"))
        .and(body_json(json!({"fileIDs": [100, 101], "type": "mldata"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"fileID": 100, "type": "mldata", "encryptedData": crypto::b64_encode(&blob.encrypted_data),
                      "decryptionHeader": crypto::b64_encode(&blob.decryption_header), "updatedAt": 77}],
            "pendingIndexFileIDs": [101], "errFileIDs": []
        })))
        .mount(&server)
        .await;
    let r = c
        .fetch_files_data(FileDataType::MlData, &[100, 101])
        .await
        .unwrap();
    assert_eq!(r.pending_index_file_ids, vec![101]);
    let d = &r.data[0];
    assert_eq!(d.updated_at, Some(77));
    let plain = crypto::decrypt_blob(
        &crypto::b64_decode(&d.encrypted_data).unwrap(),
        &crypto::b64_decode(&d.decryption_header).unwrap(),
        &key,
    )
    .unwrap();
    assert_eq!(plain, b"{\"faces\":[]}");
    assert!(
        c.fetch_files_data(FileDataType::MlData, &[1; 201])
            .await
            .is_err()
    );

    Mock::given(method("GET"))
        .and(path("/files/data/fetch"))
        .and(query_param("fileID", "5"))
        .and(query_param("type", "vid_preview"))
        .and(query_param("preferNoContent", "true"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    assert!(
        c.fetch_file_data(FileDataType::VideoPreview, 5)
            .await
            .unwrap()
            .is_none()
    );

    Mock::given(method("PUT"))
        .and(path("/files/data"))
        .and(body_json(json!({
            "fileID": 100, "type": "mldata",
            "encryptedData": crypto::b64_encode(&blob.encrypted_data),
            "decryptionHeader": crypto::b64_encode(&blob.decryption_header),
            "lastUpdatedAt": 77
        })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    c.put_file_data(100, FileDataType::MlData, &blob, Some(77), None)
        .await
        .unwrap();

    Mock::given(method("POST"))
        .and(path("/files/data/status-diff"))
        .and(body_json(json!({"lastUpdatedAt": 0})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"diff": [
            {"fileID": 100, "userID": 1, "type": "mldata", "isDeleted": false, "size": 10, "updatedAt": 77}
        ]})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/files/data/status-diff"))
        .and(body_json(json!({"lastUpdatedAt": 77})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"diff": null})))
        .mount(&server)
        .await;
    let d = c.file_data_status_diff(0).await.unwrap();
    assert_eq!(d[0].updated_at, 77);
    assert!(c.file_data_status_diff(77).await.unwrap().is_empty());

    Mock::given(method("GET"))
        .and(path("/files/data/preview-upload-url"))
        .and(query_param("fileID", "200"))
        .and(query_param("type", "vid_preview"))
        .and(query_param("isMultiPart", "true"))
        .and(query_param("count", "2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "objectID": "pv_0123456789abcdefghijkl",
            "partURLs": [format!("{}/s/p1", server.uri()), format!("{}/s/p2", server.uri())],
            "completeURL": format!("{}/s/done", server.uri())
        })))
        .mount(&server)
        .await;
    let up = c
        .preview_upload_url(200, FileDataType::VideoPreview, Some(2))
        .await
        .unwrap();
    assert_eq!(up.object_id, "pv_0123456789abcdefghijkl");
    let parts = up.part_urls.unwrap();
    for (i, p) in ["p1", "p2"].iter().enumerate() {
        Mock::given(method("PUT"))
            .and(path(format!("/s/{p}")))
            .and(header(
                "Content-MD5",
                photovault_node::api::files::content_md5(p.as_bytes()).as_str(),
            ))
            .and(|r: &Request| !r.headers.contains_key("X-Auth-Token"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("ETag", format!("\"etag{}\"", i + 1)),
            )
            .mount(&server)
            .await;
    }
    let mut etags = vec![];
    for (url, p) in parts.iter().zip(["p1", "p2"]) {
        let md5 = photovault_node::api::files::content_md5(p.as_bytes());
        etags.push(
            c.put_presigned(url, bytes::Bytes::from(p), Some(&md5))
                .await
                .unwrap()
                .unwrap(),
        );
    }
    Mock::given(method("POST"))
        .and(path("/s/done"))
        .and(body_string(
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"etag1\"</ETag></Part>\
             <Part><PartNumber>2</PartNumber><ETag>\"etag2\"</ETag></Part></CompleteMultipartUpload>",
        ))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    c.complete_multipart(up.complete_url.as_deref().unwrap(), &etags)
        .await
        .unwrap();

    Mock::given(method("POST"))
        .and(path("/files/data/preview-upload-url"))
        .and(body_json(
            json!({"fileID": 200, "type": "vid_preview", "contentLength": 2, "contentMD5": "x"}),
        ))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"objectID": "pv_a", "url": "https://s/x"})),
        )
        .mount(&server)
        .await;
    let v2 = c
        .preview_upload_url_v2(200, FileDataType::VideoPreview, 2, "x")
        .await
        .unwrap();
    assert_eq!(v2.url.as_deref(), Some("https://s/x"));
    Mock::given(method("POST"))
        .and(path("/files/data/multipart-preview-upload-url"))
        .and(body_json(json!({"fileID": 200, "type": "vid_preview", "contentLength": 9, "partLength": 5, "partMd5s": ["a", "b"]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"objectID": "pv_b", "partURLs": ["u1", "u2"], "completeURL": "c"})))
        .mount(&server)
        .await;
    let mp = c
        .multipart_preview_upload_url(
            200,
            FileDataType::VideoPreview,
            9,
            5,
            &["a".into(), "b".into()],
        )
        .await
        .unwrap();
    assert_eq!(mp.part_urls.unwrap().len(), 2);

    let playlist = crypto::encrypt_blob(b"playlist", &key).unwrap();
    Mock::given(method("PUT"))
        .and(path("/files/video-data"))
        .and(body_partial_json(json!({
            "fileID": 200, "objectID": "pv_0123456789abcdefghijkl", "objectSize": 1234,
            "playlist": crypto::b64_encode(&playlist.encrypted_data),
            "playlistHeader": crypto::b64_encode(&playlist.decryption_header)
        })))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    c.put_video_data(200, "pv_0123456789abcdefghijkl", 1234, &playlist, None)
        .await
        .unwrap();
}
