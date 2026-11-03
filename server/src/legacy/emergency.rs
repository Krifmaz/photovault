//! Trusted contacts ("Legacy"): museum's `controller/emergency` and
//! `repo/emergency`, over the `emergency_contact` and `emergency_recovery` tables.
//!
//! A user invites a trusted contact, handing over their recovery key encrypted
//! to the contact's public key (`encrypted_key`, opaque to the server). Once the
//! contact accepts, they may start a recovery; after the notice period (unless
//! the owner rejects it first) the contact can fetch the encrypted recovery key
//! and reset the owner's password.

use crate::db::{Engine, now_micros};
use crate::entities::ids::new_uuid;
use crate::entities::schema::{begin_write, for_update, lock_user};
use crate::error::{ApiError, ApiResult};
use crate::legacy::users::{BasicUser, active_emails, email_of};
use crate::state::AppState;
use axum::http::StatusCode;
use serde::Serialize;

/// Contact states (museum's `ente.ContactState`).
pub mod state {
    /// Owner invited the contact.
    pub const INVITED: &str = "INVITED";
    /// Owner revoked the contact.
    pub const REVOKED: &str = "REVOKED";
    /// Contact accepted.
    pub const ACCEPTED: &str = "ACCEPTED";
    /// Contact left.
    pub const CONTACT_LEFT: &str = "CONTACT_LEFT";
    /// Contact declined the invite.
    pub const CONTACT_DENIED: &str = "CONTACT_DENIED";
}

/// Recovery statuses (museum's `ente.RecoveryStatus`).
pub mod status {
    /// Only used for notifications.
    pub const INITIATED: &str = "INITIATED";
    /// Notice period running.
    pub const WAITING: &str = "WAITING";
    /// Owner rejected.
    pub const REJECTED: &str = "REJECTED";
    /// Password was reset.
    pub const RECOVERED: &str = "RECOVERED";
    /// Contact stopped it.
    pub const STOPPED: &str = "STOPPED";
    /// Recovery may proceed.
    pub const READY: &str = "READY";
}

const MICROS_PER_HOUR: i64 = 3_600_000_000;
const MICROS_PER_DAY: i64 = 24 * MICROS_PER_HOUR;

fn forbidden() -> ApiError {
    ApiError::status(StatusCode::FORBIDDEN)
}

fn not_found() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND)
}

/// A row of `emergency_contact`.
#[derive(Debug, Clone)]
pub struct ContactRow {
    /// Account owner.
    pub user_id: i64,
    /// Trusted contact.
    pub emergency_contact_id: i64,
    /// Contact state.
    pub state: String,
    /// Notice period in hours.
    pub notice_period_in_hrs: i64,
    /// Owner's recovery key encrypted for the contact.
    pub encrypted_key: Option<String>,
}

type ContactTuple = (i64, i64, String, i64, Option<String>);

impl From<ContactTuple> for ContactRow {
    fn from(t: ContactTuple) -> Self {
        Self {
            user_id: t.0,
            emergency_contact_id: t.1,
            state: t.2,
            notice_period_in_hrs: t.3,
            encrypted_key: t.4,
        }
    }
}

/// A row of `emergency_recovery`.
#[derive(Debug, Clone)]
pub struct RecoverRow {
    /// Session id (UUID).
    pub id: String,
    /// Account owner.
    pub user_id: i64,
    /// Trusted contact.
    pub emergency_contact_id: i64,
    /// Status.
    pub status: String,
    /// When the notice period ends (µs).
    pub wait_till: i64,
    /// Next reminder email (µs).
    pub next_reminder_at: i64,
    /// Creation time (µs).
    pub created_at: i64,
}

type RecoverTuple = (String, i64, i64, String, Option<i64>, Option<i64>, i64);

impl From<RecoverTuple> for RecoverRow {
    fn from(t: RecoverTuple) -> Self {
        Self {
            id: t.0,
            user_id: t.1,
            emergency_contact_id: t.2,
            status: t.3,
            wait_till: t.4.unwrap_or(0),
            next_reminder_at: t.5.unwrap_or(0),
            created_at: t.6,
        }
    }
}

