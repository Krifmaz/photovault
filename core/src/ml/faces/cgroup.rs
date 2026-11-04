//! The `cgroup` user entity: a person, shared with the Ente apps.
//!
//! The JSON is Ente's exactly (web `CGroupUserEntityData` / zod
//! `RemoteCGroupData` in `user-entity/index.ts`, mobile `PersonData` in
//! `models/ml/face/person.dart`):
//!
//! ```json
//! { "name": "Ana", "assigned": [{"id": "cluster_…", "faces": ["12_0.1_…"]}],
//!   "rejectedFaceIDs": [], "manuallyAssigned": [], "isHidden": false,
//!   "avatarFaceID": "12_0.1_…", "isPinned": false, "hideFromMemories": false,
//!   "birthDate": "1990-05-17", "email": null, "userID": null }
//! ```
//!
//! [`CGroupData`] keeps the whole object and changes only the keys it is
//! asked to, so keys written by newer clients survive a rewrite (also keys
//! inside `assigned` cluster objects). Reads are as lenient as the apps'
//! parsers (`null` entries in `assigned` are skipped, `manuallyAssigned`
//! may hold numeric strings).
//!
//! Based on Ente (ente.com).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;

/// A face cluster: an id and a set of face ids (an array on the wire).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaceCluster {
    /// `cluster_<nanoid>`.
    pub id: String,
    /// Face ids (`<fileID>_<xMin>_<yMin>_<xMax>_<yMax>`).
    pub faces: Vec<String>,
}

/// Names older mobile clients used to mark an ignored person.
const LEGACY_HIDDEN_NAMES: [&str; 2] = ["(hidden)", "(ignored)"];

/// A `cgroup`'s data, kept as the raw JSON object.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CGroupData {
    raw: Map<String, Value>,
}

impl CGroupData {
    /// Wrap decrypted entity JSON (a non-object becomes an empty object).
    #[must_use]
    pub fn from_value(v: Value) -> Self {
        match v {
            Value::Object(raw) => Self { raw },
            _ => Self::default(),
        }
    }

    /// A new person for one cluster, as the web app writes it (`addCGroup`,
    /// and `ignoreCluster` with an empty name and `hidden`).
    #[must_use]
    pub fn new(name: &str, cluster: FaceCluster, hidden: bool) -> Self {
        let mut d = Self::default();
        d.raw.insert("name".into(), Value::from(name));
        d.set_assigned(vec![cluster]);
        d.raw
            .insert("rejectedFaceIDs".into(), Value::Array(Vec::new()));
        d.raw
            .insert("manuallyAssigned".into(), Value::Array(Vec::new()));
        d.raw.insert("isHidden".into(), Value::Bool(hidden));
        d.raw.insert("isPinned".into(), Value::Bool(false));
        d.raw.insert("hideFromMemories".into(), Value::Bool(false));
        d
    }

    /// The JSON to encrypt. The keys every client requires are filled in
    /// when missing (the web app's parser rejects a cgroup without a
    /// boolean `isHidden`); everything else is written back as read.
    #[must_use]
    pub fn to_value(&self) -> Value {
        let mut raw = self.raw.clone();
        if !raw.get("isHidden").is_some_and(Value::is_boolean) {
            raw.insert("isHidden".into(), Value::Bool(false));
        }
        if !raw.get("name").is_some_and(Value::is_string) {
            raw.insert("name".into(), Value::from(""));
        }
        for k in ["assigned", "rejectedFaceIDs", "manuallyAssigned"] {
            if !raw.get(k).is_some_and(Value::is_array) {
                raw.insert(k.into(), Value::Array(Vec::new()));
            }
        }
        Value::Object(raw)
    }

    /// The raw object (read only).
    #[must_use]
    pub fn raw(&self) -> &Map<String, Value> {
        &self.raw
    }

    /// The name as stored (`""` when absent).
    #[must_use]
    pub fn raw_name(&self) -> &str {
        self.raw.get("name").and_then(Value::as_str).unwrap_or("")
    }

