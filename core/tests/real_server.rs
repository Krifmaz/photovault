//! End to end against the real Photovault server, in process (SQLite and
//! local disk storage): sign up like an Ente app, then drive everything
//! through `Core` exactly as the Android app does.

mod support;

use photovault_core::api::Api;
use photovault_core::{CoreError, FileKind, LoginStep, fetch_server_info};
use reqwest::Method;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use support::{Proxy, ProxyConfig, TestCore, assert_err, info_json, jpeg, server, signup};

fn sha(b: &[u8]) -> Vec<u8> {
    Sha256::digest(b).to_vec()
}

fn done(step: LoginStep) -> photovault_core::Account {
    match step {
        LoginStep::Done { account } => account,
        other => panic!("expected Done, got {other:?}"),
    }
}

/// Upload a JPEG through the queue on one device, sync it on a second one,
/// read the timeline and fetch + decrypt thumbnail and original.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_sync_timeline_download() {
    const EMAIL: &str = "alice@example.org";
    const PASSWORD: &str = "correct horse battery staple";
    let srv = server(&[EMAIL]).await;
    let acc = signup(&srv.url, EMAIL, PASSWORD).await;

    // The base server is a stock Ente-compatible one: no /photovault/info.
    let info = fetch_server_info(srv.url.clone()).await.unwrap();
    assert!(!info.is_photovault);
    assert_eq!(info.public_url, srv.url);

    // Device A: log in with SRP.
    let a = TestCore::connected(&srv.url).await;
    assert_eq!(a.core.active_base_url().as_deref(), Some(srv.url.as_str()));
    assert_err(
        a.core.login(EMAIL.into(), "wrong password".into()).await,
        CoreError::IncorrectPassword,
    );
    let account = done(a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap());
    assert_eq!(account.email, EMAIL);
    assert_eq!(account.server_url, srv.url);
    assert_eq!(a.core.current_account(), Some(account.clone()));

    // Upload a small JPEG via the persisted queue.
    let photo = jpeg(640, 480, 7);
    let path = a.dir.path().join("IMG_0001.jpg");
    std::fs::write(&path, &photo).unwrap();
    a.core
        .enqueue_upload(path.display().to_string(), None)
        .unwrap();
    let st = a.core.upload_status();
    assert_eq!(st.pending, 1);
    assert_eq!(st.bytes_total, photo.len() as u64);
    let st = a.core.run_uploads().await.unwrap();
    assert_eq!((st.pending, st.uploading, st.failed), (0, 0, 0), "{st:?}");
    // The uploader's own library already has it.
    assert_eq!(a.core.timeline_count(), 1);
    // Enqueuing the same file again is deduplicated (no second file).
    a.core
        .enqueue_upload(path.display().to_string(), None)
        .unwrap();
    a.core.run_uploads().await.unwrap();
    a.core.sync(None).await.unwrap();
    assert_eq!(a.core.timeline_count(), 1);

    // Device B: a second Core, a fresh login, a sync.
    let b = TestCore::connected(&srv.url).await;
    done(b.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap());
    assert_eq!(b.core.timeline_count(), 0);
    let p = b.core.sync(None).await.unwrap();
    assert!(p.done);
    assert_eq!(p.files, 1);
    assert_eq!(b.core.timeline_count(), 1);
    let items = b.core.timeline_page(0, 50);
    assert_eq!(items.len(), 1);
    let item = &items[0];
    assert_eq!(item.title, "IMG_0001.jpg");
    assert_eq!(item.kind, FileKind::Image);
    assert_eq!((item.width, item.height), (640, 480));
    assert!(!item.has_stream);
    assert_eq!(b.core.timeline_page(1, 50), vec![]);
    let cols = b.core.collections();
    assert_eq!(cols.len(), 1, "{cols:?}");
    assert_eq!(cols[0].kind, "uncategorized");
    assert_eq!(cols[0].count, 1);
    assert_eq!(cols[0].cover_file_id, Some(item.id));
    assert_eq!(b.core.collection_page(cols[0].id, 0, 10), items);

    // Thumbnail: a decrypted JPEG, then served from the (encrypted) cache.
    let thumb = b.core.thumbnail(item.id).await.unwrap();
    assert_eq!(&thumb[..2], &[0xFF, 0xD8]);
    let img = image::load_from_memory(&thumb).unwrap();
    assert!(img.width() <= 720 && img.height() <= 720);
    assert_eq!(b.core.thumbnail(item.id).await.unwrap(), thumb);

    // Original: byte for byte, also a second time from the cache.
    let dest = b.dir.path().join("out.jpg");
    let got = b
        .core
        .download(item.id, dest.display().to_string())
        .await
        .unwrap();
    assert_eq!(sha(&std::fs::read(&got).unwrap()), sha(&photo));
    std::fs::remove_file(&dest).unwrap();
    b.core
        .download(item.id, dest.display().to_string())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), photo);
    assert_eq!(b.core.stream_playlist(item.id).await.unwrap(), None);

    // Nothing in B's store or cache is plaintext.
    let needle = &photo[200..264];
    for f in walk(&b.dir.path().join("data"))
        .into_iter()
        .chain(walk(&b.dir.path().join("cache")))
    {
        let bytes = std::fs::read(&f).unwrap();
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle),
            "{} holds plaintext",
            f.display()
        );
        assert!(
            !bytes.windows(12).any(|w| w == b"IMG_0001.jpg"),
            "{} holds the file name",
            f.display()
        );
    }

    // Storage usage falls back to /users/details/v2 on a stock server.
    let u = b.core.storage_usage().await.unwrap();
    assert!(u.used_bytes > 0, "{u:?}");

    // A restart keeps the session and the library.
    let b_data = b.data_dir();
    let b_cache = b.cache_dir();
    let id = item.id;
    let core = b.core.clone();
    drop(core);
    let b2 =
        photovault_core::Core::new(b_data.display().to_string(), b_cache.display().to_string());
    // (`b` still holds the first instance; a second on the same files is fine.)
    let b2 = b2.unwrap();
    assert_eq!(b2.current_account().map(|a| a.email), Some(EMAIL.into()));
    assert_eq!(b2.timeline_count(), 1);
    assert_eq!(b2.thumbnail(id).await.unwrap(), thumb);

    // Logout wipes the library but keeps the server.
    b.core.logout().await.unwrap();
    assert_eq!(b.core.current_account(), None);
    assert_eq!(b.core.timeline_count(), 0);
    assert_eq!(b.core.saved_servers().len(), 1);
    assert_err(b.core.sync(None).await, CoreError::NotLoggedIn);
    let _ = acc;
}

