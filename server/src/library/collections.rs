//! Collection repository: rows, access checks, listing (owned and shared), sharing,
//! membership changes and the collection file diff, following museum's
//! `CollectionRepository` / `CollectionController`.

use super::errors;
use super::files::{self, FILE_COLUMNS, bind_ids, has_duplicates, lock_files, row_to_file};
use super::model::{
    Collection, CollectionAttributes, CollectionFileItem, CollectionUser, File, MagicMetadata,
    action, filter_public_urls_for_role, role,
};
use super::sql::{
    begin, db_err, get_bool, get_i64, get_opt_i64, get_opt_str, get_str, greatest, ids_list,
};
use super::{actions, links, objects, users};
use crate::auth::App;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use sqlx::AnyConnection;
use sqlx::any::AnyRow;
use std::collections::{BTreeMap, HashSet};

/// museum's `CollectionDiffLimit`.
pub const DIFF_LIMIT: usize = 2500;
/// museum's `DefaultMaxBatchSize`.
pub const MAX_BATCH: usize = 1000;
/// Valid collection types.
pub const VALID_TYPES: [&str; 4] = ["album", "folder", "favorites", "uncategorized"];

/// A `collections` row.
#[derive(Debug, Clone, Default)]
pub struct CollectionRow {
    /// Id.
    pub id: i64,
    /// Owner.
    pub owner_id: i64,
    /// Key encrypted with the owner's master key.
    pub encrypted_key: String,
    /// Nonce.
    pub key_decryption_nonce: String,
    /// Legacy plaintext name.
    pub name: Option<String>,
    /// Encrypted name.
    pub encrypted_name: Option<String>,
    /// Name nonce.
    pub name_decryption_nonce: Option<String>,
    /// Type.
    pub kind: String,
    /// Attributes JSON.
    pub attributes: CollectionAttributes,
    /// Last change.
    pub updation_time: i64,
    /// Deleted.
    pub is_deleted: bool,
    /// Private magic metadata.
    pub magic_metadata: Option<MagicMetadata>,
    /// Public magic metadata.
    pub pub_magic_metadata: Option<MagicMetadata>,
    /// App.
    pub app: String,
}

const COLLECTION_COLUMNS: &str =
    "collection_id, owner_id, encrypted_key, key_decryption_nonce, name,
    encrypted_name, name_decryption_nonce, type, attributes, updation_time, is_deleted,
    magic_metadata, pub_magic_metadata, app";

impl CollectionRow {
    fn from_row(r: &AnyRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: get_i64(r, "collection_id")?,
            owner_id: get_i64(r, "owner_id")?,
            encrypted_key: get_str(r, "encrypted_key")?,
            key_decryption_nonce: get_str(r, "key_decryption_nonce")?,
            name: get_opt_str(r, "name")?,
            encrypted_name: get_opt_str(r, "encrypted_name")?,
            name_decryption_nonce: get_opt_str(r, "name_decryption_nonce")?,
            kind: get_str(r, "type")?,
            attributes: serde_json::from_str(&get_str(r, "attributes")?).unwrap_or_default(),
            updation_time: get_i64(r, "updation_time")?,
            is_deleted: get_bool(r, "is_deleted")?,
            magic_metadata: MagicMetadata::parse(get_opt_str(r, "magic_metadata")?.as_deref()),
            pub_magic_metadata: MagicMetadata::parse(
                get_opt_str(r, "pub_magic_metadata")?.as_deref(),
            ),
            app: get_str(r, "app")?,
        })
    }

    /// The JSON view (owner email empty, sharees and links not loaded).
    pub fn to_collection(&self) -> Collection {
        let mut c = Collection {
            id: self.id,
            owner: CollectionUser {
                id: self.owner_id,
                ..Default::default()
            },
            encrypted_key: self.encrypted_key.clone(),
            key_decryption_nonce: self.key_decryption_nonce.clone(),
            kind: self.kind.clone(),
            attributes: self.attributes.clone(),
            updation_time: self.updation_time,
            is_deleted: self.is_deleted,
            magic_metadata: self.magic_metadata.clone(),
            pub_magic_metadata: self.pub_magic_metadata.clone(),
            app: self.app.clone(),
            ..Default::default()
        };
        match self.name.as_deref() {
            Some(n) if !n.is_empty() => c.name = n.to_string(),
            _ => {
                c.encrypted_name = self.encrypted_name.clone().unwrap_or_default();
                c.name_decryption_nonce = self.name_decryption_nonce.clone().unwrap_or_default();
            }
        }
        c
    }

    /// `AllowSharing`.
    pub fn allow_sharing(&self) -> bool {
        self.kind != "uncategorized"
    }

    /// `AllowParticipantSharing`.
    pub fn allow_participant_sharing(&self, r: &str) -> bool {
        self.kind != "uncategorized" || r == role::VIEWER
    }

    /// `AllowDelete`.
    pub fn allow_delete(&self) -> bool {
        self.kind != "favorites" && self.kind != "uncategorized"
    }

    /// The app as an [`App`].
    pub fn app(&self) -> App {
        App::parse(&self.app)
    }
}

/// Fetch a collection row; missing → 404 `{}`.
pub async fn get_row(state: &AppState, id: i64) -> ApiResult<CollectionRow> {
    let r = sqlx::query(&format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections WHERE collection_id = $1"
    ))
    .bind(id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?
    .ok_or_else(errors::not_found)?;
    CollectionRow::from_row(&r).map_err(db_err)
}

/// museum's `CollectionRepository.Get`: the collection with its active public links.
pub async fn get(state: &AppState, id: i64) -> ApiResult<(CollectionRow, Collection)> {
    let row = get_row(state, id).await?;
    let mut c = row.to_collection();
    let mut urls = links::active_public_urls(state, &[id], row.app()).await?;
    c.public_urls = urls.remove(&id);
    Ok((row, c))
}

/// Bump `collections.updation_time` for `ids`.
pub async fn bump_collections(
    conn: &mut AnyConnection,
    ids: &[i64],
    time: i64,
) -> Result<(), sqlx::Error> {
    if ids.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE collections SET updation_time = $1 WHERE collection_id IN ({})",
        ids_list(2, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(time), ids)
        .execute(conn)
        .await?;
    Ok(())
}

/// The actor's (non-deleted) share role, if any (`GetCollectionShareeRole`).
pub async fn sharee_role(state: &AppState, cid: i64, uid: i64) -> ApiResult<Option<String>> {
    let r = sqlx::query(
        "SELECT role_type FROM collection_shares WHERE collection_id = $1 AND to_user_id = $2 AND is_deleted = FALSE",
    )
    .bind(cid)
    .bind(uid)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?;
    match r {
        None => Ok(None),
        Some(r) => Ok(Some(get_str(&r, "role_type").map_err(db_err)?)),
    }
}

/// Result of an access check.
#[derive(Debug, Clone)]
pub struct Access {
    /// The collection row.
    pub row: CollectionRow,
    /// The JSON view including public links.
    pub collection: Collection,
    /// The actor's role (`OWNER`, or the normalised share role).
    pub role: &'static str,
}

/// museum's `AccessController.GetCollection`.
///
/// * not owner and `verify_owner` → 403 `{}`
/// * not owner and no live share → 404 `{}`
/// * collection app differs from the request app → 403 `{}`
/// * deleted and not `include_deleted` → 404 `{}`
pub async fn access(
    state: &AppState,
    cid: i64,
    actor: i64,
    app: App,
    include_deleted: bool,
    verify_owner: bool,
) -> ApiResult<Access> {
    let (row, collection) = get(state, cid).await?;
    if verify_owner && actor != row.owner_id {
        return Err(errors::denied());
    }
    let r = if actor == row.owner_id {
        role::OWNER
    } else {
        let s = sharee_role(state, cid, actor)
            .await?
            .ok_or_else(errors::not_found)?;
        role::normalize(&s)
    };
    if row.app != app.as_str() {
        return Err(errors::denied());
    }
    if !include_deleted && row.is_deleted {
        return Err(errors::not_found());
    }
    Ok(Access {
        row,
        collection,
        role: r,
    })
}

