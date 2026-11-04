//! Ente's user entities (`/user-entity/...`): end-to-end encrypted JSON
//! records of a type (`cgroup` = people, `location` = location tags),
//! encrypted with a per-type entity key that is itself encrypted with the
//! master key.
//!
//! Exactly as the Ente apps do it (web
//! `web/packages/new/photos/services/user-entity/{index,remote}.ts`, mobile
//! `mobile/apps/photos/lib/services/entity_service.dart`):
//!
//! - **Entity key**: one per type. `GET /user-entity/key?type=` answers
//!   `{encryptedKey, header}`: a random 32-byte key sealed with
//!   `crypto_secretbox` under the master key (museum calls the nonce
//!   `header`). On 404 the client generates one and `POST`s it. The
//!   decrypted key is cached sealed under the device key in kv
//!   `ml:entity_key:<type>`.
//! - **Data**: `JSON.stringify(data)`, gzipped for every type except
//!   `location` and `person` (mobile `EntityType.isZipped`; web gzips
//!   `cgroup` only and knows no other gzipped type), then encrypted with
//!   the entity key as one secretstream message (`encryptBlob`):
//!   `{encryptedData, header}`, both base64.
//! - **Diff**: `GET /user-entity/entity/diff?type=&sinceTime=&limit=500`
//!   answers `{diff: [{id, encryptedData, header, isDeleted, updatedAt}]}`
//!   ordered by `updatedAt` (exclusive cursor). The cursor (µs) is kept in
//!   kv `ml:entity_since:<type>`.
//! - **Writes**: `POST /user-entity/entity {type, encryptedData, header}`
//!   (answers the new entity with its `id`), `PUT` with `id` too, and
//!   `DELETE /user-entity/entity?id=`.
//!
//! Decrypted entities live sealed in the `ml_entities` table; read them with
//! [`crate::store::Store::entities`].
//!
//! Based on Ente (ente.com).

use std::io::{Read as _, Write as _};

use crate::client::Inner;
use crate::crypto;
use crate::ml::store::kv;
use crate::{CoreError, Result};
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

/// Entity type of people.
pub(crate) const CGROUP: &str = "cgroup";
/// Entity type of location tags.
pub(crate) const LOCATION: &str = "location";

/// Page size of diffs (`defaultDiffLimit` / `EntityService.fetchLimit`).
pub const DIFF_LIMIT: usize = 500;

/// Largest decompressed entity accepted (a guard against gzip bombs; a
/// person with tens of thousands of faces is well below it).
const MAX_ENTITY_BYTES: u64 = 64 * 1024 * 1024;

/// Whether entities of `kind` are gzipped before encryption (mobile
/// `EntityType.isZipped`).
#[must_use]
pub fn is_gzipped(kind: &str) -> bool {
    !matches!(kind, "location" | "person")
}

/// Encrypt an entity's data like the Ente apps: JSON, gzipped if
/// [`is_gzipped`], one secretstream message under `entity_key`. Returns
/// `(encryptedData, header)` in base64.
pub fn encrypt_data(kind: &str, data: &Value, entity_key: &[u8]) -> Result<(String, String)> {
    let mut json = serde_json::to_vec(data)?;
    let bytes = if is_gzipped(kind) {
        let mut enc = GzEncoder::new(Vec::with_capacity(json.len() / 2), Compression::default());
        enc.write_all(&json)?;
        let gz = enc.finish()?;
        crypto::wipe(&mut json);
        gz
    } else {
        json
    };
    let mut bytes = bytes;
    let r = crypto::encrypt_blob(&bytes, entity_key);
    crypto::wipe(&mut bytes);
    let (ct, header) = r?;
    Ok((crypto::b64(&ct), crypto::b64(&header)))
}

/// Inverse of [`encrypt_data`].
pub fn decrypt_data(
    kind: &str,
    encrypted_data: &str,
    header: &str,
    entity_key: &[u8],
) -> Result<Value> {
    let ct = crypto::unb64(encrypted_data)?;
    let header = crypto::unb64(header)?;
    let mut plain = crypto::decrypt_blob(&ct, &header, entity_key)?;
    let r = if is_gzipped(kind) {
        let mut json = Vec::new();
        let read = GzDecoder::new(plain.as_slice())
            .take(MAX_ENTITY_BYTES + 1)
            .read_to_end(&mut json);
        let r = match read {
            Ok(n) if n as u64 > MAX_ENTITY_BYTES => {
                Err(CoreError::protocol("user entity is too large"))
            }
            Ok(_) => serde_json::from_slice(&json).map_err(CoreError::from),
            Err(e) => Err(CoreError::protocol(format!("user entity: bad gzip: {e}"))),
        };
        crypto::wipe(&mut json);
        r
    } else {
        serde_json::from_slice(&plain).map_err(CoreError::from)
    };
    crypto::wipe(&mut plain);
    r
}

