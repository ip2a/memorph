//! Integrated hook API routes.
//!
//! This module does not start a standalone server. Its router is merged into
//! the existing memorph API router so Web, API-only, and Desktop/Tauri surfaces
//! all share the same hook runtime.

use anyhow::{Context, Result};
use axum::{
    extract::{Path, Query},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};
use uuid::Uuid;

use crate::hooks::identity::runtime_session_id_for_event;
use crate::hooks::model::{HookEvent, RuntimeSession, RuntimeSessionStatus};
use crate::hooks::normalizer;
use crate::hooks::protocol::{HookIngestRequest, HookIngestResponse, HookRuntimeEndpoint};
use crate::hooks::runtime::{RuntimeCleanupReport, RuntimeState};
use crate::hooks::store::{self, RuntimeSessionStore};

static RUNTIME_STATE: OnceLock<RwLock<RuntimeState>> = OnceLock::new();
static RUNTIME_ENDPOINT: OnceLock<RwLock<Option<HookRuntimeEndpoint>>> = OnceLock::new();

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

#[derive(Debug, Serialize)]
struct HookStatusPayload {
    server: HookServerStatus,
    runtime_sessions: usize,
    active_sessions: usize,
    providers: Vec<crate::hooks::profiles::HookProviderProfile>,
}

