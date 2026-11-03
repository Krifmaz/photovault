//! Ente's `mldata` file-data format: face and CLIP indexes for one file.
//!
//! On the wire, `mldata` is a JSON object, gzipped, then encrypted with the
//! file's key as one libsodium secretstream blob (`encryptBlob`), and stored
//! through `PUT /files/data` with `type = "mldata"`.
//!
//! The JSON object has two known top-level keys, `face` and `clip`. Clients
//! that know newer keys add them, and every client keeps keys it does not
//! know when it rewrites the object. This module mirrors the two reference
//! implementations exactly:
//!
//! - web/desktop: `web/packages/new/photos/services/ml/{ml-data,worker,face,clip,index-flags}.ts`
//!   (zod schemas `RemoteFaceIndex`, `RemoteCLIPIndex`, `ParsedRemoteMLData`);
//! - mobile: `mobile/apps/photos/lib/services/filedata/model/file_data.dart`,
//!   `models/ml/ml_versions.dart` and `utils/ml_util.dart`.
//!
//! The typed indexes serialise their fields in the web client's order
//! (`version`, `client`, `flags`, `width`, `height`, `faces`); merged objects
//! are `serde_json` maps, whose keys come out sorted. No reader depends on key
//! order. Integral values (`version`, `flags`, `width`, `height`) are always
//! written as JSON integers because the mobile client reads them with
//! `as int`.
//!
//! Based on Ente (ente.com).

use std::io::{Read, Write};

use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};

use crate::crypto::{self, CryptoError, EncryptedBlob, SecretKey};

/// The `type` of this file data on the server (`/files/data`).
pub const FILE_DATA_TYPE: &str = "mldata";

/// Current face indexing version (`faceIndexingVersion` on web,
/// `faceMlVersion` on mobile). Remote face indexes with a lower version are
/// ignored and recomputed.
pub const FACE_INDEXING_VERSION: i64 = 1;

/// Current CLIP indexing version (`clipIndexingVersion` on web,
/// `clipMlVersion` on mobile).
pub const CLIP_INDEXING_VERSION: i64 = 1;

/// Largest decompressed `mldata` JSON that clients accept (`maxMLDataBytes`).
pub const MAX_MLDATA_BYTES: usize = 4 * 1024 * 1024;

/// The `client` string this node writes into every index it produces,
/// in Ente's `<package>/<version>` form.
pub const CLIENT_IDENTIFIER: &str = concat!("io.photovault.node/", env!("CARGO_PKG_VERSION"));

/// Top-level key of the face index.
pub const FACE_KEY: &str = "face";
/// Top-level key of the CLIP index.
pub const CLIP_KEY: &str = "clip";

/// Cross-client index flag bits (`index-flags.ts`, `ml_versions.dart`).
///
/// Bits are immutable and never reused. Absent `flags` on the wire means 0
/// (an index from before flags existed).
pub mod flags {
    /// Produced by Ente's Rust ML runtime (`ente-ml`), which this node uses.
    pub const RUNTIME_RUST: i64 = 1 << 0;
    /// At least one model ran on CoreML.
    pub const COREML: i64 = 1 << 1;
    /// At least one model ran on WebGPU.
    pub const WEBGPU: i64 = 1 << 2;

    /// Flags for an index computed with Ente's Rust runtime, as
    /// `mlIndexFlagsForRustResult` computes them.
    ///
    /// There are no bits for CUDA, ROCm or MIGraphX: the apps define the bit
    /// set, and inventing bits would collide with future upstream ones, so
    /// those providers are reported as plain Rust-runtime results.
    #[must_use]
    pub const fn for_rust_result(used_coreml: bool, used_webgpu: bool) -> i64 {
        RUNTIME_RUST | if used_coreml { COREML } else { 0 } | if used_webgpu { WEBGPU } else { 0 }
    }
}

