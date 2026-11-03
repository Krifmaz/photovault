//! Local media handling at upload time: type detection, Ente's content
//! hash, EXIF (capture time, location, dimensions) and thumbnails.
//!
//! Trimmed from `importers/src/media.rs` (branch `claude/project-thread-cp9x81`):
//! the pure-Rust parts only (no external programs, no Live Photo bundling).
//!
//! - [`hash_file`] is Ente's `metadata.hash` (`chunkHashInit/Update/Final` in
//!   `web/packages/gallery/services/upload/upload-service.ts`).
//! - [`read_embedded_info`] follows `web/packages/gallery/services/exif.ts`
//!   (date precedence, offsets, local-time interpretation, orientation swap
//!   of dimensions).
//! - [`make_thumbnail`] follows `web/packages/gallery/services/upload/thumbnail.ts`
//!   (longest edge 720, JPEG quality 0.7 stepping down to 0.5 while the
//!   result is over 100 KiB).

use crate::crypto::{ENCRYPTION_CHUNK_SIZE, Hasher};
use base64::Engine as _;
use chrono::{FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone};
use std::io::{BufReader, Read};
use std::path::Path;

/// Kind of a local media file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    /// A still image (Ente `fileType` 0).
    Image,
    /// A video (Ente `fileType` 1).
    Video,
}

/// A position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeoPoint {
    /// Latitude in degrees.
    pub latitude: f64,
    /// Longitude in degrees.
    pub longitude: f64,
}

/// Longest edge of a generated thumbnail, matching the Ente apps.
pub const THUMBNAIL_MAX_DIMENSION: u32 = 720;

/// Ente's soft cap on thumbnail size: the JPEG quality is lowered (down to
/// 0.5) while the thumbnail is bigger than this (`maxThumbnailSize`).
pub const THUMBNAIL_MAX_SIZE: usize = 100 * 1024;

/// What EXIF/container metadata could tell us about a file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmbeddedInfo {
    /// Capture time (DateTimeOriginal, with offset if present), µs since epoch.
    pub creation_time_us: Option<i64>,
    /// GPS position.
    pub location: Option<GeoPoint>,
    /// Pixel dimensions.
    pub dimensions: Option<(u32, u32)>,
}

// ---------------------------------------------------------------------------
// Type detection
// ---------------------------------------------------------------------------

/// Image extensions (lower case): everything the Ente apps treat as images,
/// plus common camera RAW formats.
const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "pjpeg", "heic", "heif", "hif", "avif", "png", "apng", "gif",
    "bmp", "dib", "tif", "tiff", "webp", "jxl", "jp2", "j2k", "ico", "psd", "svg",
    // Camera RAW.
    "dng", "raw", "cr2", "cr3", "crw", "nef", "nrw", "arw", "srf", "sr2", "orf", "raf", "rw2",
    "rwl", "pef", "ptx", "srw", "x3f", "3fr", "fff", "erf", "kdc", "dcr", "dcs", "mef", "mos",
    "mrw", "iiq", "gpr",
];

/// Video extensions (lower case).
const VIDEO_EXTENSIONS: &[&str] = &[
    "mov", "qt", "mp4", "m4v", "3gp", "3g2", "3gpp", "avi", "mkv", "webm", "wmv", "asf", "flv",
    "f4v", "mpg", "mpeg", "mpe", "m2v", "mp", "mts", "m2ts", "ts", "ogv", "vob", "mxf", "divx",
    "insv", "lrv",
];

/// Extensions that are never media, so we do not bother sniffing them.
const NON_MEDIA_EXTENSIONS: &[&str] = &[
    "csv", "json", "txt", "xml", "html", "htm", "pdf", "zip", "aae", "xmp", "plist", "db",
    "sqlite", "log", "md", "ini", "thm",
];

/// Image or video, from the file extension (and magic bytes if ambiguous).
/// Returns `None` for files that are not media (e.g. `.csv`, `.json`).
pub fn detect_kind(path: &Path) -> Option<MediaKind> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some(e) if IMAGE_EXTENSIONS.contains(&e) => Some(MediaKind::Image),
        Some(e) if VIDEO_EXTENSIONS.contains(&e) => Some(MediaKind::Video),
        Some(e) if NON_MEDIA_EXTENSIONS.contains(&e) => None,
        _ => {
            let mut head = [0u8; 32];
            let n = std::fs::File::open(path)
                .and_then(|mut f| read_up_to(&mut f, &mut head))
                .ok()?;
            detect_kind_from_magic(&head[..n])
        }
    }
}

