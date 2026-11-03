//! Owner side of public links: album links (`public_collection_tokens`) and single-file
//! links (`public_file_tokens`), with museum's table and column names.
//!
//! The public-links workstream serves the anonymous side of these links and can reuse
//! [`album_url`], [`file_url`], [`active_collection_link`] and [`verify_link_password_jwt`].
//!
//! Link hosts come from `[accounts] public_albums_url` (album and file links) and
//! `public_locker_url` (Locker links), each defaulting to the server's `public_url`.

use super::errors;
use super::model::{PublicUrl, role};
use super::sql::{begin, db_err, get_bool, get_i64, get_opt_i64, get_opt_str, get_str, ids_list};
use crate::auth::App;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use sqlx::AnyConnection;
use sqlx::any::AnyRow;
use std::collections::HashMap;

/// Length of link access tokens (museum's `AccessTokenLength`).
pub const ACCESS_TOKEN_LENGTH: usize = 10;
const BASE57: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
const BASE62: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn random_from(alphabet: &[u8], n: usize) -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..n)
        .map(|_| alphabet[rng.random_range(0..alphabet.len())] as char)
        .collect()
}

/// museum's `base.NewID(prefix)`: `"<prefix>_"` + 22 base62 characters.
pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", random_from(BASE62, 22))
}

/// A link access token: upper-cased short-uuid characters, like museum.
pub fn new_access_token() -> String {
    random_from(BASE57, ACCESS_TOKEN_LENGTH).to_uppercase()
}

/// `configured` (a `[accounts]` app URL), or the server's `public_url` when empty.
pub fn app_host(state: &AppState, configured: &str) -> String {
    if configured.is_empty() {
        &state.config.public_url
    } else {
        configured
    }
    .trim_end_matches('/')
    .to_string()
}

/// Base URL of the public albums web app (`accounts.public_albums_url`).
pub fn albums_host(state: &AppState) -> String {
    app_host(state, &state.config.accounts.public_albums_url)
}

/// Base URL of the public Locker share web app (`accounts.public_locker_url`).
pub fn locker_host(state: &AppState) -> String {
    app_host(state, &state.config.accounts.public_locker_url)
}

/// `GetAlbumUrl`: `{albums}/?t=TOKEN`, Locker `{locker}/c/TOKEN`.
pub fn album_url(state: &AppState, app: App, token: &str) -> String {
    if app == App::Locker {
        format!("{}/c/{token}", locker_host(state))
    } else {
        format!("{}/?t={token}", albums_host(state))
    }
}

/// `FileLink`: `{albums}/file/?t=TOKEN`, Locker `{locker}/TOKEN`.
pub fn file_url(state: &AppState, app: App, token: &str) -> String {
    if app == App::Locker {
        format!("{}/{token}", locker_host(state))
    } else {
        format!("{}/file/?t={token}", albums_host(state))
    }
}

/// `ensureAccessTokenAvailable`: no active link of either kind uses `token`.
async fn token_available(conn: &mut AnyConnection, token: &str) -> Result<bool, sqlx::Error> {
    let a: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM public_file_tokens WHERE access_token = $1 AND is_disabled = FALSE",
    )
    .bind(token)
    .fetch_optional(&mut *conn)
    .await?;
    let b: Option<(String,)> =
        sqlx::query_as("SELECT access_token FROM public_collection_tokens WHERE access_token = $1")
            .bind(token)
            .fetch_optional(&mut *conn)
            .await?;
    Ok(a.is_none() && b.is_none())
}

/// An album link row (`ente.CollectionLinkRow`).
#[derive(Debug, Clone, Default)]
pub struct CollectionLinkRow {
    /// Row id.
    pub id: i64,
    /// Collection.
    pub collection_id: i64,
    /// Access token.
    pub token: String,
    /// Device limit.
    pub device_limit: i64,
    /// Expiry (0 = never).
    pub valid_till: i64,
    /// Disabled.
    pub is_disabled: bool,
    /// Password hash (set by the owner's client).
    pub pass_hash: Option<String>,
    /// KDF nonce.
    pub nonce: Option<String>,
    /// KDF memory limit.
    pub mem_limit: Option<i64>,
    /// KDF ops limit.
    pub ops_limit: Option<i64>,
    /// Downloads allowed.
    pub enable_download: bool,
    /// Uploads allowed.
    pub enable_collect: bool,
    /// Comments allowed.
    pub enable_comment: bool,
    /// Joining allowed.
    pub enable_join: bool,
    /// Minimum participant role that sees the link.
    pub min_role: Option<String>,
}