fn walk(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

/// Forgot password: email code, recovery key, new password; the old
/// password stops working, the new one works and the library survives.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_key_login() {
    const EMAIL: &str = "bob@example.org";
    const PASSWORD: &str = "first password";
    const NEW_PASSWORD: &str = "second password";
    let srv = server(&[EMAIL]).await;
    let acc = signup(&srv.url, EMAIL, PASSWORD).await;

    // Upload one file first, so we can check the keys still open it.
    let a = TestCore::connected(&srv.url).await;
    done(a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap());
    let photo = jpeg(64, 48, 1);
    let path = a.dir.path().join("before.jpg");
    std::fs::write(&path, &photo).unwrap();
    a.core
        .enqueue_upload(path.display().to_string(), None)
        .unwrap();
    a.core.run_uploads().await.unwrap();

    let c = TestCore::connected(&srv.url).await;
    c.core.send_email_ott(EMAIL.into()).await.unwrap();
    assert_err(
        c.core
            .verify_email_ott(EMAIL.into(), "000000".into(), String::new())
            .await,
        CoreError::IncorrectCode,
    );
    let step = c
        .core
        .verify_email_ott(EMAIL.into(), support::OTT.into(), String::new())
        .await
        .unwrap();
    assert_eq!(step, LoginStep::NeedsRecoveryKey);
    // A wrong key (valid words, other key) and garbage are both rejected,
    // and the pending login survives them.
    let other = photovault_core::crypto::recovery_key_to_mnemonic(&[9u8; 32]).unwrap();
    assert_err(
        c.core.recover_with_key(other, NEW_PASSWORD.into()).await,
        CoreError::IncorrectCode,
    );
    assert_err(
        c.core
            .recover_with_key("not a key".into(), NEW_PASSWORD.into())
            .await,
        CoreError::IncorrectCode,
    );
    // Words as a user might type them: upper case, extra spaces.
    let typed = format!("  {}  ", acc.recovery_words.to_uppercase());
    let account = c
        .core
        .recover_with_key(typed, NEW_PASSWORD.into())
        .await
        .unwrap();
    assert_eq!(account.email, EMAIL);
    c.core.sync(None).await.unwrap();
    assert_eq!(c.core.timeline_count(), 1);
    let id = c.core.timeline_page(0, 1)[0].id;
    let dest = c.dir.path().join("x.jpg");
    c.core
        .download(id, dest.display().to_string())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), photo);

    // Old password no longer works; the new one does (fresh device).
    let d = TestCore::connected(&srv.url).await;
    assert_err(
        d.core.login(EMAIL.into(), PASSWORD.into()).await,
        CoreError::IncorrectPassword,
    );
    done(
        d.core
            .login(EMAIL.into(), NEW_PASSWORD.into())
            .await
            .unwrap(),
    );
    d.core.sync(None).await.unwrap();
    assert_eq!(d.core.timeline_count(), 1);

    // The hex form of the recovery key works too.
    let e = TestCore::connected(&srv.url).await;
    e.core.send_email_ott(EMAIL.into()).await.unwrap();
    assert_eq!(
        e.core
            .verify_email_ott(EMAIL.into(), support::OTT.into(), String::new())
            .await
            .unwrap(),
        LoginStep::NeedsRecoveryKey
    );
    e.core
        .recover_with_key(acc.recovery_hex.clone(), PASSWORD.into())
        .await
        .unwrap();
    // The other sessions were not logged out.
    a.core.sync(None).await.unwrap();
    assert_eq!(a.core.timeline_count(), 1);
}

