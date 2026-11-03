//! Database access for users, key attributes, tokens, subscriptions, bonuses and usage.

use super::*;
use crate::auth::App;
use crate::db::{Db, now_micros};
use serde::{Deserialize, Serialize};
use sqlx::Row;

/// One microsecond-year, as museum's `NYearsFromNow` approximates it (365.25 days).
const YEAR_MICROS: i64 = 365 * 24 * 3600 * 1_000_000 + 6 * 3600 * 1_000_000;
/// Museum `TrialPeriodDuration` (years) for the free/self-hosted subscription.
pub const FREE_PLAN_YEARS: i64 = 100;
/// Museum `StorageOverflowAboveSubscriptionLimit`.
pub const STORAGE_OVERFLOW: i64 = 50 * 1024 * 1024;

/// A user row (museum `ente.User`).
#[derive(Debug, Clone)]
pub struct User {
    /// Id.
    pub id: i64,
    /// Decrypted email.
    pub email: String,
    /// Email lookup hash.
    pub hash: String,
    /// Creation time (µs).
    pub creation_time: i64,
    /// Family admin, if in a family.
    pub family_admin_id: Option<i64>,
    /// TOTP enabled.
    pub is_two_factor_enabled: bool,
    /// Email OTP required on every login.
    pub email_mfa: bool,
}

type UserRow = (i64, String, String, String, Option<i64>, i64, i64, i64, i64);

const USER_COLS: &str = "user_id, encrypted_email, email_decryption_nonce, email_hash, family_admin_id, creation_time, CASE WHEN is_two_factor_enabled THEN 1 ELSE 0 END, CASE WHEN email_mfa THEN 1 ELSE 0 END, CASE WHEN is_deleted THEN 1 ELSE 0 END";

/// Museum `UserRepo.Get`: 404 `{}` if missing, 410 `{}` if deleted.
pub async fn get_user(state: &AppState, user_id: i64) -> ApiResult<User> {
    let row: Option<UserRow> =
        sqlx::query_as(&format!("SELECT {USER_COLS} FROM users WHERE user_id = $1"))
            .bind(user_id)
            .fetch_optional(&state.db.pool)
            .await?;
    let (id, ct, nonce, hash, fam, created, tfa, mfa, deleted) = row.ok_or_else(not_found_plain)?;
    if deleted != 0 || hash == deleted_email_hash(id) {
        return Err(gone_plain());
    }
    Ok(User {
        id,
        email: decrypt_email(state, &ct, &nonce)?,
        hash,
        creation_time: created,
        family_admin_id: fam,
        is_two_factor_enabled: tfa != 0,
        email_mfa: mfa != 0,
    })
}

/// Museum `UserController.GetUser`: deleted accounts become `USER_NOT_FOUND`.
pub async fn get_user_not_deleted(state: &AppState, user_id: i64) -> ApiResult<User> {
    get_user(state, user_id).await.map_err(|e| {
        if e.status == axum::http::StatusCode::GONE {
            user_not_found()
        } else {
            e
        }
    })
}

/// Museum `GetUserIDWithEmailUnrestricted` (email is normalised here).
pub async fn user_id_for_email(state: &AppState, email: &str) -> ApiResult<Option<i64>> {
    let hash = email_hash(state, &normalize_email(email));
    let row: Option<(i64,)> = sqlx::query_as("SELECT user_id FROM users WHERE email_hash = $1")
        .bind(hash)
        .fetch_optional(&state.db.pool)
        .await?;
    Ok(row.map(|r| r.0))
}