#[derive(Debug, Serialize)]
struct HookServerStatus {
    running: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at: Option<chrono::DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
struct HookOverviewPayload {
    generated_at: chrono::DateTime<Utc>,
    summary: HookOverviewSummary,
    server: HookServerStatus,
    providers: Vec<crate::agent_management::AgentManagementEntry>,
    runtime_sessions: Vec<RuntimeSession>,
    recent_errors: Vec<store::HookErrorRecord>,
    recent_events: Vec<HookEvent>,
}

#[derive(Debug, Serialize)]
struct HookOverviewSummary {
    providers: usize,
    supported_providers: usize,
    installed_ok: usize,
    not_installed: usize,
    needs_attention: usize,
    active_runtime_sessions: usize,
    linked_sessions: usize,
    weakly_linked_sessions: usize,
    no_session_match: usize,
    recent_errors: usize,
}

#[derive(Debug, Serialize)]
struct HookProviderOverviewPayload {
    generated_at: chrono::DateTime<Utc>,
    provider: crate::agent_management::AgentManagementEntry,
    runtime_sessions: Vec<RuntimeSession>,
    recent_events: Vec<HookEvent>,
    recent_errors: Vec<store::HookErrorRecord>,
}

#[derive(Debug, Deserialize)]
struct EventsQuery {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct RuntimeSessionsQuery {
    provider: Option<String>,
    session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DiagnosticsQuery {
    event_limit: Option<usize>,
    error_limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct CleanupQuery {
    idle_after_seconds: Option<i64>,
    orphan_after_seconds: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct SessionDiagnosisQuery {
    provider: Option<String>,
    hook_filter: Option<crate::core::SessionHookFilter>,
    limit: Option<usize>,
    offset: Option<usize>,
}

pub fn router() -> Router {
    Router::new()
        .route("/api/v1/hooks/overview", get(get_overview))
        .route(
            "/api/v1/hooks/session-diagnosis",
            get(list_session_diagnosis),
        )
        .route("/api/v1/hooks/status", get(get_status))
        .route(
            "/api/v1/hooks/providers/{provider}/installed",
            get(list_installed_hooks),
        )
        .route(
            "/api/v1/hooks/providers/{provider}/installed/{event}/{index}/{fingerprint}",
            delete(remove_installed_hook),
        )
        .route("/api/v1/hooks/ingest", post(ingest_event))
        .route("/api/v1/hooks/events", get(list_events))
        .route("/api/v1/hooks/runtime-sessions", get(list_runtime_sessions))
        .route("/api/v1/hooks/diagnostics", get(get_diagnostics))
        .route("/api/v1/hooks/doctor", post(run_doctor))
        .route("/api/v1/hooks/cleanup", post(cleanup_runtime_sessions))
        .route(
            "/api/v1/hooks/providers/{provider}/overview",
            get(provider_overview),
        )
        .route(
            "/api/v1/hooks/providers/{provider}/operations/{operation}",
            post(run_provider_hook_operation),
        )
        .route(
            "/api/v1/hooks/providers/{provider}/status",
            get(provider_status),
        )
}

async fn list_session_diagnosis(Query(query): Query<SessionDiagnosisQuery>) -> impl IntoResponse {
    let providers: Vec<String> = query
        .provider
        .map(|providers| {
            providers
                .split(',')
                .map(str::trim)
                .filter(|provider| !provider.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let hook_filter = query
        .hook_filter
        .unwrap_or(crate::core::SessionHookFilter::Attention);
    let params = crate::core::SessionListParams {
        all: true,
        providers,
        cwd: None,
        include_message_counts: false,
        limit: Some(query.limit.unwrap_or(8).clamp(1, 100)),
        offset: query.offset,
        sort: crate::core::SessionListSort::HookAttention,
        hook_filter,
    };

    match crate::api::run_blocking(move || crate::core::projection::list_sessions(&params)).await {
        Ok(groups) => HookApiResponse::success(groups).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn list_installed_hooks(Path(provider): Path<String>) -> impl IntoResponse {
    match crate::api::run_blocking(move || crate::hooks::discovery::list(&provider)).await {
        Ok(payload) => HookApiResponse::success(payload).into_response(),
        Err(error) => hook_error(StatusCode::BAD_REQUEST, error).into_response(),
    }
}

async fn remove_installed_hook(
    Path((provider, event, index, fingerprint)): Path<(String, String, usize, String)>,
) -> impl IntoResponse {
    match crate::api::run_blocking(move || {
        crate::hooks::discovery::remove(&provider, &event, index, &fingerprint)
    })
    .await
    {
        Ok(payload) => HookApiResponse::success(payload).into_response(),
        Err(error) => hook_error(StatusCode::BAD_REQUEST, error).into_response(),
    }
}

async fn get_overview() -> impl IntoResponse {
    match crate::api::run_blocking(build_overview).await {
        Ok(payload) => HookApiResponse::success(payload).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn provider_overview(Path(provider): Path<String>) -> impl IntoResponse {
    match crate::api::run_blocking(move || build_provider_overview(&provider)).await {
        Ok(payload) => HookApiResponse::success(payload).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn run_provider_hook_operation(
    Path((provider, operation)): Path<(String, String)>,
) -> impl IntoResponse {
    match crate::api::run_blocking(move || run_hook_operation(&provider, &operation)).await {
        Ok(report) => HookApiResponse::success(report).into_response(),
        Err(error) => hook_error(StatusCode::BAD_REQUEST, error).into_response(),
    }
}

fn run_hook_operation(
    provider: &str,
    operation: &str,
) -> Result<crate::hooks::model::HookOperationReport> {
    let operation = normalize_hook_operation(operation)?;
    if !crate::hooks::capabilities::supports_setting(provider, operation.setting_id()) {
        anyhow::bail!(
            "Hook operation is not supported for provider: {}.{}",
            provider,
            operation.setting_id()
        );
    }
    crate::hooks::operations::run_operation(provider, operation)
}

fn normalize_hook_operation(
    operation: &str,
) -> Result<crate::hooks::strategies::HookConfigOperation> {
    crate::hooks::strategies::HookConfigOperation::from_setting_id(operation.trim())
        .ok_or_else(|| anyhow::anyhow!("Unknown hook operation: {operation}"))
}

fn build_provider_overview(provider: &str) -> Result<HookProviderOverviewPayload> {
    let provider_entry = crate::agent_management::get_agent_management_entry(provider)
        .with_context(|| format!("Failed to collect hook overview for provider: {provider}"))?;
    let runtime_sessions: Vec<RuntimeSession> = runtime_sessions_snapshot()
        .into_iter()
        .filter(|session| session.provider == provider)
        .collect();
    let recent_events: Vec<HookEvent> = store::load_recent_events(100)
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event.provider == provider)
        .take(20)
        .collect();
    let recent_errors: Vec<store::HookErrorRecord> = store::load_recent_errors(100)
        .unwrap_or_default()
        .into_iter()
        .filter(|error| hook_error_mentions_provider(error, provider))
        .take(20)
        .collect();

    Ok(HookProviderOverviewPayload {
        generated_at: Utc::now(),
        provider: provider_entry,
        runtime_sessions,
        recent_events,
        recent_errors,
    })
}

fn hook_error_mentions_provider(error: &store::HookErrorRecord, provider: &str) -> bool {
    error.scope.contains(provider) || error.message.contains(provider)
}

fn build_overview() -> Result<HookOverviewPayload> {
    let providers = crate::agent_management::list_agent_management_entries()
        .context("Failed to collect hook provider management entries")?;
    let runtime_sessions = runtime_sessions_snapshot();
    let recent_errors = store::load_recent_errors(10).unwrap_or_default();
    let recent_events = store::load_recent_events(20).unwrap_or_default();

    let supported_providers = providers
        .iter()
        .filter(|provider| provider.hook_profile.is_some())
        .count();
    let installed_ok = providers
        .iter()
        .filter(|provider| {
            provider.hook.status == crate::hooks::model::HookHealthStatus::InstalledOk
        })
        .count();
    let not_installed = providers
        .iter()
        .filter(|provider| {
            provider.hook.status == crate::hooks::model::HookHealthStatus::NotInstalled
        })
        .count();
    let needs_attention = providers
        .iter()
        .filter(|provider| hook_status_needs_attention(&provider.hook.status))
        .count();
    let active_runtime_sessions = runtime_sessions
        .iter()
        .filter(|session| {
            !matches!(
                session.status,
                RuntimeSessionStatus::Completed | RuntimeSessionStatus::Failed
            )
        })
        .count();
    let linked_sessions = providers
        .iter()
        .map(|provider| provider.hook_diagnosis.linked)
        .sum();
    let weakly_linked_sessions = providers
        .iter()
        .map(|provider| provider.hook_diagnosis.weakly_linked)
        .sum();
    let no_session_match = providers
        .iter()
        .map(|provider| provider.hook_diagnosis.no_session_match)
        .sum();

    Ok(HookOverviewPayload {
        generated_at: Utc::now(),
        summary: HookOverviewSummary {
            providers: providers.len(),
            supported_providers,
            installed_ok,
            not_installed,
            needs_attention,
            active_runtime_sessions,
            linked_sessions,
            weakly_linked_sessions,
            no_session_match,
            recent_errors: recent_errors.len(),
        },
        server: current_hook_server_status(),
        providers,
        runtime_sessions,
        recent_errors,
        recent_events,
    })
}

fn hook_status_needs_attention(status: &crate::hooks::model::HookHealthStatus) -> bool {
    matches!(
        status,
        crate::hooks::model::HookHealthStatus::InstalledDisabled
            | crate::hooks::model::HookHealthStatus::InstalledStaleBinary
            | crate::hooks::model::HookHealthStatus::InstalledStaleEndpoint
            | crate::hooks::model::HookHealthStatus::InstalledBrokenConfig
            | crate::hooks::model::HookHealthStatus::InstalledConflict
            | crate::hooks::model::HookHealthStatus::Repairable
            | crate::hooks::model::HookHealthStatus::NeedsUserAction
    )
}

pub fn publish_runtime_endpoint(endpoint: &str) -> Result<HookRuntimeEndpoint> {
    let endpoint = HookRuntimeEndpoint {
        endpoint: endpoint.trim_end_matches('/').to_string(),
        token: current_or_new_token(),
        pid: std::process::id(),
        started_at: Utc::now(),
    };

    {
        let lock = runtime_endpoint_cell();
        let mut guard = lock.write().unwrap();
        *guard = Some(endpoint.clone());
    }

    store::save_server_runtime(&endpoint).context("Failed to save hook server runtime endpoint")?;
    Ok(endpoint)
}

pub fn current_runtime_endpoint() -> Option<HookRuntimeEndpoint> {
    runtime_endpoint_cell().read().unwrap().clone()
}

pub fn linked_runtime_sessions(
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> Vec<RuntimeSession> {
    let snapshot = runtime_sessions_snapshot();
    linked_runtime_sessions_from_snapshot(&snapshot, provider, session_id, workspace_dir)
}

pub fn runtime_sessions_snapshot() -> Vec<RuntimeSession> {
    let _ = cleanup_runtime_state(crate::hooks::lifecycle::RuntimeCleanupOptions::default());
    let state = runtime_state().read().unwrap();
    state.sessions.values().cloned().collect()
}

pub fn linked_runtime_sessions_from_snapshot(
    snapshot: &[RuntimeSession],
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> Vec<RuntimeSession> {
    let mut sessions: Vec<RuntimeSession> = snapshot
        .iter()
        .filter(|session| {
            runtime_session_matches_session(session, provider, session_id, workspace_dir)
        })
        .cloned()
        .collect();
    sessions.sort_by(|left, right| right.last_event_at.cmp(&left.last_event_at));
    sessions
}

fn current_or_new_token() -> String {
    current_runtime_endpoint()
        .map(|endpoint| endpoint.token)
        .filter(|token| !token.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

#[cfg(test)]
fn linked_runtime_sessions_from_state(
    state: &RuntimeState,
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> Vec<RuntimeSession> {
    let mut sessions: Vec<RuntimeSession> = state
        .sessions
        .values()
        .filter(|session| {
            runtime_session_matches_session(session, provider, session_id, workspace_dir)
        })
        .cloned()
        .collect();
    sessions.sort_by(|left, right| right.last_event_at.cmp(&left.last_event_at));
    sessions
}

fn runtime_session_matches_session(
    session: &RuntimeSession,
    provider: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> bool {
    if session.provider != provider {
        return false;
    }

    if runtime_session_matches_provider_session_id(session, provider, session_id) {
        return true;
    }

    if session
        .correlation
        .as_ref()
        .map(|correlation| correlation.session_id == session_id)
        .unwrap_or(false)
    {
        return true;
    }

    runtime_session_matches_workspace(session, provider, workspace_dir)
}

fn runtime_session_matches_provider_session_id(
    session: &RuntimeSession,
    provider: &str,
    session_id: &str,
) -> bool {
    let Some(provider_session_id) = session
        .provider_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };

    provider_session_id == session_id
        || provider_session_id == format!("{provider}-{session_id}")
        || provider_session_id.ends_with(&format!("-{session_id}"))
}

fn runtime_session_matches_workspace(
    session: &RuntimeSession,
    provider: &str,
    workspace_dir: Option<&str>,
) -> bool {
    let Some(workspace_dir) = workspace_dir.filter(|value| !value.trim().is_empty()) else {
        return false;
    };

    let session_workspace = session
        .correlation
        .as_ref()
        .and_then(|correlation| correlation.project_dir.as_deref())
        .or_else(|| session.cwd.as_deref().and_then(|cwd| cwd.to_str()));

    crate::core::session_management::workspace_matches(
        provider,
        session_workspace,
        Some(workspace_dir),
    )
}

fn runtime_state() -> &'static RwLock<RuntimeState> {
    RUNTIME_STATE.get_or_init(|| RwLock::new(load_runtime_state().unwrap_or_default()))
}

fn runtime_endpoint_cell() -> &'static RwLock<Option<HookRuntimeEndpoint>> {
    RUNTIME_ENDPOINT.get_or_init(|| RwLock::new(store::load_server_runtime().unwrap_or(None)))
}

fn load_runtime_state() -> Result<RuntimeState> {
    let stored = store::load_runtime_sessions()?;
    Ok(RuntimeState {
        sessions: stored
            .sessions
            .into_iter()
            .map(|session| (session.runtime_id.clone(), session))
            .collect(),
    })
}

fn persist_runtime_state(state: &RuntimeState) -> Result<()> {
    store::save_runtime_sessions(&RuntimeSessionStore {
        version: 1,
        sessions: state.sessions.values().cloned().collect(),
    })
}

async fn get_status() -> impl IntoResponse {
    match crate::api::run_blocking(|| {
        let _ = cleanup_runtime_state(crate::hooks::lifecycle::RuntimeCleanupOptions::default());
        let state = runtime_state().read().unwrap();
        Ok(HookStatusPayload {
            server: current_hook_server_status(),
            runtime_sessions: state.sessions.len(),
            active_sessions: state.active_sessions().len(),
            providers: crate::hooks::registry::profiles(),
        })
    })
    .await
    {
        Ok(payload) => HookApiResponse::success(payload).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

fn current_hook_server_status() -> HookServerStatus {
    let endpoint = current_runtime_endpoint();
    HookServerStatus {
        running: endpoint.is_some(),
        endpoint: endpoint.as_ref().map(|value| value.endpoint.clone()),
        pid: endpoint.as_ref().map(|value| value.pid),
        started_at: endpoint.as_ref().map(|value| value.started_at),
    }
}

async fn ingest_event(
    headers: HeaderMap,
    Json(request): Json<HookIngestRequest>,
) -> impl IntoResponse {
    if let Err(error) = crate::api::run_blocking(move || authorize_ingest(&headers)).await {
        return hook_error(StatusCode::UNAUTHORIZED, error).into_response();
    }

    let mut events = match normalizer::normalize_request(&request) {
        Ok(events) => events,
        Err(error) => {
            let message = error.to_string();
            let stored_message = message.clone();
            let _ = crate::api::run_blocking(move || {
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

    match crate::api::run_blocking(move || {
        let mut event_ids = Vec::new();
        let mut updates = Vec::new();
        for event in &events {
            event_ids.push(event.event_id.clone());
            if let Err(error) = store::append_event(event) {
                let _ = store::append_error("append_event", error.to_string());
                return Err(error);
            }
            let correlation = crate::hooks::correlation::correlate_event(event);
            updates.push((event, correlation));
        }
        {
            let mut state = runtime_state().write().unwrap();
            for (event, correlation) in updates {
                let runtime_id = runtime_session_id_for_event(event);
                state.apply_event(event);
                if let Some(correlation) = correlation {
                    state.attach_correlation(&runtime_id, correlation);
                }
            }
            if let Err(error) = persist_runtime_state(&state) {
                let _ = store::append_error("persist_runtime_state", error.to_string());
                return Err(error);
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

async fn list_events(Query(query): Query<EventsQuery>) -> impl IntoResponse {
    let limit = query.limit.unwrap_or(100).min(1000);
    match crate::api::run_blocking(move || store::load_recent_events(limit)).await {
        Ok(events) => HookApiResponse::success(events).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn list_runtime_sessions(Query(query): Query<RuntimeSessionsQuery>) -> impl IntoResponse {
    match crate::api::run_blocking(move || {
        Ok(runtime_sessions_snapshot()
            .into_iter()
            .filter(|session| {
                query
                    .provider
                    .as_deref()
                    .map(|provider| session.provider == provider)
                    .unwrap_or(true)
            })
            .filter(|session| {
                query
                    .session_id
                    .as_deref()
                    .map(|session_id| session.provider_session_id.as_deref() == Some(session_id))
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>())
    })
    .await
    {
        Ok(sessions) => HookApiResponse::success(sessions).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn provider_status(Path(provider): Path<String>) -> impl IntoResponse {
    match crate::api::run_blocking(move || crate::hooks::operations::status(&provider)).await {
        Ok(status) => HookApiResponse::success(status).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn get_diagnostics(Query(query): Query<DiagnosticsQuery>) -> impl IntoResponse {
    let options = crate::hooks::diagnostics::HookDiagnosticsOptions {
        event_limit: query.event_limit.unwrap_or(100).min(1000),
        error_limit: query.error_limit.unwrap_or(50).min(1000),
    };
    match crate::api::run_blocking(move || crate::hooks::diagnostics::collect(options)).await {
        Ok(report) => HookApiResponse::success(report).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn run_doctor(
    Json(request): Json<crate::hooks::doctor::HookDoctorRequest>,
) -> impl IntoResponse {
    match crate::api::run_blocking(move || crate::hooks::doctor::verify(request)).await {
        Ok(report) => HookApiResponse::success(report).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn cleanup_runtime_sessions(Query(query): Query<CleanupQuery>) -> impl IntoResponse {
    let options = crate::hooks::lifecycle::RuntimeCleanupOptions {
        idle_after_seconds: query.idle_after_seconds.unwrap_or(30 * 60),
        orphan_after_seconds: query.orphan_after_seconds.unwrap_or(60 * 60),
    };
    match crate::api::run_blocking(move || cleanup_runtime_state(options)).await {
        Ok(report) => HookApiResponse::success(report).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

fn cleanup_runtime_state(
    options: crate::hooks::lifecycle::RuntimeCleanupOptions,
) -> Result<RuntimeCleanupReport> {
    let mut state = runtime_state().write().unwrap();
    let report = state.cleanup_stale_sessions(
        Utc::now(),
        options.idle_after(),
        options.orphan_after(),
        crate::hooks::lifecycle::pid_is_alive_with_start_time,
    );
    if report.idle > 0 || report.orphaned > 0 {
        persist_runtime_state(&state)?;
    }
    drop(state);
    Ok(report)
}

fn enrich_event_from_environment(
    event: &mut HookEvent,
    environment: &crate::hooks::protocol::HookBridgeEnvironment,
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
    let Some(endpoint) = current_runtime_endpoint() else {
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
pub(crate) fn reset_for_tests() {
    *runtime_state().write().unwrap() = RuntimeState::default();
    *runtime_endpoint_cell().write().unwrap() = None;
}

#[cfg(test)]
pub(crate) fn set_runtime_endpoint_for_tests(endpoint: HookRuntimeEndpoint) {
    *runtime_endpoint_cell().write().unwrap() = Some(endpoint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::{RuntimeSessionCorrelation, RuntimeSessionId};
    use axum::{body::to_bytes, body::Body, http::Request};
    use chrono::Utc;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tower::util::ServiceExt;

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        crate::hooks::test_support::test_runtime_guard()
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

    fn runtime_session(
        runtime_id: &str,
        provider: &str,
        provider_session_id: Option<&str>,
    ) -> RuntimeSession {
        RuntimeSession {
            runtime_id: RuntimeSessionId::new(runtime_id),
            provider: provider.to_string(),
            provider_session_id: provider_session_id.map(str::to_string),
            run_id: None,
            cwd: None,
            pid: None,
            parent_pid: None,
            pid_start_time: None,
            tty: None,
            terminal_vars: BTreeMap::new(),
            process_ancestry: Vec::new(),
            correlation: None,
            model: None,
            session_title: None,
            transcript_path: None,
            workspace_roots: Vec::new(),
            last_user_prompt: None,
            last_assistant_message: None,
            last_tool_result: None,
            last_error: None,
            stop_reason: None,
            compact_count: 0,
            tool_call_count: 0,
            failed_tool_count: 0,
            permission_request_count: 0,
            question_count: 0,
            status: RuntimeSessionStatus::Running,
            current_tool: None,
            pending_permission: None,
            pending_question: None,
            recent_activity: Vec::new(),
            subagents: BTreeMap::new(),
            last_event_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn linked_runtime_sessions_match_provider_session_id() {
        let mut state = RuntimeState::default();
        state.sessions.insert(
            RuntimeSessionId::new("sample:session:s1"),
            runtime_session("sample:session:s1", "sample", Some("s1")),
        );

        let sessions = linked_runtime_sessions_from_state(&state, "sample", "s1", None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].provider_session_id.as_deref(), Some("s1"));
    }

    #[test]
    fn linked_runtime_sessions_match_correlation_session_id() {
        let mut state = RuntimeState::default();
        let mut session = runtime_session("sample:fp:abc", "sample", None);
        session.correlation = Some(RuntimeSessionCorrelation {
            provider: "sample".to_string(),
            session_id: "session-from-correlation".to_string(),
            title: None,
            project_dir: None,
            source_path: None,
            matched_by: Some("workspace".to_string()),
        });
        state.sessions.insert(session.runtime_id.clone(), session);

        let sessions =
            linked_runtime_sessions_from_state(&state, "sample", "session-from-correlation", None);
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0]
                .correlation
                .as_ref()
                .map(|value| value.session_id.as_str()),
            Some("session-from-correlation")
        );
    }

    #[test]
    fn linked_runtime_sessions_fall_back_to_workspace_match() {
        let mut state = RuntimeState::default();
        let mut session = runtime_session("sample:fp:workspace", "sample", None);
        session.cwd = Some(PathBuf::from("/tmp/project"));
        state.sessions.insert(session.runtime_id.clone(), session);

        let sessions = linked_runtime_sessions_from_state(
            &state,
            "sample",
            "missing-session-id",
            Some("/tmp/project"),
        );
        assert_eq!(sessions.len(), 1);
        assert_eq!(
            sessions[0].cwd.as_deref().and_then(|path| path.to_str()),
            Some("/tmp/project")
        );
    }

    #[tokio::test]
    async fn status_route_exposes_hook_provider_profiles() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();

        let (status, value) = read_json(
            router(),
            Request::builder()
                .uri("/api/v1/hooks/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let providers = value["data"]["providers"].as_array().unwrap();
        let descriptor = crate::hooks::registry::all()
            .first()
            .copied()
            .expect("at least one hook provider");
        let provider = providers
            .iter()
            .find(|provider| provider["provider"] == descriptor.provider())
            .unwrap_or_else(|| panic!("missing hook profile for {}", descriptor.provider()));
        assert!(provider["events"].as_array().unwrap().len() > 0);
    }

    #[tokio::test]
    async fn overview_route_exposes_hook_center_payload() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();

        let (status, value) = read_json(
            router(),
            Request::builder()
                .uri("/api/v1/hooks/overview")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let data = &value["data"];
        assert!(data["summary"]["providers"].as_u64().unwrap() > 0);
        assert!(data["providers"].as_array().unwrap().len() > 0);
        assert!(data["runtime_sessions"].as_array().is_some());
        assert_eq!(data["server"]["running"], false);
    }

    #[tokio::test]
    async fn provider_overview_route_exposes_provider_hook_detail() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();
        let provider = crate::hooks::registry::all()
            .first()
            .copied()
            .expect("at least one hook provider")
            .provider();

        let (status, value) = read_json(
            router(),
            Request::builder()
                .uri(format!("/api/v1/hooks/providers/{provider}/overview"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        let data = &value["data"];
        assert_eq!(data["provider"]["provider_id"], provider);
        assert!(data["runtime_sessions"].as_array().is_some());
        assert!(data["recent_events"].as_array().is_some());
        assert!(data["recent_errors"].as_array().is_some());
    }

    #[test]
    fn hook_operation_normalizes_setting_and_short_names() {
        use crate::hooks::strategies::HookConfigOperation;

        assert_eq!(
            normalize_hook_operation("install").unwrap(),
            HookConfigOperation::Install
        );
        assert_eq!(
            normalize_hook_operation("verify_hook").unwrap(),
            HookConfigOperation::Verify
        );
        assert_eq!(
            normalize_hook_operation("repair").unwrap(),
            HookConfigOperation::Repair
        );
        assert!(normalize_hook_operation("approve").is_err());
    }

    #[tokio::test]
    async fn hook_operation_route_rejects_unknown_operation() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/providers/claude/operations/approve")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["ok"], false);
    }

    #[tokio::test]
    async fn session_diagnosis_route_exposes_session_groups() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();

        let (status, value) = read_json(
            router(),
            Request::builder()
                .uri("/api/v1/hooks/session-diagnosis?hook_filter=attention&limit=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(value["data"].as_array().is_some());
    }

    #[tokio::test]
    async fn session_diagnosis_route_accepts_non_default_hook_filter() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();

        let (status, value) = read_json(
            router(),
            Request::builder()
                .uri("/api/v1/hooks/session-diagnosis?hook_filter=weak&limit=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert!(value["data"].as_array().is_some());
    }

    #[tokio::test]
    async fn ingest_requires_hook_token() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();
        set_runtime_endpoint_for_tests(HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "test-token".to_string(),
            pid: 1,
            started_at: Utc::now(),
        });
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
    async fn ingest_updates_runtime_sessions() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();
        let endpoint = HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "test-token".to_string(),
            pid: 1,
            started_at: Utc::now(),
        };
        set_runtime_endpoint_for_tests(endpoint.clone());
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

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("GET")
                .uri("/api/v1/hooks/runtime-sessions")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"].as_array().unwrap().len(), 1);
        assert_eq!(value["data"][0]["provider"], "generic");
        assert_eq!(value["data"][0]["status"], "running");
    }

    #[tokio::test]
    async fn diagnostics_route_redacts_runtime_token() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();
        set_runtime_endpoint_for_tests(HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "secret-token".to_string(),
            pid: 1,
            started_at: Utc::now(),
        });

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("GET")
                .uri("/api/v1/hooks/diagnostics")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);
        assert_eq!(value["data"]["server"]["token"], "redacted");
        assert_ne!(value["data"]["server"]["token"], "secret-token");
    }

    #[tokio::test]
    async fn doctor_route_checks_profiled_providers() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();
        let provider = crate::hooks::registry::all()
            .first()
            .copied()
            .expect("at least one hook provider")
            .provider();

        let request = crate::hooks::doctor::HookDoctorRequest {
            repair: false,
            providers: vec![provider.to_string(), "missing".to_string()],
        };
        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/doctor")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&request).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["checked"], 1);
        assert_eq!(value["data"]["results"][0]["provider"], provider);
    }

    #[tokio::test]
    async fn cleanup_route_marks_idle_runtime_sessions() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();
        let endpoint = HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "test-token".to_string(),
            pid: 1,
            started_at: Utc::now(),
        };
        set_runtime_endpoint_for_tests(endpoint.clone());
        let old_timestamp = (Utc::now() - chrono::Duration::seconds(10)).timestamp();
        let request = HookIngestRequest::new(
            "generic",
            "tool_started",
            json!({"session_id": "idle-session", "timestamp": old_timestamp, "tool_name": "Bash"}),
        );
        let (status, _) = read_json(
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

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/cleanup?idle_after_seconds=1&orphan_after_seconds=3600")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["idle"], 1);
    }

    #[tokio::test]
    async fn blocking_permission_ingest_records_without_decision() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();
        let endpoint = HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "test-token".to_string(),
            pid: 1,
            started_at: Utc::now(),
        };
        set_runtime_endpoint_for_tests(endpoint.clone());

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
        let data = value["data"].as_object().unwrap();
        assert_eq!(
            data.keys().cloned().collect::<Vec<_>>(),
            vec!["accepted", "event_ids"]
        );
    }

    #[tokio::test]
    async fn decision_and_pending_routes_are_not_registered() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();

        for (method, uri) in [
            ("GET", "/api/v1/hooks/pending"),
            ("GET", "/api/v1/hooks/pending/example"),
            ("POST", "/api/v1/hooks/pending/example/decision"),
            ("GET", "/api/v1/hooks/policy"),
            ("PUT", "/api/v1/hooks/policy"),
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
