//! Search on a synthetic library: a real [`Inner`] (store, sealed records,
//! library index) filled with collections, files, `mldata` and location
//! tags, then queried through `Core` and the internal API.

use super::*;
use crate::client::Session;
use crate::library::{CollectionRec, FileRec};
use crate::metadata::VISIBILITY_HIDDEN;
use crate::ml::mldata::RemoteMlData;
use crate::ml::{FaceRef, Person};
use crate::{Account, Core};
use chrono::TimeZone;
use serde_json::json;

const USER: i64 = 1;

/// µs since the epoch of a local date at noon.
fn local(y: i32, m: u32, d: u32) -> i64 {
    Local
        .with_ymd_and_hms(y, m, d, 12, 0, 0)
        .earliest()
        .unwrap()
        .timestamp_micros()
}

struct Lib {
    core: Core,
    _dir: tempfile::TempDir,
}

impl Lib {
    fn inner(&self) -> &Inner {
        &self.core.inner
    }
}

fn session() -> Session {
    let ka = serde_json::from_value(json!({
        "kekSalt": "", "encryptedKey": "", "keyDecryptionNonce": "", "publicKey": "",
        "encryptedSecretKey": "", "secretKeyDecryptionNonce": "", "memLimit": 1, "opsLimit": 1
    }))
    .unwrap();
    Session {
        account: Account {
            user_id: USER,
            email: "a@example.org".into(),
            server_url: "http://127.0.0.1:1".into(),
        },
        token: "t".into(),
        master_key: vec![1; 32],
        secret_key: vec![2; 32],
        public_key: vec![3; 32],
        key_attributes: ka,
    }
}

fn collection(
    id: i64,
    name: &str,
    kind: &str,
    owned: bool,
    visibility: Option<i64>,
) -> CollectionRec {
    CollectionRec {
        id,
        key: vec![0; 32],
        name: name.into(),
        kind: kind.into(),
        owner_id: if owned { USER } else { 2 },
        owned,
        sub_type: None,
        visibility,
        updation_time: id,
    }
}

fn file(
    id: i64,
    title: &str,
    file_type: i64,
    taken: i64,
    loc: Option<(f64, f64)>,
    caption: Option<&str>,
) -> FileRec {
    let mut md = json!({"fileType": file_type, "title": title, "creationTime": taken,
                        "modificationTime": taken, "hash": format!("h{id}")});
    if let Some((lat, long)) = loc {
        md["latitude"] = json!(lat);
        md["longitude"] = json!(long);
    }
    let mut pm = serde_json::Map::new();
    if let Some(c) = caption {
        pm.insert("caption".into(), json!(c));
    }
    FileRec {
        id,
        owner_id: USER,
        key: vec![0; 32],
        file_header: String::new(),
        thumb_header: String::new(),
        file_size: Some(1),
        metadata: md,
        pub_magic: pm,
        magic: serde_json::Map::new(),
    }
}

const LISBON: (f64, f64) = (38.7223, -9.1393);
const PORTO: (f64, f64) = (41.1579, -8.6291);

