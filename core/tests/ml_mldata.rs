//! Verifies `ml::mldata` against the formats the real Ente apps write and
//! read.
//!
//! The fixtures in `tests/ml_fixtures/` are shaped exactly like the apps'
//! output (key order, number formatting, placeholder conventions):
//!
//! - `web_desktop.json`: web/desktop `worker.ts` (`JSON.stringify` of
//!   `{face: {version, client, flags, width, height, faces}, clip: {...}}`,
//!   with `Float32Array` values widened to f64);
//! - `mobile.json`, `mobile_no_faces_legacy.json`: mobile `file_data.dart`
//!   `toJson` order, Dart double formatting (`0.0`), `flags` omitted when 0,
//!   and the `Face.empty` placeholder (`<fileID>_0_0_0_0`) for no faces;
//! - `future_keys.json`, `old_face_version.json`, `invalid_face.json`,
//!   `mobile_equal_landmarks.json`: the merge and skip edge cases.
//!
//! Two independent re-implementations of the apps' readers check every
//! fixture and every mutation of it: [`zod`] (web `ml-data.ts` schemas) and
//! [`mobile`] (`RemoteFaceEmbedding.fromJson` and friends).

use photovault_core::crypto;
use photovault_core::ml::mldata::{
    self, CLIENT_IDENTIFIER, Face, IndexPlan, MlDataError, RemoteClipIndex, RemoteFaceIndex,
    RemoteMlData,
};
use serde_json::{Map, Value, json};

const FIXTURES: &[&str] = &[
    "web_desktop",
    "mobile",
    "mobile_no_faces_legacy",
    "future_keys",
    "old_face_version",
    "invalid_face",
    "mobile_equal_landmarks",
];