/// Image or video from the first bytes of a file (at least 12 are needed
/// for most formats). `None` if the signature is not a known media format.
pub fn detect_kind_from_magic(head: &[u8]) -> Option<MediaKind> {
    let starts = |sig: &[u8]| head.starts_with(sig);
    if starts(&[0xFF, 0xD8, 0xFF])
        || starts(b"\x89PNG\r\n\x1a\n")
        || starts(b"GIF87a")
        || starts(b"GIF89a")
        || starts(b"BM")
        || starts(b"II*\0")
        || starts(b"MM\0*")
        || starts(b"8BPS")
        || starts(&[0xFF, 0x0A])
        || starts(b"\0\0\0\x0cJXL \r\n\x87\n")
    {
        return Some(MediaKind::Image);
    }
    if head.len() >= 12 && starts(b"RIFF") {
        return match &head[8..12] {
            b"WEBP" => Some(MediaKind::Image),
            b"AVI " => Some(MediaKind::Video),
            _ => None,
        };
    }
    if starts(&[0x1A, 0x45, 0xDF, 0xA3]) // Matroska / WebM (EBML)
        || starts(&[0x00, 0x00, 0x01, 0xBA]) // MPEG program stream
        || starts(&[0x00, 0x00, 0x01, 0xB3]) // MPEG video
        || starts(&[0x30, 0x26, 0xB2, 0x75]) // ASF / WMV
        || starts(b"FLV")
    {
        return Some(MediaKind::Video);
    }
    if head.len() >= 12 && &head[4..8] == b"ftyp" {
        let brand = &head[8..12];
        return match brand {
            b"heic" | b"heix" | b"heim" | b"heis" | b"hevc" | b"hevx" | b"hevm" | b"hevs"
            | b"mif1" | b"msf1" | b"avif" | b"avis" | b"crx " | b"jxl " => Some(MediaKind::Image),
            _ => Some(MediaKind::Video),
        };
    }
    if head.len() >= 8 && matches!(&head[4..8], b"moov" | b"mdat" | b"wide" | b"free") {
        return Some(MediaKind::Video);
    }
    None
}