/// Errors reading or writing `mldata`.
#[derive(Debug, thiserror::Error)]
pub enum MlDataError {
    /// The gzip stream was invalid.
    #[error("invalid gzip data: {0}")]
    Gzip(#[from] std::io::Error),
    /// The decompressed JSON exceeds [`MAX_MLDATA_BYTES`].
    #[error("decompressed mldata exceeds {MAX_MLDATA_BYTES} bytes")]
    TooLarge,
    /// The decompressed bytes were not UTF-8.
    #[error("mldata is not valid UTF-8")]
    Utf8,
    /// The JSON was malformed.
    #[error("invalid mldata JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The JSON was valid but its top level is not an object.
    #[error("mldata JSON is not an object")]
    NotObject,
    /// The object lacks a key the mobile app requires before uploading.
    #[error("mldata is missing the `{0}` index")]
    Incomplete(&'static str),
    /// Encryption or decryption failed.
    #[error(transparent)]
    Crypto(#[from] CryptoError),
}

/// Result alias for this module.
pub type Result<T> = std::result::Result<T, MlDataError>;

/// A point, normalised to `[0, 1]` relative to the image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    /// Horizontal position.
    pub x: f64,
    /// Vertical position.
    pub y: f64,
}

/// A face bounding box, normalised to `[0, 1]` relative to the image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FaceBox {
    /// Left edge.
    pub x: f64,
    /// Top edge.
    pub y: f64,
    /// Width.
    pub width: f64,
    /// Height.
    pub height: f64,
}

/// Where a face was detected.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Detection {
    /// Bounding box.
    #[serde(rename = "box")]
    pub face_box: FaceBox,
    /// Five landmarks: left eye, right eye, nose, left and right mouth
    /// corner. Empty for the mobile app's "no face" placeholder.
    pub landmarks: Vec<Point>,
}

/// One detected face.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    /// `<fileID>_<xMin>_<yMin>_<xMax>_<yMax>` with five-digit fractions,
    /// stable across equivalent reindexing.
    #[serde(rename = "faceID")]
    pub face_id: String,
    /// Box and landmarks.
    pub detection: Detection,
    /// Detection confidence.
    pub score: f64,
    /// Laplacian blur variance (higher is sharper).
    pub blur: f64,
    /// Face embedding (192 floats for MobileFaceNet).
    pub embedding: Vec<f64>,
}

impl Face {
    /// Builds a face from Ente's Rust ML output, exactly as the web worker's
    /// `faceIndexFromAnalysisResult` does: the box is converted from
    /// `[xMin, yMin, xMax, yMax]` to `{x, y, width, height}` and `f32` values
    /// are widened to `f64` (like `Array.from(Float32Array)`).
    #[must_use]
    pub fn from_rust_result(
        face_id: String,
        box_xyxy: [f32; 4],
        keypoints: &[[f32; 2]],
        score: f32,
        blur: f32,
        embedding: &[f32],
    ) -> Self {
        let [x0, y0, x1, y1] = box_xyxy.map(f64::from);
        Self {
            face_id,
            detection: Detection {
                face_box: FaceBox {
                    x: x0,
                    y: y0,
                    // Float32Array elements are read as f64, then subtracted.
                    width: x1 - x0,
                    height: y1 - y0,
                },
                landmarks: keypoints
                    .iter()
                    .map(|p| Point {
                        x: f64::from(p[0]),
                        y: f64::from(p[1]),
                    })
                    .collect(),
            },
            score: f64::from(score),
            blur: f64::from(blur),
            embedding: embedding.iter().copied().map(f64::from).collect(),
        }
    }
}

/// The remote face index of one file (`RemoteFaceIndex`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteFaceIndex {
    /// Indexing version; see [`FACE_INDEXING_VERSION`].
    #[serde(serialize_with = "ser_js_number")]
    pub version: f64,
    /// Producer, `<package>/<version>`.
    pub client: String,
    /// Bit set from [`flags`]; 0 when absent.
    #[serde(default, deserialize_with = "de_js_int")]
    pub flags: i64,
    /// Width of the decoded image the face coordinates refer to.
    #[serde(serialize_with = "ser_js_number")]
    pub width: f64,
    /// Height of the decoded image.
    #[serde(serialize_with = "ser_js_number")]
    pub height: f64,
    /// Detected faces (empty when there are none).
    pub faces: Vec<Face>,
}

/// The remote CLIP index of one file (`RemoteCLIPIndex`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RemoteClipIndex {
    /// Indexing version; see [`CLIP_INDEXING_VERSION`].
    #[serde(serialize_with = "ser_js_number")]
    pub version: f64,
    /// Producer, `<package>/<version>`.
    pub client: String,
    /// Bit set from [`flags`]; 0 when absent.
    #[serde(default, deserialize_with = "de_js_int")]
    pub flags: i64,
    /// Normalised image embedding (512 floats for MobileCLIP).
    pub embedding: Vec<f64>,
}

