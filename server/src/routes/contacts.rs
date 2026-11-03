//! Endpoints: /contacts/* and /attachments/* — encrypted contact cards and
//! their attachments (profile pictures).
//!
//! Mirrors museum's `pkg/api/contact.go`, `pkg/controller/contact` and
//! `pkg/repo/contact` over `contact_entity` and `user_attachments`.
//! Attachment objects use museum's key layout
//! `<userID>/user-attachments/<type>/<attachmentID>` and are uploaded/downloaded
//! directly through signed URLs from [`crate::storage::ObjectStore`].
//! Uploads are staged in `user_attachment_uploads` (museum uses its generic
//! `temp_objects`) until committed; stale stages and deleted attachments are
//! removed from the store by [`cleanup_attachments`].

use crate::auth::StorageUser;
use crate::db::now_micros;
use crate::entities::http::{JsonBody, QueryMap};
use crate::entities::ids::new_nano_id;
use crate::entities::schema::{begin_write, for_update, has_column, has_table};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;

/// Routes for this group.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/contacts", post(create))
        .route("/contacts/diff", get(diff))
        .route("/contacts/{id}", get(get_one).put(update).delete(remove))
        .route("/attachments/{type}/upload-url", post(attachment_upload_url))
        .route("/attachments/{type}/{attachment_id}", get(get_attachment))
        .route(
            "/contacts/{id}/attachments/{type}",
            put(attach).get(current_attachment).delete(delete_attachment),
        )
        .route(
            "/contacts/{id}/profile-picture/upload-url",
            post(profile_picture_upload_url),
        )
        .route(
            "/contacts/{id}/profile-picture",
            put(attach_profile_picture)
                .get(get_profile_picture)
                .delete(delete_profile_picture),
        )
}

/// The only attachment type.
pub const PROFILE_PICTURE: &str = "profile_picture";
/// Largest encrypted profile picture.
pub const MAX_PROFILE_PICTURE_SIZE: i64 = 512 * 1024;
const URL_VALIDITY: Duration = Duration::from_secs(7 * 24 * 3600);

fn bind_error() -> ApiError {
    ApiError::status(StatusCode::BAD_REQUEST)
}

fn not_found_typed() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "")
}

fn valid_contact_id(id: &str) -> bool {
    id.starts_with("ct_") && id.len() > 3
}

fn valid_attachment_id(id: &str) -> bool {
    id.starts_with("ua_") && id.len() > 3
}

fn check_contact_id(id: &str) -> ApiResult<()> {
    if valid_contact_id(id) {
        Ok(())
    } else {
        Err(ApiError::bad_request("invalid contact id"))
    }
}

/// Validated attachment type and its maximum encrypted size.
fn attachment_policy(raw: &str) -> ApiResult<i64> {
    if raw == PROFILE_PICTURE {
        Ok(MAX_PROFILE_PICTURE_SIZE)
    } else {
        Err(ApiError::bad_request(format!("invalid attachment type: {raw}")))
    }
}

/// museum's `AttachmentObjectKey`.
pub fn attachment_object_key(user_id: i64, kind: &str, attachment_id: &str) -> String {
    format!("{user_id}/user-attachments/{kind}/{attachment_id}")
}

/// museum's contact `Entity`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactEntity {
    id: String,
    #[serde(rename = "userID")]
    user_id: i64,
    #[serde(rename = "contactUserID")]
    contact_user_id: i64,
    email: Option<String>,
    #[serde(rename = "profilePictureAttachmentID")]
    profile_picture_attachment_id: Option<String>,
    encrypted_key: Option<String>,
    encrypted_data: Option<String>,
    is_deleted: bool,
    created_at: i64,
    updated_at: i64,
}

type ContactTuple = (
    String,
    i64,
    i64,
    Option<String>,
    String,
    Option<String>,
    bool,
    i64,
    i64,
    Option<String>,
    Option<String>,
);

const SELECT_WITH_EMAIL: &str = "SELECT c.id, c.user_id, c.contact_user_id, c.profile_picture_attachment_id, c.encrypted_key, \
     c.encrypted_data, c.is_deleted, c.created_at, c.updated_at, u.encrypted_email, u.email_decryption_nonce \
     FROM contact_entity c LEFT JOIN users u ON u.user_id = c.contact_user_id";