/// TOTP second factor after SRP, and with an email code.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn totp_two_factor() {
    const EMAIL: &str = "dave@example.org";
    const PASSWORD: &str = "dave's password";
    let srv = server(&[EMAIL]).await;
    let acc = signup(&srv.url, EMAIL, PASSWORD).await;

    // Turn on TOTP like the apps do.
    let api = Api::new(&srv.url).with_token(acc.token.clone());
    let v: Value = api
        .json(Method::POST, "/users/two-factor/setup", &json!({}))
        .await
        .unwrap();
    let secret = v["secretCode"].as_str().unwrap().to_string();
    // The server only checks the size of this recovery blob (24 + 16 bytes).
    let (ct, nonce) = photovault_core::crypto::secretbox_seal(&[7u8; 24], &acc.master_key).unwrap();
    api.json_no_content(
        Method::POST,
        "/users/two-factor/enable",
        &json!({
            "code": totp_now(&secret),
            "encryptedTwoFactorSecret": photovault_core::crypto::b64(&ct),
            "twoFactorSecretDecryptionNonce": photovault_core::crypto::b64(&nonce),
        }),
    )
    .await
    .unwrap();

    let a = TestCore::connected(&srv.url).await;
    let step = a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap();
    let LoginStep::TwoFactorTotp { session_id } = step else {
        panic!("expected TOTP, got {step:?}");
    };
    assert_err(
        a.core
            .verify_totp(session_id.clone(), "000000".into(), String::new())
            .await,
        CoreError::IncorrectCode,
    );
    let account = done(
        a.core
            .verify_totp(session_id, totp_now(&secret), String::new())
            .await
            .unwrap(),
    );
    assert_eq!(account.email, EMAIL);
    a.core.sync(None).await.unwrap();

    // Email code first, then TOTP, then the password.
    let b = TestCore::connected(&srv.url).await;
    b.core.send_email_ott(EMAIL.into()).await.unwrap();
    let step = b
        .core
        .verify_email_ott(EMAIL.into(), support::OTT.into(), PASSWORD.into())
        .await
        .unwrap();
    let LoginStep::TwoFactorTotp { session_id } = step else {
        panic!("expected TOTP, got {step:?}");
    };
    done(
        b.core
            // The next period's code: the server rejects a replayed code.
            .verify_totp(session_id, totp_at(&secret, 30), PASSWORD.into())
            .await
            .unwrap(),
    );
}

fn totp_now(secret: &str) -> String {
    totp_at(secret, 0)
}

fn totp_at(secret: &str, offset: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    photovault_server::users::totp::code_at(secret, now + offset).unwrap()
}

