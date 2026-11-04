//! Family plans (museum `pkg/controller/family` + `pkg/repo/family.go`).

use super::*;
use crate::db::{Db, now_micros};
use serde::Serialize;

/// Museum `maxFamilyMemberLimit` (admin included).
pub const MAX_FAMILY_MEMBERS: usize = 6;

/// Family membership status (museum `ente.MemberStatus`).
pub mod status {
    /// The admin's own row.
    pub const SELF: &str = "SELF";
    /// Family closed.
    pub const CLOSED: &str = "CLOSED";
    /// Invite pending.
    pub const INVITED: &str = "INVITED";
    /// Active member.
    pub const ACCEPTED: &str = "ACCEPTED";
    /// Invite declined.
    pub const DECLINED: &str = "DECLINED";
    /// Invite revoked by the admin.
    pub const REVOKED: &str = "REVOKED";
    /// Member removed by the admin.
    pub const REMOVED: &str = "REMOVED";
    /// Member left.
    pub const LEFT: &str = "LEFT";
}

/// A `families` row (museum `ente.FamilyMember`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyMember {
    /// Membership id (UUID).
    pub id: String,
    /// Member email.
    pub email: String,
    /// Status.
    pub status: String,
    /// Optional storage cap for this member.
    pub storage_limit: Option<i64>,
    /// Member id, only for active members.
    #[serde(rename = "userID")]
    pub user_id: Option<i64>,
    /// Usage, only for active members.
    pub usage: i64,
    /// Whether this row is the admin.
    pub is_admin: bool,
    /// Member user id.
    #[serde(skip)]
    pub member_user_id: i64,
    /// Admin user id.
    #[serde(skip)]
    pub admin_user_id: i64,
}

/// Museum `ente.FamilyMemberResponse`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FamilyMemberResponse {
    /// Members (active and invited).
    pub members: Vec<FamilyMember>,
    /// Admin plan storage (excluding bonuses).
    pub storage: i64,
    /// Admin plan expiry.
    pub expiry_time: i64,
    /// Admin's usable bonus.
    pub admin_bonus: i64,
}

type MemberRow = (String, i64, i64, String, Option<i64>);

fn to_member(r: MemberRow) -> FamilyMember {
    let (id, admin, member, status, limit) = r;
    FamilyMember {
        id,
        email: String::new(),
        status,
        storage_limit: limit,
        user_id: None,
        usage: 0,
        is_admin: admin == member,
        member_user_id: member,
        admin_user_id: admin,
    }
}

const MEMBER_COLS: &str = "id, admin_id, member_id, status, storage_limit";

/// Members of `admin_id`'s family with one of `statuses`.
pub async fn members_with_status(
    db: &Db,
    admin_id: i64,
    statuses: &[&str],
) -> ApiResult<Vec<FamilyMember>> {
    let rows: Vec<MemberRow> = sqlx::query_as(&format!(
        "SELECT {MEMBER_COLS} FROM families WHERE admin_id = $1 ORDER BY created_at"
    ))
    .bind(admin_id)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(to_member)
        .filter(|m| statuses.contains(&m.status.as_str()))
        .collect())
}

/// Active (ACCEPTED or SELF) members as `(member_id, storage_limit)`.
pub async fn active_members(db: &Db, admin_id: i64) -> ApiResult<Vec<(i64, Option<i64>)>> {
    Ok(
        members_with_status(db, admin_id, &[status::ACCEPTED, status::SELF])
            .await?
            .into_iter()
            .map(|m| (m.member_user_id, m.storage_limit))
            .collect(),
    )
}

async fn member_by(db: &Db, col: &str, v: &str) -> ApiResult<FamilyMember> {
    let row: Option<MemberRow> = sqlx::query_as(&format!(
        "SELECT {MEMBER_COLS} FROM families WHERE {col} = $1"
    ))
    .bind(v)
    .fetch_optional(&db.pool)
    .await?;
    row.map(to_member).ok_or_else(not_found_plain)
}

