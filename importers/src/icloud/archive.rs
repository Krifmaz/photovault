//! Uniform access to the parts of an iCloud export: zip files read in place
//! and directories the user already extracted.
//!
//! Zips are never extracted wholesale. The central directory is read once
//! when the source opens; afterwards single entries are copied to a temp
//! file only when the item that needs them is about to be uploaded.

use anyhow::Context as _;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One input the user named: a zip or a directory.
#[derive(Clone)]
pub(crate) enum Container {
    /// An extracted export (or a part of one) on disk.
    Dir,
    /// A zip read in place. Shared so blocking tasks can extract from it.
    Zip {
        /// Path of the zip file, for messages.
        path: PathBuf,
        /// The opened archive.
        archive: Arc<Mutex<zip::ZipArchive<File>>>,
    },
}

/// A file inside a container.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    /// Index of the container in the source's container list.
    pub container: usize,
    /// For zips the entry index; unused for directories.
    pub zip_index: usize,
    /// Path inside the container with `/` separators (zip entry name, or
    /// path relative to the directory).
    pub inner: String,
    /// `inner` without a leading `iCloud Photos Part N of M/` component, so
    /// the same logical folder in different parts compares equal.
    pub logical: String,
    /// Absolute path on disk (directories only).
    pub disk_path: Option<PathBuf>,
    /// Modification time, µs since the epoch, if known.
    pub mtime_us: Option<i64>,
    /// Uncompressed size in bytes.
    pub size: u64,
}

impl Entry {
    /// Final path component.
    pub fn file_name(&self) -> &str {
        self.inner.rsplit('/').next().unwrap_or(&self.inner)
    }

    /// Parent of `inner` (physical folder; distinct for every part).
    pub fn physical_dir(&self) -> String {
        let dir = self.inner.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        format!("{}:{}", self.container, dir)
    }

    /// Parent of `logical` (the same for a folder across parts).
    pub fn logical_dir(&self) -> &str {
        self.logical.rsplit_once('/').map(|(d, _)| d).unwrap_or("")
    }
}

/// Is `component` the root folder Apple puts in each part,
/// `iCloud Photos Part 3 of 12` (also tolerating `_`/`-` and case)?
pub(crate) fn is_part_folder(component: &str) -> bool {
    let c = component.to_ascii_lowercase().replace(['_', '-'], " ");
    let Some(rest) = c.strip_prefix("icloud photos part ") else {
        return false;
    };
    let mut it = rest.split(" of ");
    matches!(
        (it.next(), it.next(), it.next()),
        (Some(a), Some(b), None)
            if !a.is_empty() && !b.is_empty()
                && a.trim().chars().all(|ch| ch.is_ascii_digit())
                && b.trim().chars().all(|ch| ch.is_ascii_digit())
    )
}

/// Strip a leading part folder (and, for a zip named after a part, nothing
/// else) from a `/`-separated path.
pub(crate) fn logical_path(inner: &str) -> String {
    let mut parts: Vec<&str> = inner.split('/').filter(|c| !c.is_empty()).collect();
    while parts.len() > 1 && is_part_folder(parts[0]) {
        parts.remove(0);
    }
    parts.join("/")
}

/// Is this file a zip (by extension or signature)?
fn looks_like_zip(path: &Path) -> bool {
    if path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
    {
        return true;
    }
    let mut head = [0u8; 4];
    File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head == *b"PK\x03\x04"
}

/// Open every input and list its files.
pub(crate) fn open_all(inputs: &[PathBuf]) -> anyhow::Result<(Vec<Container>, Vec<Entry>)> {
    let mut containers = Vec::new();
    let mut entries = Vec::new();
    for input in inputs {
        let idx = containers.len();
        let meta = std::fs::metadata(input)
            .with_context(|| format!("cannot read input {}", input.display()))?;
        if meta.is_dir() {
            list_dir(idx, input, &mut entries)?;
            containers.push(Container::Dir);
        } else if looks_like_zip(input) {
            let f = File::open(input)?;
            let mut archive = zip::ZipArchive::new(f).with_context(|| {
                format!(
                    "{} is not a readable zip (incomplete download?)",
                    input.display()
                )
            })?;
            list_zip(idx, &mut archive, &mut entries);
            containers.push(Container::Zip {
                path: input.clone(),
                archive: Arc::new(Mutex::new(archive)),
            });
        } else {
            anyhow::bail!(
                "{} is neither a directory nor a zip of an iCloud Photos export",
                input.display()
            );
        }
    }
    Ok((containers, entries))
}

