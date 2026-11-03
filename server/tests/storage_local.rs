//! Local backend through HTTP, the way the apps use it: bound Content-MD5 /
//! length, multipart with ETags and the web client's completion XML, Range,
//! HEAD, CORS preflight, and storage metrics.

use photovault_server::storage::PartSpec;
use photovault_server::testing::TestServer;
use std::time::Duration;

const TTL: Duration = Duration::from_secs(600);
const MD5_HELLO: &str = "XUFAKrxLKna5cZ2REBfFkg==";

fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("client")
}

fn b64md5(data: &[u8]) -> String {
    use base64::Engine;
    use md5::Digest;
    base64::engine::general_purpose::STANDARD.encode(md5::Md5::digest(data))
}

#[tokio::test]
async fn bound_md5_and_length_like_s3() {
    let srv = TestServer::start().await.expect("server");
    let c = client();
    let url = srv
        .state
        .store
        .presign_put("1/v2", TTL, Some(5), Some(MD5_HELLO))
        .await
        .expect("presign");
    // Missing / different Content-MD5: the signed header does not match.
    let r = c.put(&url).body("hello").send().await.expect("put");
    assert_eq!(r.status(), 403);
    let r = c
        .put(&url)
        .header("Content-MD5", "1B2M2Y8AsgTpgAmY7PhCfg==")
        .body("hello")
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 403);
    // Right header, wrong body of the right length.
    let r = c
        .put(&url)
        .header("Content-MD5", MD5_HELLO)
        .body("hellO")
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 400);
    assert!(r.text().await.expect("body").contains("BadDigest"));
    // Wrong length.
    let r = c
        .put(&url)
        .header("Content-MD5", MD5_HELLO)
        .body("hello!")
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 403);
    assert_eq!(srv.state.store.head("1/v2").await.expect("head"), None);
    let r = c
        .put(&url)
        .header("Content-MD5", MD5_HELLO)
        .body("hello")
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers().get("etag").and_then(|v| v.to_str().ok()),
        Some("\"5d41402abc4b2a76b9719d911017c592\"")
    );

    // Unbound URL: a Content-MD5 the client sends anyway is still verified.
    let url = srv
        .state
        .store
        .presign_put("1/v1", TTL, None, None)
        .await
        .expect("presign");
    let r = c
        .put(&url)
        .header("Content-MD5", MD5_HELLO)
        .body("nope!")
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 400);
    let r = c.put(&url).body("anything").send().await.expect("put");
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn multipart_like_web_client() {
    let srv = TestServer::start().await.expect("server");
    let c = client();
    let parts: [&[u8]; 3] = [b"first-part|", b"second-part|", b"end"];
    let specs: Vec<PartSpec> = parts
        .iter()
        .map(|p| PartSpec {
            len: Some(p.len() as i64),
            md5: Some(b64md5(p)),
        })
        .collect();
    let mp = srv
        .state
        .store
        .create_multipart_with("1/mp", &specs, TTL)
        .await
        .expect("create");
    let json = serde_json::to_value(&mp).expect("json");
    assert!(json.get("partURLs").is_some() && json.get("completeURL").is_some());
    assert!(json.get("upload_id").is_none() && json.get("uploadId").is_none());

    // A part URL bound to an MD5 refuses a part without it.
    let r = c
        .put(&mp.part_urls[0])
        .body(parts[0])
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 403);
    let mut xml_parts = Vec::new();
    for (i, (url, data)) in mp.part_urls.iter().zip(parts).enumerate() {
        let r = c
            .put(url)
            .header("Content-MD5", b64md5(data))
            .body(data)
            .send()
            .await
            .expect("put part");
        assert_eq!(r.status(), 200);
        let etag = r
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .expect("etag")
            .to_string();
        assert!(etag.starts_with('"') && etag.len() == 34);
        xml_parts.push(format!(
            "<Part><PartNumber>{}</PartNumber><ETag>{etag}</ETag></Part>",
            i + 1
        ));
    }
    // Same shape as createMultipartUploadRequestBody in the web app.
    let xml = format!(
        "<CompleteMultipartUpload>\n{}\n</CompleteMultipartUpload>",
        xml_parts.join("\n")
    );
    let bad = xml.replacen(
        "<PartNumber>2</PartNumber><ETag>\"",
        "<PartNumber>2</PartNumber><ETag>\"0",
        1,
    );
    let r = c
        .post(&mp.complete_url)
        .header("Content-Type", "text/xml")
        .body(bad)
        .send()
        .await
        .expect("complete");
    assert_eq!(r.status(), 400);
    let r = c
        .post(&mp.complete_url)
        .header("Content-Type", "text/xml")
        .body(xml.clone())
        .send()
        .await
        .expect("complete");
    assert_eq!(r.status(), 200);
    assert!(r.text().await.expect("body").contains("-3&quot;</ETag>"));
    assert_eq!(
        srv.state
            .store
            .get_bytes("1/mp")
            .await
            .expect("get")
            .as_ref(),
        b"first-part|second-part|end"
    );
    // A retried completion after success is still 200.
    let r = c
        .post(&mp.complete_url)
        .body(xml)
        .send()
        .await
        .expect("complete");
    assert_eq!(r.status(), 200);
    // The part URL is useless once the upload is gone.
    let r = c
        .put(&mp.part_urls[2])
        .header("Content-MD5", b64md5(parts[2]))
        .body(parts[2])
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn get_range_head_and_cors() {
    let srv = TestServer::start().await.expect("server");
    let c = client();
    srv.state
        .store
        .put_bytes("1/r", bytes::Bytes::from_static(b"0123456789"))
        .await
        .expect("put");
    let url = srv
        .state
        .store
        .presign_get("1/r", TTL)
        .await
        .expect("presign");
    let r = c
        .get(&url)
        .header("Range", "bytes=2-4")
        .send()
        .await
        .expect("get");
    assert_eq!(r.status(), 206);
    assert_eq!(
        r.headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok()),
        Some("bytes 2-4/10")
    );
    assert_eq!(r.bytes().await.expect("body").as_ref(), b"234");
    let r = c
        .get(&url)
        .header("Range", "bytes=-3")
        .send()
        .await
        .expect("get");
    assert_eq!(r.bytes().await.expect("body").as_ref(), b"789");
    let r = c
        .get(&url)
        .header("Range", "bytes=10-")
        .send()
        .await
        .expect("get");
    assert_eq!(r.status(), 416);
    let r = c.head(&url).send().await.expect("head");
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok()),
        Some("10")
    );
    // GET URL cannot be used to PUT.
    let r = c.put(&url).body("x").send().await.expect("put");
    assert_eq!(r.status(), 403);
    let missing = srv
        .state
        .store
        .presign_get("1/none", TTL)
        .await
        .expect("presign");
    assert_eq!(c.get(&missing).send().await.expect("get").status(), 404);

    // Browser preflight for a part upload from another origin, arbitrary headers.
    let put = srv
        .state
        .store
        .presign_put("1/cors", TTL, None, None)
        .await
        .expect("presign");
    let r = c
        .request(reqwest::Method::OPTIONS, &put)
        .header("Origin", "https://web.example.com")
        .header("Access-Control-Request-Method", "PUT")
        .header(
            "Access-Control-Request-Headers",
            "content-md5,x-client-package,x-anything",
        )
        .send()
        .await
        .expect("preflight");
    assert!(r.status().is_success());
    let h = |r: &reqwest::Response, k: &str| {
        r.headers()
            .get(k)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase()
    };
    assert_eq!(
        h(&r, "access-control-allow-origin"),
        "https://web.example.com"
    );
    assert!(h(&r, "access-control-allow-methods").contains("put"));
    assert!(h(&r, "access-control-allow-headers").contains("x-anything"));
    let r = c
        .put(&put)
        .header("Origin", "https://web.example.com")
        .body("x")
        .send()
        .await
        .expect("put");
    assert_eq!(r.status(), 200);
    assert!(h(&r, "access-control-expose-headers").contains("etag"));

    let m = c
        .get(format!("{}/metrics", srv.url))
        .send()
        .await
        .expect("metrics")
        .text()
        .await
        .expect("text");
    assert!(m.contains(
        "photovault_storage_operations_total{backend=\"local\",op=\"presign_put\",result=\"ok\"}"
    ));
    assert!(m.contains("photovault_db_pool_max_connections"));
    assert!(m.contains("photovault_storage_transfer_bytes_total{direction=\"download\"}"));
    // Signed query strings never end up in metric labels.
    assert!(!m.contains("pv-sig"));
}
