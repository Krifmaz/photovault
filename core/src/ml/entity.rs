//! Ente's user entities (`/user-entity/...`): end-to-end encrypted JSON
//! records of a type (`cgroup` = people, `location` = location tags),
//! encrypted with a per-type entity key that is itself encrypted with the
//! master key.
//!
//! STUB (contract v1.2): owned by the `core-faces` track. Internal API used
//! by `faces` and `search`:
//!
//! - `sync(inner, kind)`: pull the diff into `ml_entities`;
//! - `create(inner, kind, data) -> id` / `update(inner, kind, id, data)` /
//!   `delete(inner, kind, id)`: write through to the server, then the store;
//! - read with [`crate::store::Store::entities`].

use crate::Result;
use crate::client::Inner;
use serde_json::Value;

/// Entity type of people.
pub(crate) const CGROUP: &str = "cgroup";
/// Entity type of location tags.
pub(crate) const LOCATION: &str = "location";

/// Pull new and changed entities of `kind` into the store.
pub(crate) async fn sync(inner: &Inner, kind: &str) -> Result<()> {
    let _ = (inner, kind);
    Ok(())
}

/// Create an entity; returns its id.
pub(crate) async fn create(inner: &Inner, kind: &str, data: &Value) -> Result<String> {
    let _ = (inner, kind, data);
    Err(crate::CoreError::storage(
        "user entities are not implemented yet",
    ))
}

/// Replace an entity's data.
pub(crate) async fn update(inner: &Inner, kind: &str, id: &str, data: &Value) -> Result<()> {
    let _ = (inner, kind, id, data);
    Err(crate::CoreError::storage(
        "user entities are not implemented yet",
    ))
}

/// Delete an entity.
pub(crate) async fn delete(inner: &Inner, kind: &str, id: &str) -> Result<()> {
    let _ = (inner, kind, id);
    Err(crate::CoreError::storage(
        "user entities are not implemented yet",
    ))
}
