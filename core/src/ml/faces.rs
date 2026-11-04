//! Faces and people: Ente's face clustering, people as `cgroup` entities
//! (shared with the Ente apps), face crops.
//!
//! STUB (contract v1.2): owned by the `core-faces` track.
//!
//! Internal API for `search` and `memories`: [`rebuild`], [`people`],
//! [`person_file_ids`].

use super::{FaceRef, FileFace, Person};
use crate::client::Inner;
use crate::{Core, CoreError, Result, TimelineItem};

/// Clusters and people, rebuilt by [`rebuild`].
#[derive(Debug, Default)]
pub(crate) struct FacesCache {}

/// Recluster and rebuild people after `mldata` or `cgroup` changes.
pub(crate) fn rebuild(inner: &Inner) -> Result<()> {
    let _ = inner;
    Ok(())
}

/// People (named first, then unnamed clusters with at least
/// `min_files` files), hidden ones only if `include_hidden`.
pub(crate) fn people(inner: &Inner, include_hidden: bool, min_files: u32) -> Vec<Person> {
    let _ = (inner, include_hidden, min_files);
    Vec::new()
}

/// Files of a person or cluster, newest first.
pub(crate) fn person_file_ids(inner: &Inner, person_id: &str) -> Vec<i64> {
    let _ = (inner, person_id);
    Vec::new()
}

#[uniffi::export]
impl Core {
    /// People and unnamed clusters with at least `min_files` files, named
    /// first (most files first), hidden ones only with `include_hidden`.
    pub fn people(&self, include_hidden: bool, min_files: u32) -> Vec<Person> {
        let _ = (include_hidden, min_files);
        Vec::new()
    }

    /// One person or cluster.
    pub fn person(&self, person_id: String) -> Option<Person> {
        let _ = person_id;
        None
    }

    /// Files of a person or cluster, newest first.
    pub fn person_files(&self, person_id: String) -> Vec<TimelineItem> {
        let _ = person_id;
        Vec::new()
    }

    /// Faces found on a file, with who they are.
    pub fn file_faces(&self, file_id: i64) -> Vec<FileFace> {
        let _ = file_id;
        Vec::new()
    }

    /// Unnamed clusters that look like this person, best first
    /// ("Review suggestions").
    pub fn person_suggestions(&self, person_id: String) -> Vec<Person> {
        let _ = person_id;
        Vec::new()
    }

    /// A square JPEG crop of a face (cached, encrypted at rest).
    pub async fn face_thumbnail(&self, face_id: String) -> Result<Vec<u8>, CoreError> {
        let _ = face_id;
        Err(CoreError::storage("faces are not implemented yet"))
    }

    /// Name a cluster (creates a `cgroup`) or rename a person.
    pub async fn name_person(&self, person_id: String, name: String) -> Result<Person, CoreError> {
        let _ = (person_id, name);
        Err(CoreError::storage("faces are not implemented yet"))
    }

    /// Merge `from_id` (person or cluster) into `into_id`.
    pub async fn merge_people(
        &self,
        into_id: String,
        from_id: String,
    ) -> Result<Person, CoreError> {
        let _ = (into_id, from_id);
        Err(CoreError::storage("faces are not implemented yet"))
    }

    /// Accept or reject a suggested cluster for a person.
    pub async fn answer_suggestion(
        &self,
        person_id: String,
        cluster_id: String,
        accept: bool,
    ) -> Result<(), CoreError> {
        let _ = (person_id, cluster_id, accept);
        Err(CoreError::storage("faces are not implemented yet"))
    }

    /// Hide or unhide a person (Ente's "ignore").
    pub async fn set_person_hidden(
        &self,
        person_id: String,
        hidden: bool,
    ) -> Result<(), CoreError> {
        let _ = (person_id, hidden);
        Err(CoreError::storage("faces are not implemented yet"))
    }

    /// Choose a person's avatar face.
    pub async fn set_person_cover(
        &self,
        person_id: String,
        face_id: String,
    ) -> Result<(), CoreError> {
        let _ = (person_id, face_id);
        Err(CoreError::storage("faces are not implemented yet"))
    }

    /// Set or clear a person's birth date (`YYYY-MM-DD`).
    pub async fn set_person_birth_date(
        &self,
        person_id: String,
        birth_date: Option<String>,
    ) -> Result<(), CoreError> {
        let _ = (person_id, birth_date);
        Err(CoreError::storage("faces are not implemented yet"))
    }

    /// Remove faces that are not this person (they are rejected for it).
    pub async fn remove_faces_from_person(
        &self,
        person_id: String,
        face_ids: Vec<String>,
    ) -> Result<(), CoreError> {
        let _ = (person_id, face_ids);
        Err(CoreError::storage("faces are not implemented yet"))
    }

    /// Delete a person (its `cgroup`); the faces become unnamed clusters.
    pub async fn delete_person(&self, person_id: String) -> Result<(), CoreError> {
        let _ = person_id;
        Err(CoreError::storage("faces are not implemented yet"))
    }
}

#[allow(dead_code)]
fn _face_ref_is_used(_: FaceRef) {}