/// Museum `IsUserOnPaidPlan`: the user (or their family admin) has a non-free product or a
/// paid add-on bonus.
pub async fn is_on_paid_plan(db: &Db, user_id: i64) -> ApiResult<bool> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT COUNT(*) FROM users u WHERE u.user_id = $1 AND ( \
           EXISTS (SELECT 1 FROM users s WHERE s.user_id = COALESCE(u.family_admin_id, u.user_id) AND s.subscription_product_id <> 'free') \
           OR EXISTS (SELECT 1 FROM storage_bonus sb WHERE sb.user_id = COALESCE(u.family_admin_id, u.user_id) AND sb.type NOT IN ('SIGN_UP', 'REFERRAL')))",
    )
    .bind(user_id)
    .fetch_optional(&db.pool)
    .await?;
    Ok(matches!(row, Some((n,)) if n > 0))
}

/// Museum `HasActiveSelfOrFamilySubscription(user, mustBeOnPaidPlan=true)`; any failure is
/// reported as 402 `{}` like `CreateFamily`/`InviteMember` do.
pub async fn require_paid(db: &Db, user_id: i64) -> ApiResult<()> {
    let fam: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT family_admin_id FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&db.pool)
            .await?;
    let sub_user = fam.and_then(|f| f.0).unwrap_or(user_id);
    let sub = subscription(db, sub_user).await?;
    if sub.expiry_time < now_micros() && active_bonus(db, sub_user).await?.max_expiry() <= 0 {
        return Err(no_active_subscription());
    }
    if !is_on_paid_plan(db, user_id).await? {
        return Err(no_active_subscription());
    }
    Ok(())
}

/// Museum `CreateFamily`.
pub async fn create_family(state: &AppState, admin_id: i64) -> ApiResult<()> {
    require_paid(&state.db, admin_id).await?;
    let admin = get_user(state, admin_id).await?;
    if let Some(f) = admin.family_admin_id {
        if f != admin_id {
            return Err(bad_request_plain());
        }
        return Ok(());
    }
    let now = now_micros();
    let mut tx = state.db.pool.begin().await?;
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM families WHERE admin_id = $1 AND member_id = $2")
            .bind(admin_id)
            .bind(admin_id)
            .fetch_optional(&mut *tx)
            .await?;
    if existing.is_some() {
        sqlx::query("UPDATE families SET status = $1, updated_at = $2 WHERE admin_id = $3 AND member_id = $4")
            .bind(status::SELF)
            .bind(now)
            .bind(admin_id)
            .bind(admin_id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("INSERT INTO families (id, admin_id, member_id, status, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6)")
            .bind(new_uuid())
            .bind(admin_id)
            .bind(admin_id)
            .bind(status::SELF)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await?;
    }
    let r = sqlx::query(
        "UPDATE users SET family_admin_id = $1 WHERE user_id = $2 AND family_admin_id IS NULL",
    )
    .bind(admin_id)
    .bind(admin_id)
    .execute(&mut *tx)
    .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::internal("exactly one row should be updated"));
    }
    tx.commit().await?;
    Ok(())
}