impl CollectionLinkRow {
    fn from_row(r: &AnyRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: get_i64(r, "id")?,
            collection_id: get_i64(r, "collection_id")?,
            token: get_str(r, "access_token")?,
            device_limit: get_i64(r, "device_limit")?,
            valid_till: get_i64(r, "valid_till")?,
            is_disabled: get_bool(r, "is_disabled")?,
            pass_hash: get_opt_str(r, "pw_hash")?,
            nonce: get_opt_str(r, "pw_nonce")?,
            mem_limit: get_opt_i64(r, "mem_limit")?,
            ops_limit: get_opt_i64(r, "ops_limit")?,
            enable_download: get_bool(r, "enable_download")?,
            enable_collect: get_bool(r, "enable_collect")?,
            enable_comment: get_bool(r, "enable_comment")?,
            enable_join: get_bool(r, "enable_join")?,
            min_role: get_opt_str(r, "min_role")?.map(|s| role::normalize(&s).to_string()),
        })
    }

    /// The `PublicURL` view of this row.
    pub fn to_public_url(&self, state: &AppState, app: App) -> PublicUrl {
        let pw = self.nonce.is_some();
        PublicUrl {
            url: album_url(state, app, &self.token),
            device_limit: self.device_limit,
            valid_till: self.valid_till,
            enable_download: self.enable_download,
            enable_collect: self.enable_collect,
            enable_comment: self.enable_comment,
            password_enabled: pw,
            nonce: self.nonce.clone(),
            mem_limit: if pw { self.mem_limit } else { None },
            ops_limit: if pw { self.ops_limit } else { None },
            enable_join: self.enable_join,
            min_role: self.min_role.clone(),
        }
    }

    /// `CanJoin`.
    pub fn can_join(&self) -> ApiResult<()> {
        if self.is_disabled {
            return Err(errors::bad_msg("link disabled"));
        }
        if self.valid_till > 0 && self.valid_till < now_micros() {
            return Err(errors::bad_msg("token expired"));
        }
        if !self.enable_download {
            return Err(errors::bad_msg("can not join as download is disabled"));
        }
        if !self.enable_join {
            return Err(errors::bad_msg("can not join as join is disabled"));
        }
        Ok(())
    }
}

const LINK_COLUMNS: &str = "id, collection_id, access_token, valid_till, device_limit, is_disabled, pw_hash,
    pw_nonce, mem_limit, ops_limit, enable_download, enable_collect, enable_comment, enable_join, min_role";

/// `GetActiveCollectionLinkRow`: the collection's non-disabled link (404 `{}` if none).
pub async fn active_collection_link(state: &AppState, cid: i64) -> ApiResult<CollectionLinkRow> {
    let r = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM public_collection_tokens WHERE collection_id = $1 AND is_disabled = FALSE"
    ))
    .bind(cid)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?
    .ok_or_else(errors::not_found)?;
    CollectionLinkRow::from_row(&r).map_err(db_err)
}

/// `GetCollectionLinkRowByToken`: the link with this access token, disabled ones included
/// (the public-link middleware reports those as gone).
pub async fn collection_link_by_token(
    state: &AppState,
    token: &str,
) -> ApiResult<Option<CollectionLinkRow>> {
    let r = sqlx::query(&format!(
        "SELECT {LINK_COLUMNS} FROM public_collection_tokens WHERE access_token = $1"
    ))
    .bind(token)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?;
    r.as_ref()
        .map(CollectionLinkRow::from_row)
        .transpose()
        .map_err(db_err)
}

/// `GetCollectionToActivePublicURLMap`: every id in `cids` maps to its (possibly empty) links.
pub async fn active_public_urls(
    state: &AppState,
    cids: &[i64],
    app: App,
) -> ApiResult<HashMap<i64, Vec<PublicUrl>>> {
    let mut out: HashMap<i64, Vec<PublicUrl>> = cids.iter().map(|c| (*c, Vec::new())).collect();
    if cids.is_empty() {
        return Ok(out);
    }
    let sql = format!(
        "SELECT {LINK_COLUMNS} FROM public_collection_tokens WHERE is_disabled = FALSE AND collection_id IN ({})",
        ids_list(1, cids.len())
    );
    let rows = super::files::bind_ids(sqlx::query(&sql), cids)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?;
    for r in rows {
        let l = CollectionLinkRow::from_row(&r).map_err(db_err)?;
        out.entry(l.collection_id)
            .or_default()
            .push(l.to_public_url(state, app));
    }
    Ok(out)
}

/// Bump a collection after its link changed, so collection diffs carry the new `publicURLs`.
async fn touch_collection(conn: &mut AnyConnection, cid: i64, now: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE collections SET updation_time = $1 WHERE collection_id = $2")
        .bind(now)
        .bind(cid)
        .execute(conn)
        .await?;
    Ok(())
}

