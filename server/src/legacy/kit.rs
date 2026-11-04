//! Legacy kits: museum's `controller/legacy_kit` and `repo/legacy_kit`.
//!
//! The owner splits a kit secret into printed shares (2-of-3) client-side and
//! uploads only ciphertext plus an X25519 public key derived from the secret.
//! A holder who reconstructs the secret proves possession by opening a sealed
//! challenge, which opens (or resumes) a recovery session; after the notice
//! period they receive the encrypted recovery blob and can reset the password.

use crate::db::{Engine, now_micros};
use crate::entities::ids::new_uuid;
use crate::entities::schema::{begin_write, for_update, lock_user};
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use axum::http::StatusCode;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Recovery statuses (museum's `LegacyKitRecoveryStatus`).
pub mod status {
    /// Notice period running.
    pub const WAITING: &str = "WAITING";
    /// Recovery may proceed.
    pub const READY: &str = "READY";
    /// Owner blocked it.
    pub const BLOCKED: &str = "BLOCKED";
    /// Cancelled.
    pub const CANCELLED: &str = "CANCELLED";
    /// Password was reset.
    pub const RECOVERED: &str = "RECOVERED";
}

/// Allowed notice periods in hours.
pub const VALID_NOTICE_HOURS: [i64; 5] = [0, 24, 168, 360, 720];
/// Most active kits per user.
pub const MAX_ACTIVE_KITS: i64 = 5;
/// Only variant: 2-of-3 shares.
pub const VARIANT_TWO_OF_THREE: i64 = 1;
const TOKEN_LIFETIME: i64 = 24 * 3_600_000_000;

fn not_found() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND)
}

/// A `legacy_kit` row.
#[derive(Debug, Clone)]
pub struct KitRow {
    /// Kit id (UUID, client-chosen).
    pub id: String,
    /// Owner.
    pub user_id: i64,
    /// Variant.
    pub variant: i64,
    /// Notice period in hours.
    pub notice_period_in_hrs: i64,
    /// Encrypted recovery key blob.
    pub encrypted_recovery_blob: String,
    /// X25519 public key (base64).
    pub auth_public_key: String,
    /// Owner-only encrypted blob.
    pub encrypted_owner_blob: String,
    /// Soft-deleted.
    pub is_deleted: bool,
    /// Created (µs).
    pub created_at: i64,
    /// Updated (µs).
    pub updated_at: i64,
}

type KitTuple = (String, i64, i64, i64, String, String, String, i64, i64, i64);
const KIT_COLS: &str = "id, user_id, variant, notice_period_in_hrs, encrypted_recovery_blob, auth_public_key, encrypted_owner_blob, CASE WHEN is_deleted THEN 1 ELSE 0 END, created_at, updated_at";

impl From<KitTuple> for KitRow {
    fn from(t: KitTuple) -> Self {
        Self {
            id: t.0,
            user_id: t.1,
            variant: t.2,
            notice_period_in_hrs: t.3,
            encrypted_recovery_blob: t.4,
            auth_public_key: t.5,
            encrypted_owner_blob: t.6,
            is_deleted: t.7 != 0,
            created_at: t.8,
            updated_at: t.9,
        }
    }
}

/// museum's `LegacyKitRecoveryInitiator`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Initiator {
    /// Client-reported share indexes (audit hint only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub used_part_indexes: Vec<i64>,
    /// Client IP.
    #[serde(default)]
    pub ip: String,
    /// User agent.
    #[serde(default)]
    pub user_agent: String,
}

/// A `legacy_kit_recovery_session` row.
#[derive(Debug, Clone)]
pub struct SessionRow {
    /// Session id.
    pub id: String,
    /// Kit id.
    pub kit_id: String,
    /// Owner.
    pub user_id: i64,
    /// Status.
    pub status: String,
    /// Notice period captured at open.
    pub effective_notice_period_in_hrs: i64,
    /// End of wait (µs).
    pub wait_till: i64,
    /// Audit trail.
    pub initiators: Vec<Initiator>,
    /// Created (µs).
    pub created_at: i64,
}