/// Museum `InviteMember`. Sends the invite email (logged when SMTP is off).
pub async fn invite_member(
    state: &AppState,
    admin_id: i64,
    email: &str,
    storage_limit: Option<i64>,
) -> ApiResult<()> {
    require_paid(&state.db, admin_id).await?;
    let admin = get_user(state, admin_id).await?;
    match admin.family_admin_id {
        None => return Err(bad_request_plain()),
        Some(f) if f != admin_id => return Err(bad_request_plain()),
        _ => {}
    }
    let members = members_with_status(
        &state.db,
        admin_id,
        &[status::INVITED, status::ACCEPTED, status::SELF],
    )
    .await?;
    let Some(member_id) = user_id_for_email(state, email).await? else {
        if members.len() >= MAX_FAMILY_MEMBERS {
            return Err(family_size_limit());
        }
        return Err(not_found_plain());
    };
    if member_id == admin_id {
        return Err(already_in_family());
    }
    if members.len() >= MAX_FAMILY_MEMBERS
        && !members
            .iter()
            .any(|m| m.member_user_id == member_id && m.status == status::INVITED)
    {
        return Err(family_size_limit());
    }
    let member = get_user(state, member_id).await?;
    if member.family_admin_id.is_some() {
        return Err(already_in_family());
    }
    let sub = subscription(&state.db, member_id).await?;
    if sub.is_active_paid() && !sub.attributes.is_cancelled {
        return Err(conflict_plain());
    }
    let token = random_url_safe(32);
    let now = now_micros();
    let existing: Option<(String, String)> =
        sqlx::query_as("SELECT id, status FROM families WHERE admin_id = $1 AND member_id = $2")
            .bind(admin_id)
            .bind(member_id)
            .fetch_optional(&state.db.pool)
            .await?;
    match existing {
        None => {
            sqlx::query("INSERT INTO families (id, admin_id, member_id, status, token, storage_limit, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)")
                .bind(new_uuid())
                .bind(admin_id)
                .bind(member_id)
                .bind(status::INVITED)
                .bind(&token)
                .bind(storage_limit)
                .bind(now)
                .bind(now)
                .execute(&state.db.pool)
                .await?;
        }
        Some((id, st)) if st != status::INVITED && st != status::ACCEPTED => {
            sqlx::query(
                "UPDATE families SET status = $1, token = $2, updated_at = $3 WHERE id = $4",
            )
            .bind(status::INVITED)
            .bind(&token)
            .bind(now)
            .bind(id)
            .execute(&state.db.pool)
            .await?;
        }
        Some(_) => {}
    }
    let active: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT token FROM families WHERE admin_id = $1 AND member_id = $2 AND status = $3",
    )
    .bind(admin_id)
    .bind(member_id)
    .bind(status::INVITED)
    .fetch_optional(&state.db.pool)
    .await?;
    let token = active.and_then(|t| t.0).ok_or_else(not_found_plain)?;
    let accounts = state.config.accounts.accounts_url.trim_end_matches('/');
    let link = format!(
        "{accounts}/family#inviteToken={}",
        url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>()
    );
    let body = format!(
        "{} has invited you to join their family plan.\n\nAccept the invite: {link}\n",
        admin.email
    );
    crate::mail::deliver(
        &state.config,
        &member.email,
        "You've been invited to join a family plan",
        &body,
    )
    .await;
    Ok(())
}

/// Museum `FetchMembersForAdminID`.
pub async fn fetch_members_for_admin(
    state: &AppState,
    admin_id: i64,
) -> ApiResult<FamilyMemberResponse> {
    let mut members = members_with_status(
        &state.db,
        admin_id,
        &[status::INVITED, status::ACCEPTED, status::SELF],
    )
    .await?;
    if members.is_empty() {
        return Err(ApiError::internal("member count can not be zero"));
    }
    let mut admin_storage = 0;
    let mut admin_expiry = 0;
    for m in members.iter_mut() {
        let active = m.status == status::ACCEPTED || m.status == status::SELF;
        if active {
            m.user_id = Some(m.member_user_id);
            m.usage = usage(&state.db, m.member_user_id).await?;
        }
        if let Ok(u) = get_user(state, m.member_user_id).await {
            m.email = u.email;
        }
        if m.is_admin {
            let sub = subscription(&state.db, m.member_user_id).await?;
            admin_storage = crate::capacity::effective_limit(state, sub.storage).await?;
            admin_expiry = sub.expiry_time;
        }
    }
    let bonus = active_bonus(&state.db, admin_id).await?;
    let admin_bonus = if admin_expiry < now_micros() {
        bonus.usable_bonus(0)
    } else {
        bonus.usable_bonus(admin_storage)
    };
    Ok(FamilyMemberResponse {
        members,
        storage: admin_storage,
        expiry_time: admin_expiry,
        admin_bonus,
    })
}

/// Museum `FetchMembers` (400 `{}` when not in a family).
pub async fn fetch_members(state: &AppState, user_id: i64) -> ApiResult<FamilyMemberResponse> {
    let u = get_user(state, user_id).await?;
    let admin = u.family_admin_id.ok_or_else(bad_request_plain)?;
    fetch_members_for_admin(state, admin).await
}