/// `POST /collections/share-url` body (`ente.CreatePublicAccessTokenRequest`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CreateLinkRequest {
    /// Collection.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Allow uploads.
    pub enable_collect: bool,
    /// Allow comments.
    pub enable_comment: bool,
    /// Allow joining (default true).
    pub enable_join: Option<bool>,
    /// Expiry.
    pub valid_till: i64,
    /// Device limit (0-50).
    pub device_limit: i64,
}

/// `CollectionLinkController.CreateLink`: a new link, or the existing active one.
pub async fn create_collection_link(
    state: &AppState,
    app: App,
    req: &CreateLinkRequest,
) -> ApiResult<PublicUrl> {
    let join = req.enable_join.unwrap_or(true);
    for _ in 0..5 {
        let token = new_access_token();
        let now = now_micros();
        let mut tx = begin(&state.db).await.map_err(db_err)?;
        if !token_available(tx.conn(), &token).await.map_err(db_err)? {
            continue;
        }
        let res = sqlx::query(
            "INSERT INTO public_collection_tokens (collection_id, access_token, valid_till, device_limit,
                enable_collect, enable_comment, enable_join, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
        )
        .bind(req.collection_id)
        .bind(&token)
        .bind(req.valid_till)
        .bind(req.device_limit)
        .bind(req.enable_collect)
        .bind(req.enable_comment)
        .bind(join)
        .bind(now)
        .execute(tx.conn())
        .await;
        match res {
            Ok(_) => {
                touch_collection(tx.conn(), req.collection_id, now)
                    .await
                    .map_err(db_err)?;
                tx.commit().await.map_err(db_err)?;
                return Ok(PublicUrl {
                    url: album_url(state, app, &token),
                    valid_till: req.valid_till,
                    device_limit: req.device_limit,
                    enable_download: true,
                    enable_collect: req.enable_collect,
                    enable_comment: req.enable_comment,
                    enable_join: join,
                    password_enabled: false,
                    ..Default::default()
                });
            }
            Err(e) if super::objects::is_unique_violation(&e) => {
                drop(tx);
                // Either an active link exists for the collection or the token collided.
                let map = active_public_urls(state, &[req.collection_id], app).await?;
                if let Some(u) = map.get(&req.collection_id).and_then(|v| v.first()) {
                    return Ok(u.clone());
                }
            }
            Err(e) => return Err(db_err(e)),
        }
    }
    Err(errors::internal(
        "failed to generate unique access token for collection link",
    ))
}

/// `PUT /collections/share-url` body (`ente.UpdatePublicAccessTokenRequest`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdateLinkRequest {
    /// Collection.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// New expiry.
    pub valid_till: Option<i64>,
    /// New device limit.
    pub device_limit: Option<i64>,
    /// Password hash.
    pub pass_hash: Option<String>,
    /// KDF nonce.
    pub nonce: Option<String>,
    /// KDF memory limit.
    pub mem_limit: Option<i64>,
    /// KDF ops limit.
    pub ops_limit: Option<i64>,
    /// Downloads.
    pub enable_download: Option<bool>,
    /// Uploads.
    pub enable_collect: Option<bool>,
    /// Comments.
    pub enable_comment: Option<bool>,
    /// Remove the password.
    pub disable_password: Option<bool>,
    /// Joining.
    pub enable_join: Option<bool>,
    /// Minimum role.
    pub min_role: Option<String>,
}

fn validate_device_limit(d: i64) -> ApiResult<()> {
    if !(0..=50).contains(&d) {
        return Err(errors::bad_msg(format!(
            "device limit: {d} out of range [0-50]"
        )));
    }
    Ok(())
}

/// `validatePublicLinkKDFParams`.
pub fn validate_kdf(mem: Option<i64>, ops: Option<i64>) -> ApiResult<()> {
    match (mem, ops) {
        (None, None) => Ok(()),
        (Some(67108864), Some(2)) => Ok(()),
        _ => Err(errors::bad_msg("invalid KDF parameters")),
    }
}

/// Shared password-parameter validation for album and file link updates.
#[allow(clippy::too_many_arguments)]
fn validate_link_update(
    device_limit: Option<i64>,
    valid_till: Option<i64>,
    pass_hash: &Option<String>,
    nonce: &Option<String>,
    mem: Option<i64>,
    ops: Option<i64>,
    disable_password: Option<bool>,
) -> ApiResult<()> {
    if let Some(d) = device_limit {
        validate_device_limit(d)?;
    }
    if let Some(v) = valid_till
        && v != 0
        && v < now_micros()
    {
        return Err(errors::bad_msg(
            "valid till should be greater than current timestamp",
        ));
    }
    let none = nonce.is_none() && pass_hash.is_none() && mem.is_none() && ops.is_none();
    let all = nonce.is_some() && pass_hash.is_some() && mem.is_some() && ops.is_some();
    if !(none || all) {
        return Err(errors::bad_msg(
            "all password params should be either present or missing",
        ));
    }
    validate_kdf(mem, ops)?;
    if all && disable_password == Some(true) {
        return Err(errors::bad_msg(
            "can not set and disable password in same request",
        ));
    }
    Ok(())
}

