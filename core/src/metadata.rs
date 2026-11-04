//! File metadata and magic metadata in Ente's formats.
//!
//! Copied from `importers/src/ente/metadata.rs` (branch `claude/project-thread-cp9x81`).
//!
//! * File metadata (`web/packages/media/file-metadata.ts`, `FileMetadata`):
//!   JSON `{fileType, title, creationTime, modificationTime, hash, latitude,
//!   longitude, duration?}` encrypted with the file key as one secretstream
//!   message (`encryptMetadataJSON`).
//! * Magic metadata (`web/packages/media/magic-metadata.ts`): a JSON object
//!   with null/undefined keys dropped, encrypted the same way, sent as
//!   `{version, count, data, header}` where `count` is the number of keys and
//!   `version` is the version currently stored on the server (1 for new).

use super::model::RemoteMagicMetadata;
use crate::crypto;
use serde::Serialize;
use serde_json::{Map, Value};

/// Ente `fileType` for images.
pub const FILE_TYPE_IMAGE: i64 = 0;
/// Ente `fileType` for videos.
pub const FILE_TYPE_VIDEO: i64 = 1;
/// Ente `fileType` for Live Photos.
pub const FILE_TYPE_LIVE_PHOTO: i64 = 2;

/// `ItemVisibility.archived` (files and collections).
pub const VISIBILITY_ARCHIVED: i64 = 1;
/// `ItemVisibility.hidden` (collections).
pub const VISIBILITY_HIDDEN: i64 = 2;

/// `CollectionSubType.defaultHidden`.
pub const SUB_TYPE_DEFAULT_HIDDEN: i64 = 1;

/// Separator between the image and video hash of a Live Photo
/// (`kLivePhotoHashSeparator` in the mobile app, `${imageHash}:${videoHash}`
/// on the web).
pub const LIVE_PHOTO_HASH_SEPARATOR: &str = ":";

/// The plaintext file metadata, field names and order as the web app writes
/// them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileMetadata {
    /// 0 image, 1 video, 2 Live Photo.
    pub file_type: i64,
    /// File name (for a Live Photo, the image's name).
    pub title: String,
    /// Capture time, µs since the epoch.
    pub creation_time: i64,
    /// Modification time, µs since the epoch.
    pub modification_time: i64,
    /// Base64 BLAKE2b content hash; `image:video` for Live Photos.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    /// Duration in whole seconds (videos).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<i64>,
    /// Latitude.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    /// Longitude.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
}

/// Decrypted magic metadata.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MagicMetadata {
    /// Version as stored on the server.
    pub version: i64,
    /// The JSON object.
    pub data: Map<String, Value>,
}

/// Drop null values, as `createMagicMetadata` does.
fn compact(data: &Map<String, Value>) -> Map<String, Value> {
    data.iter()
        .filter(|(_, v)| !v.is_null())
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

/// Encrypt `data` as magic metadata carrying `version`.
pub fn encrypt_magic(
    data: &Map<String, Value>,
    version: i64,
    key: &[u8],
) -> crate::Result<RemoteMagicMetadata> {
    let data = compact(data);
    let json = serde_json::to_vec(&data)?;
    let (ct, header) = crypto::encrypt_blob(&json, key)?;
    Ok(RemoteMagicMetadata {
        version,
        count: data.len() as i64,
        data: crypto::b64(&ct),
        header: crypto::b64(&header),
    })
}

/// Decrypt server magic metadata.
pub fn decrypt_magic(remote: &RemoteMagicMetadata, key: &[u8]) -> crate::Result<MagicMetadata> {
    let json = crypto::decrypt_blob(
        &crypto::unb64(&remote.data)?,
        &crypto::unb64(&remote.header)?,
        key,
    )?;
    let parsed = serde_json::from_slice::<Value>(&json)
        .map_err(|e| crate::CoreError::protocol(format!("magic metadata is not JSON: {e}")))?;
    let data = match parsed {
        Value::Object(m) => m,
        _ => Map::new(),
    };
    Ok(MagicMetadata {
        version: remote.version,
        data,
    })
}

/// Encrypt any JSON value with `key` as `(encryptedData, decryptionHeader)`
/// (base64), like `encryptMetadataJSON`.
pub fn encrypt_json<T: Serialize>(value: &T, key: &[u8]) -> crate::Result<(String, String)> {
    let (ct, header) = crypto::encrypt_blob(&serde_json::to_vec(value)?, key)?;
    Ok((crypto::b64(&ct), crypto::b64(&header)))
}

/// Decrypt `encryptMetadataJSON` output.
pub fn decrypt_json(encrypted_data: &str, header: &str, key: &[u8]) -> crate::Result<Value> {
    let plain = crypto::decrypt_blob(
        &crypto::unb64(encrypted_data)?,
        &crypto::unb64(header)?,
        key,
    )?;
    serde_json::from_slice(&plain)
        .map_err(|e| crate::CoreError::protocol(format!("metadata is not JSON: {e}")))
}

/// The content hash of decrypted file metadata, as `metadataHash` in the
/// web app computes it: `hash`, or `imageHash:videoHash` for Live Photos
/// uploaded by old web versions.
pub fn metadata_hash(metadata: &Value) -> Option<String> {
    let s = |k: &str| {
        metadata
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    };
    if let Some(h) = s("hash") {
        return Some(h.to_string());
    }
    let is_live = metadata.get("fileType").and_then(Value::as_i64) == Some(FILE_TYPE_LIVE_PHOTO);
    match (is_live, s("imageHash"), s("videoHash")) {
        (true, Some(i), Some(v)) => Some(format!("{i}{LIVE_PHOTO_HASH_SEPARATOR}{v}")),
        _ => None,
    }
}

/// The public magic metadata of a new upload: `w`, `h` and `caption`, as
/// `extractImageOrVideoMetadata` fills them.
pub fn public_magic_data(
    dimensions: Option<(u32, u32)>,
    caption: Option<&str>,
) -> Map<String, Value> {
    let mut m = Map::new();
    if let Some((w, h)) = dimensions {
        if w > 0 {
            m.insert("w".into(), w.into());
        }
        if h > 0 {
            m.insert("h".into(), h.into());
        }
    }
    if let Some(c) = caption.filter(|c| !c.is_empty()) {
        m.insert("caption".into(), c.into());
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metadata_json_matches_web_field_names() {
        let m = FileMetadata {
            file_type: FILE_TYPE_IMAGE,
            title: "a.jpg".into(),
            creation_time: 1,
            modification_time: 2,
            hash: Some("h".into()),
            duration: None,
            latitude: Some(1.5),
            longitude: Some(-2.0),
        };
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"fileType":0,"title":"a.jpg","creationTime":1,"modificationTime":2,"hash":"h","latitude":1.5,"longitude":-2.0}"#
        );
    }

    #[test]
    fn hash_of_live_photo_variants() {
        assert_eq!(metadata_hash(&json!({"hash": "x"})), Some("x".into()));
        assert_eq!(
            metadata_hash(&json!({"fileType": 2, "imageHash": "i", "videoHash": "v"})),
            Some("i:v".into())
        );
        assert_eq!(
            metadata_hash(&json!({"fileType": 0, "imageHash": "i", "videoHash": "v"})),
            None
        );
        assert_eq!(metadata_hash(&json!({})), None);
    }

    #[test]
    fn public_magic_skips_empty() {
        assert!(public_magic_data(None, Some("")).is_empty());
        let m = public_magic_data(Some((4, 3)), Some("hi"));
        assert_eq!(Value::Object(m), json!({"w": 4, "h": 3, "caption": "hi"}));
    }
}
