//! Shared helpers for the media and iCloud tests: synthetic JPEGs with
//! hand-built EXIF, so no binary fixtures are needed.

#![allow(dead_code)]

use exif::experimental::Writer;
use exif::{Field, In, Rational, Tag, Value};

/// EXIF fields to embed in a generated JPEG.
#[derive(Default, Clone)]
pub struct ExifSpec {
    /// `YYYY:MM:DD HH:MM:SS`.
    pub date_time_original: Option<&'static str>,
    /// `+HH:MM`.
    pub offset_time_original: Option<&'static str>,
    /// EXIF orientation 1..8.
    pub orientation: Option<u16>,
    /// (lat, lon) in signed degrees.
    pub gps: Option<(f64, f64)>,
    /// ImageDescription.
    pub description: Option<&'static str>,
}

fn dms(v: f64) -> Vec<Rational> {
    let v = v.abs();
    let d = v.trunc();
    let m = ((v - d) * 60.0).trunc();
    let s = ((v - d) * 60.0 - m) * 60.0;
    vec![
        Rational::from((d as u32, 1)),
        Rational::from((m as u32, 1)),
        Rational::from(((s * 10000.0).round() as u32, 10000)),
    ]
}

fn ascii(s: &str) -> Value {
    Value::Ascii(vec![s.as_bytes().to_vec()])
}

fn field(tag: Tag, value: Value) -> Field {
    Field {
        tag,
        ifd_num: In::PRIMARY,
        value,
    }
}

/// The TIFF blob of an EXIF APP1 segment.
pub fn exif_blob(spec: &ExifSpec) -> Vec<u8> {
    let mut fields = Vec::new();
    if let Some(d) = spec.date_time_original {
        fields.push(field(Tag::DateTimeOriginal, ascii(d)));
    }
    if let Some(o) = spec.offset_time_original {
        fields.push(field(Tag::OffsetTimeOriginal, ascii(o)));
    }
    if let Some(o) = spec.orientation {
        fields.push(field(Tag::Orientation, Value::Short(vec![o])));
    }
    if let Some(d) = spec.description {
        fields.push(field(Tag::ImageDescription, ascii(d)));
    }
    if let Some((lat, lon)) = spec.gps {
        let ns = if lat < 0.0 { "S" } else { "N" };
        let ew = if lon < 0.0 { "W" } else { "E" };
        fields.push(field(Tag::GPSLatitudeRef, ascii(ns)));
        fields.push(field(Tag::GPSLatitude, Value::Rational(dms(lat))));
        fields.push(field(Tag::GPSLongitudeRef, ascii(ew)));
        fields.push(field(Tag::GPSLongitude, Value::Rational(dms(lon))));
    }
    // The writer needs at least one field in IFD0.
    fields.push(field(Tag::Software, ascii("photovault-test")));
    let mut w = Writer::new();
    for f in &fields {
        w.push_field(f);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    w.write(&mut out, false).expect("write exif");
    out.into_inner()
}

/// A `w`×`h` JPEG (left half red, right half blue) with the given EXIF.
pub fn jpeg(w: u32, h: u32, spec: &ExifSpec) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, _| {
        if x < w / 2 {
            image::Rgb([220, 20, 20])
        } else {
            image::Rgb([20, 20, 220])
        }
    });
    let mut plain = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut plain, 80)
        .encode_image(&img)
        .unwrap();
    let blob = exif_blob(spec);
    let mut out = Vec::with_capacity(plain.len() + blob.len() + 10);
    out.extend_from_slice(&plain[..2]); // SOI
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&((blob.len() + 8) as u16).to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(&blob);
    out.extend_from_slice(&plain[2..]);
    out
}

/// Whether a program is on PATH (and runs).
pub fn have(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("-version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// Make a 2 s 320×240 test video with ffmpeg; `None` if ffmpeg is missing.
pub fn make_video(path: &std::path::Path, extra: &[&str]) -> Option<()> {
    if !have("ffmpeg") {
        eprintln!("ffmpeg not installed; skipping video part of test");
        return None;
    }
    let ok = std::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=320x240:rate=10",
            "-pix_fmt",
            "yuv420p",
        ])
        .args(extra)
        .arg(path)
        .status()
        .ok()?
        .success();
    assert!(ok, "ffmpeg failed to make a test video");
    Some(())
}
