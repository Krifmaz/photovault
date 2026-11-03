//! Tests for `photovault_import::media`: type detection, Ente's content hash,
//! embedded metadata, thumbnails and the Live Photo zip.

mod common;

use chrono::TimeZone as _;
use common::{ExifSpec, jpeg, make_video};
use photovault_import::item::{MediaFile, MediaKind};
use photovault_import::media::{self, THUMBNAIL_MAX_DIMENSION};
use std::io::Read as _;
use std::path::Path;

fn write(dir: &Path, name: &str, data: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, data).unwrap();
    p
}

fn mf(path: &Path) -> MediaFile {
    MediaFile {
        path: path.to_path_buf(),
        file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
    }
}

#[test]
fn detect_kind_by_extension() {
    use MediaKind::*;
    let cases = [
        ("IMG_0001.HEIC", Some(Image)),
        ("a.heif", Some(Image)),
        ("a.JPG", Some(Image)),
        ("a.jpeg", Some(Image)),
        ("a.png", Some(Image)),
        ("a.webp", Some(Image)),
        ("a.avif", Some(Image)),
        ("a.gif", Some(Image)),
        ("a.tiff", Some(Image)),
        ("a.DNG", Some(Image)),
        ("a.cr3", Some(Image)),
        ("a.nef", Some(Image)),
        ("a.arw", Some(Image)),
        ("IMG_0001.MOV", Some(Video)),
        ("a.mp4", Some(Video)),
        ("a.m4v", Some(Video)),
        ("a.3gp", Some(Video)),
        ("a.avi", Some(Video)),
        ("a.mkv", Some(Video)),
        ("a.webm", Some(Video)),
        ("Photo Details.csv", None),
        ("AlbumInfo.json", None),
        ("IMG_0001.AAE", None),
    ];
    for (name, want) in cases {
        assert_eq!(media::detect_kind(Path::new(name)), want, "{name}");
    }
}

#[test]
fn detect_kind_by_magic() {
    let dir = tempfile::tempdir().unwrap();
    let j = write(dir.path(), "noext", &jpeg(8, 8, &ExifSpec::default()));
    assert_eq!(media::detect_kind(&j), Some(MediaKind::Image));
    let mut heic = vec![0, 0, 0, 0x18];
    heic.extend_from_slice(b"ftypheic\0\0\0\0mif1heic");
    let h = write(dir.path(), "heic.bin", &heic);
    assert_eq!(media::detect_kind(&h), Some(MediaKind::Image));
    let mut mov = vec![0, 0, 0, 0x14];
    mov.extend_from_slice(b"ftypqt  \0\0\0\0qt  ");
    let m = write(dir.path(), "mov.bin", &mov);
    assert_eq!(media::detect_kind(&m), Some(MediaKind::Video));
    let t = write(dir.path(), "text.bin", b"hello world, not media");
    assert_eq!(media::detect_kind(&t), None);
    assert_eq!(
        media::detect_kind_from_magic(b"RIFF\0\0\0\0WEBPVP8 "),
        Some(MediaKind::Image)
    );
    assert_eq!(
        media::detect_kind_from_magic(b"RIFF\0\0\0\0AVI LIST"),
        Some(MediaKind::Video)
    );
    assert_eq!(
        media::detect_kind_from_magic(&[0x1A, 0x45, 0xDF, 0xA3, 0, 0, 0, 0]),
        Some(MediaKind::Video)
    );
}

/// Expected values computed with Python: `base64(hashlib.blake2b(data).digest())`.
#[test]
fn hash_matches_blake2b_512() {
    let dir = tempfile::tempdir().unwrap();
    let empty = write(dir.path(), "empty", b"");
    assert_eq!(
        media::hash_file(&empty).unwrap(),
        "eGoC90IBWQPGxv2FJVLScpEvR0DhWEdhiobiF/cfVBnSXhAxr+5YUxOJZESTTrBLkDpoWxRIt1XVb3Aa/pvizg=="
    );
    let small = write(dir.path(), "small", b"hello photovault");
    assert_eq!(
        media::hash_file(&small).unwrap(),
        "PbjqgBDoHxVwXRrSyzB7t7PIpE2MJ+ShK8HLI9yDLeBpBZMSIFEZS5D1ZkC8wYl3NUzggmcnMtbjPHLudP09mw=="
    );
    // Larger than one 4 MiB chunk.
    let data: Vec<u8> = (0..5 * 1024 * 1024 + 123u64)
        .map(|i| ((i * 31 + 7) % 251) as u8)
        .collect();
    let big = write(dir.path(), "big", &data);
    assert_eq!(
        media::hash_file(&big).unwrap(),
        "5iV4IHzgsY76tl1DKil23hKQgSMpiqW9fYbefwNTQHYIEKaQi0SMDcFfFLxyoSb3tiVT92w3f3ZIz1xNsfhEMA=="
    );
}