type SessionTuple = (String, String, i64, String, i64, i64, String, i64);
const SESSION_COLS: &str = "id, kit_id, user_id, status, effective_notice_period_in_hrs, wait_till, initiators, created_at";

impl From<SessionTuple> for SessionRow {
    fn from(t: SessionTuple) -> Self {
        Self {
            id: t.0,
            kit_id: t.1,
            user_id: t.2,
            status: t.3,
            effective_notice_period_in_hrs: t.4,
            wait_till: t.5,
            initiators: serde_json::from_str(&t.6).unwrap_or_default(),
            created_at: t.7,
        }
    }
}

/// museum's `LegacyKitRecoverySession`.
#[derive(Debug, Clone, Serialize)]
pub struct RecoverySession {
    /// Session id.
    pub id: String,
    /// Kit id.
    #[serde(rename = "kitID")]
    pub kit_id: String,
    /// Status (`WAITING` shows as `READY` once the wait is over).
    pub status: String,
    /// Remaining wait in microseconds.
    #[serde(rename = "waitTill")]
    pub wait_till: i64,
    /// Created (µs).
    #[serde(rename = "createdAt")]
    pub created_at: i64,
}

impl SessionRow {
    /// museum's `toRecoverySession`.
    pub fn to_api(&self) -> RecoverySession {
        let remaining = (self.wait_till - now_micros()).max(0);
        let mut status = self.status.clone();
        if status == status::WAITING && remaining == 0 {
            status = status::READY.into();
        }
        RecoverySession {
            id: self.id.clone(),
            kit_id: self.kit_id.clone(),
            status,
            wait_till: remaining,
            created_at: self.created_at,
        }
    }
}

/// museum's `LegacyKit`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegacyKit {
    /// Kit id.
    pub id: String,
    /// Variant.
    pub variant: i64,
    /// Notice period in hours.
    pub notice_period_in_hours: i64,
    /// Where holders go to recover.
    pub legacy_url: String,
    /// Owner-only encrypted blob.
    pub encrypted_owner_blob: String,
    /// Created (µs).
    pub created_at: i64,
    /// Updated (µs).
    pub updated_at: i64,
    /// Active session, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_recovery_session: Option<RecoverySession>,
}

/// URL of the recovery web app (`PHOTOVAULT_LEGACY_URL`, museum's `apps.legacy`).
/// Defaults to this server's public URL, never a third-party site.
pub fn legacy_url() -> String {
    let var = |k: &str| {
        std::env::var(k)
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| !s.is_empty())
    };
    var("PHOTOVAULT_LEGACY_URL")
        .or_else(|| var("PHOTOVAULT_PUBLIC_URL"))
        .unwrap_or_else(|| "http://localhost:8080".to_string())
}

/// Convert a kit row for the API.
pub fn to_api(k: &KitRow, session: Option<&SessionRow>) -> LegacyKit {
    LegacyKit {
        id: k.id.clone(),
        variant: k.variant,
        notice_period_in_hours: k.notice_period_in_hrs,
        legacy_url: legacy_url(),
        encrypted_owner_blob: k.encrypted_owner_blob.clone(),
        created_at: k.created_at,
        updated_at: k.updated_at,
        active_recovery_session: session.map(SessionRow::to_api),
    }
}

/// museum's `ValidateSealedBoxPublicKey`: 32 bytes and not a low-order point.
pub fn validate_public_key(b64: &str) -> bool {
    let Ok(pk) = crate::crypto::unb64(b64) else {
        return false;
    };
    if pk.len() != 32 {
        return false;
    }
    let scalar =
        match hex::decode("a5465c1d0f3f1e0d49f5cf0a5dbf3d74b2c1f5d9a604de884812a4ccf4a4c5f0") {
            Ok(s) => s,
            Err(_) => return false,
        };
    crate::crypto::init();
    let mut out = [0u8; 32];
    // SAFETY: all buffers are 32 bytes; crypto_scalarmult returns -1 for an all-zero result.
    let rc =
        unsafe { libsodium_sys::crypto_scalarmult(out.as_mut_ptr(), scalar.as_ptr(), pk.as_ptr()) };
    rc == 0
}

