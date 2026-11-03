//! Checksums, catalog sweep, scrubbing and repair from a replica, against a
//! real server state (SQLite + local storage in a temp dir).

use photovault_server::durability::catalog::state;
use photovault_server::durability::config::TargetConfig;
use photovault_server::durability::scrub::{self, Outcome};
use photovault_server::durability::{events, sweep};
use photovault_server::storage::ObjectStore;
use photovault_server::testing::TestServer;
use std::path::PathBuf;
use std::sync::Arc;

fn sha(data: &[u8]) -> String {
    use sha2::Digest;
    hex::encode(sha2::Sha256::digest(data))
}

/// A server with one local replication target; returns the target dir too.
async fn server_with_target() -> (TestServer, PathBuf, tempfile::TempDir) {
    let backup = tempfile::tempdir().expect("tempdir");
    let path = backup.path().to_path_buf();
    let srv = TestServer::start_with(|c| {
        c.durability.enabled = false;
        let t: TargetConfig = toml::from_str(&format!(
            "name = \"usb\"\nkind = \"local\"\npath = {:?}\n",
            path.display().to_string()
        ))
        .expect("target toml");
        c.durability.targets = vec![t];
    })
    .await
    .expect("server");
    (srv, path, backup)
}

fn root(srv: &TestServer) -> PathBuf {
    srv.state
        .local_store
        .as_ref()
        .expect("local store")
        .root()
        .to_path_buf()
}

async fn scrub_key(srv: &TestServer, key: &str) -> Outcome {
    let row = srv.state.catalog.get(key).await.expect("db").expect("row");
    let limiter = photovault_server::durability::ratelimit::RateLimiter::new(0);
    let mut stats = scrub::ScrubStats::default();
    scrub::verify_one(&srv.state, &row, &limiter, &mut stats)
        .await
        .expect("verify")
}

#[tokio::test]
async fn checksums_recorded_at_write_time() {
    let srv = TestServer::start_with(|c| c.durability.enabled = false)
        .await
        .expect("server");
    srv.state
        .store
        .put_bytes("1/a", bytes::Bytes::from_static(b"hello"))
        .await
        .expect("put");
    let row = srv.state.catalog.get("1/a").await.unwrap().expect("row");
    assert_eq!(row.state, state::OK);
    assert_eq!(row.sha256.as_deref(), Some(sha(b"hello").as_str()));
    assert_eq!(row.size, 5);

    // Signed-URL upload, the way the apps upload.
    let url = srv
        .state
        .store
        .presign_put("1/b", std::time::Duration::from_secs(60), None, None)
        .await
        .expect("presign");
    let _ = rustls::crypto::ring::default_provider().install_default();
    let c = reqwest::Client::builder().no_proxy().build().unwrap();
    let r = c.put(&url).body("ciphertext!").send().await.expect("put");
    assert!(r.status().is_success(), "{}", r.status());
    let row = srv.state.catalog.get("1/b").await.unwrap().expect("row");
    assert_eq!(row.sha256.as_deref(), Some(sha(b"ciphertext!").as_str()));

    // Server-side copy carries the checksum over.
    srv.state.store.copy("1/a", "1/c").await.expect("copy");
    let row = srv.state.catalog.get("1/c").await.unwrap().expect("row");
    assert_eq!(row.sha256.as_deref(), Some(sha(b"hello").as_str()));

    // Deleting through the server is recorded, and a sweep does not call it missing.
    srv.state.store.delete("1/a").await.expect("delete");
    assert_eq!(
        srv.state.catalog.get("1/a").await.unwrap().unwrap().state,
        state::DELETED
    );
    let s = sweep::run_once(&srv.state).await.expect("sweep");
    assert_eq!(s["missing"], 0);
    // With no targets there is no retention: the row is purged.
    assert!(srv.state.catalog.get("1/a").await.unwrap().is_none());
}

#[tokio::test]
async fn sweep_finds_unrecorded_and_missing_objects() {
    let srv = TestServer::start_with(|c| c.durability.enabled = false)
        .await
        .expect("server");
    let root = root(&srv);
    // Written behind the server's back (like an S3 presigned upload).
    std::fs::create_dir_all(root.join("7")).unwrap();
    std::fs::write(root.join("7/x"), b"external").unwrap();
    srv.state
        .store
        .put_bytes("7/y", bytes::Bytes::from_static(b"tracked"))
        .await
        .unwrap();
    let s = sweep::run_once(&srv.state).await.expect("sweep");
    assert_eq!(s["new"], 1);
    assert_eq!(s["listed"], 2);
    let row = srv.state.catalog.get("7/x").await.unwrap().expect("row");
    assert_eq!(row.state, state::UNHASHED);

    // The scrubber hashes it on first read.
    assert_eq!(scrub_key(&srv, "7/x").await, Outcome::Ok);
    let row = srv.state.catalog.get("7/x").await.unwrap().unwrap();
    assert_eq!(
        (row.state.as_str(), row.sha256),
        (state::OK, Some(sha(b"external")))
    );

    // Gone without a delete: missing, with a critical event.
    std::fs::remove_file(root.join("7/y")).unwrap();
    let s = sweep::run_once(&srv.state).await.expect("sweep");
    assert_eq!(s["missing"], 1);
    assert_eq!(
        srv.state.catalog.get("7/y").await.unwrap().unwrap().state,
        state::MISSING
    );
    let ev = events::recent(&srv.state.db, 10).await.unwrap();
    assert!(
        ev.iter()
            .any(|e| e.kind == "sweep.missing" && e.subject == "7/y")
    );

    // Without a replica it cannot be repaired.
    assert!(matches!(
        scrub_key(&srv, "7/y").await,
        Outcome::Unrepairable(_)
    ));
}

