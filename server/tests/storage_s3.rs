//! S3 backend against a real S3 implementation. Skipped unless
//! `PHOTOVAULT_TEST_S3_ENDPOINT` (plus `_REGION`, `_BUCKET`, `_ACCESS_KEY`,
//! `_SECRET_KEY`) is set; `scripts/test-s3-garage.sh` starts a Garage
//! container and sets them.

use base64::Engine;
use md5::Digest;
use photovault_server::config::S3Config;
use photovault_server::storage::s3::S3Store;
use photovault_server::storage::{ObjectStore, PartSpec};
use std::time::Duration;

const TTL: Duration = Duration::from_secs(600);

fn store() -> Option<S3Store> {
    let v = |k: &str| std::env::var(format!("PHOTOVAULT_TEST_S3_{k}")).ok();
    let Some(endpoint) = v("ENDPOINT") else {
        eprintln!("PHOTOVAULT_TEST_S3_ENDPOINT not set; skipping S3 tests");
        return None;
    };
    Some(
        S3Store::new(&S3Config {
            endpoint,
            public_endpoint: None,
            region: v("REGION").unwrap_or_else(|| "us-east-1".into()),
            bucket: v("BUCKET").expect("bucket"),
            access_key: v("ACCESS_KEY").expect("access key"),
            secret_key: v("SECRET_KEY").expect("secret key"),
            secret_key_file: None,
            path_style: true,
        })
        .expect("store"),
    )
}

fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
}

fn b64md5(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(md5::Md5::digest(data))
}

fn key(name: &str) -> String {
    format!(
        "1/{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    )
}

#[tokio::test]
async fn server_side_operations() {
    let Some(s) = store() else { return };
    let k = key("obj");
    assert_eq!(s.head(&k).await.expect("head"), None);
    s.put_bytes(&k, bytes::Bytes::from_static(b"hello"))
        .await
        .expect("put");
    assert_eq!(s.head(&k).await.expect("head"), Some(5));
    assert_eq!(s.get_bytes(&k).await.expect("get").as_ref(), b"hello");
    let k2 = key("copy with space");
    s.copy(&k, &k2).await.expect("copy");
    assert_eq!(s.get_bytes(&k2).await.expect("get").as_ref(), b"hello");
    s.delete(&k).await.expect("delete");
    s.delete(&k).await.expect("delete missing");
    assert_eq!(s.head(&k).await.expect("head"), None);
    assert!(matches!(
        s.get_bytes(&k).await,
        Err(photovault_server::storage::StorageError::NotFound)
    ));
    s.delete(&k2).await.expect("delete");
}

#[tokio::test]
async fn presigned_put_get_with_bound_md5() {
    let Some(s) = store() else { return };
    let c = client();
    let k = key("v2");
    let url = s
        .presign_put(&k, TTL, Some(5), Some(&b64md5(b"hello")))
        .await
        .expect("presign");
    let r = c.put(&url).body("hello").send().await.expect("put");
    // AWS answers 403, Garage 400: either way the upload is refused.
    assert!(
        r.status().is_client_error(),
        "missing signed Content-MD5 must be refused"
    );
    let r = c
        .put(&url)
        .header("Content-MD5", b64md5(b"hello"))
        .body("hellO")
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 400, "BadDigest expected");
    let r = c
        .put(&url)
        .header("Content-MD5", b64md5(b"hello"))
        .body("hello")
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 200);
    let get = s.presign_get(&k, TTL).await.expect("presign");
    let r = c.get(&get).send().await.expect("get");
    assert_eq!(r.bytes().await.expect("body").as_ref(), b"hello");
    let r = c
        .get(&get)
        .header("Range", "bytes=1-2")
        .send()
        .await
        .expect("get");
    assert_eq!(r.status(), 206);
    assert_eq!(r.bytes().await.expect("body").as_ref(), b"el");
    s.delete(&k).await.expect("delete");
}

#[tokio::test]
async fn presigned_multipart_like_web_client() {
    let Some(s) = store() else { return };
    let c = client();
    let k = key("mp");
    let p1 = vec![7u8; 5 * 1024 * 1024];
    let p2 = b"tail".to_vec();
    let parts = [p1, p2];
    let specs: Vec<PartSpec> = parts
        .iter()
        .map(|p| PartSpec {
            len: Some(p.len() as i64),
            md5: Some(b64md5(p)),
        })
        .collect();
    let mp = s
        .create_multipart_with(&k, &specs, TTL)
        .await
        .expect("create");
    assert!(!mp.upload_id.is_empty());
    let mut xml = String::from("<CompleteMultipartUpload>\n");
    for (i, (url, data)) in mp.part_urls.iter().zip(&parts).enumerate() {
        let r = c
            .put(url)
            .header("Content-MD5", b64md5(data))
            .body(data.clone())
            .send()
            .await
            .expect("part");
        assert_eq!(r.status(), 200);
        let etag = r
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .expect("etag")
            .to_string();
        xml.push_str(&format!(
            "<Part><PartNumber>{}</PartNumber><ETag>{etag}</ETag></Part>\n",
            i + 1
        ));
    }
    xml.push_str("</CompleteMultipartUpload>");
    let r = c
        .post(&mp.complete_url)
        .header("Content-Type", "text/xml")
        .body(xml)
        .send()
        .await
        .expect("complete");
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap_or_default());
    assert_eq!(s.head(&k).await.expect("head"), Some(5 * 1024 * 1024 + 4));
    s.delete(&k).await.expect("delete");

    // Unbound variant + abort.
    let k = key("aborted");
    let mp = s.create_multipart(&k, 2, TTL).await.expect("create");
    let r = c
        .put(&mp.part_urls[0])
        .body("x")
        .send()
        .await
        .expect("part");
    assert_eq!(r.status(), 200);
    s.abort_multipart(&k, &mp.upload_id).await.expect("abort");
    s.abort_multipart(&k, &mp.upload_id)
        .await
        .expect("abort again");
    let r = c
        .put(&mp.part_urls[1])
        .body("y")
        .send()
        .await
        .expect("part");
    assert!(r.status().is_client_error());
}
