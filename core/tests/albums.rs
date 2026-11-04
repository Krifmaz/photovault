//! Albums, sharing, public links, hidden albums and trash, end to end
//! against the real Photovault server in process, driven through `Core`
//! exactly as the Android app does. Whatever one device does, a second
//! device (or a second user) must see after a sync: the server holds only
//! what Ente's apps would write.

mod support;

use photovault_core::crypto;
use photovault_core::{Core, LoginStep, PublicLinkUpdate, ShareRole};
use serde_json::Value;
use std::sync::Arc;
use support::{TestCore, jpeg, server, signup};

const PASSWORD: &str = "correct horse battery staple";

fn logged_in(step: LoginStep) {
    assert!(matches!(step, LoginStep::Done { .. }), "{step:?}");
}

async fn device(url: &str, email: &str) -> TestCore {
    let d = TestCore::connected(url).await;
    logged_in(d.core.login(email.into(), PASSWORD.into()).await.unwrap());
    d
}

/// Upload `n` small JPEGs into the uncategorized collection; returns their ids.
async fn upload(d: &TestCore, n: u8) -> Vec<i64> {
    for i in 0..n {
        let p = d.dir.path().join(format!("IMG_{i:04}.jpg"));
        std::fs::write(&p, jpeg(64, 48, i.wrapping_mul(37))).unwrap();
        d.core
            .enqueue_upload(p.display().to_string(), None)
            .unwrap();
    }
    let st = d.core.run_uploads().await.unwrap();
    assert_eq!((st.pending, st.failed), (0, 0), "{st:?}");
    d.core.timeline_page(0, 1000).iter().map(|i| i.id).collect()
}

fn ids_in(core: &Arc<Core>, cid: i64) -> Vec<i64> {
    let mut v: Vec<i64> = core
        .collection_page(cid, 0, 1000)
        .iter()
        .map(|i| i.id)
        .collect();
    v.sort();
    v
}