impl RecoverRow {
    /// museum's `RecoverRow.CanRecover`.
    pub fn can_recover(&self) -> Result<(), &'static str> {
        if self.status != status::READY && self.status != status::WAITING {
            return Err("recovery status is not waiting or ready");
        }
        if self.wait_till > now_micros() && self.status == status::WAITING {
            return Err("recovery wait time is not over");
        }
        Ok(())
    }
}

const CONTACT_COLS: &str =
    "user_id, emergency_contact_id, state, notice_period_in_hrs, encrypted_key";
const RECOVER_COLS: &str =
    "id, user_id, emergency_contact_id, status, wait_till, next_reminder_at, created_at";

/// `EmergencyContactEntity`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContactEntity {
    /// Owner.
    pub user: BasicUser,
    /// Trusted contact.
    pub emergency_contact: BasicUser,
    /// State.
    pub state: String,
    /// Notice period in days.
    pub recovery_notice_in_days: i64,
}

/// `RecoverySession`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoverySession {
    /// Session id.
    pub id: String,
    /// Owner.
    pub user: BasicUser,
    /// Trusted contact.
    pub emergency_contact: BasicUser,
    /// Status (`WAITING` turns into `READY` once the wait is over).
    pub status: String,
    /// Remaining wait in microseconds (not a timestamp).
    pub wait_till: i64,
    /// Creation time (µs).
    pub created_at: i64,
}

/// `EmergencyDataResponse`.
#[derive(Debug, Serialize)]
pub struct InfoResponse {
    /// The caller's trusted contacts.
    pub contacts: Vec<ContactEntity>,
    /// Recovery sessions on the caller's account.
    #[serde(rename = "recoverSessions")]
    pub recover_sessions: Vec<RecoverySession>,
    /// Accounts the caller is a trusted contact for.
    #[serde(rename = "othersEmergencyContact")]
    pub others_emergency_contact: Vec<ContactEntity>,
    /// Recovery sessions the caller started for others.
    #[serde(rename = "othersRecoverySession")]
    pub others_recovery_session: Vec<RecoverySession>,
}

