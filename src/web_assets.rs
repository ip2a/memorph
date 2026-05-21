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

pub(crate) fn has_assets() -> bool {
    !embedded::ASSETS.is_empty()
}

pub(crate) fn find(path: &str) -> Option<(&'static [u8], &'static str)> {
    embedded::ASSETS
        .iter()
        .find(|asset| asset.path == path)
        .map(|asset| (asset.contents, asset.mime))
}

pub(crate) fn response_for_asset(path: &str) -> Response {
    match find(path) {
        Some((contents, mime)) => Response::builder()
            .status(StatusCode::OK)
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static(mime),
            )
            .body(Body::from(contents.to_vec()))
            .expect("static asset response is valid"),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
