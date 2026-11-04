//! PHOTOVAULT: server connection (new file).
//!
//! Lets the app send asset downloads to its own server instead of Ente's
//! hosts: a download URL that starts with a registered upstream prefix (for
//! example `https://models.ente.com/`) is fetched from the mirror prefix
//! (for example `http://10.0.1.7:8095/photovault/models/`) instead. The
//! catalog, sizes and SHA-256 checks are unchanged, so a mirror can only
//! serve the exact files the app expects. Process-wide, set by the app.
//!
//! A prefix can also be blocked: downloads under it fail without any request
//! (the app has no mirror for them and must not fall back to Ente's host).

use std::sync::RwLock;

/// (upstream prefix, mirror prefix); `None` = blocked.
static MIRRORS: RwLock<Vec<(String, Option<String>)>> = RwLock::new(Vec::new());

fn with_slash(prefix: &str) -> String {
    let trimmed = prefix.trim_end_matches('/');
    format!("{trimmed}/")
}

/// Fetches URLs under `upstream` from `mirror` instead; `None` (or an empty
/// mirror) removes the rule, so downloads go to `upstream` again.
pub fn set_url_mirror(upstream: &str, mirror: Option<&str>) {
    let upstream = with_slash(upstream);
    let mut mirrors = MIRRORS.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    mirrors.retain(|(from, _)| *from != upstream);
    if let Some(mirror) = mirror.map(str::trim).filter(|m| !m.is_empty()) {
        mirrors.push((upstream, Some(with_slash(mirror))));
    }
}

/// Blocks downloads of URLs under `upstream` (replaces any mirror for it).
pub fn block_url_prefix(upstream: &str) {
    let upstream = with_slash(upstream);
    let mut mirrors = MIRRORS.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    mirrors.retain(|(from, _)| *from != upstream);
    mirrors.push((upstream, None));
}

/// Blocks `upstream` unless a rule for it (mirror or block) is already set.
pub fn block_url_prefix_if_unset(upstream: &str) {
    let upstream = with_slash(upstream);
    let mut mirrors = MIRRORS.write().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !mirrors.iter().any(|(from, _)| *from == upstream) {
        mirrors.push((upstream, None));
    }
}

/// True when downloads of `url` are blocked.
pub fn is_blocked(url: &str) -> bool {
    let mirrors = MIRRORS.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    mirrors
        .iter()
        .any(|(from, to)| to.is_none() && url.starts_with(from.as_str()))
}

/// The URL a download of `url` actually goes to.
pub fn mirrored_url(url: &str) -> String {
    let mirrors = MIRRORS.read().unwrap_or_else(std::sync::PoisonError::into_inner);
    for (from, to) in mirrors.iter() {
        if let (Some(to), Some(rest)) = (to, url.strip_prefix(from.as_str())) {
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

    #[test]
    fn blocks_prefixes_until_a_mirror_is_set() {
        let upstream = "https://blocked.example/";
        block_url_prefix_if_unset(upstream);
        assert!(is_blocked("https://blocked.example/a.onnx"));
        assert!(!is_blocked("https://other.example/a.onnx"));
        set_url_mirror(upstream, Some("http://10.0.1.7:8095/m/"));
        assert!(!is_blocked("https://blocked.example/a.onnx"));
        block_url_prefix_if_unset(upstream);
        assert!(!is_blocked("https://blocked.example/a.onnx"));
        assert_eq!(
            mirrored_url("https://blocked.example/a.onnx"),
            "http://10.0.1.7:8095/m/a.onnx"
        );
        block_url_prefix(upstream);
        assert!(is_blocked("https://blocked.example/a.onnx"));
        assert_eq!(
            mirrored_url("https://blocked.example/a.onnx"),
            "https://blocked.example/a.onnx"
        );
        set_url_mirror(upstream, None);
    }
}
