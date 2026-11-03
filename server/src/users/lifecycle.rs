//! Account deletion and self-service recovery (museum `user.go` `handleAccountDeletion`,
//! `account_recovery.go`).

use super::*;
use crate::db::now_micros;
use axum::http::StatusCode;

/// Days a deleted account stays recoverable (museum `accountRecoveryLinkValidityDays`).
pub const RECOVERY_DAYS: i64 = 7;
const DAY: i64 = 24 * 3600 * 1_000_000;

/// Museum `handleAccountDeletion`: reset family access, revoke sessions and push tokens,
/// mark the account deleted and schedule its data for cleanup. Returns
/// `isSubscriptionCancelled` (always true here: there is no payment provider to cancel).
pub async fn delete_account(state: &AppState, user_id: i64, send_email: bool) -> ApiResult<bool> {
    let user = get_user(state, user_id).await?;
    // TODO(integration): call `crate::legacy::emergency::handle_account_deletion` and
    // `crate::public::memory::delete_for_user` here once those modules are merged in.
    family::reset_family_access(state, user_id).await?;
    revoke_tokens(&state.db, user_id, Revoke::All).await?;
    let now = now_micros();
    let mut tx = state.db.pool.begin().await?;
    sqlx::query("DELETE FROM push_tokens WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE users SET encrypted_email = '', email_decryption_nonce = '', email_hash = $1, is_deleted = TRUE WHERE user_id = $2")
        .bind(deleted_email_hash(user_id))
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM data_cleanup WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO data_cleanup (user_id, stage, stage_schedule_time, email_hash, created_at, updated_at) VALUES ($1, 'scheduled', $2, $3, $4, $5)")
        .bind(user_id)
        .bind(now + RECOVERY_DAYS * DAY)
        .bind(&user.hash)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    if send_email {
        let link = recovery_link(state, user_id, &user.email);
        crate::mail::deliver(
            &state.config,
            &user.email,
            "Your account has been deleted",
            &format!(
                "Your account has been deleted and its data will be removed in {RECOVERY_DAYS} days.\n\nIf this was a mistake, recover it here: {link}\n"
            ),
        )
        .await;
    }
    Ok(true)
}

/// Museum `getAccountRecoveryLink`.
pub fn recovery_link(state: &AppState, user_id: i64, email: &str) -> String {
    let claim = crate::jwt::WebClaim {
        user_id,
        expiry_time: now_micros() + RECOVERY_DAYS * DAY,
        email: email.to_string(),
        claim_scope: Some(crate::jwt::scope::RESTORE_ACCOUNT.into()),
        ..Default::default()
    };
    let token = crate::jwt::sign(&state.keys.jwt, &claim);
    format!(
        "{}/recover-account#recoveryToken={}",
        state.config.accounts.accounts_url.trim_end_matches('/'),
        url::form_urlencoded::byte_serialize(token.as_bytes()).collect::<String>()
    )
}

fn invalid_link() -> ApiError {
    api(
        StatusCode::BAD_REQUEST,
        "ACCOUNT_RECOVERY_INVALID_LINK",
        "invalid account recovery link",
    )
}
fn unavailable() -> ApiError {
    api(
        StatusCode::GONE,
        "ACCOUNT_RECOVERY_UNAVAILABLE",
        "account can no longer be recovered",
    )
}
fn email_in_use() -> ApiError {
    api(
        StatusCode::CONFLICT,
        "ACCOUNT_RECOVERY_EMAIL_IN_USE",
        "account recovery email already in use",
    )
}

/// `(user id, email)` from a recovery token.
pub fn parse_recovery_token(state: &AppState, token: &str) -> ApiResult<(i64, String)> {
    let claim = crate::jwt::verify_web(&state.keys.jwt, token, crate::jwt::scope::RESTORE_ACCOUNT)
        .map_err(|e| match e {
            crate::jwt::JwtError::Expired => api(
                StatusCode::GONE,
                "ACCOUNT_RECOVERY_LINK_EXPIRED",
                "expired account recovery link",
            ),
            _ => invalid_link(),
        })?;
    if claim.user_id == 0 || claim.email.is_empty() {
        return Err(invalid_link());
    }
    Ok((claim.user_id, claim.email))
}

/// Museum `accountRecoveryStatus`: `ready` or `recovered`.
pub async fn recovery_status(
    state: &AppState,
    user_id: i64,
    email: &str,
    allow_already_recovered: bool,
) -> ApiResult<&'static str> {
    match get_user(state, user_id).await {
        Ok(u) => {
            if allow_already_recovered && normalize_email(&u.email) == normalize_email(email) {
                return Ok("recovered");
            }
            if !allow_already_recovered {
                return Err(bad_request(
                    "account is already recovered or userID is linked to another active account",
                ));
            }
            return Err(unavailable());
        }
        Err(e) if e.status == StatusCode::NOT_FOUND => return Err(unavailable()),
        Err(e) if e.status == StatusCode::GONE => {}
        Err(e) => return Err(e),
    }
    if key_attributes(&state.db, user_id).await?.is_none() {
        return Err(unavailable());
    }
    let row: Option<(String,)> =
        sqlx::query_as("SELECT stage FROM data_cleanup WHERE user_id = $1 AND stage = 'scheduled'")
            .bind(user_id)
            .fetch_optional(&state.db.pool)
            .await?;
    if row.is_none() {
        return Err(unavailable());
    }
    if user_id_for_email(state, email)
        .await?
        .is_some_and(|other| other != user_id)
    {
        return Err(email_in_use());
    }
    Ok("ready")
}

/// Museum `recoverAccount`.
pub async fn recover_account(
    state: &AppState,
    user_id: i64,
    email: &str,
    allow_already_recovered: bool,
) -> ApiResult<&'static str> {
    if recovery_status(state, user_id, email, allow_already_recovered).await? == "recovered" {
        return Ok("recovered");
    }
    let email = normalize_email(email);
    let hash = email_hash(&state.keys, &email);
    let (ct, nonce) = encrypt_email(state, &email)?;
    let mut tx = state.db.pool.begin().await?;
    let row: Option<(Option<String>,)> = sqlx::query_as(
        "SELECT email_hash FROM data_cleanup WHERE user_id = $1 AND stage = 'scheduled'",
    )
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((stored,)) = row else {
        return Err(unavailable());
    };
    if stored.is_some_and(|s| s != hash) {
        return Err(unavailable());
    }
    sqlx::query("UPDATE users SET encrypted_email = $1, email_decryption_nonce = $2, email_hash = $3, is_deleted = FALSE WHERE user_id = $4")
        .bind(ct)
        .bind(nonce)
        .bind(&hash)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| match &e {
            sqlx::Error::Database(d) if d.is_unique_violation() => email_in_use(),
            _ => e.into(),
        })?;
    sqlx::query("DELETE FROM data_cleanup WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok("recovered")
}
