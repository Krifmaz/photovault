//! File repository: rows → [`File`], ownership and access checks, and the file write paths
//! (create, update, meta file, trash, magic metadata, thumbnail) with museum's semantics.

use super::errors;
use super::model::{CollectionFileItem, File, FileAttributes, FileInfo, MagicMetadata, action};
use super::objects::{self, MAX_FILE_SIZE};
use super::sql::{
    WriteTx, begin, db_err, for_update, get_bool, get_i64, get_opt_i64, get_opt_str, get_str,
    ids_list,
};
use super::{collections, usage};
use crate::auth::App;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::state::AppState;
use sqlx::AnyConnection;
use sqlx::any::AnyRow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

/// Object type of the file's main object in `object_keys.o_type`.
pub const O_FILE: &str = "file";
/// Object type of the thumbnail.
pub const O_THUMBNAIL: &str = "thumbnail";

/// Columns selected by [`row_to_file`]; join `files f` with `collection_files cf`.
pub const FILE_COLUMNS: &str = "f.file_id, f.owner_id, cf.collection_id, cf.c_owner_id,
    cf.encrypted_key AS cf_encrypted_key, cf.key_decryption_nonce AS cf_key_decryption_nonce,
    f.file_decryption_header, f.thumbnail_decryption_header, f.metadata_decryption_header,
    f.encrypted_metadata, f.magic_metadata, f.pub_magic_metadata, f.info,
    cf.action_user, cf.action, cf.is_deleted AS cf_is_deleted, cf.updation_time AS cf_updation_time,
    cf.created_at AS cf_created_at";

/// museum's `convertRowsToFiles`: a collection-file row with the file's encrypted attributes.
pub fn row_to_file(r: &AnyRow) -> Result<File, sqlx::Error> {
    Ok(File {
        id: get_i64(r, "file_id")?,
        owner_id: get_i64(r, "owner_id")?,
        collection_id: get_i64(r, "collection_id")?,
        collection_owner_id: get_opt_i64(r, "c_owner_id")?,
        collection_added_at: get_opt_i64(r, "cf_created_at")?,
        encrypted_key: get_str(r, "cf_encrypted_key")?,
        key_decryption_nonce: get_str(r, "cf_key_decryption_nonce")?,
        file: FileAttributes {
            decryption_header: get_str(r, "file_decryption_header")?,
            ..Default::default()
        },
        thumbnail: FileAttributes {
            decryption_header: get_str(r, "thumbnail_decryption_header")?,
            ..Default::default()
        },
        metadata: FileAttributes {
            decryption_header: get_str(r, "metadata_decryption_header")?,
            encrypted_data: get_str(r, "encrypted_metadata")?,
            ..Default::default()
        },
        is_deleted: get_bool(r, "cf_is_deleted")?,
        updation_time: get_i64(r, "cf_updation_time")?,
        magic_metadata: MagicMetadata::parse(get_opt_str(r, "magic_metadata")?.as_deref()),
        pub_magic_metadata: MagicMetadata::parse(get_opt_str(r, "pub_magic_metadata")?.as_deref()),
        info: get_opt_str(r, "info")?.and_then(|s| serde_json::from_str(&s).ok()),
        action: get_opt_str(r, "action")?,
        action_user: get_opt_i64(r, "action_user")?,
    })
}

/// Bind a list of ids onto a query.
pub fn bind_ids<'q>(
    mut q: sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>>,
    ids: &[i64],
) -> sqlx::query::Query<'q, sqlx::Any, sqlx::any::AnyArguments<'q>> {
    for id in ids {
        q = q.bind(*id);
    }
    q
}

/// `ContainsDuplicateInInt64Array`.
pub fn has_duplicates(ids: &[i64]) -> bool {
    let mut seen = HashSet::new();
    ids.iter().any(|i| !seen.insert(*i))
}

/// `GetOwnerAndApp`: `None` app when unset. Missing file → 404 `{}`.
pub async fn owner_and_app(state: &AppState, file_id: i64) -> ApiResult<(i64, Option<String>)> {
    let r = sqlx::query("SELECT owner_id, app FROM files WHERE file_id = $1")
        .bind(file_id)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(db_err)?
        .ok_or_else(errors::not_found)?;
    Ok((
        get_i64(&r, "owner_id").map_err(db_err)?,
        get_opt_str(&r, "app")
            .map_err(db_err)?
            .filter(|s| !s.is_empty()),
    ))
}

/// `GetOwnerToFileIDsMap` (missing ids are simply absent).
pub async fn owner_to_file_ids(
    state: &AppState,
    ids: &[i64],
) -> ApiResult<BTreeMap<i64, Vec<i64>>> {
    let mut out: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let sql = format!(
        "SELECT owner_id, file_id FROM files WHERE file_id IN ({})",
        ids_list(1, ids.len())
    );
    let rows = bind_ids(sqlx::query(&sql), ids)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?;
    for r in rows {
        out.entry(get_i64(&r, "owner_id").map_err(db_err)?)
            .or_default()
            .push(get_i64(&r, "file_id").map_err(db_err)?);
    }
    Ok(out)
}

/// `FileRepository.VerifyFileOwner`: none found → 400; several owners or not the actor → 403;
/// some missing → 400 (all `{}`).
pub async fn verify_file_owner(state: &AppState, owner: i64, ids: &[i64]) -> ApiResult<()> {
    let map = owner_to_file_ids(state, ids).await?;
    if map.is_empty() {
        return Err(errors::bad());
    }
    if map.len() > 1 {
        return Err(errors::denied());
    }
    match map.get(&owner) {
        Some(v) if v.len() == ids.len() => Ok(()),
        Some(_) => Err(errors::bad()),
        None => Err(errors::denied()),
    }
}

