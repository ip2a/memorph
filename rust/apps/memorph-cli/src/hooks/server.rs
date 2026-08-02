//! Integrated hook API routes.
//!
//! This module does not start a standalone server. Its router is merged into
//! the existing memorph API router so Web, API-only, and Desktop/Tauri surfaces
//! all share the same hook runtime.

use anyhow::Result;
use axum::{
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use memorph::hooks::model::HookEvent;
use memorph::hooks::normalizer;
use memorph::hooks::protocol::{HookIngestRequest, HookIngestResponse};
use memorph::hooks::store;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct HookApiResponse<T: Serialize> {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl<T: Serialize> HookApiResponse<T> {
    fn success(data: T) -> Json<Self> {
        Json(Self {
            ok: true,
            data: Some(data),
            error: None,
        })
    }
}

pub fn router() -> Router {
    Router::new().route("/api/v1/hooks/ingest", post(ingest_event))
}

async fn ingest_event(
    headers: HeaderMap,
    Json(request): Json<HookIngestRequest>,
) -> impl IntoResponse {
    if let Err(error) = memorph::runtime::run_blocking(move || authorize_ingest(&headers)).await {
        return hook_error(StatusCode::UNAUTHORIZED, error).into_response();
    }

    let mut events = match normalizer::normalize_request(&request) {
        Ok(events) => events,
        Err(error) => {
            let message = error.to_string();
            let stored_message = message.clone();
            let _ = memorph::runtime::run_blocking(move || {
                store::append_error("normalize", stored_message)?;
                Ok(())
            })
            .await;
            return hook_error(StatusCode::BAD_REQUEST, message).into_response();
        }
    };
    for event in &mut events {
        enrich_event_from_environment(event, &request.environment);
    }

    match memorph::runtime::run_blocking(move || {
        let mut event_ids = Vec::new();
        let mut updates = Vec::new();
        for event in &events {
            event_ids.push(event.event_id.clone());
            if let Err(error) = store::append_event(event) {
                let _ = store::append_error("append_event", error.to_string());
                return Err(error);
            }
            updates.push(event);
        }
        {
            let mut state = memorph::hooks::runtime_state::runtime_state()
                .write()
                .unwrap();
            for event in updates {
                state.apply_event(event);
            }
        }
        Ok(HookIngestResponse::accepted(event_ids))
    })
    .await
    {
        Ok(response) => HookApiResponse::success(response).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

fn enrich_event_from_environment(
    event: &mut HookEvent,
    environment: &memorph::hooks::protocol::HookBridgeEnvironment,
) {
    if event.cwd.is_none() {
        event.cwd = environment.cwd.clone();
    }
    if event.pid.is_none() {
        event.pid = environment.pid;
    }
    if event.parent_pid.is_none() {
        event.parent_pid = environment.parent_pid;
    }
    if event.pid_start_time.is_none() {
        event.pid_start_time = environment.pid_start_time.clone();
    }
    if event.tty.is_none() {
        event.tty = environment.tty.clone();
    }
    if event.process_ancestry.is_empty() {
        event.process_ancestry = environment.process_ancestry.clone();
    }
}

fn authorize_ingest(headers: &HeaderMap) -> Result<()> {
    let Some(endpoint) = memorph::hooks::runtime_state::current_runtime_endpoint() else {
        anyhow::bail!("Hook runtime endpoint is not published for this process");
    };
    let header_token = headers
        .get("x-memorph-hook-token")
        .and_then(|value| value.to_str().ok())
        .or_else(|| bearer_token(headers));
    match header_token {
        Some(token) if token == endpoint.token => Ok(()),
        _ => anyhow::bail!("Invalid hook token"),
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    value.strip_prefix("Bearer ").map(str::trim)
}

fn hook_error(status: StatusCode, msg: impl ToString) -> impl IntoResponse {
    (
        status,
        Json(HookApiResponse::<()> {
            ok: false,
            data: None,
            error: Some(msg.to_string()),
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::to_bytes, body::Body, http::Request};
    use chrono::Utc;
    use memorph::hooks::protocol::HookRuntimeEndpoint;
    use serde_json::json;
    use tower::util::ServiceExt;

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        memorph::hooks::test_support::test_runtime_guard()
    }

    async fn read_json(app: Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, value)
    }

    async fn read_status(app: Router, request: Request<Body>) -> StatusCode {
        app.oneshot(request).await.unwrap().status()
    }

    fn configure_test_runtime() -> HookRuntimeEndpoint {
        let endpoint = HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "test-token".to_string(),
            pid: 1,
            started_at: Utc::now(),
        };
        memorph::hooks::runtime_state::set_runtime_endpoint_for_tests(endpoint.clone());
        endpoint
    }

    #[tokio::test]
    async fn ingest_requires_hook_token() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        memorph::hooks::runtime_state::reset_for_tests();
        configure_test_runtime();
        let request = HookIngestRequest::new("generic", "heartbeat", json!({}));
        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/ingest")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(value["ok"], false);
    }

    #[tokio::test]
    async fn ingest_persists_event_and_updates_runtime_state() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        memorph::hooks::runtime_state::reset_for_tests();
        let endpoint = configure_test_runtime();
        let request = HookIngestRequest::new(
            "generic",
            "tool_started",
            json!({
                "session_id": "session-1",
                "tool": {"name": "Bash", "input": {"command": "cargo check"}}
            }),
        );
        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/ingest")
                .header("content-type", "application/json")
                .header("x-memorph-hook-token", endpoint.token)
                .body(Body::from(serde_json::to_vec(&request).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["accepted"], true);
        let events = store::load_recent_events(10).unwrap();
        assert!(events
            .iter()
            .any(|event| event.provider_session_id.as_deref() == Some("session-1")));

        let sessions = memorph::hooks::runtime_state::runtime_sessions_snapshot();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].provider, "generic");
        assert_eq!(
            sessions[0].provider_session_id.as_deref(),
            Some("session-1")
        );
    }

    #[tokio::test]
    async fn blocking_permission_ingest_records_without_decision() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        memorph::hooks::runtime_state::reset_for_tests();
        let endpoint = configure_test_runtime();

        let mut request = HookIngestRequest::new(
            "generic",
            "permission_requested",
            json!({
                "session_id": "session-observe-only",
                "permission_prompt": "Allow shell command?",
                "tool": {"name": "Bash", "input": {"command": "cargo test"}}
            }),
        );
        request.blocking = true;
        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/ingest")
                .header("content-type", "application/json")
                .header("x-memorph-hook-token", endpoint.token)
                .body(Body::from(serde_json::to_vec(&request).unwrap()))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["accepted"], true);
        assert_eq!(value["data"].as_object().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn removed_hook_management_routes_are_not_registered() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        memorph::hooks::runtime_state::reset_for_tests();

        for (method, uri) in [
            ("GET", "/api/v1/hooks/status"),
            ("GET", "/api/v1/hooks/overview"),
            ("GET", "/api/v1/hooks/runtime-sessions"),
            ("GET", "/api/v1/hooks/diagnostics"),
            ("POST", "/api/v1/hooks/cleanup"),
            ("POST", "/api/v1/hooks/providers/sample/operations/install"),
            ("GET", "/api/v1/hooks/providers/sample/overview"),
            ("GET", "/api/v1/hooks/pending"),
            ("GET", "/api/v1/hooks/policy"),
        ] {
            let status = read_status(
                router(),
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        }
    }
}
