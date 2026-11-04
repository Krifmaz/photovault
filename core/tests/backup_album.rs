//! Contract v1.2: `backup_album`, the album a device folder backs up into,
//! against the real server in process.

mod support;

use photovault_core::api::Api;
use photovault_core::{LoginStep, crypto};
use serde_json::Value;
use support::{TestCore, jpeg, server, signup};

const EMAIL: &str = "camera@example.org";
const PASSWORD: &str = "correct horse battery staple";

async fn signed_in(url: &str) -> TestCore {
    let c = TestCore::connected(url).await;
    match c.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap() {
        LoginStep::Done { .. } => c,
        other => panic!("expected Done, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn folder_album_like_ente() {
    let srv = server(&[EMAIL]).await;
    let acc = signup(&srv.url, EMAIL, PASSWORD).await;
    let a = signed_in(&srv.url).await;

    // Created once, then reused (also for a name with stray spaces).
    let id = a.core.backup_album("Camera".into()).await.unwrap();
    assert_eq!(a.core.backup_album(" Camera ".into()).await.unwrap(), id);
    let other = a.core.backup_album("Screenshots".into()).await.unwrap();
    assert_ne!(other, id);
    assert!(a.core.backup_album("  ".into()).await.is_err());

    // Upload into it through the queue.
    let path = a.dir.path().join("IMG_0002.jpg");
    std::fs::write(&path, jpeg(320, 240, 3)).unwrap();
    a.core
        .enqueue_upload(path.display().to_string(), Some(id))
        .unwrap();
    let st = a.core.run_uploads().await.unwrap();
    assert_eq!((st.pending, st.failed), (0, 0), "{st:?}");

    // On the server: Ente's folder collection, name and path the same secretbox under the collection key.
    let api = Api::new(&srv.url).with_token(acc.token.clone());
    let res: Value = api
        .get_json("/collections/v2", &[("sinceTime", "0".to_string())])
        .await
        .unwrap();
    let col = res["collections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"].as_i64() == Some(id))
        .expect("collection on the server")
        .clone();
    assert_eq!(col["type"], "folder");
    assert_eq!(col["attributes"]["version"], 1);
    assert_eq!(col["attributes"]["encryptedPath"], col["encryptedName"]);
    assert_eq!(
        col["attributes"]["pathDecryptionNonce"],
        col["nameDecryptionNonce"]
    );
    let s = |v: &Value| crypto::unb64(v.as_str().unwrap()).unwrap();
    let key = crypto::secretbox_open(
        &s(&col["encryptedKey"]),
        &s(&col["keyDecryptionNonce"]),
        &acc.master_key,
    )
    .unwrap();
    let path_plain = crypto::secretbox_open(
        &s(&col["attributes"]["encryptedPath"]),
        &s(&col["attributes"]["pathDecryptionNonce"]),
        &key,
    )
    .unwrap();
    assert_eq!(path_plain, b"Camera");

    // A second device finds the same album after a sync, and the photo in it.
    let b = signed_in(&srv.url).await;
    b.core.sync(None).await.unwrap();
    assert_eq!(b.core.backup_album("Camera".into()).await.unwrap(), id);
    let cols = b.core.collections();
    let cam = cols.iter().find(|c| c.id == id).expect("Camera album");
    assert_eq!(
        (cam.name.as_str(), cam.kind.as_str(), cam.count),
        ("Camera", "folder", 1)
    );
    assert_eq!(b.core.collection_page(id, 0, 10).len(), 1);
}