/// The known indexes of an `mldata` object (`ParsedRemoteMLData`).
///
/// Parsing is all-or-nothing like the web client's `safeParse`: if either
/// index is present but malformed, nothing is parsed and both are recomputed.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct ParsedRemoteMlData {
    /// The face index, if present (`null` counts as absent).
    #[serde(default)]
    pub face: Option<RemoteFaceIndex>,
    /// The CLIP index, if present (`null` counts as absent).
    #[serde(default)]
    pub clip: Option<RemoteClipIndex>,
}

/// Remote `mldata` for one file, as fetched (`RemoteMLData` on web).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemoteMlData {
    /// The whole JSON object, including keys this node does not know.
    /// Updates start from this so newer clients' data survives.
    pub raw: Map<String, Value>,
    /// The known indexes, or `None` if they did not validate.
    pub parsed: Option<ParsedRemoteMlData>,
    /// Server `updatedAt` of the fetched object, sent back as
    /// `lastUpdatedAt` for optimistic concurrency.
    pub updated_at: Option<i64>,
}

impl RemoteMlData {
    /// Parses decompressed `mldata` JSON (`remoteMLDataFromJSONString`).
    ///
    /// Fails only when the bytes are not UTF-8 JSON with an object at the
    /// top level; invalid known indexes leave [`Self::parsed`] as `None`.
    pub fn from_json(json: &[u8], updated_at: Option<i64>) -> Result<Self> {
        let text = std::str::from_utf8(json).map_err(|_| MlDataError::Utf8)?;
        let Value::Object(raw) = serde_json::from_str::<Value>(text)? else {
            return Err(MlDataError::NotObject);
        };
        let parsed = parse_known(&raw);
        Ok(Self {
            raw,
            parsed,
            updated_at,
        })
    }

    /// The remote face index if it is current, else `None`.
    ///
    /// Current means it validated, `version >= FACE_INDEXING_VERSION`
    /// (web `worker.ts`), and the mobile app would not discard it
    /// (`_shouldDiscardRemoteEmbedding`: every face has landmarks with
    /// `x == y`, a legacy bug signature).
    #[must_use]
    pub fn current_face(&self) -> Option<&RemoteFaceIndex> {
        let face = self.parsed.as_ref()?.face.as_ref()?;
        (face.version >= FACE_INDEXING_VERSION as f64 && !mobile_discards_face_index(face))
            .then_some(face)
    }

    /// The remote CLIP index if it validated and
    /// `version >= CLIP_INDEXING_VERSION`, else `None`.
    #[must_use]
    pub fn current_clip(&self) -> Option<&RemoteClipIndex> {
        let clip = self.parsed.as_ref()?.clip.as_ref()?;
        (clip.version >= CLIP_INDEXING_VERSION as f64).then_some(clip)
    }

    /// Which models still need to run for this file.
    #[must_use]
    pub fn plan(&self) -> IndexPlan {
        IndexPlan {
            run_faces: self.current_face().is_none(),
            run_clip: self.current_clip().is_none(),
        }
    }
}

/// Validates the known keys like `ParsedRemoteMLData.safeParse`.
fn parse_known(raw: &Map<String, Value>) -> Option<ParsedRemoteMlData> {
    let mut known = Map::new();
    for key in [FACE_KEY, CLIP_KEY] {
        if let Some(v) = raw.get(key) {
            known.insert(key.to_owned(), v.clone());
        }
    }
    serde_json::from_value(Value::Object(known)).ok()
}

/// Mobile's legacy-bug check: `true` when the faces list is non-empty and
/// every face has non-empty landmarks that all satisfy `x == y`.
fn mobile_discards_face_index(face: &RemoteFaceIndex) -> bool {
    !face.faces.is_empty()
        && face.faces.iter().all(|f| {
            !f.detection.landmarks.is_empty() && f.detection.landmarks.iter().all(|p| p.x == p.y)
        })
}

/// Which models must run for a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IndexPlan {
    /// Run face detection and embedding.
    pub run_faces: bool,
    /// Run the CLIP image model.
    pub run_clip: bool,
}

impl IndexPlan {
    /// Plan for a file with the given remote data (or none).
    #[must_use]
    pub fn for_remote(existing: Option<&RemoteMlData>) -> Self {
        existing.map_or(
            Self {
                run_faces: true,
                run_clip: true,
            },
            RemoteMlData::plan,
        )
    }

    /// `true` when nothing needs to run (both remote indexes are current).
    #[must_use]
    pub const fn is_done(self) -> bool {
        !self.run_faces && !self.run_clip
    }
}

