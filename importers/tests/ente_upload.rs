//! End-to-end tests of the Ente client against a mock museum: log in, import
//! images, Live Photos, large (multipart) videos, duplicates, favourites,
//! archived, hidden and described items, then decrypt everything the server
//! received and check the plaintext.

mod support;

use photovault_import::ente::{
    Credentials, PeopleMode, RetryPolicy, UploadOptions, UploadOutcome, Uploader,
};
use photovault_import::item::{GeoPoint, ImportItem, MediaFile, MediaKind};
use serde_json::json;
use std::io::Read as _;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use support::{Account, AccountOptions, MockServer};

const EMAIL: &str = "alice@example.org";
const PASSWORD: &str = "correct horse battery staple";

fn opts() -> UploadOptions {
    UploadOptions {
        default_album: "Imported".into(),
        people: PeopleMode::Albums,
        people_album_prefix: String::new(),
        dry_run: false,
    }
}

fn fast_retry() -> RetryPolicy {
    RetryPolicy {
        attempts: 4,
        base_delay: Duration::from_millis(5),
        max_delay: Duration::from_millis(20),
    }
}

fn creds(server: &MockServer, password: &str) -> Credentials {
    Credentials {
        endpoint: server.url.clone(),
        email: EMAIL.into(),
        password: password.into(),
    }
}

fn no_prompt() -> anyhow::Result<String> {
    panic!("unexpected prompt")
}

async fn connect(server: &MockServer, opts: UploadOptions) -> Uploader {
    Uploader::connect_with_retry(
        creds(server, PASSWORD),
        opts,
        &no_prompt,
        &no_prompt,
        fast_retry(),
    )
    .await
    .expect("connect")
}

