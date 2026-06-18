use anyhow::Result;
use axum::Router;
use tower_http::cors::{Any, CorsLayer};

use crate::api;
use crate::web;

pub fn build_router() -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = api::router();
    Router::new()
        .merge(api_routes)
        .merge(web::router())
        .fallback(web::serve_app)
        .layer(cors)
}

pub fn build_api_router() -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    api::router().layer(cors)
}

pub async fn run(port: u16, no_open: bool) -> Result<()> {
    crate::cache::init_watcher();

    let app = build_router();
    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let url = format!("http://{}", listener.local_addr()?);
    let _ = crate::hooks::server::publish_runtime_endpoint(&url);
    println!("memorph server started: {}", url);

    if !no_open {
        if let Err(err) = open::that(&url) {
            eprintln!("Failed to open browser: {err}");
        }
    }

    axum::serve(listener, app).await?;
    Ok(())
}

pub async fn run_api(port: u16) -> Result<()> {
    crate::cache::init_watcher();

    let app = build_api_router();
    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let url = format!("http://{}", listener.local_addr()?);
    let _ = crate::hooks::server::publish_runtime_endpoint(&url);

    println!("memorph API server started: {}", url);
    println!("API base path: /api/v1");

    axum::serve(listener, app).await?;
    Ok(())
}
