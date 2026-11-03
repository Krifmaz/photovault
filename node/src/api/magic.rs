//! Public magic metadata updates (`PUT /files/public-magic-metadata`).
//!
//! Public magic metadata is a small JSON object per file (`w`, `h`, edited
//! name, the streamable-video flag `sv`, …), encrypted with the file key as a
//! libsodium secretstream blob, exactly like file metadata. The apps update
//! it by sending the whole re-encrypted object with the version they read;
//! museum rejects the write with `409 Conflict` when the version on the
//! server is different (another client changed it first) and otherwise
//! stores it and increments the version.
//!
//! The node uses this for one thing only: like the Ente apps, it sets
//! `sv = 1` on a video that needs no streamable preview, so no client tries
//! to generate one again.

use super::models::MagicMetadata;
use super::{ApiClient, ApiError, Result};
use crate::crypto::{self, SecretKey};
use reqwest::Method;
use serde::Serialize;
use serde_json::{Map, Value};

/// Body of `PUT /files/public-magic-metadata` (museum
/// `UpdateMultipleMagicMetadataRequest`).
#[derive(Serialize)]
struct UpdateRequest<'a> {
    #[serde(rename = "metadataList")]
    metadata_list: [UpdateItem<'a>; 1],
}

#[derive(Serialize)]
struct UpdateItem<'a> {
    id: i64,
    #[serde(rename = "magicMetadata")]
    magic_metadata: &'a MagicMetadata,
}

/// Merge `updates` into the decrypted `current` public magic metadata and
/// encrypt the result with the file key, as the Ente web app's
/// `updateRemotePublicMagicMetadata` does.
///
/// `current_version` is the `version` of the magic metadata the file had
/// when it was read (`None` when the file had none: the web app then sends
/// 1, which museum accepts because there is nothing to compare against).
/// `count` is the number of top-level keys.
pub fn encrypt_public_magic_metadata(
    current: Option<&Value>,
    current_version: Option<i64>,
    updates: &Map<String, Value>,
    file_key: &SecretKey,
) -> Result<MagicMetadata> {
    let mut data = match current {
        Some(Value::Object(m)) => m.clone(),
        Some(Value::Null) | None => Map::new(),
        Some(_) => {
            return Err(ApiError::InvalidInput(
                "public magic metadata is not a JSON object".into(),
            ));
        }
    };
    for (k, v) in updates {
        data.insert(k.clone(), v.clone());
    }
    let json = serde_json::to_vec(&data).map_err(|e| ApiError::InvalidInput(e.to_string()))?;
    let blob = crypto::encrypt_blob(&json, file_key)?;
    Ok(MagicMetadata {
        version: current_version.unwrap_or(1),
        count: data.len() as i64,
        data: crypto::b64_encode(&blob.encrypted_data),
        header: crypto::b64_encode(&blob.decryption_header),
    })
}

impl ApiClient {
    /// `PUT /files/public-magic-metadata` for one file. `metadata.version`
    /// must be the version read from the server; a stale version answers
    /// `409` ([`ApiError::status`] `Some(409)`).
    pub async fn update_public_magic_metadata(
        &self,
        file_id: i64,
        metadata: &MagicMetadata,
    ) -> Result<()> {
        let body = UpdateRequest {
            metadata_list: [UpdateItem {
                id: file_id,
                magic_metadata: metadata,
            }],
        };
        self.send_json_no_content(Method::PUT, "/files/public-magic-metadata", &body, true)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merges_and_encrypts() {
        let key = SecretKey::generate();
        let current = json!({"w": 1920, "h": 1080});
        let mut up = Map::new();
        up.insert("sv".into(), json!(1));
        let mm = encrypt_public_magic_metadata(Some(&current), Some(3), &up, &key).unwrap();
        assert_eq!(mm.version, 3);
        assert_eq!(mm.count, 3);
        let plain = crypto::decrypt_blob(
            &crypto::b64_decode(&mm.data).unwrap(),
            &crypto::b64_decode(&mm.header).unwrap(),
            &key,
        )
        .unwrap();
        let v: Value = serde_json::from_slice(&plain).unwrap();
        assert_eq!(v, json!({"w": 1920, "h": 1080, "sv": 1}));

        let mm = encrypt_public_magic_metadata(None, None, &up, &key).unwrap();
        assert_eq!((mm.version, mm.count), (1, 1));
        assert!(encrypt_public_magic_metadata(Some(&json!([1])), None, &up, &key).is_err());
    }

    #[test]
    fn request_shape() {
        let mm = MagicMetadata {
            version: 2,
            count: 1,
            data: "ZA==".into(),
            header: "aA==".into(),
        };
        let body = UpdateRequest {
            metadata_list: [UpdateItem {
                id: 7,
                magic_metadata: &mm,
            }],
        };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            json!({"metadataList": [{"id": 7, "magicMetadata":
                {"version": 2, "count": 1, "data": "ZA==", "header": "aA=="}}]})
        );
    }
}