/// Pseudo-random but deterministic bytes.
fn bytes(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn write(dir: &Path, name: &str, data: &[u8]) -> MediaFile {
    let path = dir.join(name);
    std::fs::write(&path, data).unwrap();
    MediaFile {
        path,
        file_name: name.into(),
    }
}

/// A tiny real JPEG to use as the source thumbnail.
fn tiny_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(8, 6, image::Rgb([200, 120, 40]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

fn image_item(dir: &Path, id: &str, name: &str, data: &[u8]) -> ImportItem {
    let mut item = ImportItem::new(id, MediaKind::Image, write(dir, name, data));
    let thumb = dir.join(format!("{id}-thumb.jpg"));
    std::fs::write(&thumb, tiny_jpeg()).unwrap();
    item.thumbnail = Some(thumb);
    item.creation_time_us = Some(1_600_000_000_000_000);
    item.modification_time_us = Some(1_600_000_100_000_000);
    item.dimensions = Some((4032, 3024));
    item.location = Some(GeoPoint {
        latitude: 48.8584,
        longitude: 2.2945,
    });
    item
}

fn hash(data: &[u8]) -> String {
    let mut h = photovault_import::crypto::Hasher::new();
    for c in data.chunks(photovault_import::crypto::ENCRYPTION_CHUNK_SIZE) {
        h.update(c);
    }
    photovault_import::crypto::b64(&h.finalize())
}

fn zip_entries(data: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(data)).expect("live photo zip");
    (0..z.len())
        .map(|i| {
            let mut f = z.by_index(i).unwrap();
            let mut b = Vec::new();
            f.read_to_end(&mut b).unwrap();
            (f.name().unwrap().to_string(), b)
        })
        .collect()
}

#[tokio::test]
async fn srp_totp_login_and_full_import() {
    let account = Account::new(
        EMAIL,
        PASSWORD,
        AccountOptions {
            srp: true,
            totp: Some("123456".into()),
            ..Default::default()
        },
    );
    let server = MockServer::start(account).await;
    let dir = tempfile::tempdir().unwrap();

    let totp_calls = AtomicUsize::new(0);
    let totp = || {
        totp_calls.fetch_add(1, Ordering::SeqCst);
        Ok("123456".to_string())
    };
    let mut up = Uploader::connect_with_retry(
        creds(&server, PASSWORD),
        opts(),
        &totp,
        &no_prompt,
        fast_retry(),
    )
    .await
    .expect("connect");
    assert_eq!(totp_calls.load(Ordering::SeqCst), 1);
    assert_eq!(up.library_len(), 0);
    assert_eq!(server.count("POST /users/srp/create-session"), 1);
    assert_eq!(server.count("POST /users/two-factor/verify"), 1);

    // 1. A favourite, archived image with a description, an album and people.
    let img_data = bytes(1, 300_000);
    let mut img = image_item(dir.path(), "a", "IMG_0001.JPG", &img_data);
    img.favorite = true;
    img.archived = true;
    img.description = Some("Eiffel tower".into());
    img.albums = vec!["Paris 2020".into()];
    img.people = vec!["Bob".into(), "Carol".into()];
    let UploadOutcome::Uploaded(img_id) = up.import(&img).await.unwrap() else {
        panic!("expected upload")
    };

    // 2. A Live Photo in the same album.
    let still = bytes(2, 200_000);
    let motion = bytes(3, 500_000);
    let mut live = image_item(dir.path(), "b", "IMG_0002.HEIC", &still);
    live.kind = MediaKind::LivePhoto;
    live.live_video = Some(write(dir.path(), "IMG_0002.MOV", &motion));
    live.albums = vec!["Paris 2020".into()];
    let UploadOutcome::Uploaded(live_id) = up.import(&live).await.unwrap() else {
        panic!("expected upload")
    };

    // 3. A video of 4 chunks + 1 byte: 5 chunks, so multipart (one part).
    // 4. A video above one part (5 chunks + more): two parts.
    let small_video = bytes(4, 4 * 4 * 1024 * 1024 + 1);
    let mut v1 = ImportItem::new(
        "c",
        MediaKind::Video,
        write(dir.path(), "clip.mp4", &small_video),
    );
    v1.creation_time_us = Some(1_500_000_000_000_000);
    let UploadOutcome::Uploaded(v1_id) = up.import(&v1).await.unwrap() else {
        panic!("expected upload")
    };
    let big_video = bytes(5, 6 * 4 * 1024 * 1024 + 12345);
    let v2 = ImportItem::new(
        "d",
        MediaKind::Video,
        write(dir.path(), "long.mp4", &big_video),
    );
    let UploadOutcome::Uploaded(v2_id) = up.import(&v2).await.unwrap() else {
        panic!("expected upload")
    };

    // 5. A duplicate of (1) from another source record, in another album.
    let mut dup = image_item(dir.path(), "e", "copy.jpg", &img_data);
    dup.albums = vec!["Best of".into()];
    assert_eq!(
        up.import(&dup).await.unwrap(),
        UploadOutcome::Duplicate(img_id)
    );

    // 6. A hidden item that also claims an album and favourite.
    let mut hidden = image_item(dir.path(), "f", "secret.png", &bytes(6, 1000));
    hidden.hidden = true;
    hidden.favorite = true;
    hidden.albums = vec!["Paris 2020".into()];
    let UploadOutcome::Uploaded(hidden_id) = up.import(&hidden).await.unwrap() else {
        panic!("expected upload")
    };

    // --- Check what the server holds. ---
    let cols = server.collections();
    let by_name = |n: &str| {
        cols.iter()
            .find(|c| c.2 == n)
            .unwrap_or_else(|| panic!("{n}"))
    };
    assert_eq!(by_name("Paris 2020").1, "album");
    assert_eq!(by_name("Bob").1, "album");
    assert_eq!(by_name("Carol").1, "album");
    assert_eq!(by_name("Best of").1, "album");
    assert_eq!(by_name("Imported").1, "album");
    assert_eq!(by_name("Favorites").1, "favorites");
    let h = by_name(".hidden");
    assert_eq!(h.1, "album");
    assert_eq!(h.3, Some(json!({"subType": 1, "visibility": 2})));
    assert_eq!(cols.len(), 7, "{cols:?}");

    let f = server.decrypt_file(img_id);
    assert_eq!(f.content, img_data);
    assert_eq!(&f.thumbnail[..2], &[0xFF, 0xD8]);
    assert_eq!(
        f.collections,
        vec!["Best of", "Bob", "Carol", "Favorites", "Paris 2020"]
    );
    assert_eq!(
        f.metadata,
        json!({
            "fileType": 0,
            "title": "IMG_0001.JPG",
            "creationTime": 1_600_000_000_000_000i64,
            "modificationTime": 1_600_000_100_000_000i64,
            "hash": hash(&img_data),
            "latitude": 48.8584,
            "longitude": 2.2945,
        })
    );
    let (v, count, data) = f.pub_magic.clone().expect("pub magic");
    assert_eq!((v, count), (1, 3));
    assert_eq!(
        data,
        json!({"w": 4032, "h": 3024, "caption": "Eiffel tower"})
    );
    // Archived via PUT /files/magic-metadata: sent version 1, stored as 2.
    let (v, count, data) = f.magic.clone().expect("magic");
    assert_eq!((v, count), (2, 1));
    assert_eq!(data, json!({"visibility": 1}));

    let f = server.decrypt_file(live_id);
    assert_eq!(f.metadata["fileType"], 2);
    assert_eq!(f.metadata["title"], "IMG_0002.HEIC");
    assert_eq!(
        f.metadata["hash"],
        format!("{}:{}", hash(&still), hash(&motion))
    );
    assert_eq!(f.collections, vec!["Paris 2020"]);
    assert!(f.magic.is_none());
    let entries = zip_entries(&f.content);
    assert_eq!(entries.len(), 2);
    let img_entry = entries.iter().find(|e| e.0.starts_with("image")).unwrap();
    let vid_entry = entries.iter().find(|e| e.0.starts_with("video")).unwrap();
    assert_eq!(img_entry.1, still);
    assert_eq!(vid_entry.1, motion);

    let f = server.decrypt_file(v1_id);
    assert_eq!(f.content, small_video);
    assert_eq!(f.encrypted_size, small_video.len() + 5 * 17);
    assert_eq!(f.metadata["fileType"], 1);
    assert_eq!(f.metadata["creationTime"], 1_500_000_000_000_000i64);
    assert_eq!(f.collections, vec!["Imported"]);
    assert!(f.pub_magic.is_none());

    let f = server.decrypt_file(v2_id);
    assert_eq!(f.content, big_video);
    assert_eq!(f.encrypted_size, big_video.len() + 7 * 17);

    let f = server.decrypt_file(hidden_id);
    assert_eq!(f.collections, vec![".hidden"]);

    assert_eq!(server.file_ids().len(), 5);
    // Checksummed endpoints were used, not the legacy ones.
    assert_eq!(server.count("GET /files/upload-urls"), 0);
    assert_eq!(server.count("POST /files/multipart-upload-url"), 2);
}

#[tokio::test]
async fn reconnect_indexes_library_and_dedups() {
    let account = Account::new(
        EMAIL,
        PASSWORD,
        AccountOptions {
            srp: true,
            ..Default::default()
        },
    );
    let server = MockServer::start(account).await;
    server.state().diff_page_size = 1;
    let dir = tempfile::tempdir().unwrap();

    let a = bytes(10, 5000);
    let b = bytes(11, 6000);
    let mut up = connect(&server, opts()).await;
    let UploadOutcome::Uploaded(a_id) = up
        .import(&image_item(dir.path(), "a", "a.jpg", &a))
        .await
        .unwrap()
    else {
        panic!()
    };
    let mut bi = image_item(dir.path(), "b", "b.jpg", &b);
    bi.archived = true;
    let UploadOutcome::Uploaded(b_id) = up.import(&bi).await.unwrap() else {
        panic!()
    };
    let mut live = image_item(dir.path(), "l", "l.jpg", &bytes(12, 100));
    live.kind = MediaKind::LivePhoto;
    live.live_video = Some(write(dir.path(), "l.mov", &bytes(13, 100)));
    let UploadOutcome::Uploaded(live_id) = up.import(&live).await.unwrap() else {
        panic!()
    };
    drop(up);

    // A fresh session pages through the diff (one file per page).
    let mut up = connect(&server, opts()).await;
    assert_eq!(up.library_len(), 3);
    // Three files in one collection, one per page.
    assert_eq!(server.count("GET /collections/v2/diff"), 3);

    // Same content again, now archived, favourite and in a new album.
    let mut again = image_item(dir.path(), "a2", "renamed.jpg", &a);
    again.archived = true;
    again.favorite = true;
    again.albums = vec!["New".into()];
    assert_eq!(
        up.import(&again).await.unwrap(),
        UploadOutcome::Duplicate(a_id)
    );
    let f = server.decrypt_file(a_id);
    assert_eq!(f.collections, vec!["Favorites", "Imported", "New"]);
    assert_eq!(f.magic.unwrap().2, json!({"visibility": 1}));

    // Already archived and in its album: nothing to change.
    let before = server.state().requests.len();
    let mut bi2 = image_item(dir.path(), "b2", "b.jpg", &b);
    bi2.archived = true;
    assert_eq!(
        up.import(&bi2).await.unwrap(),
        UploadOutcome::Duplicate(b_id)
    );
    assert_eq!(server.state().requests.len(), before);

    let mut live2 = live.clone();
    live2.source_id = "l2".into();
    assert_eq!(
        up.import(&live2).await.unwrap(),
        UploadOutcome::Duplicate(live_id)
    );
    assert_eq!(server.file_ids().len(), 3);
}

#[tokio::test]
async fn email_code_login_legacy_urls_and_retries() {
    let account = Account::new(EMAIL, PASSWORD, AccountOptions::default());
    let server = MockServer::start(account).await;
    {
        let mut s = server.state();
        s.legacy_only = true;
        s.fail_next.insert("POST /files".into(), 2);
        s.fail_next.insert("GET /collections/v2".into(), 1);
    }
    let otp_calls = AtomicUsize::new(0);
    let otp = || {
        otp_calls.fetch_add(1, Ordering::SeqCst);
        Ok("424242".to_string())
    };
    let mut up = Uploader::connect_with_retry(
        creds(&server, PASSWORD),
        opts(),
        &no_prompt,
        &otp,
        fast_retry(),
    )
    .await
    .expect("connect");
    assert_eq!(otp_calls.load(Ordering::SeqCst), 1);
    assert_eq!(server.count("POST /users/srp/create-session"), 0);

    let dir = tempfile::tempdir().unwrap();
    let data = bytes(20, 1234);
    let UploadOutcome::Uploaded(id) = up
        .import(&image_item(dir.path(), "x", "x.jpg", &data))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(server.decrypt_file(id).content, data);
    assert_eq!(server.count("POST /files"), 3);
    assert_eq!(server.count("GET /files/upload-urls"), 2);

    let video = bytes(21, 5 * 4 * 1024 * 1024);
    let UploadOutcome::Uploaded(vid) = up
        .import(&ImportItem::new(
            "v",
            MediaKind::Video,
            write(dir.path(), "v.mp4", &video),
        ))
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(server.decrypt_file(vid).content, video);
    assert_eq!(server.count("GET /files/multipart-upload-urls"), 1);
}

#[tokio::test]
async fn email_mfa_uses_email_code_then_password_for_keys() {
    let account = Account::new(
        EMAIL,
        PASSWORD,
        AccountOptions {
            srp: true,
            email_mfa: true,
            ..Default::default()
        },
    );
    let server = MockServer::start(account).await;
    let otp = || Ok("424242".to_string());
    Uploader::connect_with_retry(
        creds(&server, PASSWORD),
        opts(),
        &no_prompt,
        &otp,
        fast_retry(),
    )
    .await
    .expect("connect");
    assert_eq!(server.count("POST /users/verify-email"), 1);
    assert_eq!(server.count("POST /users/srp/verify-session"), 0);
}

#[tokio::test]
async fn wrong_password_and_passkey_are_reported() {
    let account = Account::new(
        EMAIL,
        PASSWORD,
        AccountOptions {
            srp: true,
            ..Default::default()
        },
    );
    let server = MockServer::start(account).await;
    let e = Uploader::connect_with_retry(
        creds(&server, "wrong"),
        opts(),
        &no_prompt,
        &no_prompt,
        fast_retry(),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{e:#}").contains("incorrect email or password"),
        "{e:#}"
    );

    let account = Account::new(
        EMAIL,
        PASSWORD,
        AccountOptions {
            srp: true,
            passkey: true,
            ..Default::default()
        },
    );
    let server = MockServer::start(account).await;
    let e = Uploader::connect_with_retry(
        creds(&server, PASSWORD),
        opts(),
        &no_prompt,
        &no_prompt,
        fast_retry(),
    )
    .await
    .unwrap_err();
    assert!(format!("{e:#}").contains("passkey"), "{e:#}");
}

#[tokio::test]
async fn dry_run_sends_nothing_and_caption_mode() {
    let account = Account::new(
        EMAIL,
        PASSWORD,
        AccountOptions {
            srp: true,
            ..Default::default()
        },
    );
    let server = MockServer::start(account).await;
    let dir = tempfile::tempdir().unwrap();
    let mut dry = opts();
    dry.dry_run = true;
    let mut up = connect(&server, dry).await;
    let mut item = image_item(dir.path(), "a", "a.jpg", &bytes(30, 100));
    item.albums = vec!["X".into()];
    assert_eq!(up.import(&item).await.unwrap(), UploadOutcome::DryRun);
    assert_eq!(server.count("POST /collections"), 0);
    assert_eq!(server.count("POST /files"), 0);
    assert!(server.state().objects.is_empty());

    let mut o = opts();
    o.people = PeopleMode::Caption;
    let mut up = connect(&server, o).await;
    item.people = vec!["Dana".into()];
    item.description = Some("Picnic".into());
    let UploadOutcome::Uploaded(id) = up.import(&item).await.unwrap() else {
        panic!()
    };
    let f = server.decrypt_file(id);
    assert_eq!(f.collections, vec!["X"]);
    assert_eq!(f.pub_magic.unwrap().2["caption"], "Picnic\nPeople: Dana");
}