/// Read as many bytes as fit in `buf` (fewer only at end of file).
fn read_up_to(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------------------
// Hash
// ---------------------------------------------------------------------------

/// Ente's content hash of a file: base64 of the 64-byte BLAKE2b digest,
/// computed in [`crate::crypto::ENCRYPTION_CHUNK_SIZE`] chunks.
///
/// For a Live Photo Ente stores `"<image hash>:<video hash>"`; callers join
/// the two hashes themselves.
pub fn hash_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Hasher::new();
    let mut buf = vec![0u8; ENCRYPTION_CHUNK_SIZE];
    loop {
        let n = read_up_to(&mut f, &mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        if n < buf.len() {
            break;
        }
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(hasher.finalize()))
}

// ---------------------------------------------------------------------------
// Embedded metadata
// ---------------------------------------------------------------------------

/// Read EXIF (JPEG, HEIC, PNG, TIFF). Videos (no container parser here) and
/// files without EXIF give `Default`. Never fails.
pub fn read_embedded_info(path: &Path) -> EmbeddedInfo {
    match detect_kind(path) {
        Some(MediaKind::Video) => EmbeddedInfo::default(),
        _ => {
            let mut info = std::fs::File::open(path)
                .ok()
                .and_then(|f| {
                    exif::Reader::new()
                        .read_from_container(&mut BufReader::new(f))
                        .ok()
                })
                .map(|e| info_from_exif(&e))
                .unwrap_or_default();
            if info.dimensions.is_none() {
                info.dimensions = image_header_dimensions(path);
            }
            info
        }
    }
}

/// Dimensions from the image header via the `image` crate (no full decode).
fn image_header_dimensions(path: &Path) -> Option<(u32, u32)> {
    let dims = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    let orientation = read_exif_orientation(path).unwrap_or(1);
    Some(if (5..=8).contains(&orientation) {
        (dims.1, dims.0)
    } else {
        dims
    })
}

/// The EXIF orientation tag (1–8) of a file, if present.
fn read_exif_orientation(path: &Path) -> Option<u32> {
    let f = std::fs::File::open(path).ok()?;
    let e = exif::Reader::new()
        .read_from_container(&mut BufReader::new(f))
        .ok()?;
    e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?
        .value
        .get_uint(0)
}

/// First ASCII value of a primary-IFD tag.
fn exif_ascii(e: &exif::Exif, tag: exif::Tag) -> Option<String> {
    match &e.get_field(tag, exif::In::PRIMARY)?.value {
        exif::Value::Ascii(v) => v.first().map(|b| {
            String::from_utf8_lossy(b)
                .trim_end_matches('\0')
                .to_string()
        }),
        exif::Value::Undefined(b, _) => Some(
            String::from_utf8_lossy(b)
                .trim_end_matches('\0')
                .to_string(),
        ),
        _ => None,
    }
}

/// Extract the parts of [`EmbeddedInfo`] Ente uses from parsed EXIF.
fn info_from_exif(e: &exif::Exif) -> EmbeddedInfo {
    use exif::Tag;
    let date = |dt: Tag, sub: Tag, off: Tag| {
        let d = exif_ascii(e, dt)?;
        parse_exif_date(
            &d,
            exif_ascii(e, sub).as_deref(),
            exif_ascii(e, off).as_deref(),
        )
    };
    // Ente: DateTimeOriginal ?? DateTimeDigitized ?? DateTime.
    let creation_time_us = date(
        Tag::DateTimeOriginal,
        Tag::SubSecTimeOriginal,
        Tag::OffsetTimeOriginal,
    )
    .or_else(|| {
        date(
            Tag::DateTimeDigitized,
            Tag::SubSecTimeDigitized,
            Tag::OffsetTimeDigitized,
        )
    })
    .or_else(|| date(Tag::DateTime, Tag::SubSecTime, Tag::OffsetTime));

    let location = exif_gps(e);

    let uint = |t: Tag| {
        e.get_field(t, exif::In::PRIMARY)
            .and_then(|f| f.value.get_uint(0))
            .filter(|&v| v > 0)
    };
    let orientation = uint(Tag::Orientation).unwrap_or(1);
    let dims = match (uint(Tag::PixelXDimension), uint(Tag::PixelYDimension)) {
        (Some(w), Some(h)) => Some((w, h)),
        _ => match (uint(Tag::ImageWidth), uint(Tag::ImageLength)) {
            (Some(w), Some(h)) => Some((w, h)),
            _ => None,
        },
    };
    let dimensions = dims.map(|(w, h)| {
        if (5..=8).contains(&orientation) {
            (h, w)
        } else {
            (w, h)
        }
    });
    EmbeddedInfo {
        creation_time_us,
        location,
        dimensions,
    }
}

/// GPS position from EXIF, honouring the N/S and E/W reference tags.
fn exif_gps(e: &exif::Exif) -> Option<GeoPoint> {
    use exif::Tag;
    let coord = |t: Tag, r: Tag, neg: u8| -> Option<f64> {
        let v = match &e.get_field(t, exif::In::PRIMARY)?.value {
            exif::Value::Rational(v) if !v.is_empty() => {
                let part = |i: usize| v.get(i).map(|x| x.to_f64()).unwrap_or(0.0);
                part(0) + part(1) / 60.0 + part(2) / 3600.0
            }
            _ => return None,
        };
        let negative = exif_ascii(e, r)
            .and_then(|s| s.bytes().next())
            .is_some_and(|c| c.eq_ignore_ascii_case(&neg));
        let v = if negative { -v } else { v };
        v.is_finite().then_some(v)
    };
    let latitude = coord(Tag::GPSLatitude, Tag::GPSLatitudeRef, b'S')?;
    let longitude = coord(Tag::GPSLongitude, Tag::GPSLongitudeRef, b'W')?;
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }
    Some(GeoPoint {
        latitude,
        longitude,
    })
}

