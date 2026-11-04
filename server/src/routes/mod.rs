//! HTTP routes, one module per museum endpoint group.

pub mod admin;
pub mod authenticator;
pub mod billing;
pub mod cast;
pub mod collection_actions;
pub mod collections;
pub mod contacts;
pub mod discount;
pub mod emergency;
pub mod family;
pub mod file_data;
pub mod file_link;
pub mod files;
pub mod health;
pub mod legacy_kit;
pub mod memory_share;
pub mod misc;
pub mod objects;
pub mod passkeys;
pub mod photovault;
pub mod public_collection;
pub mod push;
pub mod remote_store;
pub mod social;
pub mod storage_bonus;
pub mod trash;
pub mod user_entity;
pub mod users;

use crate::state::AppState;
use axum::Router;

/// All API routes (without global middleware).
pub fn api(state: &AppState) -> Router<AppState> {
    Router::new()
        .merge(public_collection::social_router(state.clone()))
        .merge(admin::router())
        .merge(authenticator::router())
        .merge(billing::router())
        .merge(cast::router())
        .merge(collection_actions::router())
        .merge(collections::router())
        .merge(contacts::router())
        .merge(discount::router())
        .merge(emergency::router())
        .merge(family::router())
        .merge(file_data::router())
        .merge(file_link::router())
        .merge(files::router())
        .merge(health::router())
        .merge(legacy_kit::router())
        .merge(memory_share::router())
        .merge(misc::router())
        .merge(objects::router())
        .merge(passkeys::router())
        .merge(photovault::router())
        .merge(crate::asset_mirror::router(state.config.asset_mirror))
        .merge(public_collection::router())
        .merge(push::router())
        .merge(remote_store::router())
        .merge(social::router())
        .merge(storage_bonus::router())
        .merge(trash::router())
        .merge(user_entity::router())
        .merge(users::router())
}