/// Decrypt an entity key fetched from `/user-entity/key` with the master
/// key.
pub fn open_entity_key(encrypted_key: &str, header: &str, master_key: &[u8]) -> Result<Vec<u8>> {
    let ct = crypto::unb64(encrypted_key)?;
    let nonce = crypto::unb64(header)?;
    Ok(crypto::secretbox_open(&ct, &nonce, master_key)?)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteKey {
    encrypted_key: String,
    header: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteEntity {
    id: String,
    #[serde(default)]
    encrypted_data: Option<String>,
    #[serde(default)]
    header: Option<String>,
    #[serde(default)]
    is_deleted: bool,
    #[serde(default)]
    updated_at: i64,
}

#[derive(Deserialize)]
struct Diff {
    diff: Vec<RemoteEntity>,
}

fn key_kv(kind: &str) -> String {
    format!("{}{kind}", kv::ENTITY_KEY_PREFIX)
}

fn since_kv(kind: &str) -> String {
    format!("{}{kind}", kv::ENTITY_SINCE_PREFIX)
}

/// The decrypted entity key of `kind`: from the sealed kv cache, else the
/// server's, else a new one that is uploaded (Ente's
/// `getOrCreateEntityKey`).
pub(crate) async fn entity_key(inner: &Inner, kind: &str) -> Result<Vec<u8>> {
    if let Some(k) = inner.store().get_sealed::<Vec<u8>>(&key_kv(kind))? {
        return Ok(k);
    }
    let session = inner.session()?;
    let api = inner.api()?;
    let fetch = || async {
        match api
            .get_json::<RemoteKey>("/user-entity/key", &[("type", kind.to_string())])
            .await
        {
            Ok(k) => Ok(Some(k)),
            Err(e) if e.status() == Some(404) => Ok(None),
            Err(e) => Err(e),
        }
    };
    let key = match fetch().await? {
        // Decrypt before caching so malformed remote data cannot poison
        // local state.
        Some(k) => open_entity_key(&k.encrypted_key, &k.header, &session.master_key)?,
        None => {
            let mut key = crypto::generate_key();
            let (ct, nonce) = crypto::secretbox_seal(&key, &session.master_key)?;
            let (encrypted_key, header) = (crypto::b64(&ct), crypto::b64(&nonce));
            // Sanity check, like the web app: it must open as if fetched.
            let check = open_entity_key(&encrypted_key, &header, &session.master_key)?;
            if check != key {
                crypto::wipe(&mut key);
                return Err(CoreError::crypto("entity key does not round trip"));
            }
            let posted = api
                .json_no_content(
                    Method::POST,
                    "/user-entity/key",
                    &json!({"type": kind, "encryptedKey": encrypted_key, "header": header}),
                )
                .await;
            match posted {
                Ok(()) => key,
                // Another device created one meanwhile: use that.
                Err(e) if e.status() == Some(409) => {
                    crypto::wipe(&mut key);
                    let k = fetch()
                        .await?
                        .ok_or_else(|| CoreError::protocol("entity key vanished"))?;
                    open_entity_key(&k.encrypted_key, &k.header, &session.master_key)?
                }
                Err(e) => {
                    crypto::wipe(&mut key);
                    return Err(e);
                }
            }
        }
    };
    inner.store().put_sealed(&key_kv(kind), &key)?;
    Ok(key)
}

/// Pull new and changed entities of `kind` into the store. Returns whether
/// anything changed.
pub(crate) async fn sync(inner: &Inner, kind: &str) -> Result<bool> {
    let mut key = entity_key(inner, kind).await?;
    let r = sync_with_key(inner, kind, &key).await;
    crypto::wipe(&mut key);
    r
}

async fn sync_with_key(inner: &Inner, kind: &str, key: &[u8]) -> Result<bool> {
    let api = inner.api()?;
    let mut since: i64 = inner
        .store()
        .get_plain(&since_kv(kind))?
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut changed = false;
    loop {
        let page: Diff = api
            .get_json(
                "/user-entity/entity/diff",
                &[
                    ("type", kind.to_string()),
                    ("sinceTime", since.to_string()),
                    ("limit", DIFF_LIMIT.to_string()),
                ],
            )
            .await?;
        if page.diff.is_empty() {
            break;
        }
        let n = page.diff.len();
        // Decrypting is CPU work: do it before taking the store lock.
        let mut decoded = Vec::with_capacity(n);
        for e in page.diff {
            since = since.max(e.updated_at);
            let data = match (e.is_deleted, e.encrypted_data, e.header) {
                (true, _, _) => None,
                (false, Some(d), Some(h)) => Some(decrypt_data(kind, &d, &h, key)?),
                (false, _, _) => {
                    return Err(CoreError::protocol(format!(
                        "user entity {} has no data",
                        e.id
                    )));
                }
            };
            decoded.push((e.id, e.updated_at, data));
        }
        {
            let st = inner.store();
            // One transaction per page: the page and its cursor land together.
            st.conn().execute_batch("BEGIN IMMEDIATE")?;
            let r = (|| -> Result<()> {
                for (id, updated_at, data) in &decoded {
                    match data {
                        Some(d) => st.put_entity(kind, id, *updated_at, d)?,
                        None => st.delete_entity(kind, id)?,
                    }
                }
                st.put_plain(&since_kv(kind), &since.to_string())
            })();
            match r {
                Ok(()) => st.conn().execute_batch("COMMIT")?,
                Err(e) => {
                    let _ = st.conn().execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        }
        changed = true;
        if n < DIFF_LIMIT {
            break;
        }
    }
    Ok(changed)
}

/// Create an entity; returns its id.
pub(crate) async fn create(inner: &Inner, kind: &str, data: &Value) -> Result<String> {
    let mut key = entity_key(inner, kind).await?;
    let enc = encrypt_data(kind, data, &key);
    crypto::wipe(&mut key);
    let (encrypted_data, header) = enc?;
    let res: Value = inner
        .api()?
        .json(
            Method::POST,
            "/user-entity/entity",
            &json!({"type": kind, "encryptedData": encrypted_data, "header": header}),
        )
        .await?;
    let id = res
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| CoreError::protocol("created entity has no id"))?
        .to_string();
    let updated_at = updated_at_of(&res);
    inner.store().put_entity(kind, &id, updated_at, data)?;
    Ok(id)
}

/// Replace an entity's data.
pub(crate) async fn update(inner: &Inner, kind: &str, id: &str, data: &Value) -> Result<()> {
    put(inner, kind, id, data, None).await
}

/// Replace an entity's data only if the server still has the version
/// with `expected_updated_at` (`expectedUpdatedAt`, museum's optimistic
/// concurrency, as mobile sends it). Returns `false`, and changes nothing,
/// when another client wrote it meanwhile (409).
pub(crate) async fn update_if_unchanged(
    inner: &Inner,
    kind: &str,
    id: &str,
    data: &Value,
    expected_updated_at: i64,
) -> Result<bool> {
    match put(inner, kind, id, data, Some(expected_updated_at)).await {
        Ok(()) => Ok(true),
        Err(e) if e.status() == Some(409) => Ok(false),
        Err(e) => Err(e),
    }
}

async fn put(
    inner: &Inner,
    kind: &str,
    id: &str,
    data: &Value,
    expected_updated_at: Option<i64>,
) -> Result<()> {
    let mut key = entity_key(inner, kind).await?;
    let enc = encrypt_data(kind, data, &key);
    crypto::wipe(&mut key);
    let (encrypted_data, header) = enc?;
    let mut body =
        json!({"id": id, "type": kind, "encryptedData": encrypted_data, "header": header});
    if let Some(t) = expected_updated_at {
        body["expectedUpdatedAt"] = json!(t);
    }
    // The answer (the updated entity) is not needed: the next diff brings
    // the server's `updatedAt`.
    inner
        .api()?
        .json_no_content(Method::PUT, "/user-entity/entity", &body)
        .await?;
    inner
        .store()
        .put_entity(kind, id, crate::store::now_us(), data)?;
    Ok(())
}

/// Delete an entity.
pub(crate) async fn delete(inner: &Inner, kind: &str, id: &str) -> Result<()> {
    let path = format!(
        "/user-entity/entity?{}",
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("id", id)
            .finish()
    );
    let r = inner
        .api()?
        .json_no_content(Method::DELETE, &path, &json!({}))
        .await;
    match r {
        Ok(_) => {}
        // Already gone on the server.
        Err(e) if e.status() == Some(404) => {}
        Err(e) => return Err(e),
    }
    inner.store().delete_entity(kind, id)?;
    Ok(())
}

/// `updatedAt` of an entity answer (now if the answer has none).
fn updated_at_of(v: &Value) -> i64 {
    v.get("updatedAt")
        .and_then(Value::as_i64)
        .unwrap_or_else(crate::store::now_us)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_round_trips_gzipped_or_not() {
        crypto::init();
        let key = crypto::generate_key();
        let data = json!({"name": "Ana", "assigned": [], "isHidden": false, "x": {"y": 1}});
        for kind in [CGROUP, LOCATION] {
            let (d, h) = encrypt_data(kind, &data, &key).unwrap();
            assert_eq!(decrypt_data(kind, &d, &h, &key).unwrap(), data);
            // The plaintext is what the apps expect: gzip only for cgroup.
            let plain = crypto::decrypt_blob(
                &crypto::unb64(&d).unwrap(),
                &crypto::unb64(&h).unwrap(),
                &key,
            )
            .unwrap();
            assert_eq!(plain.starts_with(&[0x1f, 0x8b]), kind == CGROUP);
        }
        assert!(decrypt_data(CGROUP, "AAAA", "AAAA", &key).is_err());
    }
}