fn base58_decode(s: &str) -> Vec<u8> {
    const A: &str = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut bytes: Vec<u8> = Vec::new(); // little endian
    for c in s.chars() {
        let mut carry = A.find(c).expect("base58 digit") as u32;
        for b in bytes.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            bytes.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    let zeros = s.chars().take_while(|c| *c == '1').count();
    let mut out = vec![0u8; zeros];
    out.extend(bytes.iter().rev());
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn albums_create_rename_add_remove_hide_delete() {
    const EMAIL: &str = "alice@example.org";
    let srv = server(&[EMAIL]).await;
    signup(&srv.url, EMAIL, PASSWORD).await;
    let a = device(&srv.url, EMAIL).await;
    let files = upload(&a, 3).await;
    assert_eq!(files.len(), 3);

    // Create and rename.
    let album = a.core.create_album("  Trip  ".into()).await.unwrap();
    assert_eq!(album.name, "Trip");
    assert_eq!(album.kind, "album");
    a.core
        .rename_album(album.id, "Summer trip".into())
        .await
        .unwrap();
    let d = a.core.album_details(album.id).unwrap();
    assert_eq!(d.collection.name, "Summer trip");
    assert!(d.owned && !d.hidden && d.sharees.is_empty() && d.public_link.is_none());
    assert_eq!(d.my_role, ShareRole::Owner);

    // Add two files.
    a.core
        .add_to_album(album.id, files[..2].to_vec())
        .await
        .unwrap();
    let mut want = files[..2].to_vec();
    want.sort();
    assert_eq!(ids_in(&a.core, album.id), want);
    assert_eq!(a.core.album_details(album.id).unwrap().collection.count, 2);
    assert!(a.core.albums().iter().any(|x| x.collection.id == album.id));

    // A second device sees the same album, name and files.
    let b = device(&srv.url, EMAIL).await;
    b.core.sync(None).await.unwrap();
    let bd = b.core.album_details(album.id).unwrap();
    assert_eq!(bd.collection.name, "Summer trip");
    assert_eq!(ids_in(&b.core, album.id), want);

    // Remove one: it is still in Uncategorized, so it just leaves the album.
    a.core
        .remove_from_album(album.id, vec![files[0]])
        .await
        .unwrap();
    assert_eq!(ids_in(&a.core, album.id), vec![files[1]]);
    assert_eq!(a.core.timeline_count(), 3);
    b.core.sync(None).await.unwrap();
    assert_eq!(ids_in(&b.core, album.id), vec![files[1]]);

    // Hide the album: it leaves albums() and shows in hidden_albums().
    a.core.set_album_hidden(album.id, true).await.unwrap();
    assert!(!a.core.albums().iter().any(|x| x.collection.id == album.id));
    assert!(
        a.core
            .hidden_albums()
            .iter()
            .any(|x| x.collection.id == album.id)
    );
    b.core.sync(None).await.unwrap();
    assert!(
        b.core.album_details(album.id).unwrap().hidden,
        "hidden on device B"
    );
    a.core.set_album_hidden(album.id, false).await.unwrap();
    assert!(a.core.albums().iter().any(|x| x.collection.id == album.id));
    b.core.sync(None).await.unwrap();
    assert!(!b.core.album_details(album.id).unwrap().hidden);

    // Hide a file: it moves to the default hidden collection, out of the timeline.
    a.core.hide_files(vec![files[2]]).await.unwrap();
    assert_eq!(a.core.timeline_count(), 2);
    let hidden = a.core.hidden_albums();
    let dh = hidden
        .iter()
        .find(|h| h.is_default_hidden)
        .expect("default hidden");
    assert_eq!(ids_in(&a.core, dh.collection.id), vec![files[2]]);
    b.core.sync(None).await.unwrap();
    assert_eq!(b.core.timeline_count(), 2);
    a.core.unhide_files(vec![files[2]], None).await.unwrap();
    assert_eq!(a.core.timeline_count(), 3);
    assert!(ids_in(&a.core, dh.collection.id).is_empty());

    // Delete the album keeping its photos: they stay in the library.
    a.core.delete_album(album.id, true).await.unwrap();
    assert!(a.core.album_details(album.id).is_none());
    assert_eq!(a.core.timeline_count(), 3);
    b.core.sync(None).await.unwrap();
    assert!(b.core.album_details(album.id).is_none());
    assert_eq!(b.core.timeline_count(), 3);

    // A photo only in an album moves to Uncategorized when removed from it.
    let only = a.core.create_album("Only".into()).await.unwrap();
    let unc = a
        .core
        .albums()
        .into_iter()
        .find(|x| x.collection.kind == "uncategorized")
        .unwrap();
    a.core.add_to_album(only.id, vec![files[0]]).await.unwrap();
    a.core
        .remove_from_album(unc.collection.id, vec![files[0]])
        .await
        .unwrap();
    assert_eq!(ids_in(&a.core, only.id), vec![files[0]]);
    a.core
        .remove_from_album(only.id, vec![files[0]])
        .await
        .unwrap();
    assert!(ids_in(&a.core, unc.collection.id).contains(&files[0]));
    assert_eq!(a.core.timeline_count(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sharing_and_public_links() {
    const ALICE: &str = "alice@example.org";
    const BOB: &str = "bob@example.org";
    let srv = server(&[ALICE, BOB]).await;
    signup(&srv.url, ALICE, PASSWORD).await;
    signup(&srv.url, BOB, PASSWORD).await;
    let a = device(&srv.url, ALICE).await;
    let files = upload(&a, 2).await;
    let album = a.core.create_album("Family".into()).await.unwrap();
    a.core.add_to_album(album.id, files.clone()).await.unwrap();

    // Unknown email: a clear error.
    let e = a
        .core
        .share_album(album.id, "nobody@example.org".into(), ShareRole::Viewer)
        .await
        .unwrap_err();
    assert!(
        e.to_string().contains("404") || e.to_string().contains("no account"),
        "{e}"
    );

    // Share with Bob as a collaborator.
    let sharees = a
        .core
        .share_album(album.id, BOB.into(), ShareRole::Collaborator)
        .await
        .unwrap();
    assert_eq!(sharees.len(), 1);
    assert_eq!(sharees[0].email, BOB);
    assert_eq!(sharees[0].role, ShareRole::Collaborator);
    assert_eq!(a.core.album_details(album.id).unwrap().sharees, sharees);

    // Bob syncs: the album, its name and both photos decrypt with the sealed key.
    let b = device(&srv.url, BOB).await;
    b.core.sync(None).await.unwrap();
    let bd = b
        .core
        .album_details(album.id)
        .expect("shared album on Bob's side");
    assert!(!bd.owned);
    assert_eq!(bd.owner_email, ALICE);
    assert_eq!(bd.my_role, ShareRole::Collaborator);
    assert_eq!(bd.collection.name, "Family");
    let mut want = files.clone();
    want.sort();
    assert_eq!(ids_in(&b.core, album.id), want);
    assert!(!b.core.thumbnail(files[0]).await.unwrap().is_empty());

    // A collaborator can add their own photos.
    let bob_files = upload(&b, 1).await;
    b.core
        .add_to_album(album.id, bob_files.clone())
        .await
        .unwrap();
    a.core.sync(None).await.unwrap();
    assert!(ids_in(&a.core, album.id).contains(&bob_files[0]));

    // Public link: the key is in the fragment, and the public album opens with it.
    let link = a.core.create_public_link(album.id).await.unwrap();
    let (base, frag) = link.url.split_once('#').expect("key fragment");
    assert!(!link.password_enabled);
    assert_eq!(link.valid_till_us, 0);
    let token = url::Url::parse(base)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "t")
        .map(|(_, v)| v.into_owned())
        .expect("t= token");
    let key = base58_decode(frag);
    assert_eq!(key.len(), 32);
    let info: Value = reqwest::Client::new()
        .get(format!("{}/public-collection/info", srv.url))
        .header("X-Auth-Access-Token", &token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let c = &info["collection"];
    let name = crypto::secretbox_open(
        &crypto::unb64(c["encryptedName"].as_str().unwrap()).unwrap(),
        &crypto::unb64(c["nameDecryptionNonce"].as_str().unwrap()).unwrap(),
        &key,
    )
    .expect("the fragment's key opens the album");
    assert_eq!(name, b"Family");

    // Update: expiry, device limit, no downloads, collect, password; then remove the password.
    let until = 4_102_444_800_000_000; // 2100-01-01
    let l2 = a
        .core
        .update_public_link(
            album.id,
            PublicLinkUpdate {
                valid_till_us: Some(until),
                device_limit: Some(5),
                enable_download: Some(false),
                enable_collect: Some(true),
                enable_join: None,
                password: Some("hunter2".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(l2.valid_till_us, until);
    assert_eq!(l2.device_limit, 5);
    assert!(!l2.enable_download && l2.enable_collect && l2.password_enabled);
    assert_eq!(l2.url, link.url);
    assert_eq!(
        a.core.album_details(album.id).unwrap().public_link,
        Some(l2)
    );
    let l3 = a
        .core
        .update_public_link(
            album.id,
            PublicLinkUpdate {
                password: Some(String::new()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!l3.password_enabled);

    // A second device of Alice's sees the link (with the same key) after a sync.
    let a2 = device(&srv.url, ALICE).await;
    a2.core.sync(None).await.unwrap();
    assert_eq!(
        a2.core
            .album_details(album.id)
            .unwrap()
            .public_link
            .map(|l| l.url),
        Some(link.url.clone())
    );

    // Remove the link: the public page stops answering.
    a.core.delete_public_link(album.id).await.unwrap();
    assert!(
        a.core
            .album_details(album.id)
            .unwrap()
            .public_link
            .is_none()
    );
    let st = reqwest::Client::new()
        .get(format!("{}/public-collection/info", srv.url))
        .header("X-Auth-Access-Token", &token)
        .send()
        .await
        .unwrap()
        .status();
    assert!(!st.is_success(), "{st}");

    // Unshare: Bob loses the album on his next sync.
    let left = a.core.unshare_album(album.id, BOB.into()).await.unwrap();
    assert!(left.is_empty());
    b.core.sync(None).await.unwrap();
    assert!(b.core.album_details(album.id).is_none());

    // Share again as viewer; Bob leaves.
    a.core
        .share_album(album.id, BOB.into(), ShareRole::Viewer)
        .await
        .unwrap();
    b.core.sync(None).await.unwrap();
    assert_eq!(
        b.core.album_details(album.id).unwrap().my_role,
        ShareRole::Viewer
    );
    b.core.leave_album(album.id).await.unwrap();
    assert!(b.core.album_details(album.id).is_none());
    b.core.sync(None).await.unwrap();
    assert!(b.core.album_details(album.id).is_none());
    a.core.sync(None).await.unwrap();
    assert!(a.core.album_details(album.id).unwrap().sharees.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn trash_restore_delete_empty() {
    const EMAIL: &str = "alice@example.org";
    let srv = server(&[EMAIL]).await;
    signup(&srv.url, EMAIL, PASSWORD).await;
    let a = device(&srv.url, EMAIL).await;
    let files = upload(&a, 4).await;
    assert_eq!(a.core.sync_trash().await.unwrap(), 0);

    // Trash three: out of the timeline, into the trash right away.
    a.core.move_to_trash(files[..3].to_vec()).await.unwrap();
    assert_eq!(a.core.timeline_count(), 1);
    assert_eq!(a.core.trash_count(), 3);
    let page = a.core.trash_page(0, 10);
    assert!(page.iter().all(|t| t.delete_by_us > 0));
    // The trash still has thumbnails.
    assert!(!a.core.thumbnail(files[0]).await.unwrap().is_empty());
    assert_eq!(a.core.sync_trash().await.unwrap(), 3);

    // Another device: sync shows them gone from the timeline and in the trash.
    let b = device(&srv.url, EMAIL).await;
    b.core.sync(None).await.unwrap();
    assert_eq!(b.core.timeline_count(), 1);
    assert_eq!(b.core.sync_trash().await.unwrap(), 3);
    assert!(!b.core.thumbnail(files[1]).await.unwrap().is_empty());

    // Restore one into a new album.
    let album = a.core.create_album("Saved".into()).await.unwrap();
    a.core
        .restore_from_trash(vec![files[0]], Some(album.id))
        .await
        .unwrap();
    assert_eq!(ids_in(&a.core, album.id), vec![files[0]]);
    assert_eq!(a.core.trash_count(), 2);
    assert_eq!(a.core.timeline_count(), 2);
    // Restore one to Uncategorized.
    a.core
        .restore_from_trash(vec![files[1]], None)
        .await
        .unwrap();
    assert_eq!(a.core.timeline_count(), 3);
    assert_eq!(a.core.trash_count(), 1);

    b.core.sync(None).await.unwrap();
    assert_eq!(b.core.timeline_count(), 3);
    assert_eq!(b.core.sync_trash().await.unwrap(), 1);

    // Delete one for good.
    a.core.delete_from_trash(vec![files[2]]).await.unwrap();
    assert_eq!(a.core.trash_count(), 0);
    assert_eq!(b.core.sync_trash().await.unwrap(), 0);

    // Trash the rest, then empty the trash.
    a.core.move_to_trash(files.clone()).await.unwrap();
    // files[2] is gone already; the other three are trashed.
    assert_eq!(a.core.timeline_count(), 0);
    a.core.sync_trash().await.unwrap();
    assert_eq!(a.core.trash_count(), 3);
    a.core.empty_trash().await.unwrap();
    assert_eq!(a.core.trash_count(), 0);
    assert_eq!(a.core.sync_trash().await.unwrap(), 0);
    assert_eq!(b.core.sync_trash().await.unwrap(), 0);

    // Deleting an album with its photos sends them to the trash; the key of
    // the deleted album still opens them there.
    let more = upload(&a, 2).await;
    let doomed = a.core.create_album("Doomed".into()).await.unwrap();
    a.core.add_to_album(doomed.id, more.clone()).await.unwrap();
    let unc = a
        .core
        .albums()
        .into_iter()
        .find(|x| x.collection.kind == "uncategorized")
        .unwrap();
    a.core
        .remove_from_album(unc.collection.id, more.clone())
        .await
        .unwrap();
    assert_eq!(ids_in(&a.core, doomed.id).len(), 2);
    a.core.delete_album(doomed.id, false).await.unwrap();
    b.core.sync(None).await.unwrap();
    assert!(b.core.album_details(doomed.id).is_none());
    // The server trashes them in its background pass.
    photovault_server::library::tasks::run_once(&srv.state).await;
    assert_eq!(
        b.core.sync_trash().await.unwrap(),
        2,
        "trashed files of a deleted album"
    );
    assert!(!b.core.thumbnail(more[0]).await.unwrap().is_empty());
    assert_eq!(a.core.sync_trash().await.unwrap(), 2);
    a.core.restore_from_trash(more.clone(), None).await.unwrap();
    assert_eq!(a.core.timeline_count(), 2);
}
