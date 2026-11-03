//! End-to-end tests of the iCloud source against synthetic exports, laid
//! out as Apple ships them (`iCloud Photos Part N of M/…`), both as
//! extracted folders and as split zips read in place.

mod common;

use chrono::TimeZone as _;
use common::{ExifSpec, jpeg};
use photovault_import::icloud::{ICloudOptions, ICloudSource};
use photovault_import::item::{ImportItem, MediaKind, Source};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const DETAILS_HEADER: &str =
    "imgName,fileChecksum,favorite,hidden,deleted,originalCreationDate,viewCount,importDate\n";

/// (path inside the export, bytes), per part.
fn export_files() -> Vec<Vec<(String, Vec<u8>)>> {
    let p1 = "iCloud Photos Part 1 of 2";
    let p2 = "iCloud Photos Part 2 of 2";
    let plain = || jpeg(40, 30, &ExifSpec::default());
    let part1 = vec![
        (
            format!("{p1}/Photos/IMG_0001.JPG"),
            jpeg(
                40,
                30,
                &ExifSpec {
                    date_time_original: Some("2022:06:04 14:11:30"),
                    offset_time_original: Some("+02:00"),
                    ..Default::default()
                },
            ),
        ),
        (format!("{p1}/Photos/IMG_E0001.JPG"), plain()),
        (
            format!("{p1}/Photos/IMG_0002.JPG"),
            jpeg(
                40,
                30,
                &ExifSpec {
                    gps: Some((-33.8568, 151.2153)),
                    description: Some("Opera House"),
                    ..Default::default()
                },
            ),
        ),
        (format!("{p1}/Photos/IMG_0003.JPG"), plain()),
        (format!("{p1}/Photos/IMG_0005.JPG"), plain()),
        (format!("{p1}/Photos/IMG_0006.JPG"), plain()),
        (format!("{p1}/Photos/IMG_0006-1.JPG"), plain()),
        (
            format!("{p1}/Photos/Photo Details.csv"),
            format!(
                "{DETAILS_HEADER}\
                IMG_0001.JPG,CK1,no,no,no,\"Saturday June 4,2022 12:11 PM GMT\",3,\"Saturday June 4,2022 12:12 PM GMT\"\n\
                IMG_E0001.JPG,CK1E,no,no,no,\"Saturday June 4,2022 12:11 PM GMT\",0,\"Saturday June 4,2022 12:12 PM GMT\"\n\
                IMG_0002.JPG,CK2,yes,yes,no,\"Thursday August 2,2018 11:19 AM GMT\",1,\"Thursday August 2,2018 11:19 AM GMT\"\n\
                IMG_0003.JPG,CK3,no,no,yes,\"Thursday August 2,2018 11:20 AM GMT\",0,\"Thursday August 2,2018 11:20 AM GMT\"\n\
                IMG_0005.JPG,CK5,no,no,no,\"Sunday May 14,2023 3:36 AM GMT\",0,\"Sunday May 14,2023 3:37 AM GMT\"\n\
                IMG_0006.JPG,CK6A,no,no,no,\"Monday January 1,2024 9:00 AM GMT\",0,\"Monday January 1,2024 9:00 AM GMT\"\n\
                IMG_0006.JPG,CK6B,no,no,no,\"Tuesday January 2,2024 9:00 PM GMT\",0,\"Tuesday January 2,2024 9:00 PM GMT\"\n"
            )
            .into_bytes(),
        ),
        (
            format!("{p1}/Albums/Trip.csv"),
            b"imgName\nIMG_0001.JPG\nIMG_0005.MOV\n".to_vec(),
        ),
        (
            format!("{p1}/Memories/Summer.csv"),
            b"imgName\nIMG_0002.JPG\n".to_vec(),
        ),
    ];
    let part2 = vec![
        (
            format!("{p2}/Photos/IMG_0005.MOV"),
            b"\0\0\0\x14ftypqt  \0\0\0\0qt  fake movie".to_vec(),
        ),
        (format!("{p2}/Photos/IMG_0010.JPG"), plain()),
        (
            format!("{p2}/Photos/Photo Details-1.csv"),
            format!(
                "{DETAILS_HEADER}IMG_0005.MOV,CK5V,yes,no,no,\"Sunday May 14,2023 3:36 AM GMT\",0,\"Sunday May 14,2023 3:37 AM GMT\"\n"
            )
            .into_bytes(),
        ),
        (format!("{p2}/Recently Deleted/IMG_0099.JPG"), plain()),
        (
            format!("{p2}/Shared Albums/Family/AlbumInfo.json"),
            br#"{"albumName":"Family","creationDate":"Monday August 17,2015 12:52 PM GMT",
               "allowContributions":true,"isPublic":false,"owner":"me","participants":[],
               "photos":[{"name":"IMG_0100_a_very_long_original_file_name.JPG",
                          "dateCreated":"Friday January 8,2021 1:16 AM GMT",
                          "contributor":"me","comments":[]}]}"#
                .to_vec(),
        ),
        (
            format!("{p2}/Shared Albums/Family/IMG_0100_a_very_long_origi.JPG"),
            plain(),
        ),
    ];
    vec![part1, part2]
}