impl CreateLinkRequest {
    /// museum's `Validate`.
    pub fn validate(&self) -> ApiResult<()> {
        validate_device_limit(self.device_limit)
    }
}

impl UpdateLinkRequest {
    /// museum's `Validate`.
    pub fn validate(&self) -> ApiResult<()> {
        if self.device_limit.is_none()
            && self.valid_till.is_none()
            && self.disable_password.is_none()
            && self.nonce.is_none()
            && self.pass_hash.is_none()
            && self.enable_download.is_none()
            && self.enable_collect.is_none()
            && self.enable_comment.is_none()
            && self.enable_join.is_none()
            && self.min_role.is_none()
        {
            return Err(errors::bad_msg("all parameters are missing"));
        }
        validate_link_update(
            self.device_limit,
            self.valid_till,
            &self.pass_hash,
            &self.nonce,
            self.mem_limit,
            self.ops_limit,
            self.disable_password,
        )?;
        if let Some(r) = &self.min_role
            && !role::is_valid_share_role(r)
        {
            return Err(errors::bad_msg(format!("invalid min role {r}")));
        }
        Ok(())
    }
}

/// `CollectionLinkController.UpdateSharedUrl`.
pub async fn update_collection_link(
    state: &AppState,
    app: App,
    req: &UpdateLinkRequest,
) -> ApiResult<PublicUrl> {
    let mut l = active_collection_link(state, req.collection_id).await?;
    if let Some(v) = req.valid_till {
        l.valid_till = v;
    }
    if let Some(v) = req.device_limit {
        l.device_limit = v;
    }
    if req.pass_hash.is_some()
        && req.nonce.is_some()
        && req.ops_limit.is_some()
        && req.mem_limit.is_some()
    {
        l.pass_hash = req.pass_hash.clone();
        l.nonce = req.nonce.clone();
        l.ops_limit = req.ops_limit;
        l.mem_limit = req.mem_limit;
    } else if req.disable_password == Some(true) {
        l.pass_hash = None;
        l.nonce = None;
        l.ops_limit = None;
        l.mem_limit = None;
    }
    if let Some(v) = req.enable_download {
        l.enable_download = v;
    }
    if let Some(v) = req.enable_collect {
        l.enable_collect = v;
    }
    if let Some(v) = req.enable_comment {
        l.enable_comment = v;
    }
    if let Some(v) = req.enable_join {
        l.enable_join = v;
    }
    if let Some(r) = &req.min_role {
        l.min_role = Some(r.clone());
    }
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    sqlx::query(
        "UPDATE public_collection_tokens SET valid_till = $1, device_limit = $2, pw_hash = $3,
            pw_nonce = $4, mem_limit = $5, ops_limit = $6, enable_download = $7, enable_collect = $8,
            enable_comment = $9, enable_join = $10, min_role = $11, updated_at = $12 WHERE id = $13",
    )
    .bind(l.valid_till)
    .bind(l.device_limit)
    .bind(l.pass_hash.clone())
    .bind(l.nonce.clone())
    .bind(l.mem_limit)
    .bind(l.ops_limit)
    .bind(l.enable_download)
    .bind(l.enable_collect)
    .bind(l.enable_comment)
    .bind(l.enable_join)
    .bind(l.min_role.clone())
    .bind(now)
    .bind(l.id)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    touch_collection(tx.conn(), l.collection_id, now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    let mut u = l.to_public_url(state, app);
    // museum reports passwordEnabled from the hash and echoes the KDF params as stored.
    u.password_enabled = l.pass_hash.as_deref().is_some_and(|h| !h.is_empty());
    u.nonce = l.nonce.clone();
    u.mem_limit = l.mem_limit;
    u.ops_limit = l.ops_limit;
    Ok(u)
}

/// `CollectionLinkRepo.DisableSharing` (inside a transaction): disable the active link.
pub async fn disable_collection_link(
    conn: &mut AnyConnection,
    cid: i64,
    now: i64,
) -> Result<(), sqlx::Error> {
    let r = sqlx::query(
        "UPDATE public_collection_tokens SET is_disabled = TRUE, updated_at = $1
         WHERE collection_id = $2 AND is_disabled = FALSE",
    )
    .bind(now)
    .bind(cid)
    .execute(&mut *conn)
    .await?;
    if r.rows_affected() > 0 {
        touch_collection(conn, cid, now).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct LinkPasswordClaim {
    #[serde(rename = "passKey", default)]
    pass_key: String,
    #[serde(rename = "expiryTime", default)]
    expiry_time: i64,
}

/// `validateJWTToken`: the link password JWT (HS256 with the server JWT key, claims
/// `{"passKey","expiryTime"}`) must be valid, unexpired, and carry `pass_hash`. Else 401 `{}`.
pub fn verify_link_password_jwt(state: &AppState, jwt: &str, pass_hash: &str) -> ApiResult<()> {
    use hmac::{Hmac, Mac};
    let bad = errors::unauthorized;
    let mut parts = jwt.split('.');
    let (Some(h), Some(p), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let header: serde_json::Value = crate::crypto::unb64url(h)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(bad)?;
    if header.get("alg").and_then(|a| a.as_str()) != Some("HS256") {
        return Err(bad());
    }
    let sig = crate::crypto::unb64url(s).map_err(|_| bad())?;
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(&state.keys.jwt).map_err(|_| bad())?;
    mac.update(format!("{h}.{p}").as_bytes());
    mac.verify_slice(&sig).map_err(|_| bad())?;
    let claims: LinkPasswordClaim = crate::crypto::unb64url(p)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(bad)?;
    if claims.expiry_time < now_micros() || claims.pass_key != pass_hash {
        return Err(bad());
    }
    Ok(())
}

/// `ente.FileUrl`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileUrl {
    /// Link id (`pft_…`).
    #[serde(rename = "linkID")]
    pub link_id: String,
    /// Shareable URL.
    pub url: String,
    /// Owner.
    #[serde(rename = "ownerID")]
    pub owner_id: i64,
    /// File.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Disabled.
    pub is_disabled: bool,
    /// Expiry.
    pub valid_till: i64,
    /// Device limit.
    pub device_limit: i64,
    /// Password set.
    pub password_enabled: bool,
    /// KDF nonce.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    /// KDF memory limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mem_limit: Option<i64>,
    /// KDF ops limit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ops_limit: Option<i64>,
    /// Downloads allowed.
    pub enable_download: bool,
    /// Creation time.
    pub created_at: i64,
    /// File key encrypted for the link.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_file_key: Option<String>,
    /// Nonce for `encryptedFileKey`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_file_key_nonce: Option<String>,
    /// KDF nonce for the link secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kdf_nonce: Option<String>,
    /// KDF memory limit for the link secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kdf_mem_limit: Option<i64>,
    /// KDF ops limit for the link secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kdf_ops_limit: Option<i64>,
    /// Share key encrypted for the owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_share_key: Option<String>,
}

const FILE_LINK_COLUMNS: &str = "id, file_id, owner_id, app, access_token, valid_till, device_limit,
    is_disabled, pw_hash, pw_nonce, mem_limit, ops_limit, enable_download, created_at, updated_at,
    encrypted_file_key, encrypted_file_key_nonce, kdf_nonce, kdf_mem_limit, kdf_ops_limit, encrypted_share_key";

fn file_url_from_row(state: &AppState, app: App, r: &AnyRow) -> Result<FileUrl, sqlx::Error> {
    let token = get_str(r, "access_token")?;
    Ok(FileUrl {
        link_id: get_str(r, "id")?,
        url: file_url(state, app, &token),
        owner_id: get_i64(r, "owner_id")?,
        file_id: get_i64(r, "file_id")?,
        is_disabled: get_bool(r, "is_disabled")?,
        valid_till: get_i64(r, "valid_till")?,
        device_limit: get_i64(r, "device_limit")?,
        password_enabled: get_opt_str(r, "pw_hash")?.is_some(),
        nonce: get_opt_str(r, "pw_nonce")?,
        mem_limit: get_opt_i64(r, "mem_limit")?,
        ops_limit: get_opt_i64(r, "ops_limit")?,
        enable_download: get_bool(r, "enable_download")?,
        created_at: get_i64(r, "created_at")?,
        encrypted_file_key: get_opt_str(r, "encrypted_file_key")?,
        encrypted_file_key_nonce: get_opt_str(r, "encrypted_file_key_nonce")?,
        kdf_nonce: get_opt_str(r, "kdf_nonce")?,
        kdf_mem_limit: get_opt_i64(r, "kdf_mem_limit")?,
        kdf_ops_limit: get_opt_i64(r, "kdf_ops_limit")?,
        encrypted_share_key: get_opt_str(r, "encrypted_share_key")?,
    })
}

async fn active_file_link_row(state: &AppState, file_id: i64) -> ApiResult<Option<AnyRow>> {
    sqlx::query(&format!(
        "SELECT {FILE_LINK_COLUMNS} FROM public_file_tokens WHERE file_id = $1 AND is_disabled = FALSE"
    ))
    .bind(file_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)
}

/// A file link row (`ente.FileLinkRow`) as the public file-link endpoints need it,
/// including the password hash.
#[derive(Debug, Clone)]
pub struct FileLinkRow {
    /// Link id (`pft_…`).
    pub link_id: String,
    /// File.
    pub file_id: i64,
    /// File owner.
    pub owner_id: i64,
    /// Access token.
    pub access_token: String,
    /// Disabled.
    pub is_disabled: bool,
    /// Expiry (0 = never).
    pub valid_till: i64,
    /// Device limit (0 = unlimited).
    pub device_limit: i64,
    /// Password hash.
    pub pw_hash: Option<String>,
    /// KDF nonce for the password.
    pub pw_nonce: Option<String>,
    /// KDF memory limit for the password.
    pub mem_limit: Option<i64>,
    /// KDF ops limit for the password.
    pub ops_limit: Option<i64>,
    /// Downloads allowed.
    pub enable_download: bool,
    /// File key encrypted for the link.
    pub encrypted_file_key: Option<String>,
    /// Nonce for `encrypted_file_key`.
    pub encrypted_file_key_nonce: Option<String>,
    /// KDF nonce for the link secret.
    pub kdf_nonce: Option<String>,
    /// KDF memory limit for the link secret.
    pub kdf_mem_limit: Option<i64>,
    /// KDF ops limit for the link secret.
    pub kdf_ops_limit: Option<i64>,
}

impl FileLinkRow {
    fn from_row(r: &AnyRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            link_id: get_str(r, "id")?,
            file_id: get_i64(r, "file_id")?,
            owner_id: get_i64(r, "owner_id")?,
            access_token: get_str(r, "access_token")?,
            is_disabled: get_bool(r, "is_disabled")?,
            valid_till: get_i64(r, "valid_till")?,
            device_limit: get_i64(r, "device_limit")?,
            pw_hash: get_opt_str(r, "pw_hash")?,
            pw_nonce: get_opt_str(r, "pw_nonce")?,
            mem_limit: get_opt_i64(r, "mem_limit")?,
            ops_limit: get_opt_i64(r, "ops_limit")?,
            enable_download: get_bool(r, "enable_download")?,
            encrypted_file_key: get_opt_str(r, "encrypted_file_key")?,
            encrypted_file_key_nonce: get_opt_str(r, "encrypted_file_key_nonce")?,
            kdf_nonce: get_opt_str(r, "kdf_nonce")?,
            kdf_mem_limit: get_opt_i64(r, "kdf_mem_limit")?,
            kdf_ops_limit: get_opt_i64(r, "kdf_ops_limit")?,
        })
    }
}

/// `GetFileUrlRowByToken`: the active link with this token, else a disabled one.
pub async fn file_link_by_token(state: &AppState, token: &str) -> ApiResult<Option<FileLinkRow>> {
    let r = sqlx::query(&format!(
        "SELECT {FILE_LINK_COLUMNS} FROM public_file_tokens WHERE access_token = $1
         ORDER BY CASE WHEN is_disabled = FALSE THEN 0 ELSE 1 END, created_at DESC LIMIT 1"
    ))
    .bind(token)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?;
    r.as_ref()
        .map(FileLinkRow::from_row)
        .transpose()
        .map_err(db_err)
}

/// `GetActiveFileUrlToken`: the file's active link, if any.
pub async fn active_file_link(state: &AppState, file_id: i64) -> ApiResult<Option<FileLinkRow>> {
    active_file_link_row(state, file_id)
        .await?
        .as_ref()
        .map(FileLinkRow::from_row)
        .transpose()
        .map_err(db_err)
}

/// `POST /files/share-url` body (`ente.CreateFileUrl`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CreateFileUrlRequest {
    /// File.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// App the link belongs to.
    pub app: String,
    /// File key encrypted for the link.
    pub encrypted_file_key: Option<String>,
    /// Nonce.
    pub encrypted_file_key_nonce: Option<String>,
    /// KDF nonce.
    pub kdf_nonce: Option<String>,
    /// KDF memory limit.
    pub kdf_mem_limit: Option<i64>,
    /// KDF ops limit.
    pub kdf_ops_limit: Option<i64>,
    /// Share key encrypted for the owner.
    pub encrypted_share_key: Option<String>,
}

