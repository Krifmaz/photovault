//! Admin API, billing (self-hosted), family plans, storage bonus / referrals, remote store
//! and feature flags, custom domains, push tokens, discount coupons, events, paste and ping.
//! Runs on SQLite, or Postgres with `PHOTOVAULT_TEST_DATABASE_URL`.

mod users_common;

use photovault_server::crypto;
use serde_json::{Value, json};
use users_common::*;

const GIB: i64 = 1024 * 1024 * 1024;

fn far_future() -> i64 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_micros() as i64)
        + 365 * 24 * 3600 * 1_000_000
}

#[tokio::test]
async fn admin_billing_and_family() {
    let (a, b, c3) = ("admin@example.org", "bob@example.org", "carol@example.org");
    let srv = server(&[a, b, c3]).await;
    let c = Client::new(&srv);
    let admin = signup(&c, a).await;
    let bob = signup(&c, b).await;
    let carol = signup(&c, c3).await;
    let ac = c.with_token(&admin.token);
    let bc = c.with_token(&bob.token);

    // Only the first account (no `admins` configured) is an admin.
    let (s, v) = bc.get("/admin/users?sinceTime=0").await;
    assert_eq!(s, 401, "{v}");
    let (s, v) = ac.get("/admin/users?sinceTime=0").await;
    assert_eq!(s, 200, "{v}");
    let users = v["users"].as_array().unwrap();
    assert_eq!(users.len(), 3);
    assert!(users.iter().any(|u| u["ID"] == bob.id && u["email"] == b));

    // /admin/user by id and by email (shape the CLI relies on).
    let (s, v) = ac.get(&format!("/admin/user?id={}", bob.id)).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["user"]["ID"], bob.id);
    assert_eq!(v["user"]["email"], b);
    assert_eq!(v["subscription"]["productID"], "free");
    assert_eq!(v["storageConsumedStatus"], "available");
    assert!(!v["tokens"].as_array().unwrap().is_empty());
    assert_eq!(v["details"]["email"], b);
    let (s, v) = ac.get(&format!("/admin/user?email={b}")).await;
    assert_eq!((s, v["user"]["ID"].as_i64()), (200, Some(bob.id)));
    let (s, _) = ac.get("/admin/user?email=nobody@example.org").await;
    assert_eq!(s, 404);

    // Billing is self-hosted: no plans, free plan from the configured quota.
    let (s, v) = bc.get("/billing/user-plans").await;
    assert_eq!(s, 200);
    assert_eq!(v["plans"], json!([]));
    assert_eq!(v["freePlan"]["period"], "year");
    let (s, v) = bc.get("/billing/subscription").await;
    assert_eq!(s, 200);
    assert_eq!(v["subscription"]["productID"], "free");
    let (s, _) = bc
        .post(
            "/billing/verify-subscription",
            json!({"productID": "pro", "paymentProvider": "playstore", "verificationData": "x"}),
        )
        .await;
    assert_eq!(s, 400);

    // Family needs a paid plan.
    let (s, _) = ac.post("/family/create", json!({})).await;
    assert_eq!(s, 402);
    let expiry = far_future();
    let (s, v) = ac
        .put(
            "/admin/user/subscription",
            json!({
                "userID": admin.id, "productID": "selfhosted_pro", "storage": 500 * GIB,
                "expiryTime": expiry, "transactionID": "manual", "paymentProvider": "",
                "attributes": {"isCancelled": false}
            }),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (_, v) = ac.get(&format!("/admin/user?id={}", admin.id)).await;
    assert_eq!(v["subscription"]["storage"], 500 * GIB);
    assert_eq!(v["subscription"]["expiryTime"], expiry);
    assert_eq!(v["subscription"]["originalTransactionID"], "manual");

    let (s, v) = ac.post("/family/create", json!({})).await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = ac
        .post(
            "/family/add-member",
            json!({"email": b, "storageLimit": 10 * GIB}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (s, _) = ac
        .post("/family/add-member", json!({"email": "ghost@example.org"}))
        .await;
    assert_eq!(s, 404);
    let (token,): (String,) =
        sqlx::query_as("SELECT token FROM families WHERE member_id = $1 AND status = 'INVITED'")
            .bind(bob.id)
            .fetch_one(&srv.state.db.pool)
            .await
            .unwrap();
    let (s, v) = c.get(&format!("/family/invite-info/{token}")).await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["adminEmail"], a);
    let (s, v) = c
        .post("/family/accept-invite", json!({"token": token}))
        .await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = ac.get("/family/members").await;
    assert_eq!(s, 200, "{v}");
    let members = v["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
    let bob_row = members.iter().find(|m| m["email"] == b).unwrap();
    assert_eq!(bob_row["status"], "ACCEPTED");
    assert_eq!(bob_row["storageLimit"], 10 * GIB);
    assert_eq!(v["storage"], 500 * GIB);
    // Bob now sees the family in his details, and cannot apply a referral code.
    let (_, v) = bc.get("/users/details/v2").await;
    assert_eq!(v["familyData"]["members"].as_array().unwrap().len(), 2);
    let (_, v) = bc.get("/storage-bonus/referral-view").await;
    assert_eq!(v["isFamilyMember"], true);
    assert_eq!(v["enableApplyCode"], false);

    let member_id = bob_row["id"].as_str().unwrap().to_string();
    let (s, v) = ac
        .post(
            "/family/modify-storage",
            json!({"id": member_id, "storageLimit": 20 * GIB}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    // Families JWT works for the family web app.
    let (_, v) = ac.get("/users/families-token").await;
    let fam = c.with_token(v["familiesToken"].as_str().unwrap());
    let (s, v) = fam.get("/family/members").await;
    assert_eq!(s, 200, "{v}");
    let (s, _) = fam
        .delete(&format!("/family/remove-member/{member_id}"), None)
        .await;
    assert_eq!(s, 200);
    let (_, v) = bc.get("/users/details/v2").await;
    assert!(v.get("familyData").is_none(), "{v}");

    // Support operations.
    let (s, v) = ac
        .post(
            "/admin/user/bonus",
            json!({"bonusType": "ADD_ON_SUPPORT", "action": "ADD", "userID": carol.id, "year": 1, "storageInGB": 100}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let cc = c.with_token(&carol.token);
    let (_, v) = cc.get("/storage-bonus/details").await;
    assert_eq!(v["bonuses"][0]["type"], "ADD_ON_SUPPORT");
    assert_eq!(v["bonuses"][0]["storage"], 100 * GIB);
    let (s, _) = ac
        .post(
            "/admin/user/bonus",
            json!({"bonusType": "SIGN_UP", "action": "ADD", "userID": carol.id, "year": 1, "storageInGB": 1}),
        )
        .await;
    assert_eq!(s, 400);
    let (s, _) = ac
        .post(
            "/admin/user/bonus",
            json!({"bonusType": "ADD_ON_SUPPORT", "action": "REMOVE", "userID": carol.id}),
        )
        .await;
    assert_eq!(s, 200);

    let (s, v) = ac
        .post(
            "/admin/user/update-flag",
            json!({"userID": carol.id, "key": "betaUser", "value": "true"}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (_, v) = cc.get("/remote-store/feature-flags").await;
    assert_eq!(v["betaUser"], true);
    let (s, _) = ac
        .post(
            "/admin/user/update-flag",
            json!({"userID": carol.id, "key": "mapEnabled", "value": "true"}),
        )
        .await;
    assert_eq!(s, 400, "mapEnabled is not admin editable");

    let (s, v) = ac.get(&format!("/admin/email-hash?email={c3}")).await;
    assert_eq!(s, 200);
    let hash = v["hash"].as_str().unwrap().to_string();
    let (s, v) = ac
        .post("/admin/emails-from-hashes", json!({"hashes": [hash]}))
        .await;
    assert_eq!((s, v["emails"].clone()), (200, json!([c3])));

    let (s, _) = ac
        .post(
            "/admin/user/add-ott",
            json!({"email": c3, "code": "654321", "app": "photos", "expiryTime": far_future()}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, v) = c
        .post("/users/verify-email", json!({"email": c3, "ott": "654321"}))
        .await;
    assert_eq!(s, 200, "{v}");
    assert!(v["encryptedToken"].is_string());

    let (s, _) = ac
        .post(
            "/admin/user/terminate-session",
            json!({"userID": carol.id, "token": carol.token}),
        )
        .await;
    assert_eq!(s, 200);
    let (s, _) = cc.get("/users/details/v2").await;
    assert_eq!(s, 401);

    let (s, v) = ac
        .post("/admin/user/disable-2fa", json!({"userID": carol.id}))
        .await;
    assert_eq!(s, 200, "{v}");
    let (s, _) = ac.post("/admin/user/disable-2fa", json!({})).await;
    assert_eq!(s, 400);

    // Admin deletion and recovery.
    let (s, v) = ac
        .delete(&format!("/admin/user/delete?email={c3}"), None)
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["userID"], carol.id);
    let (s, v) = ac
        .get(&format!("/admin/user/scheduled-deletions?email={c3}"))
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["scheduledDeletions"][0]["userID"], carol.id);
    let (s, v) = ac
        .post(
            "/admin/user/recover",
            json!({"userID": carol.id, "emailID": c3}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = ac.get(&format!("/admin/user?email={c3}")).await;
    assert_eq!((s, v["user"]["ID"].as_i64()), (200, Some(carol.id)));

    // Destructive admin calls need a session younger than 10 minutes.
    sqlx::query("UPDATE tokens SET creation_time = creation_time - 3600000000 WHERE user_id = $1")
        .bind(admin.id)
        .execute(&srv.state.db.pool)
        .await
        .unwrap();
    let (s, v) = ac.get("/admin/users?sinceTime=0").await;
    assert_eq!((s, v["message"].as_str()), (400, Some("Token is too old")));

    // Listmonk is not configured on a self-hosted server.
    let (s, _) = ac.get("/admin/listmonk/missing-subscribers/count").await;
    assert_eq!(s, 400);
}

#[tokio::test]
async fn referrals_remote_store_and_misc() {
    let (a, b) = ("rita@example.org", "sam@example.org");
    let srv = server(&[a, b]).await;
    let c = Client::new(&srv);
    let rita = signup(&c, a).await;
    let sam = signup(&c, b).await;
    let rc = c.with_token(&rita.token);
    let sc = c.with_token(&sam.token);

    // Referral code: generated on first view, changeable, claimable once.
    let (s, v) = rc.get("/storage-bonus/referral-view").await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["planInfo"]["planType"], "10_GB_ON_UPGRADE");
    assert_eq!(v["code"].as_str().unwrap().len(), 6);
    assert_eq!(v["remainingCodeChangeAttempts"], 3);
    let (s, _) = rc
        .post("/storage-bonus/change-code", json!({"code": "ab-c"}))
        .await;
    assert_eq!(s, 400);
    let (s, v) = rc
        .post("/storage-bonus/change-code", json!({"code": "rita2024"}))
        .await;
    assert_eq!(s, 200, "{v}");
    let (_, v) = rc.get("/storage-bonus/referral-view").await;
    assert_eq!(v["code"], "RITA2024");
    assert_eq!(v["codeChangeAttempts"], 1);
    let (s, v) = sc
        .post("/storage-bonus/change-code", json!({"code": "RITA2024"}))
        .await;
    assert_eq!((s, v["code"].as_str()), (400, Some("CODE_ALREADY_EXISTS")));

    let (s, v) = sc
        .post("/storage-bonus/referral-claim?code=NOPE99", json!({}))
        .await;
    assert_eq!((s, v["code"].as_str()), (404, Some("INVALID_CODE")));
    let (s, v) = sc
        .post("/storage-bonus/referral-claim?code=RITA2024", json!({}))
        .await;
    assert_eq!(s, 200, "{v}");
    let (s, v) = sc
        .post("/storage-bonus/referral-claim?code=RITA2024", json!({}))
        .await;
    assert_eq!((s, v["code"].as_str()), (409, Some("CODE_ALREADY_APPLIED")));
    let (_, v) = sc.get("/storage-bonus/details").await;
    assert_eq!(v["hasAppliedCode"], true);
    assert_eq!(v["bonuses"][0]["storage"], 10 * GIB);
    let (_, v) = rc.get("/storage-bonus/details").await;
    assert_eq!(v["refCount"], 1);
    assert_eq!(v["referralStats"][0]["planType"], "10_GB_ON_UPGRADE");
    let (_, v) = sc.get("/users/details/v2").await;
    assert_eq!(v["storageBonus"], 10 * GIB);

    // Remote store.
    let (s, _) = rc.get("/remote-store?key=mapEnabled").await;
    assert_eq!(s, 404);
    let (s, v) = rc
        .get("/remote-store?key=mapEnabled&defaultValue=false")
        .await;
    assert_eq!((s, v["value"].as_str()), (200, Some("false")));
    let (s, _) = rc
        .post(
            "/remote-store/update",
            json!({"key": "mapEnabled", "value": "yes"}),
        )
        .await;
    assert_eq!(s, 400);
    let (s, _) = rc
        .post(
            "/remote-store/update",
            json!({"key": "internalUser", "value": "true"}),
        )
        .await;
    assert_eq!(s, 400, "not user editable");
    let (s, _) = rc
        .post(
            "/remote-store/update",
            json!({"key": "mapEnabled", "value": "true"}),
        )
        .await;
    assert_eq!(s, 200);
    let (_, v) = rc.get("/remote-store?key=mapEnabled").await;
    assert_eq!(v["value"], "true");
    let (s, v) = rc.get("/remote-store/feature-flags").await;
    assert_eq!(s, 200);
    assert_eq!(v["mapEnabled"], true);
    assert_eq!(v["enableMobMultiPart"], true);
    assert!(v["serverApiFlag"].as_i64().unwrap() & 1 == 1);
    let (s, _) = rc.delete("/remote-store/mapEnabled", None).await;
    assert_eq!(s, 400, "only customDomain is removable");

    // Custom domains are unique (case-insensitive) and publicly checkable.
    let (s, _) = rc
        .post(
            "/remote-store/update",
            json!({"key": "customDomain", "value": "Photos.Rita.example"}),
        )
        .await;
    assert_eq!(s, 200);
    let (_, v) = rc.get("/remote-store/feature-flags").await;
    assert_eq!(v["customDomain"], "Photos.Rita.example");
    let (s, v) = sc
        .post(
            "/remote-store/update",
            json!({"key": "customDomain", "value": "photos.rita.example"}),
        )
        .await;
    assert_eq!((s, v["code"].as_str()), (409, Some("CONFLICT")));
    let (s, _) = c.get("/custom-domain?domain=photos.rita.example").await;
    assert_eq!(s, 200);
    let (s, _) = c.get("/custom-domain?domain=other.example").await;
    assert_eq!(s, 404);
    let (s, _) = c.get("/custom-domain?domain=not_a_domain").await;
    assert_eq!(s, 400);
    let (s, _) = rc.delete("/remote-store/customDomain", None).await;
    assert_eq!(s, 200);
    let (s, _) = c.get("/custom-domain?domain=photos.rita.example").await;
    assert_eq!(s, 404);

    // Push tokens are tied to the calling session.
    let (s, _) = rc.post("/push/token", json!({"fcmToken": "f1"})).await;
    assert_eq!(s, 200);
    let (s, _) = rc
        .post("/push/token", json!({"fcmToken": "f1", "platform": "web"}))
        .await;
    assert_eq!(s, 400);
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM push_tokens WHERE user_id = $1")
        .bind(rita.id)
        .fetch_one(&srv.state.db.pool)
        .await
        .unwrap();
    assert_eq!(n, 1);

    // Discount claims never reveal whether the email is eligible.
    let (s, v) = c
        .post(
            "/discount/claim",
            json!({"providerName": "Test", "email": a}),
        )
        .await;
    assert_eq!(s, 200);
    assert!(v["message"].is_string());
    let (s, _) = c
        .post(
            "/discount/claim",
            json!({"providerName": "Nope", "email": a}),
        )
        .await;
    assert_eq!(s, 400);
    // Rita is the instance admin (first account).
    let (s, v) = rc
        .post(
            "/discount/add-coupons",
            json!({"providerName": "Test", "codes": ["C1", " ", "C2"]}),
        )
        .await;
    assert_eq!(s, 200, "{v}");
    let (s, _) = sc
        .post(
            "/discount/add-coupons",
            json!({"providerName": "Test", "codes": ["C3"]}),
        )
        .await;
    assert_eq!(s, 401);

    // Ping and events.
    let (s, v) = c.get("/ping").await;
    assert_eq!((s, v["message"].as_str()), (200, Some("pong")));
    let (s, _) = c.get("/fire/db-m-ping").await;
    assert_eq!(s, 200);
    let id = uuid::Uuid::new_v4().to_string();
    let ev = |event: &str| json!({"id": id, "event": event, "app": "photos", "platform": "android", "data": {"src": "store"}});
    let (s, _) = c.post("/events", ev("install")).await;
    assert_eq!(s, 200);
    let (s, _) = c.post("/events", ev("log_in")).await;
    assert_eq!(s, 400);
    let (s, _) = rc.post("/events/user", ev("sign_up")).await;
    assert_eq!(s, 200);
    let (data,): (String,) =
        sqlx::query_as("SELECT data FROM events WHERE id = $1 AND event = 'sign_up'")
            .bind(&id)
            .fetch_one(&srv.state.db.pool)
            .await
            .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&data).unwrap()["src"],
        "store"
    );
}

#[tokio::test]
async fn paste_guard_and_consume() {
    let srv = server(&[]).await;
    let base = srv.url.clone();
    let ua = ("User-Agent", "Mozilla/5.0 Test");
    let create = json!({
        "encryptedData": crypto::b64(&crypto::random_bytes(64)),
        "decryptionHeader": crypto::b64(&crypto::random_bytes(24)),
        "encryptedPasteKey": crypto::b64(&crypto::random_bytes(48)),
        "encryptedPasteKeyNonce": crypto::b64(&crypto::random_bytes(24)),
        "kdfNonce": crypto::b64(&crypto::random_bytes(16)),
        "kdfMemLimit": 67108864,
        "kdfOpsLimit": 2,
    });
    let mut bad = create.clone();
    bad["kdfOpsLimit"] = json!(3);
    let (s, _) = http("POST", &format!("{base}/paste/create"), &[ua], Some(&bad)).await;
    assert_eq!(s, 400);
    let (s, v) = http(
        "POST",
        &format!("{base}/paste/create"),
        &[ua],
        Some(&create),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    let token = v["accessToken"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 10);
    let body = json!({"accessToken": token});

    // Link previews and missing guard cookies are refused.
    let (s, _) = http(
        "POST",
        &format!("{base}/paste/guard"),
        &[("User-Agent", "Slackbot 1.0")],
        Some(&body),
    )
    .await;
    assert_eq!(s, 410);
    let (s, _) = http(
        "POST",
        &format!("{base}/paste/consume"),
        &[ua, ("X-Paste-Consume", "1")],
        Some(&body),
    )
    .await;
    assert_eq!(s, 410);

    // Guard sets an HttpOnly cookie that consume requires.
    let cookie = guard_cookie(&base, &body, ua).await;
    let cookie_hdr = ("Cookie", cookie.as_str());
    let (s, _) = http(
        "POST",
        &format!("{base}/paste/consume"),
        &[ua, cookie_hdr],
        Some(&body),
    )
    .await;
    assert_eq!(s, 400, "X-Paste-Consume header required");
    let (s, v) = http(
        "POST",
        &format!("{base}/paste/consume"),
        &[ua, cookie_hdr, ("X-Paste-Consume", "1")],
        Some(&body),
    )
    .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["encryptedData"], create["encryptedData"]);
    assert_eq!(v["kdfMemLimit"], 67108864);
    // One-time: a second read is gone.
    let (s, _) = http(
        "POST",
        &format!("{base}/paste/consume"),
        &[ua, cookie_hdr, ("X-Paste-Consume", "1")],
        Some(&body),
    )
    .await;
    assert_eq!(s, 410);
}

/// POST /paste/guard and return the `paste_guard=...` cookie pair.
async fn guard_cookie(base: &str, body: &Value, ua: (&str, &str)) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let u = url::Url::parse(base).unwrap();
    let mut s = tokio::net::TcpStream::connect((u.host_str().unwrap(), u.port().unwrap()))
        .await
        .unwrap();
    let b = body.to_string();
    let req = format!(
        "POST /paste/guard HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Type: application/json\r\n{}: {}\r\nContent-Length: {}\r\n\r\n{b}",
        ua.0,
        ua.1,
        b.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).await.unwrap();
    assert!(out.starts_with("HTTP/1.1 200"), "{out}");
    let line = out
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("set-cookie:"))
        .expect("set-cookie");
    assert!(line.contains("HttpOnly") && line.contains("SameSite=Strict"));
    line.split_once(':')
        .unwrap()
        .1
        .trim()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}
