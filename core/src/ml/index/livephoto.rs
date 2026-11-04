//! The image component of an Ente Live Photo, for indexing.
//!
//! Ente stores a Live Photo as a ZIP with exactly two entries, `image.<ext>`
//! and `video.<ext>` (`web/packages/media/live-photo.ts`,
//! `decodeLivePhoto`). This is a minimal reader for that layout: the
//! central directory, stored or deflated entries, and the same expansion
//! limit as the web client (`20 x` the archive size `+ 16 MiB`).
//!
//! Based on Ente (ente.com).

use flate2::read::DeflateDecoder;
use std::io::Read;

const EOCD_SIG: u32 = 0x0605_4b50;
const CEN_SIG: u32 = 0x0201_4b50;
const LOC_SIG: u32 = 0x0403_4b50;
const MAX_EXPANDED_RATIO: u64 = 20;
const MAX_EXPANDED_OVERHEAD: u64 = 16 * 1024 * 1024;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// `^(image|video)(?:\.[a-zA-Z0-9]{1,16})?$`: which component a name is.
fn component(name: &str) -> Option<&'static str> {
    let (stem, ext) = match name.split_once('.') {
        Some((s, e)) => (s, Some(e)),
        None => (name, None),
    };
    let ext_ok = ext.is_none_or(|e| {
        (1..=16).contains(&e.len()) && e.bytes().all(|c| c.is_ascii_alphanumeric())
    });
    match stem {
        "image" if ext_ok => Some("image"),
        "video" if ext_ok => Some("video"),
        _ => None,
    }
}

/// The image bytes of a Live Photo ZIP.
pub(crate) fn image_of(zip: &[u8]) -> Result<Vec<u8>, String> {
    let bad = || "invalid Live Photo archive".to_string();
    // End of central directory: the last record with the signature.
    let tail_start = zip.len().saturating_sub(22 + 0xFFFF);
    let eocd = (tail_start..zip.len().saturating_sub(21))
        .rev()
        .find(|&i| u32_at(zip, i) == Some(EOCD_SIG))
        .ok_or_else(bad)?;
    let entries = usize::from(u16_at(zip, eocd + 10).ok_or_else(bad)?);
    let mut p = u32_at(zip, eocd + 16).ok_or_else(bad)? as usize;
    if entries != 2 {
        return Err("Live Photo must contain one image and one video".into());
    }
    let mut image = None;
    let mut has_video = false;
    for _ in 0..entries {
        if u32_at(zip, p) != Some(CEN_SIG) {
            return Err(bad());
        }
        let method = u16_at(zip, p + 10).ok_or_else(bad)?;
        let csize = u32_at(zip, p + 20).ok_or_else(bad)? as usize;
        let name_len = usize::from(u16_at(zip, p + 28).ok_or_else(bad)?);
        let extra_len = usize::from(u16_at(zip, p + 30).ok_or_else(bad)?);
        let comment_len = usize::from(u16_at(zip, p + 32).ok_or_else(bad)?);
        let local = u32_at(zip, p + 42).ok_or_else(bad)? as usize;
        let name = zip.get(p + 46..p + 46 + name_len).ok_or_else(bad)?;
        let name = std::str::from_utf8(name).map_err(|_| bad())?;
        match component(name) {
            Some("image") if image.is_none() => image = Some((method, csize, local)),
            Some("video") if !has_video => has_video = true,
            _ => return Err("invalid Live Photo component name".into()),
        }
        p += 46 + name_len + extra_len + comment_len;
    }
    let (method, csize, local) = match (image, has_video) {
        (Some(i), true) => i,
        _ => return Err("Live Photo must contain one image and one video".into()),
    };
    if u32_at(zip, local) != Some(LOC_SIG) {
        return Err(bad());
    }
    let n = usize::from(u16_at(zip, local + 26).ok_or_else(bad)?);
    let e = usize::from(u16_at(zip, local + 28).ok_or_else(bad)?);
    let start = local + 30 + n + e;
    let data = zip.get(start..start + csize).ok_or_else(bad)?;
    let limit = (zip.len() as u64)
        .saturating_mul(MAX_EXPANDED_RATIO)
        .saturating_add(MAX_EXPANDED_OVERHEAD);
    match method {
        0 => Ok(data.to_vec()),
        8 => {
            let mut out = Vec::new();
            DeflateDecoder::new(data)
                .take(limit + 1)
                .read_to_end(&mut out)
                .map_err(|e| format!("Live Photo image: {e}"))?;
            if out.len() as u64 > limit {
                return Err("Live Photo image expands too much".into());
            }
            Ok(out)
        }
        m => Err(format!("unsupported ZIP compression {m}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::DeflateEncoder;
    use std::io::Write;

    /// A ZIP with the given `(name, data, deflate)` entries.
    pub(crate) fn zip(entries: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cen = Vec::new();
        for (name, data, deflate) in entries {
            let body = if *deflate {
                let mut e = DeflateEncoder::new(Vec::new(), Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let off = out.len() as u32;
            out.extend_from_slice(&LOC_SIG.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0; 8]); // time, date, crc
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&body);

            cen.extend_from_slice(&CEN_SIG.to_le_bytes());
            cen.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            cen.extend_from_slice(&method.to_le_bytes());
            cen.extend_from_slice(&[0; 8]);
            cen.extend_from_slice(&(body.len() as u32).to_le_bytes());
            cen.extend_from_slice(&(data.len() as u32).to_le_bytes());
            cen.extend_from_slice(&(name.len() as u16).to_le_bytes());
            cen.extend_from_slice(&[0; 12]); // extra, comment, disk, attrs
            cen.extend_from_slice(&off.to_le_bytes());
            cen.extend_from_slice(name.as_bytes());
        }
        let cen_off = out.len() as u32;
        out.extend_from_slice(&cen);
        out.extend_from_slice(&EOCD_SIG.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        let n = entries.len() as u16;
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(&(cen.len() as u32).to_le_bytes());
        out.extend_from_slice(&cen_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    #[test]
    fn reads_the_image_entry() {
        let img = b"jpeg bytes".repeat(100);
        for deflate in [false, true] {
            let z = zip(&[
                ("video.mov", b"mov", deflate),
                ("image.heic", &img, deflate),
            ]);
            assert_eq!(image_of(&z).unwrap(), img);
        }
    }

    #[test]
    fn rejects_bad_layouts() {
        assert!(image_of(b"not a zip").is_err());
        assert!(image_of(&zip(&[("image.jpg", b"x", false)])).is_err());
        assert!(
            image_of(&zip(&[
                ("image.jpg", b"x", false),
                ("../video.mov", b"y", false)
            ]))
            .is_err()
        );
        assert!(
            image_of(&zip(&[
                ("image.jpg", b"x", false),
                ("image.png", b"y", false)
            ]))
            .is_err()
        );
        assert_eq!(component("image"), Some("image"));
        assert_eq!(component("video.MP4"), Some("video"));
        assert_eq!(component("image.tar.gz"), None);
    }
}