impl RemoteFaceIndex {
    /// A face index produced by this node now.
    #[must_use]
    pub fn new(width: u32, height: u32, faces: Vec<Face>, flags: i64) -> Self {
        Self {
            version: FACE_INDEXING_VERSION as f64,
            client: CLIENT_IDENTIFIER.to_owned(),
            flags,
            width: f64::from(width),
            height: f64::from(height),
            faces,
        }
    }
}

impl RemoteClipIndex {
    /// A CLIP index produced by this node now.
    #[must_use]
    pub fn new(embedding: Vec<f64>, flags: i64) -> Self {
        Self {
            version: CLIP_INDEXING_VERSION as f64,
            client: CLIENT_IDENTIFIER.to_owned(),
            flags,
            embedding,
        }
    }
}

/// Builds the object to upload, as the web worker does:
/// `{...existing.raw, face?, clip?}`.
///
/// Every key of the existing object is kept; `face` and `clip` are replaced
/// only when a new index is given. The result must contain both indexes
/// (the mobile app's `FileDataEntity.validate` rejects anything else), so
/// callers pass a new index for each model that [`IndexPlan`] said to run.
pub fn merge(
    existing: Option<&RemoteMlData>,
    face: Option<&RemoteFaceIndex>,
    clip: Option<&RemoteClipIndex>,
) -> Result<Map<String, Value>> {
    let mut out = existing.map(|e| e.raw.clone()).unwrap_or_default();
    if let Some(face) = face {
        out.insert(FACE_KEY.to_owned(), serde_json::to_value(face)?);
    }
    if let Some(clip) = clip {
        out.insert(CLIP_KEY.to_owned(), serde_json::to_value(clip)?);
    }
    validate_for_upload(&out)?;
    Ok(out)
}

/// Mobile's `FileDataEntity.validate`: both `face` and `clip` must be
/// present and non-null before an upload.
pub fn validate_for_upload(raw: &Map<String, Value>) -> Result<()> {
    for key in [FACE_KEY, CLIP_KEY] {
        if raw.get(key).is_none_or(Value::is_null) {
            return Err(MlDataError::Incomplete(key));
        }
    }
    Ok(())
}

/// Serialises to JSON and gzips it (web `gzip(JSON.stringify(...))`).
pub fn gzip_json(raw: &Map<String, Value>) -> Result<Vec<u8>> {
    let json = serde_json::to_vec(raw)?;
    let mut enc = GzEncoder::new(Vec::with_capacity(json.len() / 2), Compression::default());
    enc.write_all(&json)?;
    Ok(enc.finish()?)
}

/// Gunzips at most [`MAX_MLDATA_BYTES`] (web `gunzipWithLimit`).
pub fn gunzip_limited(gz: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    // Read one byte past the limit to detect oversize input without
    // inflating the rest of it.
    let limit = u64::try_from(MAX_MLDATA_BYTES).unwrap_or(u64::MAX) + 1;
    GzDecoder::new(gz).take(limit).read_to_end(&mut out)?;
    if out.len() > MAX_MLDATA_BYTES {
        return Err(MlDataError::TooLarge);
    }
    Ok(out)
}

/// Gzips and encrypts an object with the file key, ready for
/// `PUT /files/data` (`encryptedData` and `decryptionHeader`, base64).
pub fn seal(raw: &Map<String, Value>, file_key: &SecretKey) -> Result<EncryptedBlob> {
    Ok(crypto::encrypt_blob(&gzip_json(raw)?, file_key)?)
}

/// Decrypts, gunzips and parses `mldata` fetched from `/files/data/fetch`.
pub fn open(
    encrypted_data: &[u8],
    decryption_header: &[u8],
    file_key: &SecretKey,
    updated_at: Option<i64>,
) -> Result<RemoteMlData> {
    let gz = crypto::decrypt_blob(encrypted_data, decryption_header, file_key)?;
    RemoteMlData::from_json(&gunzip_limited(&gz)?, updated_at)
}

/// Writes a number the way `JSON.stringify` does for integral values: as a
/// JSON integer (`1`, not `1.0`), which the mobile app's `as int` needs.
fn ser_js_number<S: Serializer>(v: &f64, s: S) -> std::result::Result<S::Ok, S::Error> {
    // 2^53: the largest range where every integer is an exact f64.
    const SAFE: f64 = 9_007_199_254_740_992.0;
    if v.fract() == 0.0 && v.abs() <= SAFE {
        // Exact by the range check above.
        #[expect(clippy::cast_possible_truncation, reason = "integral and within ±2^53")]
        s.serialize_i64(*v as i64)
    } else {
        s.serialize_f64(*v)
    }
}

