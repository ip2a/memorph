use std::path::PathBuf;

use axum::body::Body;
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

mod embedded {
    include!(concat!(env!("OUT_DIR"), "/embedded_web_assets.rs"));
}

pub const MEMORPH_ASCII: &str = r#"███    ███   ███████   ███    ███   ██████   ██████   ██████   ██    ██
████  ████   ██        ████  ████  ██    ██  ██   ██  ██   ██  ██    ██
██ ████ ██   █████     ██ ████ ██  ██    ██  ██████   ██████   ████████
██  ██  ██   ██        ██  ██  ██  ██    ██  ██   ██  ██       ██    ██
██      ██   ███████   ██      ██   ██████   ██   ██  ██       ██    ██"#;

const ENV_WEB_ASSETS_DIR: &str = "MEMORPH_WEB_ASSETS_DIR";

pub(crate) fn has_assets() -> bool {
    !embedded::ASSETS.is_empty() || override_assets_dir().is_some()
}

pub(crate) fn find(path: &str) -> Option<(&'static [u8], &'static str)> {
    embedded::ASSETS
        .iter()
        .find(|asset| asset.path == path)
        .map(|asset| (asset.contents, asset.mime))
}

pub(crate) fn response_for_asset(path: &str) -> Response {
    if let Some(dir) = override_assets_dir() {
        return response_from_dir(&dir, path);
    }
    match find(path) {
        Some((contents, mime)) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, HeaderValue::from_static(mime))
            .body(Body::from(contents.to_vec()))
            .expect("static asset response is valid"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn override_assets_dir() -> Option<PathBuf> {
    std::env::var_os(ENV_WEB_ASSETS_DIR)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty() && path.is_dir())
}

fn response_from_dir(dir: &std::path::Path, path: &str) -> Response {
    let asset_path = dir.join(path);
    let safe_path = normalize_within_dir(dir, &asset_path).unwrap_or_else(|| asset_path.clone());
    match std::fs::read(&safe_path) {
        Ok(contents) => {
            let mime = mime_for_path(path);
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, HeaderValue::from_static(mime))
                .body(Body::from(contents))
                .expect("static asset response is valid")
        }
        Err(_) if path != "index.html" => response_from_dir(dir, "index.html"),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn normalize_within_dir(base: &std::path::Path, path: &std::path::Path) -> Option<PathBuf> {
    let canonical_base = std::fs::canonicalize(base).ok()?;
    let canonical_path = std::fs::canonicalize(path).ok()?;
    if canonical_path.starts_with(&canonical_base) {
        Some(canonical_path)
    } else {
        None
    }
}

fn mime_for_path(path: &str) -> &'static str {
    if path.ends_with(".html") || path.is_empty() {
        return "text/html";
    }
    if path.ends_with(".js") {
        return "text/javascript";
    }
    if path.ends_with(".css") {
        return "text/css";
    }
    if path.ends_with(".svg") {
        return "image/svg+xml";
    }
    if path.ends_with(".json") {
        return "application/json";
    }
    "application/octet-stream"
}
