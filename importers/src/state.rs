//! Resume journal: an append-only JSON-lines file recording which source
//! items were imported, so an interrupted import can simply be run again.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// One journal line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    /// Source name ("immich", "icloud").
    pub source: String,
    /// The item's [`crate::item::ImportItem::source_id`].
    pub source_id: String,
    /// Ente file id it ended up as.
    pub file_id: i64,
}

/// The journal of completed items for one source.
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    source: String,
    done: HashSet<String>,
}

impl Journal {
    /// Open (or create) the journal at `path` and load entries for `source`.
    /// Malformed lines (e.g. a torn last write) are ignored.
    pub fn open(path: &Path, source: &str) -> anyhow::Result<Self> {
        let mut done = HashSet::new();
        if path.exists() {
            let f = std::fs::File::open(path)?;
            for line in std::io::BufReader::new(f).lines() {
                let line = line?;
                if let Ok(e) = serde_json::from_str::<Entry>(&line)
                    && e.source == source
                {
                    done.insert(e.source_id);
                }
            }
        }
        Ok(Self {
            path: path.to_path_buf(),
            source: source.to_string(),
            done,
        })
    }

    /// Whether `source_id` was already imported.
    pub fn contains(&self, source_id: &str) -> bool {
        self.done.contains(source_id)
    }

    /// Number of completed items.
    pub fn len(&self) -> usize {
        self.done.len()
    }

    /// True if nothing was imported yet.
    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }

    /// Record a completed item and flush it to disk.
    pub fn record(&mut self, source_id: &str, file_id: i64) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent()
            && !dir.as_os_str().is_empty()
        {
            std::fs::create_dir_all(dir)?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let entry = Entry {
            source: self.source.clone(),
            source_id: source_id.to_string(),
            file_id,
        };
        writeln!(f, "{}", serde_json::to_string(&entry)?)?;
        f.sync_data()?;
        self.done.insert(source_id.to_string());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_survive_reopen_and_ignore_other_sources() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("j.jsonl");
        let mut j = Journal::open(&p, "immich").unwrap();
        assert!(j.is_empty());
        j.record("a", 1).unwrap();
        let mut k = Journal::open(&p, "icloud").unwrap();
        k.record("b", 2).unwrap();
        // Torn trailing line must not break loading.
        std::fs::OpenOptions::new()
            .append(true)
            .open(&p)
            .unwrap()
            .write_all(b"{\"source\":\"imm")
            .unwrap();
        let j = Journal::open(&p, "immich").unwrap();
        assert!(j.contains("a"));
        assert!(!j.contains("b"));
        assert_eq!(j.len(), 1);
    }
}
