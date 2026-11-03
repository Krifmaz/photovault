//! The parts of Immich's API types the importer reads.
//!
//! Every field is optional or defaulted so that the importer keeps working
//! against older and newer servers that add, rename or drop fields. Names
//! follow Immich's OpenAPI spec (`AssetResponseDto`, `ExifResponseDto`,
//! `AlbumResponseDto`, ...), checked against Immich v3.4.

use serde::Deserialize;

/// `GET /server/version` (`ServerVersionResponseDto`).
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
pub struct ServerVersion {
    /// Major version.
    #[serde(default)]
    pub major: u32,
    /// Minor version.
    #[serde(default)]
    pub minor: u32,
    /// Patch version.
    #[serde(default)]
    pub patch: u32,
}

impl ServerVersion {
    /// The newest API this importer knows about; assumed when the server does
    /// not report a version.
    pub const LATEST: ServerVersion = ServerVersion {
        major: 3,
        minor: 4,
        patch: 0,
    };

    /// True if this version is at least `major.minor`.
    pub fn at_least(&self, major: u32, minor: u32) -> bool {
        (self.major, self.minor) >= (major, minor)
    }
}

/// `GET /users/me` (`UserResponseDto`), only the id.
#[derive(Debug, Clone, Deserialize)]
pub struct User {
    /// User id.
    pub id: String,
}

/// An album user entry (`AlbumUserResponseDto`).
#[derive(Debug, Clone, Deserialize)]
pub struct AlbumUser {
    /// The user.
    pub user: User,
}

/// An asset id, as found in the `assets` list of pre-v3 album responses.
#[derive(Debug, Clone, Deserialize)]
pub struct AssetRef {
    /// Asset id.
    pub id: String,
}

/// `AlbumResponseDto`, only what the importer needs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    /// Album id.
    pub id: String,
    /// Album name.
    #[serde(default)]
    pub album_name: String,
    /// Number of assets (informational).
    #[serde(default)]
    pub asset_count: Option<u64>,
    /// Owner and members (v3+; the first entry is the owner).
    #[serde(default)]
    pub album_users: Vec<AlbumUser>,
    /// The album's assets. Only filled by `GET /albums/{id}` on servers
    /// before v3, which removed it in favour of `POST /search/metadata`.
    #[serde(default)]
    pub assets: Option<Vec<AssetRef>>,
}

/// A person attached to an asset (`PersonResponseDto`), names only.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    /// Person id.
    #[serde(default)]
    pub id: String,
    /// Name; empty for people the user never named.
    #[serde(default)]
    pub name: String,
    /// Hidden by the user in Immich's people view.
    #[serde(default)]
    pub is_hidden: bool,
}

/// `ExifResponseDto`, only what the importer needs.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exif {
    /// Capture instant (UTC, ISO 8601). Immich stores the EXIF
    /// DateTimeOriginal converted to an instant using the photo's offset.
    #[serde(default)]
    pub date_time_original: Option<String>,
    /// Time zone of the capture: an IANA name (`Europe/Berlin`) or a fixed
    /// offset (`UTC+2`, `UTC+05:30`). `None` if the file had none.
    #[serde(default)]
    pub time_zone: Option<String>,
    /// GPS latitude in degrees.
    #[serde(default)]
    pub latitude: Option<f64>,
    /// GPS longitude in degrees.
    #[serde(default)]
    pub longitude: Option<f64>,
    /// Description / caption.
    #[serde(default)]
    pub description: Option<String>,
    /// Width as stored in EXIF (before orientation is applied).
    #[serde(default)]
    pub exif_image_width: Option<f64>,
    /// Height as stored in EXIF (before orientation is applied).
    #[serde(default)]
    pub exif_image_height: Option<f64>,
    /// EXIF orientation, as a string (`"6"`) or number depending on version.
    #[serde(default)]
    pub orientation: Option<serde_json::Value>,
}

/// `AssetResponseDto`, only what the importer needs.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    /// Asset id.
    pub id: String,
    /// `IMAGE`, `VIDEO`, `AUDIO` or `OTHER`.
    #[serde(rename = "type", default)]
    pub kind: String,
    /// Owner user id.
    #[serde(default)]
    pub owner_id: Option<String>,
    /// File name at upload.
    #[serde(default)]
    pub original_file_name: String,
    /// Path on the Immich server. Used to recognise motion parts Immich
    /// extracted from Android motion photos.
    #[serde(default)]
    pub original_path: Option<String>,
    /// Capture instant (UTC).
    #[serde(default)]
    pub file_created_at: Option<String>,
    /// File modification instant (UTC).
    #[serde(default)]
    pub file_modified_at: Option<String>,
    /// Local wall-clock capture time, written as if it were UTC. Not an
    /// instant: `2024-06-01T14:00:00.000Z` means 14:00 where it was taken.
    #[serde(default)]
    pub local_date_time: Option<String>,
    /// Favourite.
    #[serde(default)]
    pub is_favorite: bool,
    /// Archived (pre-v1.133 field; still sent by later servers).
    #[serde(default)]
    pub is_archived: Option<bool>,
    /// In the trash.
    #[serde(default)]
    pub is_trashed: bool,
    /// Visibility (v1.133+): `timeline`, `archive`, `hidden` or `locked`.
    #[serde(default)]
    pub visibility: Option<String>,
    /// Pre-v1.133 visibility flag (`false` for Live Photo motion parts).
    #[serde(default)]
    pub is_visible: Option<bool>,
    /// For a Live Photo still: the id of its (hidden) motion video asset.
    #[serde(default)]
    pub live_photo_video_id: Option<String>,
    /// Width after orientation (v2+).
    #[serde(default)]
    pub width: Option<f64>,
    /// Height after orientation (v2+).
    #[serde(default)]
    pub height: Option<f64>,
    /// EXIF data (with `withExif`).
    #[serde(default)]
    pub exif_info: Option<Exif>,
    /// People (with `withPeople`).
    #[serde(default)]
    pub people: Option<Vec<Person>>,
}

/// The `assets` part of `SearchResponseDto`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchAssets {
    /// The page's assets.
    #[serde(default)]
    pub items: Vec<Asset>,
    /// Cursor for the next page (v3.2+).
    #[serde(default)]
    pub next_cursor: Option<String>,
    /// Next page number as a string (before v3.2).
    #[serde(default)]
    pub next_page: Option<String>,
}

/// `SearchResponseDto`.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    /// Matching assets.
    pub assets: SearchAssets,
}

/// `AssetStatsResponseDto`.
#[derive(Debug, Clone, Deserialize)]
pub struct AssetStats {
    /// Number of matching assets.
    #[serde(default)]
    pub total: u64,
}
