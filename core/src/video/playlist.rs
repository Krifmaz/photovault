//! Ente's streamable-video playlist: the decrypted `vid_preview` file data,
//! its HLS template, and the AES-128 segments it describes.
//!
//! Format (Ente, `github.com/ente-io/ente` at b2ed3dc, 2026-11-03):
//!
//! * Generation (`mobile/apps/photos/lib/services/video_preview_service.dart`,
//!   `_createPreview`, around lines 515-590): a random 16-byte AES key is written
//!   to an ffmpeg `-hls_key_info_file` whose URI line is
//!   `data:text/plain;base64,<key>`; ffmpeg runs with
//!   `-f hls -hls_flags single_file -hls_list_size 0`. The result is one
//!   `output.ts` holding every segment, each encrypted on its own with
//!   AES-128-CBC and PKCS#7 padding (ffmpeg `hlsenc.c` `append_single_file`),
//!   and an `output.m3u8` with one `#EXT-X-KEY:METHOD=AES-128,URI="data:…",IV=0x…`
//!   and an `#EXT-X-BYTERANGE:<len>@<offset>` per segment.
//! * Upload (`lib/services/filedata/preview_upload.dart`, `_reportVideoPreview`
//!   around line 857): `output.ts` is PUT as is to the URL from
//!   `GET /files/data/preview-upload-url?fileID&type=vid_preview`; the JSON
//!   `{"playlist": <m3u8>, "type": "hls_video", "width", "height", "size"}` is
//!   gzipped and encrypted with the **file key** as one secretstream message
//!   (`gzipAndEncryptJson`, `lib/utils/gzip.dart`; web `encryptBlob`) and
//!   registered with `PUT /files/video-data {fileID, objectID, objectSize,
//!   playlist, playlistHeader}`.
//! * Playback (web `web/packages/gallery/services/video.ts`
//!   `hlsPlaylistDataForFile` lines 185-256; mobile `_getPlaylist` around line
//!   967): `GET /files/data/fetch?fileID&type=vid_preview&preferNoContent=true`
//!   (204 or 404: none), decrypt `encryptedData`/`decryptionHeader` with the file
//!   key, gunzip (at most 8 MiB, `maxHLSMetadataBytes`), check `type ==
//!   "hls_video"`, then `GET /files/data/preview?fileID&type=vid_preview` for the
//!   segment object's URL, and replace every URI line of the template with it
//!   after validating each tag (`reconstructHLSPlaylist`,
//!   `web/packages/gallery/utils/hls.ts`; `lib/utils/hls_playlist.dart`).
//!
//! Core validates the template exactly like `reconstructHLSPlaylist` and then,
//! instead of handing the key to the player, keeps it: the loopback server
//! ([`super::loopback`]) serves a key-less playlist and decrypts each segment
//! itself.

use crate::{CoreError, Result};
use aes::cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::Engine as _;
use serde::Deserialize;
use std::io::Read as _;

/// `maxHLSMetadataBytes`: cap of the gunzipped playlist JSON.
pub const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;
/// `maxHLSPlaylistBytes`: cap of a reconstructed playlist.
pub const MAX_PLAYLIST_BYTES: usize = 32 * 1024 * 1024;
/// `type` of a video preview playlist.
pub const HLS_VIDEO: &str = "hls_video";

/// The decrypted `vid_preview` JSON (web `PlaylistJSON`).
#[derive(Debug, Clone, Deserialize)]
pub struct PlaylistJson {
    /// `hls_video`.
    #[serde(rename = "type")]
    pub kind: String,
    /// The m3u8 template.
    pub playlist: String,
    /// Stream width (may be null when ffprobe failed on the client).
    #[serde(default)]
    pub width: Option<f64>,
    /// Stream height.
    #[serde(default)]
    pub height: Option<f64>,
    /// Bytes of the segment object.
    #[serde(default)]
    pub size: Option<f64>,
}

/// Gunzip (bounded) and parse the decrypted file data.
pub fn parse_playlist_json(gzipped: &[u8]) -> Result<PlaylistJson> {
    let mut out = Vec::new();
    let mut dec = flate2::read::GzDecoder::new(gzipped).take(MAX_METADATA_BYTES as u64 + 1);
    dec.read_to_end(&mut out)
        .map_err(|e| CoreError::crypto(format!("playlist is not gzip: {e}")))?;
    if out.len() > MAX_METADATA_BYTES {
        return Err(CoreError::crypto("playlist exceeds the allowed size"));
    }
    serde_json::from_slice(&out).map_err(|e| CoreError::crypto(format!("bad playlist JSON: {e}")))
}

/// Gzip and serialise, as Ente's clients do before encrypting.
#[cfg(test)]
pub fn encode_playlist_json(v: &serde_json::Value) -> Vec<u8> {
    use std::io::Write as _;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(v.to_string().as_bytes())
        .expect("writing to a Vec");
    enc.finish().expect("writing to a Vec")
}