/// Add (or re-invite) a trusted contact. Returns whether a row changed.
pub async fn add_contact(
    st: &AppState,
    user_id: i64,
    contact_id: i64,
    encrypted_key: &str,
    notice_hrs: i64,
) -> ApiResult<bool> {
    if user_id == contact_id {
        return Err(ApiError::bad_request(
            "user cannot add themself as emergency contact",
        ));
    }
    let now = now_micros();
    let r = sqlx::query(
        "INSERT INTO emergency_contact(user_id, emergency_contact_id, state, encrypted_key, notice_period_in_hrs, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $6) \
         ON CONFLICT (user_id, emergency_contact_id) DO UPDATE SET state = $3, encrypted_key = $4, notice_period_in_hrs = $5, updated_at = $6 \
         WHERE emergency_contact.state IN ('CONTACT_DENIED', 'CONTACT_LEFT', 'REVOKED')",
    )
    .bind(user_id)
    .bind(contact_id)
    .bind(state::INVITED)
    .bind(encrypted_key)
    .bind(notice_hrs)
    .bind(now)
    .execute(&st.db.pool)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Contacts (INVITED/ACCEPTED) where the user is owner or trusted contact.
pub async fn active_contacts_for_user(st: &AppState, user_id: i64) -> ApiResult<Vec<ContactRow>> {
    let rows: Vec<ContactTuple> = sqlx::query_as(&format!(
        "SELECT {CONTACT_COLS} FROM emergency_contact WHERE (user_id = $1 OR emergency_contact_id = $1) AND state IN ('ACCEPTED', 'INVITED')"
    ))
    .bind(user_id)
    .fetch_all(&st.db.pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

async fn active_recovery_sessions(st: &AppState, user_id: i64) -> ApiResult<Vec<RecoverRow>> {
    let rows: Vec<RecoverTuple> = sqlx::query_as(&format!(
        "SELECT {RECOVER_COLS} FROM emergency_recovery WHERE (user_id = $1 OR emergency_contact_id = $1) AND status IN ('WAITING', 'READY') ORDER BY created_at"
    ))
    .bind(user_id)
    .fetch_all(&st.db.pool)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// `GET /emergency-contacts/info`.
pub async fn info(st: &AppState, user_id: i64) -> ApiResult<InfoResponse> {
    let contacts = active_contacts_for_user(st, user_id).await?;
    let sessions = active_recovery_sessions(st, user_id).await?;
    let mut ids = Vec::new();
    for c in &contacts {
        ids.push(c.user_id);
        ids.push(c.emergency_contact_id);
    }
    for s in &sessions {
        ids.push(s.user_id);
        ids.push(s.emergency_contact_id);
    }
    let emails = active_emails(st, &ids).await?;
    let basic = |id: i64| {
        emails.get(&id).map(|e| BasicUser {
            id,
            email: e.clone(),
        })
    };
    let mut out = InfoResponse {
        contacts: vec![],
        recover_sessions: vec![],
        others_emergency_contact: vec![],
        others_recovery_session: vec![],
    };
    for c in contacts {
        let (Some(user), Some(ec)) = (basic(c.user_id), basic(c.emergency_contact_id)) else {
            continue;
        };
        let e = ContactEntity {
            user,
            emergency_contact: ec,
            state: c.state,
            recovery_notice_in_days: c.notice_period_in_hrs / 24,
        };
        if c.user_id == user_id {
            out.contacts.push(e);
        } else {
            out.others_emergency_contact.push(e);
        }
    }
    let now = now_micros();
    for s in sessions {
        let (Some(user), Some(ec)) = (basic(s.user_id), basic(s.emergency_contact_id)) else {
            continue;
        };
        let mut wait = s.wait_till - now;
        let mut status = s.status;
        if wait < 0 {
            if status == status::WAITING {
                status = status::READY.to_string();
            }
            wait = 0;
        }
        let e = RecoverySession {
            id: s.id,
            user,
            emergency_contact: ec,
            status,
            wait_till: wait,
            created_at: s.created_at,
        };
        if s.user_id == user_id {
            out.recover_sessions.push(e);
        } else {
            out.others_recovery_session.push(e);
        }
    }
    Ok(out)
}

fn valid_previous_states(new_state: &str) -> &'static str {
    match new_state {
        state::INVITED => "('REVOKED', 'CONTACT_LEFT', 'CONTACT_DENIED')",
        state::ACCEPTED => "('INVITED', 'ACCEPTED')",
        state::CONTACT_LEFT => "('INVITED', 'ACCEPTED')",
        state::CONTACT_DENIED => "('INVITED')",
        _ => "('INVITED', 'ACCEPTED')", // REVOKED
    }
}

/// museum's `validateUpdateReq`.
pub fn validate_update(
    actor: i64,
    user_id: i64,
    contact_id: i64,
    new_state: &str,
) -> ApiResult<()> {
    if contact_id == user_id {
        return Err(ApiError::bad_request("contact and user can not be same"));
    }
    if contact_id != actor && user_id != actor {
        return Err(forbidden());
    }
    let ok = if actor == contact_id {
        matches!(
            new_state,
            state::ACCEPTED | state::CONTACT_LEFT | state::CONTACT_DENIED
        )
    } else {
        matches!(new_state, state::INVITED | state::REVOKED)
    };
    if ok {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "Can not update state to {new_state}"
        )))
    }
}