fn write_dirs(root: &Path) -> Vec<PathBuf> {
    let mut parts = Vec::new();
    for files in export_files() {
        for (name, data) in &files {
            let p = root.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, data).unwrap();
        }
        let part = files[0].0.split('/').next().unwrap().to_string();
        parts.push(root.join(part));
    }
    parts
}

fn write_zips(root: &Path) -> Vec<PathBuf> {
    let mut zips = Vec::new();
    for files in export_files() {
        let part = files[0].0.split('/').next().unwrap().to_string();
        let path = root.join(format!("{part}.zip"));
        let mut z = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let opts = zip::write::SimpleFileOptions::default();
        for (name, data) in &files {
            z.start_file(name.as_str(), opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        zips.push(path);
    }
    zips
}

async fn collect(src: &mut ICloudSource, skip: &[&str]) -> Vec<ImportItem> {
    let skip: Vec<String> = skip.iter().map(|s| s.to_string()).collect();
    let f = move |id: &str| skip.iter().any(|s| s == id);
    let mut out = Vec::new();
    while let Some(item) = src.next(&f).await.unwrap() {
        out.push(item);
    }
    out
}

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
    chrono::Utc
        .with_ymd_and_hms(y, mo, d, h, mi, s)
        .unwrap()
        .timestamp_micros()
}

fn by_id(items: &[ImportItem]) -> BTreeMap<String, &ImportItem> {
    items.iter().map(|i| (i.source_id.clone(), i)).collect()
}

fn check_items(items: &[ImportItem]) {
    let m = by_id(items);
    let ids: Vec<&str> = m.keys().map(String::as_str).collect();
    assert_eq!(
        ids,
        [
            "ck:CK1",
            "ck:CK2",
            "ck:CK5",
            "ck:CK6A",
            "ck:CK6B",
            "path:Photos/IMG_0010.JPG",
            "shared:Shared Albums/Family/IMG_0100_a_very_long_origi.JPG",
        ],
        "deleted (CK3, Recently Deleted) and edited (IMG_E0001) are skipped"
    );

    // EXIF agrees with the CSV within a minute, so its seconds are kept.
    let i1 = m["ck:CK1"];
    assert_eq!(i1.kind, MediaKind::Image);
    assert_eq!(i1.primary.file_name, "IMG_0001.JPG");
    assert_eq!(i1.creation_time_us, Some(utc(2022, 6, 4, 12, 11, 30)));
    assert_eq!(i1.albums, ["Trip"]);
    assert!(!i1.favorite && !i1.hidden);
    assert_eq!(i1.dimensions, Some((40, 30)));

    let i2 = m["ck:CK2"];
    assert!(i2.favorite && i2.hidden, "hidden favourite");
    assert_eq!(i2.creation_time_us, Some(utc(2018, 8, 2, 11, 19, 0)));
    let loc = i2.location.expect("gps");
    assert!((loc.latitude + 33.8568).abs() < 1e-5 && (loc.longitude - 151.2153).abs() < 1e-5);
    assert_eq!(i2.description.as_deref(), Some("Opera House"));
    assert!(i2.albums.is_empty(), "memories are not albums by default");

    // Live Photo across parts; favourite and album come from the video half.
    let i5 = m["ck:CK5"];
    assert_eq!(i5.kind, MediaKind::LivePhoto);
    assert_eq!(i5.primary.file_name, "IMG_0005.JPG");
    let v = i5.live_video.as_ref().expect("video half");
    assert_eq!(v.file_name, "IMG_0005.MOV");
    assert!(std::fs::read(&v.path).unwrap().ends_with(b"fake movie"));
    assert!(i5.favorite);
    assert_eq!(i5.albums, ["Trip"]);
    assert_eq!(i5.creation_time_us, Some(utc(2023, 5, 14, 3, 36, 0)));

    // `IMG_0006-1.JPG` on disk is the second `IMG_0006.JPG` row.
    let a = m["ck:CK6A"];
    let b = m["ck:CK6B"];
    assert_eq!(a.primary.file_name, "IMG_0006.JPG");
    assert_eq!(b.primary.file_name, "IMG_0006.JPG");
    assert!(b.primary.path.to_string_lossy().ends_with("IMG_0006-1.JPG"));
    assert_eq!(b.creation_time_us, Some(utc(2024, 1, 2, 21, 0, 0)));

    // No CSV row: falls back to the file time.
    let i10 = m["path:Photos/IMG_0010.JPG"];
    assert!(i10.creation_time_us.is_some());

    let s = m["shared:Shared Albums/Family/IMG_0100_a_very_long_origi.JPG"];
    assert_eq!(s.albums, ["Family"]);
    assert_eq!(
        s.primary.file_name,
        "IMG_0100_a_very_long_original_file_name.JPG"
    );
    assert_eq!(s.creation_time_us, Some(utc(2021, 1, 8, 1, 16, 0)));

    for i in items {
        assert!(i.primary.path.exists(), "{:?}", i.primary.path);
        assert_eq!(i.live_video.is_some(), i.kind == MediaKind::LivePhoto);
    }
}

#[tokio::test]
async fn extracted_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let parts = write_dirs(tmp.path());
    let mut src = ICloudSource::open(&parts, ICloudOptions::default()).unwrap();
    assert_eq!(src.total().await.unwrap(), Some(7));
    assert_eq!(src.stats().live_photos, 1);
    assert_eq!(src.stats().edited_skipped, 1);
    assert_eq!(src.stats().deleted_skipped, 2);
    let items = collect(&mut src, &[]).await;
    check_items(&items);
    // Files in extracted folders are used in place and never deleted.
    for i in &items {
        src.done(i).await.unwrap();
        assert!(i.primary.path.exists());
    }
}