/// `FileLinkController.CreateLink`: a new link, or the active one (filling empty secrets).
pub async fn create_file_link(
    state: &AppState,
    user_id: i64,
    app: App,
    req: &CreateFileUrlRequest,
) -> ApiResult<FileUrl> {
    validate_kdf(req.kdf_mem_limit, req.kdf_ops_limit)?;
    if req.app != app.as_str() {
        return Err(errors::bad_msg("app mismatch"));
    }
    let (owner, _) = super::files::owner_and_app(state, req.file_id).await?;
    if owner != user_id {
        return Err(errors::denied_msg("not file owner"));
    }
    for _ in 0..5 {
        let token = new_access_token();
        let now = now_micros();
        let mut tx = begin(&state.db).await.map_err(db_err)?;
        if !token_available(tx.conn(), &token).await.map_err(db_err)? {
            continue;
        }
        let res = sqlx::query(
            "INSERT INTO public_file_tokens (id, file_id, owner_id, access_token, app, encrypted_file_key,
                encrypted_file_key_nonce, kdf_nonce, kdf_mem_limit, kdf_ops_limit, encrypted_share_key,
                created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)",
        )
        .bind(new_id("pft"))
        .bind(req.file_id)
        .bind(user_id)
        .bind(&token)
        .bind(&req.app)
        .bind(req.encrypted_file_key.clone())
        .bind(req.encrypted_file_key_nonce.clone())
        .bind(req.kdf_nonce.clone())
        .bind(req.kdf_mem_limit)
        .bind(req.kdf_ops_limit)
        .bind(req.encrypted_share_key.clone())
        .bind(now)
        .execute(tx.conn())
        .await;
        match res {
            Ok(_) => {
                tx.commit().await.map_err(db_err)?;
            }
            Err(e) if super::objects::is_unique_violation(&e) => {
                drop(tx);
                let Some(row) = active_file_link_row(state, req.file_id).await? else {
                    continue;
                };
                let link_id = get_str(&row, "id").map_err(db_err)?;
                sqlx::query(
                    "UPDATE public_file_tokens SET
                        encrypted_file_key = COALESCE(encrypted_file_key, $1),
                        encrypted_file_key_nonce = COALESCE(encrypted_file_key_nonce, $2),
                        kdf_nonce = COALESCE(kdf_nonce, $3),
                        kdf_mem_limit = COALESCE(kdf_mem_limit, $4),
                        kdf_ops_limit = COALESCE(kdf_ops_limit, $5),
                        encrypted_share_key = COALESCE(encrypted_share_key, $6)
                     WHERE id = $7",
                )
                .bind(req.encrypted_file_key.clone())
                .bind(req.encrypted_file_key_nonce.clone())
                .bind(req.kdf_nonce.clone())
                .bind(req.kdf_mem_limit)
                .bind(req.kdf_ops_limit)
                .bind(req.encrypted_share_key.clone())
                .bind(link_id)
                .execute(&state.db.pool)
                .await
                .map_err(db_err)?;
            }
            Err(e) => return Err(db_err(e)),
        }
        let row = active_file_link_row(state, req.file_id)
            .await?
            .ok_or_else(errors::not_found)?;
        return file_url_from_row(state, app, &row).map_err(db_err);
    }
    Err(errors::internal(
        "failed to generate unique access token for file link",
    ))
}

