mod archive;
pub mod download;
mod store;
// PHOTOVAULT: server connection (download mirror).
mod mirror;
pub use mirror::{
    block_url_prefix, block_url_prefix_if_unset, is_blocked, mirrored_url, set_url_mirror,
};

pub use store::{Asset, AssetDownloadProgress, AssetFile, AssetStore, AssetStoreError};