/// Parse an EXIF date (`YYYY:MM:DD HH:MM:SS`) with optional sub-seconds and
/// offset (`+HH:MM`) into µs since the epoch.
///
/// Like Ente (`parseExifDate`), a date without an offset is interpreted in
/// the local time zone of the machine running the import. Ente's sentinel
/// dates (the epoch and `4501-01-01T00:00:00`) are rejected.
pub fn parse_exif_date(date: &str, subsec: Option<&str>, offset: Option<&str>) -> Option<i64> {
    let date = date.trim();
    let naive = NaiveDateTime::parse_from_str(date, "%Y:%m:%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(date, "%Y-%m-%d %H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(date, "%Y:%m:%d %H:%M"))
        .ok()?;
    if naive == sentinel_4501() {
        return None;
    }
    let mut us = 0i64;
    if let Some(s) = subsec.map(str::trim).filter(|s| !s.is_empty()) {
        let digits: String = s
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .take(6)
            .collect();
        if !digits.is_empty() {
            us = format!("{digits:0<6}").parse().unwrap_or(0);
        }
    }
    let base = match offset.and_then(parse_offset) {
        Some(off) => off.from_local_datetime(&naive).single()?.timestamp_micros(),
        None => local_timestamp_us(&naive)?,
    };
    let ts = base + us;
    (ts != 0).then_some(ts)
}

fn sentinel_4501() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(4501, 1, 1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .unwrap_or_default()
}

/// µs since the epoch of a wall-clock time in the local time zone (earliest
/// instant if ambiguous, as JavaScript's `Date` does).
fn local_timestamp_us(naive: &NaiveDateTime) -> Option<i64> {
    Local
        .from_local_datetime(naive)
        .earliest()
        .or_else(|| {
            Local
                .from_local_datetime(&(*naive + chrono::Duration::hours(1)))
                .earliest()
        })
        .map(|d| d.timestamp_micros())
}

/// Parse `Z`, `+HH:MM`, `+HHMM` or `+HH` into an offset.
fn parse_offset(s: &str) -> Option<FixedOffset> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("z") {
        return FixedOffset::east_opt(0);
    }
    let (sign, rest) = match s.as_bytes().first()? {
        b'+' => (1, &s[1..]),
        b'-' => (-1, &s[1..]),
        _ => return None,
    };
    let digits: String = rest.chars().filter(|c| *c != ':').collect();
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m) = match digits.len() {
        2 => (digits.parse::<i32>().ok()?, 0),
        4 => (
            digits[..2].parse::<i32>().ok()?,
            digits[2..].parse::<i32>().ok()?,
        ),
        _ => return None,
    };
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

// ---------------------------------------------------------------------------
// Thumbnails
// ---------------------------------------------------------------------------

/// Scale `(w, h)` to fit in `max` on the longest edge, never up-scaling
/// (Ente's `scaledImageDimensions`).
pub fn scaled_dimensions(width: u32, height: u32, max: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    let f = (max as f64 / width as f64)
        .min(max as f64 / height as f64)
        .min(1.0);
    let w = (width as f64 * f).round() as u32;
    let h = (height as f64 * f).round() as u32;
    (w.max(1), h.max(1))
}

/// Decode an image with the `image` crate and apply its EXIF orientation.
fn decode_with_image_crate(path: &Path) -> Option<image::DynamicImage> {
    use image::ImageDecoder as _;
    let reader = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    img.apply_orientation(orientation);
    Some(img)
}

/// The small JPEG preview many cameras (and RAW files) embed in EXIF IFD1.
fn exif_embedded_thumbnail(path: &Path) -> Option<Vec<u8>> {
    let f = std::fs::File::open(path).ok()?;
    let e = exif::Reader::new()
        .read_from_container(&mut BufReader::new(f))
        .ok()?;
    let off = e
        .get_field(exif::Tag::JPEGInterchangeFormat, exif::In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let len = e
        .get_field(exif::Tag::JPEGInterchangeFormatLength, exif::In::THUMBNAIL)?
        .value
        .get_uint(0)? as usize;
    let bytes = e.buf().get(off..off.checked_add(len)?)?;
    let img = image::load_from_memory(bytes).ok()?;
    encode_thumbnail(&img)
}

/// Resize to Ente's thumbnail size and encode as JPEG with Ente's quality
/// loop: start at 0.7 and step down by 0.1 (not below 0.5) while the result
/// is over [`THUMBNAIL_MAX_SIZE`] and each step still saves at least 10 %.
pub fn encode_thumbnail(img: &image::DynamicImage) -> Option<Vec<u8>> {
    let (w, h) = scaled_dimensions(img.width(), img.height(), THUMBNAIL_MAX_DIMENSION);
    if w == 0 || h == 0 {
        return None;
    }
    let resized = if (w, h) == (img.width(), img.height()) {
        img.clone()
    } else {
        img.resize_exact(w, h, image::imageops::FilterType::Triangle)
    };
    // JPEG has no alpha: composite onto black, as a canvas does.
    let rgba = resized.to_rgba8();
    let mut rgb = image::RgbImage::new(w, h);
    for (dst, src) in rgb.pixels_mut().zip(rgba.pixels()) {
        let a = src[3] as u16;
        *dst = image::Rgb([
            (src[0] as u16 * a / 255) as u8,
            (src[1] as u16 * a / 255) as u8,
            (src[2] as u16 * a / 255) as u8,
        ]);
    }
    let encode = |q: u8| -> Option<Vec<u8>> {
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q)
            .encode_image(&rgb)
            .ok()?;
        Some(out)
    };
    let mut quality: i32 = 70;
    let mut prev_size = usize::MAX;
    let mut blob = encode(quality as u8)?;
    quality -= 10;
    while quality >= 50
        && blob.len() > THUMBNAIL_MAX_SIZE
        && (prev_size.saturating_sub(blob.len()) as f64 * 100.0 / prev_size as f64) >= 10.0
    {
        prev_size = blob.len();
        blob = encode(quality as u8)?;
        quality -= 10;
    }
    Some(blob)
}