fn list_dir(idx: usize, root: &Path, out: &mut Vec<Entry>) -> anyhow::Result<()> {
    for e in walkdir::WalkDir::new(root).follow_links(false) {
        let e = e.with_context(|| format!("listing {}", root.display()))?;
        if !e.file_type().is_file() {
            continue;
        }
        let Ok(rel) = e.path().strip_prefix(root) else {
            continue;
        };
        let inner = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        let meta = e.metadata().ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime_us = meta
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_micros() as i64);
        out.push(Entry {
            container: idx,
            zip_index: 0,
            logical: logical_path(&inner),
            inner,
            disk_path: Some(e.path().to_path_buf()),
            mtime_us,
            size,
        });
    }
    Ok(())
}

fn list_zip(idx: usize, archive: &mut zip::ZipArchive<File>, out: &mut Vec<Entry>) {
    for i in 0..archive.len() {
        let Ok(f) = archive.by_index(i) else { continue };
        if f.is_dir() || f.is_symlink() {
            continue;
        }
        // Refuse names that would escape a directory (`..`, absolute paths).
        if f.enclosed_name().is_none() {
            continue;
        }
        let Ok(name) = f.name() else { continue };
        let inner = name.replace('\\', "/");
        let mtime_us = f.last_modified().and_then(zip_time_us);
        let size = f.size();
        out.push(Entry {
            container: idx,
            zip_index: i,
            logical: logical_path(&inner),
            inner,
            disk_path: None,
            mtime_us,
            size,
        });
    }
}

/// Zip timestamps have no zone; like other tools we read them as local time.
fn zip_time_us(t: zip::DateTime) -> Option<i64> {
    use chrono::TimeZone as _;
    let naive = chrono::NaiveDate::from_ymd_opt(t.year() as i32, t.month() as u32, t.day() as u32)?
        .and_hms_opt(t.hour() as u32, t.minute() as u32, t.second() as u32)?;
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|d| d.timestamp_micros())
}

/// Read a small entry (CSV, JSON) fully into memory, up to `limit` bytes.
pub(crate) fn read_small(
    containers: &[Container],
    entry: &Entry,
    limit: u64,
) -> anyhow::Result<Vec<u8>> {
    let mut buf = Vec::new();
    match &containers[entry.container] {
        Container::Dir => {
            let p = entry.disk_path.as_ref().context("missing disk path")?;
            File::open(p)?.take(limit).read_to_end(&mut buf)?;
        }
        Container::Zip { archive, path } => {
            let mut a = archive
                .lock()
                .map_err(|_| anyhow::anyhow!("zip lock poisoned"))?;
            let f = a
                .by_index(entry.zip_index)
                .with_context(|| format!("{}: {}", path.display(), entry.inner))?;
            f.take(limit).read_to_end(&mut buf)?;
        }
    }
    Ok(buf)
}

/// Make the bytes of `entry` available as a file. Directory entries are used
/// in place (returns `(path, false)`); zip entries are copied to
/// `dest_dir/<file name>` (returns `(path, true)`: a temp file to delete).
pub(crate) fn materialize(
    container: &Container,
    entry: &Entry,
    dest_dir: &Path,
) -> anyhow::Result<(PathBuf, bool)> {
    match container {
        Container::Dir => Ok((entry.disk_path.clone().context("missing disk path")?, false)),
        Container::Zip { archive, path } => {
            std::fs::create_dir_all(dest_dir)?;
            let name = sanitize_file_name(entry.file_name());
            let out_path = dest_dir.join(name);
            let mut a = archive
                .lock()
                .map_err(|_| anyhow::anyhow!("zip lock poisoned"))?;
            let mut f = a
                .by_index(entry.zip_index)
                .with_context(|| format!("{}: {}", path.display(), entry.inner))?;
            let mut out = File::create(&out_path)?;
            std::io::copy(&mut f, &mut out)
                .with_context(|| format!("extracting {} from {}", entry.inner, path.display()))?;
            Ok((out_path, true))
        }
    }
}

/// A file name that is safe as a single path component.
fn sanitize_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' | ':' => '_',
            c => c,
        })
        .collect();
    match s.as_str() {
        "" | "." | ".." => "file".to_string(),
        _ => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_folders() {
        assert!(is_part_folder("iCloud Photos Part 1 of 19"));
        assert!(is_part_folder(
            "icloud_photos_part_2_of_3".replace('_', " ").as_str()
        ));
        assert!(!is_part_folder("Photos"));
        assert!(!is_part_folder("iCloud Photos Part x of 2"));
        assert_eq!(
            logical_path("iCloud Photos Part 2 of 3/Photos/IMG_1.JPG"),
            "Photos/IMG_1.JPG"
        );
        assert_eq!(logical_path("Photos/IMG_1.JPG"), "Photos/IMG_1.JPG");
    }
}
