//! Photovault's own additions: `/photovault/info`, `/photovault/usage`, real storage
//! numbers in museum's endpoints, quotas, email-code delivery modes, admin-created
//! accounts with signups closed, CORS origins and the web app on the API port.

mod users_common;

use photovault_server::admin_cli::{AdminCmd, run};
use photovault_server::capacity;
use photovault_server::testing::TestServer;
use serde_json::json;
use users_common::*;

#[tokio::test]
async fn info_is_public_and_complete() {
    let srv = TestServer::start_with(|c| {
        c.server_name = "Home photos".into();
        c.home_url = Some("http://192.168.1.10:8080".into());
        c.mdns = false;
    })
    .await
    .unwrap();
    let c = Client::new(&srv);
    let (s, v) = c.get("/photovault/info").await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["product"], "photovault");
    assert_eq!(v["name"], "Home photos");
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(v["apiVersion"], 1);
    assert_eq!(v["homeUrl"], "http://192.168.1.10:8080");
    assert_eq!(v["publicUrl"], srv.url.as_str());
    assert_eq!(v["signupsAllowed"], true);
    assert_eq!(v["emailVerification"], "log");
    assert_eq!(v["storage"], "local");
    assert!(
        v["features"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f == "admin-cli")
    );
    assert_eq!(v["albumsUrl"], format!("{}/albums", srv.url));
    // No Cast receiver configured: the apps pair by code only.
    assert!(v["castAppId"].is_null());
}

#[tokio::test]
async fn info_announces_the_servers_own_cast_receiver() {
    let srv = TestServer::start_with(|c| {
        c.mdns = false;
        c.cast_app_id = Some("1A2B3C4D".into());
    })
    .await
    .unwrap();
    let (s, v) = Client::new(&srv).get("/photovault/info").await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["castAppId"], "1A2B3C4D");
    assert_eq!(v["castUrl"], format!("{}/cast", srv.url));
}

#[tokio::test]
async fn usage_reports_disk_and_quota() {
    let srv = server(&["u@example.org"]).await;
    let c = Client::new(&srv);
    let acc = signup(&c, "u@example.org").await;
    let u = c.with_token(&acc.token);

    let (s, _) = Client::new(&srv).get("/photovault/usage").await;
    assert_eq!(s, 401);

    let (s, v) = u.get("/photovault/usage").await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["usedBytes"], 0);
    assert_eq!(v["unlimited"], false);
    assert_eq!(v["limitSource"], "disk");
    assert!(v["quotaBytes"].is_null());
    let limit = v["limitBytes"].as_i64().unwrap();
    assert!(limit > 0);

    // Museum's endpoints carry the same limit (within the free-space cache window).
    let (_, d) = u.get("/users/details/v2").await;
    assert_eq!(d["subscription"]["storage"].as_i64(), Some(limit));
    let (_, b) = u.get("/billing/subscription").await;
    assert_eq!(b["subscription"]["storage"].as_i64(), Some(limit));

    // A quota below the free space wins.
    run(
        &srv.state,
        AdminCmd::SetQuota {
            user: "u@example.org".into(),
            size: "1MB".into(),
        },
    )
    .await
    .unwrap();
    let (_, v) = u.get("/photovault/usage").await;
    assert_eq!(
        (v["limitBytes"].as_i64(), v["limitSource"].as_str()),
        (Some(1 << 20), Some("quota"))
    );
    assert_eq!(v["quotaBytes"], 1 << 20);
    let (_, d) = u.get("/users/details/v2").await;
    assert_eq!(d["subscription"]["storage"], 1 << 20);
    // Uploads beyond the quota (+ museum's 50 MB overflow) are refused with 426.
    let (s, _) = u
        .post(
            "/files/upload-url",
            json!({"contentLength": 200 << 20, "contentMD5": "1B2M2Y8AsgTpgAmY7PhCfg=="}),
        )
        .await;
    assert!(s == 426 || s == 400, "{s}");

    // `none` removes it again.
    run(
        &srv.state,
        AdminCmd::SetQuota {
            user: "u@example.org".into(),
            size: "none".into(),
        },
    )
    .await
    .unwrap();
    let (_, v) = u.get("/photovault/usage").await;
    assert_eq!(v["limitSource"], "disk");
}

