//! Public "collect" uploads: viewers of an album link with `enableCollect` upload
//! files into the album, owned by and charged to the album owner
//! (museum `GetUploadURLWithMetadata`, `GetMultipartUploadURLWithMetadata`, `FileController.Create`).
//! All three are the library's own upload paths, run as the album owner.

use super::models::File;
use crate::auth::App;
use crate::db::now_micros;
use crate::error::ApiResult;
use crate::library::files;
use crate::library::objects::{
    self, MultipartUploadUrlRequest, MultipartUploadUrls, UploadUrl, UploadUrlRequest,
};
use crate::state::AppState;

/// Single-PUT upload URL charged to `owner_id`, staged like any other upload.
pub async fn upload_url(
    state: &AppState,
    owner_id: i64,
    app: App,
    client: &str,
    req: &UploadUrlRequest,
) -> ApiResult<UploadUrl> {
    objects::upload_url_with_metadata(state, owner_id, req, app, client).await
}

/// Multipart upload URLs charged to `owner_id`, binding each part's length and MD5.
pub async fn multipart_upload_url(
    state: &AppState,
    owner_id: i64,
    app: App,
    client: &str,
    req: &MultipartUploadUrlRequest,
) -> ApiResult<MultipartUploadUrls> {
    objects::multipart_upload_url_with_metadata(state, owner_id, req, app, client).await
}

/// Create `file` in the owner's collection from objects already uploaded
/// (museum `CollectionLinkController.CreateFile` → `FileController.Create`).
pub async fn create_file(
    state: &AppState,
    owner_id: i64,
    app: App,
    mut file: File,
) -> ApiResult<File> {
    // Public links may only create, never update, and always as the owner.
    file.id = 0;
    file.owner_id = owner_id;
    file.updation_time = now_micros();
    file.is_deleted = false;
    file.action = None;
    file.action_user = None;
    files::create(state, owner_id, app, file).await
}