/// `GET /files/share-url(s)`: the owner's links created after `since` (newest update first).
pub async fn list_file_links(
    state: &AppState,
    user_id: i64,
    app: App,
    since: i64,
    limit: i64,
) -> ApiResult<Vec<FileUrl>> {
    let rows = sqlx::query(&format!(
        "SELECT {FILE_LINK_COLUMNS} FROM public_file_tokens
         WHERE owner_id = $1 AND created_at > $2 AND app = $3 ORDER BY updated_at DESC LIMIT $4"
    ))
    .bind(user_id)
    .bind(since)
    .bind(app.as_str())
    .bind(if limit <= 0 { 500 } else { limit })
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    rows.iter()
        .map(|r| file_url_from_row(state, app, r))
        .collect::<Result<_, _>>()
        .map_err(db_err)
}

/// `PUT /files/share-url` body (`ente.UpdateFileUrl`; password fields have no JSON tags
/// in museum, so both `passHash` and `PassHash` spellings are accepted).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct UpdateFileUrlRequest {
    /// Link id.
    #[serde(rename = "linkID")]
    pub link_id: String,
    /// File.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Expiry.
    #[serde(rename = "validTill")]
    pub valid_till: Option<i64>,
    /// Device limit.
    #[serde(rename = "deviceLimit")]
    pub device_limit: Option<i64>,
    /// Password hash.
    #[serde(rename = "passHash", alias = "PassHash", alias = "passhash")]
    pub pass_hash: Option<String>,
    /// KDF nonce.
    #[serde(rename = "nonce", alias = "Nonce")]
    pub nonce: Option<String>,
    /// KDF memory limit.
    #[serde(rename = "memLimit", alias = "MemLimit", alias = "memlimit")]
    pub mem_limit: Option<i64>,
    /// KDF ops limit.
    #[serde(rename = "opsLimit", alias = "OpsLimit", alias = "opslimit")]
    pub ops_limit: Option<i64>,
    /// Downloads.
    #[serde(rename = "enableDownload")]
    pub enable_download: Option<bool>,
    /// Remove the password.
    #[serde(rename = "disablePassword")]
    pub disable_password: Option<bool>,
}