#[tokio::test]
async fn usage_reports_the_whole_disk_like_df() {
    const GB: i64 = 1_000_000_000;
    // A 100 GB disk with 93 GB used by anything (other apps, the OS, our photos).
    let srv = TestServer::start_with(|c| {
        c.disk_override = Some(capacity::Disk {
            total: 100 * GB,
            used: 93 * GB,
            available: 7 * GB,
            source: capacity::DiskSource::Disk,
        });
        c.hardcoded_ott.insert("d@example.org".into(), OTT.into());
    })
    .await
    .unwrap();
    let c = Client::new(&srv);
    let acc = signup(&c, "d@example.org").await;
    let u = c.with_token(&acc.token);
    let (s, v) = u.get("/photovault/usage").await;
    assert_eq!(s, 200, "{v}");
    // The disk itself, not "used + free".
    assert_eq!(v["disk"]["totalBytes"], 100 * GB);
    assert_eq!(v["disk"]["usedBytes"], 93 * GB);
    assert_eq!(v["disk"]["availableBytes"], 7 * GB);
    assert_eq!(v["disk"]["usedPercent"], 93.0);
    assert_eq!(v["disk"]["nearlyFull"], true);
    assert_eq!(v["disk"]["source"], "disk");
    assert_eq!(v["nearlyFullPercent"], 90.0);
    // The account's own photos are reported separately.
    assert_eq!(v["userBytes"], 0);
    assert_eq!(v["usedBytes"], 0);
    // No quota: the limit is the disk's size, what is left is the disk's rest.
    assert_eq!(v["limitBytes"], 100 * GB);
    assert_eq!(v["freeBytes"], 7 * GB);
    assert_eq!(v["limitSource"], "disk");
    assert!(v["quotaBytes"].is_null());
    // Ente's endpoints carry the disk's size: no plan, no placeholder.
    let (_, d) = u.get("/users/details/v2").await;
    assert_eq!(d["subscription"]["storage"], 100 * GB);
    let (_, b) = u.get("/billing/subscription").await;
    assert_eq!(b["subscription"]["storage"], 100 * GB);

    // An admin quota is the limit; the rest is capped by the disk.
    run(
        &srv.state,
        AdminCmd::SetQuota {
            user: "d@example.org".into(),
            size: "50GB".into(),
        },
    )
    .await
    .unwrap();
    let (_, v) = u.get("/photovault/usage").await;
    assert_eq!(v["limitSource"], "quota");
    assert_eq!(v["limitBytes"], v["quotaBytes"]);
    assert_eq!(v["freeBytes"], 7 * GB);
    // The disk is still reported, so the app can warn.
    assert_eq!(v["disk"]["nearlyFull"], true);
    let (_, d) = u.get("/users/details/v2").await;
    assert_eq!(d["subscription"]["storage"], v["quotaBytes"]);
}

#[tokio::test]
async fn usage_is_not_nearly_full_on_a_roomy_disk() {
    let srv = TestServer::start_with(|c| {
        c.disk_override = Some(capacity::Disk {
            total: 1000,
            used: 500,
            available: 500,
            source: capacity::DiskSource::Disk,
        });
        c.hardcoded_ott.insert("r@example.org".into(), OTT.into());
    })
    .await
    .unwrap();
    let c = Client::new(&srv);
    let acc = signup(&c, "r@example.org").await;
    let (_, v) = c.with_token(&acc.token).get("/photovault/usage").await;
    assert_eq!(v["disk"]["usedPercent"], 50.0);
    assert_eq!(v["disk"]["nearlyFull"], false);
}

#[tokio::test]
async fn storage_limit_and_unlimited_rules() {
    assert_eq!(capacity::combine(0, None), capacity::UNLIMITED);
    let srv = server(&["s@example.org"]).await;
    let c = Client::new(&srv);
    let acc = signup(&c, "s@example.org").await;
    let u = c.with_token(&acc.token);
    // A configured server limit smaller than the disk applies.
    let srv2 = TestServer::start_with(|c| {
        c.storage_limit = Some(10 << 20);
        c.hardcoded_ott.insert("t@example.org".into(), OTT.into());
    })
    .await
    .unwrap();
    let c2 = Client::new(&srv2);
    let acc2 = signup(&c2, "t@example.org").await;
    let (_, v) = c2.with_token(&acc2.token).get("/photovault/usage").await;
    assert_eq!(
        (v["limitBytes"].as_i64(), v["limitSource"].as_str()),
        (Some(10 << 20), Some("storage-limit"))
    );
    // The budget stands in for the disk.
    assert_eq!(v["disk"]["totalBytes"], 10 << 20);
    assert_eq!(v["disk"]["source"], "storage-limit");
    let (_, d) = u.get("/users/details/v2").await;
    assert_ne!(d["subscription"]["storage"], capacity::UNLIMITED);
}

