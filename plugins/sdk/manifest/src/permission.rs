//! The permission model.
//!
//! A plugin lists every permission it needs in its manifest. The user approves
//! them on install, can see them later, and can revoke any of them at any time.
//! The host checks the granted set on **every** host call; a call outside the
//! granted set fails with [`crate::abi::ErrorCode::PermissionDenied`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

/// Something a plugin may read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadScope {
    /// Decrypted file and album metadata: names, dates, GPS, type, captions,
    /// favourite and archive state, album membership.
    Metadata,
    /// Decrypted thumbnails.
    Thumbnails,
    /// Decrypted original files (photos and videos).
    Originals,
}

/// Something a plugin may change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteScope {
    /// Add tags to files.
    Tags,
    /// Create albums the user owns and add or remove files in them.
    Albums,
    /// Move files to the trash (recoverable, as in Ente).
    Trash,
    /// Write copies of files into the export folder the user picked for this
    /// plugin. The plugin never sees the real path.
    Export,
}

/// One permission, as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Permission {
    /// Read access.
    Read(ReadScope),
    /// Write access.
    Write(WriteScope),
    /// Outbound HTTP(S) to one host pattern (`api.example.com` or
    /// `*.example.com`).
    Network(String),
}

impl Permission {
    /// A short sentence for the install and permissions screens.
    pub fn describe(&self) -> String {
        match self {
            Permission::Read(ReadScope::Metadata) => {
                "Read photo and album details (names, dates, locations, captions)".into()
            }
            Permission::Read(ReadScope::Thumbnails) => "View thumbnails".into(),
            Permission::Read(ReadScope::Originals) => "View original photos and videos".into(),
            Permission::Write(WriteScope::Tags) => "Add tags to photos".into(),
            Permission::Write(WriteScope::Albums) => "Create albums and change your albums".into(),
            Permission::Write(WriteScope::Trash) => "Move photos to the trash".into(),
            Permission::Write(WriteScope::Export) => {
                "Save copies to the export folder you pick".into()
            }
            Permission::Network(host) => format!("Connect to {host}"),
        }
    }

    /// Whether this permission lets data leave the device or destroys data, so
    /// the app should highlight it on the install screen.
    pub fn is_sensitive(&self) -> bool {
        matches!(
            self,
            Permission::Network(_)
                | Permission::Read(ReadScope::Originals)
                | Permission::Write(WriteScope::Trash)
        )
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Permission::Read(s) => write!(f, "read:{}", scope_name(s)),
            Permission::Write(s) => write!(f, "write:{}", scope_name(s)),
            Permission::Network(h) => write!(f, "network:{h}"),
        }
    }
}

fn scope_name<T: Serialize>(s: &T) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The `[permissions]` table of a manifest, and also the set a user granted.
///
/// ```toml
/// [permissions]
/// read = ["metadata", "thumbnails"]
/// write = ["albums"]
/// network = ["api.example.com"]
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionSet {
    /// Read scopes.
    #[serde(default)]
    pub read: BTreeSet<ReadScope>,
    /// Write scopes.
    #[serde(default)]
    pub write: BTreeSet<WriteScope>,
    /// Host patterns for outbound HTTP(S).
    #[serde(default)]
    pub network: BTreeSet<String>,
}

impl PermissionSet {
    /// An empty set (nothing granted).
    pub fn none() -> Self {
        Self::default()
    }