/// `GetSharees`: live sharees of a collection (deleted accounts excluded), emails decrypted.
pub async fn sharees(state: &AppState, cid: i64) -> ApiResult<Vec<CollectionUser>> {
    let rows = sqlx::query(
        "SELECT u.user_id, u.encrypted_email, u.email_decryption_nonce, cs.role_type
         FROM users u JOIN collection_shares cs
           ON cs.collection_id = $1 AND cs.to_user_id = u.user_id AND cs.is_deleted = FALSE
         WHERE u.is_deleted = FALSE
         ORDER BY cs.shared_at, u.user_id",
    )
    .bind(cid)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(CollectionUser {
            id: get_i64(&r, "user_id").map_err(db_err)?,
            email: users::decrypt_email(
                state,
                get_opt_str(&r, "encrypted_email").map_err(db_err)?,
                get_opt_str(&r, "email_decryption_nonce").map_err(db_err)?,
            )?,
            name: String::new(),
            role: get_str(&r, "role_type").map_err(db_err)?,
        });
    }
    Ok(out)
}

/// `GetCollectionIDsSharedWithUser`.
pub async fn shared_with_user(state: &AppState, uid: i64) -> ApiResult<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT collection_id FROM collection_shares WHERE to_user_id = $1 AND is_deleted = FALSE",
    )
    .bind(uid)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    rows.iter()
        .map(|r| get_i64(r, "collection_id"))
        .collect::<Result<_, _>>()
        .map_err(db_err)
}

/// `GetCollectionsSharedWithOrByUser`.
pub async fn shared_with_or_by_user(state: &AppState, uid: i64) -> ApiResult<Vec<i64>> {
    let rows = sqlx::query(
        "SELECT DISTINCT collection_id FROM collection_shares
         WHERE (to_user_id = $1 OR from_user_id = $1) AND is_deleted = FALSE",
    )
    .bind(uid)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    rows.iter()
        .map(|r| get_i64(r, "collection_id"))
        .collect::<Result<_, _>>()
        .map_err(db_err)
}

/// `WithSharingDetailsForUser` + the non-owner filtering of `GetCollection`.
pub async fn with_sharing_details(
    state: &AppState,
    a: Access,
    actor: i64,
) -> ApiResult<Collection> {
    let mut c = a.collection;
    c.sharees = Some(sharees(state, c.id).await?);
    c.owner.email = users::email_of(state, c.owner.id).await?;
    if actor != c.owner.id {
        let r = sqlx::query(
            "SELECT encrypted_key, magic_metadata, shared_at FROM collection_shares
             WHERE collection_id = $1 AND to_user_id = $2 AND is_deleted = FALSE",
        )
        .bind(c.id)
        .bind(actor)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(db_err)?
        .ok_or_else(|| errors::internal("share key missing"))?;
        c.encrypted_key = get_str(&r, "encrypted_key").map_err(db_err)?;
        c.shared_magic_metadata = MagicMetadata::parse(
            get_opt_str(&r, "magic_metadata")
                .map_err(db_err)?
                .as_deref(),
        );
        c.shared_at = get_opt_i64(&r, "shared_at").map_err(db_err)?;
    }
    if a.role != role::OWNER {
        c.magic_metadata = None;
        c.public_urls = c
            .public_urls
            .map(|u| filter_public_urls_for_role(u, a.role));
    }
    Ok(c)
}

/// `GetCollectionsOwnedByUserV2`: owned collections changed after `since`, oldest first,
/// with sharees and active links; owner email is `""`.
pub async fn owned_since(
    state: &AppState,
    uid: i64,
    since: i64,
    app: App,
    limit: Option<i64>,
) -> ApiResult<Vec<Collection>> {
    let mut sql = format!(
        "SELECT {COLLECTION_COLUMNS} FROM collections
         WHERE owner_id = $1 AND updation_time > $2 AND app = $3 ORDER BY updation_time, collection_id"
    );
    if limit.is_some() {
        sql.push_str(" LIMIT $4");
    }
    let mut q = sqlx::query(&sql).bind(uid).bind(since).bind(app.as_str());
    if let Some(l) = limit {
        q = q.bind(l);
    }
    let rows = q.fetch_all(&state.db.pool).await.map_err(db_err)?;
    let mut out = Vec::with_capacity(rows.len());
    let mut ids = Vec::with_capacity(rows.len());
    for r in &rows {
        let row = CollectionRow::from_row(r).map_err(db_err)?;
        ids.push(row.id);
        out.push(row.to_collection());
    }
    let mut urls = links::active_public_urls(state, &ids, app).await?;
    for c in &mut out {
        c.sharees = Some(sharees(state, c.id).await?);
        c.public_urls = Some(urls.remove(&c.id).unwrap_or_default());
    }
    Ok(out)
}

/// `GetCollectionsSharedWithUser`: collections shared with `uid` whose share or collection
/// changed after `since` (unshared ones appear deleted), ordered by effective time.
pub async fn shared_since(
    state: &AppState,
    uid: i64,
    since: i64,
    app: App,
    limit: Option<i64>,
) -> ApiResult<Vec<Collection>> {
    let eff = greatest(&state.db, "cs.updation_time", "c.updation_time");
    let mut sql = format!(
        "SELECT c.collection_id, c.owner_id, c.type,
            cs.encrypted_key AS share_key, cs.is_deleted AS share_deleted, c.is_deleted AS c_deleted,
            u.encrypted_email, u.email_decryption_nonce,
            c.name, c.encrypted_name, c.name_decryption_nonce, c.app, c.pub_magic_metadata,
            cs.magic_metadata AS share_mmd, cs.role_type, cs.shared_at,
            CASE WHEN cs.is_deleted = TRUE THEN cs.updation_time ELSE {eff} END AS effective_time
         FROM collections c
         JOIN users u ON c.owner_id = u.user_id
         JOIN collection_shares cs ON c.collection_id = cs.collection_id
         WHERE cs.to_user_id = $1
           AND (cs.is_deleted = TRUE OR c.is_deleted = TRUE OR u.is_deleted = FALSE)
           AND c.app = $3
           AND ((cs.is_deleted = FALSE AND (cs.updation_time > $2 OR c.updation_time > $2))
                OR (cs.is_deleted = TRUE AND cs.updation_time > $2))
         ORDER BY effective_time, c.collection_id"
    );
    // $3 appears before $2's second use but after its first, so the order rule holds.
    if limit.is_some() {
        sql.push_str(" LIMIT $4");
    }
    let mut q = sqlx::query(&sql).bind(uid).bind(since).bind(app.as_str());
    if let Some(l) = limit {
        q = q.bind(l);
    }
    let rows = q.fetch_all(&state.db.pool).await.map_err(db_err)?;
    let mut out = Vec::with_capacity(rows.len());
    let mut live_ids = Vec::new();
    let mut roles = BTreeMap::new();
    for r in &rows {
        let share_deleted = get_bool(r, "share_deleted").map_err(db_err)?;
        let deleted = share_deleted || get_bool(r, "c_deleted").map_err(db_err)?;
        let mut c = Collection {
            id: get_i64(r, "collection_id").map_err(db_err)?,
            owner: CollectionUser {
                id: get_i64(r, "owner_id").map_err(db_err)?,
                ..Default::default()
            },
            encrypted_key: get_str(r, "share_key").map_err(db_err)?,
            kind: get_str(r, "type").map_err(db_err)?,
            updation_time: get_i64(r, "effective_time").map_err(db_err)?,
            is_deleted: deleted,
            ..Default::default()
        };
        if !share_deleted {
            c.app = get_str(r, "app").map_err(db_err)?;
            c.pub_magic_metadata = MagicMetadata::parse(
                get_opt_str(r, "pub_magic_metadata")
                    .map_err(db_err)?
                    .as_deref(),
            );
            c.shared_magic_metadata =
                MagicMetadata::parse(get_opt_str(r, "share_mmd").map_err(db_err)?.as_deref());
            c.shared_at = get_opt_i64(r, "shared_at").map_err(db_err)?;
            match get_opt_str(r, "name").map_err(db_err)? {
                Some(n) if !n.is_empty() => c.name = n,
                _ => {
                    c.encrypted_name = get_opt_str(r, "encrypted_name")
                        .map_err(db_err)?
                        .unwrap_or_default();
                    c.name_decryption_nonce = get_opt_str(r, "name_decryption_nonce")
                        .map_err(db_err)?
                        .unwrap_or_default();
                }
            }
        }
        if !deleted {
            c.owner.email = users::decrypt_email(
                state,
                get_opt_str(r, "encrypted_email").map_err(db_err)?,
                get_opt_str(r, "email_decryption_nonce").map_err(db_err)?,
            )?;
            c.sharees = Some(sharees(state, c.id).await?);
            live_ids.push(c.id);
        } else {
            c.sharees = Some(Vec::new());
        }
        let r_role = if share_deleted {
            role::VIEWER
        } else {
            match role::normalize(&get_str(r, "role_type").map_err(db_err)?) {
                role::UNKNOWN => role::VIEWER,
                x => x,
            }
        };
        roles.insert(c.id, r_role);
        out.push(c);
    }
    let mut urls = links::active_public_urls(state, &live_ids, app).await?;
    for c in &mut out {
        c.public_urls = Some(if c.is_deleted {
            Vec::new()
        } else {
            filter_public_urls_for_role(
                urls.remove(&c.id).unwrap_or_default(),
                roles.get(&c.id).copied().unwrap_or(role::VIEWER),
            )
        });
    }
    Ok(out)
}