fn fixture(name: &str) -> Vec<u8> {
    let path = format!(
        "{}/tests/ml_fixtures/{name}.json",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn obj(v: Value) -> Map<String, Value> {
    match v {
        Value::Object(m) => m,
        other => panic!("not an object: {other}"),
    }
}

/// Re-implementation of the web app's zod schemas (`ml-data.ts`, zod v4).
mod zod {
    use serde_json::{Map, Value};

    fn number(v: Option<&Value>) -> bool {
        v.is_some_and(Value::is_number)
    }

    /// `z.number().int().default(0)`: absent is fine, otherwise a safe int.
    fn int_default(v: Option<&Value>) -> bool {
        match v {
            None => true,
            Some(Value::Number(n)) => n
                .as_f64()
                .is_some_and(|f| f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0),
            Some(_) => false,
        }
    }

    fn string(v: Option<&Value>) -> bool {
        v.is_some_and(Value::is_string)
    }

    fn array_of(v: Option<&Value>, item: impl Fn(&Value) -> bool) -> bool {
        v.and_then(Value::as_array)
            .is_some_and(|a| a.iter().all(item))
    }

    fn object(v: &Value) -> Option<&Map<String, Value>> {
        v.as_object()
    }

    fn point(v: &Value) -> bool {
        object(v).is_some_and(|o| number(o.get("x")) && number(o.get("y")))
    }

    fn face(v: &Value) -> bool {
        let Some(o) = object(v) else { return false };
        let det = o.get("detection").and_then(object);
        let bx = det.and_then(|d| d.get("box")).and_then(object);
        string(o.get("faceID"))
            && bx.is_some_and(|b| {
                ["x", "y", "width", "height"]
                    .iter()
                    .all(|k| number(b.get(*k)))
            })
            && array_of(det.and_then(|d| d.get("landmarks")), point)
            && number(o.get("score"))
            && number(o.get("blur"))
            && array_of(o.get("embedding"), Value::is_number)
    }

    /// `RemoteFaceIndex`.
    pub fn face_index(v: &Value) -> bool {
        object(v).is_some_and(|o| {
            number(o.get("version"))
                && string(o.get("client"))
                && int_default(o.get("flags"))
                && number(o.get("width"))
                && number(o.get("height"))
                && array_of(o.get("faces"), face)
        })
    }

    /// `RemoteCLIPIndex`.
    pub fn clip_index(v: &Value) -> bool {
        object(v).is_some_and(|o| {
            number(o.get("version"))
                && string(o.get("client"))
                && int_default(o.get("flags"))
                && array_of(o.get("embedding"), Value::is_number)
        })
    }

    /// `ParsedRemoteMLData.safeParse(raw).success`, plus which indexes exist.
    pub fn parsed(raw: &Map<String, Value>) -> Option<(bool, bool)> {
        let check = |key: &str, f: fn(&Value) -> bool| match raw.get(key) {
            None | Some(Value::Null) => Some(false),
            Some(v) => f(v).then_some(true),
        };
        Some((check("face", face_index)?, check("clip", clip_index)?))
    }
}

/// Re-implementation of the mobile app's reader and pre-upload check
/// (`file_data.dart`, `face.dart`, `detection.dart`, `box.dart`,
/// `landmark.dart`). Dart's `as int` rejects JSON numbers with a fraction
/// part or `.0`, so this works on the raw JSON text's number kinds.
mod mobile {
    use serde_json::{Map, Value};

    fn int(v: Option<&Value>) -> Result<i64, String> {
        v.and_then(Value::as_i64)
            .ok_or_else(|| format!("expected int, got {v:?}"))
    }

    fn double(v: Option<&Value>) -> Result<f64, String> {
        v.and_then(Value::as_f64)
            .ok_or_else(|| format!("expected num, got {v:?}"))
    }

    fn face(v: &Value) -> Result<(), String> {
        let o = v.as_object().ok_or("face is not an object")?;
        let id = o.get("faceID").and_then(Value::as_str).ok_or("faceID")?;
        // getFileIdFromFaceId<int>
        id.split('_')
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .ok_or("faceID has no file id prefix")?;
        for e in o
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or("embedding")?
        {
            double(Some(e))?;
        }
        double(o.get("score"))?;
        // blur may be absent (kLapacianDefault), but not a non-number.
        if let Some(b) = o.get("blur").filter(|b| !b.is_null()) {
            double(Some(b))?;
        }
        let det = o
            .get("detection")
            .and_then(Value::as_object)
            .ok_or("detection")?;
        let bx = det.get("box").and_then(Value::as_object).ok_or("box")?;
        double(bx.get("x").or_else(|| bx.get("xMin")))?;
        double(bx.get("y").or_else(|| bx.get("yMin")))?;
        double(bx.get("width"))?;
        double(bx.get("height"))?;
        for l in det
            .get("landmarks")
            .and_then(Value::as_array)
            .ok_or("landmarks")?
        {
            double(l.get("x"))?;
            double(l.get("y"))?;
        }
        Ok(())
    }

    /// `FileDataEntity.validate()` then `RemoteFaceEmbedding.fromJson` and
    /// `RemoteClipEmbedding.fromJson`.
    pub fn accepts(raw: &Map<String, Value>) -> Result<(), String> {
        let f = raw
            .get("face")
            .filter(|v| !v.is_null())
            .ok_or("Face embedding is null")?;
        let c = raw
            .get("clip")
            .filter(|v| !v.is_null())
            .ok_or("Clip embedding is null")?;
        let f = f.as_object().ok_or("face")?;
        for x in f.get("faces").and_then(Value::as_array).ok_or("faces")? {
            face(x)?;
        }
        int(f.get("version"))?;
        f.get("client").and_then(Value::as_str).ok_or("client")?;
        int(f.get("height"))?;
        int(f.get("width"))?;
        if let Some(fl) = f.get("flags").filter(|v| !v.is_null()) {
            int(Some(fl))?;
        }
        let c = c.as_object().ok_or("clip")?;
        for e in c
            .get("embedding")
            .and_then(Value::as_array)
            .ok_or("embedding")?
        {
            double(Some(e))?;
        }
        int(c.get("version"))?;
        c.get("client").and_then(Value::as_str).ok_or("client")?;
        if let Some(fl) = c.get("flags").filter(|v| !v.is_null()) {
            int(Some(fl))?;
        }
        Ok(())
    }
}

/// Our parse must agree with zod on every input.
fn assert_agrees_with_zod(raw: &Map<String, Value>, what: &str) {
    let ours = RemoteMlData::from_json(&serde_json::to_vec(raw).unwrap(), None).unwrap();
    let theirs = zod::parsed(raw);
    let ours_shape = ours
        .parsed
        .as_ref()
        .map(|p| (p.face.is_some(), p.clip.is_some()));
    assert_eq!(ours_shape, theirs, "parse disagrees with zod for {what}");
}

/// Every JSON value position in `v`, with a mutated copy of the root.
fn mutations(root: &Value) -> Vec<(String, Value)> {
    fn walk(v: &Value, path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
        out.push(path.clone());
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    path.push(k.clone());
                    walk(x, path, out);
                    path.pop();
                }
            }
            // Only the first array element: the rest have the same shape.
            Value::Array(a) => {
                if let Some(x) = a.first() {
                    path.push("0".into());
                    walk(x, path, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }
    fn at<'a>(v: &'a mut Value, path: &[String]) -> &'a mut Value {
        path.iter().fold(v, |v, k| match v {
            Value::Array(a) => &mut a[k.parse::<usize>().unwrap()],
            _ => &mut v[k.as_str()],
        })
    }
    let mut paths = Vec::new();
    walk(root, &mut Vec::new(), &mut paths);
    let mut out = Vec::new();
    for p in paths.into_iter().filter(|p| !p.is_empty()) {
        let name = p.join(".");
        for (label, replacement) in [
            ("null", Value::Null),
            ("string", json!("x")),
            ("float", json!(1.5)),
            ("integral float", json!(2.0)),
            ("array", json!([])),
        ] {
            let mut copy = root.clone();
            *at(&mut copy, &p) = replacement;
            out.push((format!("{name} = {label}"), copy));
        }
        let (parent, last) = p.split_at(p.len() - 1);
        let mut copy = root.clone();
        let removed = match at(&mut copy, parent) {
            Value::Object(m) => m.remove(&last[0]).is_some(),
            _ => false,
        };
        if removed {
            out.push((format!("{name} removed"), copy));
        }
    }
    out
}

#[test]
fn fixtures_parse_like_zod_including_every_mutation() {
    for name in FIXTURES {
        let raw = obj(serde_json::from_slice(&fixture(name)).unwrap());
        assert_agrees_with_zod(&raw, name);
        for (what, mutated) in mutations(&Value::Object(raw.clone())) {
            if let Value::Object(m) = mutated {
                assert_agrees_with_zod(&m, &format!("{name}: {what}"));
            }
        }
    }
}

#[test]
fn skip_rules_match_the_apps() {
    let plan = |name: &str| {
        RemoteMlData::from_json(&fixture(name), Some(1))
            .unwrap()
            .plan()
    };
    let done = IndexPlan {
        run_faces: false,
        run_clip: false,
    };
    assert_eq!(plan("web_desktop"), done);
    assert_eq!(plan("mobile"), done);
    assert_eq!(plan("mobile_no_faces_legacy"), done);
    assert_eq!(plan("future_keys"), done);
    // Face version 0 < faceIndexingVersion: faces only.
    assert_eq!(
        plan("old_face_version"),
        IndexPlan {
            run_faces: true,
            run_clip: false
        }
    );
    // safeParse fails as a whole: both rerun.
    assert_eq!(
        plan("invalid_face"),
        IndexPlan {
            run_faces: true,
            run_clip: true
        }
    );
    // Mobile discards indexes whose landmarks all have x == y.
    assert_eq!(
        plan("mobile_equal_landmarks"),
        IndexPlan {
            run_faces: true,
            run_clip: false
        }
    );
    assert!(IndexPlan::for_remote(None).run_faces && IndexPlan::for_remote(None).run_clip);
    assert!(IndexPlan::for_remote(Some(&RemoteMlData::default())).run_faces);
}

#[test]
fn fixtures_parse_to_expected_values() {
    let web = RemoteMlData::from_json(&fixture("web_desktop"), Some(7)).unwrap();
    let p = web.parsed.as_ref().unwrap();
    let face = p.face.as_ref().unwrap();
    assert_eq!((face.width, face.height, face.flags), (4032.0, 3024.0, 5));
    assert_eq!(face.faces.len(), 2);
    assert_eq!(face.faces[0].embedding.len(), 192);
    assert!(face.faces[0].face_id.starts_with("1001_"));
    assert_eq!(p.clip.as_ref().unwrap().embedding.len(), 512);
    assert_eq!(web.updated_at, Some(7));

    let legacy = RemoteMlData::from_json(&fixture("mobile_no_faces_legacy"), None).unwrap();
    let face = legacy.current_face().unwrap();
    assert_eq!(face.flags, 0, "absent flags mean legacy 0");
    assert_eq!(face.faces[0].face_id, "3003_0_0_0_0");
    assert!(face.faces[0].embedding.is_empty());
    assert_eq!(legacy.current_clip().unwrap().flags, 0);
}

#[test]
fn typed_round_trip_is_lossless_for_app_output() {
    for name in ["web_desktop", "mobile", "mobile_no_faces_legacy"] {
        let raw = obj(serde_json::from_slice(&fixture(name)).unwrap());
        let parsed = RemoteMlData::from_json(&fixture(name), None)
            .unwrap()
            .parsed
            .unwrap();
        let face: Value = serde_json::to_value(parsed.face.as_ref().unwrap()).unwrap();
        let clip: Value = serde_json::to_value(parsed.clip.as_ref().unwrap()).unwrap();
        // Same values (flags 0 is written explicitly, like web does).
        let mut expect_face = raw["face"].clone();
        let mut expect_clip = raw["clip"].clone();
        for v in [&mut expect_face, &mut expect_clip] {
            if v.get("flags").is_none() {
                v["flags"] = json!(0);
            }
        }
        assert_eq!(normalize(&face), normalize(&expect_face), "{name} face");
        assert_eq!(normalize(&clip), normalize(&expect_clip), "{name} clip");
    }
}

/// Compares numbers by value (Dart's `0.0` equals JS's `0`).
fn normalize(v: &Value) -> Value {
    match v {
        Value::Number(n) => json!(n.as_f64().unwrap()),
        Value::Array(a) => Value::Array(a.iter().map(normalize).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, v)| (k.clone(), normalize(v))).collect())
        }
        other => other.clone(),
    }
}

