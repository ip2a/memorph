use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use std::path::Path;

use crate::web_assets;

pub fn router() -> Router {
    Router::new().layer(middleware::from_fn(deny_api_paths))
}

pub async fn serve_app(request: Request) -> Response {
    let path = request.uri().path().trim_start_matches('/');
    let asset_path = if path.is_empty() { "index.html" } else { path };

    if web_assets::has_override_assets() {
        return web_assets::response_for_asset(asset_path);
    }

    if let Some(response) = asset_response(asset_path) {
        return response;
    }

    if !web_assets::has_assets() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "memorph web assets are missing. The web frontend was not embedded during build.",
        )
            .into_response();
    }

    if has_file_extension(asset_path) {
        return StatusCode::NOT_FOUND.into_response();
    }

    web_assets::response_for_asset("index.html")
}

pub fn asset_response(path: &str) -> Option<Response> {
    web_assets::find(path).map(|_| web_assets::response_for_asset(path))
}

async fn deny_api_paths(request: Request, next: Next) -> Response {
    if request.uri().path().starts_with("/api/") {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}

fn has_file_extension(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some()
}
