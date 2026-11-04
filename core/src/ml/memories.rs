//! Memories: Ente's smart memories (on this day, people, trips, time,
//! CLIP themes, fillers), computed on device.
//!
//! STUB (contract v1.2): owned by the `core-memories` track.

use super::Memory;
use crate::client::Inner;
use crate::{Core, CoreError};

/// Memories computed for a day, rebuilt lazily after [`invalidate`].
#[derive(Debug, Default)]
pub(crate) struct MemoriesCache {}

/// Drop the cache (library, people or `mldata` changed).
pub(crate) fn invalidate(inner: &Inner) {
    let _ = inner;
}

#[uniffi::export]
impl Core {
    /// Today's memories for the user's local time (`now_us` µs since the
    /// epoch, `utc_offset_secs` the local offset), unseen first.
    pub fn memories(&self, now_us: i64, utc_offset_secs: i32) -> Vec<Memory> {
        let _ = (now_us, utc_offset_secs);
        Vec::new()
    }

    /// Mark a memory as seen.
    pub fn mark_memory_seen(&self, memory_id: String) -> Result<(), CoreError> {
        let _ = memory_id;
        Ok(())
    }
}