/// The shape museum sends for a shared collection that is deleted or no longer shared.
pub fn shared_response(c: Collection) -> serde_json::Value {
    if !c.is_deleted {
        return serde_json::to_value(c).unwrap_or_default();
    }
    serde_json::json!({
        "id": c.id,
        "owner": {"id": c.owner.id, "email": ""},
        "encryptedKey": c.encrypted_key,
        "type": c.kind,
        "attributes": {},
        "updationTime": c.updation_time,
        "isDeleted": true,
    })
}

/// `ente.ValidateBase64Length`: strict base64 of exactly `size` bytes.
pub fn validate_b64_len(v: &str, size: usize) -> ApiResult<()> {
    if v.len() != size.div_ceil(3) * 4 {
        return Err(errors::bad_msg("invalid length"));
    }
    match crate::crypto::unb64(v) {
        Err(_) => Err(errors::bad_msg("invalid base64")),
        Ok(d) if d.len() != size => Err(errors::bad_msg("invalid length")),
        // Strict decoding: the canonical re-encoding must match (no stray padding bits).
        Ok(d) if crate::crypto::b64(&d) != v => Err(errors::bad_msg("invalid base64")),
        Ok(_) => Ok(()),
    }
}

/// `validateOwnedCollectionKey`: secretbox of a 32-byte key (48 bytes) and a 24-byte nonce.
pub fn validate_owned_key(key: &str, nonce: &str) -> ApiResult<()> {
    validate_b64_len(key, 48)?;
    validate_b64_len(nonce, 24)
}

/// `validateSealedCollectionKey`: a sealed box of a 32-byte key (80 bytes).
pub fn validate_sealed_key(key: &str) -> ApiResult<()> {
    validate_b64_len(key, 80)
}

/// `POST /collections` body.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CreateRequest {
    /// Key encrypted with the master key.
    pub encrypted_key: String,
    /// Nonce.
    pub key_decryption_nonce: String,
    /// Legacy name.
    pub name: String,
    /// Encrypted name.
    pub encrypted_name: String,
    /// Name nonce.
    pub name_decryption_nonce: String,
    /// Type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Attributes.
    pub attributes: CollectionAttributes,
    /// Private magic metadata.
    pub magic_metadata: Option<MagicMetadata>,
    /// Public magic metadata.
    #[serde(rename = "pubMagicMetadata")]
    pub pub_magic_metadata: Option<MagicMetadata>,
}

/// `CollectionController.Create`. Favorites/uncategorized are unique per owner and app; a
/// second create returns the existing one.
pub async fn create(
    state: &AppState,
    uid: i64,
    app: App,
    req: CreateRequest,
) -> ApiResult<Collection> {
    validate_owned_key(&req.encrypted_key, &req.key_decryption_nonce)?;
    users::require_key_attributes(state, uid).await?;
    let requested_type = req.kind.clone();
    let kind = if req.kind == "CollectionType.album" {
        "album".to_string()
    } else {
        req.kind.clone()
    };
    if !VALID_TYPES.contains(&kind.as_str()) {
        return Err(errors::internal(format!(
            "unexpected collection type {kind}"
        )));
    }
    let now = now_micros();
    let attrs = serde_json::to_string(&req.attributes).unwrap_or_else(|_| "{}".into());
    let res = sqlx::query(
        "INSERT INTO collections (owner_id, encrypted_key, key_decryption_nonce, name, encrypted_name,
            name_decryption_nonce, type, attributes, updation_time, magic_metadata, pub_magic_metadata, app)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12) RETURNING collection_id",
    )
    .bind(uid)
    .bind(&req.encrypted_key)
    .bind(&req.key_decryption_nonce)
    .bind(&req.name)
    .bind(&req.encrypted_name)
    .bind(&req.name_decryption_nonce)
    .bind(&kind)
    .bind(attrs)
    .bind(now)
    .bind(req.magic_metadata.as_ref().map(MagicMetadata::to_db))
    .bind(req.pub_magic_metadata.as_ref().map(MagicMetadata::to_db))
    .bind(app.as_str())
    .fetch_one(&state.db.pool)
    .await;
    match res {
        Ok(r) => Ok(Collection {
            id: get_i64(&r, "collection_id").map_err(db_err)?,
            owner: CollectionUser {
                id: uid,
                ..Default::default()
            },
            encrypted_key: req.encrypted_key,
            key_decryption_nonce: req.key_decryption_nonce,
            name: req.name,
            encrypted_name: req.encrypted_name,
            name_decryption_nonce: req.name_decryption_nonce,
            kind,
            attributes: req.attributes,
            sharees: None,
            public_urls: None,
            updation_time: now,
            magic_metadata: req.magic_metadata,
            app: app.as_str().to_string(),
            pub_magic_metadata: req.pub_magic_metadata,
            ..Default::default()
        }),
        Err(e)
            if objects::is_unique_violation(&e)
                && (kind == "favorites" || kind == "uncategorized") =>
        {
            let r = sqlx::query(&format!(
                "SELECT {COLLECTION_COLUMNS} FROM collections WHERE owner_id = $1 AND type = $2 AND app = $3"
            ))
            .bind(uid)
            .bind(&requested_type)
            .bind(app.as_str())
            .fetch_optional(&state.db.pool)
            .await
            .map_err(db_err)?
            .ok_or_else(errors::not_found)?;
            let row = CollectionRow::from_row(&r).map_err(db_err)?;
            if row.is_deleted {
                return Err(errors::internal("special collection is deleted"));
            }
            Ok(row.to_collection())
        }
        Err(e) => Err(db_err(e)),
    }
}

/// `collectionForShareMutation`: the owner, or an ADMIN sharee; otherwise 403 `{}`.
pub async fn for_share_mutation(
    state: &AppState,
    cid: i64,
    actor: i64,
) -> ApiResult<CollectionRow> {
    let row = get_row(state, cid).await?;
    if actor == row.owner_id {
        return Ok(row);
    }
    match sharee_role(state, cid, actor).await? {
        Some(r) if r == role::ADMIN => Ok(row),
        _ => Err(errors::denied()),
    }
}

fn validate_share_recipient(actor: i64, owner: i64, recipient: i64) -> ApiResult<()> {
    if recipient == actor || recipient == owner {
        return Err(errors::bad());
    }
    Ok(())
}

