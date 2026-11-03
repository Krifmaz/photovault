//! Private (signed-in) comments, reactions and social endpoints.

mod social_support;

use serde_json::json;
use social_support::*;

#[tokio::test]
async fn comments_reactions_and_access() {
    let srv = start().await;
    let st = &srv.state;
    let (owner, owner_tok) = user(st, "owner@example.org").await;
    let (sharee, sharee_tok) = user(st, "sharee@example.org").await;
    let (admin, admin_tok) = user(st, "admin@example.org").await;
    let (_stranger, stranger_tok) = user(st, "stranger@example.org").await;
    collection(st, 10, owner, "photos").await;
    share(st, 10, owner, sharee, "VIEWER").await;
    share(st, 10, owner, admin, "ADMIN").await;
    add_file(st, 10, 100, owner).await;
    let base = &srv.url;
    let h = |t: &str| vec![("X-Auth-Token", t.to_string())];
    macro_rules! call {
        ($m:expr, $path:expr, $tok:expr, $body:expr) => {{
            let hs = h($tok);
            let hs: Vec<(&str, &str)> = hs.iter().map(|(k, v)| (*k, v.as_str())).collect();
            http($m, &format!("{base}{}", $path), &hs, $body).await
        }};
    }

    // Unauthenticated → 401.
    assert_eq!(
        http(
            "GET",
            &format!("{base}/comments/diff?collectionID=10"),
            &[],
            None
        )
        .await
        .0,
        401
    );
    // Bad collectionID → 400.
    assert_eq!(
        call!("GET", "/comments/diff?collectionID=0", &owner_tok, None).0,
        400
    );
    // Stranger has no access → 404 (museum: no share row).
    assert_eq!(
        call!("GET", "/comments/diff?collectionID=10", &stranger_tok, None).0,
        404
    );

    // Create a file comment (server-generated id).
    let c = cipher(40);
    let (s, b) = call!(
        "POST",
        "/comments",
        &sharee_tok,
        Some(json!({"collectionID": 10, "fileID": 100, "cipher": c, "nonce": "n1"}))
    );
    assert_eq!(s, 201, "{b}");
    let cid = b["id"].as_str().unwrap().to_string();
    assert!(cid.starts_with("cmt_") && cid.len() == 25);

    // Client-supplied id; invalid id → 400.
    let my_id = "cmt_abcdefghijklmnopqrstu";
    let (s, b) = call!(
        "POST",
        "/comments",
        &owner_tok,
        Some(json!({"id": my_id, "collectionID": 10, "cipher": c, "nonce": "n"}))
    );
    assert_eq!((s, b["id"].as_str()), (201, Some(my_id)));
    assert_eq!(
        call!(
            "POST",
            "/comments",
            &owner_tok,
            Some(json!({"id": "cmt_../x", "collectionID": 10, "cipher": c, "nonce": "n"}))
        )
        .0,
        400
    );
    // Missing cipher → 400; file not in collection → 403; stranger → 404.
    assert_eq!(
        call!(
            "POST",
            "/comments",
            &owner_tok,
            Some(json!({"collectionID": 10, "nonce": "n"}))
        )
        .0,
        400
    );
    assert_eq!(
        call!(
            "POST",
            "/comments",
            &owner_tok,
            Some(json!({"collectionID": 10, "fileID": 999, "cipher": c, "nonce": "n"}))
        )
        .0,
        403
    );
    assert_eq!(
        call!(
            "POST",
            "/comments",
            &stranger_tok,
            Some(json!({"collectionID": 10, "cipher": c, "nonce": "n"}))
        )
        .0,
        404
    );

    // Replies must match the parent's file scope.
    assert_eq!(
        call!(
            "POST",
            "/comments",
            &owner_tok,
            Some(json!({"collectionID": 10, "parentCommentID": cid, "cipher": c, "nonce": "n"}))
        )
        .0,
        400
    );
    let (s, b) = call!(
        "POST",
        "/comments",
        &owner_tok,
        Some(
            json!({"collectionID": 10, "fileID": 100, "parentCommentID": cid, "cipher": c, "nonce": "n"})
        )
    );
    assert_eq!(s, 201);
    let reply = b["id"].as_str().unwrap().to_string();

    // Diff: all three, file filter → two.
    let (s, b) = call!("GET", "/comments/diff?collectionID=10", &sharee_tok, None);
    assert_eq!(s, 200);
    assert_eq!(b["comments"].as_array().unwrap().len(), 3);
    assert_eq!(b["hasMore"], false);
    let first = &b["comments"][0];
    assert_eq!(first["id"], cid.as_str());
    assert_eq!(first["collectionID"], 10);
    assert_eq!(first["fileID"], 100);
    assert_eq!(first["userID"], sharee);
    assert_eq!(first["isDeleted"], false);
    assert!(first.get("parentCommentID").is_none() && first.get("anonUserID").is_none());
    let (_, b) = call!(
        "GET",
        "/comments/diff?collectionID=10&fileID=100&limit=1",
        &sharee_tok,
        None
    );
    assert_eq!(b["comments"].as_array().unwrap().len(), 1);
    assert_eq!(b["hasMore"], true);
    let since = b["comments"][0]["updatedAt"].as_i64().unwrap();
    let (_, b) = call!(
        "GET",
        &format!("/comments/diff?collectionID=10&fileID=100&sinceTime={since}"),
        &sharee_tok,
        None
    );
    assert_eq!(b["comments"][0]["id"], reply.as_str());

    // Update: only the author.
    let c2 = cipher(50);
    assert_eq!(
        call!(
            "PUT",
            &format!("/comments/{cid}"),
            &owner_tok,
            Some(json!({"cipher": c2, "nonce": "n2"}))
        )
        .0,
        403
    );
    assert_eq!(
        call!(
            "PUT",
            &format!("/comments/{cid}"),
            &sharee_tok,
            Some(json!({"cipher": c2, "nonce": "n2"}))
        )
        .0,
        200
    );
    assert_eq!(
        call!(
            "PUT",
            "/comments/cmt_nope",
            &sharee_tok,
            Some(json!({"cipher": c2, "nonce": "n2"}))
        )
        .0,
        404
    );

    // Reactions: wrong length → 400; on file; on comment (file context required).
    let rc = reaction_cipher();
    assert_eq!(rc.len(), 156);
    assert_eq!(
        call!(
            "PUT",
            "/reactions",
            &sharee_tok,
            Some(json!({"collectionID": 10, "fileID": 100, "cipher": "abc", "nonce": "n"}))
        )
        .0,
        400
    );
    let (s, b) = call!(
        "PUT",
        "/reactions",
        &sharee_tok,
        Some(json!({"collectionID": 10, "fileID": 100, "cipher": rc, "nonce": "n"}))
    );
    assert_eq!(s, 200, "{b}");
    let r1 = b["id"].as_str().unwrap().to_string();
    assert!(r1.starts_with("rct_"));
    // Upsert on the same target returns the same id.
    let (_, b) = call!(
        "PUT",
        "/reactions",
        &sharee_tok,
        Some(
            json!({"collectionID": 10, "fileID": 100, "cipher": reaction_cipher(), "nonce": "n2"})
        )
    );
    assert_eq!(b["id"].as_str(), Some(r1.as_str()));
    assert_eq!(
        call!(
            "PUT",
            "/reactions",
            &owner_tok,
            Some(json!({"collectionID": 10, "commentID": cid, "cipher": rc, "nonce": "n"}))
        )
        .0,
        400
    );
    let (s, b) = call!(
        "PUT",
        "/reactions",
        &owner_tok,
        Some(
            json!({"collectionID": 10, "fileID": 100, "commentID": cid, "cipher": rc, "nonce": "n"})
        )
    );
    assert_eq!(s, 200);
    let r2 = b["id"].as_str().unwrap().to_string();
    // Collection-level reaction.
    assert_eq!(
        call!(
            "PUT",
            "/reactions",
            &admin_tok,
            Some(json!({"collectionID": 10, "cipher": rc, "nonce": "n"}))
        )
        .0,
        200
    );

    let (_, b) = call!("GET", "/reactions/diff?collectionID=10", &owner_tok, None);
    assert_eq!(b["reactions"].as_array().unwrap().len(), 3);
    let on_comment = b["reactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == r2.as_str())
        .unwrap();
    assert_eq!(on_comment["commentID"], cid.as_str());
    assert!(on_comment.get("fileID").is_none());
    // fileID filter includes reactions on that file's comments.
    let (_, b) = call!(
        "GET",
        "/reactions/diff?collectionID=10&fileID=100",
        &owner_tok,
        None
    );
    assert_eq!(b["reactions"].as_array().unwrap().len(), 2);
    let (_, b) = call!(
        "GET",
        &format!("/reactions/diff?collectionID=10&commentID={cid}"),
        &owner_tok,
        None
    );
    assert_eq!(b["reactions"].as_array().unwrap().len(), 1);

    // Delete reaction: only the reactor.
    assert_eq!(
        call!("DELETE", &format!("/reactions/{r1}"), &owner_tok, None).0,
        403
    );
    assert_eq!(
        call!("DELETE", &format!("/reactions/{r1}"), &sharee_tok, None).0,
        200
    );

    // Viewer cannot delete others' comments; admin can (moderation); reactions on it go too.
    assert_eq!(
        call!("DELETE", &format!("/comments/{reply}"), &sharee_tok, None).0,
        403
    );
    assert_eq!(
        call!("DELETE", &format!("/comments/{cid}"), &admin_tok, None).0,
        200
    );
    assert_eq!(
        call!("DELETE", &format!("/comments/{cid}"), &sharee_tok, None).0,
        404
    );
    // Cannot reply to or react on a deleted comment.
    assert_eq!(
        call!("POST", "/comments", &owner_tok, Some(json!({"collectionID": 10, "fileID": 100, "parentCommentID": cid, "cipher": c, "nonce": "n"}))).0,
        400
    );

    // Unified diff: tombstones carry no cipher.
    let (s, b) = call!("GET", "/social/diff?collectionID=10", &owner_tok, None);
    assert_eq!(s, 200);
    let dc = b["comments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == cid.as_str())
        .unwrap()
        .clone();
    assert_eq!(dc["isDeleted"], true);
    assert!(dc.get("cipher").is_none() && dc.get("nonce").is_none());
    let dr = b["reactions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == r2.as_str())
        .unwrap()
        .clone();
    assert_eq!(dr["isDeleted"], true);
    assert_eq!(b["hasMoreComments"], false);
    assert_eq!(b["hasMoreReactions"], false);
    // commentsSinceTime overrides sinceTime for comments only.
    let far = 9_000_000_000_000_000i64;
    let (_, b) = call!(
        "GET",
        &format!("/social/diff?collectionID=10&sinceTime={far}&commentsSinceTime=0"),
        &owner_tok,
        None
    );
    assert_eq!(b["comments"].as_array().unwrap().len(), 3);
    assert_eq!(b["reactions"].as_array().unwrap().len(), 0);

    // Anon profiles: none yet → null.
    let (s, b) = call!(
        "GET",
        "/social/anon-profiles?collectionID=10",
        &sharee_tok,
        None
    );
    assert_eq!(s, 200);
    assert!(b["profiles"].is_null());

    // Latest updates.
    let (s, b) = call!("GET", "/comments-reactions/updated-at", &sharee_tok, None);
    assert_eq!(s, 200);
    let u = &b["updates"][0];
    assert_eq!(u["collectionID"], 10);
    assert!(u["commentsUpdatedAt"].as_i64().unwrap() > 0);
    assert!(u["reactionsUpdatedAt"].as_i64().unwrap() > 0);
    assert!(u.get("anonProfilesUpdatedAt").is_none());
    let (_, b) = call!("GET", "/comments-reactions/updated-at", &stranger_tok, None);
    assert!(b["updates"].is_null());

    // Leaving a collection removes the user's activity.
    photovault_server::social::remove_user_social_activity(&st.db, 10, owner)
        .await
        .unwrap();
    let (_, b) = call!("GET", "/comments/diff?collectionID=10", &admin_tok, None);
    assert!(
        b["comments"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["userID"] != owner || c["isDeleted"] == true)
    );

    // Deleted collection → 404; other app's collection → 403.
    collection(st, 11, owner, "locker").await;
    assert_eq!(
        call!("GET", "/comments/diff?collectionID=11", &owner_tok, None).0,
        403
    );
    sqlx::query("UPDATE collections SET is_deleted = TRUE WHERE collection_id = 10")
        .execute(&st.db.pool)
        .await
        .unwrap();
    assert_eq!(
        call!("GET", "/comments/diff?collectionID=10", &owner_tok, None).0,
        404
    );
}