fn to_entity(st: &AppState, t: ContactTuple) -> ApiResult<ContactEntity> {
    let mut e = ContactEntity {
        id: t.0,
        user_id: t.1,
        contact_user_id: t.2,
        email: None,
        profile_picture_attachment_id: t.3,
        encrypted_key: Some(t.4),
        encrypted_data: t.5,
        is_deleted: t.6,
        created_at: t.7,
        updated_at: t.8,
    };
    if e.is_deleted {
        e.profile_picture_attachment_id = None;
        e.encrypted_key = None;
        e.encrypted_data = None;
        return Ok(e);
    }
    if let (Some(enc), Some(nonce)) = (t.9.filter(|s| !s.is_empty()), t.10.filter(|s| !s.is_empty())) {
        e.email = Some(
            crate::legacy::users::decrypt_email(st, &enc, &nonce)
                .ok_or_else(|| ApiError::internal("failed to decrypt contact email"))?,
        );
    }
    Ok(e)
}

async fn fetch(st: &AppState, user_id: i64, id: &str) -> ApiResult<ContactEntity> {
    let row: Option<ContactTuple> =
        sqlx::query_as(&format!("{SELECT_WITH_EMAIL} WHERE c.id = $1 AND c.user_id = $2"))
            .bind(id)
            .bind(user_id)
            .fetch_optional(&st.db.pool)
            .await?;
    to_entity(st, row.ok_or_else(not_found_typed)?)
}

/// `(contact_user_id, profile_picture_attachment_id, is_deleted)` under lock.
async fn contact_for_update(
    conn: &mut sqlx::AnyConnection,
    engine: crate::db::Engine,
    user_id: i64,
    id: &str,
) -> ApiResult<(i64, Option<String>, bool)> {
    let row: Option<(i64, Option<String>, bool)> = sqlx::query_as(&format!(
        "SELECT contact_user_id, profile_picture_attachment_id, is_deleted FROM contact_entity WHERE id = $1 AND user_id = $2{}",
        for_update(engine)
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    row.ok_or_else(not_found_typed)
}

async fn contact_user_exists(st: &AppState, contact_user_id: i64) -> ApiResult<bool> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM users WHERE user_id = $1 AND encrypted_email IS NOT NULL",
    )
    .bind(contact_user_id)
    .fetch_one(&st.db.pool)
    .await?;
    Ok(n > 0)
}

/// museum's `CanCreateContact`: the two users share an active trusted-contact
/// relationship, a family, or a shared collection.
pub async fn can_create_contact(st: &AppState, actor: i64, contact: i64) -> ApiResult<bool> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM emergency_contact WHERE state IN ('INVITED', 'ACCEPTED') \
         AND ((user_id = $1 AND emergency_contact_id = $2) OR (user_id = $2 AND emergency_contact_id = $1))",
    )
    .bind(actor)
    .bind(contact)
    .fetch_one(&st.db.pool)
    .await?;
    if n > 0 {
        return Ok(true);
    }
    if has_column(&st.db, "users", "family_admin_id").await {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM users a JOIN users b ON a.family_admin_id = b.family_admin_id \
             WHERE a.user_id = $1 AND b.user_id = $2 AND a.family_admin_id IS NOT NULL",
        )
        .bind(actor)
        .bind(contact)
        .fetch_one(&st.db.pool)
        .await?;
        if n > 0 {
            return Ok(true);
        }
    }
    if has_table(&st.db, "collection_shares").await {
        let (n,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM collection_shares a JOIN collection_shares b ON a.collection_id = b.collection_id \
             WHERE a.is_deleted = FALSE AND b.is_deleted = FALSE \
             AND (a.to_user_id = $1 OR a.from_user_id = $1) AND (b.to_user_id = $2 OR b.from_user_id = $2)",
        )
        .bind(actor)
        .bind(contact)
        .fetch_one(&st.db.pool)
        .await?;
        if n > 0 {
            return Ok(true);
        }
    }
    Ok(false)
}

/// museum's `TouchContactsForContactUser`: bump `updated_at` of every live
/// contact card pointing at `contact_user_id` (e.g. after an email change), so
/// owners re-sync the email. For the users workstream to call.
pub async fn touch_contacts_for_contact_user(st: &AppState, contact_user_id: i64) -> ApiResult<()> {
    sqlx::query(
        "UPDATE contact_entity SET updated_at = $1 WHERE contact_user_id = $2 AND is_deleted = FALSE",
    )
    .bind(now_micros())
    .bind(contact_user_id)
    .execute(&st.db.pool)
    .await?;
    Ok(())
}

