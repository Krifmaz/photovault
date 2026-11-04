//! Face thumbnails: a square JPEG crop around a face, with Ente's padding
//! (web `extractFaceCrop` in `services/ml/crop.ts`: 40 % of the face size
//! on each side, reduced symmetrically down to 10 % at an image edge),
//! then widened to a square.
//!
//! The crop comes from the original when it is an image the `image` crate
//! decodes (from the encrypted file cache, else downloaded into memory),
//! else from the thumbnail. Crops are cached under `<cache>/faces`,
//! encrypted with the file's key (`header || secretstream blob`), like the
//! thumbnail cache; nothing decrypted touches the disk.
//!
//! Based on Ente (ente.com).

use super::CROP_CACHE_DIR;
use crate::client::Inner;
use crate::crypto;
use crate::{CoreError, Result};
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Regular padding around the face (fraction of its size).
const REGULAR_PADDING: f64 = 0.4;
/// Padding kept at an image edge.
const MINIMUM_PADDING: f64 = 0.1;
/// Largest crop edge in pixels.
const MAX_EDGE: u32 = 256;
/// JPEG quality (Ente: 0.8).
const JPEG_QUALITY: u8 = 80;
/// Originals larger than this are not fetched for a crop.
const MAX_ORIGINAL_BYTES: u64 = 40 * 1024 * 1024;
/// Size the crop cache is trimmed to.
const CROP_CACHE_MAX: u64 = 64 * 1024 * 1024;
/// Secretstream header length.
const HEADER_BYTES: usize = 24;

/// A face box normalised to `[0, 1]`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NormBox {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// The square crop rectangle `(x, y, side)` in pixels: Ente's padded crop,
/// widened to a square around its centre and kept inside the image.
pub(crate) fn crop_rect(b: NormBox, iw: u32, ih: u32) -> Option<(u32, u32, u32)> {
    let (iw_f, ih_f) = (f64::from(iw), f64::from(ih));
    let fx = b.x * iw_f;
    let fy = b.y * ih_f;
    let fw = b.w * iw_f;
    let fh = b.h * ih_f;
    if fw <= 0.0 || fh <= 0.0 || !fx.is_finite() || !fy.is_finite() {
        return None;
    }
    let x_crop = fx - fw * REGULAR_PADDING;
    let x_over = (x_crop.min(0.0)).abs() / fw;
    let w_crop = fw * (1.0 + 2.0 * REGULAR_PADDING)
        - 2.0 * x_over.min(REGULAR_PADDING - MINIMUM_PADDING) * fw;
    let y_crop = fy - fh * REGULAR_PADDING;
    let y_over = (y_crop.min(0.0)).abs() / fh;
    let h_crop = fh * (1.0 + 2.0 * REGULAR_PADDING)
        - 2.0 * y_over.min(REGULAR_PADDING - MINIMUM_PADDING) * fh;
    let x = x_crop.clamp(0.0, iw_f);
    let y = y_crop.clamp(0.0, ih_f);
    let w = w_crop.clamp(0.0, iw_f - x);
    let h = h_crop.clamp(0.0, ih_f - y);
    // Square around the centre.
    let side = w.max(h).min(iw_f).min(ih_f).round().max(1.0);
    let cx = x + w / 2.0;
    let cy = y + h / 2.0;
    let sx = (cx - side / 2.0).clamp(0.0, (iw_f - side).max(0.0));
    let sy = (cy - side / 2.0).clamp(0.0, (ih_f - side).max(0.0));
    Some((sx.round() as u32, sy.round() as u32, side as u32))
}

/// Decode an image (EXIF orientation applied: face boxes refer to the
/// image as displayed).
fn decode(bytes: &[u8]) -> Option<image::DynamicImage> {
    use image::ImageDecoder as _;
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    img.apply_orientation(orientation);
    Some(img)
}

/// Crop a face out of an encoded image: a square JPEG.
pub(crate) fn crop_face(bytes: &[u8], b: NormBox) -> Option<Vec<u8>> {
    let img = decode(bytes)?;
    let (x, y, side) = crop_rect(b, img.width(), img.height())?;
    let mut c = img.crop_imm(x, y, side, side);
    if side > MAX_EDGE {
        c = c.resize_exact(MAX_EDGE, MAX_EDGE, image::imageops::FilterType::Lanczos3);
    }
    let rgb = c.to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY)
        .encode_image(&rgb)
        .ok()?;
    Some(out)
}

fn crop_path(inner: &Inner, face_id: &str) -> PathBuf {
    // Face ids are `<int>_<decimals>…`: safe as file names; anything else
    // is hex encoded.
    let safe = face_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-');
    let name = if safe && !face_id.starts_with('.') {
        face_id.to_string()
    } else {
        hex::encode(face_id)
    };
    inner.cache_dir.join(CROP_CACHE_DIR).join(name)
}

/// The face's box: from the snapshot, else from the stored `mldata`.
fn face_box(inner: &Inner, file_id: i64, face_id: &str) -> Result<NormBox> {
    let snap = inner.ml.faces.snapshot();
    if let Some(f) = snap.face(face_id) {
        return Ok(NormBox {
            x: f64::from(f.x),
            y: f64::from(f.y),
            w: f64::from(f.w),
            h: f64::from(f.h),
        });
    }
    let d = inner
        .store()
        .ml_data(file_id)?
        .ok_or_else(|| CoreError::storage(format!("no face index for file {file_id}")))?;
    let f = d
        .current_face()
        .and_then(|fi| fi.faces.iter().find(|f| f.face_id == face_id))
        .ok_or_else(|| CoreError::storage(format!("unknown face {face_id}")))?;
    let b = &f.detection.face_box;
    Ok(NormBox {
        x: b.x,
        y: b.y,
        w: b.width,
        h: b.height,
    })
}

