//! Local media handling: type detection, content hashes, thumbnails, EXIF,
//! and Live Photo bundles in Ente's format.
//!
//! Everything here mirrors what the Ente apps do at upload time, so files
//! imported by Photovault are indistinguishable from files uploaded by the
//! official apps:
//!
//! - [`hash_file`] is Ente's `metadata.hash` (`chunkHashInit/Update/Final` in
//!   `web/packages/gallery/services/upload/upload-service.ts`).
//! - [`read_embedded_info`] follows `web/packages/gallery/services/exif.ts`
//!   (date precedence, offsets, local-time interpretation, orientation swap
//!   of dimensions) and the FFmpeg metadata parser in
//!   `web/packages/gallery/services/ffmpeg/index.ts`.
//! - [`make_thumbnail`] follows `web/packages/gallery/services/upload/thumbnail.ts`
//!   (longest edge 720, JPEG quality 0.7 stepping down to 0.5 while the
//!   result is over 100 KiB, video frame at 1 s falling back to 0 s).
//! - [`live_photo_bundle`] follows `encodeLivePhoto` in
//!   `web/packages/media/live-photo.ts` and the mobile app's
//!   `module/live_photo/archive.dart`.
//!
//! External programs (`ffprobe`, `ffmpeg`, `heif-convert`, `magick`) are
//! optional. They are always started with an explicit argument vector, never
//! through a shell, and a missing program only means a lesser fallback.

use crate::crypto::{ENCRYPTION_CHUNK_SIZE, Hasher};
use crate::item::{GeoPoint, MediaFile, MediaKind};
use base64::Engine as _;
use chrono::{FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone};
use std::io::{BufReader, Read, Seek, Write};
use std::path::Path;
use std::process::{Command, Stdio};

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
pub fn hash_file(path: &Path) -> anyhow::Result<String> {
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

/// Read EXIF (JPEG, HEIC, PNG, TIFF) or, for videos, the container creation
/// time and location via `ffprobe` if available. Never fails: unknown is `Default`.
pub fn read_embedded_info(path: &Path) -> EmbeddedInfo {
    match detect_kind(path) {
        Some(MediaKind::Video) => read_video_info(path).unwrap_or_default(),
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

/// The free-text description embedded in a photo's EXIF (`ImageDescription`),
/// if any. Ente's apps also show this as the file's caption.
pub fn read_description(path: &Path) -> Option<String> {
    let f = std::fs::File::open(path).ok()?;
    let e = exif::Reader::new()
        .read_from_container(&mut BufReader::new(f))
        .ok()?;
    let s = exif_ascii(&e, exif::Tag::ImageDescription)?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
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

/// Parse an ISO 8601 date as found in video metadata (`creation_time`,
/// `com.apple.quicktime.creationdate`): with `Z` or an offset it is exact,
/// otherwise it is local time (like Ente's `parseMetadataDate`).
pub fn parse_metadata_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // Split off a trailing offset: Z, +HH:MM, +HHMM.
    let (body, offset) = split_offset(s);
    let body = body.trim_end();
    let formats = [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M",
        "%Y:%m:%d %H:%M:%S%.f",
    ];
    let naive = formats
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(body, f).ok())
        .or_else(|| {
            NaiveDate::parse_from_str(body, "%Y-%m-%d")
                .ok()
                .and_then(|d| d.and_hms_opt(0, 0, 0))
        })?;
    // The QuickTime epoch (1904-01-01) is what muxers write for "unset".
    if naive == sentinel_4501() || naive.date() <= NaiveDate::from_ymd_opt(1904, 1, 1)? {
        return None;
    }
    let ts = match offset {
        Some(off) => off.from_local_datetime(&naive).single()?.timestamp_micros(),
        None => local_timestamp_us(&naive)?,
    };
    (ts != 0).then_some(ts)
}

fn split_offset(s: &str) -> (&str, Option<FixedOffset>) {
    if let Some(b) = s.strip_suffix(['Z', 'z']) {
        return (b, FixedOffset::east_opt(0));
    }
    // Look for the last + or - after the time part (position > 10).
    if let Some(i) = s.rfind(['+', '-'])
        && i > 10
        && let Some(off) = parse_offset(&s[i..])
    {
        return (&s[..i], Some(off));
    }
    (s, None)
}

/// Parse an ISO 6709 location string such as `+37.3318-122.0312+012.000/`
/// the way Ente does (`parseFFMetadataLocation`): the first two signed
/// numbers are latitude and longitude; zero values are rejected.
pub fn parse_iso6709(s: &str) -> Option<GeoPoint> {
    let b = s.as_bytes();
    let mut nums = Vec::new();
    let mut i = 0;
    while i < b.len() && nums.len() < 2 {
        if b[i] == b'+' || b[i] == b'-' {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if let Ok(v) = s[start..i].parse::<f64>() {
                nums.push(v);
            }
        } else {
            i += 1;
        }
    }
    match nums[..] {
        [lat, lon]
            if lat != 0.0
                && lon != 0.0
                && (-90.0..=90.0).contains(&lat)
                && (-180.0..=180.0).contains(&lon) =>
        {
            Some(GeoPoint {
                latitude: lat,
                longitude: lon,
            })
        }
        _ => None,
    }
}

/// Is an external program available on `PATH`?
fn have_program(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let p = dir.join(name);
            p.is_file() || (cfg!(windows) && p.with_extension("exe").is_file())
        })
    })
}