/// `CollectionRepository.Share`: upsert a share (undeleting it) and bump the collection.
pub async fn upsert_share(
    state: &AppState,
    cid: i64,
    from: i64,
    to: i64,
    encrypted_key: &str,
    r: &str,
    now: i64,
) -> ApiResult<()> {
    if !matches!(r, role::VIEWER | role::COLLABORATOR | role::ADMIN) {
        return Err(errors::internal(format!("invalid role {r}")));
    }
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    sqlx::query(
        "INSERT INTO collection_shares (collection_id, from_user_id, to_user_id, encrypted_key,
            updation_time, role_type, shared_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (collection_id, from_user_id, to_user_id) DO UPDATE SET
            is_deleted = FALSE, updation_time = $5, role_type = $6, encrypted_key = $4,
            shared_at = CASE WHEN collection_shares.is_deleted = TRUE THEN $7 ELSE collection_shares.shared_at END",
    )
    .bind(cid)
    .bind(from)
    .bind(to)
    .bind(encrypted_key)
    .bind(now)
    .bind(r)
    .bind(now)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    let res = sqlx::query(
        "UPDATE collections SET updation_time = $1 WHERE collection_id = $2 AND is_deleted = FALSE",
    )
    .bind(now)
    .bind(cid)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    if res.rows_affected() == 0 {
        return Err(errors::collection_deleted());
    }
    tx.commit().await.map_err(db_err)
}

/// `POST /collections/share` body (`ente.AlterShareRequest`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShareRequest {
    /// Collection.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Recipient email.
    pub email: String,
    /// Collection key sealed to the recipient's public key.
    pub encrypted_key: String,
    /// Role (default VIEWER).
    pub role: Option<String>,
}

/// `CollectionController.Share`: returns the sharees after the change.
pub async fn share(
    state: &AppState,
    actor: i64,
    app: App,
    req: &ShareRequest,
) -> ApiResult<Vec<CollectionUser>> {
    validate_sealed_key(&req.encrypted_key)?;
    let r = req.role.clone().unwrap_or_else(|| role::VIEWER.into());
    let c = for_share_mutation(state, req.collection_id, actor).await?;
    if !c.allow_participant_sharing(&r) {
        return Err(errors::bad());
    }
    let to = users::lookup_user_id(state, actor, &req.email)
        .await?
        .ok_or_else(errors::not_found)?;
    validate_share_recipient(actor, c.owner_id, to)?;
    upsert_share(
        state,
        c.id,
        c.owner_id,
        to,
        &req.encrypted_key,
        &r,
        now_micros(),
    )
    .await?;
    access(state, c.id, actor, app, false, false).await?;
    sharees(state, c.id).await
}

/// `CollectionController.BatchShare`.
pub async fn batch_share(
    state: &AppState,
    actor: i64,
    app: App,
    shares: &[ShareRequest],
) -> ApiResult<Vec<CollectionUser>> {
    if shares.is_empty() {
        return Err(errors::bad());
    }
    if shares.len() > 10 {
        return Err(errors::too_large());
    }
    let cid = shares[0].collection_id;
    let mut seen = HashSet::new();
    for s in shares {
        if s.collection_id != cid {
            return Err(errors::bad());
        }
        let e = users::normalize_email(&s.email);
        if e.is_empty() || !seen.insert(e) {
            return Err(errors::bad());
        }
    }
    let c = for_share_mutation(state, cid, actor).await?;
    if c.is_deleted {
        return Err(errors::collection_deleted());
    }
    let mut resolved = Vec::with_capacity(shares.len());
    for s in shares {
        validate_sealed_key(&s.encrypted_key)?;
        let r = s.role.clone().unwrap_or_else(|| role::VIEWER.into());
        if !matches!(r.as_str(), role::VIEWER | role::COLLABORATOR | role::ADMIN) {
            return Err(errors::bad_msg(format!("invalid role {r}")));
        }
        if !c.allow_participant_sharing(&r) {
            return Err(errors::bad());
        }
        let to = users::lookup_user_id(state, actor, &s.email)
            .await?
            .ok_or_else(errors::not_found)?;
        validate_share_recipient(actor, c.owner_id, to)?;
        resolved.push((to, s.encrypted_key.clone(), r));
    }
    resolved.sort_by_key(|r| r.0);
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    for (to, key, r) in &resolved {
        sqlx::query(
            "INSERT INTO collection_shares (collection_id, from_user_id, to_user_id, encrypted_key,
                updation_time, role_type, shared_at)
             VALUES ($1, $2, $3, $4, $5, $6, $5)
             ON CONFLICT (collection_id, from_user_id, to_user_id) DO UPDATE SET
                is_deleted = FALSE, updation_time = $5, role_type = $6, encrypted_key = $4,
                shared_at = CASE WHEN collection_shares.is_deleted = TRUE THEN $5 ELSE collection_shares.shared_at END",
        )
        .bind(cid)
        .bind(c.owner_id)
        .bind(*to)
        .bind(key)
        .bind(now)
        .bind(r)
        .execute(tx.conn())
        .await
        .map_err(db_err)?;
    }
    let res = sqlx::query(
        "UPDATE collections SET updation_time = $1 WHERE collection_id = $2 AND is_deleted = FALSE",
    )
    .bind(now)
    .bind(cid)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    if res.rows_affected() == 0 {
        return Err(errors::collection_deleted());
    }
    tx.commit().await.map_err(db_err)?;
    access(state, cid, actor, app, false, false).await?;
    sharees(state, cid).await
}

/// One `/collections/share/bulk` item.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BulkShareItem {
    /// Collection.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Sealed collection key.
    pub encrypted_key: String,
    /// Role.
    pub role: String,
}

/// `/collections/share/bulk` body.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BulkShareRequest {
    /// Recipient.
    #[serde(rename = "recipientUserID")]
    pub recipient_user_id: i64,
    /// Recipient email (must belong to the recipient).
    pub recipient_email: String,
    /// `manual` or `automatic`.
    pub source: String,
    /// Collections.
    pub collections: Vec<BulkShareItem>,
}

/// Per-collection result of a bulk (un)share.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BulkResult {
    /// Collection.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// Status string (`shared`, `already_shared`, `unshared`, …, `failed`).
    pub status: &'static str,
}

fn valid_source(s: &str) -> bool {
    s == "manual" || s == "automatic"
}

/// `CollectionController.BulkShare`. Automatic shares require the same family; families
/// are not supported by this server, so they are always `AUTOMATIC_SHARE_RECIPIENT_NOT_ELIGIBLE`.
pub async fn bulk_share(
    state: &AppState,
    actor: i64,
    req: &BulkShareRequest,
) -> ApiResult<Vec<BulkResult>> {
    if !valid_source(&req.source) || req.collections.is_empty() {
        return Err(errors::bad());
    }
    if req.collections.len() > 100 {
        return Err(errors::too_large());
    }
    let ids: Vec<i64> = req.collections.iter().map(|c| c.collection_id).collect();
    if has_duplicates(&ids) {
        return Err(errors::bad());
    }
    if req.recipient_user_id <= 0 || req.recipient_user_id == actor {
        return Err(errors::bad());
    }
    users::verify_user_id(state, actor, &req.recipient_email, req.recipient_user_id).await?;
    if req.source == "automatic" {
        return Err(errors::automatic_share_not_eligible());
    }
    let mut out = Vec::with_capacity(req.collections.len());
    for item in &req.collections {
        let res: ApiResult<&'static str> = async {
            validate_sealed_key(&item.encrypted_key)?;
            let c = for_share_mutation(state, item.collection_id, actor).await?;
            if !c.allow_participant_sharing(&item.role) {
                return Err(errors::bad());
            }
            validate_share_recipient(actor, c.owner_id, req.recipient_user_id)?;
            upsert_share(
                state,
                c.id,
                c.owner_id,
                req.recipient_user_id,
                &item.encrypted_key,
                &item.role,
                now_micros(),
            )
            .await?;
            Ok("shared")
        }
        .await;
        out.push(BulkResult {
            collection_id: item.collection_id,
            status: res.unwrap_or_else(|e| {
                tracing::warn!(status = %e.status, "bulk collection share failed");
                "failed"
            }),
        });
    }
    Ok(out)
}