/// Decrypted email of an active user.
pub async fn email_for_user(state: &AppState, user_id: i64) -> ApiResult<Option<String>> {
    match get_user(state, user_id).await {
        Ok(u) => Ok(Some(u.email)),
        Err(e) if e.code.is_none() && (e.status.as_u16() == 404 || e.status.as_u16() == 410) => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// Create an account for `email` with the configured default quota and a free subscription.
pub async fn create_user(state: &AppState, email: &str, source: Option<&str>) -> ApiResult<i64> {
    let email = normalize_email(email);
    let (ct, nonce) = encrypt_email(state, &email)?;
    let now = now_micros();
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO users (encrypted_email, email_decryption_nonce, email_hash, creation_time, storage_quota, subscription_expiry, source) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING user_id",
    )
    .bind(ct)
    .bind(nonce)
    .bind(email_hash(state, &email))
    .bind(now)
    .bind(state.config.default_storage_bytes)
    .bind(now + FREE_PLAN_YEARS * YEAR_MICROS)
    .bind(source.map(str::to_string))
    .fetch_one(&state.db.pool)
    .await?;
    Ok(id)
}

/// Museum `ente.KeyAttributes` (every field is always serialised).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyAttributes {
    /// Salt for the key-encryption key.
    pub kek_salt: String,
    /// Master key encrypted with the KEK.
    pub encrypted_key: String,
    /// Nonce for `encrypted_key`.
    pub key_decryption_nonce: String,
    /// X25519 public key.
    pub public_key: String,
    /// Secret key encrypted with the master key.
    pub encrypted_secret_key: String,
    /// Nonce for `encrypted_secret_key`.
    pub secret_key_decryption_nonce: String,
    /// Argon2 memory limit.
    pub mem_limit: i64,
    /// Argon2 ops limit.
    pub ops_limit: i64,
    /// Master key encrypted with the recovery key.
    pub master_key_encrypted_with_recovery_key: String,
    /// Nonce for the above.
    pub master_key_decryption_nonce: String,
    /// Recovery key encrypted with the master key.
    pub recovery_key_encrypted_with_master_key: String,
    /// Nonce for the above.
    pub recovery_key_decryption_nonce: String,
}

/// Key attributes, if the user has set them.
pub async fn key_attributes(db: &Db, user_id: i64) -> ApiResult<Option<KeyAttributes>> {
    let row = sqlx::query(
        "SELECT kek_salt, encrypted_key, key_decryption_nonce, public_key, encrypted_secret_key, secret_key_decryption_nonce, mem_limit, ops_limit, \
         master_key_encrypted_with_recovery_key, master_key_decryption_nonce, recovery_key_encrypted_with_master_key, recovery_key_decryption_nonce \
         FROM key_attributes WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&db.pool)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let opt = |i: usize| -> String {
        r.try_get::<Option<String>, _>(i)
            .ok()
            .flatten()
            .unwrap_or_default()
    };
    Ok(Some(KeyAttributes {
        kek_salt: r.try_get(0)?,
        encrypted_key: r.try_get(1)?,
        key_decryption_nonce: r.try_get(2)?,
        public_key: r.try_get(3)?,
        encrypted_secret_key: r.try_get(4)?,
        secret_key_decryption_nonce: r.try_get(5)?,
        mem_limit: r.try_get(6)?,
        ops_limit: r.try_get(7)?,
        master_key_encrypted_with_recovery_key: opt(8),
        master_key_decryption_nonce: opt(9),
        recovery_key_encrypted_with_master_key: opt(10),
        recovery_key_decryption_nonce: opt(11),
    }))
}

fn none_if_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// Insert key attributes (fails if present).
pub async fn insert_key_attributes(db: &Db, user_id: i64, k: &KeyAttributes) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO key_attributes (user_id, kek_salt, encrypted_key, key_decryption_nonce, public_key, encrypted_secret_key, secret_key_decryption_nonce, \
         mem_limit, ops_limit, master_key_encrypted_with_recovery_key, master_key_decryption_nonce, recovery_key_encrypted_with_master_key, recovery_key_decryption_nonce, creation_time) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
    )
    .bind(user_id)
    .bind(&k.kek_salt)
    .bind(&k.encrypted_key)
    .bind(&k.key_decryption_nonce)
    .bind(&k.public_key)
    .bind(&k.encrypted_secret_key)
    .bind(&k.secret_key_decryption_nonce)
    .bind(k.mem_limit)
    .bind(k.ops_limit)
    .bind(none_if_empty(&k.master_key_encrypted_with_recovery_key))
    .bind(none_if_empty(&k.master_key_decryption_nonce))
    .bind(none_if_empty(&k.recovery_key_encrypted_with_master_key))
    .bind(none_if_empty(&k.recovery_key_decryption_nonce))
    .bind(now_micros())
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// The user's X25519 public key (museum `GetPublicKey`); 404 `{}` without key attributes.
pub async fn public_key(db: &Db, user_id: i64) -> ApiResult<String> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT public_key FROM key_attributes WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&db.pool)
            .await?;
    row.map(|r| r.0).ok_or_else(not_found_plain)
}