/// `AccessController.VerifyFileOwnership`: duplicates → 400, then [`verify_file_owner`].
pub async fn verify_file_ownership(state: &AppState, owner: i64, ids: &[i64]) -> ApiResult<()> {
    if has_duplicates(ids) {
        return Err(errors::bad());
    }
    verify_file_owner(state, owner, ids).await
}

/// `AccessController.CanAccessFile`: the actor owns each file, or it is live (not removed)
/// in a collection shared with or by the actor. Duplicates → 400; otherwise 403 `{}`.
pub async fn can_access_files(state: &AppState, actor: i64, ids: &[i64]) -> ApiResult<()> {
    if has_duplicates(ids) {
        return Err(errors::bad());
    }
    let map = owner_to_file_ids(state, ids).await?;
    let others: Vec<i64> = map
        .iter()
        .filter(|(o, _)| **o != actor)
        .flat_map(|(_, v)| v.iter().copied())
        .collect();
    if others.is_empty() {
        return Ok(());
    }
    let shared = collections::shared_with_or_by_user(state, actor).await?;
    if shared.is_empty() {
        return Err(errors::denied());
    }
    let sql = format!(
        "SELECT DISTINCT file_id FROM collection_files WHERE file_id IN ({}) AND is_deleted = FALSE
         AND (action IS NULL OR action <> 'REMOVE') AND collection_id IN ({})",
        ids_list(1, others.len()),
        ids_list(others.len() + 1, shared.len())
    );
    let rows = bind_ids(bind_ids(sqlx::query(&sql), &others), &shared)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?;
    if rows.len() != others.len() {
        return Err(errors::denied());
    }
    Ok(())
}

/// A file's stored object of type `o_type`: `(object_key, size)`.
pub async fn get_object(
    state: &AppState,
    file_id: i64,
    o_type: &str,
) -> ApiResult<Option<(String, i64)>> {
    let r = sqlx::query(
        "SELECT object_key, size FROM object_keys WHERE file_id = $1 AND o_type = $2 AND is_deleted = FALSE",
    )
    .bind(file_id)
    .bind(o_type)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?;
    match r {
        None => Ok(None),
        Some(r) => Ok(Some((
            get_str(&r, "object_key").map_err(db_err)?,
            get_i64(&r, "size").map_err(db_err)?,
        ))),
    }
}

/// `ObjectRepository.GetAccessibleObject`: the object key when `actor` may read the file
/// (owner, or live in a collection shared with/by the actor). No access → 403 `{}`;
/// missing file or object → 404 `{}`.
pub async fn accessible_object(
    state: &AppState,
    file_id: i64,
    actor: i64,
    o_type: &str,
) -> ApiResult<String> {
    let r = sqlx::query(
        "SELECT f.owner_id,
            (SELECT COUNT(*) FROM collection_files cf
               JOIN collection_shares cs ON cs.collection_id = cf.collection_id
              WHERE cf.file_id = f.file_id AND cf.is_deleted = FALSE
                AND (cf.action IS NULL OR cf.action <> 'REMOVE')
                AND cs.is_deleted = FALSE AND (cs.to_user_id = $2 OR cs.from_user_id = $2)) AS shared,
            ok.object_key
         FROM files f
         LEFT JOIN object_keys ok ON ok.file_id = f.file_id AND ok.o_type = $3 AND ok.is_deleted = FALSE
         WHERE f.file_id = $1",
    )
    .bind(file_id)
    .bind(actor)
    .bind(o_type)
    .fetch_optional(&state.db.pool)
    .await
    .map_err(db_err)?
    .ok_or_else(errors::not_found)?;
    let owner = get_i64(&r, "owner_id").map_err(db_err)?;
    let shared = get_i64(&r, "shared").map_err(db_err)?;
    if owner != actor && shared == 0 {
        return Err(errors::denied());
    }
    get_opt_str(&r, "object_key")
        .map_err(db_err)?
        .ok_or_else(errors::not_found)
}

/// `sizeOf`: HEAD with museum's three attempts (500 ms, 1 s back-off).
pub async fn size_of(state: &AppState, key: &str) -> Result<i64, String> {
    let mut last = String::new();
    for i in 0..3u64 {
        match state.store.head(key).await {
            Ok(Some(n)) => return Ok(n as i64),
            Ok(None) => last = format!("object {key} not found"),
            Err(e) => last = e.to_string(),
        }
        if i < 2 {
            tokio::time::sleep(Duration::from_millis(500 * (i + 1))).await;
        }
    }
    Err(last)
}

/// Bump `collection_files.updation_time` for a file's rows (optionally only live ones) and
/// the collections that hold them.
pub async fn bump_file_memberships(
    conn: &mut AnyConnection,
    file_ids: &[i64],
    time: i64,
    only_active: bool,
) -> Result<(), sqlx::Error> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let active = if only_active {
        " AND is_deleted = FALSE"
    } else {
        ""
    };
    let ids = ids_list(2, file_ids.len());
    let sel = format!(
        "SELECT DISTINCT collection_id FROM collection_files WHERE collection_id > $1 AND file_id IN ({ids}){active}"
    );
    let cids: Vec<i64> = bind_ids(sqlx::query(&sel).bind(0i64), file_ids)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|r| get_i64(r, "collection_id"))
        .collect::<Result<_, _>>()?;
    let upd =
        format!("UPDATE collection_files SET updation_time = $1 WHERE file_id IN ({ids}){active}");
    bind_ids(sqlx::query(&upd).bind(time), file_ids)
        .execute(&mut *conn)
        .await?;
    collections::bump_collections(conn, &cids, time).await
}