/// `UnShareContext`: returns `not_shared`, `already_unshared` or `unshared`.
pub async fn unshare_user(state: &AppState, cid: i64, to: i64) -> ApiResult<&'static str> {
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    let r = sqlx::query(&format!(
        "SELECT is_deleted FROM collection_shares WHERE collection_id = $1 AND to_user_id = $2{}",
        super::sql::for_update(&state.db)
    ))
    .bind(cid)
    .bind(to)
    .fetch_optional(tx.conn())
    .await
    .map_err(db_err)?;
    let Some(r) = r else {
        return Ok("not_shared");
    };
    let status = if get_bool(&r, "is_deleted").map_err(db_err)? {
        "already_unshared"
    } else {
        "unshared"
    };
    sqlx::query(
        "UPDATE collection_shares SET is_deleted = TRUE, updation_time = $1
         WHERE collection_id = $2 AND to_user_id = $3 AND is_deleted = FALSE",
    )
    .bind(now)
    .bind(cid)
    .bind(to)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    sqlx::query(
        "UPDATE collection_files SET is_deleted = TRUE, updation_time = $1
         WHERE collection_id = $2 AND f_owner_id = $3 AND is_deleted = FALSE",
    )
    .bind(now)
    .bind(cid)
    .bind(to)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    bump_collections(tx.conn(), &[cid], now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(status)
}

/// museum's `cleanupRevokedShare`: soft-delete the revoked participant's comments and
/// reactions in the collection and revoke their cast sessions for it. Best effort: the
/// share is already gone, so failures are logged rather than returned.
pub async fn cleanup_revoked_share(state: &AppState, cid: i64, uid: i64) {
    if let Err(e) = crate::social::remove_user_social_activity(&state.db, cid, uid).await {
        tracing::warn!(collection_id = cid, user_id = uid, error = ?e, "social cleanup after unshare failed");
    }
    if let Err(e) =
        crate::public::cast::revoke_for_user_and_collection(&state.db.pool, cid, uid).await
    {
        tracing::warn!(collection_id = cid, user_id = uid, error = %e, "cast revoke after unshare failed");
    }
}

/// `CollectionController.HandleAccountDeletion` (`ResetUserSharingAccess`): end every share
/// with or by the user (cleaning up the sharee's comments, reactions and cast sessions),
/// revoke the user's cast sessions, and disable the user's album and file links.
/// Memory shares are deleted separately (`public::memory::delete_for_user`).
pub async fn handle_account_deletion(state: &AppState, uid: i64) -> ApiResult<()> {
    let rows = sqlx::query(
        "SELECT collection_id, to_user_id FROM collection_shares
         WHERE (from_user_id = $1 OR to_user_id = $1) AND is_deleted = FALSE",
    )
    .bind(uid)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    for r in &rows {
        let cid = get_i64(r, "collection_id").map_err(db_err)?;
        let to = get_i64(r, "to_user_id").map_err(db_err)?;
        unshare_user(state, cid, to).await?;
        crate::social::remove_user_social_activity(&state.db, cid, to).await?;
    }
    crate::public::cast::revoke_for_user(&state.db.pool, uid)
        .await
        .map_err(db_err)?;
    let now = now_micros();
    let owned = sqlx::query(
        "SELECT t.collection_id FROM public_collection_tokens t
         JOIN collections c ON c.collection_id = t.collection_id
         WHERE c.owner_id = $1 AND t.is_disabled = FALSE",
    )
    .bind(uid)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    for r in &owned {
        let cid = get_i64(r, "collection_id").map_err(db_err)?;
        links::disable_collection_link(tx.conn(), cid, now)
            .await
            .map_err(db_err)?;
    }
    sqlx::query(
        "UPDATE public_file_tokens SET is_disabled = TRUE, updated_at = $1
         WHERE owner_id = $2 AND is_disabled = FALSE",
    )
    .bind(now)
    .bind(uid)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// `CollectionController.UnShare` (`{collectionID, email}`): returns the remaining sharees.
pub async fn unshare(
    state: &AppState,
    actor: i64,
    cid: i64,
    email: &str,
) -> ApiResult<Vec<CollectionUser>> {
    let c = for_share_mutation(state, cid, actor).await?;
    let mut list = sharees(state, cid).await?;
    let target = users::normalize_email(email);
    let idx = list
        .iter()
        .position(|s| users::normalize_email(&s.email) == target)
        .ok_or_else(errors::not_found)?;
    let to = list[idx].id;
    if to == actor || to == c.owner_id {
        return Err(errors::denied());
    }
    unshare_user(state, cid, to).await?;
    cleanup_revoked_share(state, cid, to).await;
    list.remove(idx);
    Ok(list)
}

/// `/collections/unshare/bulk` body.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BulkUnshareRequest {
    /// Recipient.
    #[serde(rename = "recipientUserID")]
    pub recipient_user_id: i64,
    /// `manual` or `automatic`.
    pub source: String,
    /// Collections.
    #[serde(rename = "collectionIDs")]
    pub collection_ids: Vec<i64>,
}

/// `CollectionController.BulkUnShare`.
pub async fn bulk_unshare(
    state: &AppState,
    actor: i64,
    req: &BulkUnshareRequest,
) -> ApiResult<Vec<BulkResult>> {
    if !valid_source(&req.source) || req.collection_ids.is_empty() {
        return Err(errors::bad());
    }
    if req.collection_ids.len() > 100 {
        return Err(errors::too_large());
    }
    if has_duplicates(&req.collection_ids) {
        return Err(errors::bad());
    }
    if req.recipient_user_id <= 0 || req.recipient_user_id == actor {
        return Err(errors::bad());
    }
    let mut out = Vec::with_capacity(req.collection_ids.len());
    for cid in &req.collection_ids {
        let res: ApiResult<&'static str> = async {
            let c = for_share_mutation(state, *cid, actor).await?;
            if req.source == "automatic" && actor != c.owner_id {
                return Err(errors::denied());
            }
            validate_share_recipient(actor, c.owner_id, req.recipient_user_id)?;
            let status = unshare_user(state, *cid, req.recipient_user_id).await?;
            if status != "not_shared" {
                cleanup_revoked_share(state, *cid, req.recipient_user_id).await;
            }
            Ok(status)
        }
        .await;
        out.push(BulkResult {
            collection_id: *cid,
            status: res.unwrap_or("failed"),
        });
    }
    Ok(out)
}

/// `CollectionController.Leave`.
pub async fn leave(state: &AppState, actor: i64, cid: i64) -> ApiResult<()> {
    let c = get_row(state, cid).await?;
    if c.owner_id == actor {
        return Err(errors::denied());
    }
    if !shared_with_user(state, actor).await?.contains(&cid) {
        return Ok(());
    }
    unshare_user(state, cid, actor).await?;
    cleanup_revoked_share(state, cid, actor).await;
    Ok(())
}

/// `CollectionController.JoinViaLink`. `access_token`/`access_jwt` come from the
/// `X-Auth-Access-Token(-JWT)` headers or `accessToken(JWT)` query parameters.
pub async fn join_link(
    state: &AppState,
    actor: i64,
    cid: i64,
    encrypted_key: &str,
    access_token: &str,
    access_jwt: &str,
) -> ApiResult<()> {
    validate_sealed_key(encrypted_key)?;
    let c = get_row(state, cid).await?;
    if c.owner_id == actor || !c.allow_sharing() {
        return Err(errors::bad());
    }
    let link = links::active_collection_link(state, cid).await?;
    link.can_join().map_err(|_| errors::bad())?;
    if link.token != access_token {
        return Err(errors::denied());
    }
    if let Some(h) = link.pass_hash.as_deref().filter(|h| !h.is_empty()) {
        links::verify_link_password_jwt(state, access_jwt, h)?;
    }
    let r = if link.enable_collect {
        role::COLLABORATOR
    } else {
        role::VIEWER
    };
    upsert_share(
        state,
        cid,
        c.owner_id,
        actor,
        encrypted_key,
        r,
        now_micros(),
    )
    .await
}