fn node_face() -> RemoteFaceIndex {
    let face = Face::from_rust_result(
        "42_10000_20000_30000_50000".into(),
        [0.1, 0.2, 0.3, 0.5],
        &[
            [0.15, 0.3],
            [0.25, 0.3],
            [0.2, 0.38],
            [0.16, 0.45],
            [0.24, 0.45],
        ],
        0.91,
        300.25,
        &[0.1; 192],
    );
    RemoteFaceIndex::new(
        1024,
        768,
        vec![face],
        mldata::flags::for_rust_result(false, true),
    )
}

fn node_clip() -> RemoteClipIndex {
    RemoteClipIndex::new(
        vec![0.044_194_173_824_159_22; 512],
        mldata::flags::RUNTIME_RUST,
    )
}

#[test]
fn node_output_is_accepted_by_web_and_mobile_readers() {
    let merged = mldata::merge(None, Some(&node_face()), Some(&node_clip())).unwrap();
    assert_eq!(zod::parsed(&merged), Some((true, true)));
    mobile::accepts(&merged).unwrap();
    assert_eq!(merged["face"]["client"], json!(CLIENT_IDENTIFIER));
    assert_eq!(merged["face"]["flags"], json!(5));
    // Integers stay integers for Dart's `as int`.
    let text = serde_json::to_string(&merged).unwrap();
    assert!(
        text.contains(r#""width":1024"#) && text.contains(r#""height":768"#),
        "{text}"
    );

    // An empty face list (web convention) is accepted by mobile too.
    let empty = RemoteFaceIndex::new(10, 10, vec![], 1);
    let merged = mldata::merge(None, Some(&empty), Some(&node_clip())).unwrap();
    mobile::accepts(&merged).unwrap();
}

#[test]
fn every_fixture_is_accepted_by_the_mobile_reader() {
    // Including `invalid_face`: mobile defaults a missing `blur`
    // (kLapacianDefault) where zod rejects it.
    for name in FIXTURES {
        let raw = obj(serde_json::from_slice(&fixture(name)).unwrap());
        mobile::accepts(&raw).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn json_numbers_round_trip_bit_exact() {
    // Kept indexes are re-serialised from parsed JSON: every value must
    // come back identical (serde_json `float_roundtrip`).
    for name in FIXTURES {
        let raw: Value = serde_json::from_slice(&fixture(name)).unwrap();
        let again: Value = serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(raw, again, "{name}");
    }
}

#[test]
fn merge_keeps_unknown_keys_and_replaces_only_new_indexes() {
    let existing = RemoteMlData::from_json(&fixture("future_keys"), Some(9)).unwrap();
    // Nothing to run: merging nothing reproduces the object exactly.
    let same = mldata::merge(Some(&existing), None, None).unwrap();
    assert_eq!(same, existing.raw);

    let clip = node_clip();
    let merged = mldata::merge(Some(&existing), None, Some(&clip)).unwrap();
    assert_eq!(merged["pets"], existing.raw["pets"]);
    assert_eq!(merged["ocr"], existing.raw["ocr"]);
    // The kept face index, including its unknown nested key, is untouched.
    assert_eq!(merged["face"], existing.raw["face"]);
    assert_eq!(merged["face"]["newField"], json!(true));
    assert_eq!(merged["clip"], serde_json::to_value(&clip).unwrap());

    // Invalid known index: both replaced, unknown keys kept.
    let bad = RemoteMlData::from_json(&fixture("invalid_face"), None).unwrap();
    let merged = mldata::merge(Some(&bad), Some(&node_face()), Some(&clip)).unwrap();
    assert_eq!(merged["ocr"], json!({"keep": "me"}));
    assert_eq!(zod::parsed(&merged), Some((true, true)));
}

#[test]
fn merge_refuses_objects_mobile_would_reject() {
    let err = mldata::merge(None, Some(&node_face()), None).unwrap_err();
    assert!(matches!(err, MlDataError::Incomplete("clip")), "{err}");
    let err = mldata::merge(None, None, Some(&node_clip())).unwrap_err();
    assert!(matches!(err, MlDataError::Incomplete("face")), "{err}");
}

#[test]
fn gzip_and_encryption_round_trip() {
    crypto::init();
    let key = [7u8; 32];
    let raw = mldata::merge(None, Some(&node_face()), Some(&node_clip())).unwrap();

    let gz = mldata::gzip_json(&raw).unwrap();
    assert_eq!(&gz[..2], &[0x1f, 0x8b], "gzip magic");
    assert_eq!(
        serde_json::from_slice::<Value>(&mldata::gunzip_limited(&gz).unwrap()).unwrap(),
        Value::Object(raw.clone())
    );

    let (encrypted_data, decryption_header) = mldata::seal(&raw, &key).unwrap();
    let opened = mldata::open(&encrypted_data, &decryption_header, &key, Some(123)).unwrap();
    assert_eq!(opened.raw, raw);
    assert_eq!(opened.updated_at, Some(123));
    assert!(opened.plan().is_done());

    let wrong = [8u8; 32];
    assert!(matches!(
        mldata::open(&encrypted_data, &decryption_header, &wrong, None),
        Err(MlDataError::Crypto(_))
    ));
}

#[test]
fn app_fixtures_survive_seal_and_open() {
    crypto::init();
    let key = [1u8; 32];
    for name in FIXTURES {
        let raw = obj(serde_json::from_slice(&fixture(name)).unwrap());
        let (ct, hdr) = mldata::seal(&raw, &key).unwrap();
        let back = mldata::open(&ct, &hdr, &key, None).unwrap();
        assert_eq!(back.raw, raw, "{name}");
    }
}