/// Lock the owner's file rows for the rest of the transaction (museum's `lockFiles`).
pub async fn lock_files(
    tx: &mut WriteTx,
    state: &AppState,
    owner: i64,
    ids: &[i64],
) -> Result<(), sqlx::Error> {
    let fu = for_update(&state.db);
    if ids.is_empty() || fu.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "SELECT file_id FROM files WHERE owner_id = $1 AND file_id IN ({}) ORDER BY file_id{fu}",
        ids_list(2, ids.len())
    );
    bind_ids(sqlx::query(&sql).bind(owner), ids)
        .fetch_all(tx.conn())
        .await?;
    Ok(())
}

fn validate_create_or_update(user_id: i64, f: &File, is_create: bool) -> ApiResult<()> {
    let prefix = format!("{user_id}/");
    if !f.file.object_key.starts_with(&prefix) || !f.thumbnail.object_key.starts_with(&prefix) {
        return Err(errors::bad());
    }
    if f.file.object_key == f.thumbnail.object_key {
        return Err(errors::bad());
    }
    if is_create && (f.encrypted_key.is_empty() || f.key_decryption_nonce.is_empty()) {
        return Err(errors::bad());
    }
    if f.file.decryption_header.is_empty() || f.thumbnail.decryption_header.is_empty() {
        return Err(errors::bad());
    }
    if f.updation_time == 0 {
        return Err(errors::bad());
    }
    Ok(())
}

fn info_json(i: &FileInfo) -> String {
    serde_json::to_string(i).unwrap_or_else(|_| "{}".into())
}

fn mmd_db(m: &Option<MagicMetadata>) -> Option<String> {
    m.as_ref().map(MagicMetadata::to_db)
}

/// `FileController.Create`: validate, verify both objects, check quota, then insert the file,
/// its membership, its object keys and consume the staged uploads in one transaction.
/// A repeated identical request returns the existing file id (museum's duplicate handling).
pub async fn create(state: &AppState, user_id: i64, app: App, mut file: File) -> ApiResult<File> {
    validate_create_or_update(user_id, &file, true)?;
    let c = collections::get_row(state, file.collection_id).await?;
    if c.app != app.as_str() {
        return Err(errors::internal("ctx app is different from collection app"));
    }
    if c.owner_id != user_id {
        return Err(errors::denied());
    }
    if c.is_deleted {
        return Err(errors::collection_deleted());
    }
    if file.owner_id != user_id {
        return Err(errors::denied());
    }
    let (fs, ts) = tokio::join!(
        size_of(state, &file.file.object_key),
        size_of(state, &file.thumbnail.object_key)
    );
    let file_size = fs.map_err(|e| {
        tracing::warn!(error = %e, "could not find size of file");
        errors::object_size_fetch_failed()
    })?;
    let thumb_size = ts.map_err(|e| {
        tracing::warn!(error = %e, "could not find size of thumbnail");
        errors::object_size_fetch_failed()
    })?;
    if file_size > MAX_FILE_SIZE {
        return Err(errors::too_large());
    }
    if file.file.size != 0 && file.file.size != file_size {
        return Err(errors::bad());
    }
    file.file.size = file_size;
    if file.thumbnail.size != 0 && file.thumbnail.size != thumb_size {
        return Err(errors::bad());
    }
    file.thumbnail.size = thumb_size;
    usage::can_upload(state, user_id, Some(file_size + thumb_size), app).await?;
    let info = FileInfo {
        file_size,
        thumb_size,
    };
    file.info = Some(info);

    match insert_file(state, user_id, app, &mut file, &info).await {
        Ok(()) => Ok(file),
        Err(InsertError::Duplicate(key)) => on_duplicate(state, file, &key, app).await,
        Err(InsertError::Api(e)) => Err(e),
    }
}

enum InsertError {
    Duplicate(String),
    Api(crate::error::ApiError),
}

impl From<crate::error::ApiError> for InsertError {
    fn from(e: crate::error::ApiError) -> Self {
        InsertError::Api(e)
    }
}

async fn insert_file(
    state: &AppState,
    user_id: i64,
    app: App,
    file: &mut File,
    info: &FileInfo,
) -> Result<(), InsertError> {
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    let r = sqlx::query(
        "INSERT INTO files (owner_id, app, encrypted_metadata, file_decryption_header,
            thumbnail_decryption_header, metadata_decryption_header, magic_metadata,
            pub_magic_metadata, info, updation_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING file_id",
    )
    .bind(file.owner_id)
    .bind(app.as_str())
    .bind(&file.metadata.encrypted_data)
    .bind(&file.file.decryption_header)
    .bind(&file.thumbnail.decryption_header)
    .bind(&file.metadata.decryption_header)
    .bind(mmd_db(&file.magic_metadata))
    .bind(mmd_db(&file.pub_magic_metadata))
    .bind(info_json(info))
    .bind(file.updation_time)
    .fetch_one(tx.conn())
    .await
    .map_err(db_err)?;
    file.id = get_i64(&r, "file_id").map_err(db_err)?;
    insert_membership(
        tx.conn(),
        file.collection_id,
        file.id,
        &file.encrypted_key,
        &file.key_decryption_nonce,
        file.updation_time,
        user_id,
        file.owner_id,
    )
    .await
    .map_err(db_err)?;
    collections::bump_collections(tx.conn(), &[file.collection_id], file.updation_time)
        .await
        .map_err(db_err)?;
    for (o_type, key, size) in [
        (O_FILE, file.file.object_key.clone(), file.file.size),
        (
            O_THUMBNAIL,
            file.thumbnail.object_key.clone(),
            file.thumbnail.size,
        ),
    ] {
        let res = sqlx::query(
            "INSERT INTO object_keys (file_id, o_type, object_key, size) VALUES ($1, $2, $3, $4)",
        )
        .bind(file.id)
        .bind(o_type)
        .bind(&key)
        .bind(size)
        .execute(tx.conn())
        .await;
        if let Err(e) = res {
            if objects::is_unique_violation(&e) {
                return Err(InsertError::Duplicate(key));
            }
            return Err(db_err(e).into());
        }
    }
    objects::consume_file_upload(tx.conn(), &file.file.object_key, file.owner_id, app).await?;
    objects::consume_file_upload(tx.conn(), &file.thumbnail.object_key, file.owner_id, app).await?;
    usage::adjust(tx.conn(), file.owner_id, info.file_size + info.thumb_size)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// Insert a `collection_files` row (a new membership).
#[allow(clippy::too_many_arguments)]
pub async fn insert_membership(
    conn: &mut AnyConnection,
    collection_id: i64,
    file_id: i64,
    encrypted_key: &str,
    nonce: &str,
    time: i64,
    c_owner: i64,
    f_owner: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO collection_files (collection_id, file_id, encrypted_key, key_decryption_nonce,
            is_deleted, updation_time, c_owner_id, f_owner_id, created_at)
         VALUES ($1, $2, $3, $4, FALSE, $5, $6, $7, $5)",
    )
    .bind(collection_id)
    .bind(file_id)
    .bind(encrypted_key)
    .bind(nonce)
    .bind(time)
    .bind(c_owner)
    .bind(f_owner)
    .execute(conn)
    .await?;
    Ok(())
}

