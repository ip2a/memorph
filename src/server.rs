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
    let web_routes = web::router();

    Router::new()
        .merge(web_routes)
        .merge(api_routes)
        .layer(cors)
}

pub async fn run(port: u16, no_open: bool) -> Result<()> {
    let app = build_router();
    let addr = format!("127.0.0.1:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    let url = format!("http://{}", addr);
    println!("memorph server started: {}", url);

    if !no_open {
        if let Err(err) = open::that(&url) {
            eprintln!("Failed to open browser: {err}");
        }
    }

    axum::serve(listener, app).await?;
    Ok(())
}