/// A small black JPEG (300×150, like Ente's `fallbackThumbnail`).
pub fn placeholder_thumbnail() -> Vec<u8> {
    let img = image::RgbImage::new(300, 150);
    let mut out = Vec::new();
    let _ = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 70).encode_image(&img);
    out
}

/// Make a JPEG thumbnail (longest edge [`THUMBNAIL_MAX_DIMENSION`]) of a
/// local file: decode with the `image` crate (EXIF orientation applied),
/// else the EXIF-embedded preview, else a placeholder, so an upload never
/// fails just for a thumbnail. Videos get the placeholder here (the core has
/// no video decoder). Blocking: call from a blocking thread.
pub fn make_thumbnail(path: &Path, kind: MediaKind) -> Vec<u8> {
    let made = match kind {
        MediaKind::Video => None,
        MediaKind::Image => decode_with_image_crate(path)
            .and_then(|i| encode_thumbnail(&i))
            .or_else(|| exif_embedded_thumbnail(path)),
    };
    made.unwrap_or_else(placeholder_thumbnail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_from_names_and_magic() {
        assert_eq!(
            detect_kind_from_magic(&[0xFF, 0xD8, 0xFF, 0xE0]),
            Some(MediaKind::Image)
        );
        assert_eq!(
            detect_kind_from_magic(b"\0\0\0\x18ftypmp42\0\0\0\0"),
            Some(MediaKind::Video)
        );
        assert_eq!(detect_kind_from_magic(b"hello world!"), None);
        assert_eq!(scaled_dimensions(4000, 3000, 720), (720, 540));
        assert_eq!(scaled_dimensions(100, 50, 720), (100, 50));
    }

    #[test]
    fn exif_dates() {
        assert_eq!(
            parse_exif_date("2023:05:01 09:30:00", None, Some("+00:00")),
            Some(1_682_933_400_000_000)
        );
        assert_eq!(
            parse_exif_date("2023:05:01 11:30:00", Some("5"), Some("+02:00")),
            Some(1_682_933_400_500_000)
        );
        assert_eq!(
            parse_exif_date("4501:01:01 00:00:00", None, Some("Z")),
            None
        );
        assert_eq!(parse_exif_date("garbage", None, None), None);
    }

    #[test]
    fn thumbnails_are_jpeg_and_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.png");
        image::RgbImage::from_fn(1600, 900, |x, y| {
            image::Rgb([(x % 256) as u8, (y % 256) as u8, 80])
        })
        .save(&p)
        .unwrap();
        let t = make_thumbnail(&p, MediaKind::Image);
        let img = image::load_from_memory(&t).unwrap();
        assert_eq!((img.width(), img.height()), (720, 405));
        assert_eq!(&t[..3], &[0xFF, 0xD8, 0xFF]);
        let junk = dir.path().join("junk.jpg");
        std::fs::write(&junk, b"not an image").unwrap();
        assert_eq!(
            make_thumbnail(&junk, MediaKind::Image),
            placeholder_thumbnail()
        );
    }
}
