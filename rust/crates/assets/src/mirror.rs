//! PHOTOVAULT: server connection (new file).
//!
//! Lets the app send asset downloads to its own server instead of Ente's
//! hosts: a download URL that starts with a registered upstream prefix (for
//! example `https://models.ente.com/`) is fetched from the mirror prefix
//! (for example `http://10.0.1.7:8095/photovault/models/`) instead. The
//! catalog, sizes and SHA-256 checks are unchanged, so a mirror can only
//! serve the exact files the app expects. Process-wide, set by the app.

use std::sync::RwLock;

static MIRRORS: RwLock<Vec<(String, String)>> = RwLock::new(Vec::new());

fn with_slash(prefix: &str) -> String {
    let trimmed = prefix.trim_end_matches('/');
    format!("{trimmed}/")
}

/// Fetches URLs under `upstream` from `mirror` instead; `None` (or an empty
/// mirror) removes the rule, so downloads go to `upstream` again.
pub fn set_url_mirror(upstream: &str, mirror: Option<&str>) {
    let upstream = with_slash(upstream);
    let mut mirrors = MIRRORS.write().unwrap_or_else(|e| e.into_inner());
    mirrors.retain(|(from, _)| *from != upstream);
    if let Some(mirror) = mirror.map(str::trim).filter(|m| !m.is_empty()) {
        mirrors.push((upstream, with_slash(mirror)));
    }
}

/// The URL a download of `url` actually goes to.
pub fn mirrored_url(url: &str) -> String {
    let mirrors = MIRRORS.read().unwrap_or_else(|e| e.into_inner());
    for (from, to) in mirrors.iter() {
        if let Some(rest) = url.strip_prefix(from.as_str()) {
            return format!("{to}{rest}");
        }
    }
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_only_registered_prefixes() {
        set_url_mirror(
            "https://models.ente.com",
            Some("http://10.0.1.7:8095/photovault/models/"),
        );
        assert_eq!(
            mirrored_url("https://models.ente.com/PP-OCRv5/ppocrv5_dict.txt"),
            "http://10.0.1.7:8095/photovault/models/PP-OCRv5/ppocrv5_dict.txt"
        );
        assert_eq!(
            mirrored_url("https://models.ente.community/x.onnx"),
            "https://models.ente.community/x.onnx"
        );
        assert_eq!(
            mirrored_url("https://example.com/a"),
            "https://example.com/a"
        );
        set_url_mirror("https://models.ente.com/", None);
        assert_eq!(
            mirrored_url("https://models.ente.com/a.onnx"),
            "https://models.ente.com/a.onnx"
        );
    }
}