/// `GetFileAttributesFromObjectKey`: the stored attributes of the file owning `key`.
async fn file_attributes_from_object_key(state: &AppState, key: &str) -> ApiResult<File> {
    let r = sqlx::query("SELECT file_id FROM object_keys WHERE object_key = $1")
        .bind(key)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(db_err)?
        .ok_or_else(errors::not_found)?;
    let id = get_i64(&r, "file_id").map_err(db_err)?;
    let f = sqlx::query(
        "SELECT owner_id, encrypted_metadata, file_decryption_header, thumbnail_decryption_header,
            metadata_decryption_header FROM files WHERE file_id = $1",
    )
    .bind(id)
    .fetch_one(&state.db.pool)
    .await
    .map_err(db_err)?;
    let mut out = File {
        id,
        owner_id: get_i64(&f, "owner_id").map_err(db_err)?,
        ..Default::default()
    };
    out.file.decryption_header = get_str(&f, "file_decryption_header").map_err(db_err)?;
    out.thumbnail.decryption_header = get_str(&f, "thumbnail_decryption_header").map_err(db_err)?;
    out.metadata.decryption_header = get_str(&f, "metadata_decryption_header").map_err(db_err)?;
    out.metadata.encrypted_data = get_str(&f, "encrypted_metadata").map_err(db_err)?;
    let rows = sqlx::query("SELECT o_type, object_key, size FROM object_keys WHERE file_id = $1")
        .bind(id)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?;
    for r in rows {
        let attrs = match get_str(&r, "o_type").map_err(db_err)?.as_str() {
            O_FILE => &mut out.file,
            O_THUMBNAIL => &mut out.thumbnail,
            _ => continue,
        };
        attrs.object_key = get_str(&r, "object_key").map_err(db_err)?;
        attrs.size = get_i64(&r, "size").map_err(db_err)?;
    }
    Ok(out)
}

async fn on_duplicate(state: &AppState, mut file: File, key: &str, app: App) -> ApiResult<File> {
    let existing = file_attributes_from_object_key(state, key).await?;
    let same = file.thumbnail.object_key == existing.thumbnail.object_key
        && file.thumbnail.size == existing.thumbnail.size
        && file.thumbnail.decryption_header == existing.thumbnail.decryption_header
        && file.file.object_key == existing.file.object_key
        && file.file.size == existing.file.size
        && file.file.decryption_header == existing.file.decryption_header
        && file.metadata.encrypted_data == existing.metadata.encrypted_data
        && file.metadata.decryption_header == existing.metadata.decryption_header
        && file.owner_id == existing.owner_id;
    if same {
        let (_, existing_app) = owner_and_app(state, existing.id).await?;
        if existing_app.is_some_and(|a| a != app.as_str()) {
            return Err(errors::denied());
        }
        file.id = existing.id;
        return Ok(file);
    }
    Err(errors::upload_conflict())
}