#[tokio::test]
async fn parent_folder_of_extracted_parts() {
    let tmp = tempfile::tempdir().unwrap();
    write_dirs(tmp.path());
    let mut src =
        ICloudSource::open(&[tmp.path().to_path_buf()], ICloudOptions::default()).unwrap();
    check_items(&collect(&mut src, &[]).await);
}

#[tokio::test]
async fn split_zips_read_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let zips = write_zips(tmp.path());
    let scratch = tempfile::tempdir().unwrap();
    let opts = ICloudOptions {
        temp_dir: Some(scratch.path().to_path_buf()),
        ..Default::default()
    };
    let mut src = ICloudSource::open(&zips, opts).unwrap();
    assert_eq!(src.total().await.unwrap(), Some(7));
    // Nothing is extracted before items are pulled.
    let count_files = || {
        walkdir::WalkDir::new(scratch.path())
            .into_iter()
            .filter(|e| e.as_ref().unwrap().file_type().is_file())
            .count()
    };
    assert_eq!(count_files(), 0);
    let items = collect(&mut src, &[]).await;
    check_items(&items);
    assert_eq!(count_files(), 8, "7 items, one with two files");
    for i in &items {
        src.done(i).await.unwrap();
        assert!(!i.primary.path.exists(), "temp file removed by done()");
    }
    assert_eq!(count_files(), 0);
}