/// Whether `ffprobe` is on `PATH`.
pub fn have_ffprobe() -> bool {
    have_program("ffprobe")
}

/// Whether `ffmpeg` is on `PATH`.
pub fn have_ffmpeg() -> bool {
    have_program("ffmpeg")
}

/// Video creation time, location and dimensions via `ffprobe`.
fn read_video_info(path: &Path) -> Option<EmbeddedInfo> {
    if !have_ffprobe() {
        return None;
    }
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    Some(info_from_ffprobe(&v))
}

/// Interpret `ffprobe -print_format json -show_format -show_streams` output.
pub fn info_from_ffprobe(v: &serde_json::Value) -> EmbeddedInfo {
    let format_tags = v.pointer("/format/tags");
    let streams = v
        .get("streams")
        .and_then(|s| s.as_array())
        .cloned()
        .unwrap_or_default();
    // Tags are matched case-insensitively; some muxers upper-case them.
    let tag = |name: &str| -> Option<String> {
        let find = |tags: Option<&serde_json::Value>| {
            tags?.as_object()?.iter().find_map(|(k, v)| {
                k.eq_ignore_ascii_case(name)
                    .then(|| v.as_str().map(str::to_string))
                    .flatten()
            })
        };
        find(format_tags).or_else(|| streams.iter().find_map(|s| find(s.get("tags"))))
    };
    // Ente prefers Apple's creationdate (it carries the offset), then creation_time.
    let creation_time_us = tag("com.apple.quicktime.creationdate")
        .and_then(|s| parse_metadata_date(&s))
        .or_else(|| tag("creation_time").and_then(|s| parse_metadata_date(&s)));
    let location = tag("com.apple.quicktime.location.ISO6709")
        .and_then(|s| parse_iso6709(&s))
        .or_else(|| tag("location").and_then(|s| parse_iso6709(&s)));
    let dimensions = streams
        .iter()
        .find(|s| s.get("codec_type").and_then(|c| c.as_str()) == Some("video"))
        .and_then(|s| {
            let w = s.get("width")?.as_u64()? as u32;
            let h = s.get("height")?.as_u64()? as u32;
            let rotation = s
                .get("side_data_list")
                .and_then(|l| l.as_array())
                .and_then(|l| l.iter().find_map(|d| d.get("rotation")?.as_f64()))
                .or_else(|| {
                    s.pointer("/tags/rotate")
                        .and_then(|r| r.as_str())
                        .and_then(|r| r.parse().ok())
                })
                .unwrap_or(0.0);
            let quarter = ((rotation / 90.0).round() as i64).rem_euclid(2) == 1;
            (w > 0 && h > 0).then_some(if quarter { (h, w) } else { (w, h) })
        });
    EmbeddedInfo {
        creation_time_us,
        location,
        dimensions,
    }
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

/// Make a JPEG thumbnail (longest edge [`THUMBNAIL_MAX_DIMENSION`]) for an
/// item. Tries, in order: `source_thumbnail`, decoding `media` with the
/// `image` crate, then `ffmpeg` (HEIC, RAW, video frames). Falls back to a
/// small neutral placeholder so an upload never fails just for a thumbnail.
pub async fn make_thumbnail(
    media: &MediaFile,
    kind: MediaKind,
    source_thumbnail: Option<&Path>,
) -> Vec<u8> {
    let media = media.clone();
    let source = source_thumbnail.map(Path::to_path_buf);
    let task = tokio::task::spawn_blocking(move || {
        make_thumbnail_blocking(&media, kind, source.as_deref())
    });
    match task.await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "thumbnail task failed; using placeholder");
            placeholder_thumbnail()
        }
    }
}