#[tokio::test]
async fn default_quota_applies_to_new_accounts() {
    let srv = TestServer::start_with(|c| {
        c.default_storage_bytes = 5 << 30;
        c.hardcoded_ott.insert("q@example.org".into(), OTT.into());
    })
    .await
    .unwrap();
    let c = Client::new(&srv);
    let acc = signup(&c, "q@example.org").await;
    let (_, v) = c.with_token(&acc.token).get("/photovault/usage").await;
    assert_eq!(v["quotaBytes"], 5_i64 << 30);
    // Unless the disk is smaller, the default quota is the limit.
    if v["limitSource"] != "disk" {
        assert_eq!(v["limitSource"], "default-quota");
    }
}

#[tokio::test]
async fn admin_created_account_with_signups_closed() {
    let srv = TestServer::start_with(|c| c.allow_signups = false)
        .await
        .unwrap();
    let c = Client::new(&srv);
    let (_, v) = c.get("/photovault/info").await;
    assert_eq!(v["signupsAllowed"], false);
    // Strangers cannot sign up.
    let (s, _) = c
        .post(
            "/users/ott",
            json!({"email": "x@example.org", "purpose": "signup"}),
        )
        .await;
    assert_eq!(s, 403);
    // Even with skip-verification a stranger's direct verify gets nothing (no pending code).
    let (s, _) = c
        .post(
            "/users/verify-email",
            json!({"email": "x@example.org", "ott": "000000"}),
        )
        .await;
    assert!(s == 410 || s == 401 || s == 404, "{s}");

    run(
        &srv.state,
        AdminCmd::CreateUser {
            email: "new@example.org".into(),
            quota: Some("2GB".into()),
            admin: false,
        },
    )
    .await
    .unwrap();
    // The invited person can request a code and finish signing up in the app.
    let (s, _) = c
        .post(
            "/users/ott",
            json!({"email": "new@example.org", "purpose": "signup"}),
        )
        .await;
    assert_eq!(s, 200);
    let otts = photovault_server::users::login::pending_otts(&srv.state, "new@example.org")
        .await
        .unwrap();
    assert!(otts.len() >= 2, "admin code + app code");
    let (s, v) = c
        .post(
            "/users/verify-email",
            json!({"email": "new@example.org", "ott": otts[0].0}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["token"].is_string(), "{v}");

    // signups on|off is stored and takes effect at once.
    run(
        &srv.state,
        AdminCmd::Signups {
            state: Some("on".into()),
        },
    )
    .await
    .unwrap();
    let (_, v) = c.get("/photovault/info").await;
    assert_eq!(v["signupsAllowed"], true);
    run(
        &srv.state,
        AdminCmd::Signups {
            state: Some("off".into()),
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn skip_email_verification_accepts_any_code_for_pending_requests() {
    let srv = TestServer::start_with(|c| c.skip_email_verification = true)
        .await
        .unwrap();
    let c = Client::new(&srv);
    let (_, v) = c.get("/photovault/info").await;
    assert_eq!(v["emailVerification"], "skip");
    let (s, _) = c
        .post(
            "/users/verify-email",
            json!({"email": "solo@example.org", "ott": "111111"}),
        )
        .await;
    assert_ne!(s, 200, "no pending request: refused");
    let (s, _) = c
        .post(
            "/users/ott",
            json!({"email": "solo@example.org", "purpose": "signup"}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = c
        .post(
            "/users/verify-email",
            json!({"email": "solo@example.org", "ott": "111111"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
}

#[tokio::test]
async fn cors_allows_only_configured_origins_but_info_is_open() {
    let srv = TestServer::start_with(|c| c.web_origins = vec!["https://photos.example.com".into()])
        .await
        .unwrap();
    let origin = |o: &'static str, path: &'static str| {
        let url = format!("{}{path}", srv.url);
        async move { support_get(&url, o).await }
    };
    assert_eq!(
        origin("https://photos.example.com", "/ping")
            .await
            .as_deref(),
        Some("https://photos.example.com")
    );
    assert_eq!(origin("https://evil.example", "/ping").await, None);
    assert_eq!(
        origin("https://evil.example", "/photovault/info")
            .await
            .as_deref(),
        Some("https://evil.example")
    );
}

/// GET with headers; returns status, lower-case headers and body text.
async fn raw(
    url: &str,
    headers: &[(&str, &str)],
) -> (u16, std::collections::HashMap<String, String>, String) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut r = reqwest::Client::new().get(url);
    for (k, v) in headers {
        r = r.header(*k, *v);
    }
    let res = r.send().await.unwrap();
    let status = res.status().as_u16();
    let h = res
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();
    (status, h, res.text().await.unwrap())
}

async fn support_get(url: &str, origin: &str) -> Option<String> {
    let (_, h, _) = raw(url, &[("Origin", origin)]).await;
    h.get("access-control-allow-origin").cloned()
}

#[tokio::test]
async fn web_app_is_served_on_the_api_port() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(web.path().join("index.html"), "<title>Photovault</title>").unwrap();
    std::fs::write(web.path().join("gallery.html"), "gallery").unwrap();
    std::fs::create_dir_all(web.path().join("_next/static")).unwrap();
    std::fs::write(web.path().join("_next/static/app.js"), "js").unwrap();
    let dir = web.path().to_path_buf();
    let srv = TestServer::start_with(move |c| c.web_dir = Some(dir))
        .await
        .unwrap();
    let get = |p: &'static str| {
        let url = format!("{}{p}", srv.url);
        async move { raw(&url, &[]).await }
    };
    let (s, _, body) = get("/").await;
    assert_eq!((s, body.as_str()), (200, "<title>Photovault</title>"));
    let (s, _, body) = get("/gallery").await;
    assert_eq!((s, body.as_str()), (200, "gallery"));
    let (s, h, _) = get("/_next/static/app.js").await;
    assert_eq!(s, 200);
    assert!(h["cache-control"].contains("immutable"));
    // Client-side routes fall back to index.html; missing assets are 404.
    let (s, _, body) = get("/shared-albums").await;
    assert_eq!((s, body.as_str()), (200, "<title>Photovault</title>"));
    let (s, _, _) = get("/missing.js").await;
    assert_eq!(s, 404);
    // API paths are unchanged.
    let (s, _, body) = get("/ping").await;
    assert_eq!(s, 200, "{body}");
    let (s, _, _) = get("/photovault/info").await;
    assert_eq!(s, 200);
}

/// The cast and embed web apps share a path prefix with the cast API: API routes must win,
/// everything else under `/cast/` loads the cast app, and `/embed` stays framable.
#[tokio::test]
async fn cast_api_wins_over_the_cast_web_app_and_embed_is_framable() {
    let web = tempfile::tempdir().unwrap();
    std::fs::write(web.path().join("index.html"), "photos").unwrap();
    for app in ["cast", "embed"] {
        std::fs::create_dir_all(web.path().join(app)).unwrap();
        std::fs::write(web.path().join(app).join("index.html"), app).unwrap();
    }
    let dir = web.path().to_path_buf();
    let srv = TestServer::start_with(move |c| c.web_dir = Some(dir))
        .await
        .unwrap();

    // Web app pages.
    let (s, _, body) = raw(&format!("{}/cast/", srv.url), &[]).await;
    assert_eq!((s, body.as_str()), (200, "cast"));
    let (s, _, body) = raw(&format!("{}/cast/pair", srv.url), &[]).await;
    assert_eq!((s, body.as_str()), (200, "cast"));

    // Cast API routes are answered by the API, not by cast/index.html.
    let c = Client::new(&srv);
    let (s, v) = c
        .post(
            "/cast/device-info",
            json!({"publicKey": photovault_server::crypto::b64(&[1u8; 32])}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let code = v["deviceCode"].as_str().expect("deviceCode").to_string();
    let (s, _, body) = raw(&format!("{}/cast/device-info/{code}", srv.url), &[]).await;
    assert_ne!(body, "cast", "status {s}");
    let (s, _, body) = raw(&format!("{}/cast/cast-data/{code}", srv.url), &[]).await;
    assert_ne!(body, "cast", "status {s}");
    let (s, _, body) = raw(&format!("{}/cast/info", srv.url), &[]).await;
    assert_ne!(body, "cast", "status {s}");

    // Embed: served and framable (no X-Frame-Options / frame-ancestors).
    let (s, h, body) = raw(&format!("{}/embed/", srv.url), &[]).await;
    assert_eq!((s, body.as_str()), (200, "embed"));
    assert!(!h.contains_key("x-frame-options"));
    assert!(
        !h.get("content-security-policy")
            .is_some_and(|v| v.contains("frame-ancestors"))
    );
}
