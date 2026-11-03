//! End-to-end tests of the Immich source against a mock Immich server.
//!
//! The mock speaks both the current API (v3.2+: `filter` + cursor) and the
//! v1.133–v3.1 API (top-level `visibility` + page numbers, album contents in
//! `GET /albums/{id}`), and deliberately returns some assets a real server
//! would filter out (trashed, hidden motion parts, a partner's asset) so the
//! client-side checks are exercised too.

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use photovault_import::immich::{ImmichOptions, ImmichSource};
use photovault_import::item::{ImportItem, Source};
use serde_json::{Value, json};

const KEY: &str = "test-api-key";
const ME: &str = "user-me";
const PAGE: usize = 3;

/// Mock server state.
struct Mock {
    major: u32,
    minor: u32,
    /// Assets the search lists (whatever the filter says).
    listed: Vec<Value>,
    /// Assets only reachable by id (motion parts).
    by_id: Vec<Value>,
    /// Every request: "METHOD /path?query".
    log: Mutex<Vec<String>>,
    /// Paths that fail once with 503 before succeeding.
    fail_once: Mutex<HashSet<String>>,
}

type Shared = Arc<Mock>;

fn asset(id: &str, kind: &str, name: &str, extra: Value) -> Value {
    let mut a = json!({
        "id": id, "type": kind, "ownerId": ME, "originalFileName": name,
        "originalPath": format!("/data/library/{ME}/{name}"),
        "fileCreatedAt": "2023-03-04T05:06:07.000Z",
        "fileModifiedAt": "2023-03-05T00:00:00.000Z",
        "localDateTime": "2023-03-04T05:06:07.000Z",
        "isFavorite": false, "isArchived": false, "isTrashed": false, "isOffline": false,
        "visibility": "timeline", "livePhotoVideoId": null, "width": null, "height": null,
        "checksum": "x", "thumbhash": null, "duration": null,
        "exifInfo": {}, "people": []
    });
    for (k, v) in extra.as_object().expect("object") {
        a[k] = v.clone();
    }
    a
}

fn library() -> (Vec<Value>, Vec<Value>) {
    let listed = vec![
        asset(
            "img1",
            "IMAGE",
            "IMG_0001.JPG",
            json!({
                "fileCreatedAt": "2024-06-01T14:00:00.000Z",
                "fileModifiedAt": "2024-06-02T00:00:00.000Z",
                // Wall clock in Berlin (UTC+2): must not be read as UTC.
                "localDateTime": "2024-06-01T16:00:00.000Z",
                "width": 4000, "height": 3000,
                "exifInfo": {
                    "dateTimeOriginal": "2024-06-01T14:00:00.000Z", "timeZone": "Europe/Berlin",
                    "latitude": 52.5163, "longitude": 13.3777, "description": "  Brandenburg Gate ",
                    "exifImageWidth": 4000, "exifImageHeight": 3000
                },
                "people": [
                    {"id": "p1", "name": "Ada", "isHidden": false},
                    {"id": "p2", "name": "", "isHidden": false},
                    {"id": "p3", "name": "Secret", "isHidden": true}
                ]
            }),
        ),
        asset(
            "live2",
            "IMAGE",
            "IMG_0002.HEIC",
            json!({
                "livePhotoVideoId": "vid2",
                "localDateTime": "2023-03-04T14:06:07.000Z",
                "exifInfo": {"timeZone": "UTC+9", "exifImageWidth": 4032, "exifImageHeight": 3024, "orientation": "6"}
            }),
        ),
        // A real server hides this; the mock lists it to test the client check.
        asset(
            "vid2",
            "VIDEO",
            "IMG_0002.MOV",
            json!({"visibility": "hidden"}),
        ),
        asset(
            "arch3",
            "IMAGE",
            "IMG_0003.JPG",
            json!({"visibility": "archive", "isArchived": true}),
        ),
        asset(
            "fav4",
            "VIDEO",
            "VID_0004.MP4",
            json!({"isFavorite": true, "width": 1920, "height": 1080}),
        ),
        asset(
            "trash5",
            "IMAGE",
            "IMG_0005.JPG",
            json!({"isTrashed": true}),
        ),
        asset(
            "partner6",
            "IMAGE",
            "IMG_0006.JPG",
            json!({"ownerId": "someone-else"}),
        ),
        asset(
            "mp7",
            "IMAGE",
            "PXL_0007.MP.jpg",
            json!({"livePhotoVideoId": "mpvid7"}),
        ),
        asset("audio8", "AUDIO", "voice.m4a", json!({})),
    ];
    let by_id = vec![
        asset(
            "vid2",
            "VIDEO",
            "IMG_0002.MOV",
            json!({"visibility": "hidden"}),
        ),
        asset(
            "mpvid7",
            "VIDEO",
            "PXL_0007.MP.mp4",
            json!({"visibility": "hidden", "originalPath": format!("/data/encoded-video/{ME}/ab/cd/mpvid7-MP.mp4")}),
        ),
    ];
    (listed, by_id)
}