/// Go's `[]byte` JSON field: standard base64; returns the canonical encoding.
fn bytes_field(raw: &str) -> ApiResult<String> {
    let b = crate::crypto::unb64(raw).map_err(|_| bind_error())?;
    Ok(crate::crypto::b64(&b))
}

fn is_unique_violation(e: &sqlx::Error) -> bool {
    e.as_database_error()
        .is_some_and(|d| d.is_unique_violation())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest {
    #[serde(rename = "contactUserID", default)]
    contact_user_id: i64,
    #[serde(default)]
    encrypted_key: Option<String>,
    #[serde(default)]
    encrypted_data: Option<String>,
}

async fn create(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    JsonBody(req): JsonBody<CreateRequest>,
) -> ApiResult<Json<ContactEntity>> {
    let (Some(key), Some(data)) = (req.encrypted_key, req.encrypted_data) else {
        return Err(bind_error());
    };
    if req.contact_user_id == 0 {
        return Err(bind_error());
    }
    let key = bytes_field(&key)?;
    let data = bytes_field(&data)?;
    if req.contact_user_id < 0 {
        return Err(ApiError::bad_request("contactUserID must be greater than 0"));
    }
    if key.is_empty() {
        return Err(ApiError::bad_request("encryptedKey is required"));
    }
    if data.is_empty() {
        return Err(ApiError::bad_request("encryptedData is required"));
    }
    if !contact_user_exists(&st, req.contact_user_id).await? {
        return Err(ApiError::bad_request("contactUserID does not exist"));
    }
    if !can_create_contact(&st, user.user_id, req.contact_user_id).await? {
        return Err(ApiError::bad_request(
            "contactUserID is not eligible to be added as a contact",
        ));
    }
    let id = new_nano_id("ct").ok_or_else(|| ApiError::internal("id generation failed"))?;
    let now = now_micros();
    let r = sqlx::query(
        "INSERT INTO contact_entity(id, user_id, contact_user_id, encrypted_key, encrypted_data, is_deleted, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, $6, $6)",
    )
    .bind(&id)
    .bind(user.user_id)
    .bind(req.contact_user_id)
    .bind(&key)
    .bind(&data)
    .bind(now)
    .execute(&st.db.pool)
    .await;
    match r {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::bad_request(
                "contact already exists for contactUserID",
            ));
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Json(fetch(&st, user.user_id, &id).await?))
}