/// museum's `UpdateContact` (validation, state change, cancelling active
/// recoveries, notifications). Unchanged rows are not an error.
pub async fn update_contact(
    st: &AppState,
    actor: i64,
    user_id: i64,
    contact_id: i64,
    new_state: &str,
) -> ApiResult<()> {
    validate_update(actor, user_id, contact_id, new_state)?;
    let db = &st.db;
    let now = now_micros();
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, user_id).await? {
        return Err(not_found());
    }
    let prev = valid_previous_states(new_state);
    let sql = if new_state == state::ACCEPTED || new_state == state::INVITED {
        format!(
            "UPDATE emergency_contact SET state = $1, updated_at = $2 WHERE user_id = $3 AND emergency_contact_id = $4 AND state IN {prev}"
        )
    } else {
        format!(
            "UPDATE emergency_contact SET state = $1, encrypted_key = NULL, updated_at = $2 WHERE user_id = $3 AND emergency_contact_id = $4 AND state IN {prev}"
        )
    };
    let r = sqlx::query(&sql)
        .bind(new_state)
        .bind(now)
        .bind(user_id)
        .bind(contact_id)
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() == 0 {
        tracing::warn!(
            actor,
            user_id,
            contact_id,
            new_state,
            "no update applied for emergency contact"
        );
        return Ok(());
    }
    let mut cancelled = 0;
    let cancel_status = if new_state == state::REVOKED {
        status::REJECTED
    } else {
        status::STOPPED
    };
    if matches!(
        new_state,
        state::CONTACT_DENIED | state::CONTACT_LEFT | state::REVOKED
    ) {
        let r = sqlx::query(
            "UPDATE emergency_recovery SET status = $1, updated_at = $2 WHERE user_id = $3 AND emergency_contact_id = $4 AND status IN ('WAITING', 'READY')",
        )
        .bind(cancel_status)
        .bind(now)
        .bind(user_id)
        .bind(contact_id)
        .execute(&mut *tx)
        .await?;
        cancelled = r.rows_affected();
    }
    tx.commit().await?;
    for _ in 0..cancelled {
        notify_recovery(st, user_id, contact_id, cancel_status, None);
    }
    notify_contact(st, user_id, contact_id, new_state);
    Ok(())
}

/// `POST /emergency-contacts/update-recovery-notice` (1..=60 days).
pub async fn update_recovery_notice(
    st: &AppState,
    user_id: i64,
    contact_id: i64,
    days: i64,
) -> ApiResult<()> {
    if !(1..=60).contains(&days) {
        return Err(ApiError::bad_request(
            "recovery notice must be between 1 and 60 days",
        ));
    }
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, user_id).await? {
        return Err(not_found());
    }
    let contact = contact_for_update(&mut tx, db.engine, user_id, contact_id).await?;
    if has_active_recovery(&mut tx, user_id, contact_id).await? {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "ACTIVE_RECOVERY_SESSION",
            "cannot update recovery notice while there is an active recovery session",
        ));
    }
    match contact {
        Some(c) if c.state == state::INVITED || c.state == state::ACCEPTED => {}
        _ => {
            return Err(ApiError::bad_request(
                "emergency contact not found or not in valid state",
            ));
        }
    }
    sqlx::query(
        "UPDATE emergency_contact SET notice_period_in_hrs = $1, updated_at = $2 WHERE user_id = $3 AND emergency_contact_id = $4",
    )
    .bind(days * 24)
    .bind(now_micros())
    .bind(user_id)
    .bind(contact_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn contact_for_update(
    conn: &mut sqlx::AnyConnection,
    engine: Engine,
    user_id: i64,
    contact_id: i64,
) -> ApiResult<Option<ContactRow>> {
    let row: Option<ContactTuple> = sqlx::query_as(&format!(
        "SELECT {CONTACT_COLS} FROM emergency_contact WHERE user_id = $1 AND emergency_contact_id = $2{}",
        for_update(engine)
    ))
    .bind(user_id)
    .bind(contact_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.map(Into::into))
}

async fn has_active_recovery(
    conn: &mut sqlx::AnyConnection,
    user_id: i64,
    contact_id: i64,
) -> ApiResult<bool> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM emergency_recovery WHERE user_id = $1 AND emergency_contact_id = $2 AND status IN ('WAITING', 'READY')",
    )
    .bind(user_id)
    .bind(contact_id)
    .fetch_one(&mut *conn)
    .await?;
    Ok(n > 0)
}

