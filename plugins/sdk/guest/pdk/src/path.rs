//! Safe relative paths for [`crate::host::export_file`].
//!
//! The host validates every export path too, but a plugin that builds paths
//! from album names and file titles (which users and other people's shared
//! albums control) should clean them itself so that the result is predictable
//! on every operating system.

/// Longest component [`sanitize_component`] returns, in characters.
pub const MAX_COMPONENT_CHARS: usize = 120;

/// Turn any string into one safe path component.
///
/// * `/ \ : * ? " < > |` and control characters become `_`;
/// * leading and trailing spaces and trailing dots are removed (Windows drops
///   them silently);
/// * empty, `.` and `..` become `_`;
/// * Windows device names (`CON`, `NUL`, `COM1`, ...) get a `_` prefix;
/// * the result is cut to [`MAX_COMPONENT_CHARS`] characters, keeping a short
///   extension.
///
/// ```
/// use photovault_plugin_pdk::path::sanitize_component;
/// assert_eq!(sanitize_component("../Trip: Rome/2024"), "__Trip_ Rome_2024");
/// assert_eq!(sanitize_component("CON.txt"), "_CON.txt");
/// ```
pub fn sanitize_component(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let mut s = cleaned.trim().trim_end_matches(['.', ' ']).to_owned();
    // A leading run of dots would make `..foo` or a hidden file.
    if s.starts_with('.') {
        s = format!("_{}", s.trim_start_matches('.'));
    }
    if s.is_empty() {
        return "_".to_owned();
    }
    if is_windows_device(&s) {
        s.insert(0, '_');
    }
    truncate_keeping_extension(&s, MAX_COMPONENT_CHARS)
}

fn is_windows_device(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
}

fn truncate_keeping_extension(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let (stem, ext) = split_extension(s);
    let ext_len = ext.chars().count();
    if ext_len > 0 && ext_len < 16 {
        let keep = max - ext_len;
        let stem: String = stem.chars().take(keep).collect();
        format!("{stem}{ext}")
    } else {
        s.chars().take(max).collect()
    }
}

/// Split `name` into stem and extension, the extension including its dot
/// (`"a.tar.gz"` → `("a.tar", ".gz")`, `"README"` → `("README", "")`).
/// A leading dot is not an extension.
pub fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => name.split_at(i),
        _ => (name, ""),
    }
}

/// Join already-sanitised components with `/`.
pub fn join<S: AsRef<str>>(components: &[S]) -> String {
    components
        .iter()
        .map(AsRef::as_ref)
        .collect::<Vec<_>>()
        .join("/")
}

/// Whether `path` is acceptable to [`crate::host::export_file`]: relative,
/// `/`-separated, no empty, `.` or `..` components, no backslashes or NULs.
pub fn is_safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains(['\\', '\0'])
        && path
            .split('/')
            .all(|c| !c.is_empty() && c != "." && c != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn components() {
        assert_eq!(sanitize_component("Holidays 2024"), "Holidays 2024");
        assert_eq!(sanitize_component(".."), "_");
        assert_eq!(sanitize_component("."), "_");
        assert_eq!(sanitize_component(""), "_");
        assert_eq!(sanitize_component("   "), "_");
        assert_eq!(sanitize_component(".hidden"), "_hidden");
        assert_eq!(sanitize_component("a\u{0}b\nc"), "a_b_c");
        assert_eq!(sanitize_component("trailing. . "), "trailing");
        assert_eq!(sanitize_component("nul"), "_nul");
        assert_eq!(sanitize_component("COM1.jpg"), "_COM1.jpg");
        assert_eq!(sanitize_component("COM0.jpg"), "COM0.jpg");
        assert_eq!(sanitize_component("Console"), "Console");
        let long = format!("{}.jpeg", "x".repeat(300));
        let s = sanitize_component(&long);
        assert_eq!(s.chars().count(), MAX_COMPONENT_CHARS);
        assert!(s.ends_with(".jpeg"));
        for s in ["a/b", "../x", "C:\\x", "x*?"] {
            assert!(is_safe_relative_path(&sanitize_component(s)), "{s}");
        }
    }

    #[test]
    fn paths() {
        assert!(is_safe_relative_path("a/b/c.jpg"));
        for bad in ["", "/a", "a//b", "a/../b", "./a", "a\\b", "a/"] {
            assert!(!is_safe_relative_path(bad), "{bad}");
        }
        assert_eq!(split_extension("a.tar.gz"), ("a.tar", ".gz"));
        assert_eq!(split_extension(".bashrc"), (".bashrc", ""));
        assert_eq!(split_extension("x."), ("x.", ""));
        assert_eq!(join(&["a", "b"]), "a/b");
    }
}