async fn remove_member_row(db: &Db, admin_id: i64, member_id: i64, reason: &str) -> ApiResult<()> {
    let mut tx = db.pool.begin().await?;
    let r = sqlx::query("UPDATE families SET status = $1, updated_at = $2 WHERE admin_id = $3 AND member_id = $4 AND status = $5")
        .bind(reason)
        .bind(now_micros())
        .bind(admin_id)
        .bind(member_id)
        .bind(status::ACCEPTED)
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::internal("exactly one row should be updated"));
    }
    sqlx::query(
        "UPDATE users SET family_admin_id = NULL WHERE user_id = $1 AND family_admin_id = $2",
    )
    .bind(member_id)
    .bind(admin_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn notify(state: &AppState, to_user: i64, subject: &str, body: &str) {
    if let Ok(Some(email)) = email_for_user(state, to_user).await {
        crate::mail::deliver(&state.config, &email, subject, body).await;
    }
}

/// Museum `RemoveMember`.
pub async fn remove_member(state: &AppState, admin_id: i64, id: &str) -> ApiResult<()> {
    let m = member_by(&state.db, "id", id).await?;
    if m.admin_user_id != admin_id {
        return Err(permission_denied());
    }
    if m.status == status::REMOVED {
        return Ok(());
    }
    if m.status != status::ACCEPTED {
        return Err(bad_request_plain());
    }
    remove_member_row(&state.db, admin_id, m.member_user_id, status::REMOVED).await?;
    notify(
        state,
        m.member_user_id,
        "You have been removed from the family plan",
        "You have been removed from the family plan. Your account now uses its own plan.\n",
    )
    .await;
    Ok(())
}

/// Museum `RevokeInvite`.
pub async fn revoke_invite(state: &AppState, admin_id: i64, id: &str) -> ApiResult<()> {
    let m = member_by(&state.db, "id", id).await?;
    if m.admin_user_id != admin_id {
        return Err(permission_denied());
    }
    if m.status == status::REVOKED {
        return Ok(());
    }
    if m.status != status::INVITED {
        return Err(bad_request_plain());
    }
    sqlx::query("UPDATE families SET status = $1, updated_at = $2 WHERE admin_id = $3 AND member_id = $4 AND status = $5")
        .bind(status::REVOKED)
        .bind(now_micros())
        .bind(admin_id)
        .bind(m.member_user_id)
        .bind(status::INVITED)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

/// Museum `LeaveFamily`.
pub async fn leave_family(state: &AppState, user_id: i64) -> ApiResult<()> {
    let u = get_user(state, user_id).await?;
    let Some(admin) = u.family_admin_id else {
        return Ok(());
    };
    if admin == user_id {
        return Err(permission_denied());
    }
    remove_member_row(&state.db, admin, user_id, status::LEFT).await?;
    notify(
        state,
        admin,
        &format!("{} has left your family plan", u.email),
        &format!("{} has left your family plan.\n", u.email),
    )
    .await;
    Ok(())
}

async fn remove_members(state: &AppState, admin_id: i64) -> ApiResult<()> {
    let ms = fetch_members_for_admin(state, admin_id).await?;
    for m in ms.members {
        if m.is_admin {
            continue;
        }
        if m.status == status::ACCEPTED {
            remove_member(state, admin_id, &m.id).await?;
        } else if m.status == status::INVITED {
            revoke_invite(state, admin_id, &m.id).await?;
        }
    }
    Ok(())
}

/// Museum `CloseFamily`.
pub async fn close_family(state: &AppState, admin_id: i64) -> ApiResult<()> {
    remove_members(state, admin_id).await?;
    let mut tx = state.db.pool.begin().await?;
    sqlx::query("DELETE FROM families WHERE admin_id = $1")
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
    let r = sqlx::query("UPDATE users SET family_admin_id = NULL WHERE family_admin_id = $1")
        .bind(admin_id)
        .execute(&mut *tx)
        .await?;
    if r.rows_affected() != 1 {
        return Err(ApiError::internal("exactly one row should be updated"));
    }
    tx.commit().await?;
    Ok(())
}

/// Museum `ResetUserFamilyAccess` (used on account deletion).
pub async fn reset_family_access(state: &AppState, user_id: i64) -> ApiResult<()> {
    let u = get_user(state, user_id).await?;
    match u.family_admin_id {
        None => {
            sqlx::query("UPDATE families SET status = $1, updated_at = $2 WHERE member_id = $3 AND status = $4")
                .bind(status::DECLINED)
                .bind(now_micros())
                .bind(user_id)
                .bind(status::INVITED)
                .execute(&state.db.pool)
                .await?;
            Ok(())
        }
        Some(a) if a != user_id => leave_family(state, user_id).await,
        Some(_) => remove_members(state, user_id).await,
    }
}

/// Museum `ModifyMemberStorage`.
pub async fn modify_member_storage(
    state: &AppState,
    actor: i64,
    id: &str,
    limit: Option<i64>,
) -> ApiResult<()> {
    let m = member_by(&state.db, "id", id).await?;
    if m.admin_user_id != actor {
        return Err(permission_denied());
    }
    if m.is_admin {
        return Err(bad_request("can not limit admin storage"));
    }
    if m.status != status::ACCEPTED {
        return Err(bad_request_plain());
    }
    if let Some(l) = limit {
        let data = fetch_members_for_admin(state, m.admin_user_id)
            .await
            .map_err(|_| bad_request_plain())?;
        if l > data.storage + data.admin_bonus {
            return Err(storage_limit_exceeded());
        }
        if usage(&state.db, m.member_user_id).await? > l {
            return Err(bad_request("Failed to reduce storage"));
        }
    }
    sqlx::query("UPDATE families SET storage_limit = $1, updated_at = $2 WHERE id = $3")
        .bind(limit)
        .bind(now_micros())
        .bind(id)
        .execute(&state.db.pool)
        .await?;
    Ok(())
}

/// Museum `ente.InviteInfoResponse`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InviteInfo {
    /// Membership id.
    pub id: String,
    /// Inviting admin's email.
    pub admin_email: String,
}

/// Museum `InviteInfo`.
pub async fn invite_info(state: &AppState, token: &str) -> ApiResult<InviteInfo> {
    let m = member_by(&state.db, "token", token).await?;
    if m.status != status::INVITED {
        return Err(bad_request_plain());
    }
    let admin = get_user(state, m.admin_user_id).await?;
    if admin.family_admin_id != Some(admin.id) {
        return Err(ApiError::internal("inviter is no longer a family admin"));
    }
    Ok(InviteInfo {
        id: m.id,
        admin_email: admin.email,
    })
}

/// Museum `ente.AcceptInviteResponse`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptInviteResponse {
    /// Admin email.
    pub admin_email: String,
    /// Storage of the admin's plan plus usable bonus.
    pub storage: i64,
    /// Admin plan expiry.
    pub expiry_time: i64,
}