/// `POST /emergency-contacts/start-recovery` (actor must be the trusted contact).
pub async fn start_recovery(
    st: &AppState,
    actor: i64,
    user_id: i64,
    contact_id: i64,
) -> ApiResult<()> {
    if contact_id == user_id {
        return Err(ApiError::bad_request("contact and user can not be same"));
    }
    if contact_id != actor {
        return Err(forbidden());
    }
    let db = &st.db;
    let mut tx = begin_write(db).await?;
    if !lock_user(&mut tx, db.engine, user_id).await? {
        return Err(not_found());
    }
    let contact = contact_for_update(&mut tx, db.engine, user_id, contact_id)
        .await?
        .ok_or_else(not_found)?;
    if contact.state != state::ACCEPTED {
        return Err(not_found());
    }
    if contact.notice_period_in_hrs <= 24 {
        return Err(ApiError::bad_request(
            "notice period should be greater than 24 hours",
        ));
    }
    if has_active_recovery(&mut tx, user_id, contact_id).await? {
        tracing::warn!(actor, user_id, "recovery already active; no email sent");
        return Ok(());
    }
    let now = now_micros();
    let wait_till = now + contact.notice_period_in_hrs * MICROS_PER_HOUR;
    let mut next_reminder = now + 7 * MICROS_PER_DAY;
    if next_reminder >= wait_till {
        next_reminder = now + MICROS_PER_DAY;
    }
    sqlx::query(
        "INSERT INTO emergency_recovery(id, user_id, emergency_contact_id, status, wait_till, next_reminder_at, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $7)",
    )
    .bind(new_uuid())
    .bind(user_id)
    .bind(contact_id)
    .bind(status::WAITING)
    .bind(wait_till)
    .bind(next_reminder)
    .bind(now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    notify_recovery(
        st,
        user_id,
        contact_id,
        status::INITIATED,
        Some(contact.notice_period_in_hrs / 24),
    );
    Ok(())
}

/// A recovery session by id.
pub async fn recovery_by_id(st: &AppState, id: &str) -> ApiResult<RecoverRow> {
    let row: Option<RecoverTuple> = sqlx::query_as(&format!(
        "SELECT {RECOVER_COLS} FROM emergency_recovery WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(&st.db.pool)
    .await?;
    row.map(Into::into).ok_or_else(not_found)
}

async fn matching_session(
    st: &AppState,
    id: &str,
    user_id: i64,
    contact_id: i64,
) -> ApiResult<RecoverRow> {
    let s = recovery_by_id(st, id).await?;
    if s.user_id != user_id || s.emergency_contact_id != contact_id {
        return Err(forbidden());
    }
    Ok(s)
}

async fn set_session_status(st: &AppState, s: &RecoverRow, new_status: &str) -> ApiResult<bool> {
    let now = now_micros();
    let r = if new_status == status::READY {
        sqlx::query(
            "UPDATE emergency_recovery SET status = $1, wait_till = $2, updated_at = $2 WHERE id = $3 AND user_id = $4 AND emergency_contact_id = $5 AND status IN ('WAITING', 'READY')",
        )
        .bind(new_status)
        .bind(now)
        .bind(&s.id)
        .bind(s.user_id)
        .bind(s.emergency_contact_id)
        .execute(&st.db.pool)
        .await?
    } else {
        sqlx::query(
            "UPDATE emergency_recovery SET status = $1, updated_at = $2 WHERE id = $3 AND user_id = $4 AND emergency_contact_id = $5 AND status IN ('WAITING', 'READY')",
        )
        .bind(new_status)
        .bind(now)
        .bind(&s.id)
        .bind(s.user_id)
        .bind(s.emergency_contact_id)
        .execute(&st.db.pool)
        .await?
    };
    Ok(r.rows_affected() > 0)
}

/// `POST /emergency-contacts/reject-recovery` (owner).
pub async fn reject_recovery(
    st: &AppState,
    actor: i64,
    id: &str,
    user_id: i64,
    contact_id: i64,
) -> ApiResult<()> {
    if contact_id == user_id {
        return Err(ApiError::bad_request("contact and user can not be same"));
    }
    if user_id != actor {
        return Err(forbidden());
    }
    let s = matching_session(st, id, user_id, contact_id).await?;
    if s.user_id != actor {
        return Err(forbidden());
    }
    if !set_session_status(st, &s, status::REJECTED).await? {
        return Err(ApiError::conflict("recovery session is no longer active"));
    }
    notify_recovery(
        st,
        s.user_id,
        s.emergency_contact_id,
        status::REJECTED,
        None,
    );
    Ok(())
}

/// `POST /emergency-contacts/approve-recovery` (owner skips the notice period).
pub async fn approve_recovery(
    st: &AppState,
    actor: i64,
    id: &str,
    user_id: i64,
    contact_id: i64,
) -> ApiResult<()> {
    if contact_id == user_id {
        return Err(ApiError::bad_request("contact and user can not be same"));
    }
    let s = matching_session(st, id, user_id, contact_id).await?;
    if s.user_id != actor {
        return Err(forbidden());
    }
    if set_session_status(st, &s, status::READY).await? {
        notify_recovery(st, s.user_id, s.emergency_contact_id, status::READY, None);
    } else {
        tracing::warn!(actor, "no row updated while approving recovery");
    }
    Ok(())
}

/// `POST /emergency-contacts/stop-recovery` (trusted contact).
pub async fn stop_recovery(
    st: &AppState,
    actor: i64,
    id: &str,
    user_id: i64,
    contact_id: i64,
) -> ApiResult<()> {
    if contact_id == user_id {
        return Err(ApiError::bad_request("contact and user can not be same"));
    }
    if contact_id != actor {
        return Err(forbidden());
    }
    let s = matching_session(st, id, user_id, contact_id).await?;
    if s.emergency_contact_id != actor {
        return Err(forbidden());
    }
    if !set_session_status(st, &s, status::STOPPED).await? {
        return Err(ApiError::conflict("recovery session is no longer active"));
    }
    notify_recovery(st, s.user_id, s.emergency_contact_id, status::STOPPED, None);
    Ok(())
}

/// museum's `checkRecoveryAndGetContact`: the caller is the trusted contact of a
/// session that can recover now; returns the active contact row.
pub async fn check_recovery(st: &AppState, actor: i64, id: &str) -> ApiResult<ContactRow> {
    let s = recovery_by_id(st, id).await?;
    if s.emergency_contact_id != actor {
        return Err(forbidden());
    }
    s.can_recover().map_err(ApiError::bad_request)?;
    let row: Option<ContactTuple> = sqlx::query_as(&format!(
        "SELECT {CONTACT_COLS} FROM emergency_contact WHERE user_id = $1 AND emergency_contact_id = $2 AND state = 'ACCEPTED'"
    ))
    .bind(s.user_id)
    .bind(s.emergency_contact_id)
    .fetch_optional(&st.db.pool)
    .await?;
    let c: ContactRow = row.map(Into::into).ok_or_else(not_found)?;
    if c.encrypted_key.is_none() {
        return Err(not_found());
    }
    Ok(c)
}

/// museum's `Repo.CompleteRecovery`, inside the password-update transaction.
pub async fn complete_recovery_tx(
    conn: &mut sqlx::AnyConnection,
    engine: Engine,
    session_id: &str,
    user_id: i64,
    contact_id: i64,
) -> ApiResult<()> {
    let c = contact_for_update(conn, engine, user_id, contact_id)
        .await?
        .ok_or_else(not_found)?;
    if c.state != state::ACCEPTED || c.encrypted_key.is_none() {
        return Err(not_found());
    }
    let row: Option<RecoverTuple> = sqlx::query_as(&format!(
        "SELECT {RECOVER_COLS} FROM emergency_recovery WHERE id = $1{}",
        for_update(engine)
    ))
    .bind(session_id)
    .fetch_optional(&mut *conn)
    .await?;
    let s: RecoverRow = row.map(Into::into).ok_or_else(not_found)?;
    if s.user_id != user_id || s.emergency_contact_id != contact_id {
        return Err(forbidden());
    }
    s.can_recover().map_err(ApiError::bad_request)?;
    sqlx::query("UPDATE emergency_recovery SET status = $1, updated_at = $2 WHERE id = $3")
        .bind(status::RECOVERED)
        .bind(now_micros())
        .bind(session_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Send the "invited" notifications after [`add_contact`] changed a row.
pub fn update_notification_invited(st: &AppState, user_id: i64, contact_id: i64) {
    notify_contact(st, user_id, contact_id, state::INVITED);
}

/// Send the "recovered" notifications after a successful password reset.
pub fn notify_recovered(st: &AppState, user_id: i64, contact_id: i64) {
    notify_recovery(st, user_id, contact_id, status::RECOVERED, None);
}

/// museum's `HandleAccountDeletion`: revoke the user's trusted contacts and
/// leave the accounts they are a trusted contact for. For the users workstream
/// to call when an account is deleted.
pub async fn handle_account_deletion(st: &AppState, user_id: i64) -> ApiResult<()> {
    for c in active_contacts_for_user(st, user_id).await? {
        if c.user_id == user_id {
            update_contact(st, user_id, user_id, c.emergency_contact_id, state::REVOKED).await?;
        } else {
            update_contact(st, user_id, c.user_id, user_id, state::CONTACT_LEFT).await?;
        }
    }
    Ok(())
}

/// museum's `SendRecoveryReminder` (hourly): flip finished notice periods to
/// READY and send reminder emails.
pub async fn run_reminders_once(st: &AppState) -> ApiResult<()> {
    let now = now_micros();
    let rows: Vec<RecoverTuple> = sqlx::query_as(&format!(
        "SELECT {RECOVER_COLS} FROM emergency_recovery WHERE status = 'WAITING' AND next_reminder_at < $1"
    ))
    .bind(now)
    .fetch_all(&st.db.pool)
    .await?;
    for row in rows.into_iter().map(RecoverRow::from) {
        let days_left = (row.wait_till - row.next_reminder_at) / MICROS_PER_DAY;
        if row.wait_till < now_micros() {
            let r = sqlx::query(
                "UPDATE emergency_recovery SET status = 'READY', wait_till = $1, updated_at = $1 WHERE id = $2 AND status IN ('WAITING', 'READY')",
            )
            .bind(now_micros())
            .bind(&row.id)
            .execute(&st.db.pool)
            .await;
            match r {
                Ok(_) => notify_recovery(
                    st,
                    row.user_id,
                    row.emergency_contact_id,
                    status::READY,
                    None,
                ),
                Err(e) => {
                    tracing::error!(error = %e, session = %row.id, "failed to update recovery status")
                }
            }
        } else if days_left >= 2 {
            let next = if days_left > 9 {
                row.next_reminder_at + 7 * MICROS_PER_DAY
            } else if days_left > 2 {
                row.wait_till - 2 * MICROS_PER_DAY
            } else {
                row.wait_till
            };
            if let Err(e) =
                sqlx::query("UPDATE emergency_recovery SET next_reminder_at = $1 WHERE id = $2")
                    .bind(next)
                    .bind(&row.id)
                    .execute(&st.db.pool)
                    .await
            {
                tracing::error!(error = %e, session = %row.id, "failed to update next reminder");
                continue;
            }
            notify_recovery(
                st,
                row.user_id,
                row.emergency_contact_id,
                status::WAITING,
                Some(days_left),
            );
        }
    }
    Ok(())
}

// ---- notifications ---------------------------------------------------------

struct Mail {
    to: String,
    subject: &'static str,
    body: String,
}

fn contact_mails(legacy: &str, trusted: &str, new_state: &str) -> Vec<Mail> {
    let m = |to: &str, subject: &'static str, body: String| Mail {
        to: to.to_string(),
        subject,
        body,
    };
    match new_state {
        state::INVITED => vec![
            m(
                trusted,
                "You have been added as a trusted contact",
                format!(
                    "{legacy} has added you as a trusted contact. Open the app to accept the invite; you will then be able to recover their account if they ever lose access."
                ),
            ),
            m(
                legacy,
                "Trusted contact invited",
                format!(
                    "You have invited {trusted} as a trusted contact. They will be able to help recover your account once they accept."
                ),
            ),
        ],
        state::REVOKED => vec![m(
            trusted,
            "Legacy account access removed",
            format!("{legacy} has removed you as a trusted contact."),
        )],
        state::CONTACT_LEFT => vec![m(
            legacy,
            "Trusted contact removed",
            format!("{trusted} is no longer your trusted contact."),
        )],
        state::CONTACT_DENIED => vec![m(
            legacy,
            "Legacy invite rejected",
            format!("{trusted} has declined your invite to be a trusted contact."),
        )],
        state::ACCEPTED => vec![m(
            legacy,
            "Trusted contact added",
            format!("{trusted} has accepted your invite and is now your trusted contact."),
        )],
        _ => vec![],
    }
}

fn recovery_mails(
    legacy: &str,
    trusted: &str,
    new_status: &str,
    days_left: Option<i64>,
) -> Vec<Mail> {
    let m = |to: &str, subject: &'static str, body: String| Mail {
        to: to.to_string(),
        subject,
        body,
    };
    match new_status {
        status::INITIATED => {
            let days = days_left.unwrap_or(30);
            vec![m(
                legacy,
                "Account recovery initiated",
                format!(
                    "{trusted} has started recovering your account. If you do nothing, they will be able to reset your password in {days} days. If this was not expected, open the app and reject the recovery."
                ),
            )]
        }
        status::RECOVERED => vec![
            m(
                legacy,
                "Account password reset",
                format!("{trusted} has recovered your account and reset its password."),
            ),
            m(
                trusted,
                "Account recovery successful",
                format!("You have recovered the account of {legacy}."),
            ),
        ],
        status::STOPPED => vec![m(
            legacy,
            "Account recovery cancelled",
            format!("{trusted} has cancelled the recovery of your account."),
        )],
        status::REJECTED => vec![m(
            trusted,
            "Account recovery blocked",
            format!("{legacy} has blocked your request to recover their account."),
        )],
        status::WAITING => {
            let days = days_left.unwrap_or(0);
            vec![m(
                legacy,
                "Account recovery due",
                format!(
                    "{trusted} is recovering your account. They will be able to reset your password in {days} days unless you reject the recovery in the app."
                ),
            )]
        }
        status::READY => vec![
            m(
                trusted,
                "Account recoverable",
                format!("You can now recover the account of {legacy}."),
            ),
            m(
                legacy,
                "Account recoverable",
                format!("{trusted} can now recover your account."),
            ),
        ],
        _ => vec![],
    }
}

fn send_all(
    st: &AppState,
    legacy_id: i64,
    trusted_id: i64,
    build: impl FnOnce(&str, &str) -> Vec<Mail> + Send + 'static,
) {
    let st = st.clone();
    tokio::spawn(async move {
        let (Ok(Some(legacy)), Ok(Some(trusted))) = (
            email_of(&st, legacy_id).await,
            email_of(&st, trusted_id).await,
        ) else {
            tracing::warn!(
                legacy_id,
                trusted_id,
                "legacy notification skipped: user not found"
            );
            return;
        };
        for mail in build(&legacy, &trusted) {
            if let Err(e) = crate::mail::send(&st.config, &mail.to, mail.subject, &mail.body).await
            {
                tracing::error!(error = %e, subject = mail.subject, "failed to send legacy email");
                return;
            }
        }
    });
}

fn notify_contact(st: &AppState, legacy_id: i64, trusted_id: i64, new_state: &str) {
    let s = new_state.to_string();
    send_all(st, legacy_id, trusted_id, move |l, t| {
        contact_mails(l, t, &s)
    });
}

fn notify_recovery(
    st: &AppState,
    legacy_id: i64,
    trusted_id: i64,
    new_status: &str,
    days_left: Option<i64>,
) {
    let s = new_status.to_string();
    send_all(st, legacy_id, trusted_id, move |l, t| {
        recovery_mails(l, t, &s, days_left)
    });
}