#[tokio::test]
async fn skip_is_honoured_and_ids_are_stable() {
    let tmp = tempfile::tempdir().unwrap();
    let zips = write_zips(tmp.path());
    let mut src = ICloudSource::open(&zips, ICloudOptions::default()).unwrap();
    let items = collect(&mut src, &["ck:CK1", "ck:CK5"]).await;
    let ids: Vec<_> = items.iter().map(|i| i.source_id.as_str()).collect();
    assert_eq!(items.len(), 5);
    assert!(!ids.contains(&"ck:CK1") && !ids.contains(&"ck:CK5"));

    // The same export as folders yields the same ids.
    let tmp2 = tempfile::tempdir().unwrap();
    let parts = write_dirs(tmp2.path());
    let mut src2 = ICloudSource::open(&parts, ICloudOptions::default()).unwrap();
    let ids2: Vec<String> = collect(&mut src2, &[])
        .await
        .into_iter()
        .map(|i| i.source_id)
        .collect();
    for id in ids {
        assert!(ids2.iter().any(|x| x == id));
    }
}

#[tokio::test]
async fn options_change_the_plan() {
    let tmp = tempfile::tempdir().unwrap();
    let parts = write_dirs(tmp.path());
    let opts = ICloudOptions {
        live_photos: false,
        shared_albums: false,
        memories_as_albums: true,
        include_edited: true,
        temp_dir: None,
    };
    let mut src = ICloudSource::open(&parts, opts).unwrap();
    let items = collect(&mut src, &[]).await;
    let m = by_id(&items);
    assert!(m.contains_key("ck:CK1E"), "edited render included");
    assert!(m.contains_key("ck:CK5V"), "video imported on its own");
    assert_eq!(m["ck:CK5"].kind, MediaKind::Image);
    assert_eq!(m["ck:CK5V"].kind, MediaKind::Video);
    assert!(!m.keys().any(|k| k.starts_with("shared:")));
    assert_eq!(m["ck:CK2"].albums, ["Memory: Summer"]);
    assert_eq!(items.len(), 8);
}

#[tokio::test]
async fn real_video_half_gets_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let photos = tmp.path().join("iCloud Photos Part 1 of 1/Photos");
    std::fs::create_dir_all(&photos).unwrap();
    std::fs::write(
        photos.join("IMG_0042.JPG"),
        jpeg(40, 30, &ExifSpec::default()),
    )
    .unwrap();
    if common::make_video(
        &photos.join("IMG_0042.MOV"),
        &["-metadata", "location=+37.3318-122.0312/"],
    )
    .is_none()
    {
        return;
    }
    let mut src =
        ICloudSource::open(&[tmp.path().to_path_buf()], ICloudOptions::default()).unwrap();
    let items = collect(&mut src, &[]).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].kind, MediaKind::LivePhoto);
    // The still has no GPS; the video's QuickTime location is used.
    if common::have("ffprobe") {
        let loc = items[0].location.expect("location from the video half");
        assert!((loc.latitude - 37.3318).abs() < 1e-4);
    }
}

#[test]
fn bad_inputs_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("notes.txt");
    std::fs::write(&f, b"hello").unwrap();
    assert!(ICloudSource::open(&[f], ICloudOptions::default()).is_err());
    let z = tmp.path().join("iCloud Photos Part 1 of 2.zip");
    std::fs::write(&z, b"PK\x03\x04truncated").unwrap();
    let e = ICloudSource::open(&[z], ICloudOptions::default())
        .err()
        .unwrap();
    assert!(format!("{e:#}").contains("not a readable zip"));
    assert!(ICloudSource::open(&[], ICloudOptions::default()).is_err());
}