/// The library:
///
/// | id | name                | type  | taken      | where  | collections         |
/// |----|---------------------|-------|------------|--------|---------------------|
/// | 1  | IMG_0001.jpg        | image | 2024-05-17 | Lisbon | Lisbon Trip         |
/// | 2  | VID_0002.mp4        | video | 2023-05-17 |        | uncategorized       |
/// | 3  | Screenshot_2024.png | image | 2024-06-01 |        | uncategorized       |
/// | 4  | DSC_0004.NEF        | image | 2022-12-25 | Porto  | Lisbon Trip         |
/// | 5  | live.heic           | live  | 2021-01-01 |        | Beach Day (shared)  |
/// | 6  | secret.jpg          | image | 2024-05-17 | Lisbon | Secret (hidden)     |
/// | 7  | both.jpg            | image | 2020-03-03 |        | Secret + uncategorized |
///
/// File 1 has the caption "Sunset at the river", file 6 "sunset secret".
fn library() -> Lib {
    crate::crypto::init();
    let dir = tempfile::tempdir().unwrap();
    let inner = Inner::open(dir.path().join("data"), dir.path().join("cache"), None).unwrap();
    inner.set_session(session()).unwrap();
    for c in [
        collection(10, "Uncategorized", "uncategorized", true, None),
        collection(11, "Lisbon Trip", "album", true, None),
        collection(12, "Secret", "album", true, Some(VISIBILITY_HIDDEN)),
        collection(13, "Beach Day", "album", false, None),
    ] {
        inner.save_collection(&c).unwrap();
    }
    let files = [
        (
            file(
                1,
                "IMG_0001.jpg",
                0,
                local(2024, 5, 17),
                Some(LISBON),
                Some("Sunset at the river"),
            ),
            vec![11],
        ),
        (
            file(2, "VID_0002.mp4", 1, local(2023, 5, 17), None, None),
            vec![10],
        ),
        (
            file(3, "Screenshot_2024.png", 0, local(2024, 6, 1), None, None),
            vec![10],
        ),
        (
            file(4, "DSC_0004.NEF", 0, local(2022, 12, 25), Some(PORTO), None),
            vec![11],
        ),
        (
            file(5, "live.heic", 2, local(2021, 1, 1), None, None),
            vec![13],
        ),
        (
            file(
                6,
                "secret.jpg",
                0,
                local(2024, 5, 17),
                Some(LISBON),
                Some("sunset secret"),
            ),
            vec![12],
        ),
        (
            file(7, "both.jpg", 0, local(2020, 3, 3), None, None),
            vec![12, 10],
        ),
    ];
    for (f, cols) in files {
        for c in cols {
            inner.save_file(&f, c).unwrap();
        }
    }
    {
        let st = inner.store();
        let tag = |name: &str, (lat, long): (f64, f64), radius: f64| {
            json!({"name": name, "radius": radius, "aSquare": 0.0, "bSquare": 0.0,
                   "centerPoint": {"latitude": lat, "longitude": long}})
        };
        st.put_entity("location", "tag-lisbon", 1, &tag("Lisbon", LISBON, 5.0))
            .unwrap();
        st.put_entity(
            "location",
            "tag-porto",
            1,
            &tag("Porto", (41.16, -8.63), 2.0),
        )
        .unwrap();
        st.put_entity("location", "tag-null", 1, &tag("Nowhere", (0.5, 0.5), 1.0))
            .unwrap();
        st.put_entity("cgroup", "p1", 1, &json!({"name": "Lisbon"}))
            .unwrap();
    }
    inner.rebuild_index().unwrap();
    let lib = Lib {
        core: Core { inner },
        _dir: dir,
    };
    // Build the caches now so the sync calls do not race the background.
    let _ = extras(lib.inner(), None);
    lib
}

fn ids(items: &[TimelineItem]) -> Vec<i64> {
    items.iter().map(|i| i.id).collect()
}

fn rows(s: &[SearchSuggestion]) -> Vec<(SearchKind, &str, &str, u64)> {
    s.iter()
        .map(|s| (s.kind, s.key.as_str(), s.label.as_str(), s.count))
        .collect()
}

