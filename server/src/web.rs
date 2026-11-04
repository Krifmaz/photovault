//! The built web app (`PHOTOVAULT_WEB_DIR`), served on the API port so one
//! container is enough. API routes always win; any other `GET`/`HEAD` is looked
//! up in the web folder as `<path>`, `<path>.html`, `<path>/index.html`, then
//! falls back to the nearest `index.html` above the path (client-side routes;
//! sub-apps such as `albums/`, `cast/` and `accounts/` have their own) or `404.html`.
//!
//! API routes are matched first (this is the router's fallback), so `/cast/*` API
//! calls never reach `cast/index.html`. No framing headers are sent: `/embed` must
//! stay framable if security headers are ever added here.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use std::path::{Component, Path, PathBuf};
use tower::ServiceExt;
use tower_http::services::ServeFile;

/// Resolve a request path to a file under `root`, or `None`.
pub fn resolve(root: &Path, uri_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(uri_path)?;
    let rel = Path::new(decoded.trim_start_matches('/'));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let base = root.join(rel);
    let mut candidates = vec![base.clone()];
    if !decoded.ends_with('/') && !decoded.is_empty() && decoded != "/" {
        let mut html = base.clone().into_os_string();
        html.push(".html");
        candidates.push(PathBuf::from(html));
    }
    candidates.push(base.join("index.html"));
    candidates.into_iter().find(|p| p.is_file())
}

/// The `index.html` of the deepest folder above `uri_path` that has one
/// (`/albums/abc` → `albums/index.html`, `/gallery` → `index.html`).
pub fn nearest_index(root: &Path, uri_path: &str) -> Option<PathBuf> {
    let decoded = percent_decode(uri_path)?;
    let rel = Path::new(decoded.trim_start_matches('/'));
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    let mut dir = rel.parent();
    while let Some(d) = dir {
        let candidate = root.join(d).join("index.html");
        if candidate.is_file() {
            return Some(candidate);
        }
        dir = d.parent();
    }
    Some(root.join("index.html")).filter(|p| p.is_file())
}

fn percent_decode(s: &str) -> Option<String> {
    let bytes: Vec<u8> =
        url::form_urlencoded::parse(format!("p={}", s.replace('+', "%2B")).as_bytes())
            .next()
            .map(|(_, v)| v.into_owned().into_bytes())?;
    let out = String::from_utf8(bytes).ok()?;
    (!out.contains('\0')).then_some(out)
}

/// The fallback handler for unmatched requests.
pub async fn serve(root: PathBuf, req: Request) -> Response {
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return StatusCode::NOT_FOUND.into_response();
    }
    let path = req.uri().path().to_string();
    let (file, status) = match resolve(&root, &path) {
        Some(f) => (f, StatusCode::OK),
        // Paths with an extension are assets: a real 404. Others are app routes.
        None if Path::new(&path).extension().is_some() => match root.join("404.html") {
            p if p.is_file() => (p, StatusCode::NOT_FOUND),
            _ => return StatusCode::NOT_FOUND.into_response(),
        },
        None => match nearest_index(&root, &path) {
            Some(p) => (p, StatusCode::OK),
            None => return StatusCode::NOT_FOUND.into_response(),
        },
    };
    // Next.js puts content-hashed files under `_next/static`: cache those for a year.
    let immutable = path.starts_with("/_next/static/");
    let req = Request::builder()
        .method(req.method().clone())
        .uri("/")
        .body(Body::empty())
        .unwrap_or_default();
    match ServeFile::new(&file).oneshot(req).await {
        Ok(res) => {
            let mut res = res.map(Body::new);
            if status != StatusCode::OK {
                *res.status_mut() = status;
            }
            let cache = if immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            if let Ok(v) = header::HeaderValue::from_str(cache) {
                res.headers_mut().insert(header::CACHE_CONTROL, v);
            }
            res
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolution_and_traversal() {
        let t = tempfile::tempdir().unwrap();
        let r = t.path();
        std::fs::write(r.join("index.html"), "i").unwrap();
        std::fs::write(r.join("gallery.html"), "g").unwrap();
        std::fs::create_dir_all(r.join("shared/a")).unwrap();
        std::fs::write(r.join("shared/a/index.html"), "s").unwrap();
        assert_eq!(resolve(r, "/"), Some(r.join("index.html")));
        assert_eq!(resolve(r, "/gallery"), Some(r.join("gallery.html")));
        assert_eq!(
            resolve(r, "/shared/a/"),
            Some(r.join("shared/a/index.html"))
        );
        assert_eq!(resolve(r, "/shared/a"), Some(r.join("shared/a/index.html")));
        assert_eq!(resolve(r, "/../etc/passwd"), None);
        assert_eq!(resolve(r, "/%2e%2e/etc/passwd"), None);
        assert_eq!(resolve(r, "/missing"), None);
        std::fs::create_dir_all(r.join("albums")).unwrap();
        std::fs::write(r.join("albums/index.html"), "a").unwrap();
        assert_eq!(
            nearest_index(r, "/albums/tok123"),
            Some(r.join("albums/index.html"))
        );
        assert_eq!(
            nearest_index(r, "/albums/x/y"),
            Some(r.join("albums/index.html"))
        );
        assert_eq!(nearest_index(r, "/gallery"), Some(r.join("index.html")));
        assert_eq!(nearest_index(r, "/../x"), None);
    }
}