    /// Flatten into individual permissions, in a stable order.
    pub fn iter(&self) -> impl Iterator<Item = Permission> + '_ {
        self.read
            .iter()
            .map(|s| Permission::Read(*s))
            .chain(self.write.iter().map(|s| Permission::Write(*s)))
            .chain(self.network.iter().cloned().map(Permission::Network))
    }

    /// Build from individual permissions.
    pub fn from_permissions<I: IntoIterator<Item = Permission>>(it: I) -> Self {
        let mut set = Self::default();
        for p in it {
            set.insert(p);
        }
        set
    }

    /// Add one permission.
    pub fn insert(&mut self, p: Permission) {
        match p {
            Permission::Read(s) => {
                self.read.insert(s);
            }
            Permission::Write(s) => {
                self.write.insert(s);
            }
            Permission::Network(h) => {
                self.network.insert(h);
            }
        }
    }

    /// Remove one permission. Returns whether it was present.
    pub fn remove(&mut self, p: &Permission) -> bool {
        match p {
            Permission::Read(s) => self.read.remove(s),
            Permission::Write(s) => self.write.remove(s),
            Permission::Network(h) => self.network.remove(h),
        }
    }

    /// Whether `p` is in the set.
    pub fn contains(&self, p: &Permission) -> bool {
        match p {
            Permission::Read(s) => self.read.contains(s),
            Permission::Write(s) => self.write.contains(s),
            Permission::Network(h) => self.network.contains(h),
        }
    }

    /// Whether the set grants reading `scope`.
    pub fn can_read(&self, scope: ReadScope) -> bool {
        self.read.contains(&scope)
    }

    /// Whether the set grants writing `scope`.
    pub fn can_write(&self, scope: WriteScope) -> bool {
        self.write.contains(&scope)
    }

    /// Whether every permission in `self` is also in `other`.
    pub fn is_subset(&self, other: &PermissionSet) -> bool {
        self.read.is_subset(&other.read)
            && self.write.is_subset(&other.write)
            && self.network.is_subset(&other.network)
    }

    /// Permissions in `self` that are not in `other`.
    pub fn difference(&self, other: &PermissionSet) -> PermissionSet {
        PermissionSet {
            read: self.read.difference(&other.read).copied().collect(),
            write: self.write.difference(&other.write).copied().collect(),
            network: self.network.difference(&other.network).cloned().collect(),
        }
    }

    /// Permissions in both sets.
    pub fn intersection(&self, other: &PermissionSet) -> PermissionSet {
        PermissionSet {
            read: self.read.intersection(&other.read).copied().collect(),
            write: self.write.intersection(&other.write).copied().collect(),
            network: self.network.intersection(&other.network).cloned().collect(),
        }
    }

    /// Whether nothing is in the set.
    pub fn is_empty(&self) -> bool {
        self.read.is_empty() && self.write.is_empty() && self.network.is_empty()
    }
}

/// Whether `pattern` is an acceptable network host pattern: a lowercase DNS
/// name, optionally starting with `*.`. Bare `*`, IP-literal wildcards, ports,
/// schemes and paths are rejected so a plugin cannot ask for "the whole
/// internet".
pub fn valid_host_pattern(pattern: &str) -> bool {
    let name = pattern.strip_prefix("*.").unwrap_or(pattern);
    if name.is_empty() || name.len() > 253 || !name.contains('.') {
        return false;
    }
    name.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_ops() {
        let a = PermissionSet::from_permissions([
            Permission::Read(ReadScope::Metadata),
            Permission::Write(WriteScope::Albums),
            Permission::Network("api.example.com".into()),
        ]);
        let b = PermissionSet::from_permissions([Permission::Read(ReadScope::Metadata)]);
        assert!(b.is_subset(&a));
        assert!(!a.is_subset(&b));
        assert_eq!(a.difference(&b).iter().count(), 2);
        assert_eq!(a.intersection(&b), b);
        assert_eq!(
            a.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            ["read:metadata", "write:albums", "network:api.example.com"]
        );
    }

    #[test]
    fn host_patterns() {
        for ok in ["api.example.com", "*.example.com", "a-b.c0.io"] {
            assert!(valid_host_pattern(ok), "{ok}");
        }
        for bad in [
            "*",
            "*.com",
            "localhost",
            "https://x.com",
            "x.com:443",
            "x.com/p",
            "X.com",
            "-a.com",
            "",
        ] {
            assert!(!valid_host_pattern(bad), "{bad}");
        }
    }
}