/// Museum `AcceptInvite`.
pub async fn accept_invite(state: &AppState, token: &str) -> ApiResult<AcceptInviteResponse> {
    let m = member_by(&state.db, "token", token).await?;
    let admin = get_user(state, m.admin_user_id).await?;
    if admin.family_admin_id != Some(admin.id) {
        return Err(ApiError::internal("inviter is no longer a family admin"));
    }
    if m.status != status::ACCEPTED {
        if m.status != status::INVITED {
            return Err(unauthorized_plain());
        }
        let mut tx = state.db.pool.begin().await?;
        sqlx::query("UPDATE families SET status = $1, updated_at = $2 WHERE token = $3")
            .bind(status::ACCEPTED)
            .bind(now_micros())
            .bind(token)
            .execute(&mut *tx)
            .await?;
        let r = sqlx::query(
            "UPDATE users SET family_admin_id = $1 WHERE user_id = $2 AND family_admin_id IS NULL",
        )
        .bind(m.admin_user_id)
        .bind(m.member_user_id)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() != 1 {
            return Err(ApiError::internal("exactly one row should be updated"));
        }
        tx.commit().await?;
        if let Ok(member) = get_user(state, m.member_user_id).await {
            notify(
                state,
                admin.id,
                &format!("{} has accepted your invitation", member.email),
                &format!("{} has joined your family plan.\n", member.email),
            )
            .await;
        }
    }
    let bonus = active_bonus(&state.db, admin.id).await?;
    let mut sub = subscription(&state.db, admin.id).await?;
    sub.storage = crate::capacity::effective_limit(state, sub.storage).await?;
    let usable = if sub.expiry_time < now_micros() {
        bonus.usable_bonus(0)
    } else {
        bonus.usable_bonus(sub.storage)
    };
    Ok(AcceptInviteResponse {
        admin_email: admin.email,
        storage: sub.storage + usable,
        expiry_time: sub.expiry_time,
    })
}
