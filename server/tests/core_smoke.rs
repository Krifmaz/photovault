//! Smoke test for the core: health, metrics, auth rejection, local signed-URL round trip.

use photovault_server::testing::TestServer;
use std::time::Duration;

async fn http(method: &str, url: &str, body: Vec<u8>) -> (u16, Vec<u8>) {
    // Tiny HTTP client over TCP so tests need no TLS stack.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(url).unwrap();
    let mut s = tokio::net::TcpStream::connect((u.host_str().unwrap(), u.port().unwrap()))
        .await
        .unwrap();
    let pq = match u.query() {
        Some(q) => format!("{}?{}", u.path(), q),
        None => u.path().to_string(),
    };
    let req = format!(
        "{method} {pq} HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    s.write_all(&body).await.unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.unwrap();
    let text = String::from_utf8_lossy(&out).to_string();
    let code = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let idx = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
    (code, out[idx..].to_vec())
}

#[tokio::test]
async fn core_smoke() {
    let srv = TestServer::start().await.unwrap();
    assert_eq!(
        http("GET", &format!("{}/healthz", srv.url), vec![]).await.0,
        200
    );
    let (c, b) = http("GET", &format!("{}/metrics", srv.url), vec![]).await;
    assert_eq!(c, 200);
    assert!(String::from_utf8_lossy(&b).contains("photovault_http_requests"));

    let put = srv
        .state
        .store
        .presign_put("1/obj", Duration::from_secs(60), Some(5), None)
        .await
        .unwrap();
    assert_eq!(http("PUT", &put, b"hello".to_vec()).await.0, 200);
    assert_eq!(srv.state.store.head("1/obj").await.unwrap(), Some(5));
    let get = srv
        .state
        .store
        .presign_get("1/obj", Duration::from_secs(60))
        .await
        .unwrap();
    let (c, b) = http("GET", &get, vec![]).await;
    assert_eq!((c, b.as_slice()), (200, b"hello".as_slice()));
    // Tampered signature is refused.
    assert_eq!(
        http("GET", &get.replace("pv-exp=", "pv-exp=1"), vec![])
            .await
            .0,
        403
    );

    let mp = srv
        .state
        .store
        .create_multipart("1/big", 2, Duration::from_secs(60))
        .await
        .unwrap();
    assert_eq!(http("PUT", &mp.part_urls[0], b"ab".to_vec()).await.0, 200);
    assert_eq!(http("PUT", &mp.part_urls[1], b"cd".to_vec()).await.0, 200);
    let xml = b"<CompleteMultipartUpload><Part><PartNumber>1</PartNumber></Part><Part><PartNumber>2</PartNumber></Part></CompleteMultipartUpload>".to_vec();
    assert_eq!(http("POST", &mp.complete_url, xml).await.0, 200);
    assert_eq!(
        srv.state.store.get_bytes("1/big").await.unwrap().as_ref(),
        b"abcd"
    );
}