/// A file large enough for a multipart upload (more than four 4 MiB chunks).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn multipart_upload_roundtrip() {
    const EMAIL: &str = "erin@example.org";
    const PASSWORD: &str = "erin's password";
    let srv = server(&[EMAIL]).await;
    signup(&srv.url, EMAIL, PASSWORD).await;
    let a = TestCore::connected(&srv.url).await;
    done(a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap());

    // Not a decodable image: the thumbnail falls back to a placeholder.
    let big: Vec<u8> = (0..(17 * 1024 * 1024u32 + 123))
        .map(|i| (i.wrapping_mul(31) % 251) as u8)
        .collect();
    let path = a.dir.path().join("BIG.jpg");
    std::fs::write(&path, &big).unwrap();
    a.core
        .enqueue_upload(path.display().to_string(), None)
        .unwrap();
    let st = a.core.run_uploads().await.unwrap();
    assert_eq!((st.pending, st.failed), (0, 0), "{st:?}");

    let b = TestCore::connected(&srv.url).await;
    done(b.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap());
    b.core.sync(None).await.unwrap();
    let items = b.core.timeline_page(0, 10);
    assert_eq!(items.len(), 1);
    let thumb = b.core.thumbnail(items[0].id).await.unwrap();
    assert_eq!(&thumb[..2], &[0xFF, 0xD8]);
    let dest = b.dir.path().join("big.out");
    b.core
        .download(items[0].id, dest.display().to_string())
        .await
        .unwrap();
    assert_eq!(sha(&std::fs::read(&dest).unwrap()), sha(&big));
}

/// An upload into a named album, and an interrupted queue that resumes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn upload_queue_survives_restart() {
    const EMAIL: &str = "frank@example.org";
    const PASSWORD: &str = "frank's password";
    let srv = server(&[EMAIL]).await;
    signup(&srv.url, EMAIL, PASSWORD).await;
    let a = TestCore::connected(&srv.url).await;
    done(a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap());
    let p1 = a.dir.path().join("one.jpg");
    let p2 = a.dir.path().join("two.png");
    std::fs::write(&p1, jpeg(32, 32, 1)).unwrap();
    let png = {
        let img = image::RgbImage::from_pixel(20, 10, image::Rgb([1, 2, 3]));
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    };
    std::fs::write(&p2, &png).unwrap();
    a.core
        .enqueue_upload(p1.display().to_string(), None)
        .unwrap();
    a.core
        .enqueue_upload(p2.display().to_string(), None)
        .unwrap();
    assert_eq!(a.core.upload_status().pending, 2);

    // "Process death" before the queue ran: a new Core on the same files.
    let data = a.data_dir();
    let cache = a.cache_dir();
    let a2 = photovault_core::Core::new(data.display().to_string(), cache.display().to_string())
        .unwrap();
    assert_eq!(a2.upload_status().pending, 2);
    let st = a2.run_uploads().await.unwrap();
    assert_eq!((st.pending, st.failed), (0, 0), "{st:?}");
    assert_eq!(a2.timeline_count(), 2);
    let items = a2.timeline_page(0, 10);
    let two = items.iter().find(|i| i.title == "two.png").unwrap();
    assert_eq!((two.width, two.height), (20, 10));
    let dest = a.dir.path().join("two.out");
    a2.download(two.id, dest.display().to_string())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), png);

    // A missing file is refused; one deleted after queueing fails without
    // blocking the queue.
    assert!(
        a2.enqueue_upload(a.dir.path().join("gone.jpg").display().to_string(), None)
            .is_err()
    );
    let p3 = a.dir.path().join("three.jpg");
    std::fs::write(&p3, jpeg(16, 16, 3)).unwrap();
    a2.enqueue_upload(p3.display().to_string(), None).unwrap();
    std::fs::remove_file(&p3).unwrap();
    let p4 = a.dir.path().join("four.jpg");
    std::fs::write(&p4, jpeg(16, 16, 4)).unwrap();
    a2.enqueue_upload(p4.display().to_string(), None).unwrap();
    let st = a2.run_uploads().await.unwrap();
    assert_eq!(st.failed, 1, "{st:?}");
    assert_eq!(a2.timeline_count(), 3);
}