/// `FileController.Update` (`PUT /files/update`, or `POST /files` with an id).
/// Returns `(id, updationTime)`.
pub async fn update(
    state: &AppState,
    user_id: i64,
    app: App,
    mut file: File,
) -> ApiResult<(i64, i64)> {
    validate_create_or_update(user_id, &file, false)?;
    let (owner, file_app) = owner_and_app(state, file.id).await?;
    if owner != user_id || file_app.as_deref().is_some_and(|a| a != app.as_str()) {
        return Err(errors::denied());
    }
    file.owner_id = owner;
    let (old_file_key, old_file_size) = get_object(state, file.id, O_FILE)
        .await?
        .ok_or_else(errors::not_found)?;
    let (old_thumb_key, old_thumb_size) = get_object(state, file.id, O_THUMBNAIL)
        .await?
        .ok_or_else(errors::not_found)?;
    let file_size = size_of(state, &file.file.object_key)
        .await
        .map_err(errors::internal)?;
    if file_size > MAX_FILE_SIZE {
        return Err(errors::too_large());
    }
    if file.file.size != 0 && file.file.size != file_size {
        return Err(errors::bad());
    }
    let thumb_size = size_of(state, &file.thumbnail.object_key)
        .await
        .map_err(errors::internal)?;
    if file.thumbnail.size != 0 && file.thumbnail.size != thumb_size {
        return Err(errors::bad());
    }
    let diff = (file_size + thumb_size) - (old_file_size + old_thumb_size);
    usage::can_upload(state, user_id, Some(diff), app).await?;
    let mut old_objects = Vec::new();
    let mut staged = Vec::new();
    if old_thumb_key != file.thumbnail.object_key {
        old_objects.push(old_thumb_key);
        staged.push(file.thumbnail.object_key.clone());
    }
    if old_file_key != file.file.object_key {
        old_objects.push(old_file_key);
        staged.push(file.file.object_key.clone());
    }
    let mut info = file.info.unwrap_or_default();
    info.file_size = file_size;
    info.thumb_size = thumb_size;

    let mut tx = begin(&state.db).await.map_err(db_err)?;
    sqlx::query(
        "UPDATE files SET encrypted_metadata = $1, file_decryption_header = $2,
            thumbnail_decryption_header = $3, metadata_decryption_header = $4,
            updation_time = $5, info = $6 WHERE file_id = $7",
    )
    .bind(&file.metadata.encrypted_data)
    .bind(&file.file.decryption_header)
    .bind(&file.thumbnail.decryption_header)
    .bind(&file.metadata.decryption_header)
    .bind(file.updation_time)
    .bind(info_json(&info))
    .bind(file.id)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    bump_file_memberships(tx.conn(), &[file.id], file.updation_time, false)
        .await
        .map_err(db_err)?;
    for (o_type, key, size) in [
        (O_FILE, &file.file.object_key, file_size),
        (O_THUMBNAIL, &file.thumbnail.object_key, thumb_size),
    ] {
        sqlx::query(
            "UPDATE object_keys SET object_key = $1, size = $2 WHERE file_id = $3 AND o_type = $4",
        )
        .bind(key)
        .bind(size)
        .bind(file.id)
        .bind(o_type)
        .execute(tx.conn())
        .await
        .map_err(db_err)?;
    }
    usage::adjust(tx.conn(), file.owner_id, diff)
        .await
        .map_err(db_err)?;
    for k in &staged {
        objects::consume_file_upload(tx.conn(), k, file.owner_id, app).await?;
    }
    objects::queue_object_deletion(tx.conn(), &old_objects)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok((file.id, file.updation_time))
}

/// `POST /files/meta` request (`ente.MetaFile`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct MetaFile {
    /// Must be 0.
    pub id: i64,
    /// Target collection.
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
    /// File key encrypted with the collection key.
    #[serde(rename = "encryptedKey")]
    pub encrypted_key: String,
    /// Nonce.
    #[serde(rename = "keyDecryptionNonce")]
    pub key_decryption_nonce: String,
    /// Encrypted metadata.
    pub metadata: FileAttributes,
    /// Private magic metadata.
    #[serde(rename = "magicMetadata")]
    pub magic_metadata: Option<MagicMetadata>,
    /// Public magic metadata.
    #[serde(rename = "pubMagicMetadata")]
    pub pub_magic_metadata: Option<MagicMetadata>,
}

/// `FileController.CreateMetaFile`: a Locker file with metadata only.
pub async fn create_meta_file(
    state: &AppState,
    user_id: i64,
    app: App,
    m: MetaFile,
) -> ApiResult<File> {
    let c = collections::get_row(state, m.collection_id).await?;
    if c.owner_id != user_id {
        return Err(errors::denied());
    }
    if c.is_deleted {
        return Err(errors::collection_deleted());
    }
    if app != App::Locker || c.app != App::Locker.as_str() {
        return Err(errors::internal("meta file creation restricted to locker"));
    }
    usage::can_upload(state, user_id, None, app).await?;
    let now = now_micros();
    let info = FileInfo::default();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    let r = sqlx::query(
        "INSERT INTO files (owner_id, app, encrypted_metadata, file_decryption_header,
            thumbnail_decryption_header, metadata_decryption_header, magic_metadata,
            pub_magic_metadata, info, updation_time)
         VALUES ($1, $2, $3, '', '', $4, $5, $6, $7, $8) RETURNING file_id",
    )
    .bind(user_id)
    .bind(app.as_str())
    .bind(&m.metadata.encrypted_data)
    .bind(&m.metadata.decryption_header)
    .bind(mmd_db(&m.magic_metadata))
    .bind(mmd_db(&m.pub_magic_metadata))
    .bind(info_json(&info))
    .bind(now)
    .fetch_one(tx.conn())
    .await
    .map_err(db_err)?;
    let id = get_i64(&r, "file_id").map_err(db_err)?;
    insert_membership(
        tx.conn(),
        m.collection_id,
        id,
        &m.encrypted_key,
        &m.key_decryption_nonce,
        now,
        user_id,
        user_id,
    )
    .await
    .map_err(db_err)?;
    collections::bump_collections(tx.conn(), &[m.collection_id], now)
        .await
        .map_err(db_err)?;
    usage::adjust(tx.conn(), user_id, 0).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(File {
        id,
        owner_id: user_id,
        collection_id: m.collection_id,
        encrypted_key: m.encrypted_key,
        key_decryption_nonce: m.key_decryption_nonce,
        metadata: m.metadata,
        updation_time: now,
        magic_metadata: m.magic_metadata,
        pub_magic_metadata: m.pub_magic_metadata,
        info: Some(info),
        ..Default::default()
    })
}

/// One `/files/trash` item.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(default)]
pub struct TrashItem {
    /// File id.
    #[serde(rename = "fileID")]
    pub file_id: i64,
    /// Collection the file is trashed from (restored there by default).
    #[serde(rename = "collectionID")]
    pub collection_id: i64,
}