/// One media segment of the preview.
pub struct Segment {
    /// The `#EXTINF` duration as written (seconds).
    pub duration: String,
    /// `(offset, length)` in the segment object; `None`: the whole object.
    pub range: Option<(u64, u64)>,
    /// AES-128 key.
    pub key: [u8; 16],
    /// CBC IV.
    pub iv: [u8; 16],
}

impl std::fmt::Debug for Segment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Segment")
            .field("duration", &self.duration)
            .field("range", &self.range)
            .finish_non_exhaustive()
    }
}

impl Drop for Segment {
    fn drop(&mut self) {
        crate::crypto::wipe(&mut self.key);
    }
}

impl Segment {
    /// Duration in seconds.
    pub fn seconds(&self) -> f64 {
        self.duration.parse().unwrap_or(0.0)
    }

    /// Decrypt this segment's ciphertext (AES-128-CBC, PKCS#7).
    pub fn decrypt(&self, ct: &[u8]) -> Result<Vec<u8>> {
        if ct.is_empty() || !ct.len().is_multiple_of(16) {
            return Err(CoreError::crypto("segment is not a whole number of blocks"));
        }
        let dec = cbc::Decryptor::<aes::Aes128>::new(&self.key.into(), &self.iv.into());
        let mut buf = ct.to_vec();
        let n = dec
            .decrypt_padded_mut::<Pkcs7>(&mut buf)
            .map_err(|_| CoreError::crypto("segment does not decrypt"))?
            .len();
        buf.truncate(n);
        Ok(buf)
    }
}

/// A validated preview: its segments, in order.
#[derive(Debug)]
pub struct Preview {
    /// `#EXT-X-TARGETDURATION` (or the longest segment, rounded up).
    pub target_duration: u64,
    /// Segments.
    pub segments: Vec<Segment>,
}

impl Preview {
    /// Total duration in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        (self.segments.iter().map(Segment::seconds).sum::<f64>() * 1000.0).round() as u64
    }

    /// The playlist the loopback server serves: no key, segment `n` at the
    /// relative URI `n.ts`.
    pub fn local_playlist(&self) -> String {
        let mut out = String::with_capacity(96 + self.segments.len() * 32);
        out.push_str("#EXTM3U\n#EXT-X-VERSION:3\n");
        out.push_str(&format!("#EXT-X-TARGETDURATION:{}\n", self.target_duration));
        out.push_str("#EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n");
        for (i, s) in self.segments.iter().enumerate() {
            out.push_str(&format!("#EXTINF:{},\n{i}.ts\n", s.duration));
        }
        out.push_str("#EXT-X-ENDLIST\n");
        out
    }
}

fn all(s: &str, f: impl Fn(u8) -> bool) -> bool {
    !s.is_empty() && s.bytes().all(f)
}

fn digits(s: &str) -> bool {
    all(s, |b| b.is_ascii_digit())
}

/// `\d+(?:\.\d+)?`
fn decimal(s: &str) -> bool {
    match s.split_once('.') {
        Some((a, b)) => digits(a) && digits(b),
        None => digits(s),
    }
}

/// One tag line, checked against Ente's `allowedTag` (web `utils/hls.ts`):
///
/// ```text
/// ^#(?:EXTM3U|EXT-X-(?:VERSION:\d+|TARGETDURATION:\d+|MEDIA-SEQUENCE:\d+|
///   KEY:METHOD=AES-128,URI="data:text\/plain;base64,[A-Za-z0-9+/]{22}==",IV=0x[0-9a-fA-F]{32}|
///   BYTERANGE:\d+(?:@\d+)?|ENDLIST)|EXTINF:\d+(?:\.\d+)?,$)$
/// ```
#[derive(Debug, PartialEq)]
enum Tag<'a> {
    Header,
    Version,
    TargetDuration(&'a str),
    MediaSequence,
    Key {
        key: &'a str,
        iv: &'a str,
    },
    ByteRange {
        len: &'a str,
        offset: Option<&'a str>,
    },
    EndList,
    Inf(&'a str),
}

fn tag(line: &str) -> Option<Tag<'_>> {
    if line == "#EXTM3U" {
        return Some(Tag::Header);
    }
    if line == "#EXT-X-ENDLIST" {
        return Some(Tag::EndList);
    }
    if let Some(v) = line.strip_prefix("#EXT-X-VERSION:") {
        return digits(v).then_some(Tag::Version);
    }
    if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
        return digits(v).then_some(Tag::TargetDuration(v));
    }
    if let Some(v) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
        return digits(v).then_some(Tag::MediaSequence);
    }
    if let Some(v) = line.strip_prefix("#EXT-X-BYTERANGE:") {
        return match v.split_once('@') {
            Some((len, off)) if digits(len) && digits(off) => Some(Tag::ByteRange {
                len,
                offset: Some(off),
            }),
            None if digits(v) => Some(Tag::ByteRange {
                len: v,
                offset: None,
            }),
            _ => None,
        };
    }
    if let Some(v) = line.strip_prefix("#EXTINF:") {
        let d = v.strip_suffix(',')?;
        return decimal(d).then_some(Tag::Inf(d));
    }
    if let Some(v) = line.strip_prefix("#EXT-X-KEY:METHOD=AES-128,URI=\"data:text/plain;base64,") {
        let (key, rest) = v.split_at_checked(24)?;
        let iv = rest.strip_prefix("\",IV=0x")?;
        let key_ok = key.ends_with("==")
            && all(&key[..22], |b| {
                b.is_ascii_alphanumeric() || b == b'+' || b == b'/'
            });
        let iv_ok = iv.len() == 32 && all(iv, |b| b.is_ascii_hexdigit());
        return (key_ok && iv_ok).then_some(Tag::Key { key, iv });
    }
    None
}

