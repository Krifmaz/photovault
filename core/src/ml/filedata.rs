//! Client of Ente's file-data API (`/files/data/*`) for `mldata`, and of the
//! remote store (`/remote-store`) that holds the ML on/off flag.
//!
//! Mirrors `web/packages/gallery/services/file-data.ts`,
//! `web/packages/new/photos/services/remote-store.ts` and the mobile
//! `services/filedata/filedata_service.dart`; the JSON matches
//! `server/src/routes/file_data.rs` and `server/src/library/filedata.rs`.
//!
//! Based on Ente (ente.com).

use crate::api::Api;
use crate::{CoreError, Result, crypto};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Most file ids per `POST /files/data/fetch` (`embeddingFetchLimit` on
/// mobile; the server refuses more than 200).
pub(crate) const FETCH_BATCH: usize = 200;

/// Remote-store key of the ML switch. Despite its name it controls all ML
/// (faces and magic search) in every Ente client.
pub(crate) const ML_REMOTE_KEY: &str = "faceSearchEnabled";

/// One row of `POST /files/data/status-diff` (`FDStatus`).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct FdStatus {
    #[serde(rename = "fileID")]
    pub file_id: i64,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(rename = "isDeleted", default)]
    pub is_deleted: bool,
    #[serde(rename = "updatedAt")]
    pub updated_at: i64,
}

#[derive(Deserialize)]
struct DiffResponse {
    #[serde(default)]
    diff: Option<Vec<FdStatus>>,
}

/// One page of the status diff after `since`, oldest first. Empty when
/// nothing changed (the server answers `{"diff": null}`).
pub(crate) async fn status_diff(api: &Api, since: i64) -> Result<Vec<FdStatus>> {
    let r: DiffResponse = api
        .json(
            Method::POST,
            "/files/data/status-diff",
            &json!({ "lastUpdatedAt": since }),
        )
        .await?;
    Ok(r.diff.unwrap_or_default())
}

/// Encrypted file data as fetched (`RemoteFileData`).
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RemoteFileData {
    #[serde(rename = "fileID")]
    pub file_id: i64,
    #[serde(rename = "encryptedData")]
    pub encrypted_data: String,
    #[serde(rename = "decryptionHeader")]
    pub decryption_header: String,
    /// Optional only for older self-hosted museums.
    #[serde(rename = "updatedAt", default)]
    pub updated_at: Option<i64>,
}

#[derive(Deserialize)]
struct FetchResponse {
    #[serde(default)]
    data: Vec<RemoteFileData>,
}

/// `POST /files/data/fetch` for up to [`FETCH_BATCH`] ids. Results are
/// sparse and unordered: associate them by `file_id`.
pub(crate) async fn fetch(api: &Api, kind: &str, file_ids: &[i64]) -> Result<Vec<RemoteFileData>> {
    if file_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for chunk in file_ids.chunks(FETCH_BATCH) {
        let r: FetchResponse = api
            .json(
                Method::POST,
                "/files/data/fetch",
                &json!({ "type": kind, "fileIDs": chunk }),
            )
            .await?;
        out.extend(r.data);
    }
    Ok(out)
}

#[derive(Serialize)]
struct PutRequest<'a> {
    #[serde(rename = "fileID")]
    file_id: i64,
    #[serde(rename = "type")]
    kind: &'a str,
    #[serde(rename = "encryptedData")]
    encrypted_data: String,
    #[serde(rename = "decryptionHeader")]
    decryption_header: String,
    /// The `updatedAt` this client last saw (0 when there was nothing),
    /// so the server refuses to overwrite newer data (409).
    #[serde(rename = "lastUpdatedAt")]
    last_updated_at: i64,
}

/// `PUT /files/data` with already encrypted data.
pub(crate) async fn put(
    api: &Api,
    file_id: i64,
    kind: &str,
    encrypted_data: &[u8],
    decryption_header: &[u8],
    last_updated_at: i64,
) -> Result<()> {
    api.json_no_content(
        Method::PUT,
        "/files/data",
        &PutRequest {
            file_id,
            kind,
            encrypted_data: crypto::b64(encrypted_data),
            decryption_header: crypto::b64(decryption_header),
            last_updated_at,
        },
    )
    .await
}

#[derive(Deserialize)]
struct RemoteValue {
    value: String,
}

/// A boolean remote-store flag (`getRemoteFlag`): `GET /remote-store` with
/// `defaultValue=false`.
pub(crate) async fn get_remote_flag(api: &Api, key: &str) -> Result<bool> {
    let v: Option<RemoteValue> = api
        .get_json(
            "/remote-store",
            &[("key", key.to_string()), ("defaultValue", "false".into())],
        )
        .await?;
    Ok(v.is_some_and(|v| v.value == "true"))
}

/// Set a boolean remote-store flag (`updateRemoteFlag`):
/// `POST /remote-store/update` with the value as `JSON.stringify(bool)`.
pub(crate) async fn update_remote_flag(api: &Api, key: &str, value: bool) -> Result<()> {
    api.json_no_content(
        Method::POST,
        "/remote-store/update",
        &json!({ "key": key, "value": if value { "true" } else { "false" } }),
    )
    .await
}

/// `true` for a 409 answer (the data changed on the server since it was
/// read, or it already exists).
pub(crate) fn is_conflict(e: &CoreError) -> bool {
    e.status() == Some(409)
}

/// `true` for an error that a retry later can fix (network, 5xx, 429).
pub(crate) fn is_transient(e: &CoreError) -> bool {
    match e {
        CoreError::Network { .. } | CoreError::Unreachable { .. } => true,
        CoreError::Server { status, .. } => *status == 429 || *status >= 500,
        _ => false,
    }
}
