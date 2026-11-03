//! Public-link comments, reactions and anonymous identities, mounted behind a stand-in
//! `/public-collection` middleware that inserts `PublicCollectionCtx`.

mod social_support;

use serde_json::{Value, json};
use social_support::*;

#[tokio::test]
async fn public_social_flow() {
    let srv = start().await;
    let st = &srv.state;
    let (owner, owner_tok) = user(st, "owner@example.io").await;
    let (viewer, viewer_tok) = user(st, "john@example.io").await;
    collection(st, 20, owner, "photos").await;
    collection(st, 21, owner, "photos").await;
    add_file(st, 20, 200, owner).await;
    let url = start_public(st).await;
    let ctx = format!("20:{owner}:1");
    let disabled = format!("20:{owner}:0");
    let other = format!("21:{owner}:1");

    async fn call(
        url: &str,
        m: &str,
        path: &str,
        hs: &[(&str, &str)],
        body: Option<Value>,
    ) -> (u16, Value) {
        http(m, &format!("{url}/public-collection{path}"), hs, body).await
    }

    // No context → 401; comments disabled → 405 with museum's code.
    assert_eq!(call(&url, "GET", "/comments/diff", &[], None).await.0, 401);
    let (s, b) = call(
        &url,
        "GET",
        "/comments/diff",
        &[("X-Test-Collection", &disabled)],
        None,
    )
    .await;
    assert_eq!(
        (s, b["code"].as_str()),
        (405, Some("PUBLIC_COMMENT_DISABLED"))
    );

    // Anonymous identity: name too long → 400 ANON_NAME_TOO_LONG.
    let h = [("X-Test-Collection", ctx.as_str())];
    let (s, b) = call(
        &url,
        "POST",
        "/anon-identity",
        &h,
        Some(json!({"cipher": cipher(16 + 51), "nonce": "n"})),
    )
    .await;
    assert_eq!((s, b["code"].as_str()), (400, Some("ANON_NAME_TOO_LONG")));
    let (s, b) = call(
        &url,
        "POST",
        "/anon-identity",
        &h,
        Some(json!({"cipher": cipher(16 + 10), "nonce": "n"})),
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let anon = b["anonUserID"].as_str().unwrap().to_string();
    let anon_tok = b["token"].as_str().unwrap().to_string();
    assert!(anon.starts_with("anon_"));
    assert!(b["expiresAt"].as_i64().unwrap() > photovault_server::db::now_micros());

    // Anonymous comment without identity → 401; with header token → 201.
    let c = cipher(16 + 100);
    assert_eq!(
        call(
            &url,
            "POST",
            "/comments",
            &h,
            Some(json!({"cipher": c, "nonce": "n"}))
        )
        .await
        .0,
        401
    );
    // Too long (> 280 plaintext bytes).
    let (s, b) = call(&url, "POST", "/comments", &h, Some(json!({"cipher": cipher(16 + 281), "nonce": "n", "anonUserID": anon, "anonToken": anon_tok}))).await;
    assert_eq!(
        (s, b["code"].as_str()),
        (400, Some("PUBLIC_COMMENT_TOO_LONG"))
    );
    let anon_hdr = [
        ("X-Test-Collection", ctx.as_str()),
        ("X-Anon-User-Token", anon_tok.as_str()),
    ];
    let (s, b) = call(
        &url,
        "POST",
        "/comments",
        &anon_hdr,
        Some(json!({"fileID": 200, "cipher": c, "nonce": "n"})),
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let anon_comment = b["id"].as_str().unwrap().to_string();
    // Body identity (anonUserID + anonToken) works too; bearer + X-Anon-User-ID as well.
    let (s, _) = call(
        &url,
        "POST",
        "/comments",
        &h,
        Some(json!({"cipher": c, "nonce": "n", "anonUserID": anon, "anonToken": anon_tok})),
    )
    .await;
    assert_eq!(s, 201);
    let bearer = format!("Bearer {anon_tok}");
    let bearer_hdr = [
        ("X-Test-Collection", ctx.as_str()),
        ("X-Anon-User-ID", anon.as_str()),
        ("Authorization", bearer.as_str()),
    ];
    let (s, _) = call(
        &url,
        "POST",
        "/comments",
        &bearer_hdr,
        Some(json!({"cipher": c, "nonce": "n"})),
    )
    .await;
    assert_eq!(s, 201);
    // Mismatched subject → 400; forged token → 401.
    assert_eq!(call(&url, "POST", "/comments", &h, Some(json!({"cipher": c, "nonce": "n", "anonUserID": "anon_other", "anonToken": anon_tok}))).await.0, 400);
    assert_eq!(
        call(
            &url,
            "POST",
            "/comments",
            &h,
            Some(json!({"cipher": c, "nonce": "n", "anonUserID": anon, "anonToken": "x.y.z"}))
        )
        .await
        .0,
        401
    );
    // Identity from another collection's link → 403.
    let other_hdr = [
        ("X-Test-Collection", other.as_str()),
        ("X-Anon-User-Token", anon_tok.as_str()),
    ];
    assert_eq!(
        call(
            &url,
            "POST",
            "/comments",
            &other_hdr,
            Some(json!({"cipher": c, "nonce": "n"}))
        )
        .await
        .0,
        403
    );

    // A signed-in viewer comments through the link (no share needed, like museum).
    let viewer_hdr = [
        ("X-Test-Collection", ctx.as_str()),
        ("X-Auth-Token", viewer_tok.as_str()),
    ];
    let (s, b) = call(
        &url,
        "POST",
        "/comments",
        &viewer_hdr,
        Some(json!({"fileID": 200, "parentCommentID": anon_comment, "cipher": c, "nonce": "n"})),
    )
    .await;
    assert_eq!(s, 201, "{b}");
    let viewer_comment = b["id"].as_str().unwrap().to_string();
    // Invalid auth token → 401.
    assert_eq!(
        call(
            &url,
            "POST",
            "/comments",
            &[
                ("X-Test-Collection", ctx.as_str()),
                ("X-Auth-Token", "bogus")
            ],
            Some(json!({"cipher": c, "nonce": "n"}))
        )
        .await
        .0,
        401
    );

    // Diff shows anonymous author fields.
    let (s, b) = call(&url, "GET", "/comments/diff?fileID=200", &h, None).await;
    assert_eq!(s, 200);
    let list = b["comments"].as_array().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0]["userID"], -1);
    assert_eq!(list[0]["anonUserID"], anon.as_str());
    assert_eq!(list[1]["parentCommentID"], anon_comment.as_str());

    // Edit: only the author, and only via this collection's link.
    let c2 = cipher(16 + 20);
    assert_eq!(
        call(
            &url,
            "PUT",
            &format!("/comments/{anon_comment}"),
            &viewer_hdr,
            Some(json!({"cipher": c2, "nonce": "n"}))
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &url,
            "PUT",
            &format!("/comments/{anon_comment}"),
            &anon_hdr,
            Some(json!({"cipher": c2, "nonce": "n"}))
        )
        .await
        .0,
        200
    );

    // Reactions: anonymous on file, viewer on a comment.
    let rc = reaction_cipher();
    let (s, b) = call(
        &url,
        "POST",
        "/reactions",
        &anon_hdr,
        Some(json!({"fileID": 200, "cipher": rc, "nonce": "n"})),
    )
    .await;
    assert_eq!(s, 200, "{b}");
    let anon_reaction = b["id"].as_str().unwrap().to_string();
    assert_eq!(
        call(
            &url,
            "POST",
            "/reactions",
            &anon_hdr,
            Some(json!({"fileID": 200, "cipher": "short", "nonce": "n"}))
        )
        .await
        .0,
        400
    );
    let (s, _) = call(
        &url,
        "POST",
        "/reactions",
        &viewer_hdr,
        Some(json!({"fileID": 200, "commentID": anon_comment, "cipher": rc, "nonce": "n"})),
    )
    .await;
    assert_eq!(s, 200);
    let (_, b) = call(&url, "GET", "/reactions/diff", &h, None).await;
    assert_eq!(b["reactions"].as_array().unwrap().len(), 2);
    // Delete reaction: others → 403, the reactor (empty body) → 200.
    assert_eq!(
        call(
            &url,
            "DELETE",
            &format!("/reactions/{anon_reaction}"),
            &viewer_hdr,
            None
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &url,
            "DELETE",
            &format!("/reactions/{anon_reaction}"),
            &anon_hdr,
            None
        )
        .await
        .0,
        200
    );

    // Participants: masked emails of signed-in participants only.
    let (s, b) = call(&url, "GET", "/participants/masked-emails", &h, None).await;
    assert_eq!(s, 200);
    assert_eq!(
        b["participants"],
        json!([{"userID": viewer, "emailMasked": "jo**@e********o"}])
    );

    // Anon profiles (public and private views).
    let (_, b) = call(&url, "GET", "/anon-profiles", &h, None).await;
    assert_eq!(b["profiles"][0]["anonUserID"], anon.as_str());
    assert_eq!(b["profiles"][0]["collectionID"], 20);
    let (s, b) = http(
        "GET",
        &format!("{}/social/anon-profiles?collectionID=20", srv.url),
        &[("X-Auth-Token", owner_tok.as_str())],
        None,
    )
    .await;
    assert_eq!(s, 200);
    assert_eq!(b["profiles"].as_array().unwrap().len(), 1);
    let (_, b) = http(
        "GET",
        &format!("{}/comments-reactions/updated-at", srv.url),
        &[("X-Auth-Token", owner_tok.as_str())],
        None,
    )
    .await;
    assert!(b["updates"][0]["anonProfilesUpdatedAt"].as_i64().unwrap() > 0);
    // Delete comment: a signed-in non-member cannot moderate (404 like museum: no share row); the owner can.
    assert_eq!(
        call(
            &url,
            "DELETE",
            &format!("/comments/{anon_comment}"),
            &viewer_hdr,
            Some(json!({}))
        )
        .await
        .0,
        404
    );
    let owner_hdr = [
        ("X-Test-Collection", ctx.as_str()),
        ("X-Auth-Token", owner_tok.as_str()),
    ];
    assert_eq!(
        call(
            &url,
            "DELETE",
            &format!("/comments/{anon_comment}"),
            &owner_hdr,
            None
        )
        .await
        .0,
        200
    );
    // The viewer deletes their own comment.
    assert_eq!(
        call(
            &url,
            "DELETE",
            &format!("/comments/{viewer_comment}"),
            &viewer_hdr,
            None
        )
        .await
        .0,
        200
    );

    // Unified public diff.
    let (s, b) = call(&url, "GET", "/social/diff", &h, None).await;
    assert_eq!(s, 200);
    let gone = b["comments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == anon_comment.as_str())
        .unwrap()
        .clone();
    assert_eq!(gone["isDeleted"], true);
    assert!(gone.get("cipher").is_none());
    assert!(
        b["reactions"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["isDeleted"] == true)
    );
    assert_eq!(b["hasMoreComments"], false);
}
