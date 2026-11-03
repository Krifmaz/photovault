//! Immich source: reads a library from a running Immich server over its
//! REST API and yields [`crate::item::ImportItem`]s.
//!
//! # What is mapped
//!
//! | Immich                                         | Ente / [`ImportItem`](crate::item::ImportItem) |
//! |------------------------------------------------|-------------------------------------------------|
//! | original file (`GET /assets/{id}/original`)    | `primary`                                       |
//! | preview (`GET /assets/{id}/thumbnail?size=preview`) | `thumbnail`                                |
//! | Live Photo (`livePhotoVideoId`)                | `LivePhoto` with `live_video`; the motion asset is not imported again |
//! | albums, owned and shared with the user         | `albums` (names)                                |
//! | people on the asset (named, not hidden)        | `people` (names only, no face data)             |
//! | `isFavorite`                                   | `favorite`                                      |
//! | `visibility = archive` (`isArchived` before v1.133) | `archived`                                 |
//! | `visibility = locked` (opt-in)                 | `hidden`                                        |
//! | `exifInfo.description`                         | `description`                                   |
//! | `exifInfo.latitude/longitude`                  | `location`                                      |
//! | `dateTimeOriginal` / `fileCreatedAt` / `localDateTime` | `creation_time_us` (see [`dates`])      |
//! | `fileModifiedAt`                               | `modification_time_us`                          |
//! | `width`/`height` (or EXIF size)                | `dimensions`                                    |
//!
//! Trashed assets, assets owned by partners or other album members, and
//! Immich's hidden Live Photo motion parts are skipped.
//!
//! # Server versions
//!
//! Written against Immich v3.4 (filter-based `POST /search/metadata` with
//! cursor pagination, added in v3.2). Older servers are detected through
//! `GET /server/version` and use the earlier request shapes: top-level
//! `visibility` with page numbers (v1.133 to v3.1), and `isArchived` /
//! `isVisible` before that. Before v3 album contents come from
//! `GET /albums/{id}`; from v3 on from a search by album.
//!
//! # API key permissions
//!
//! `user.read`, `album.read`, `asset.read`, `asset.view`, `asset.download`
//! and, for progress totals, `asset.statistics` (optional).

pub mod api;
pub mod dates;
pub mod model;
mod source;

pub use dates::NaiveTimes;
pub use source::{ImmichOptions, ImmichSource};