async fn get_one(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Json<ContactEntity>> {
    check_contact_id(&id)?;
    Ok(Json(fetch(&st, user.user_id, &id).await?))
}

async fn diff(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    let since = q.int("sinceTime", i64::MIN, i64::MAX)?;
    let limit = q.int("limit", i16::MIN as i64, i16::MAX as i64)?;
    if limit == 0 {
        return Err(bind_error());
    }
    if !(1..=5000).contains(&limit) {
        return Err(ApiError::bad_request("limit must be between 1 and 5000"));
    }
    let rows: Vec<ContactTuple> = sqlx::query_as(&format!(
        "{SELECT_WITH_EMAIL} WHERE c.user_id = $1 AND c.updated_at > $2 ORDER BY c.updated_at LIMIT $3"
    ))
    .bind(user.user_id)
    .bind(since)
    .bind(limit)
    .fetch_all(&st.db.pool)
    .await?;
    let diff = rows
        .into_iter()
        .map(|r| to_entity(&st, r))
        .collect::<ApiResult<Vec<_>>>()?;
    Ok(Json(json!({ "diff": diff })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateRequest {
    #[serde(rename = "contactUserID", default)]
    contact_user_id: i64,
    #[serde(default)]
    encrypted_data: Option<String>,
}

async fn update(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path(id): Path<String>,
    JsonBody(req): JsonBody<UpdateRequest>,
) -> ApiResult<Json<ContactEntity>> {
    let Some(data) = req.encrypted_data else {
        return Err(bind_error());
    };
    if req.contact_user_id == 0 {
        return Err(bind_error());
    }
    let data = bytes_field(&data)?;
    check_contact_id(&id)?;
    if req.contact_user_id < 0 {
        return Err(ApiError::bad_request("contactUserID must be greater than 0"));
    }
    if data.is_empty() {
        return Err(ApiError::bad_request("encryptedData is required"));
    }
    if !contact_user_exists(&st, req.contact_user_id).await? {
        return Err(ApiError::bad_request("contactUserID does not exist"));
    }
    let state: Option<(i64, bool)> = sqlx::query_as(
        "SELECT contact_user_id, is_deleted FROM contact_entity WHERE id = $1 AND user_id = $2",
    )
    .bind(&id)
    .bind(user.user_id)
    .fetch_optional(&st.db.pool)
    .await?;
    let (current_contact, deleted) = state.ok_or_else(not_found_typed)?;
    if (deleted || current_contact != req.contact_user_id)
        && !can_create_contact(&st, user.user_id, req.contact_user_id).await?
    {
        return Err(ApiError::bad_request(
            "contactUserID is not eligible to be added as a contact",
        ));
    }
    let r = sqlx::query(
        "UPDATE contact_entity SET contact_user_id = $1, encrypted_data = $2, is_deleted = FALSE, updated_at = $3 \
         WHERE id = $4 AND user_id = $5",
    )
    .bind(req.contact_user_id)
    .bind(&data)
    .bind(now_micros())
    .bind(&id)
    .bind(user.user_id)
    .execute(&st.db.pool)
    .await;
    match r {
        Ok(r) if r.rows_affected() == 1 => {}
        Ok(_) => {
            fetch(&st, user.user_id, &id).await?;
            return Err(ApiError::internal("exactly one row should be updated"));
        }
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::bad_request(
                "contact already exists for contactUserID",
            ));
        }
        Err(e) => return Err(e.into()),
    }
    Ok(Json(fetch(&st, user.user_id, &id).await?))
}

async fn mark_attachment_deleted(
    conn: &mut sqlx::AnyConnection,
    user_id: i64,
    kind: &str,
    attachment_id: &str,
) -> ApiResult<()> {
    sqlx::query(
        "UPDATE user_attachments SET is_deleted = TRUE, pending_sync = TRUE, sync_locked_till = 0, updated_at = $1 \
         WHERE attachment_id = $2 AND user_id = $3 AND attachment_type = $4 AND is_deleted = FALSE",
    )
    .bind(now_micros())
    .bind(attachment_id)
    .bind(user_id)
    .bind(kind)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn remove(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    check_contact_id(&id)?;
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    let (_, picture, deleted) = contact_for_update(&mut tx, db.engine, user.user_id, &id).await?;
    if deleted {
        tx.commit().await?;
        return Ok(StatusCode::OK);
    }
    if let Some(p) = picture {
        mark_attachment_deleted(&mut tx, user.user_id, PROFILE_PICTURE, &p).await?;
    }
    sqlx::query(
        "UPDATE contact_entity SET is_deleted = TRUE, encrypted_data = NULL, profile_picture_attachment_id = NULL, updated_at = $1 \
         WHERE id = $2 AND user_id = $3",
    )
    .bind(now_micros())
    .bind(&id)
    .bind(user.user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadUrlRequest {
    #[serde(default)]
    content_length: i64,
    #[serde(rename = "contentMD5", default)]
    content_md5: String,
}

async fn upload_url(
    st: &AppState,
    user_id: i64,
    kind: &str,
    req: UploadUrlRequest,
) -> ApiResult<Json<Value>> {
    if req.content_length == 0 || req.content_md5.is_empty() {
        return Err(bind_error());
    }
    let max = attachment_policy(kind)?;
    if req.content_length < 0 {
        return Err(ApiError::bad_request("contentLength must be greater than 0"));
    }
    if req.content_length > max {
        return Err(ApiError::bad_request(format!("{kind} must be <= {max} bytes")));
    }
    if req.content_md5.trim().is_empty() {
        return Err(ApiError::bad_request("contentMD5 is required"));
    }
    let attachment_id =
        new_nano_id("ua").ok_or_else(|| ApiError::internal("id generation failed"))?;
    let key = attachment_object_key(user_id, kind, &attachment_id);
    let url = st
        .store
        .presign_put(&key, URL_VALIDITY, Some(req.content_length), Some(&req.content_md5))
        .await?;
    let now = now_micros();
    sqlx::query(
        "INSERT INTO user_attachment_uploads(object_key, user_id, attachment_type, attachment_id, content_length, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(&key)
    .bind(user_id)
    .bind(kind)
    .bind(&attachment_id)
    .bind(req.content_length)
    .bind(now + 2 * URL_VALIDITY.as_micros() as i64)
    .bind(now)
    .execute(&st.db.pool)
    .await?;
    Ok(Json(json!({ "attachmentID": attachment_id, "url": url })))
}

async fn attachment_upload_url(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path(kind): Path<String>,
    JsonBody(req): JsonBody<UploadUrlRequest>,
) -> ApiResult<Json<Value>> {
    upload_url(&st, user.user_id, &kind, req).await
}

async fn profile_picture_upload_url(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path(_id): Path<String>,
    JsonBody(req): JsonBody<UploadUrlRequest>,
) -> ApiResult<Json<Value>> {
    upload_url(&st, user.user_id, PROFILE_PICTURE, req).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommitRequest {
    #[serde(rename = "attachmentID", default)]
    attachment_id: String,
    #[serde(default)]
    size: i64,
}

async fn commit_attachment(
    st: &AppState,
    user_id: i64,
    contact_id: &str,
    kind: &str,
    req: CommitRequest,
) -> ApiResult<Json<ContactEntity>> {
    if req.attachment_id.is_empty() || req.size == 0 {
        return Err(bind_error());
    }
    check_contact_id(contact_id)?;
    let max = attachment_policy(kind)?;
    if !valid_attachment_id(&req.attachment_id) {
        return Err(ApiError::bad_request("invalid attachmentID"));
    }
    if req.size < 0 {
        return Err(ApiError::bad_request("size must be greater than 0"));
    }
    if req.size > max {
        return Err(ApiError::bad_request(format!("{kind} must be <= {max} bytes")));
    }
    let key = attachment_object_key(user_id, kind, &req.attachment_id);
    let staged: Option<(i64,)> =
        sqlx::query_as("SELECT user_id FROM user_attachment_uploads WHERE object_key = $1")
            .bind(&key)
            .fetch_optional(&st.db.pool)
            .await?;
    if staged.is_none() {
        return Err(ApiError::bad_request("staged attachment upload not found"));
    }
    match st.store.head(&key).await? {
        Some(n) if n as i64 == req.size => {}
        _ => return Err(ApiError::internal("attachment size does not match expected size")),
    }
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    let (_, current, deleted) = contact_for_update(&mut tx, db.engine, user_id, contact_id).await?;
    if deleted {
        return Err(ApiError::bad_request("contact is already deleted"));
    }
    let staged: Option<(i64,)> = sqlx::query_as(&format!(
        "SELECT user_id FROM user_attachment_uploads WHERE object_key = $1{}",
        for_update(db.engine)
    ))
    .bind(&key)
    .fetch_optional(&mut *tx)
    .await?;
    if staged.is_none() {
        return Err(ApiError::bad_request("staged attachment upload not found"));
    }
    let now = now_micros();
    let r = sqlx::query(
        "INSERT INTO user_attachments(attachment_id, user_id, attachment_type, size, latest_bucket, is_deleted, pending_sync, sync_locked_till, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, FALSE, TRUE, 0, $6, $6)",
    )
    .bind(&req.attachment_id)
    .bind(user_id)
    .bind(kind)
    .bind(req.size)
    .bind(st.store.name())
    .bind(now)
    .execute(&mut *tx)
    .await;
    match r {
        Ok(_) => {}
        Err(e) if is_unique_violation(&e) => {
            return Err(ApiError::bad_request("attachmentID already exists"));
        }
        Err(e) => return Err(e.into()),
    }
    if let Some(cur) = current.filter(|c| !c.is_empty()) {
        mark_attachment_deleted(&mut tx, user_id, kind, &cur).await?;
    }
    sqlx::query(
        "UPDATE contact_entity SET profile_picture_attachment_id = $1, updated_at = $2 WHERE id = $3 AND user_id = $4 AND is_deleted = FALSE",
    )
    .bind(&req.attachment_id)
    .bind(now)
    .bind(contact_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM user_attachment_uploads WHERE object_key = $1")
        .bind(&key)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Json(fetch(st, user_id, contact_id).await?))
}

async fn attach(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path((id, kind)): Path<(String, String)>,
    JsonBody(req): JsonBody<CommitRequest>,
) -> ApiResult<Json<ContactEntity>> {
    commit_attachment(&st, user.user_id, &id, &kind, req).await
}

async fn attach_profile_picture(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path(id): Path<String>,
    JsonBody(req): JsonBody<CommitRequest>,
) -> ApiResult<Json<ContactEntity>> {
    commit_attachment(&st, user.user_id, &id, PROFILE_PICTURE, req).await
}

async fn attachment_url(
    st: &AppState,
    user_id: i64,
    kind: &str,
    attachment_id: &str,
) -> ApiResult<Json<Value>> {
    attachment_policy(kind)?;
    if !valid_attachment_id(attachment_id) {
        return Err(ApiError::bad_request("invalid attachment id"));
    }
    let row: Option<(String, bool)> = sqlx::query_as(
        "SELECT attachment_type, is_deleted FROM user_attachments WHERE attachment_id = $1 AND user_id = $2",
    )
    .bind(attachment_id)
    .bind(user_id)
    .fetch_optional(&st.db.pool)
    .await?;
    let (stored_kind, deleted) = row.ok_or_else(not_found_typed)?;
    if deleted || stored_kind != kind {
        return Err(not_found_typed());
    }
    let url = st
        .store
        .presign_get(&attachment_object_key(user_id, &stored_kind, attachment_id), URL_VALIDITY)
        .await?;
    Ok(Json(json!({ "url": url })))
}

async fn get_attachment(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path((kind, attachment_id)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    attachment_url(&st, user.user_id, &kind, &attachment_id).await
}

async fn current_url(st: &AppState, user_id: i64, contact_id: &str, kind: &str) -> ApiResult<Json<Value>> {
    check_contact_id(contact_id)?;
    attachment_policy(kind)?;
    let e = fetch(st, user_id, contact_id).await?;
    if e.is_deleted {
        return Err(not_found_typed());
    }
    let id = e.profile_picture_attachment_id.ok_or_else(not_found_typed)?;
    attachment_url(st, user_id, kind, &id).await
}

async fn current_attachment(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path((id, kind)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    current_url(&st, user.user_id, &id, &kind).await
}

async fn get_profile_picture(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path(id): Path<String>,
    q: QueryMap,
) -> ApiResult<Json<Value>> {
    if let Some(attachment_id) = q.str("attachmentID") {
        return attachment_url(&st, user.user_id, PROFILE_PICTURE, attachment_id).await;
    }
    current_url(&st, user.user_id, &id, PROFILE_PICTURE).await
}

async fn remove_attachment(
    st: &AppState,
    user_id: i64,
    contact_id: &str,
    kind: &str,
) -> ApiResult<Json<ContactEntity>> {
    check_contact_id(contact_id)?;
    attachment_policy(kind)?;
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    let (_, current, deleted) = contact_for_update(&mut tx, db.engine, user_id, contact_id).await?;
    if deleted {
        return Err(ApiError::bad_request("contact is already deleted"));
    }
    if let Some(cur) = current {
        mark_attachment_deleted(&mut tx, user_id, kind, &cur).await?;
        sqlx::query(
            "UPDATE contact_entity SET profile_picture_attachment_id = NULL, updated_at = $1 WHERE id = $2 AND user_id = $3 AND is_deleted = FALSE",
        )
        .bind(now_micros())
        .bind(contact_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(Json(fetch(st, user_id, contact_id).await?))
}

async fn delete_attachment(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path((id, kind)): Path<(String, String)>,
) -> ApiResult<Json<ContactEntity>> {
    remove_attachment(&st, user.user_id, &id, &kind).await
}

async fn delete_profile_picture(
    State(st): State<AppState>,
    StorageUser(user): StorageUser,
    Path(id): Path<String>,
) -> ApiResult<Json<ContactEntity>> {
    remove_attachment(&st, user.user_id, &id, PROFILE_PICTURE).await
}

/// Delete objects of deleted attachments and of expired, never-committed
/// uploads, then their rows. Run periodically by [`crate::legacy::spawn_background`].
pub async fn cleanup_attachments(st: &AppState) -> ApiResult<()> {
    let deleted: Vec<(String, i64, String)> = sqlx::query_as(
        "SELECT attachment_id, user_id, attachment_type FROM user_attachments WHERE is_deleted = TRUE LIMIT 500",
    )
    .fetch_all(&st.db.pool)
    .await?;
    for (id, user_id, kind) in deleted {
        st.store
            .delete(&attachment_object_key(user_id, &kind, &id))
            .await?;
        sqlx::query("DELETE FROM user_attachments WHERE attachment_id = $1 AND is_deleted = TRUE")
            .bind(&id)
            .execute(&st.db.pool)
            .await?;
    }
    let expired: Vec<(String,)> = sqlx::query_as(
        "SELECT object_key FROM user_attachment_uploads WHERE expires_at < $1 LIMIT 500",
    )
    .bind(now_micros())
    .fetch_all(&st.db.pool)
    .await?;
    for (key,) in expired {
        st.store.delete(&key).await?;
        sqlx::query("DELETE FROM user_attachment_uploads WHERE object_key = $1")
            .bind(&key)
            .execute(&st.db.pool)
            .await?;
    }
    Ok(())
}
