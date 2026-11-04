#[flutter_rust_bridge::frb(init)]
pub fn init_app() {
    // Install before FRB claims the process logger.
    #[cfg(feature = "flutter")]
    crate::api::log::install();
    flutter_rust_bridge::setup_default_user_utils();
    // PHOTOVAULT: server connection. Model downloads stay blocked until the
    // app sets the server's mirror (never Ente's host); a mirror set by
    // another isolate is kept.
    ente_assets::block_url_prefix_if_unset(crate::api::ml_indexing_api::MODEL_CATALOG_PREFIX);
}