/// Album id, name, member asset ids.
const ALBUMS: &[(&str, &str, &[&str])] = &[
    ("alb-holiday", "Holiday", &["img1", "live2", "partner6"]),
    ("alb-shared", "Shared trip", &["img1", "fav4"]),
];

fn album_json(i: usize, with_assets: bool) -> Value {
    let (id, name, ids) = ALBUMS[i];
    let mut a = json!({"id": id, "albumName": name, "assetCount": ids.len(), "shared": i == 1,
        "albumUsers": [{"role": "owner", "user": {"id": if i == 1 { "someone-else" } else { ME }}}]});
    if with_assets {
        a["assets"] = json!(ids.iter().map(|id| json!({"id": id})).collect::<Vec<_>>());
    }
    a
}

fn authorised(h: &HeaderMap) -> bool {
    h.get("x-api-key").and_then(|v| v.to_str().ok()) == Some(KEY)
}

fn log(m: &Mock, line: String) {
    m.log.lock().expect("lock").push(line);
}

fn qs(q: &std::collections::HashMap<String, String>) -> String {
    let mut v: Vec<_> = q.iter().map(|(k, v)| format!("{k}={v}")).collect();
    v.sort();
    if v.is_empty() {
        String::new()
    } else {
        format!("?{}", v.join("&"))
    }
}

async fn version(State(m): State<Shared>) -> Json<Value> {
    Json(json!({"major": m.major, "minor": m.minor, "patch": 0, "prerelease": null}))
}

async fn me(State(m): State<Shared>, h: HeaderMap) -> Response {
    log(&m, "GET /users/me".into());
    if !authorised(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!({"id": ME, "email": "me@example.com", "name": "Me"})).into_response()
}

