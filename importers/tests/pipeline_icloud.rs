//! Whole-pipeline test: a synthetic iCloud export is imported with
//! [`run::run`] into a mock Ente server, then everything the server received
//! is decrypted and checked. A second run must skip everything via the
//! journal; a run with a fresh journal must find everything as duplicates.

mod common;
mod support;

use common::{ExifSpec, jpeg};
use photovault_import::ente::{Credentials, PeopleMode, RetryPolicy, UploadOptions, Uploader};
use photovault_import::icloud::{ICloudOptions, ICloudSource};
use photovault_import::run::{Summary, run};
use photovault_import::state::Journal;
use std::path::{Path, PathBuf};
use std::time::Duration;
use support::{Account, AccountOptions, MockServer};

const EMAIL: &str = "bob@example.org";
const PASSWORD: &str = "pipeline password";
const HEADER: &str =
    "imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate\n";

fn write_export(root: &Path) -> PathBuf {
    let part = root.join("iCloud Photos Part 1 of 1");
    let photos = part.join("Photos");
    std::fs::create_dir_all(&photos).unwrap();
    std::fs::create_dir_all(part.join("Albums")).unwrap();
    let a = jpeg(
        64,
        48,
        &ExifSpec {
            gps: Some((48.8584, 2.2945)),
            ..Default::default()
        },
    );
    std::fs::write(photos.join("IMG_0100.JPG"), a).unwrap();
    std::fs::write(
        photos.join("IMG_0101.JPG"),
        jpeg(32, 32, &ExifSpec::default()),
    )
    .unwrap();
    std::fs::write(
        photos.join("Photo Details.csv"),
        format!(
            "{HEADER}IMG_0100.JPG,CKA,yes,no,no,\"Monday May 1,2023 9:30 AM GMT\",0,\"Monday May 1,2023 9:31 AM GMT\"\n\
             IMG_0101.JPG,CKB,no,no,no,\"Tuesday May 2,2023 10:00 AM GMT\",0,\"Tuesday May 2,2023 10:01 AM GMT\"\n"
        ),
    )
    .unwrap();
    std::fs::write(part.join("Albums/Paris.csv"), "imgName\nIMG_0100.JPG\n").unwrap();
    part
}

async fn connect(server: &MockServer) -> Uploader {
    let no_prompt = || -> anyhow::Result<String> { panic!("unexpected prompt") };
    Uploader::connect_with_retry(
        Credentials {
            endpoint: server.url.clone(),
            email: EMAIL.into(),
            password: PASSWORD.into(),
        },
        UploadOptions {
            default_album: "Imported from iCloud".into(),
            people: PeopleMode::Albums,
            people_album_prefix: String::new(),
            dry_run: false,
        },
        &no_prompt,
        &no_prompt,
        RetryPolicy {
            attempts: 3,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        },
    )
    .await
    .expect("connect")
}

async fn import(server: &MockServer, export: &Path, journal: &Path) -> Summary {
    let mut source = ICloudSource::open(&[export.to_path_buf()], ICloudOptions::default()).unwrap();
    let mut uploader = connect(server).await;
    let mut journal = Journal::open(journal, "icloud").unwrap();
    run(&mut source, &mut uploader, &mut journal).await.unwrap()
}

#[tokio::test]
async fn icloud_export_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let export = write_export(dir.path());
    let journal = dir.path().join("journal.jsonl");
    let server = MockServer::start(Account::new(
        EMAIL,
        PASSWORD,
        AccountOptions {
            srp: true,
            ..Default::default()
        },
    ))
    .await;

    let s = import(&server, &export, &journal).await;
    assert_eq!((s.uploaded, s.failed), (2, 0), "{s:?}");

    let files: Vec<_> = server
        .file_ids()
        .into_iter()
        .map(|id| server.decrypt_file(id))
        .collect();
    assert_eq!(files.len(), 2);
    let by_title = |t: &str| {
        files
            .iter()
            .find(|f| f.metadata["title"] == t)
            .unwrap_or_else(|| panic!("{t} missing"))
    };
    let paris = by_title("IMG_0100.JPG");
    assert_eq!(
        std::fs::read(export.join("Photos/IMG_0100.JPG")).unwrap(),
        paris.content
    );
    assert_eq!(paris.metadata["fileType"], 0);
    // CSV date (UTC) in microseconds.
    assert_eq!(paris.metadata["creationTime"], 1_682_933_400_000_000i64);
    assert!((paris.metadata["latitude"].as_f64().unwrap() - 48.8584).abs() < 1e-3);
    assert!(
        paris.collections.contains(&"Paris".to_string()),
        "{:?}",
        paris.collections
    );
    assert!(
        paris.collections.contains(&"Favorites".to_string()),
        "{:?}",
        paris.collections
    );
    // Thumbnail is a real JPEG.
    assert_eq!(&paris.thumbnail[..2], &[0xFF, 0xD8]);

    let plain = by_title("IMG_0101.JPG");
    assert_eq!(plain.collections, vec!["Imported from iCloud".to_string()]);

    // Re-run: the journal skips everything before any extraction or upload.
    let s = import(&server, &export, &journal).await;
    assert_eq!((s.uploaded, s.duplicates, s.skipped), (0, 0, 2), "{s:?}");

    // Fresh journal: content hashes in the library catch the duplicates.
    let s = import(&server, &export, &dir.path().join("other.jsonl")).await;
    assert_eq!((s.uploaded, s.duplicates), (0, 2), "{s:?}");
    assert_eq!(server.file_ids().len(), 2);
}