/// The decrypted original, if it is an image small enough to crop from.
async fn original(inner: &Inner, file_id: i64) -> Option<Vec<u8>> {
    let f = inner.file_rec(file_id).ok()?;
    // Images only (a live photo's original is a zip, a video is a video).
    if f.metadata
        .get("fileType")
        .and_then(serde_json::Value::as_i64)
        != Some(0)
    {
        return None;
    }
    let header = crypto::unb64(&f.file_header).ok()?;
    let cached = inner.cache_dir.join("files").join(file_id.to_string());
    let ct = match tokio::fs::read(&cached).await {
        Ok(ct) => ct,
        Err(_) => {
            if f.file_size.is_some_and(|s| s > MAX_ORIGINAL_BYTES) {
                return None;
            }
            let res = inner
                .api()
                .ok()?
                .get_object(&format!("/files/download/{file_id}"))
                .await
                .ok()?;
            let b = res.bytes().await.ok()?;
            if b.len() as u64 > MAX_ORIGINAL_BYTES {
                return None;
            }
            b.to_vec()
        }
    };
    let key = f.key.clone();
    tokio::task::spawn_blocking(move || crypto::decrypt_stream(&ct, &header, &key).ok())
        .await
        .ok()
        .flatten()
}

/// See [`crate::Core::face_thumbnail`].
pub(crate) async fn face_thumbnail(inner: &Inner, face_id: &str) -> Result<Vec<u8>> {
    inner.session()?;
    let file_id = super::file_id_of(face_id)
        .ok_or_else(|| CoreError::storage(format!("bad face id {face_id}")))?;
    let f = inner.file_rec(file_id)?;
    let path = crop_path(inner, face_id);
    if let Ok(blob) = tokio::fs::read(&path).await {
        if blob.len() > HEADER_BYTES
            && let Ok(p) =
                crypto::decrypt_blob(&blob[HEADER_BYTES..], &blob[..HEADER_BYTES], &f.key)
        {
            return Ok(p);
        }
        let _ = tokio::fs::remove_file(&path).await;
    }
    let b = face_box(inner, file_id, face_id)?;
    let mut jpeg = None;
    if let Some(orig) = original(inner, file_id).await {
        jpeg = tokio::task::spawn_blocking(move || crop_face(&orig, b))
            .await
            .ok()
            .flatten();
    }
    if jpeg.is_none() {
        let thumb = inner.thumbnail(file_id).await?;
        jpeg = tokio::task::spawn_blocking(move || crop_face(&thumb, b))
            .await
            .ok()
            .flatten();
    }
    let jpeg = jpeg.ok_or_else(|| CoreError::storage("cannot decode the image of this face"))?;
    let (ct, header) = crypto::encrypt_blob(&jpeg, &f.key)?;
    let mut blob = header;
    blob.extend_from_slice(&ct);
    if write_atomic(&path, &blob).await.is_ok() {
        let dir = inner.cache_dir.join(CROP_CACHE_DIR);
        let _ = tokio::task::spawn_blocking(move || trim_dir(&dir, CROP_CACHE_MAX)).await;
    }
    Ok(jpeg)
}

async fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    if let Some(d) = path.parent() {
        tokio::fs::create_dir_all(d).await?;
    }
    let tmp = path.with_extension(format!("tmp{}", hex::encode(crypto::random_bytes(4))));
    tokio::fs::write(&tmp, data).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

/// Remove the oldest files until the directory is under `max` bytes.
fn trim_dir(dir: &Path, max: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| {
                (
                    m.modified().unwrap_or(std::time::UNIX_EPOCH),
                    m.len(),
                    e.path(),
                )
            })
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max {
        return;
    }
    files.sort();
    for (_, len, p) in files {
        if total <= max {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_is_square_padded_and_inside() {
        let b = NormBox {
            x: 0.4,
            y: 0.4,
            w: 0.2,
            h: 0.1,
        };
        // 1000×1000: face 200×100 at (400, 400); padded 360×180 at (320, 360);
        // square 360 around the centre (500, 450).
        assert_eq!(crop_rect(b, 1000, 1000), Some((320, 270, 360)));
        // At the top-left edge the padding shrinks and the square stays inside.
        let edge = NormBox {
            x: 0.0,
            y: 0.0,
            w: 0.3,
            h: 0.3,
        };
        let (x, y, s) = crop_rect(edge, 100, 100).unwrap();
        assert_eq!((x, y), (0, 0));
        assert!((30..=100).contains(&s));
        assert_eq!(
            crop_rect(
                NormBox {
                    x: 0.1,
                    y: 0.1,
                    w: 0.0,
                    h: 0.1
                },
                10,
                10
            ),
            None
        );
    }

    #[test]
    fn crops_a_jpeg() {
        let img = image::RgbImage::from_fn(640, 480, |x, y| image::Rgb([x as u8, y as u8, 9]));
        let mut src = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut src, 90)
            .encode_image(&img)
            .unwrap();
        let out = crop_face(
            &src,
            NormBox {
                x: 0.25,
                y: 0.25,
                w: 0.5,
                h: 0.5,
            },
        )
        .unwrap();
        let c = image::load_from_memory(&out).unwrap();
        assert_eq!(c.width(), c.height());
        assert!(c.width() <= MAX_EDGE);
    }
}