/// `upsertCollectionFiles`: add or revive memberships, clearing any pending action.
pub async fn upsert_collection_files(
    conn: &mut AnyConnection,
    cid: i64,
    c_owner: i64,
    items: &[CollectionFileItem],
    f_owner: i64,
    now: i64,
) -> Result<(), sqlx::Error> {
    let mut seen = HashSet::new();
    let mut sorted: Vec<&CollectionFileItem> = items.iter().filter(|i| seen.insert(i.id)).collect();
    sorted.sort_by_key(|i| i.id);
    for f in sorted {
        sqlx::query(
            "INSERT INTO collection_files (collection_id, file_id, encrypted_key, key_decryption_nonce,
                is_deleted, updation_time, c_owner_id, f_owner_id, created_at)
             VALUES ($1, $2, $3, $4, FALSE, $5, $6, $7, $5)
             ON CONFLICT (collection_id, file_id) DO UPDATE SET
                is_deleted = FALSE, updation_time = $5, action_user = NULL, action = NULL,
                encrypted_key = CASE WHEN collection_files.is_deleted = TRUE THEN $3 ELSE collection_files.encrypted_key END,
                key_decryption_nonce = CASE WHEN collection_files.is_deleted = TRUE THEN $4 ELSE collection_files.key_decryption_nonce END,
                created_at = CASE WHEN collection_files.is_deleted = TRUE THEN $5 ELSE collection_files.created_at END",
        )
        .bind(cid)
        .bind(f.id)
        .bind(&f.encrypted_key)
        .bind(&f.key_decryption_nonce)
        .bind(now)
        .bind(c_owner)
        .bind(f_owner)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// `getFilesInTrashOrDeleted`: ids among `ids` that are in the owner's trash (not restored).
pub async fn files_in_trash_or_deleted(
    conn: &mut AnyConnection,
    owner: i64,
    ids: &[i64],
) -> Result<Vec<i64>, sqlx::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT file_id FROM trash WHERE user_id = $1 AND is_restored = FALSE AND file_id IN ({})",
        ids_list(2, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(owner), ids)
        .fetch_all(conn)
        .await?
        .iter()
        .map(|r| get_i64(r, "file_id"))
        .collect()
}

/// `CollectionController.AddFiles`.
pub async fn add_files(
    state: &AppState,
    actor: i64,
    app: App,
    cid: i64,
    items: &[CollectionFileItem],
) -> ApiResult<()> {
    files::validate_file_items(items)?;
    let a = access(state, cid, actor, app, false, false).await?;
    if a.row.app != app.as_str() {
        return Err(errors::internal("app mismatch"));
    }
    if !role::can_add(a.role) {
        return Err(errors::denied());
    }
    let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
    files::verify_file_ownership(state, actor, &ids).await?;
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    lock_files(&mut tx, state, actor, &ids)
        .await
        .map_err(db_err)?;
    if !files_in_trash_or_deleted(tx.conn(), actor, &ids)
        .await
        .map_err(db_err)?
        .is_empty()
    {
        return Err(errors::file_in_trash());
    }
    upsert_collection_files(tx.conn(), cid, a.row.owner_id, items, actor, now)
        .await
        .map_err(db_err)?;
    bump_collections(tx.conn(), &[cid], now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)
}

/// `CollectionController.MoveFiles`.
pub async fn move_files(
    state: &AppState,
    actor: i64,
    app: App,
    from: i64,
    to: i64,
    items: &[CollectionFileItem],
) -> ApiResult<()> {
    if from == to {
        return Err(errors::bad_msg(
            "source and destination collections must differ",
        ));
    }
    files::validate_file_items(items)?;
    let r1 = access(state, from, actor, app, false, true).await?;
    let r2 = access(state, to, actor, app, false, true).await?;
    if r1.row.app != r2.row.app {
        return Err(errors::internal("move across app not supported"));
    }
    let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
    files::verify_file_ownership(state, actor, &ids).await?;
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    lock_files(&mut tx, state, actor, &ids)
        .await
        .map_err(db_err)?;
    if !files_in_trash_or_deleted(tx.conn(), actor, &ids)
        .await
        .map_err(db_err)?
        .is_empty()
    {
        return Err(errors::file_in_trash());
    }
    upsert_collection_files(tx.conn(), to, actor, items, actor, now)
        .await
        .map_err(db_err)?;
    let sql = format!(
        "UPDATE collection_files SET is_deleted = TRUE, updation_time = $1
         WHERE collection_id = $2 AND file_id IN ({})",
        ids_list(3, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(now).bind(from), &ids)
        .execute(tx.conn())
        .await
        .map_err(db_err)?;
    bump_collections(tx.conn(), &[from, to], now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)
}

/// `CollectionController.RestoreFiles`: restore trashed files into an owned collection.
pub async fn restore_files(
    state: &AppState,
    actor: i64,
    app: App,
    cid: i64,
    items: &[CollectionFileItem],
) -> ApiResult<()> {
    files::validate_file_items(items)?;
    access(state, cid, actor, app, false, true).await?;
    for f in items {
        let (owner, _) = files::owner_and_app(state, f.id).await?;
        if owner != actor {
            return Err(errors::denied());
        }
    }
    let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    lock_files(&mut tx, state, actor, &ids)
        .await
        .map_err(db_err)?;
    let in_trash = super::trash::files_in_trash_state(tx.conn(), actor, &ids)
        .await
        .map_err(db_err)?;
    let unique: HashSet<i64> = ids.iter().copied().collect();
    if in_trash.len() != unique.len() || in_trash.len() != ids.len() {
        return Err(errors::bad());
    }
    upsert_collection_files(tx.conn(), cid, actor, items, actor, now)
        .await
        .map_err(db_err)?;
    let res = sqlx::query(
        "UPDATE collections SET updation_time = $1
         WHERE collection_id = $2 AND owner_id = $3 AND is_deleted = FALSE",
    )
    .bind(now)
    .bind(cid)
    .bind(actor)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    if res.rows_affected() == 0 {
        return Err(errors::denied());
    }
    let sql = format!(
        "UPDATE trash SET is_restored = TRUE, updated_at = $1 WHERE user_id = $2 AND file_id IN ({})",
        ids_list(3, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(now).bind(actor), &ids)
        .execute(tx.conn())
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)
}

/// `FilterActiveFileIDsInCollection`: ids live in the collection (not deleted, not
/// REMOVE-pending). An id with no membership at all is an error (500).
pub async fn filter_active(state: &AppState, cid: i64, ids: &[i64]) -> ApiResult<Vec<i64>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT file_id, is_deleted, action FROM collection_files WHERE collection_id = $1 AND file_id IN ({})",
        ids_list(2, ids.len())
    );
    let rows = bind_ids(sqlx::query(&sql).bind(cid), ids)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?;
    let mut states = std::collections::HashMap::new();
    for r in rows {
        states.insert(
            get_i64(&r, "file_id").map_err(db_err)?,
            (
                get_bool(&r, "is_deleted").map_err(db_err)?,
                get_opt_str(&r, "action").map_err(db_err)?,
            ),
        );
    }
    let mut out = Vec::new();
    for id in ids {
        match states.get(id) {
            None => return Err(errors::internal("file not found in collection")),
            Some((false, a)) if a.as_deref() != Some(action::REMOVE) => out.push(*id),
            _ => {}
        }
    }
    Ok(out)
}

