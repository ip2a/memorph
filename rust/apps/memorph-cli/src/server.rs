use anyhow::{Context as _, Result};
use axum::Router;
use std::io::{self, Write};
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

pub async fn run(port: u16, no_open: bool, allow_fallback: bool, json: bool) -> Result<()> {
    memorph::cache::init_watcher();
    memorph::core::spawn_background_sync_loop();

    let app = build_router();
    let (listener, actual_port) =
        bind_with_fallback("127.0.0.1", port, allow_fallback, json).await?;
    let url = format!("http://127.0.0.1:{}", actual_port);
    if let Err(err) = memorph::hooks::runtime_state::publish_runtime_endpoint(&url) {
        memorph::logging::error("publish_runtime_endpoint", format!("{err}"));
    }
    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "result": {"interface": "web", "url": url}
            })
        );
        io::stdout().flush()?;
    } else {
        println!(
            "{}",
            memorph::i18n::format(server_language(), "cliServerStarted", &[("url", &url)])
        );
    }

    if !no_open {
        if let Err(err) = open::that(&url) {
            eprintln!(
                "{}",
                memorph::i18n::format(
                    server_language(),
                    "cliBrowserOpenFailed",
                    &[("error", &err.to_string())]
                )
            );
        }
    }

    axum::serve(listener, app).await?;
    Ok(())
}

pub async fn run_api(port: u16, allow_fallback: bool, json: bool) -> Result<()> {
    memorph::cache::init_watcher();
    let _ = memorph::config::prime_default_workspace_if_unset();
    memorph::core::spawn_background_sync_loop();

    let app = build_api_router();
    let (listener, actual_port) =
        bind_with_fallback("127.0.0.1", port, allow_fallback, json).await?;
    let url = format!("http://127.0.0.1:{}", actual_port);
    if let Err(err) = memorph::hooks::runtime_state::publish_runtime_endpoint(&url) {
        memorph::logging::error("publish_runtime_endpoint", format!("{err}"));
    }

    if json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "result": {"interface": "api", "url": url}
            })
        );
        io::stdout().flush()?;
    } else {
        println!(
            "{}",
            memorph::i18n::format(server_language(), "cliApiServerStarted", &[("url", &url)])
        );
        println!(
            "{}",
            memorph::i18n::text(server_language(), "cliApiBasePath")
        );
    }

    axum::serve(listener, app).await?;
    Ok(())
}

const FALLBACK_RANGE: u16 = 100;

fn is_port_available(host: &str, port: u16) -> bool {
    std::net::TcpListener::bind((host, port)).is_ok()
}

async fn bind_with_fallback(
    host: &str,
    port: u16,
    allow_fallback: bool,
    json: bool,
) -> Result<(tokio::net::TcpListener, u16)> {
    let addr = format!("{}:{}", host, port);
    if is_port_available(host, port) {
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .with_context(|| format!("Could not bind to {}", addr))?;
        return Ok((listener, port));
    }

    if !allow_fallback {
        anyhow::bail!("Could not bind to {}; address already in use", addr);
    }

    let max_port = port.saturating_add(FALLBACK_RANGE);
    let start_port = port.saturating_add(1);
    for try_port in start_port..=max_port {
        if !is_port_available(host, try_port) {
            continue;
        }
        let addr = format!("{}:{}", host, try_port);
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                if !json {
                    println!(
                        "{}",
                        memorph::i18n::format(
                            server_language(),
                            "cliPortFallback",
                            &[
                                ("port", &port.to_string()),
                                ("fallback", &try_port.to_string())
                            ]
                        )
                    );
                }
                return Ok((listener, try_port));
            }
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e).with_context(|| format!("Could not bind to {}", addr)),
        }
    }

    anyhow::bail!(
        "Could not bind to any port in {}:{}..={}; all addresses are in use",
        host,
        port,
        max_port
    )
}

fn server_language() -> memorph::config::UiLanguage {
    memorph::config::web_preferences()
        .map(|preferences| preferences.language)
        .unwrap_or_default()
}