fn hash_b64url(s: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(s.as_bytes()))
}

fn random_token() -> String {
    URL_SAFE_NO_PAD.encode(crate::crypto::random_bytes(32))
}

/// Insert a kit if the user is below the limit.
pub async fn create_kit(st: &AppState, user_id: i64, k: &KitRow) -> ApiResult<()> {
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, user_id).await? {
        return Err(not_found());
    }
    let (n,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM legacy_kit WHERE user_id = $1 AND is_deleted = FALSE")
            .bind(user_id)
            .fetch_one(&mut *tx)
            .await?;
    if n >= MAX_ACTIVE_KITS {
        return Err(ApiError::bad_request("legacy kit limit reached"));
    }
    let now = now_micros();
    sqlx::query(
        "INSERT INTO legacy_kit(id, user_id, variant, notice_period_in_hrs, encrypted_recovery_blob, auth_public_key, encrypted_owner_blob, is_deleted, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, $8, $8)",
    )
    .bind(&k.id)
    .bind(user_id)
    .bind(k.variant)
    .bind(k.notice_period_in_hrs)
    .bind(&k.encrypted_recovery_blob)
    .bind(&k.auth_public_key)
    .bind(&k.encrypted_owner_blob)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Active kits of a user, newest first.
pub async fn list_kits(st: &AppState, user_id: i64) -> ApiResult<Vec<KitRow>> {
    let rows: Vec<KitTuple> = sqlx::query_as(&format!(
        "SELECT {KIT_COLS} FROM legacy_kit WHERE user_id = $1 AND is_deleted = FALSE ORDER BY created_at DESC"
    ))
    .bind(user_id)
    .fetch_all(&st.db.pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Active sessions on a user's kits.
pub async fn active_sessions_for_user(st: &AppState, user_id: i64) -> ApiResult<Vec<SessionRow>> {
    let rows: Vec<SessionTuple> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLS} FROM legacy_kit_recovery_session WHERE user_id = $1 AND status IN ('WAITING', 'READY')"
    ))
    .bind(user_id)
    .fetch_all(&st.db.pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// An owner's active kit (404 `{}` otherwise).
pub async fn kit_for_owner(st: &AppState, user_id: i64, kit_id: &str) -> ApiResult<KitRow> {
    let row: Option<KitTuple> = sqlx::query_as(&format!(
        "SELECT {KIT_COLS} FROM legacy_kit WHERE id = $1 AND user_id = $2 AND is_deleted = FALSE"
    ))
    .bind(kit_id)
    .bind(user_id)
    .fetch_optional(&st.db.pool)
    .await?;
    row.map(Into::into).ok_or_else(not_found)
}

/// Any kit by id (404 `{}` when absent).
pub async fn kit_by_id(st: &AppState, kit_id: &str) -> ApiResult<KitRow> {
    let row: Option<KitTuple> =
        sqlx::query_as(&format!("SELECT {KIT_COLS} FROM legacy_kit WHERE id = $1"))
            .bind(kit_id)
            .fetch_optional(&st.db.pool)
            .await?;
    row.map(Into::into).ok_or_else(not_found)
}

async fn kit_for_owner_tx(
    conn: &mut sqlx::AnyConnection,
    engine: Engine,
    user_id: i64,
    kit_id: &str,
) -> ApiResult<Option<KitRow>> {
    let row: Option<KitTuple> = sqlx::query_as(&format!(
        "SELECT {KIT_COLS} FROM legacy_kit WHERE id = $1 AND user_id = $2 AND is_deleted = FALSE{}",
        for_update(engine)
    ))
    .bind(kit_id)
    .bind(user_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(Into::into))
}

async fn kit_by_id_tx(
    conn: &mut sqlx::AnyConnection,
    engine: Engine,
    kit_id: &str,
) -> ApiResult<Option<KitRow>> {
    let row: Option<KitTuple> = sqlx::query_as(&format!(
        "SELECT {KIT_COLS} FROM legacy_kit WHERE id = $1{}",
        for_update(engine)
    ))
    .bind(kit_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(Into::into))
}

/// Active session of a kit.
pub async fn active_session_for_kit(st: &AppState, kit_id: &str) -> ApiResult<Option<SessionRow>> {
    let row: Option<SessionTuple> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLS} FROM legacy_kit_recovery_session WHERE kit_id = $1 AND status IN ('WAITING', 'READY')"
    ))
    .bind(kit_id)
    .fetch_optional(&st.db.pool)
    .await?;
    Ok(row.map(Into::into))
}

