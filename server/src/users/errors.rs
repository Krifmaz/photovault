//! Museum error values used by the account endpoints, with museum's exact status codes and
//! bodies. Plain Go errors (`ente.ErrIncorrectOTT`, `sql.ErrNoRows`, …) serialise as `{}` with
//! a status from museum's `handler.httpStatusCode`; `ente.ApiError`s carry a `code`.

use crate::error::ApiError;
use axum::http::StatusCode;

/// `ente.ErrBadRequest` / bind errors: 400 `{}`.
pub fn bad_request_plain() -> ApiError {
    ApiError::status(StatusCode::BAD_REQUEST)
}
/// `ente.NewBadRequestWithMessage`: 400 `BAD_REQUEST`.
pub fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError::bad_request(msg)
}
/// `sql.ErrNoRows` / `ente.ErrNotFound`: 404 `{}`.
pub fn not_found_plain() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND)
}
/// `ente.ErrPermissionDenied`: 403 `{}`.
pub fn permission_denied() -> ApiError {
    ApiError::status(StatusCode::FORBIDDEN)
}
/// `ente.ErrIncorrectOTT` / `ErrIncorrectTOTP` / `ErrInvalidPassword` / `ErrAuthenticationRequired`: 401 `{}`.
pub fn unauthorized_plain() -> ApiError {
    ApiError::status(StatusCode::UNAUTHORIZED)
}
/// `ente.ErrExpiredOTT` / `ErrUserDeleted`: 410 `{}`.
pub fn gone_plain() -> ApiError {
    ApiError::status(StatusCode::GONE)
}
/// `ente.ErrTooManyBadRequest`: 429 `{}`.
pub fn too_many_plain() -> ApiError {
    ApiError::status(StatusCode::TOO_MANY_REQUESTS)
}
/// `ente.ErrNoActiveSubscription`: 402 `{}`.
pub fn no_active_subscription() -> ApiError {
    ApiError::status(StatusCode::PAYMENT_REQUIRED)
}
/// `ente.ErrStorageLimitExceeded`: 426 `{}`.
pub fn storage_limit_exceeded() -> ApiError {
    ApiError::status(StatusCode::UPGRADE_REQUIRED)
}
/// `ente.ErrBatchSizeTooLarge`: 413 `{}`.
pub fn batch_too_large() -> ApiError {
    ApiError::status(StatusCode::PAYLOAD_TOO_LARGE)
}
/// `ente.ErrCanNotInviteUserWithPaidPlan` / `ErrVersionMismatch`: 409 `{}`.
pub fn conflict_plain() -> ApiError {
    ApiError::status(StatusCode::CONFLICT)
}
/// `ente.ErrCanNotInviteUserAlreadyInFamily`: 406 `{}`.
pub fn already_in_family() -> ApiError {
    ApiError::status(StatusCode::NOT_ACCEPTABLE)
}
/// `ente.ErrFamilySizeLimitReached`: 412 `{}`.
pub fn family_size_limit() -> ApiError {
    ApiError::status(StatusCode::PRECONDITION_FAILED)
}
/// `ente.ErrNotImplemented`: 501 `{}`.
pub fn not_implemented() -> ApiError {
    ApiError::status(StatusCode::NOT_IMPLEMENTED)
}
/// Typed museum error.
pub fn api(status: StatusCode, code: &'static str, msg: &str) -> ApiError {
    ApiError::new(status, code, msg)
}
/// `USER_ALREADY_REGISTERED` (409).
pub fn user_already_registered() -> ApiError {
    api(
        StatusCode::CONFLICT,
        "USER_ALREADY_REGISTERED",
        "User is already registered",
    )
}
/// `USER_NOT_REGISTERED` (404).
pub fn user_not_registered() -> ApiError {
    api(
        StatusCode::NOT_FOUND,
        "USER_NOT_REGISTERED",
        "User is not registered",
    )
}
/// `USER_SIGNUP_INCOMPLETE` (404).
pub fn user_signup_incomplete() -> ApiError {
    api(
        StatusCode::NOT_FOUND,
        "USER_SIGNUP_INCOMPLETE",
        "User has not completed sign up",
    )
}
/// `ente.ErrUserNotFound`: `USER_NOT_FOUND` (404).
pub fn user_not_found() -> ApiError {
    api(StatusCode::NOT_FOUND, "USER_NOT_FOUND", "User is not found")
}
/// `ente.ErrNotFoundError`: `NOT_FOUND` (404).
pub fn not_found_typed(msg: &str) -> ApiError {
    api(StatusCode::NOT_FOUND, "NOT_FOUND", msg)
}
/// `EMAIL_MFA_ENABLED` (409).
pub fn email_mfa_enabled() -> ApiError {
    api(
        StatusCode::CONFLICT,
        "EMAIL_MFA_ENABLED",
        "Email MFA is enabled",
    )
}
/// `ente.NewConflictError`: `CONFLICT` (409).
pub fn conflict(msg: &str) -> ApiError {
    api(StatusCode::CONFLICT, "CONFLICT", msg)
}