#[test]
fn exif_with_offset_and_gps() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "a.jpg",
        &jpeg(
            64,
            32,
            &ExifSpec {
                date_time_original: Some("2023:05:14 10:36:07"),
                offset_time_original: Some("+02:00"),
                orientation: Some(6),
                gps: Some((48.8584, -2.2945)),
                description: Some("Eiffel-ish"),
            },
        ),
    );
    let info = media::read_embedded_info(&p);
    let want = chrono::Utc
        .with_ymd_and_hms(2023, 5, 14, 8, 36, 7)
        .unwrap()
        .timestamp_micros();
    assert_eq!(info.creation_time_us, Some(want));
    let loc = info.location.unwrap();
    assert!((loc.latitude - 48.8584).abs() < 1e-5, "{loc:?}");
    assert!((loc.longitude + 2.2945).abs() < 1e-5, "{loc:?}");
    // Orientation 6 swaps the displayed dimensions, as in Ente.
    assert_eq!(info.dimensions, Some((32, 64)));
    assert_eq!(media::read_description(&p).as_deref(), Some("Eiffel-ish"));
}

#[test]
fn exif_without_offset_is_local_time() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "a.jpg",
        &jpeg(
            16,
            16,
            &ExifSpec {
                date_time_original: Some("2020:01:02 03:04:05"),
                ..Default::default()
            },
        ),
    );
    let want = chrono::Local
        .with_ymd_and_hms(2020, 1, 2, 3, 4, 5)
        .earliest()
        .unwrap()
        .timestamp_micros();
    assert_eq!(media::read_embedded_info(&p).creation_time_us, Some(want));
    assert_eq!(media::read_embedded_info(&p).dimensions, Some((16, 16)));
}

#[test]
fn metadata_date_parsing() {
    let utc = |y, mo, d, h, mi, s| {
        chrono::Utc
            .with_ymd_and_hms(y, mo, d, h, mi, s)
            .unwrap()
            .timestamp_micros()
    };
    assert_eq!(
        media::parse_metadata_date("2023-05-14T10:36:00+0200"),
        Some(utc(2023, 5, 14, 8, 36, 0))
    );
    assert_eq!(
        media::parse_metadata_date("2023-05-14T08:36:00.000000Z"),
        Some(utc(2023, 5, 14, 8, 36, 0))
    );
    assert_eq!(
        media::parse_metadata_date("1970-01-01T00:00:00.000000Z"),
        None
    );
    assert_eq!(media::parse_metadata_date("1904-01-01T00:00:00Z"), None);
    assert_eq!(
        media::parse_exif_date("2023:05:14 10:36:07", Some("25"), Some("-05:00")),
        Some(utc(2023, 5, 14, 15, 36, 7) + 250_000)
    );
    assert_eq!(
        media::parse_exif_date("4501:01:01 00:00:00", None, Some("Z")),
        None
    );
    let loc = media::parse_iso6709("+37.3318-122.0312+012.345/").unwrap();
    assert_eq!((loc.latitude, loc.longitude), (37.3318, -122.0312));
    assert!(media::parse_iso6709("+00.0000+000.0000/").is_none());
}

#[test]
fn ffprobe_json_interpretation() {
    let v: serde_json::Value = serde_json::from_str(
        r#"{"streams":[{"codec_type":"video","width":1920,"height":1080,
            "side_data_list":[{"rotation":-90}]}],
            "format":{"tags":{"creation_time":"2022-06-04T12:11:00.000000Z",
            "com.apple.quicktime.creationdate":"2022-06-04T14:11:00+0200",
            "com.apple.quicktime.location.ISO6709":"+52.5200+013.4050+034.000/"}}}"#,
    )
    .unwrap();
    let info = media::info_from_ffprobe(&v);
    assert_eq!(
        info.creation_time_us,
        Some(
            chrono::Utc
                .with_ymd_and_hms(2022, 6, 4, 12, 11, 0)
                .unwrap()
                .timestamp_micros()
        )
    );
    assert_eq!(info.dimensions, Some((1080, 1920)));
    assert_eq!(info.location.unwrap().latitude, 52.52);
}

#[test]
fn video_metadata_via_ffprobe() {
    if !common::have("ffprobe") {
        eprintln!("ffprobe not installed; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("clip.mov");
    if make_video(
        &p,
        &[
            "-metadata",
            "creation_time=2021-03-04T05:06:07Z",
            "-metadata",
            "location=+37.3318-122.0312/",
        ],
    )
    .is_none()
    {
        return;
    }
    let info = media::read_embedded_info(&p);
    assert_eq!(
        info.creation_time_us,
        Some(
            chrono::Utc
                .with_ymd_and_hms(2021, 3, 4, 5, 6, 7)
                .unwrap()
                .timestamp_micros()
        )
    );
    assert_eq!(info.dimensions, Some((320, 240)));
    let loc = info.location.expect("location from ffprobe");
    assert!((loc.latitude - 37.3318).abs() < 1e-4);
}

fn decode(jpeg: &[u8]) -> image::DynamicImage {
    assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF], "thumbnail is a JPEG");
    image::load_from_memory(jpeg).unwrap()
}