/// Mark memberships deleted (`CollectionRepository.RemoveFilesV3`).
async fn remove_memberships(
    state: &AppState,
    cid: i64,
    c_owner: i64,
    ids: &[i64],
) -> ApiResult<()> {
    let map = files::owner_to_file_ids(state, ids).await?;
    if map.contains_key(&c_owner) {
        return Err(errors::internal(
            "can not remove files owned by album owner",
        ));
    }
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    let sql = format!(
        "UPDATE collection_files SET is_deleted = TRUE, updation_time = $1
         WHERE collection_id = $2 AND file_id IN ({})",
        ids_list(3, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(now).bind(cid), ids)
        .execute(tx.conn())
        .await
        .map_err(db_err)?;
    bump_collections(tx.conn(), &[cid], now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)
}

/// `CollectionRepository.SuggestAction`: flag memberships with a pending action.
async fn suggest_action(
    state: &AppState,
    cid: i64,
    actor: i64,
    ids: &[i64],
    act: &str,
) -> ApiResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    let sql = format!(
        "UPDATE collection_files SET action_user = $1, action = $2, updation_time = $3
         WHERE collection_id = $4 AND file_id IN ({})",
        ids_list(5, ids.len())
    );
    bind_ids(
        sqlx::query(&sql).bind(actor).bind(act).bind(now).bind(cid),
        ids,
    )
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    bump_collections(tx.conn(), &[cid], now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)
}

/// `CollectionController.RemoveFilesV3`.
pub async fn remove_files_v3(
    state: &AppState,
    actor: i64,
    app: App,
    cid: i64,
    ids: &[i64],
) -> ApiResult<()> {
    let a = access(state, cid, actor, app, false, false).await?;
    let owner = a.row.owner_id;
    let ids = filter_active(state, cid, ids).await?;
    if ids.is_empty() {
        return Ok(());
    }
    let map = files::owner_to_file_ids(state, &ids).await?;
    // isRemoveAllowed
    if map.contains_key(&owner) {
        if owner == actor {
            return Err(errors::bad_msg(
                "can not remove files owned collection owner, admins can perform remove suggestion",
            ));
        } else if a.role != role::ADMIN {
            return Err(errors::bad_msg("can not remove files owned by album owner"));
        }
    }
    if owner != actor && a.role != role::ADMIN && (map.len() > 1 || !map.contains_key(&actor)) {
        return Err(errors::denied());
    }
    let owner_files = map.get(&owner).cloned().unwrap_or_default();
    let owner_set: HashSet<i64> = owner_files.iter().copied().collect();
    let others: Vec<i64> = ids
        .iter()
        .copied()
        .filter(|i| !owner_set.contains(i))
        .collect();
    if !owner_files.is_empty() {
        suggest_action(state, cid, actor, &owner_files, action::REMOVE).await?;
        actions::create_bulk(state, owner, actor, cid, &owner_files, action::REMOVE).await?;
    }
    if !others.is_empty() {
        remove_memberships(state, cid, owner, &others).await?;
    }
    Ok(())
}

/// `CollectionController.SuggestDeleteInSharedCollection`.
pub async fn suggest_delete(
    state: &AppState,
    actor: i64,
    app: App,
    cid: i64,
    ids: &[i64],
) -> ApiResult<()> {
    let a = access(state, cid, actor, app, false, false).await?;
    if a.role != role::OWNER && a.role != role::ADMIN {
        return Err(errors::denied());
    }
    let active = filter_active(state, cid, ids).await?;
    if active.is_empty() {
        return Ok(());
    }
    let map = files::owner_to_file_ids(state, &active).await?;
    if map.contains_key(&actor) {
        return Err(errors::denied());
    }
    remove_files_v3(state, actor, app, cid, &active).await?;
    for (uid, fids) in map {
        actions::create_bulk(state, uid, actor, cid, &fids, action::DELETE_SUGGESTED).await?;
    }
    Ok(())
}

async fn diff_rows(state: &AppState, cid: i64, since: i64, limit: i64) -> ApiResult<Vec<File>> {
    let rows = sqlx::query(&format!(
        "SELECT {FILE_COLUMNS} FROM files f JOIN collection_files cf ON cf.file_id = f.file_id
         WHERE cf.collection_id = $1 AND cf.updation_time > $2
         ORDER BY cf.updation_time, cf.file_id LIMIT $3"
    ))
    .bind(cid)
    .bind(since)
    .bind(limit)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    rows.iter()
        .map(row_to_file)
        .collect::<Result<_, _>>()
        .map_err(db_err)
}

async fn files_with_version(state: &AppState, cid: i64, version: i64) -> ApiResult<Vec<File>> {
    let rows = sqlx::query(&format!(
        "SELECT {FILE_COLUMNS} FROM files f JOIN collection_files cf ON cf.file_id = f.file_id
         WHERE cf.collection_id = $1 AND cf.updation_time = $2 ORDER BY cf.file_id"
    ))
    .bind(cid)
    .bind(version)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    rows.iter()
        .map(row_to_file)
        .collect::<Result<_, _>>()
        .map_err(db_err)
}

/// museum's diff paging rule, shared by the collection and trash diffs: fetch `limit + 1`
/// rows; when there are more than `limit`, drop the trailing rows that share the version of
/// row `limit` (so a version is never split across pages). If that leaves nothing, every row
/// of that version is returned instead (`all_of_version`). Returns `(rows, hasMore)`.
pub async fn page_by_version<T, F, Fut>(
    mut rows: Vec<T>,
    limit: usize,
    version: impl Fn(&T) -> i64,
    all_of_version: F,
) -> ApiResult<(Vec<T>, bool)>
where
    F: FnOnce(i64) -> Fut,
    Fut: std::future::Future<Output = ApiResult<Vec<T>>>,
{
    if rows.len() <= limit {
        return Ok((rows, false));
    }
    let last = version(&rows[limit]);
    while rows.last().is_some_and(|r| version(r) == last) {
        rows.pop();
    }
    if !rows.is_empty() {
        return Ok((rows, true));
    }
    Ok((all_of_version(last).await?, true))
}

/// `scrubDeletedFiles`: a deleted entry carries only ids, owner, times and `"-"` metadata.
pub fn scrub_deleted(f: File) -> File {
    if !f.is_deleted {
        return f;
    }
    File {
        id: f.id,
        owner_id: f.owner_id,
        collection_id: f.collection_id,
        collection_owner_id: f.collection_owner_id,
        metadata: super::model::FileAttributes {
            encrypted_data: "-".into(),
            ..Default::default()
        },
        is_deleted: true,
        updation_time: f.updation_time,
        ..Default::default()
    }
}

/// One page of a collection's raw diff (museum's `getDiff`, before any per-viewer
/// post-processing): entries changed after `since`, oldest first, never splitting a version.
/// No access check; public links and cast use it after authenticating the collection.
pub async fn diff_page(state: &AppState, cid: i64, since: i64) -> ApiResult<(Vec<File>, bool)> {
    let rows = diff_rows(state, cid, since, DIFF_LIMIT as i64 + 1).await?;
    page_by_version(
        rows,
        DIFF_LIMIT,
        |f| f.updation_time,
        |v| files_with_version(state, cid, v),
    )
    .await
}

/// museum's `GetCollectionFileState == CollectionFileActive`: the file is in the collection,
/// not deleted, and not pending removal (`action = 'REMOVE'`).
pub async fn is_active_member(state: &AppState, cid: i64, file_id: i64) -> ApiResult<bool> {
    let r = sqlx::query(
        "SELECT is_deleted, action FROM collection_files WHERE collection_id = $1 AND file_id = $2",
    )
    .bind(cid)
    .bind(file_id)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?;
    let Some(r) = r else { return Ok(false) };
    Ok(!get_bool(&r, "is_deleted").map_err(db_err)?
        && get_opt_str(&r, "action").map_err(db_err)?.as_deref() != Some(action::REMOVE))
}

