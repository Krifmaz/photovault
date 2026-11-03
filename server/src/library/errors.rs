//! museum's error values for the library endpoints.
//!
//! museum distinguishes *untyped* errors (sentinel `errors.New` values that map to a bare
//! status with an empty `{}` body) from *typed* `ente.ApiError`s (`{"code", "message"}`).
//! The apps branch on both, so each helper here names the museum value it mirrors.

use crate::error::ApiError;
use axum::http::StatusCode;

/// `ente.ErrBadRequest` / bind and validation errors: 400 `{}`.
pub fn bad() -> ApiError {
    ApiError::status(StatusCode::BAD_REQUEST)
}

/// `ente.NewBadRequestWithMessage`: 400 `{"code":"BAD_REQUEST"}`.
pub fn bad_msg(msg: impl Into<String>) -> ApiError {
    ApiError::bad_request(msg)
}

/// `ente.ErrPermissionDenied`: 403 `{}`.
pub fn denied() -> ApiError {
    ApiError::status(StatusCode::FORBIDDEN)
}

/// `ente.NewPermissionDeniedError`: 403 `{"code":"PERMISSION_DENIED"}`.
pub fn denied_msg(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::FORBIDDEN, "PERMISSION_DENIED", msg)
}

/// `ente.ErrNotFound` / `sql.ErrNoRows`: 404 `{}`.
pub fn not_found() -> ApiError {
    ApiError::status(StatusCode::NOT_FOUND)
}

/// `ente.ErrNotFoundError`: 404 `{"code":"NOT_FOUND","message":""}`.
pub fn not_found_coded() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "")
}

/// `ente.ErrVersionMismatch`: 409 `{}`.
pub fn version_mismatch() -> ApiError {
    ApiError::status(StatusCode::CONFLICT)
}

/// `ente.ErrStorageLimitExceeded`: 426 `{}`.
pub fn storage_limit() -> ApiError {
    ApiError::status(StatusCode::UPGRADE_REQUIRED)
}

/// `ente.ErrFileTooLarge` / `ente.ErrBatchSizeTooLarge`: 413 `{}`.
pub fn too_large() -> ApiError {
    ApiError::status(StatusCode::PAYLOAD_TOO_LARGE)
}

/// Plain `errors.New`/`fmt.Errorf` in museum: 500 `{}` (the reason is logged).
pub fn internal(msg: impl std::fmt::Display) -> ApiError {
    ApiError::internal(msg)
}

/// `ente.ErrCollectionDeleted`.
pub fn collection_deleted() -> ApiError {
    ApiError::new(StatusCode::NOT_FOUND, "COLLECTION_DELETED", "")
}

/// `ente.ErrFileUploadConflict`.
pub fn upload_conflict() -> ApiError {
    ApiError::new(
        StatusCode::BAD_REQUEST,
        "FILE_UPLOAD_CONFLICT",
        "File upload conflicts with an existing file",
    )
}

/// `ente.ErrObjSizeFetchFailed`.
pub fn object_size_fetch_failed() -> ApiError {
    ApiError::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "OBJECT_SIZE_FETCH_FAILED",
        "",
    )
}

/// `ente.ErrCollectionNotEmpty`.
pub fn collection_not_empty() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "COLLECTION_NOT_EMPTY",
        "The collection is not empty",
    )
}

/// `ente.ErrFileNotFoundInAlbum`.
pub fn file_not_found_in_album() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "FILE_NOT_FOUND_IN_ALBUM",
        "File is either deleted or moved to different collection",
    )
}

/// `ente.ErrFileInTrash`.
pub fn file_in_trash() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "FILE_IN_TRASH",
        "One or more files are in trash or have been deleted, please restore them first",
    )
}

/// `ente.ErrFileLimitReached` (Locker).
pub fn file_limit_reached() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "FILE_LIMIT_REACHED",
        "Maximum file limit reached",
    )
}

/// `ente.ErrUserNotFound`.
pub fn user_not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "USER_NOT_FOUND",
        "User is either deleted or not found",
    )
}

/// `ente.ErrRecipientIdentityMismatch`.
pub fn recipient_identity_mismatch() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "RECIPIENT_IDENTITY_MISMATCH",
        "Recipient identity does not match",
    )
}

/// `ente.ErrAutomaticShareRecipientNotEligible`.
pub fn automatic_share_not_eligible() -> ApiError {
    ApiError::new(
        StatusCode::FORBIDDEN,
        "AUTOMATIC_SHARE_RECIPIENT_NOT_ELIGIBLE",
        "Automatic share recipient is not eligible",
    )
}

/// File data `INVALID_VERSION` (409).
pub fn invalid_version(msg: impl Into<String>) -> ApiError {
    ApiError::new(StatusCode::CONFLICT, "INVALID_VERSION", msg)
}

/// 401 `{}` (`ente.ErrAuthenticationRequired`, wrong link password).
pub fn unauthorized() -> ApiError {
    ApiError::status(StatusCode::UNAUTHORIZED)
}

/// Map a storage error to museum's behaviour (missing object: 404 `{}`).
pub fn storage(e: crate::storage::StorageError) -> ApiError {
    match e {
        crate::storage::StorageError::NotFound => not_found(),
        e => ApiError::internal(e),
    }
}