#[tokio::test]
async fn thumbnail_is_resized_and_oriented() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(
        dir.path(),
        "big.jpg",
        &jpeg(
            2000,
            1000,
            &ExifSpec {
                orientation: Some(6),
                ..Default::default()
            },
        ),
    );
    let t = media::make_thumbnail(&mf(&p), MediaKind::Image, None).await;
    let img = decode(&t);
    // 2000×1000 rotated 90° → 1000×2000 → longest edge 720.
    assert_eq!((img.width(), img.height()), (360, THUMBNAIL_MAX_DIMENSION));
    // After rotating 90° clockwise, the red left half is on top.
    let px = img.to_rgb8();
    let top = px.get_pixel(180, 50);
    let bottom = px.get_pixel(180, 670);
    assert!(top[0] > 150 && top[2] < 100, "top should be red: {top:?}");
    assert!(
        bottom[2] > 150 && bottom[0] < 100,
        "bottom should be blue: {bottom:?}"
    );
    assert!(t.len() <= media::THUMBNAIL_MAX_SIZE);
}

#[tokio::test]
async fn small_images_are_not_upscaled() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "s.jpg", &jpeg(100, 50, &ExifSpec::default()));
    let img = decode(&media::make_thumbnail(&mf(&p), MediaKind::Image, None).await);
    assert_eq!((img.width(), img.height()), (100, 50));
    assert_eq!(media::scaled_dimensions(4032, 3024, 720), (720, 540));
    assert_eq!(media::scaled_dimensions(10, 0, 720), (0, 0));
}

#[tokio::test]
async fn source_thumbnail_is_preferred() {
    let dir = tempfile::tempdir().unwrap();
    let garbage = write(dir.path(), "x.heic", b"not really an image");
    let src = write(
        dir.path(),
        "preview.jpg",
        &jpeg(1440, 1080, &ExifSpec::default()),
    );
    let img = decode(&media::make_thumbnail(&mf(&garbage), MediaKind::Image, Some(&src)).await);
    assert_eq!((img.width(), img.height()), (720, 540));
}

#[tokio::test]
async fn undecodable_file_gets_placeholder() {
    let dir = tempfile::tempdir().unwrap();
    let garbage = write(dir.path(), "x.cr3", b"\0\0\0\0garbage garbage");
    let img = decode(&media::make_thumbnail(&mf(&garbage), MediaKind::Image, None).await);
    assert_eq!((img.width(), img.height()), (300, 150));
}

#[tokio::test]
async fn video_thumbnail_with_ffmpeg() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("clip.mp4");
    if make_video(&p, &[]).is_none() {
        return;
    }
    let img = decode(&media::make_thumbnail(&mf(&p), MediaKind::Video, None).await);
    assert_eq!((img.width(), img.height()), (320, 240));
}

#[test]
fn live_photo_bundle_layout() {
    let dir = tempfile::tempdir().unwrap();
    let still = write(dir.path(), "IMG_0001.HEIC", b"still bytes");
    let video = write(dir.path(), "IMG_0001.MOV", b"video bytes, a bit longer");
    let bundle = media::live_photo_bundle(&mf(&still), &mf(&video)).unwrap();
    let mut z = zip::ZipArchive::new(std::fs::File::open(bundle.path()).unwrap()).unwrap();
    assert_eq!(z.len(), 2);
    let names: Vec<String> = (0..2)
        .map(|i| z.by_index(i).unwrap().name().unwrap().to_string())
        .collect();
    assert_eq!(names, ["image.HEIC", "video.MOV"]);
    for (i, want) in [&b"still bytes"[..], &b"video bytes, a bit longer"[..]]
        .iter()
        .enumerate()
    {
        let mut f = z.by_index(i).unwrap();
        assert_eq!(f.compression(), zip::CompressionMethod::Stored);
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        assert_eq!(&buf, want);
    }
    assert_eq!(media::live_photo_entry_name("image", "noext"), "image");
    assert_eq!(
        media::live_photo_entry_name("video", "a.weird-ext"),
        "video"
    );
    assert_eq!(media::live_photo_entry_name("image", ".hidden"), "image");
    let path = bundle.path().to_path_buf();
    drop(bundle);
    assert!(!path.exists(), "temp bundle is removed on drop");
}