/// Blocking body of [`make_thumbnail`].
pub fn make_thumbnail_blocking(
    media: &MediaFile,
    kind: MediaKind,
    source_thumbnail: Option<&Path>,
) -> Vec<u8> {
    if let Some(src) = source_thumbnail
        && let Some(t) = decode_with_image_crate(src).and_then(|i| encode_thumbnail(&i))
    {
        return t;
    }
    let is_video = kind == MediaKind::Video
        || (kind != MediaKind::Image && detect_kind(&media.path) == Some(MediaKind::Video));
    let result = if is_video {
        video_frame(&media.path).and_then(|i| encode_thumbnail(&i))
    } else {
        decode_with_image_crate(&media.path)
            .or_else(|| decode_with_external_tools(&media.path))
            .and_then(|i| encode_thumbnail(&i))
            .or_else(|| exif_embedded_thumbnail(&media.path))
    };
    result.unwrap_or_else(|| {
        tracing::debug!(file = %media.file_name, "no thumbnail could be made; using placeholder");
        placeholder_thumbnail()
    })
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

/// Decode stills the `image` crate cannot read (HEIC/HEIF/AVIF, RAW) with
/// whatever converter is installed: `heif-convert`, ImageMagick, `ffmpeg`.
fn decode_with_external_tools(path: &Path) -> Option<image::DynamicImage> {
    let dir = tempfile::tempdir().ok()?;
    let out = dir.path().join("frame.png");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let heif = matches!(ext.as_str(), "heic" | "heif" | "hif" | "avif");
    let mut attempts: Vec<Command> = Vec::new();
    if heif && have_program("heif-convert") {
        let mut c = Command::new("heif-convert");
        c.arg(path).arg(&out);
        attempts.push(c);
    }
    if have_program("magick") {
        let mut c = Command::new("magick");
        // `[0]` selects the first frame/primary image; `-auto-orient` applies EXIF.
        let mut input = path.as_os_str().to_owned();
        input.push("[0]");
        c.arg(input).arg("-auto-orient").arg(&out);
        attempts.push(c);
    }
    if have_ffmpeg() {
        let mut c = Command::new("ffmpeg");
        c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
            .arg(path)
            .args(["-frames:v", "1"])
            .arg(&out);
        attempts.push(c);
    }
    for mut cmd in attempts {
        let ok = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok && let Some(img) = decode_with_image_crate(&out) {
            return Some(img);
        }
        let _ = std::fs::remove_file(&out);
    }
    None
}

/// A frame of a video at 1 s (or the first frame for shorter clips), like
/// Ente's `_generateVideoThumbnail`.
fn video_frame(path: &Path) -> Option<image::DynamicImage> {
    if !have_ffmpeg() {
        return None;
    }
    let dir = tempfile::tempdir().ok()?;
    let out = dir.path().join("frame.png");
    for seek in ["00:00:01", "00:00:00"] {
        let ok = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-y", "-i"])
            .arg(path)
            .args(["-ss", seek, "-vframes", "1"])
            .arg(&out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if ok && let Some(img) = decode_with_image_crate(&out) {
            return Some(img);
        }
        let _ = std::fs::remove_file(&out);
    }
    None
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

// ---------------------------------------------------------------------------
// Live Photos
// ---------------------------------------------------------------------------

/// Name of a Live Photo zip entry: `image.<ext>` / `video.<ext>`, keeping the
/// original extension's case, or just `image` / `video` if the extension
/// would not satisfy the apps' entry pattern `^(image|video)(\.[A-Za-z0-9]{1,16})?$`.
pub fn live_photo_entry_name(prefix: &str, file_name: &str) -> String {
    // Ente's nameAndExtension: split at the last dot, unless it is the first char.
    let ext = match file_name.rfind('.') {
        Some(i) if i > 0 => Some(&file_name[i + 1..]),
        _ => None,
    };
    match ext {
        Some(e) if (1..=16).contains(&e.len()) && e.bytes().all(|c| c.is_ascii_alphanumeric()) => {
            format!("{prefix}.{e}")
        }
        _ => prefix.to_string(),
    }
}

/// Bundle a Live Photo the way the Ente apps do (a zip holding the still and
/// the video). The returned temp file is deleted when dropped.
///
/// The archive holds exactly two stored (uncompressed, as JSZip's default)
/// entries, `image.<ext>` then `video.<ext>`, named from the components'
/// original file names.
pub fn live_photo_bundle(
    image: &MediaFile,
    video: &MediaFile,
) -> anyhow::Result<tempfile::NamedTempFile> {
    let mut tmp = tempfile::Builder::new()
        .prefix("livephoto-")
        .suffix(".zip")
        .tempfile()?;
    {
        let mut zip = zip::ZipWriter::new(tmp.as_file_mut());
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored)
            .large_file(false);
        for (prefix, part) in [("image", image), ("video", video)] {
            let mut src = std::fs::File::open(&part.path)?;
            let size = src.metadata()?.len();
            let opts = opts.large_file(size >= u32::MAX as u64);
            zip.start_file(live_photo_entry_name(prefix, &part.file_name), opts)?;
            std::io::copy(&mut src, &mut zip)?;
        }
        zip.finish()?;
    }
    tmp.as_file_mut().flush()?;
    tmp.as_file_mut().rewind()?;
    Ok(tmp)
}