#[test]
fn suggestions_cover_every_kind_and_rank_like_ente() {
    let lib = library();
    let c = &lib.core;
    // Place before album (sections: magic, people, places, moments,
    // albums, files); the hidden file in Lisbon is not counted.
    assert_eq!(
        rows(&c.search_suggestions("lis".into(), 10)),
        vec![
            (SearchKind::Place, "tag-lisbon", "Lisbon", 1),
            (SearchKind::Album, "11", "Lisbon Trip", 2),
        ]
    );
    let s = c.search_suggestions("lis".into(), 10);
    assert_eq!(s[0].cover_file_id, Some(1));
    assert_eq!(s[1].cover_file_id, Some(1));
    // Captions (the hidden file's caption is not counted).
    assert_eq!(
        rows(&c.search_suggestions("sunset".into(), 10)),
        vec![(SearchKind::Caption, "sunset", "sunset", 1)]
    );
    // File-like queries put files first.
    assert_eq!(
        rows(&c.search_suggestions("IMG".into(), 10)),
        vec![(SearchKind::FileName, "IMG", "IMG", 1)]
    );
    // A year: moments first, then the file name that contains it.
    assert_eq!(
        rows(&c.search_suggestions("2024".into(), 10)),
        vec![
            (SearchKind::Date, "y:2024", "2024", 2),
            (SearchKind::FileName, "2024", "2024", 1),
        ]
    );
    assert_eq!(
        rows(&c.search_suggestions("may".into(), 10)),
        vec![(SearchKind::Date, "m:05", "May", 2)]
    );
    assert_eq!(
        rows(&c.search_suggestions("17 May".into(), 10)),
        vec![(SearchKind::Date, "md:05-17", "17 May", 2)]
    );
    assert_eq!(
        rows(&c.search_suggestions("17/05/2024".into(), 10)),
        vec![(SearchKind::Date, "ymd:2024-05-17", "17 May 2024", 1)]
    );
    assert_eq!(
        rows(&c.search_suggestions("may 2023".into(), 10)),
        vec![(SearchKind::Date, "ym:2023-05", "May 2023", 1)]
    );
    // File types.
    assert_eq!(
        rows(&c.search_suggestions("photo".into(), 10)),
        vec![(SearchKind::FileType, "image", "Photos", 4)]
    );
    assert_eq!(
        rows(&c.search_suggestions("live".into(), 10)),
        vec![
            (SearchKind::FileType, "live", "Live Photos", 1),
            (SearchKind::FileName, "live", "live", 1),
        ]
    );
    assert_eq!(
        rows(&c.search_suggestions("raw".into(), 10)),
        vec![(SearchKind::FileType, "raw", "RAW", 1)]
    );
    assert_eq!(
        rows(&c.search_suggestions("screen".into(), 10)),
        vec![
            (SearchKind::FileType, "screenshot", "Screenshots", 1),
            (SearchKind::FileName, "screen", "screen", 1),
        ]
    );
    assert_eq!(
        rows(&c.search_suggestions(".nef".into(), 10)),
        vec![
            (SearchKind::FileName, ".nef", ".nef", 1),
            (SearchKind::FileType, "ext:NEF", "NEF", 1),
        ]
    );
    // Shared albums are searchable; hidden ones are not.
    assert_eq!(
        rows(&c.search_suggestions("beach".into(), 10)),
        vec![(SearchKind::Album, "13", "Beach Day", 1)]
    );
    assert!(c.search_suggestions("secret".into(), 10).is_empty());
    // A tag with no files is not suggested; nothing for blank queries.
    assert!(c.search_suggestions("nowhere".into(), 10).is_empty());
    assert!(c.search_suggestions("  ".into(), 10).is_empty());
    // Limit.
    assert_eq!(c.search_suggestions("lis".into(), 1).len(), 1);
    // No text model: no magic row.
    assert!(
        c.search_suggestions("lisbon".into(), 0)
            .iter()
            .all(|s| s.kind != SearchKind::Magic)
    );
}