/// `FileLinkController.UpdateSharedUrl`.
pub async fn update_file_link(
    state: &AppState,
    user_id: i64,
    app: App,
    req: &UpdateFileUrlRequest,
) -> ApiResult<FileUrl> {
    if req.device_limit.is_none()
        && req.valid_till.is_none()
        && req.disable_password.is_none()
        && req.nonce.is_none()
        && req.pass_hash.is_none()
        && req.enable_download.is_none()
    {
        return Err(errors::bad_msg("all parameters are missing"));
    }
    validate_link_update(
        req.device_limit,
        req.valid_till,
        &req.pass_hash,
        &req.nonce,
        req.mem_limit,
        req.ops_limit,
        req.disable_password,
    )?;
    let row = active_file_link_row(state, req.file_id)
        .await?
        .ok_or_else(errors::not_found)?;
    if get_i64(&row, "owner_id").map_err(db_err)? != user_id {
        return Err(errors::denied_msg("not file owner"));
    }
    let mut valid_till = get_i64(&row, "valid_till").map_err(db_err)?;
    let mut device_limit = get_i64(&row, "device_limit").map_err(db_err)?;
    let mut pw = (
        get_opt_str(&row, "pw_hash").map_err(db_err)?,
        get_opt_str(&row, "pw_nonce").map_err(db_err)?,
        get_opt_i64(&row, "mem_limit").map_err(db_err)?,
        get_opt_i64(&row, "ops_limit").map_err(db_err)?,
    );
    let mut enable_download = get_bool(&row, "enable_download").map_err(db_err)?;
    if let Some(v) = req.valid_till {
        valid_till = v;
    }
    if let Some(v) = req.device_limit {
        device_limit = v;
    }
    if req.pass_hash.is_some()
        && req.nonce.is_some()
        && req.ops_limit.is_some()
        && req.mem_limit.is_some()
    {
        pw = (
            req.pass_hash.clone(),
            req.nonce.clone(),
            req.mem_limit,
            req.ops_limit,
        );
    } else if req.disable_password == Some(true) {
        pw = (None, None, None, None);
    }
    if let Some(v) = req.enable_download {
        enable_download = v;
    }
    let id = get_str(&row, "id").map_err(db_err)?;
    sqlx::query(
        "UPDATE public_file_tokens SET valid_till = $1, device_limit = $2, pw_hash = $3, pw_nonce = $4,
            mem_limit = $5, ops_limit = $6, enable_download = $7, updated_at = $8 WHERE id = $9",
    )
    .bind(valid_till)
    .bind(device_limit)
    .bind(pw.0)
    .bind(pw.1)
    .bind(pw.2)
    .bind(pw.3)
    .bind(enable_download)
    .bind(now_micros())
    .bind(id)
    .execute(&state.db.pool)
    .await
    .map_err(db_err)?;
    let row = active_file_link_row(state, req.file_id)
        .await?
        .ok_or_else(errors::not_found)?;
    file_url_from_row(state, app, &row).map_err(db_err)
}

/// `FileLinkController.Disable`: owner only.
pub async fn disable_file_link(state: &AppState, user_id: i64, file_id: i64) -> ApiResult<()> {
    let (owner, _) = super::files::owner_and_app(state, file_id).await?;
    if owner != user_id {
        return Err(errors::denied_msg("not file owner"));
    }
    let mut conn = state.db.pool.acquire().await.map_err(db_err)?;
    disable_file_links_for_files(&mut conn, &[file_id], now_micros())
        .await
        .map_err(db_err)
}

/// Disable the active links of `file_ids` (files being trashed or deleted).
pub async fn disable_file_links_for_files(
    conn: &mut AnyConnection,
    file_ids: &[i64],
    now: i64,
) -> Result<(), sqlx::Error> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE public_file_tokens SET is_disabled = TRUE, updated_at = $1
         WHERE is_disabled = FALSE AND file_id IN ({})",
        ids_list(2, file_ids.len())
    );
    super::files::bind_ids(sqlx::query(&sql).bind(now), file_ids)
        .execute(conn)
        .await?;
    Ok(())
}
