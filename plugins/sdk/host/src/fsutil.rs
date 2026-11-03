//! Small file-system helpers: atomic writes and export path sanitisation.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A temporary path next to `path`, unique within this process.
fn temp_path(path: &Path) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.tmp-{}-{n}", std::process::id()))
}

/// Create a new temporary file next to `path`, let `fill` write it, fsync it
/// and rename it over `path`. On any error the temporary file is removed.
pub(crate) fn write_atomic_with<T>(
    path: &Path,
    fill: impl FnOnce(&mut File) -> io::Result<T>,
) -> io::Result<T> {
    let tmp = temp_path(path);
    let result = (|| {
        let mut f = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        let v = fill(&mut f)?;
        f.flush()?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)?;
        Ok(v)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Atomically replace `path` with `bytes` (write temp file, then rename).
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with(path, |f| f.write_all(bytes))
}

/// Why an export path was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("export path {0}")]
pub struct BadPath(pub &'static str);

/// Longest accepted export path, in bytes.
pub const MAX_EXPORT_PATH_BYTES: usize = 1024;

/// Split a plugin-supplied export path into safe components.
///
/// Accepts only relative, `/`-separated paths whose components are non-empty
/// file names: no `..` or `.`, no leading `/`, no empty components, no NUL,
/// backslash, colon or other control characters, and at most 255 bytes per
/// component.
pub fn sanitize_export_path(path: &str) -> Result<Vec<String>, BadPath> {
    if path.is_empty() {
        return Err(BadPath("is empty"));
    }
    if path.len() > MAX_EXPORT_PATH_BYTES {
        return Err(BadPath("is too long"));
    }
    if path.starts_with('/') {
        return Err(BadPath("must be relative"));
    }
    if path
        .chars()
        .any(|c| c == '\\' || c == ':' || c.is_control())
    {
        return Err(BadPath("contains a forbidden character"));
    }
    let mut out = Vec::new();
    for comp in path.split('/') {
        if comp.is_empty() {
            return Err(BadPath("has an empty component"));
        }
        if comp == "." || comp == ".." {
            return Err(BadPath("must not contain `.` or `..`"));
        }
        if comp.len() > 255 {
            return Err(BadPath("has a component longer than 255 bytes"));
        }
        // Belt and braces: the platform must see exactly one normal component.
        let mut cs = Path::new(comp).components();
        if !matches!((cs.next(), cs.next()), (Some(Component::Normal(_)), None)) {
            return Err(BadPath("has an unsafe component"));
        }
        out.push(comp.to_owned());
    }
    Ok(out)
}

/// Resolve sanitised `components` under the export `root`, creating parent
/// directories. Refuses to follow a symbolic link anywhere below `root`, and
/// refuses if the target exists as anything other than a regular file.
/// Returns the final path and whether it already exists.
pub(crate) fn prepare_export_target(
    root: &Path,
    components: &[String],
) -> Result<(PathBuf, bool), BadPath> {
    let root = fs::canonicalize(root).map_err(|_| BadPath("root does not exist"))?;
    if !root.is_dir() {
        return Err(BadPath("root is not a folder"));
    }
    let (file, dirs) = components.split_last().ok_or(BadPath("is empty"))?;
    let mut cur = root;
    for d in dirs {
        cur.push(d);
        match fs::symlink_metadata(&cur) {
            Ok(m) if m.file_type().is_symlink() => return Err(BadPath("goes through a symlink")),
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(BadPath("goes through a file")),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&cur).map_err(|_| BadPath("could not create a folder"))?;
            }
            Err(_) => return Err(BadPath("could not be checked")),
        }
    }
    cur.push(file);
    let exists = match fs::symlink_metadata(&cur) {
        Ok(m) if m.file_type().is_symlink() => return Err(BadPath("is a symlink")),
        Ok(m) if m.is_file() => true,
        Ok(_) => return Err(BadPath("is not a regular file")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(_) => return Err(BadPath("could not be checked")),
    };
    Ok((cur, exists))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize() {
        assert_eq!(
            sanitize_export_path("Album/2024/IMG_1.jpg").unwrap(),
            ["Album", "2024", "IMG_1.jpg"]
        );
        assert!(sanitize_export_path("a..b/c").is_ok());
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../../x",
            "a/./b",
            ".",
            "a//b",
            "a/",
            "a\\b",
            "C:x",
            "a\0b",
            "a\nb",
        ] {
            assert!(sanitize_export_path(bad).is_err(), "{bad:?} accepted");
        }
        assert!(sanitize_export_path(&"a".repeat(256)).is_err());
        assert!(sanitize_export_path(&"a/".repeat(600)).is_err());
    }

    #[test]
    fn atomic_write_replaces() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.json");
        write_atomic(&p, b"1").unwrap();
        write_atomic(&p, b"22").unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"22");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1);
    }
}