/// `CollectionController.GetDiffV2`: files of a collection changed after `since`.
pub async fn diff(
    state: &AppState,
    actor: i64,
    app: App,
    cid: i64,
    since: i64,
) -> ApiResult<(Vec<File>, bool)> {
    access(state, cid, actor, app, false, false).await?;
    let (rows, has_more) = diff_page(state, cid, since).await?;
    let out = rows
        .into_iter()
        .map(|mut f| {
            if !f.is_deleted && files::hidden_by_action(&f) && f.owner_id != actor {
                f.is_deleted = true;
            }
            if f.owner_id != actor {
                f.magic_metadata = None;
                f.action = None;
                f.action_user = None;
            }
            if f.metadata.encrypted_data == "-" && !f.is_deleted {
                f.is_deleted = true;
            }
            scrub_deleted(f)
        })
        .collect();
    Ok((out, has_more))
}

/// `CollectionController.GetFile` (`GET /collections/file`).
pub async fn get_file(state: &AppState, actor: i64, cid: i64, file_id: i64) -> ApiResult<File> {
    let rows = sqlx::query(&format!(
        "SELECT {FILE_COLUMNS} FROM files f JOIN collection_files cf ON cf.file_id = f.file_id
         WHERE cf.collection_id = $1 AND cf.file_id = $2"
    ))
    .bind(cid)
    .bind(file_id)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let mut f =
        row_to_file(rows.first().ok_or_else(errors::file_not_found_in_album)?).map_err(db_err)?;
    if f.owner_id != actor {
        if !shared_with_user(state, actor).await?.contains(&cid) {
            return Err(errors::denied());
        }
        if files::hidden_by_action(&f) {
            return Err(errors::file_not_found_in_album());
        }
        f.magic_metadata = None;
        f.action = None;
        f.action_user = None;
    }
    if f.is_deleted {
        return Err(errors::file_not_found_in_album());
    }
    Ok(f)
}

/// `CollectionController.TrashV3`: schedule deletion of a collection. `keep_files` requires
/// it to be empty; otherwise the background task trashes the owner's files and removes others'.
pub async fn trash_v3(
    state: &AppState,
    actor: i64,
    app: App,
    cid: i64,
    keep_files: bool,
) -> ApiResult<()> {
    let a = access(state, cid, actor, app, true, true).await?;
    if !a.row.allow_delete() {
        return Err(errors::bad());
    }
    if a.row.is_deleted {
        return Ok(());
    }
    if keep_files {
        let r = sqlx::query(
            "SELECT COUNT(*) AS n FROM collection_files WHERE collection_id = $1 AND is_deleted = FALSE",
        )
        .bind(cid)
        .fetch_one(&state.db.pool)
        .await
        .map_err(db_err)?;
        if get_i64(&r, "n").map_err(db_err)? != 0 {
            return Err(errors::collection_not_empty());
        }
    }
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    links::disable_collection_link(tx.conn(), cid, now)
        .await
        .map_err(db_err)?;
    sqlx::query("UPDATE collection_shares SET is_deleted = TRUE, updation_time = $1 WHERE collection_id = $2")
        .bind(now)
        .bind(cid)
        .execute(tx.conn())
        .await
        .map_err(db_err)?;
    sqlx::query(
        "UPDATE collections SET is_deleted = TRUE, updation_time = $1 WHERE collection_id = $2",
    )
    .bind(now)
    .bind(cid)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    objects::enqueue(tx.conn(), objects::TRASH_COLLECTION_QUEUE, &cid.to_string())
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    if let Err(e) = crate::public::cast::revoke_for_collection(&state.db.pool, cid).await {
        tracing::warn!(collection_id = cid, error = %e, "cast revoke after collection delete failed");
    }
    Ok(())
}

/// Background half of a collection delete (`CollectionRepository.TrashV3`): trash the
/// owner's files that are still in it and remove files added by others.
pub async fn process_trashed_collection(state: &AppState, cid: i64) -> ApiResult<()> {
    let row = get_row(state, cid).await?;
    let owner = row.owner_id;
    let rows = sqlx::query(
        "SELECT cf.file_id FROM collection_files cf JOIN files f ON f.file_id = cf.file_id
         WHERE cf.collection_id = $1 AND cf.is_deleted = FALSE AND f.owner_id = $2 ORDER BY cf.file_id",
    )
    .bind(cid)
    .bind(owner)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let ids: Vec<i64> = rows
        .iter()
        .map(|r| get_i64(r, "file_id"))
        .collect::<Result<_, _>>()
        .map_err(db_err)?;
    for chunk in ids.chunks(1000) {
        let items: Vec<files::TrashItem> = chunk
            .iter()
            .map(|f| files::TrashItem {
                file_id: *f,
                collection_id: cid,
            })
            .collect();
        let mut tx = begin(&state.db).await.map_err(db_err)?;
        let now = now_micros();
        files::bump_file_memberships_and_delete(tx.conn(), chunk, now)
            .await
            .map_err(db_err)?;
        super::trash::insert_items(tx.conn(), owner, &items, now)
            .await
            .map_err(db_err)?;
        links::disable_file_links_for_files(tx.conn(), chunk, now)
            .await
            .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
    }
    let others = sqlx::query(
        "SELECT file_id FROM collection_files WHERE collection_id = $1 AND is_deleted = FALSE
           AND f_owner_id IS NOT NULL AND c_owner_id IS NOT NULL AND f_owner_id <> c_owner_id",
    )
    .bind(cid)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let others: Vec<i64> = others
        .iter()
        .map(|r| get_i64(r, "file_id"))
        .collect::<Result<_, _>>()
        .map_err(db_err)?;
    for chunk in others.chunks(1000) {
        remove_memberships(state, cid, owner, chunk).await?;
    }
    Ok(())
}

/// `CollectionController.Rename` (owner only).
pub async fn rename(
    state: &AppState,
    actor: i64,
    cid: i64,
    name: &str,
    nonce: &str,
) -> ApiResult<()> {
    let c = get_row(state, cid).await?;
    if c.owner_id != actor {
        return Err(errors::denied());
    }
    sqlx::query(
        "UPDATE collections SET encrypted_name = $1, name_decryption_nonce = $2, updation_time = $3
         WHERE collection_id = $4",
    )
    .bind(name)
    .bind(nonce)
    .bind(now_micros())
    .bind(cid)
    .execute(&state.db.pool)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// Collection private/public magic metadata (owner only; stored version = request + 1).
pub async fn update_magic_metadata(
    state: &AppState,
    actor: i64,
    cid: i64,
    mut m: MagicMetadata,
    public: bool,
) -> ApiResult<()> {
    let c = get_row(state, cid).await?;
    if c.owner_id != actor {
        return Err(errors::denied());
    }
    m.version += 1;
    let col = if public {
        "pub_magic_metadata"
    } else {
        "magic_metadata"
    };
    sqlx::query(&format!(
        "UPDATE collections SET {col} = $1, updation_time = $2 WHERE collection_id = $3"
    ))
    .bind(m.to_db())
    .bind(now_micros())
    .bind(cid)
    .execute(&state.db.pool)
    .await
    .map_err(db_err)?;
    Ok(())
}

/// `UpdateShareeMagicMetadata`: the sharee's own metadata on the share (stored as sent).
pub async fn update_sharee_magic_metadata(
    state: &AppState,
    actor: i64,
    app: App,
    cid: i64,
    m: &MagicMetadata,
) -> ApiResult<()> {
    let a = access(state, cid, actor, app, false, false).await?;
    if a.row.owner_id == actor {
        return Err(errors::bad_msg(
            "owner can not update sharee magic metadata",
        ));
    }
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    let r = sqlx::query(
        "UPDATE collection_shares SET magic_metadata = $1, updation_time = $2
         WHERE collection_id = $3 AND from_user_id = $4 AND to_user_id = $5 AND is_deleted = FALSE",
    )
    .bind(m.to_db())
    .bind(now)
    .bind(cid)
    .bind(a.row.owner_id)
    .bind(actor)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    if r.rows_affected() != 1 {
        return Err(errors::internal("invalid number of rows affected"));
    }
    bump_collections(tx.conn(), &[cid], now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)
}