async fn albums(
    State(m): State<Shared>,
    h: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    log(&m, format!("GET /albums{}", qs(&q)));
    if !authorised(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let v3 = m.major >= 3;
    let list: Vec<Value> = if v3 {
        vec![album_json(0, false), album_json(1, false)]
    } else if q.get("shared").map(String::as_str) == Some("true") {
        vec![album_json(1, false)]
    } else {
        vec![album_json(0, false)]
    };
    Json(Value::Array(list)).into_response()
}

async fn album(State(m): State<Shared>, h: HeaderMap, UrlPath(id): UrlPath<String>) -> Response {
    log(&m, format!("GET /albums/{id}"));
    if !authorised(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match ALBUMS.iter().position(|a| a.0 == id) {
        Some(i) => Json(album_json(i, m.major < 3)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn search(State(m): State<Shared>, h: HeaderMap, Json(body): Json<Value>) -> Response {
    log(&m, format!("POST /search/metadata {body}"));
    if !authorised(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let filter_mode = m.major > 3 || (m.major == 3 && m.minor >= 2);
    // Album contents.
    let album_id = if filter_mode {
        body.pointer("/filter/albumIds/any/0")
            .and_then(Value::as_str)
    } else {
        body.pointer("/albumIds/0").and_then(Value::as_str)
    };
    let items: Vec<Value> = if let Some(aid) = album_id {
        let ids = ALBUMS
            .iter()
            .find(|a| a.0 == aid)
            .map(|a| a.2)
            .unwrap_or(&[]);
        m.listed
            .iter()
            .filter(|a| ids.contains(&a["id"].as_str().unwrap_or("")))
            .cloned()
            .collect()
    } else if filter_mode {
        assert_eq!(
            body.pointer("/filter/visibility/in"),
            Some(&json!(["timeline", "archive"])),
            "{body}"
        );
        assert_eq!(body["withExif"], json!(true));
        assert_eq!(body["withPeople"], json!(true));
        assert!(
            body.get("page").is_none() && body.get("visibility").is_none(),
            "{body}"
        );
        m.listed.clone()
    } else {
        let vis = body["visibility"]
            .as_str()
            .expect("visibility in v2 mode")
            .to_owned();
        assert!(body.get("filter").is_none(), "{body}");
        // A v2 server filters by visibility; keep the deliberately wrong
        // entries (trashed, partner, hidden motion) in the timeline listing.
        m.listed
            .iter()
            .filter(|a| {
                let v = a["visibility"].as_str().unwrap_or("");
                if vis == "timeline" {
                    v != "archive"
                } else {
                    v == vis
                }
            })
            .cloned()
            .collect()
    };
    let start = if filter_mode {
        body["cursor"]
            .as_str()
            .map_or(0, |c| c.trim_start_matches("c").parse().expect("cursor"))
    } else {
        (body["page"].as_u64().unwrap_or(1) as usize - 1) * PAGE
    };
    let page: Vec<Value> = items.iter().skip(start).take(PAGE).cloned().collect();
    let more = start + PAGE < items.len();
    let (next_cursor, next_page) = match (more, filter_mode) {
        (false, _) => (Value::Null, Value::Null),
        (true, true) => (json!(format!("c{}", start + PAGE)), Value::Null),
        (true, false) => (Value::Null, json!(((start / PAGE) + 2).to_string())),
    };
    Json(json!({
        "albums": {"count": 0, "facets": [], "items": [], "total": 0},
        "assets": {"count": page.len(), "facets": [], "items": page, "nextCursor": next_cursor, "nextPage": next_page, "total": items.len()}
    }))
    .into_response()
}

async fn statistics(
    State(m): State<Shared>,
    h: HeaderMap,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    log(&m, format!("GET /assets/statistics{}", qs(&q)));
    if !authorised(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let n = match q.get("visibility").map(String::as_str) {
        Some("timeline") => 5,
        Some("archive") => 1,
        _ => 0,
    };
    Json(json!({"images": n, "videos": 0, "total": n})).into_response()
}

async fn asset_get(
    State(m): State<Shared>,
    h: HeaderMap,
    UrlPath(id): UrlPath<String>,
) -> Response {
    log(&m, format!("GET /assets/{id}"));
    if !authorised(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match m
        .by_id
        .iter()
        .chain(m.listed.iter())
        .find(|a| a["id"] == json!(id))
    {
        Some(a) => Json(a.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn original(State(m): State<Shared>, h: HeaderMap, UrlPath(id): UrlPath<String>) -> Response {
    let path = format!("/assets/{id}/original");
    log(&m, format!("GET {path}"));
    if !authorised(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if m.fail_once.lock().expect("lock").remove(&path) {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    (
        [(header::CONTENT_TYPE, "application/octet-stream")],
        format!("original:{id}"),
    )
        .into_response()
}

async fn thumbnail(
    State(m): State<Shared>,
    h: HeaderMap,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<std::collections::HashMap<String, String>>,
) -> Response {
    log(&m, format!("GET /assets/{id}/thumbnail{}", qs(&q)));
    if !authorised(&h) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if id == "fav4" {
        return StatusCode::NOT_FOUND.into_response();
    }
    (
        [(header::CONTENT_TYPE, "image/jpeg")],
        format!("preview:{id}"),
    )
        .into_response()
}

async fn serve(major: u32, minor: u32) -> (String, Shared) {
    let (listed, by_id) = library();
    let m = Arc::new(Mock {
        major,
        minor,
        listed,
        by_id,
        log: Mutex::new(Vec::new()),
        fail_once: Mutex::new(HashSet::from(["/assets/arch3/original".to_owned()])),
    });
    let app = Router::new()
        .route("/api/server/version", get(version))
        .route("/api/users/me", get(me))
        .route("/api/albums", get(albums))
        .route("/api/albums/{id}", get(album))
        .route("/api/search/metadata", post(search))
        .route("/api/assets/statistics", get(statistics))
        .route("/api/assets/{id}", get(asset_get))
        .route("/api/assets/{id}/original", get(original))
        .route("/api/assets/{id}/thumbnail", get(thumbnail))
        .with_state(m.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    (format!("http://{addr}"), m)
}

/// Everything about an item, with paths made relative to the temp dir and
/// file contents read, so it can be compared exactly.
fn describe(item: &ImportItem, tmp: &Path) -> Value {
    let rel = |p: &Path| {
        p.strip_prefix(tmp)
            .expect("inside temp dir")
            .to_string_lossy()
            .replace('\\', "/")
    };
    let read = |p: &Path| std::fs::read_to_string(p).expect("read");
    json!({
        "id": item.source_id,
        "kind": format!("{:?}", item.kind),
        "primary": [item.primary.file_name, rel(&item.primary.path), read(&item.primary.path)],
        "live": item.live_video.as_ref().map(|v| json!([v.file_name, rel(&v.path), read(&v.path)])),
        "thumb": item.thumbnail.as_ref().map(|t| json!([rel(t), read(t)])),
        "created": item.creation_time_us,
        "modified": item.modification_time_us,
        "gps": item.location.map(|g| json!([g.latitude, g.longitude])),
        "fav": item.favorite,
        "archived": item.archived,
        "hidden": item.hidden,
        "desc": item.description,
        "albums": item.albums,
        "people": item.people,
        "dims": item.dimensions,
    })
}

fn expected() -> Vec<Value> {
    vec![
        json!({
            "id": "img1", "kind": "Image",
            "primary": ["IMG_0001.JPG", "img1/original/IMG_0001.JPG", "original:img1"],
            "live": null, "thumb": ["img1/preview.jpg", "preview:img1"],
            "created": 1_717_250_400_000_000i64, "modified": 1_717_286_400_000_000i64,
            "gps": [52.5163, 13.3777], "fav": false, "archived": false, "hidden": false,
            "desc": "Brandenburg Gate", "albums": ["Holiday", "Shared trip"], "people": ["Ada"],
            "dims": [4000, 3000]
        }),
        json!({
            "id": "live2", "kind": "LivePhoto",
            "primary": ["IMG_0002.HEIC", "live2/original/IMG_0002.HEIC", "original:live2"],
            "live": ["IMG_0002.MOV", "live2/motion/IMG_0002.MOV", "original:vid2"],
            "thumb": ["live2/preview.jpg", "preview:live2"],
            // fileCreatedAt (an instant), not localDateTime (UTC+9 wall clock).
            "created": 1_677_906_367_000_000i64, "modified": 1_677_974_400_000_000i64,
            "gps": null, "fav": false, "archived": false, "hidden": false,
            "desc": null, "albums": ["Holiday"], "people": [],
            // EXIF 4032x3024 with orientation 6 (rotated 90°).
            "dims": [3024, 4032]
        }),
        json!({
            "id": "arch3", "kind": "Image",
            "primary": ["IMG_0003.JPG", "arch3/original/IMG_0003.JPG", "original:arch3"],
            "live": null, "thumb": ["arch3/preview.jpg", "preview:arch3"],
            "created": 1_677_906_367_000_000i64, "modified": 1_677_974_400_000_000i64,
            "gps": null, "fav": false, "archived": true, "hidden": false,
            "desc": null, "albums": [], "people": [], "dims": null
        }),
        json!({
            "id": "fav4", "kind": "Video",
            "primary": ["VID_0004.MP4", "fav4/original/VID_0004.MP4", "original:fav4"],
            "live": null, "thumb": null,
            "created": 1_677_906_367_000_000i64, "modified": 1_677_974_400_000_000i64,
            "gps": null, "fav": true, "archived": false, "hidden": false,
            "desc": null, "albums": ["Shared trip"], "people": [], "dims": [1920, 1080]
        }),
        json!({
            "id": "mp7", "kind": "Image",
            "primary": ["PXL_0007.MP.jpg", "mp7/original/PXL_0007.MP.jpg", "original:mp7"],
            "live": null, "thumb": ["mp7/preview.jpg", "preview:mp7"],
            "created": 1_677_906_367_000_000i64, "modified": 1_677_974_400_000_000i64,
            "gps": null, "fav": false, "archived": false, "hidden": false,
            "desc": null, "albums": [], "people": [], "dims": null
        }),
    ]
}

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

async fn import_all(
    src: &mut ImmichSource,
    skip: &(dyn for<'a> Fn(&'a str) -> bool + Sync),
) -> Vec<Value> {
    let tmp = src.temp_path().to_owned();
    let mut out = Vec::new();
    while let Some(item) = src.next(skip).await.expect("next") {
        out.push(describe(&item, &tmp));
        let dir = tmp.join(&item.source_id);
        assert!(dir.exists());
        src.done(&item).await.expect("done");
        assert!(!dir.exists(), "done() must remove the item's temp files");
    }
    out
}

fn requests(m: &Mock) -> Vec<String> {
    m.log.lock().expect("lock").clone()
}

#[tokio::test]
async fn current_api_maps_everything() {
    let (url, mock) = serve(3, 4).await;
    let dir = scratch();
    let opts = ImmichOptions {
        temp_dir: Some(dir.path().to_owned()),
        ..Default::default()
    };
    let mut src = ImmichSource::new(&url, KEY, opts).expect("source");
    assert_eq!(src.total().await.expect("total"), Some(6));
    let got = import_all(&mut src, &|_| false).await;
    assert_eq!(got, expected());
    assert!(src.failures().is_empty());
    assert_eq!(src.server_version(), Some((3, 4, 0)));

    let log = requests(&mock);
    // Search is paginated with cursors (9 listed assets, 3 per page).
    let searches: Vec<_> = log
        .iter()
        .filter(|l| l.starts_with("POST /search/metadata") && !l.contains("albumIds"))
        .collect();
    assert_eq!(searches.len(), 3, "{searches:#?}");
    assert!(searches[1].contains(r#""cursor":"c3""#) && searches[2].contains(r#""cursor":"c6""#));
    // Excluded assets are never downloaded; the motion part is downloaded only as part of the Live Photo.
    for id in ["trash5", "partner6", "audio8", "mpvid7"] {
        assert!(
            !log.iter()
                .any(|l| l.contains(&format!("/assets/{id}/original"))),
            "{id} downloaded"
        );
    }
    assert_eq!(
        log.iter()
            .filter(|l| l.as_str() == "GET /assets/vid2/original")
            .count(),
        1
    );
    // The transient 503 was retried.
    assert_eq!(
        log.iter()
            .filter(|l| l.as_str() == "GET /assets/arch3/original")
            .count(),
        2
    );
    assert!(
        log.iter()
            .any(|l| l == "GET /assets/img1/thumbnail?size=preview")
    );
    // v3 lists all albums with one call.
    assert_eq!(
        log.iter().filter(|l| l.starts_with("GET /albums")).count(),
        1
    );

    drop(src);
    assert_eq!(
        std::fs::read_dir(dir.path()).expect("read").count(),
        0,
        "temp dir removed on drop"
    );
}

#[tokio::test]
async fn skip_prevents_downloads() {
    let (url, mock) = serve(3, 4).await;
    let dir = scratch();
    let opts = ImmichOptions {
        temp_dir: Some(dir.path().to_owned()),
        thumbnails: false,
        ..Default::default()
    };
    let mut src = ImmichSource::new(&url, KEY, opts).expect("source");
    let asked = Mutex::new(Vec::new());
    let skip = |id: &str| {
        asked.lock().expect("lock").push(id.to_owned());
        id != "fav4"
    };
    let got = import_all(&mut src, &skip).await;
    assert_eq!(got.len(), 1);
    assert_eq!(got[0]["id"], "fav4");
    assert_eq!(got[0]["thumb"], Value::Null);
    assert_eq!(
        *asked.lock().expect("lock"),
        vec!["img1", "live2", "arch3", "fav4", "mp7"]
    );
    let log = requests(&mock);
    let downloads: Vec<_> = log
        .iter()
        .filter(|l| l.contains("/original") || l.contains("/thumbnail"))
        .collect();
    assert_eq!(downloads, vec!["GET /assets/fav4/original"]);
}

#[tokio::test]
async fn v2_api_with_visibility_and_pages() {
    let (url, mock) = serve(2, 5).await;
    let dir = scratch();
    let opts = ImmichOptions {
        temp_dir: Some(dir.path().to_owned()),
        ..Default::default()
    };
    let mut src = ImmichSource::new(&url, KEY, opts).expect("source");
    let got = import_all(&mut src, &|_| false).await;
    // Same items; the archive pass comes after the timeline pass.
    let mut want = expected();
    let arch = want.remove(2);
    want.push(arch);
    assert_eq!(got, want);

    let log = requests(&mock);
    assert!(log.iter().any(|l| l == "GET /albums?shared=true"));
    assert!(log.iter().any(|l| l == "GET /albums/alb-holiday"));
    let searches: Vec<_> = log
        .iter()
        .filter(|l| l.starts_with("POST /search/metadata"))
        .collect();
    assert!(
        searches
            .iter()
            .any(|l| l.contains(r#""page":2"#) && l.contains(r#""visibility":"timeline""#))
    );
    assert!(
        searches
            .iter()
            .any(|l| l.contains(r#""visibility":"archive""#))
    );
}

#[tokio::test]
async fn wrong_key_is_an_error() {
    let (url, _mock) = serve(3, 4).await;
    let dir = scratch();
    let opts = ImmichOptions {
        temp_dir: Some(dir.path().to_owned()),
        ..Default::default()
    };
    let mut src = ImmichSource::new(&url, "wrong", opts).expect("source");
    let err = src.next(&|_| false).await.expect_err("must fail");
    let msg = format!("{err:#}");
    assert!(msg.contains("401") && msg.contains("user.read"), "{msg}");
    assert_eq!(src.name(), "immich");
}