/// `FileController.Trash` + `TrashRepository.TrashFiles`.
pub async fn trash(state: &AppState, user_id: i64, items: &[TrashItem]) -> ApiResult<()> {
    let file_ids: Vec<i64> = items.iter().map(|i| i.file_id).collect();
    if has_duplicates(&file_ids) {
        return Err(errors::bad());
    }
    verify_file_ownership(state, user_id, &file_ids).await?;
    let cids: HashSet<i64> = items.iter().map(|i| i.collection_id).collect();
    for cid in cids {
        let c = collections::get_row(state, cid).await?;
        if c.owner_id != user_id {
            return Err(errors::denied());
        }
    }
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    lock_files(&mut tx, state, user_id, &file_ids)
        .await
        .map_err(db_err)?;
    let now = now_micros();
    bump_file_memberships_and_delete(tx.conn(), &file_ids, now)
        .await
        .map_err(db_err)?;
    super::trash::insert_items(tx.conn(), user_id, items, now)
        .await
        .map_err(db_err)?;
    super::links::disable_file_links_for_files(tx.conn(), &file_ids, now)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// Mark all memberships of `file_ids` deleted and bump the affected collections.
pub async fn bump_file_memberships_and_delete(
    conn: &mut AnyConnection,
    file_ids: &[i64],
    now: i64,
) -> Result<(), sqlx::Error> {
    if file_ids.is_empty() {
        return Ok(());
    }
    let ids = ids_list(1, file_ids.len());
    let sel = format!(
        "SELECT DISTINCT collection_id FROM collection_files WHERE file_id IN ({ids}) AND is_deleted = FALSE"
    );
    let cids: Vec<i64> = bind_ids(sqlx::query(&sel), file_ids)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(|r| get_i64(r, "collection_id"))
        .collect::<Result<_, _>>()?;
    let upd = format!(
        "UPDATE collection_files SET is_deleted = TRUE, updation_time = $1 WHERE file_id IN ({})",
        ids_list(2, file_ids.len())
    );
    bind_ids(sqlx::query(&upd).bind(now), file_ids)
        .execute(&mut *conn)
        .await?;
    collections::bump_collections(conn, &cids, now).await
}

/// One magic metadata update (`ente.UpdateMagicMetadata`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct MagicMetadataUpdate {
    /// File id.
    pub id: i64,
    /// New metadata; `version` must equal the stored version.
    #[serde(rename = "magicMetadata")]
    pub magic_metadata: MagicMetadata,
}