/// Reads zod's `z.number().int()`: any JSON number that is a safe integer,
/// including `1.0`.
fn de_js_int<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<i64, D::Error> {
    const MAX_SAFE: i64 = 9_007_199_254_740_991;
    let n = serde_json::Number::deserialize(d)?;
    let v = if let Some(i) = n.as_i64() {
        Some(i)
    } else if let Some(f) = n.as_f64() {
        // Exact: integral and range-checked before the cast.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "integral and range-checked"
        )]
        (f.fract() == 0.0 && f.abs() <= MAX_SAFE as f64).then_some(f as i64)
    } else {
        None
    };
    v.filter(|i| i.abs() <= MAX_SAFE)
        .ok_or_else(|| serde::de::Error::custom("expected a safe integer"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_and_flags_match_upstream() {
        assert_eq!(FACE_INDEXING_VERSION, 1);
        assert_eq!(CLIP_INDEXING_VERSION, 1);
        assert_eq!(flags::for_rust_result(false, false), 1);
        assert_eq!(flags::for_rust_result(true, false), 3);
        assert_eq!(flags::for_rust_result(false, true), 5);
        assert!(CLIENT_IDENTIFIER.starts_with("io.photovault.node/"));
    }

    #[test]
    fn integral_numbers_are_written_as_integers() {
        let s = serde_json::to_string(&RemoteClipIndex::new(vec![0.5, 1.0], 1)).unwrap();
        assert!(s.contains(r#""version":1,"#), "{s}");
        assert!(s.contains(r#""flags":1,"#), "{s}");
        let f = serde_json::to_string(&RemoteFaceIndex::new(640, 480, vec![], 5)).unwrap();
        assert_eq!(
            f,
            format!(
                r#"{{"version":1,"client":"{CLIENT_IDENTIFIER}","flags":5,"width":640,"height":480,"faces":[]}}"#
            )
        );
    }

    #[test]
    fn flags_accept_integral_floats_and_reject_fractions() {
        let base = json!({"version": 1, "client": "c", "embedding": []});
        let mut v = base.clone();
        v["flags"] = json!(3.0);
        assert_eq!(
            serde_json::from_value::<RemoteClipIndex>(v).unwrap().flags,
            3
        );
        let mut v = base.clone();
        v["flags"] = json!(1.5);
        assert!(serde_json::from_value::<RemoteClipIndex>(v).is_err());
        let mut v = base.clone();
        v["flags"] = Value::Null;
        assert!(serde_json::from_value::<RemoteClipIndex>(v).is_err());
        assert_eq!(
            serde_json::from_value::<RemoteClipIndex>(base)
                .unwrap()
                .flags,
            0
        );
    }

    #[test]
    fn face_from_rust_result_matches_web_conversion() {
        let f = Face::from_rust_result(
            "7_10000_20000_50000_70000".into(),
            [0.1, 0.2, 0.5, 0.7],
            &[[0.2, 0.3]; 5],
            0.9,
            123.0,
            &[0.25, -0.5],
        );
        assert_eq!(f.detection.face_box.x, f64::from(0.1f32));
        assert_eq!(
            f.detection.face_box.width,
            f64::from(0.5f32) - f64::from(0.1f32)
        );
        assert_eq!(f.detection.landmarks.len(), 5);
        assert_eq!(f.embedding, vec![0.25, -0.5]);
    }

    #[test]
    fn gunzip_enforces_limit() {
        let big = vec![b' '; MAX_MLDATA_BYTES + 1];
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&big).unwrap();
        let gz = enc.finish().unwrap();
        assert!(matches!(gunzip_limited(&gz), Err(MlDataError::TooLarge)));

        let ok = vec![b' '; MAX_MLDATA_BYTES];
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&ok).unwrap();
        assert_eq!(
            gunzip_limited(&enc.finish().unwrap()).unwrap().len(),
            MAX_MLDATA_BYTES
        );
    }

    #[test]
    fn non_object_and_non_utf8_are_errors() {
        assert!(matches!(
            RemoteMlData::from_json(b"[]", None),
            Err(MlDataError::NotObject)
        ));
        assert!(matches!(
            RemoteMlData::from_json(b"\xff", None),
            Err(MlDataError::Utf8)
        ));
        assert!(RemoteMlData::from_json(b"{", None).is_err());
    }
}