    /// The display name: `None` when unnamed or a legacy hidden marker.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        let n = self.raw_name();
        (!n.is_empty() && !LEGACY_HIDDEN_NAMES.contains(&n)).then_some(n)
    }

    /// Set the name.
    pub fn set_name(&mut self, name: &str) {
        self.raw.insert("name".into(), Value::from(name));
    }

    /// The `isHidden` flag as stored.
    #[must_use]
    pub fn hidden_flag(&self) -> bool {
        self.raw
            .get("isHidden")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Whether the person is hidden ("ignored"): the flag, or no name, or a
    /// legacy marker name (web `reconstructPeopleState`, mobile
    /// `PersonData.isIgnored`).
    #[must_use]
    pub fn is_ignored(&self) -> bool {
        let n = self.raw_name();
        self.hidden_flag() || n.is_empty() || LEGACY_HIDDEN_NAMES.contains(&n)
    }

    /// Set `isHidden`.
    pub fn set_hidden(&mut self, hidden: bool) {
        self.raw.insert("isHidden".into(), Value::Bool(hidden));
    }

    /// `isPinned`.
    #[must_use]
    pub fn is_pinned(&self) -> bool {
        self.raw
            .get("isPinned")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// `hideFromMemories`.
    #[must_use]
    pub fn hide_from_memories(&self) -> bool {
        self.raw
            .get("hideFromMemories")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Assigned clusters (entries that are not `{id, faces}` objects are
    /// skipped, like the apps do).
    #[must_use]
    pub fn assigned(&self) -> Vec<FaceCluster> {
        let Some(Value::Array(a)) = self.raw.get("assigned") else {
            return Vec::new();
        };
        a.iter()
            .filter_map(|c| {
                let id = c.get("id")?.as_str()?.to_string();
                let faces = c
                    .get("faces")?
                    .as_array()?
                    .iter()
                    .filter_map(|f| f.as_str().map(str::to_string))
                    .collect();
                Some(FaceCluster { id, faces })
            })
            .collect()
    }

    /// Replace the assigned clusters. Other keys of a cluster object with
    /// the same id are kept.
    pub fn set_assigned(&mut self, clusters: Vec<FaceCluster>) {
        let mut extra: HashMap<String, Map<String, Value>> = HashMap::new();
        if let Some(Value::Array(a)) = self.raw.get("assigned") {
            for c in a {
                if let (Some(obj), Some(id)) = (c.as_object(), c.get("id").and_then(Value::as_str))
                    && obj.len() > 2
                {
                    extra.insert(id.to_string(), obj.clone());
                }
            }
        }
        let arr = clusters
            .into_iter()
            .map(|c| {
                let mut obj = extra.remove(&c.id).unwrap_or_default();
                obj.insert("id".into(), Value::from(c.id));
                obj.insert(
                    "faces".into(),
                    Value::Array(c.faces.into_iter().map(Value::from).collect()),
                );
                Value::Object(obj)
            })
            .collect();
        self.raw.insert("assigned".into(), Value::Array(arr));
    }

    /// `rejectedFaceIDs`.
    #[must_use]
    pub fn rejected_face_ids(&self) -> Vec<String> {
        string_array(self.raw.get("rejectedFaceIDs"))
    }

    /// Replace `rejectedFaceIDs`.
    pub fn set_rejected_face_ids(&mut self, ids: Vec<String>) {
        self.raw.insert(
            "rejectedFaceIDs".into(),
            Value::Array(ids.into_iter().map(Value::from).collect()),
        );
    }

    /// `manuallyAssigned` file ids (numbers, or numeric strings like the
    /// mobile parser accepts).
    #[must_use]
    pub fn manually_assigned(&self) -> Vec<i64> {
        let Some(Value::Array(a)) = self.raw.get("manuallyAssigned") else {
            return Vec::new();
        };
        a.iter()
            .filter_map(|v| match v {
                Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
                Value::String(s) => s.trim().parse().ok(),
                _ => None,
            })
            .collect()
    }

    /// Replace `manuallyAssigned`.
    pub fn set_manually_assigned(&mut self, ids: Vec<i64>) {
        self.raw.insert(
            "manuallyAssigned".into(),
            Value::Array(ids.into_iter().map(Value::from).collect()),
        );
    }

    /// The avatar face, if set to a face id (older mobile clients stored a
    /// file id here, which is ignored like the web app does).
    #[must_use]
    pub fn avatar_face_id(&self) -> Option<&str> {
        self.raw
            .get("avatarFaceID")
            .and_then(Value::as_str)
            .filter(|s| s.split('_').count() > 1)
    }

    /// Set the avatar face.
    pub fn set_avatar_face_id(&mut self, face_id: &str) {
        self.raw.insert("avatarFaceID".into(), Value::from(face_id));
    }

    /// `birthDate` (`yyyy-MM-dd`).
    #[must_use]
    pub fn birth_date(&self) -> Option<&str> {
        self.raw
            .get("birthDate")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    }

    /// Set or clear `birthDate` (cleared as `null`, like the mobile app).
    pub fn set_birth_date(&mut self, date: Option<&str>) {
        self.raw
            .insert("birthDate".into(), date.map_or(Value::Null, Value::from));
    }

    /// Assigned clusters without the rejected faces.
    #[must_use]
    pub fn effective_clusters(&self) -> Vec<FaceCluster> {
        let rejected = self.rejected_face_ids();
        let mut assigned = self.assigned();
        if !rejected.is_empty() {
            let rejected: std::collections::HashSet<&str> =
                rejected.iter().map(String::as_str).collect();
            for c in &mut assigned {
                c.faces.retain(|f| !rejected.contains(f.as_str()));
            }
        }
        assigned
    }
}

fn string_array(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// Whether `s` is a `yyyy-MM-dd` date.
#[must_use]
pub fn is_valid_birth_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() && s.len() == 10
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_keys_survive_and_required_keys_are_filled() {
        let mut d = CGroupData::from_value(json!({
            "name": "Ana",
            "assigned": [null, {"id": "c1", "faces": ["1_a", "2_b"], "extra": 7}],
            "futureKey": {"a": [1, 2]},
        }));
        assert_eq!(d.assigned().len(), 1);
        d.set_assigned(vec![FaceCluster {
            id: "c1".into(),
            faces: vec!["1_a".into()],
        }]);
        let v = d.to_value();
        assert_eq!(v["futureKey"], json!({"a": [1, 2]}));
        assert_eq!(v["assigned"][0]["extra"], 7);
        assert_eq!(v["isHidden"], false);
        assert_eq!(v["rejectedFaceIDs"], json!([]));
    }

    #[test]
    fn hidden_and_legacy_names() {
        let d = CGroupData::from_value(json!({"name": "(ignored)", "isHidden": false}));
        assert!(d.is_ignored());
        assert_eq!(d.name(), None);
        let d = CGroupData::from_value(json!({"name": "Bo", "avatarFaceID": "123"}));
        assert!(!d.is_ignored());
        assert_eq!(d.avatar_face_id(), None);
        assert!(is_valid_birth_date("1990-05-17"));
        assert!(!is_valid_birth_date("1990-5-17"));
    }
}