#[test]
fn results_of_every_kind_respect_hidden_collections() {
    let lib = library();
    let inner = lib.inner();
    let r = |k: SearchKind, key: &str| ids(&results(inner, k, key));
    assert_eq!(r(SearchKind::Place, "tag-lisbon"), vec![1]);
    assert_eq!(r(SearchKind::Place, "tag-porto"), vec![4]);
    assert_eq!(r(SearchKind::Place, "tag-null"), Vec::<i64>::new());
    assert_eq!(r(SearchKind::Place, "city:Lisbon"), Vec::<i64>::new());
    assert_eq!(r(SearchKind::Caption, "SUNSET"), vec![1]);
    assert_eq!(r(SearchKind::FileName, "jpg"), vec![1, 7]);
    assert_eq!(r(SearchKind::FileName, "0004"), vec![4]);
    assert_eq!(r(SearchKind::FileName, "5"), vec![5], "or the file id");
    assert_eq!(r(SearchKind::Date, "y:2024"), vec![3, 1]);
    assert_eq!(r(SearchKind::Date, "m:05"), vec![1, 2]);
    assert_eq!(r(SearchKind::Date, "md:05-17"), vec![1, 2]);
    assert_eq!(r(SearchKind::Date, "ymd:2024-05-17"), vec![1]);
    assert_eq!(r(SearchKind::Date, "ym:2022-12"), vec![4]);
    assert_eq!(r(SearchKind::Date, "bogus"), Vec::<i64>::new());
    assert_eq!(r(SearchKind::FileType, "image"), vec![3, 1, 4, 7]);
    assert_eq!(r(SearchKind::FileType, "video"), vec![2]);
    assert_eq!(r(SearchKind::FileType, "live"), vec![5]);
    assert_eq!(r(SearchKind::FileType, "screenshot"), vec![3]);
    assert_eq!(r(SearchKind::FileType, "raw"), vec![4]);
    assert_eq!(r(SearchKind::FileType, "ext:JPG"), vec![1, 7]);
    assert_eq!(r(SearchKind::FileType, "nope"), Vec::<i64>::new());
    assert_eq!(r(SearchKind::Album, "12"), Vec::<i64>::new(), "hidden");
    let album = results(inner, SearchKind::Album, "11");
    assert_eq!(ids(&album), vec![1, 4]);
    assert!(album.iter().all(|i| i.collection_id == 11));
    assert_eq!(r(SearchKind::Album, "13"), vec![5]);
    assert_eq!(r(SearchKind::Person, "p1"), Vec::<i64>::new());
    // No text model: magic search is empty, not an error.
    assert_eq!(r(SearchKind::Magic, "sunset"), Vec::<i64>::new());
    let got = crate::runtime()
        .block_on(lib.core.magic_search("sunset".into(), 10))
        .unwrap();
    assert!(got.is_empty());
    // Through the async API too.
    let got = crate::runtime()
        .block_on(lib.core.search_results(SearchKind::Caption, "river".into()))
        .unwrap();
    assert_eq!(ids(&got), vec![1]);
    // The hidden-only file never shows up.
    for k in [
        SearchKind::Caption,
        SearchKind::FileName,
        SearchKind::Place,
        SearchKind::Date,
        SearchKind::FileType,
    ] {
        for key in ["secret", "tag-lisbon", "ymd:2024-05-17", "image"] {
            assert!(!r(k, key).contains(&6), "{k:?} {key}");
        }
    }
}

#[test]
fn discovery_lists_places_types_and_albums() {
    let lib = library();
    let d = lib.core.search_discovery();
    assert_eq!(
        rows(&d),
        vec![
            (SearchKind::Place, "tag-lisbon", "Lisbon", 1),
            (SearchKind::Place, "tag-porto", "Porto", 1),
            (SearchKind::FileType, "image", "Photos", 4),
            (SearchKind::FileType, "video", "Videos", 1),
            (SearchKind::FileType, "live", "Live Photos", 1),
            (SearchKind::FileType, "screenshot", "Screenshots", 1),
            (SearchKind::FileType, "raw", "RAW", 1),
            (SearchKind::FileType, "ext:JPG", "JPGs", 2),
            (SearchKind::FileType, "ext:HEIC", "HEICs", 1),
            (SearchKind::FileType, "ext:MP4", "MP4s", 1),
            (SearchKind::FileType, "ext:NEF", "NEFs", 1),
            (SearchKind::Album, "11", "Lisbon Trip", 2),
            (SearchKind::Album, "13", "Beach Day", 1),
        ]
    );
}