fn invalid() -> CoreError {
    CoreError::crypto("invalid HLS playlist")
}

/// Validate an Ente playlist template exactly like `reconstructHLSPlaylist`
/// and collect its segments with their keys and byte ranges.
pub fn parse_template(template: &str) -> Result<Preview> {
    // Dart's variant also rejects a lone CR.
    if template.contains('\r') && template.replace("\r\n", "\n").contains('\r') {
        return Err(invalid());
    }
    let mut lines = Vec::new();
    let mut bytes = 0usize;
    let mut key: Option<([u8; 16], [u8; 16])> = None;
    let mut inf: Option<&str> = None;
    let mut range: Option<(u64, Option<u64>)> = None;
    let mut next_offset = 0u64;
    let mut target = 0u64;
    let mut segments = Vec::new();
    for raw in template.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.is_empty() || (line.starts_with('#') && !line.starts_with("#EXT")) {
            continue;
        }
        if !line.starts_with('#') {
            // A URI line: every one is the segment object.
            let (k, iv) = key.as_ref().ok_or_else(invalid)?;
            let range = match range.take() {
                Some((len, off)) => {
                    let off = off.unwrap_or(next_offset);
                    next_offset = off.checked_add(len).ok_or_else(invalid)?;
                    Some((off, len))
                }
                None => None,
            };
            segments.push(Segment {
                duration: inf.take().unwrap_or("0").to_string(),
                range,
                key: *k,
                iv: *iv,
            });
            lines.push(line);
            bytes += 1;
            if bytes > MAX_PLAYLIST_BYTES {
                return Err(invalid());
            }
            continue;
        }
        match tag(line).ok_or_else(invalid)? {
            Tag::Key { key: k, iv } => {
                let kb = base64::engine::general_purpose::STANDARD
                    .decode(k)
                    .map_err(|_| invalid())?;
                let ivb = hex::decode(iv).map_err(|_| invalid())?;
                let (Ok(kb), Ok(ivb)) = (<[u8; 16]>::try_from(kb), <[u8; 16]>::try_from(ivb))
                else {
                    return Err(invalid());
                };
                key = Some((kb, ivb));
            }
            Tag::Inf(d) => inf = Some(d),
            Tag::ByteRange { len, offset } => {
                let len: u64 = len.parse().map_err(|_| invalid())?;
                let off = offset
                    .map(|o| o.parse::<u64>().map_err(|_| invalid()))
                    .transpose()?;
                range = Some((len, off));
            }
            Tag::TargetDuration(t) => target = t.parse().map_err(|_| invalid())?,
            Tag::Header | Tag::Version | Tag::MediaSequence | Tag::EndList => {}
        }
        bytes += line.len() + 1;
        if bytes > MAX_PLAYLIST_BYTES {
            return Err(invalid());
        }
        lines.push(line);
    }
    if key.is_none()
        || segments.is_empty()
        || lines.first() != Some(&"#EXTM3U")
        || lines.last() != Some(&"#EXT-X-ENDLIST")
    {
        return Err(invalid());
    }
    if target == 0 {
        target = segments
            .iter()
            .map(|s| s.seconds().ceil() as u64)
            .max()
            .unwrap_or(1)
            .max(1);
    }
    Ok(Preview {
        target_duration: target,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest as _, Sha256};

    const M3U8: &str = include_str!("../../tests/vectors/ente_hls/output.m3u8");
    const TS: &[u8] = include_bytes!("../../tests/vectors/ente_hls/output.ts");
    const SEGMENTS: &str = include_str!("../../tests/vectors/ente_hls/segments.txt");

    #[test]
    fn ffmpeg_output_from_entes_command_parses_and_decrypts_like_openssl() {
        let p = parse_template(M3U8).unwrap();
        assert_eq!(p.target_duration, 1);
        assert_eq!(p.segments.len(), 4);
        assert_eq!(p.duration_ms(), 4000);
        let expected: Vec<Vec<&str>> = SEGMENTS.lines().map(|l| l.split(' ').collect()).collect();
        for (s, e) in p.segments.iter().zip(&expected) {
            let (off, len) = s.range.unwrap();
            assert_eq!(off.to_string(), e[0]);
            assert_eq!(len.to_string(), e[1]);
            let pt = s.decrypt(&TS[off as usize..(off + len) as usize]).unwrap();
            assert_eq!(pt.len().to_string(), e[2]);
            assert_eq!(hex::encode(Sha256::digest(&pt)), e[3]);
            // MPEG-TS: 188-byte packets, each starting with the sync byte.
            assert_eq!(pt.len() % 188, 0);
            assert!(pt.chunks(188).all(|c| c[0] == 0x47));
        }
        let local = p.local_playlist();
        assert!(local.starts_with("#EXTM3U\n"));
        assert!(local.ends_with("#EXT-X-ENDLIST\n"));
        assert!(local.contains("#EXTINF:1.000000,\n0.ts\n"));
        assert!(local.contains("3.ts\n"));
        assert!(!local.contains("KEY") && !local.contains("output.ts"));
    }

    #[test]
    fn templates_are_validated_like_ente() {
        let key = "#EXT-X-KEY:METHOD=AES-128,URI=\"data:text/plain;base64,AAAAAAAAAAAAAAAAAAAAAA==\",IV=0x00000000000000000000000000000001";
        let ok = format!(
            "#EXTM3U\n{key}\n#EXTINF:2.5,\n#EXT-X-BYTERANGE:32@0\nseg\n#EXTINF:1,\n#EXT-X-BYTERANGE:16\nseg\n#EXT-X-ENDLIST\n"
        );
        let p = parse_template(&ok).unwrap();
        assert_eq!(p.segments[0].range, Some((0, 32)));
        // An offset-less range continues after the previous one.
        assert_eq!(p.segments[1].range, Some((32, 16)));
        assert_eq!(p.segments[0].iv[15], 1);
        assert_eq!(p.target_duration, 3);
        // CRLF is fine, comments are skipped.
        assert!(parse_template(&ok.replace('\n', "\r\n")).is_ok());
        assert!(parse_template(&ok.replace("#EXTINF:1,", "# a comment\n#EXTINF:1,")).is_ok());
        for bad in [
            ok.replace("#EXTM3U\n", ""),
            ok.replace("#EXT-X-ENDLIST\n", ""),
            ok.replace(&format!("{key}\n"), ""),
            ok.replace("AES-128", "SAMPLE-AES"),
            ok.replace("data:text/plain;base64,", "https://evil.example/"),
            ok.replace("IV=0x", "IV=0X"),
            ok.replace("#EXTINF:1,", "#EXTINF:1,title"),
            ok.replace("#EXTINF:1,", "#EXT-X-DISCONTINUITY"),
            ok.replace("#EXTINF:1,", "#EXTINF:1,\rx"),
            "#EXTM3U\n#EXT-X-ENDLIST\n".to_string(),
        ] {
            assert!(parse_template(&bad).is_err(), "accepted: {bad:?}");
        }
    }

    #[test]
    fn playlist_json_roundtrips_through_gzip_and_is_bounded() {
        let v = serde_json::json!({"type": "hls_video", "playlist": M3U8, "width": 160, "height": 120, "size": 62816});
        let p = parse_playlist_json(&encode_playlist_json(&v)).unwrap();
        assert_eq!(p.kind, HLS_VIDEO);
        assert_eq!(p.playlist, M3U8);
        assert_eq!(
            (p.width, p.height, p.size),
            (Some(160.0), Some(120.0), Some(62816.0))
        );
        // Null dimensions (ffprobe failed on the generating client) are accepted.
        let v = serde_json::json!({"type": "hls_video", "playlist": "x", "width": null, "height": null, "size": 1});
        assert!(parse_playlist_json(&encode_playlist_json(&v)).is_ok());
        assert!(parse_playlist_json(b"not gzip").is_err());
        let big = serde_json::Value::String("a".repeat(MAX_METADATA_BYTES + 1));
        assert!(parse_playlist_json(&encode_playlist_json(&big)).is_err());
    }

    #[test]
    fn damaged_segments_fail() {
        let p = parse_template(M3U8).unwrap();
        let s = &p.segments[0];
        let (off, len) = s.range.unwrap();
        let mut ct = TS[off as usize..(off + len) as usize].to_vec();
        assert!(s.decrypt(&ct[..ct.len() - 1]).is_err());
        let last = ct.len() - 1;
        ct[last] ^= 1;
        assert!(s.decrypt(&ct).is_err());
    }
}