/// `FileController.UpdateMagicMetadata`: version-checked write of private or public
/// magic metadata (stored version = request version + 1).
pub async fn update_magic_metadata(
    state: &AppState,
    user_id: i64,
    list: &[MagicMetadataUpdate],
    public: bool,
    skip_version: bool,
) -> ApiResult<()> {
    let col = if public {
        "pub_magic_metadata"
    } else {
        "magic_metadata"
    };
    for u in list {
        let r = sqlx::query(&format!(
            "SELECT owner_id, {col} FROM files WHERE file_id = $1"
        ))
        .bind(u.id)
        .fetch_optional(&state.db.pool)
        .await
        .map_err(db_err)?
        .ok_or_else(errors::not_found)?;
        if get_i64(&r, "owner_id").map_err(db_err)? != user_id {
            return Err(errors::denied());
        }
        if let Some(existing) =
            MagicMetadata::parse(get_opt_str(&r, col).map_err(db_err)?.as_deref())
            && (existing.version != u.magic_metadata.version
                || existing.count - u.magic_metadata.count > 2)
        {
            return Err(errors::version_mismatch());
        }
    }
    let mut sorted: Vec<&MagicMetadataUpdate> = list.iter().collect();
    sorted.sort_by_key(|u| u.id);
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    let mut ids = Vec::with_capacity(sorted.len());
    for u in sorted {
        let mut m = u.magic_metadata.clone();
        m.version += 1;
        ids.push(u.id);
        sqlx::query(&format!(
            "UPDATE files SET {col} = $1, updation_time = $2 WHERE file_id = $3"
        ))
        .bind(m.to_db())
        .bind(now)
        .bind(u.id)
        .execute(tx.conn())
        .await
        .map_err(db_err)?;
    }
    if !skip_version {
        bump_file_memberships(tx.conn(), &ids, now, true)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// `FileController.UpdateThumbnail` (`PUT /files/thumbnail`).
pub async fn update_thumbnail(
    state: &AppState,
    user_id: i64,
    app: App,
    file_id: i64,
    thumb: &FileAttributes,
) -> ApiResult<()> {
    if !thumb.object_key.starts_with(&format!("{user_id}/")) {
        return Err(errors::bad());
    }
    let (owner, file_app) = owner_and_app(state, file_id).await?;
    if owner != user_id || file_app.as_deref().is_some_and(|a| a != app.as_str()) {
        return Err(errors::denied());
    }
    let (old_key, old_size) = get_object(state, file_id, O_THUMBNAIL)
        .await?
        .ok_or_else(errors::not_found)?;
    let new_size = size_of(state, &thumb.object_key)
        .await
        .map_err(errors::internal)?;
    let diff = new_size - old_size;
    if diff > 0 {
        return Err(errors::internal(
            "new thumbnail larger than existing thumbnail",
        ));
    }
    usage::can_upload(state, user_id, Some(diff), app).await?;
    let now = now_micros();
    let mut tx = begin(&state.db).await.map_err(db_err)?;
    sqlx::query(
        "UPDATE files SET thumbnail_decryption_header = $1, updation_time = $2 WHERE file_id = $3",
    )
    .bind(&thumb.decryption_header)
    .bind(now)
    .bind(file_id)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    bump_file_memberships(tx.conn(), &[file_id], now, false)
        .await
        .map_err(db_err)?;
    sqlx::query(
        "UPDATE object_keys SET object_key = $1, size = $2 WHERE file_id = $3 AND o_type = $4",
    )
    .bind(&thumb.object_key)
    .bind(new_size)
    .bind(file_id)
    .bind(O_THUMBNAIL)
    .execute(tx.conn())
    .await
    .map_err(db_err)?;
    usage::adjust(tx.conn(), user_id, diff)
        .await
        .map_err(db_err)?;
    if old_key != thumb.object_key {
        objects::consume_file_upload(tx.conn(), &thumb.object_key, user_id, app).await?;
        objects::queue_object_deletion(tx.conn(), &[old_key])
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

/// `FileRepository.GetSize`: total bytes of the user's live file objects among `ids`.
pub async fn total_size(state: &AppState, user_id: i64, ids: &[i64]) -> ApiResult<i64> {
    if ids.is_empty() {
        return Ok(0);
    }
    let sql = format!(
        "SELECT CAST(COALESCE(SUM(size), 0) AS BIGINT) AS total FROM object_keys
         WHERE o_type = 'file' AND is_deleted = FALSE
           AND file_id IN (SELECT file_id FROM files WHERE owner_id = $1 AND file_id IN ({}))",
        ids_list(2, ids.len())
    );
    let r = bind_ids(sqlx::query(&sql).bind(user_id), ids)
        .fetch_one(&state.db.pool)
        .await
        .map_err(db_err)?;
    get_i64(&r, "total").map_err(db_err)
}

/// `FileController.GetFileInfo`: per-file sizes, from `files.info` or else `object_keys`.
pub async fn files_info(
    state: &AppState,
    user_id: i64,
    ids: &[i64],
) -> ApiResult<Vec<(i64, FileInfo)>> {
    verify_file_owner(state, user_id, ids).await?;
    let mut found: HashMap<i64, Option<FileInfo>> = HashMap::new();
    let sql = format!(
        "SELECT file_id, info FROM files WHERE owner_id = $1 AND file_id IN ({})",
        ids_list(2, ids.len())
    );
    for r in bind_ids(sqlx::query(&sql).bind(user_id), ids)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?
    {
        let info = get_opt_str(&r, "info")
            .map_err(db_err)?
            .and_then(|s| serde_json::from_str::<FileInfo>(&s).ok());
        found.insert(get_i64(&r, "file_id").map_err(db_err)?, info);
    }
    let missing: Vec<i64> = ids
        .iter()
        .copied()
        .filter(|i| !matches!(found.get(i), Some(Some(_))))
        .collect();
    if !missing.is_empty() {
        let sql = format!(
            "SELECT file_id, size, o_type FROM object_keys WHERE file_id IN ({})",
            ids_list(1, missing.len())
        );
        let mut from_keys: HashMap<i64, FileInfo> = HashMap::new();
        for r in bind_ids(sqlx::query(&sql), &missing)
            .fetch_all(&state.db.pool)
            .await
            .map_err(db_err)?
        {
            let e = from_keys
                .entry(get_i64(&r, "file_id").map_err(db_err)?)
                .or_default();
            let size = get_i64(&r, "size").map_err(db_err)?;
            match get_str(&r, "o_type").map_err(db_err)?.as_str() {
                O_FILE => e.file_size = size,
                O_THUMBNAIL => e.thumb_size = size,
                _ => {}
            }
        }
        for (id, info) in from_keys {
            sqlx::query("UPDATE files SET info = $1 WHERE file_id = $2 AND info IS NULL")
                .bind(info_json(&info))
                .bind(id)
                .execute(&state.db.pool)
                .await
                .map_err(db_err)?;
            found.insert(id, Some(info));
        }
    }
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        match found.get(id) {
            None => {
                return Err(crate::error::ApiError::new(
                    axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                    "INTERNAL_ERROR",
                    "failed to get fileInfo",
                ));
            }
            Some(Some(i)) => out.push((*id, *i)),
            Some(None) => out.push((
                *id,
                FileInfo {
                    file_size: -1,
                    thumb_size: -1,
                },
            )),
        }
    }
    Ok(out)
}

/// `GetDuplicateFiles`: the user's live file objects grouped by identical size.
pub async fn duplicates(state: &AppState, user_id: i64) -> ApiResult<Vec<(Vec<i64>, i64)>> {
    let rows = sqlx::query(
        "SELECT o.file_id, o.size FROM object_keys o JOIN files f ON f.file_id = o.file_id
         WHERE f.owner_id = $1 AND o.o_type = 'file' AND o.is_deleted = FALSE
         ORDER BY o.size, o.file_id",
    )
    .bind(user_id)
    .fetch_all(&state.db.pool)
    .await
    .map_err(db_err)?;
    let mut groups: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for r in rows {
        groups
            .entry(get_i64(&r, "size").map_err(db_err)?)
            .or_default()
            .push(get_i64(&r, "file_id").map_err(db_err)?);
    }
    Ok(groups
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(s, v)| (v, s))
        .collect())
}

/// `/files/count`: number of files ever created (museum reports `max(file_id)` minus an offset).
pub async fn total_file_count(state: &AppState) -> ApiResult<i64> {
    let r = sqlx::query("SELECT CAST(COALESCE(MAX(file_id), 0) AS BIGINT) AS n FROM files")
        .fetch_one(&state.db.pool)
        .await
        .map_err(db_err)?;
    get_i64(&r, "n").map_err(db_err)
}

/// Copy request item validation (`validateCollectionFileItems`).
pub fn validate_file_items(items: &[CollectionFileItem]) -> ApiResult<()> {
    for f in items {
        if f.id == 0 {
            return Err(errors::bad_msg("file ID is required"));
        }
        collections::validate_owned_key(&f.encrypted_key, &f.key_decryption_nonce)?;
    }
    Ok(())
}

/// Whether a diff row must look deleted to a non-owner viewer.
pub fn hidden_by_action(f: &File) -> bool {
    matches!(
        f.action.as_deref(),
        Some(action::REMOVE) | Some(action::DELETE_SUGGESTED)
    )
}

/// `POST /files/copy` request (`ente.CopyFileSyncRequest`).
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct CopyRequest {
    /// Collection the files are copied from (shared with the actor).
    #[serde(rename = "srcCollectionID")]
    pub src_collection_id: i64,
    /// Actor-owned collection the copies go to.
    #[serde(rename = "dstCollectionID")]
    pub dst_collection_id: i64,
    /// Files with their keys re-encrypted for the destination collection.
    pub files: Option<Vec<CollectionFileItem>>,
}

/// `FileCopyController.CopyFiles`: copy files shared with the actor into the actor's own
/// collection as new files owned by the actor (objects are copied server-side).
/// Returns old → new file ids.
pub async fn copy_files(
    state: &AppState,
    actor: i64,
    app: App,
    client: &str,
    req: &CopyRequest,
) -> ApiResult<BTreeMap<i64, i64>> {
    let items = req.files.clone().unwrap_or_default();
    validate_file_items(&items)?;
    let src = collections::access(state, req.src_collection_id, actor, app, false, false).await?;
    let dst = collections::access(state, req.dst_collection_id, actor, app, false, true).await?;
    if src.row.app != dst.row.app {
        return Err(errors::internal("copy across app not supported"));
    }
    let ids: Vec<i64> = items.iter().map(|i| i.id).collect();
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "SELECT file_id FROM collection_files WHERE collection_id = $1 AND is_deleted = FALSE
           AND file_id IN ({}) AND (action IS NULL OR action <> 'REMOVE')",
        ids_list(2, ids.len())
    );
    let live: HashSet<i64> = bind_ids(sqlx::query(&sql).bind(req.src_collection_id), &ids)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?
        .iter()
        .map(|r| get_i64(r, "file_id"))
        .collect::<Result<_, _>>()
        .map_err(db_err)?;
    if ids.iter().any(|i| !live.contains(i)) {
        return Err(errors::denied());
    }
    if owner_to_file_ids(state, &ids).await?.contains_key(&actor) {
        return Err(errors::bad_msg("can not copy files owned by actor"));
    }
    let sql = format!(
        "SELECT file_id, o_type, object_key FROM object_keys WHERE is_deleted = FALSE AND file_id IN ({})",
        ids_list(1, ids.len())
    );
    let mut objs: HashMap<(i64, String), String> = HashMap::new();
    for r in bind_ids(sqlx::query(&sql), &ids)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?
    {
        objs.insert(
            (
                get_i64(&r, "file_id").map_err(db_err)?,
                get_str(&r, "o_type").map_err(db_err)?,
            ),
            get_str(&r, "object_key").map_err(db_err)?,
        );
    }
    if objs.len() != 2 * ids.len() {
        return Err(errors::internal(format!(
            "expected {} objects, got {}",
            2 * ids.len(),
            objs.len()
        )));
    }
    usage::can_upload(state, actor, None, app).await?;
    let sql = format!(
        "SELECT file_id, owner_id, file_decryption_header, thumbnail_decryption_header,
            metadata_decryption_header, encrypted_metadata, pub_magic_metadata
         FROM files WHERE file_id IN ({})",
        ids_list(1, ids.len())
    );
    let mut sources: HashMap<i64, AnyRow> = HashMap::new();
    for r in bind_ids(sqlx::query(&sql), &ids)
        .fetch_all(&state.db.pool)
        .await
        .map_err(db_err)?
    {
        sources.insert(get_i64(&r, "file_id").map_err(db_err)?, r);
    }
    let mut out = BTreeMap::new();
    for item in &items {
        let r = sources
            .get(&item.id)
            .ok_or_else(|| errors::internal("source file attributes missing"))?;
        let mut new_keys = Vec::with_capacity(2);
        for o_type in [O_FILE, O_THUMBNAIL] {
            let src_key = objs
                .get(&(item.id, o_type.to_string()))
                .ok_or_else(|| errors::internal("source object missing"))?;
            let dst_key = objects::new_object_key(actor);
            objects::add_temp_object(
                state,
                &objects::TempObject {
                    object_key: dst_key.clone(),
                    user_id: Some(actor),
                    app: Some(app.as_str().into()),
                    purpose: Some(objects::PURPOSE_FILE_UPLOAD.into()),
                    client: Some(client.into()),
                    ..Default::default()
                },
            )
            .await?;
            state
                .store
                .copy(src_key, &dst_key)
                .await
                .map_err(errors::storage)?;
            new_keys.push(dst_key);
        }
        let file = File {
            owner_id: actor,
            collection_id: req.dst_collection_id,
            encrypted_key: item.encrypted_key.clone(),
            key_decryption_nonce: item.key_decryption_nonce.clone(),
            file: FileAttributes {
                object_key: new_keys[0].clone(),
                decryption_header: get_str(r, "file_decryption_header").map_err(db_err)?,
                ..Default::default()
            },
            thumbnail: FileAttributes {
                object_key: new_keys[1].clone(),
                decryption_header: get_str(r, "thumbnail_decryption_header").map_err(db_err)?,
                ..Default::default()
            },
            metadata: FileAttributes {
                decryption_header: get_str(r, "metadata_decryption_header").map_err(db_err)?,
                encrypted_data: get_str(r, "encrypted_metadata").map_err(db_err)?,
                ..Default::default()
            },
            pub_magic_metadata: MagicMetadata::parse(
                get_opt_str(r, "pub_magic_metadata")
                    .map_err(db_err)?
                    .as_deref(),
            ),
            updation_time: now_micros(),
            ..Default::default()
        };
        let created = create(state, actor, app, file).await?;
        out.insert(item.id, created.id);
    }
    Ok(out)
}