async fn active_session_for_kit_tx(
    conn: &mut sqlx::AnyConnection,
    engine: Engine,
    kit_id: &str,
) -> ApiResult<Option<SessionRow>> {
    let row: Option<SessionTuple> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLS} FROM legacy_kit_recovery_session WHERE kit_id = $1 AND status IN ('WAITING', 'READY'){}",
        for_update(engine)
    ))
    .bind(kit_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(Into::into))
}

async fn block_active_session_tx(
    conn: &mut sqlx::AnyConnection,
    kit_id: &str,
    user_id: i64,
) -> ApiResult<bool> {
    let r = sqlx::query(
        "UPDATE legacy_kit_recovery_session SET status = $1, updated_at = $2 \
         WHERE kit_id = $3 AND user_id = $4 AND status IN ('WAITING', 'READY') \
         AND kit_id IN (SELECT id FROM legacy_kit WHERE user_id = $4)",
    )
    .bind(status::BLOCKED)
    .bind(now_micros())
    .bind(kit_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Soft-delete a kit (blocking any active session). False when not found.
pub async fn delete_kit(st: &AppState, user_id: i64, kit_id: &str) -> ApiResult<bool> {
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, user_id).await? {
        return Err(not_found());
    }
    if kit_for_owner_tx(&mut tx, db.engine, user_id, kit_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    block_active_session_tx(&mut tx, kit_id, user_id).await?;
    let now = now_micros();
    sqlx::query(
        "UPDATE legacy_kit SET is_deleted = TRUE, deleted_at = $1, updated_at = $1 WHERE id = $2 AND user_id = $3 AND is_deleted = FALSE",
    )
    .bind(now)
    .bind(kit_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(true)
}

/// Change the notice period. False when the kit is not found.
pub async fn update_notice(
    st: &AppState,
    user_id: i64,
    kit_id: &str,
    hours: i64,
) -> ApiResult<bool> {
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, user_id).await? {
        return Err(not_found());
    }
    if kit_for_owner_tx(&mut tx, db.engine, user_id, kit_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    if active_session_for_kit_tx(&mut tx, db.engine, kit_id)
        .await?
        .is_some()
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "ACTIVE_RECOVERY_SESSION",
            "cannot update recovery notice while there is an active recovery session",
        ));
    }
    let r = sqlx::query(
        "UPDATE legacy_kit SET notice_period_in_hrs = $1, updated_at = $2 WHERE id = $3 AND user_id = $4 AND is_deleted = FALSE",
    )
    .bind(hours)
    .bind(now_micros())
    .bind(kit_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(r.rows_affected() > 0)
}

/// Block the active session of an owner's kit. False when nothing was blocked.
pub async fn block_recovery(st: &AppState, user_id: i64, kit_id: &str) -> ApiResult<bool> {
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, user_id).await? {
        return Err(not_found());
    }
    if kit_for_owner_tx(&mut tx, db.engine, user_id, kit_id)
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let updated = block_active_session_tx(&mut tx, kit_id, user_id).await?;
    tx.commit().await?;
    Ok(updated)
}