#[test]
fn library_changes_are_picked_up() {
    let lib = library();
    let inner = lib.inner();
    let before = search_data(inner, None);
    inner
        .save_file(
            &file(
                8,
                "IMG_0008.jpg",
                0,
                local(2025, 1, 1),
                Some(LISBON),
                Some("river view"),
            ),
            11,
        )
        .unwrap();
    inner.rebuild_index().unwrap();
    // The blocking paths rebuild at once.
    assert_eq!(
        ids(&results(inner, SearchKind::Caption, "river")),
        vec![8, 1]
    );
    assert_eq!(
        ids(&results(inner, SearchKind::Place, "tag-lisbon")),
        vec![8, 1]
    );
    assert!(!Arc::ptr_eq(&before, &search_data(inner, None)));
    // invalidate() keeps serving the old extras until rebuilt.
    invalidate(inner);
    assert!(state(inner).extras.is_some());
    assert_eq!(ids(&results(inner, SearchKind::Caption, "view")), vec![8]);
}

fn clip_row(inner: &Inner, id: i64, e: &[f64]) {
    let raw = json!({"clip": {"version": 1, "client": "test", "embedding": e}});
    let d = RemoteMlData::from_json(raw.to_string().as_bytes(), Some(1)).unwrap();
    inner.store().put_ml_data(id, &d).unwrap();
}

/// A unit vector at `cos` similarity to the first axis, in 512 dims.
fn at(cos: f64, tilt: usize) -> Vec<f64> {
    let mut v = vec![0.0; 512];
    v[0] = cos;
    v[1 + tilt] = (1.0 - cos * cos).sqrt();
    v
}