/// Insert a session token row.
pub async fn insert_token<'e, E>(
    exec: E,
    user_id: i64,
    app: App,
    token: &str,
    ip: &str,
    user_agent: &str,
) -> ApiResult<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Any>,
{
    let now = now_micros();
    sqlx::query(
        "INSERT INTO tokens (token_hash, user_id, app, creation_time, last_used_at, ip, user_agent) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(crate::auth::hash_token(token))
    .bind(user_id)
    .bind(app.as_str())
    .bind(now)
    .bind(now)
    .bind(ip)
    .bind(user_agent)
    .execute(exec)
    .await?;
    Ok(())
}

/// Which tokens to revoke.
#[derive(Debug, Clone, Copy)]
pub enum Revoke<'a> {
    /// One session by hash.
    One(&'a str),
    /// All except this hash.
    AllExcept(&'a str),
    /// Every session.
    All,
    /// All sessions of the given apps.
    Apps(&'a [App]),
}

/// Mark tokens deleted and drop their push registrations (museum `markTokensDeleted`).
pub async fn revoke_tokens(db: &Db, user_id: i64, which: Revoke<'_>) -> ApiResult<u64> {
    let mut tx = db.pool.begin().await?;
    let hashes: Vec<(String,)> = match which {
        Revoke::One(h) => {
            sqlx::query_as("SELECT token_hash FROM tokens WHERE user_id = $1 AND token_hash = $2 AND is_deleted = FALSE")
                .bind(user_id)
                .bind(h)
                .fetch_all(&mut *tx)
                .await?
        }
        Revoke::AllExcept(h) => {
            sqlx::query_as("SELECT token_hash FROM tokens WHERE user_id = $1 AND token_hash <> $2 AND is_deleted = FALSE")
                .bind(user_id)
                .bind(h)
                .fetch_all(&mut *tx)
                .await?
        }
        Revoke::All => {
            sqlx::query_as("SELECT token_hash FROM tokens WHERE user_id = $1 AND is_deleted = FALSE")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?
        }
        Revoke::Apps(apps) => {
            let mut v = Vec::new();
            for a in apps {
                v.extend(
                    sqlx::query_as::<_, (String,)>("SELECT token_hash FROM tokens WHERE user_id = $1 AND app = $2 AND is_deleted = FALSE")
                        .bind(user_id)
                        .bind(a.as_str())
                        .fetch_all(&mut *tx)
                        .await?,
                );
            }
            v
        }
    };
    for (h,) in &hashes {
        sqlx::query("UPDATE tokens SET is_deleted = TRUE WHERE token_hash = $1")
            .bind(h)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM push_tokens WHERE session_token_hash = $1")
            .bind(h)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(hashes.len() as u64)
}

fn missing_table(e: &sqlx::Error) -> bool {
    matches!(e, sqlx::Error::Database(d)
        if d.message().contains("no such table") || d.message().contains("does not exist"))
}

/// Bytes used by `user_id`. Like museum this reads `usage.storage_consumed` (maintained by
/// the files workstream); without that table it sums the user's live `object_keys`, and it
/// returns 0 when neither table exists.
pub async fn usage(db: &Db, user_id: i64) -> ApiResult<i64> {
    let r: Result<Option<(i64,)>, sqlx::Error> =
        sqlx::query_as("SELECT storage_consumed FROM usage WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&db.pool)
            .await;
    match r {
        Ok(row) => return Ok(row.map(|r| r.0).unwrap_or(0)),
        Err(e) if missing_table(&e) => {}
        Err(e) => return Err(e.into()),
    }
    let r: Result<(i64,), sqlx::Error> = sqlx::query_as(
        "SELECT CAST(COALESCE(SUM(ok.size), 0) AS BIGINT) FROM object_keys ok JOIN files f ON f.file_id = ok.file_id \
         WHERE f.owner_id = $1 AND ok.is_deleted = FALSE",
    )
    .bind(user_id)
    .fetch_one(&db.pool)
    .await;
    match r {
        Ok((n,)) => Ok(n),
        Err(e) if missing_table(&e) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// Number of the user's `app` files that are in at least one live collection (museum's file
/// count on `details/v2`); 0 until the files workstream's tables exist.
pub async fn file_count(db: &Db, user_id: i64, app: &str) -> ApiResult<i64> {
    Ok(app_file_stats(db, user_id, app).await?.0)
}

/// `(file count, bytes)` of the user's live files of one app (`photos`, `locker`).
pub async fn app_file_stats(db: &Db, user_id: i64, app: &str) -> ApiResult<(i64, i64)> {
    let r: Result<(i64, i64), sqlx::Error> = sqlx::query_as(
        "SELECT COUNT(*), CAST(COALESCE(SUM((SELECT SUM(ok.size) FROM object_keys ok WHERE ok.file_id = f.file_id AND ok.is_deleted = FALSE)), 0) AS BIGINT) \
         FROM files f WHERE f.owner_id = $1 AND COALESCE(f.app, 'photos') = $2 AND EXISTS \
         (SELECT 1 FROM collection_files cf WHERE cf.file_id = f.file_id AND cf.is_deleted = FALSE)",
    )
    .bind(user_id)
    .bind(app)
    .fetch_one(&db.pool)
    .await;
    match r {
        Ok(v) => Ok(v),
        Err(e) if missing_table(&e) => Ok((0, 0)),
        Err(e) => Err(e.into()),
    }
}

/// Live authenticator codes (Auth app); 0 when that workstream's table is absent.
pub async fn authenticator_count(db: &Db, user_id: i64) -> ApiResult<i64> {
    let r: Result<(i64,), sqlx::Error> = sqlx::query_as(
        "SELECT COUNT(*) FROM authenticator_entity WHERE user_id = $1 AND is_deleted = FALSE",
    )
    .bind(user_id)
    .fetch_one(&db.pool)
    .await;
    match r {
        Ok((n,)) => Ok(n),
        Err(e) if missing_table(&e) => Ok(0),
        Err(e) => Err(e.into()),
    }
}

/// Combined usage of several users.
pub async fn combined_usage(db: &Db, user_ids: &[i64]) -> ApiResult<i64> {
    let mut t = 0;
    for id in user_ids {
        t += usage(db, *id).await?;
    }
    Ok(t)
}

/// Museum `ente.SubscriptionAttributes`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionAttributes {
    /// Cancelled at period end.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_cancelled: bool,
    /// Stripe customer.
    #[serde(
        rename = "customerID",
        default,
        skip_serializing_if = "String::is_empty"
    )]
    pub customer_id: String,
    /// Opaque verification data.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub latest_verification_data: String,
    /// Stripe account country.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stripe_account_country: String,
}

/// Museum `ente.Subscription`; self-hosted it is synthesised from the user row.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    /// Id (= user id here).
    pub id: i64,
    /// Owner.
    #[serde(rename = "userID")]
    pub user_id: i64,
    /// `free` unless changed by an admin.
    #[serde(rename = "productID")]
    pub product_id: String,
    /// Bytes (`users.storage_quota`).
    pub storage: i64,
    /// `none` for the free plan.
    #[serde(rename = "originalTransactionID")]
    pub original_transaction_id: String,
    /// Expiry (µs).
    pub expiry_time: i64,
    /// Payment provider (empty).
    pub payment_provider: String,
    /// Attributes.
    pub attributes: SubscriptionAttributes,
    /// Price (empty for free).
    pub price: String,
    /// Period (`year` for free).
    pub period: String,
}

impl Subscription {
    /// Museum `IsActivePaidPlan`.
    pub fn is_active_paid(&self) -> bool {
        self.product_id != "free" && self.expiry_time > now_micros()
    }
}

/// The user's subscription (museum `BillingController.GetSubscription`).
pub async fn subscription(db: &Db, user_id: i64) -> ApiResult<Subscription> {
    let row: Option<(i64, String, i64, String, String, String, i64)> = sqlx::query_as(
        "SELECT storage_quota, subscription_product_id, subscription_expiry, subscription_payment_provider, subscription_transaction_id, subscription_attributes, creation_time \
         FROM users WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(&db.pool)
    .await?;
    let (storage, product, expiry, provider, txn, attrs, created) =
        row.ok_or_else(not_found_plain)?;
    let expiry = if expiry == 0 {
        created + FREE_PLAN_YEARS * YEAR_MICROS
    } else {
        expiry
    };
    let free = product == "free" || product == "family";
    Ok(Subscription {
        id: user_id,
        user_id,
        product_id: product,
        storage,
        original_transaction_id: txn,
        expiry_time: expiry,
        payment_provider: provider,
        attributes: serde_json::from_str(&attrs).unwrap_or_default(),
        price: String::new(),
        period: if free { "year".into() } else { String::new() },
    })
}

/// One storage bonus (museum `storagebonus.StorageBonus`).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageBonus {
    /// Bytes.
    pub storage: i64,
    /// Bonus type (`REFERRAL`, `SIGN_UP`, `ADD_ON_*`).
    #[serde(rename = "type")]
    pub bonus_type: String,
    /// Creation (µs).
    pub created_at: i64,
    /// Expiry (µs), 0 = never.
    pub valid_till: i64,
    /// Revoked.
    pub is_revoked: bool,
}

impl StorageBonus {
    fn restricts_doubling(&self) -> bool {
        !self.bonus_type.starts_with("ADD_ON_")
    }
}

/// Museum `ActiveStorageBonus`.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActiveStorageBonus {
    /// Active bonuses.
    pub storage_bonuses: Vec<StorageBonus>,
}

impl ActiveStorageBonus {
    /// Museum `GetUsableBonus`: referral bonus capped at plan + add-ons.
    pub fn usable_bonus(&self, sub_storage: i64) -> i64 {
        let referral: i64 = self
            .storage_bonuses
            .iter()
            .filter(|b| b.restricts_doubling())
            .map(|b| b.storage)
            .sum();
        let addon: i64 = self
            .storage_bonuses
            .iter()
            .filter(|b| !b.restricts_doubling())
            .map(|b| b.storage)
            .sum();
        addon + referral.min(addon + sub_storage)
    }
    /// Museum `GetMaxExpiry` (add-ons only).
    pub fn max_expiry(&self) -> i64 {
        self.storage_bonuses
            .iter()
            .filter(|b| !b.restricts_doubling())
            .map(|b| b.valid_till)
            .max()
            .unwrap_or(0)
    }
}

/// Active (unrevoked, unexpired) bonuses.
pub async fn active_bonus(db: &Db, user_id: i64) -> ApiResult<ActiveStorageBonus> {
    let rows: Vec<(i64, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT storage, type, created_at, valid_till, CASE WHEN is_revoked THEN 1 ELSE 0 END FROM storage_bonus \
         WHERE user_id = $1 AND is_revoked = FALSE AND (valid_till = 0 OR valid_till > $2) ORDER BY created_at",
    )
    .bind(user_id)
    .bind(now_micros())
    .fetch_all(&db.pool)
    .await?;
    Ok(ActiveStorageBonus {
        storage_bonuses: rows
            .into_iter()
            .map(
                |(storage, bonus_type, created_at, valid_till, is_revoked)| StorageBonus {
                    storage,
                    bonus_type,
                    created_at,
                    valid_till,
                    is_revoked: is_revoked != 0,
                },
            )
            .collect(),
    })
}

/// Museum `UsageController.CanUploadFile` for Photos: 426 `{}` when `size` more bytes would
/// exceed the plan (+50 MB overflow) and usable bonus, or the member's family limit;
/// 402 `{}` when the plan and all add-ons expired.
pub async fn can_upload(state: &AppState, user_id: i64, size: Option<i64>) -> ApiResult<()> {
    let db = &state.db;
    let fam: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT family_admin_id FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&db.pool)
            .await?;
    let family_admin = fam.and_then(|f| f.0);
    let (admin, members, member_limit) = match family_admin {
        Some(a) => {
            let ms = family::active_members(db, a).await?;
            let limit = ms
                .iter()
                .find(|m| m.0 == user_id && user_id != a)
                .and_then(|m| m.1);
            (a, ms.into_iter().map(|m| m.0).collect::<Vec<_>>(), limit)
        }
        None => (user_id, vec![user_id], None),
    };
    let sub = subscription(db, admin).await?;
    let bonus = active_bonus(db, admin).await?;
    let mut sub_storage = if sub.expiry_time > now_micros() {
        sub.storage
    } else {
        if bonus.max_expiry() <= 0 {
            return Err(no_active_subscription());
        }
        0
    };
    let mut new_usage = combined_usage(db, &members).await?;
    if let Some(s) = size {
        new_usage += s;
        sub_storage += STORAGE_OVERFLOW;
    }
    if new_usage > sub_storage + bonus.usable_bonus(sub_storage) {
        return Err(storage_limit_exceeded());
    }
    if let Some(limit) = member_limit {
        let mine = usage(db, user_id).await? + size.unwrap_or(0);
        if mine > limit + STORAGE_OVERFLOW {
            return Err(storage_limit_exceeded());
        }
    }
    Ok(())
}

/// Total bytes the user may store: own plan + usable bonus, or for family members the
/// admin's plan + bonus capped by the member's limit.
pub async fn storage_limit(state: &AppState, user_id: i64) -> ApiResult<i64> {
    let db = &state.db;
    let fam: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT family_admin_id FROM users WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(&db.pool)
            .await?;
    let admin = fam.and_then(|f| f.0).unwrap_or(user_id);
    let sub = subscription(db, admin).await?;
    let plan = if sub.expiry_time > now_micros() {
        sub.storage
    } else {
        0
    };
    let total = plan + active_bonus(db, admin).await?.usable_bonus(plan);
    if admin != user_id {
        let limit = family::active_members(db, admin)
            .await?
            .into_iter()
            .find(|m| m.0 == user_id)
            .and_then(|m| m.1);
        if let Some(l) = limit {
            return Ok(l.min(total));
        }
    }
    Ok(total)
}