/// Wrong and unreachable servers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_errors() {
    // Nothing listens on port 1.
    let r = fetch_server_info("http://127.0.0.1:1".into()).await;
    assert!(matches!(r, Err(CoreError::Unreachable { .. })), "{r:?}");
    // Something that answers but is not a photo server.
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move {
        let app = axum::Router::new().fallback(|| async { "hello" });
        let _ = axum::serve(l, app).await;
    });
    let r = fetch_server_info(url).await;
    assert!(matches!(r, Err(CoreError::NotPhotovault { .. })), "{r:?}");

    // Not logged in / no server.
    let c = TestCore::new();
    assert_eq!(c.core.active_base_url(), None);
    assert!(
        c.core
            .login("x@example.org".into(), "pw".into())
            .await
            .is_err()
    );
    assert_err(c.core.sync(None).await, CoreError::NotLoggedIn);
    assert_err(c.core.thumbnail(1).await, CoreError::NotLoggedIn);
}

/// A Photovault server: `/photovault/info` decides between the home and
/// the public address, and `/photovault/usage` gives the storage numbers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn home_public_switching_and_usage() {
    const EMAIL: &str = "gina@example.org";
    const PASSWORD: &str = "gina's password";
    let srv = server(&[EMAIL]).await;
    signup(&srv.url, EMAIL, PASSWORD).await;

    // Two listeners of one proxy: [0] plays the public URL, [1] the home URL.
    let proxy = Proxy::start(&srv.url, 2, |urls| ProxyConfig {
        info: info_json("Family photos", &urls[0], Some(&urls[1])),
        usage: Some(
            json!({"usedBytes": 1234, "limitBytes": null, "freeBytes": null, "unlimited": true}),
        ),
    })
    .await;
    let (public, home) = (proxy.urls[0].clone(), proxy.urls[1].clone());

    let info = fetch_server_info(public.clone()).await.unwrap();
    assert!(info.is_photovault);
    assert_eq!(info.name, "Family photos");
    assert_eq!(info.public_url, public);
    assert_eq!(info.home_url.as_deref(), Some(home.as_str()));

    let a = TestCore::new();
    a.core.add_server(public.clone()).await.unwrap();
    assert_eq!(a.core.active_base_url().as_deref(), Some(home.as_str()));
    let account = done(a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap());
    // The account belongs to the public URL, wherever requests go.
    assert_eq!(account.server_url, public);
    let u = a.core.storage_usage().await.unwrap();
    assert_eq!(
        (u.used_bytes, u.limit_bytes, u.free_bytes),
        (1234, None, None)
    );
    proxy.set(|c| {
        c.usage =
            Some(json!({"usedBytes": 10, "limitBytes": 100, "freeBytes": 90, "unlimited": false}))
    });
    let u = a.core.storage_usage().await.unwrap();
    assert_eq!(
        (u.used_bytes, u.limit_bytes, u.free_bytes),
        (10, Some(100), Some(90))
    );
    // Without /photovault/usage: the Ente details endpoint.
    proxy.set(|c| c.usage = None);
    let u = a.core.storage_usage().await.unwrap();
    assert_eq!(u.used_bytes, 0);

    // Away from home: the home URL does not answer, so the public one is used.
    proxy.set(|c| c.info = info_json("Family photos", &public, Some("http://127.0.0.1:1")));
    a.core.add_server(public.clone()).await.unwrap();
    assert_eq!(
        a.core.refresh_network().await.as_deref(),
        Some(public.as_str())
    );
    a.core.sync(None).await.unwrap();
    // Back home.
    proxy.set(|c| c.info = info_json("Family photos", &public, Some(&home)));
    a.core.add_server(public.clone()).await.unwrap();
    assert_eq!(
        a.core.refresh_network().await.as_deref(),
        Some(home.as_str())
    );

    // Saved servers survive and can be switched and removed.
    a.core.add_server(srv.url.clone()).await.unwrap();
    assert_eq!(a.core.saved_servers().len(), 2);
    a.core.select_server(srv.url.clone()).unwrap();
    assert_eq!(a.core.active_base_url().as_deref(), Some(srv.url.as_str()));
    assert!(a.core.select_server("http://never-added".into()).is_err());
    a.core.remove_server(srv.url.clone()).unwrap();
    assert_eq!(a.core.saved_servers().len(), 1);
    assert_eq!(a.core.active_base_url(), None);
}