/// museum's `formatChallenge`.
pub fn format_challenge(kit_id: &str, challenge: &str) -> String {
    format!("legacy-kit-open:v1\n{kit_id}\n{challenge}\n")
}

/// Create a challenge for a kit; returns (sealed challenge base64, expires_at).
pub async fn create_challenge(st: &AppState, kit: &KitRow) -> ApiResult<(String, i64)> {
    let challenge = format_challenge(
        &kit.id,
        &crate::crypto::b64(&crate::crypto::random_bytes(32)),
    );
    let pk = crate::crypto::unb64(&kit.auth_public_key).map_err(ApiError::internal)?;
    let sealed = crate::crypto::box_seal(challenge.as_bytes(), &pk).map_err(ApiError::internal)?;
    let now = now_micros();
    let expires_at = now + 3_600_000_000;
    sqlx::query(
        "INSERT INTO legacy_kit_challenge(id, kit_id, challenge_hash, expires_at, created_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(new_uuid())
    .bind(&kit.id)
    .bind(hash_b64url(&challenge))
    .bind(expires_at)
    .bind(now)
    .execute(&st.db.pool)
    .await?;
    Ok((crate::crypto::b64(&sealed), expires_at))
}

async fn issue_token(conn: &mut sqlx::AnyConnection, session_id: &str) -> ApiResult<String> {
    let raw = random_token();
    sqlx::query(
        "INSERT INTO legacy_kit_recovery_session_token(id, session_id, token_hash, created_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(new_uuid())
    .bind(session_id)
    .bind(hash_b64url(&raw))
    .bind(now_micros())
    .execute(&mut *conn)
    .await?;
    Ok(raw)
}

async fn ensure_ready_tx(
    conn: &mut sqlx::AnyConnection,
    session: SessionRow,
    now: i64,
) -> ApiResult<SessionRow> {
    if session.status != status::WAITING || session.wait_till > now {
        return Ok(session);
    }
    sqlx::query(
        "UPDATE legacy_kit_recovery_session SET status = $1, wait_till = $2, updated_at = $2 WHERE id = $3 AND status = $4",
    )
    .bind(status::READY)
    .bind(now)
    .bind(&session.id)
    .bind(status::WAITING)
    .execute(&mut *conn)
    .await?;
    let row: Option<SessionTuple> = sqlx::query_as(&format!(
        "SELECT {SESSION_COLS} FROM legacy_kit_recovery_session WHERE id = $1"
    ))
    .bind(&session.id)
    .fetch_optional(&mut *conn)
    .await?;
    row.map(Into::into).ok_or_else(not_found)
}

/// Result of [`open_or_resume`].
pub struct Opened {
    /// Kit owner.
    pub user_id: i64,
    /// The session.
    pub session: SessionRow,
    /// Raw session token for this browser.
    pub token: String,
    /// Whether a new session was created.
    pub created: bool,
}

/// museum's `OpenOrResumeRecovery`.
pub async fn open_or_resume(
    st: &AppState,
    kit_id: &str,
    challenge: &str,
    initiator: Initiator,
) -> ApiResult<Opened> {
    let db = &st.db;
    let owner: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM legacy_kit WHERE id = $1")
        .bind(kit_id)
        .fetch_optional(&db.pool)
        .await?;
    let (owner,) = owner.ok_or_else(not_found)?;
    let now = now_micros();
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, owner).await? {
        return Err(not_found());
    }
    let kit = kit_by_id_tx(&mut tx, db.engine, kit_id)
        .await?
        .ok_or_else(not_found)?;
    if kit.is_deleted {
        return Err(not_found());
    }
    let consumed = sqlx::query(
        "DELETE FROM legacy_kit_challenge WHERE kit_id = $1 AND challenge_hash = $2 AND expires_at >= $3",
    )
    .bind(kit_id)
    .bind(hash_b64url(challenge))
    .bind(now)
    .execute(&mut *tx)
    .await?;
    if consumed.rows_affected() == 0 {
        return Err(ApiError::bad_request(
            "legacy kit challenge is invalid or expired",
        ));
    }
    let existing = active_session_for_kit_tx(&mut tx, db.engine, kit_id).await?;
    let (session, token, created) = match existing {
        None => {
            let mut s = SessionRow {
                id: new_uuid(),
                kit_id: kit_id.to_string(),
                user_id: kit.user_id,
                status: status::WAITING.into(),
                effective_notice_period_in_hrs: kit.notice_period_in_hrs,
                wait_till: now + kit.notice_period_in_hrs * 3_600_000_000,
                initiators: vec![initiator],
                created_at: now,
            };
            let mut next_reminder = Some(now);
            if kit.notice_period_in_hrs == 0 {
                s.status = status::READY.into();
                s.wait_till = now;
                next_reminder = None;
            }
            let initiators = serde_json::to_string(&s.initiators).map_err(ApiError::internal)?;
            sqlx::query(
                "INSERT INTO legacy_kit_recovery_session(id, kit_id, user_id, status, effective_notice_period_in_hrs, wait_till, next_reminder_at, initiators, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)",
            )
            .bind(&s.id)
            .bind(&s.kit_id)
            .bind(s.user_id)
            .bind(&s.status)
            .bind(s.effective_notice_period_in_hrs)
            .bind(s.wait_till)
            .bind(next_reminder)
            .bind(initiators)
            .bind(now)
            .execute(&mut *tx)
            .await?;
            let token = issue_token(&mut tx, &s.id).await?;
            (s, token, true)
        }
        Some(s) => {
            let mut s = ensure_ready_tx(&mut tx, s, now).await?;
            s.initiators.push(initiator);
            let initiators = serde_json::to_string(&s.initiators).map_err(ApiError::internal)?;
            sqlx::query(
                "UPDATE legacy_kit_recovery_session SET initiators = $1, updated_at = $2 WHERE id = $3",
            )
            .bind(initiators)
            .bind(now)
            .bind(&s.id)
            .execute(&mut *tx)
            .await?;
            let token = issue_token(&mut tx, &s.id).await?;
            (s, token, false)
        }
    };
    tx.commit().await?;
    Ok(Opened {
        user_id: kit.user_id,
        session,
        token,
        created,
    })
}

async fn session_by_token_tx(
    conn: &mut sqlx::AnyConnection,
    engine: Engine,
    session_id: &str,
    token: &str,
    now: i64,
) -> ApiResult<Option<SessionRow>> {
    let th = hash_b64url(token);
    let row: Option<SessionTuple> = sqlx::query_as(&format!(
        "SELECT s.id, s.kit_id, s.user_id, s.status, s.effective_notice_period_in_hrs, s.wait_till, s.initiators, s.created_at \
         FROM legacy_kit_recovery_session s JOIN legacy_kit_recovery_session_token t ON t.session_id = s.id \
         WHERE s.id = $1 AND t.token_hash = $2 AND t.created_at > $3{}",
        match engine {
            Engine::Postgres => " FOR UPDATE OF s",
            Engine::Sqlite => "",
        }
    ))
    .bind(session_id)
    .bind(&th)
    .bind(now - TOKEN_LIFETIME)
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(r) = row {
        return Ok(Some(r.into()));
    }
    let (expired,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM legacy_kit_recovery_session_token WHERE session_id = $1 AND token_hash = $2 AND created_at <= $3",
    )
    .bind(session_id)
    .bind(&th)
    .bind(now - TOKEN_LIFETIME)
    .fetch_one(&mut *conn)
    .await?;
    if expired > 0 {
        return Err(ApiError::new(
            StatusCode::GONE,
            "SESSION_EXPIRED",
            "Legacy kit recovery access expired",
        ));
    }
    Ok(None)
}

/// museum's `GetSessionByIDAndTokenForUse`: the session for a valid token, with
/// a finished notice period promoted to READY. `None` when the token is unknown.
pub async fn session_for_use(
    st: &AppState,
    session_id: &str,
    token: &str,
) -> ApiResult<Option<SessionRow>> {
    let db = &st.db;
    let now = now_micros();
    let mut tx = begin_write(db).await?;
    let Some(s) = session_by_token_tx(&mut tx, db.engine, session_id, token, now).await? else {
        return Ok(None);
    };
    let s = ensure_ready_tx(&mut tx, s, now).await?;
    tx.commit().await?;
    Ok(Some(s))
}

/// museum's `Repo.CompleteRecovery`, inside the password-update transaction.
pub async fn complete_recovery_tx(
    conn: &mut sqlx::AnyConnection,
    engine: Engine,
    session_id: &str,
    kit_id: &str,
    user_id: i64,
    token: &str,
) -> ApiResult<()> {
    let kit = kit_by_id_tx(conn, engine, kit_id)
        .await?
        .ok_or_else(not_found)?;
    if kit.is_deleted || kit.user_id != user_id {
        return Err(not_found());
    }
    let s = session_by_token_tx(conn, engine, session_id, token, now_micros())
        .await?
        .ok_or_else(not_found)?;
    if s.kit_id != kit_id || s.user_id != user_id {
        return Err(not_found());
    }
    if s.status != status::READY {
        return Err(ApiError::bad_request("legacy kit recovery is not ready"));
    }
    sqlx::query(
        "UPDATE legacy_kit_recovery_session SET status = $1, updated_at = $2 WHERE id = $3",
    )
    .bind(status::RECOVERED)
    .bind(now_micros())
    .bind(session_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

fn delay_label(hours: i64) -> String {
    match hours {
        0 => "immediately".into(),
        24 => "1 day".into(),
        h => format!("{} days", h / 24),
    }
}

fn send(
    st: &AppState,
    user_id: i64,
    subject: &'static str,
    body: impl FnOnce(&str) -> String + Send + 'static,
) {
    let st = st.clone();
    tokio::spawn(async move {
        match crate::legacy::users::email_of(&st, user_id).await {
            Ok(Some(email)) => {
                if let Err(e) = crate::mail::send(&st.config, &email, subject, &body(&email)).await
                {
                    tracing::error!(error = %e, subject, "failed to send legacy kit email");
                }
            }
            _ => tracing::warn!(user_id, "legacy kit email skipped: owner not found"),
        }
    });
}

/// "Kit created" email.
pub fn notify_created(st: &AppState, user_id: i64) {
    send(st, user_id, "Your Legacy kit was created", |_| {
        "A Legacy kit was created for your account. Keep its printed parts with people you trust."
            .into()
    });
}

/// "Recovery started" email.
pub fn notify_started(st: &AppState, user_id: i64, notice_hrs: i64, beneficiary: Option<String>) {
    send(
        st,
        user_id,
        "Legacy Kit recovery initiated",
        move |account| {
            let who = beneficiary
                .filter(|b| !b.is_empty())
                .map(|b| format!(" by {b}"))
                .unwrap_or_default();
            let when = if notice_hrs == 0 {
                "The account can be recovered immediately.".to_string()
            } else {
                format!(
                    "The account can be recovered after {}, unless you block the recovery in the app.",
                    delay_label(notice_hrs)
                )
            };
            format!("A recovery of {account} was started with one of your Legacy kits{who}. {when}")
        },
    );
}

/// "Recovery completed" email.
pub fn notify_completed(st: &AppState, user_id: i64) {
    send(st, user_id, "Legacy Kit recovery completed", |account| {
        format!("The password of {account} was reset using a Legacy kit.")
    });
}