#[test]
fn clip_matches_threshold_order_and_invalidate() {
    let lib = library();
    let inner = lib.inner();
    let mut t = vec![0f32; 512];
    t[0] = 1.0;
    assert!(clip_matches(inner, &t, MIN_SIMILARITY).is_empty());
    clip_row(inner, 1, &at(0.30, 0));
    clip_row(inner, 2, &at(0.176, 1));
    clip_row(inner, 3, &at(0.174, 2));
    clip_row(inner, 6, &at(0.5, 3));
    clip_row(inner, 7, &[0.5; 3]); // another dimension: skipped
    // Cached until invalidated.
    assert!(clip_matches(inner, &t, MIN_SIMILARITY).is_empty());
    invalidate(inner);
    let m = clip_matches(inner, &t, MIN_SIMILARITY);
    assert_eq!(m.iter().map(|m| m.0).collect::<Vec<_>>(), vec![6, 1, 2]);
    assert!((m[0].1 - 0.5).abs() < 1e-6 && (m[2].1 - 0.176).abs() < 1e-6);
    let m = clip_matches(inner, &t, 0.25);
    assert_eq!(m.iter().map(|m| m.0).collect::<Vec<_>>(), vec![6, 1]);
    // A file that loses its CLIP index drops out after invalidate.
    inner.store().delete_ml_data(6).unwrap();
    invalidate(inner);
    assert_eq!(clip_matches(inner, &t, 0.25)[0].0, 1);
    // The embeddings are sealed at rest.
    let blob: Vec<u8> = inner
        .store()
        .conn()
        .query_row("SELECT sealed FROM ml_data WHERE file_id = 1", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(!String::from_utf8_lossy(&blob).contains("embedding"));
    // No text model in this build/data dir: no text embedding.
    assert!(text_embedding_arc(inner, "sunset").is_none());
    assert!(!magic_available(inner));
}

#[test]
fn magic_prompt_results_follow_the_prompt() {
    let lib = library();
    let inner = lib.inner();
    let d = search_data(inner, None);
    let p = clip::prompt_for("selfie").unwrap(); // recent first, videos shown
    let items = prompt_items(&d, p, &[2, 6, 4, 1]);
    assert_eq!(ids(&items), vec![1, 2, 4], "visible only, newest first");
    let p = clip::prompt_for("photo of mountains or hills").unwrap(); // best first
    assert_eq!(ids(&prompt_items(&d, p, &[4, 1, 2])), vec![4, 1, 2]);
    let p = clip::prompt_for("identity documents and cards").unwrap(); // no videos
    assert_eq!(ids(&prompt_items(&d, p, &[2, 1])), vec![1]);
    // As discovery rows.
    let src = Sources {
        people: &[],
        person_files: &|_: &str| Vec::new(),
        tags: &[],
        magic: false,
        today: Local::now().date_naive(),
    };
    let prompts: PromptMatches = vec![(0, vec![2, 1]), (13, vec![]), (14, vec![4, 1])];
    let rows_: Vec<_> = discovery_for(&d, None, &src, Some(&prompts))
        .into_iter()
        .filter(|s| s.kind == SearchKind::Magic)
        .map(|s| (s.key, s.label, s.count, s.cover_file_id))
        .collect();
    assert_eq!(
        rows_,
        vec![
            (
                "identity documents and cards".into(),
                "Identity".into(),
                1,
                Some(1)
            ),
            (
                "photo of mountains or hills".into(),
                "Hills".into(),
                2,
                Some(4)
            ),
        ]
    );
}

#[test]
fn people_are_matched_by_name_and_ranked() {
    let lib = library();
    let d = search_data(lib.inner(), None);
    let face = |file_id: i64| FaceRef {
        face_id: format!("{file_id}_0_0_1_1"),
        file_id,
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
        score: 0.9,
        blur: 100.0,
    };
    let person = |id: &str, name: Option<&str>, cover: i64| Person {
        id: id.into(),
        name: name.map(Into::into),
        is_hidden: false,
        file_count: 0,
        cover: Some(face(cover)),
        birth_date: None,
    };
    let people = vec![
        person("p-anna", Some("Joanna"), 1),
        person("p-ana", Some("Ana"), 6), // cover on a hidden file
        person("p-hanna", Some("Hanna"), 3),
        person("cluster:9", None, 4),
    ];
    let files = |id: &str| -> Vec<i64> {
        match id {
            "p-anna" => vec![1, 2, 3],
            "p-ana" => vec![6, 4],
            "p-hanna" => vec![6],
            _ => vec![4, 5],
        }
    };
    let src = Sources {
        people: &people,
        person_files: &files,
        tags: &[],
        magic: false,
        today: Local::now().date_naive(),
    };
    let s = suggestions_for(&d, None, &src, "an", 0);
    // Word starts first ("Ana"), then by files; "Hanna" only has a hidden
    // file; a cover face on a hidden file is not used.
    let got: Vec<_> = s
        .iter()
        .map(|s| {
            (
                s.kind,
                s.key.as_str(),
                s.count,
                s.cover_file_id,
                s.cover_face_id.clone(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (SearchKind::Person, "p-ana", 1, Some(4), None),
            (
                SearchKind::Person,
                "p-anna",
                3,
                Some(1),
                Some("1_0_0_1_1".into())
            ),
        ]
    );
    // Discovery: named people first (as `people()` orders them), unnamed
    // clusters with a blank label.
    let dsc = discovery_for(&d, None, &src, None);
    let got: Vec<_> = dsc
        .iter()
        .filter(|s| s.kind == SearchKind::Person)
        .map(|s| (s.key.as_str(), s.label.as_str(), s.count))
        .collect();
    assert_eq!(
        got,
        vec![
            ("p-anna", "Joanna", 3),
            ("p-ana", "Ana", 1),
            ("cluster:9", "", 2)
        ]
    );
}

#[test]
fn query_intent_orders_sections() {
    let today = Local::now().date_naive();
    assert_eq!(section_order("img_1", today)[0], Section::Files);
    assert_eq!(section_order("dsc0001", today)[0], Section::Files);
    assert_eq!(section_order("20240517_1", today)[0], Section::Files);
    assert_eq!(section_order("heic", today)[0], Section::Files);
    assert_eq!(section_order("videos", today)[0], Section::Files);
    assert_eq!(section_order("june", today)[0], Section::Moments);
    assert_eq!(section_order("2020", today)[0], Section::Moments);
    assert_eq!(section_order("beach", today), DEFAULT_ORDER.to_vec());
    assert!(word_start("lisbon trip", "trip"));
    assert!(word_start("img_0001", "0001"));
    assert!(!word_start("joanna", "an"));
    assert_eq!(extension("a.tar.gz"), Some("gz"));
    assert_eq!(extension("noext"), None);
    assert_eq!(extension("dot."), None);
}
