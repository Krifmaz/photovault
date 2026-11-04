//! Search end to end against the real Photovault server, in process:
//! upload photos on one device, sync them on another, and search there
//! through `Core` exactly as the Android app does (suggestions while
//! typing, discovery, results of each suggestion, magic search without a
//! text model).

mod support;

use chrono::{Local, TimeZone};
use photovault_core::ml::{SearchKind, SearchSuggestion};
use photovault_core::{FileKind, LoginStep, TimelineItem};
use support::{TestCore, jpeg, server, signup};

const EMAIL: &str = "searcher@example.org";
const PASSWORD: &str = "correct horse battery staple";

fn local_noon(y: i32, m: u32, d: u32) -> std::time::SystemTime {
    let t = Local
        .with_ymd_and_hms(y, m, d, 12, 0, 0)
        .earliest()
        .unwrap()
        .timestamp();
    std::time::UNIX_EPOCH + std::time::Duration::from_secs(t as u64)
}

fn rows(s: &[SearchSuggestion]) -> Vec<(SearchKind, String, String, u64)> {
    s.iter()
        .map(|s| (s.kind, s.key.clone(), s.label.clone(), s.count))
        .collect()
}

fn titles(items: &[TimelineItem]) -> Vec<&str> {
    items.iter().map(|i| i.title.as_str()).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_a_synced_library() {
    let srv = server(&[EMAIL]).await;
    signup(&srv.url, EMAIL, PASSWORD).await;

    // Device A uploads three photos taken (file time, no EXIF) on known days.
    let a = TestCore::connected(&srv.url).await;
    assert!(matches!(
        a.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap(),
        LoginStep::Done { .. }
    ));
    for (i, (name, day)) in [
        ("IMG_0001.jpg", (2024, 5, 17)),
        ("Screenshot_20240601.jpg", (2024, 6, 1)),
        ("DSC_0003.jpg", (2023, 5, 17)),
    ]
    .into_iter()
    .enumerate()
    {
        let path = a.dir.path().join(name);
        std::fs::write(&path, jpeg(64, 48, i as u8 * 40)).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(local_noon(day.0, day.1, day.2))
            .unwrap();
        a.core
            .enqueue_upload(path.display().to_string(), None)
            .unwrap();
    }
    let st = a.core.run_uploads().await.unwrap();
    assert_eq!((st.pending, st.failed), (0, 0), "{st:?}");

    // Device B syncs and searches.
    let b = TestCore::connected(&srv.url).await;
    b.core.login(EMAIL.into(), PASSWORD.into()).await.unwrap();
    // Before the first sync: nothing to find.
    assert!(b.core.search_suggestions("img".into(), 10).is_empty());
    b.core.sync(None).await.unwrap();
    assert_eq!(b.core.timeline_count(), 3);

    // While typing.
    assert_eq!(
        rows(&b.core.search_suggestions("img".into(), 10)),
        vec![(SearchKind::FileName, "img".into(), "img".into(), 1)]
    );
    assert_eq!(
        rows(&b.core.search_suggestions("2024".into(), 10)),
        vec![
            (SearchKind::Date, "y:2024".into(), "2024".into(), 2),
            (SearchKind::FileName, "2024".into(), "2024".into(), 1),
        ]
    );
    assert_eq!(
        rows(&b.core.search_suggestions("17 may".into(), 10)),
        vec![(SearchKind::Date, "md:05-17".into(), "17 May".into(), 2)]
    );
    assert_eq!(
        rows(&b.core.search_suggestions("screen".into(), 10)),
        vec![
            (
                SearchKind::FileType,
                "screenshot".into(),
                "Screenshots".into(),
                1
            ),
            (SearchKind::FileName, "screen".into(), "screen".into(), 1),
        ]
    );
    let photos = b.core.search_suggestions("photos".into(), 10);
    assert_eq!(
        rows(&photos),
        vec![(SearchKind::FileType, "image".into(), "Photos".into(), 3)]
    );
    // The newest photo is the cover.
    let newest = b.core.timeline_page(0, 1)[0].id;
    assert_eq!(photos[0].cover_file_id, Some(newest));
    // No text model on this device: no magic row.
    assert!(
        b.core
            .search_suggestions("sunset by the sea".into(), 10)
            .iter()
            .all(|s| s.kind != SearchKind::Magic)
    );

    // Results, newest first.
    let r = |kind, key: &str| {
        let core = b.core.clone();
        let key = key.to_string();
        async move { core.search_results(kind, key).await.unwrap() }
    };
    assert_eq!(
        titles(&r(SearchKind::Date, "md:05-17").await),
        vec!["IMG_0001.jpg", "DSC_0003.jpg"]
    );
    assert_eq!(
        titles(&r(SearchKind::Date, "y:2024").await),
        vec!["Screenshot_20240601.jpg", "IMG_0001.jpg"]
    );
    assert_eq!(
        titles(&r(SearchKind::FileName, "dsc").await),
        vec!["DSC_0003.jpg"]
    );
    let images = r(SearchKind::FileType, "image").await;
    assert_eq!(images.len(), 3);
    assert!(images.iter().all(|i| i.kind == FileKind::Image));
    assert_eq!(images, b.core.timeline_page(0, 10));
    assert!(r(SearchKind::FileType, "video").await.is_empty());
    assert!(r(SearchKind::Caption, "img").await.is_empty());
    assert!(r(SearchKind::Place, "no-such-tag").await.is_empty());
    assert!(r(SearchKind::Magic, "photo of food").await.is_empty());
    assert!(
        b.core
            .magic_search("sunset".into(), 20)
            .await
            .unwrap()
            .is_empty()
    );

    // Discovery: file types and extensions (no people, places or albums
    // in this library; the uncategorized collection is not an album).
    assert_eq!(
        rows(&b.core.search_discovery()),
        vec![
            (SearchKind::FileType, "image".into(), "Photos".into(), 3),
            (
                SearchKind::FileType,
                "screenshot".into(),
                "Screenshots".into(),
                1
            ),
            (SearchKind::FileType, "ext:JPG".into(), "JPGs".into(), 3),
        ]
    );
}