#[tokio::test]
async fn scrub_detects_corruption_and_repairs_from_replica() {
    let (srv, backup, _guard) = server_with_target().await;
    let root = root(&srv);
    let data = b"0123456789abcdef".repeat(1000);
    srv.state
        .store
        .put_bytes("3/obj", bytes::Bytes::from(data.clone()))
        .await
        .unwrap();
    // A good replica on the target.
    std::fs::create_dir_all(backup.join("3")).unwrap();
    std::fs::write(backup.join("3/obj"), &data).unwrap();

    assert_eq!(scrub_key(&srv, "3/obj").await, Outcome::Ok);

    // Bit rot: one byte flipped, same size.
    let mut bad = data.clone();
    bad[5000] ^= 0x40;
    std::fs::write(root.join("3/obj"), &bad).unwrap();
    assert_eq!(
        scrub_key(&srv, "3/obj").await,
        Outcome::Repaired("usb".into())
    );
    assert_eq!(std::fs::read(root.join("3/obj")).unwrap(), data);
    let row = srv.state.catalog.get("3/obj").await.unwrap().unwrap();
    assert_eq!(row.state, state::OK);
    assert!(row.repaired_at.is_some());

    // Lost entirely: restored too.
    std::fs::remove_file(root.join("3/obj")).unwrap();
    assert_eq!(
        scrub_key(&srv, "3/obj").await,
        Outcome::Repaired("usb".into())
    );
    assert_eq!(std::fs::read(root.join("3/obj")).unwrap(), data);

    let ev = events::recent(&srv.state.db, 20).await.unwrap();
    assert!(ev.iter().any(|e| e.kind == "scrub.corrupt"));
    assert!(ev.iter().any(|e| e.kind == "scrub.repaired"));
}

#[tokio::test]
async fn bad_replica_never_overwrites_primary() {
    let (srv, backup, _guard) = server_with_target().await;
    let root = root(&srv);
    let data = b"original ciphertext".to_vec();
    srv.state
        .store
        .put_bytes("4/obj", bytes::Bytes::from(data.clone()))
        .await
        .unwrap();
    // The replica is corrupt too.
    std::fs::create_dir_all(backup.join("4")).unwrap();
    std::fs::write(backup.join("4/obj"), b"0riginal ciphertext").unwrap();
    let mut bad = data.clone();
    bad[0] = b'X';
    std::fs::write(root.join("4/obj"), &bad).unwrap();
    assert!(matches!(
        scrub_key(&srv, "4/obj").await,
        Outcome::Unrepairable(_)
    ));
    // Untouched: still the (bad) primary, not the bad replica; state corrupt.
    assert_eq!(std::fs::read(root.join("4/obj")).unwrap(), bad);
    assert_eq!(
        srv.state.catalog.get("4/obj").await.unwrap().unwrap().state,
        state::CORRUPT
    );
    // No temp files left behind.
    let leftovers: Vec<_> = std::fs::read_dir(root.join("4"))
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["obj".to_string()]);
}

#[tokio::test]
async fn cli_scrub_processes_due_objects() {
    let srv = TestServer::start_with(|c| c.durability.enabled = false)
        .await
        .expect("server");
    let store: Arc<dyn ObjectStore> = srv.state.store.clone();
    for i in 0..5 {
        store
            .put_bytes(&format!("9/{i}"), bytes::Bytes::from(vec![i as u8; 100]))
            .await
            .unwrap();
    }
    // Nothing is due right after writing (just verified).
    let s = scrub::cli_scrub(&srv.state, None, None).await.unwrap();
    assert_eq!(s["verified"], 0);
    // Force them due.
    sqlx::query("UPDATE durability_objects SET verified_at = 0")
        .execute(&srv.state.db.pool)
        .await
        .unwrap();
    let s = scrub::cli_scrub(&srv.state, None, Some(3)).await.unwrap();
    assert_eq!(s["verified"], 3);
    let s = scrub::cli_scrub(&srv.state, None, None).await.unwrap();
    assert_eq!(s["verified"], 2);
}
