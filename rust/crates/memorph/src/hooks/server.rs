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
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};
use uuid::Uuid;

use crate::hooks::identity::runtime_session_id_for_event;
use crate::hooks::model::{
    HookEvent, HookEventType, PendingHookDecision, PendingHookRequest, PendingHookRequestKind,
    PendingHookRequestStatus, RuntimeSession, RuntimeSessionId, RuntimeSessionStatus,
};
use crate::hooks::normalizer;
use crate::hooks::policy::HookPolicyMode;
use crate::hooks::protocol::{
    HookDecision, HookIngestRequest, HookIngestResponse, HookRuntimeEndpoint,
};
use crate::hooks::runtime::{RuntimeCleanupReport, RuntimeState};
use crate::hooks::store::{self, PendingHookRequestStore, RuntimeSessionStore};
use crate::hooks::visibility::TerminalVisibilityContext;

static RUNTIME_STATE: OnceLock<RwLock<RuntimeState>> = OnceLock::new();
static RUNTIME_ENDPOINT: OnceLock<RwLock<Option<HookRuntimeEndpoint>>> = OnceLock::new();
static PENDING_STATE: OnceLock<RwLock<PendingHookRequestStore>> = OnceLock::new();

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
    pending_requests: usize,
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
    observed_blocking_requests: usize,
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
struct PendingRequestsQuery {
    provider: Option<String>,
    status: Option<String>,
    include_resolved: Option<bool>,
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
    pending_request_timeout_seconds: Option<i64>,
    resolved_pending_after_seconds: Option<i64>,
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
        .route("/api/v1/hooks/ingest", post(ingest_event))
        .route("/api/v1/hooks/events", get(list_events))
        .route("/api/v1/hooks/runtime-sessions", get(list_runtime_sessions))
        .route(
            "/api/v1/hooks/visibility/evaluate",
            post(evaluate_visibility),
        )
        .route("/api/v1/hooks/pending", get(list_pending_requests))
        .route("/api/v1/hooks/pending/{id}", get(get_pending_request))
        .route(
            "/api/v1/hooks/pending/{id}/decision",
            post(resolve_pending_request),
        )
        .route("/api/v1/hooks/diagnostics", get(get_diagnostics))
        .route("/api/v1/hooks/doctor", post(run_doctor))
        .route("/api/v1/hooks/cleanup", post(cleanup_runtime_sessions))
        .route("/api/v1/hooks/policy", get(get_policy).put(update_policy))
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

    match crate::core::list_sessions(&params) {
        Ok(groups) => HookApiResponse::success(groups).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn get_overview() -> impl IntoResponse {
    match build_overview() {
        Ok(payload) => HookApiResponse::success(payload).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn provider_overview(Path(provider): Path<String>) -> impl IntoResponse {
    match build_provider_overview(&provider) {
        Ok(payload) => HookApiResponse::success(payload).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn run_provider_hook_operation(
    Path((provider, operation)): Path<(String, String)>,
) -> impl IntoResponse {
    match run_hook_operation(&provider, &operation) {
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
    let pending_store = store::load_pending_requests().unwrap_or_default();
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
    let observed_blocking_requests = pending_store
        .requests
        .iter()
        .filter(|request| request.blocking)
        .count();

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
            observed_blocking_requests,
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

fn pending_state() -> &'static RwLock<PendingHookRequestStore> {
    PENDING_STATE.get_or_init(|| RwLock::new(store::load_pending_requests().unwrap_or_default()))
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

fn persist_pending_state(state: &PendingHookRequestStore) -> Result<()> {
    store::save_pending_requests(state)
}

async fn get_status() -> impl IntoResponse {
    let _ = cleanup_runtime_state(crate::hooks::lifecycle::RuntimeCleanupOptions::default());
    let state = runtime_state().read().unwrap();
    let pending = pending_state().read().unwrap();
    HookApiResponse::success(HookStatusPayload {
        server: current_hook_server_status(),
        runtime_sessions: state.sessions.len(),
        active_sessions: state.active_sessions().len(),
        pending_requests: pending
            .requests
            .iter()
            .filter(|request| request.status == PendingHookRequestStatus::Pending)
            .count(),
        providers: crate::hooks::registry::profiles(),
    })
    .into_response()
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
    if let Err(error) = authorize_ingest(&headers) {
        return hook_error(StatusCode::UNAUTHORIZED, error).into_response();
    }

    let mut events = match normalizer::normalize_request(&request) {
        Ok(events) => events,
        Err(error) => {
            let _ = store::append_error("normalize", error.to_string());
            return hook_error(StatusCode::BAD_REQUEST, error).into_response();
        }
    };
    for event in &mut events {
        enrich_event_from_environment(event, &request.environment);
    }

    let mut event_ids = Vec::new();
    let mut updates = Vec::new();
    for event in &events {
        event_ids.push(event.event_id.clone());
        if let Err(error) = store::append_event(event) {
            let _ = store::append_error("append_event", error.to_string());
            return hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
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
            return hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
        }
    }
    if !request.blocking {
        for event in &events {
            if let Err(error) = finalize_pending_requests_if_provider_continued(event) {
                let _ = store::append_error(
                    "finalize_pending_requests_if_provider_continued",
                    error.to_string(),
                );
                return hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
            }
        }
    }

    let policy = store::load_policy().unwrap_or_default();
    let evaluation = crate::hooks::policy::effective_decision(&policy, &events, request.blocking);
    let response = match evaluation {
        Some(evaluation) if evaluation.mode == HookPolicyMode::AskUser => {
            match first_user_action_event(&events, request.blocking)
                .map(|event| queue_pending_request(event, &request))
                .transpose()
            {
                Ok(Some(pending)) => {
                    HookIngestResponse::waiting_for_user(event_ids, pending.id.clone())
                }
                Ok(None) => HookIngestResponse::with_decision(event_ids, evaluation.decision),
                Err(error) => {
                    let _ = store::append_error("queue_pending_request", error.to_string());
                    return hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
                }
            }
        }
        Some(evaluation) => HookIngestResponse::with_decision(event_ids, evaluation.decision),
        None => HookIngestResponse::accepted(event_ids),
    };
    HookApiResponse::success(response).into_response()
}

async fn get_policy() -> impl IntoResponse {
    match store::load_policy() {
        Ok(policy) => HookApiResponse::success(policy).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn update_policy(Json(policy): Json<crate::hooks::policy::HookPolicy>) -> impl IntoResponse {
    match store::save_policy(&policy) {
        Ok(()) => HookApiResponse::success(policy).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn list_events(Query(query): Query<EventsQuery>) -> impl IntoResponse {
    match store::load_recent_events(query.limit.unwrap_or(100).min(1000)) {
        Ok(events) => HookApiResponse::success(events).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn list_runtime_sessions(Query(query): Query<RuntimeSessionsQuery>) -> impl IntoResponse {
    let sessions: Vec<RuntimeSession> = runtime_sessions_snapshot()
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
        .collect();
    HookApiResponse::success(sessions).into_response()
}

async fn evaluate_visibility(Json(context): Json<TerminalVisibilityContext>) -> impl IntoResponse {
    let _ = cleanup_runtime_state(crate::hooks::lifecycle::RuntimeCleanupOptions::default());
    let state = runtime_state().read().unwrap();
    let sessions: Vec<RuntimeSession> = state.sessions.values().cloned().collect();
    let decisions = crate::hooks::visibility::evaluate_sessions(&sessions, &context);
    HookApiResponse::success(decisions).into_response()
}

async fn list_pending_requests(Query(query): Query<PendingRequestsQuery>) -> impl IntoResponse {
    let _ = cleanup_runtime_state(crate::hooks::lifecycle::RuntimeCleanupOptions::default());
    let state = pending_state().read().unwrap();
    let include_resolved = query.include_resolved.unwrap_or(false);
    let requests: Vec<PendingHookRequest> = state
        .requests
        .iter()
        .filter(|request| {
            query
                .provider
                .as_deref()
                .map(|provider| request.provider == provider)
                .unwrap_or(true)
        })
        .filter(|request| {
            if let Some(status) = query.status.as_deref() {
                return pending_status_matches(&request.status, status);
            }
            include_resolved || request.status == PendingHookRequestStatus::Pending
        })
        .cloned()
        .collect();
    HookApiResponse::success(requests).into_response()
}

async fn get_pending_request(Path(id): Path<String>) -> impl IntoResponse {
    let _ = cleanup_runtime_state(crate::hooks::lifecycle::RuntimeCleanupOptions::default());
    let state = pending_state().read().unwrap();
    match state
        .requests
        .iter()
        .find(|request| request.id == id)
        .cloned()
    {
        Some(request) => HookApiResponse::success(request).into_response(),
        None => hook_error(StatusCode::NOT_FOUND, "Pending hook request not found").into_response(),
    }
}

async fn resolve_pending_request(
    Path(id): Path<String>,
    Json(decision): Json<PendingHookDecision>,
) -> impl IntoResponse {
    if !is_final_decision(&decision.decision) {
        return hook_error(
            StatusCode::BAD_REQUEST,
            "Pending hook request decision must be final",
        )
        .into_response();
    }

    let resolved = {
        let mut pending = pending_state().write().unwrap();
        let Some(request) = pending.requests.iter_mut().find(|request| request.id == id) else {
            return hook_error(StatusCode::NOT_FOUND, "Pending hook request not found")
                .into_response();
        };
        if request.status != PendingHookRequestStatus::Pending {
            return hook_error(
                StatusCode::CONFLICT,
                "Pending hook request has already been finalized",
            )
            .into_response();
        }

        let now = Utc::now();
        request.status = PendingHookRequestStatus::Resolved;
        request.decision = Some(decision.decision);
        request.response_text = decision.response_text;
        request.note = decision.note;
        request.resolved_at = Some(now);
        request.updated_at = now;
        let resolved = request.clone();

        if let Err(error) = persist_pending_state(&pending) {
            let _ = store::append_error("persist_pending_state", error.to_string());
            return hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
        }
        resolved
    };

    if let Err(error) = clear_runtime_pending_request(&resolved.runtime_id, &resolved.kind) {
        let _ = store::append_error("clear_runtime_pending_request", error.to_string());
        return hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response();
    }

    HookApiResponse::success(resolved).into_response()
}

async fn provider_status(Path(provider): Path<String>) -> impl IntoResponse {
    match crate::hooks::operations::status(&provider) {
        Ok(status) => HookApiResponse::success(status).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn get_diagnostics(Query(query): Query<DiagnosticsQuery>) -> impl IntoResponse {
    let options = crate::hooks::diagnostics::HookDiagnosticsOptions {
        event_limit: query.event_limit.unwrap_or(100).min(1000),
        error_limit: query.error_limit.unwrap_or(50).min(1000),
    };
    match crate::hooks::diagnostics::collect(options) {
        Ok(report) => HookApiResponse::success(report).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn run_doctor(
    Json(request): Json<crate::hooks::doctor::HookDoctorRequest>,
) -> impl IntoResponse {
    match crate::hooks::doctor::verify(request) {
        Ok(report) => HookApiResponse::success(report).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

async fn cleanup_runtime_sessions(Query(query): Query<CleanupQuery>) -> impl IntoResponse {
    let options = crate::hooks::lifecycle::RuntimeCleanupOptions {
        idle_after_seconds: query.idle_after_seconds.unwrap_or(30 * 60),
        orphan_after_seconds: query.orphan_after_seconds.unwrap_or(60 * 60),
        pending_request_timeout_seconds: query.pending_request_timeout_seconds.unwrap_or(5 * 60),
        resolved_pending_after_seconds: query
            .resolved_pending_after_seconds
            .unwrap_or(24 * 60 * 60),
    };
    match cleanup_runtime_state(options) {
        Ok(report) => HookApiResponse::success(report).into_response(),
        Err(error) => hook_error(StatusCode::INTERNAL_SERVER_ERROR, error).into_response(),
    }
}

fn cleanup_runtime_state(
    options: crate::hooks::lifecycle::RuntimeCleanupOptions,
) -> Result<RuntimeCleanupReport> {
    let mut state = runtime_state().write().unwrap();
    let mut report = state.cleanup_stale_sessions(
        Utc::now(),
        options.idle_after(),
        options.orphan_after(),
        crate::hooks::lifecycle::pid_is_alive_with_start_time,
    );
    if report.idle > 0 || report.orphaned > 0 {
        persist_runtime_state(&state)?;
    }
    drop(state);
    report.expired_pending =
        expire_pending_requests(Utc::now() - options.pending_request_timeout())?;
    report.resolved_pending_removed =
        cleanup_resolved_pending_requests(Utc::now() - options.resolved_pending_after())?;
    Ok(report)
}

fn cleanup_resolved_pending_requests(older_than: chrono::DateTime<Utc>) -> Result<usize> {
    let mut pending = pending_state().write().unwrap();
    let before = pending.requests.len();
    pending.requests.retain(|request| {
        request.status == PendingHookRequestStatus::Pending || request.updated_at >= older_than
    });
    let removed = before.saturating_sub(pending.requests.len());
    if removed > 0 {
        persist_pending_state(&pending)?;
    }
    Ok(removed)
}

fn expire_pending_requests(older_than: chrono::DateTime<Utc>) -> Result<usize> {
    let mut expired = Vec::new();
    {
        let mut pending = pending_state().write().unwrap();
        let now = Utc::now();
        for request in pending.requests.iter_mut().filter(|request| {
            request.status == PendingHookRequestStatus::Pending && request.created_at < older_than
        }) {
            request.status = PendingHookRequestStatus::Expired;
            request.decision = Some(HookDecision::ProviderDefault);
            request.response_text = None;
            request.note = Some("Timed out waiting for memorph user decision".to_string());
            request.resolved_at = Some(now);
            request.updated_at = now;
            expired.push((request.runtime_id.clone(), request.kind.clone()));
        }
        if !expired.is_empty() {
            persist_pending_state(&pending)?;
        }
    }

    for (runtime_id, kind) in &expired {
        clear_runtime_pending_request(runtime_id, kind)?;
    }
    Ok(expired.len())
}

fn finalize_pending_requests_if_provider_continued(event: &HookEvent) -> Result<usize> {
    if !event_finalizes_existing_pending(event) {
        return Ok(0);
    }

    let runtime_id = runtime_session_id_for_event(event);
    let mut finalized = Vec::new();
    {
        let mut pending = pending_state().write().unwrap();
        let now = Utc::now();
        for request in pending.requests.iter_mut().filter(|request| {
            request.status == PendingHookRequestStatus::Pending
                && request.provider == event.provider
                && request.runtime_id == runtime_id
        }) {
            request.status = PendingHookRequestStatus::Resolved;
            request.decision = Some(HookDecision::ProviderDefault);
            request.response_text = None;
            request.note = Some("Provider continued before memorph user decision".to_string());
            request.resolved_at = Some(now);
            request.updated_at = now;
            finalized.push((request.runtime_id.clone(), request.kind.clone()));
        }
        if !finalized.is_empty() {
            persist_pending_state(&pending)?;
        }
    }

    for (runtime_id, kind) in &finalized {
        clear_runtime_pending_request(runtime_id, kind)?;
    }
    Ok(finalized.len())
}

fn event_finalizes_existing_pending(event: &HookEvent) -> bool {
    matches!(
        event.event_type,
        HookEventType::MessageCreated
            | HookEventType::ToolStarted
            | HookEventType::ToolFinished
            | HookEventType::SessionCompleted
            | HookEventType::SessionFailed
    )
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

fn first_user_action_event(events: &[HookEvent], request_blocking: bool) -> Option<&HookEvent> {
    events.iter().find(|event| {
        matches!(
            event.event_type,
            HookEventType::PermissionRequested | HookEventType::QuestionRequested
        ) || request_blocking
    })
}

fn queue_pending_request(
    event: &HookEvent,
    request: &HookIngestRequest,
) -> Result<PendingHookRequest> {
    let runtime_id = runtime_session_id_for_event(event);
    let kind = pending_kind_for_event(event);
    let provider_request_id = pending_provider_request_id(event);
    let tool = pending_tool_for_event(event);
    let prompt = pending_prompt_for_event(event);
    let mut pending = pending_state().write().unwrap();
    if let Some(existing) = pending.requests.iter().find(|pending| {
        pending.status == PendingHookRequestStatus::Pending
            && pending.provider == event.provider
            && pending_matches_event(
                pending,
                &runtime_id,
                &kind,
                provider_request_id.as_deref(),
                tool.as_ref().and_then(|tool| tool.id.as_deref()),
                &request.request_id,
            )
    }) {
        return Ok(existing.clone());
    }

    let now = Utc::now();
    let request = PendingHookRequest {
        id: Uuid::new_v4().to_string(),
        kind,
        status: PendingHookRequestStatus::Pending,
        provider: event.provider.clone(),
        runtime_id,
        event_id: event.event_id.clone(),
        hook_request_id: request.request_id.clone(),
        provider_request_id,
        provider_session_id: event.provider_session_id.clone(),
        tool,
        prompt,
        blocking: request.blocking,
        created_at: now,
        updated_at: now,
        resolved_at: None,
        decision: None,
        response_text: None,
        note: None,
    };
    pending.requests.push(request.clone());
    persist_pending_state(&pending)?;
    Ok(request)
}

fn pending_kind_for_event(event: &HookEvent) -> PendingHookRequestKind {
    if event.event_type == HookEventType::QuestionRequested {
        PendingHookRequestKind::Question
    } else {
        PendingHookRequestKind::Permission
    }
}

fn pending_provider_request_id(event: &HookEvent) -> Option<String> {
    match event.event_type {
        HookEventType::QuestionRequested => event
            .question
            .as_ref()
            .and_then(|question| question.request_id.clone()),
        _ => event
            .permission
            .as_ref()
            .and_then(|permission| permission.request_id.clone()),
    }
}

fn pending_tool_for_event(event: &HookEvent) -> Option<crate::hooks::model::HookToolCall> {
    event.tool.clone().or_else(|| {
        event
            .permission
            .as_ref()
            .and_then(|permission| permission.tool.clone())
    })
}

fn pending_prompt_for_event(event: &HookEvent) -> Option<String> {
    event
        .question
        .as_ref()
        .map(|question| question.prompt.clone())
        .or_else(|| {
            event
                .permission
                .as_ref()
                .and_then(|permission| permission.prompt.clone())
        })
}

fn pending_matches_event(
    pending: &PendingHookRequest,
    runtime_id: &RuntimeSessionId,
    kind: &PendingHookRequestKind,
    provider_request_id: Option<&str>,
    tool_id: Option<&str>,
    hook_request_id: &str,
) -> bool {
    if &pending.runtime_id != runtime_id || &pending.kind != kind {
        return false;
    }
    if let Some(provider_request_id) = provider_request_id {
        return pending.provider_request_id.as_deref() == Some(provider_request_id);
    }
    if let Some(tool_id) = tool_id {
        return pending.tool.as_ref().and_then(|tool| tool.id.as_deref()) == Some(tool_id);
    }
    pending.hook_request_id == hook_request_id
}

fn pending_status_matches(status: &PendingHookRequestStatus, expected: &str) -> bool {
    match expected.trim().to_ascii_lowercase().as_str() {
        "pending" => status == &PendingHookRequestStatus::Pending,
        "resolved" => status == &PendingHookRequestStatus::Resolved,
        "expired" => status == &PendingHookRequestStatus::Expired,
        _ => false,
    }
}

fn is_final_decision(decision: &HookDecision) -> bool {
    matches!(
        decision,
        HookDecision::Allow
            | HookDecision::Deny
            | HookDecision::Ignore
            | HookDecision::RecordOnly
            | HookDecision::ProviderDefault
    )
}

fn clear_runtime_pending_request(
    runtime_id: &RuntimeSessionId,
    kind: &PendingHookRequestKind,
) -> Result<()> {
    let mut state = runtime_state().write().unwrap();
    if let Some(session) = state.sessions.get_mut(runtime_id) {
        match kind {
            PendingHookRequestKind::Permission => {
                session.pending_permission = None;
            }
            PendingHookRequestKind::Question => {
                session.pending_question = None;
            }
        }
        if !matches!(
            session.status,
            RuntimeSessionStatus::Completed | RuntimeSessionStatus::Failed
        ) {
            session.status = RuntimeSessionStatus::Running;
        }
        session.updated_at = Utc::now();
        persist_runtime_state(&state)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    *runtime_state().write().unwrap() = RuntimeState::default();
    *runtime_endpoint_cell().write().unwrap() = None;
    *pending_state().write().unwrap() = PendingHookRequestStore::default();
}

#[cfg(test)]
pub(crate) fn set_runtime_endpoint_for_tests(endpoint: HookRuntimeEndpoint) {
    *runtime_endpoint_cell().write().unwrap() = Some(endpoint);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::RuntimeSessionCorrelation;
    use axum::{body::to_bytes, body::Body, http::Request};
    use chrono::Utc;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};
    use tower::util::ServiceExt;

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    async fn read_json(app: Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        (status, value)
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
    async fn visibility_route_evaluates_runtime_sessions() {
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
            "tool_started",
            json!({
                "session_id": "session-1",
                "tool": {"name": "Bash"}
            }),
        );
        request
            .environment
            .vars
            .insert("TERM_PROGRAM".to_string(), "iTerm.app".to_string());
        request
            .environment
            .vars
            .insert("TMUX_PANE".to_string(), "%12".to_string());

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

        let context = json!({
            "frontmost_bundle_id": "com.googlecode.iterm2",
            "active_tmux_pane": "%12"
        });
        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/visibility/evaluate")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&context).unwrap()))
                .unwrap(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"][0]["suppress_notification"], true);
        assert_eq!(value["data"][0]["matched_by"], "tmux_pane");
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
    async fn policy_route_round_trips_policy() {
        let _guard = test_guard();
        let dir = tempfile::tempdir().unwrap();
        store::set_test_store_root(dir.path().to_path_buf());
        reset_for_tests();

        let policy = crate::hooks::policy::HookPolicy {
            global: crate::hooks::policy::HookPolicyMode::Allow,
            ..crate::hooks::policy::HookPolicy::default()
        };
        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("PUT")
                .uri("/api/v1/hooks/policy")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&policy).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["global"], "allow");

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("GET")
                .uri("/api/v1/hooks/policy")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["global"], "allow");
    }

    async fn ingest_permission_with_policy(
        mode: crate::hooks::policy::HookPolicyMode,
    ) -> serde_json::Value {
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
        store::save_policy(&crate::hooks::policy::HookPolicy {
            global: mode,
            ..crate::hooks::policy::HookPolicy::default()
        })
        .unwrap();

        let mut request = HookIngestRequest::new(
            "generic",
            "permission_requested",
            json!({
                "session_id": "session-policy",
                "permission_prompt": "Allow shell command?",
                "tool": {"name": "Bash", "input": {"command": "cargo test"}}
            }),
        );
        request.blocking = true;
        request.request_id = "policy-request-1".to_string();
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
        value
    }

    #[tokio::test]
    async fn final_policy_modes_return_decision_without_pending_request() {
        let _guard = test_guard();
        for (mode, expected) in [
            (crate::hooks::policy::HookPolicyMode::Allow, "allow"),
            (crate::hooks::policy::HookPolicyMode::Deny, "deny"),
            (
                crate::hooks::policy::HookPolicyMode::RecordOnly,
                "record_only",
            ),
        ] {
            let value = ingest_permission_with_policy(mode).await;
            assert_eq!(value["data"]["decision"], expected);
            assert!(value["data"]["pending_request_id"].is_null());
            let pending = store::load_pending_requests().unwrap();
            assert!(pending.requests.is_empty());
        }
    }

    #[tokio::test]
    async fn ask_user_policy_queues_pending_request_and_resolves_it() {
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
        store::save_policy(&crate::hooks::policy::HookPolicy {
            global: crate::hooks::policy::HookPolicyMode::AskUser,
            ..crate::hooks::policy::HookPolicy::default()
        })
        .unwrap();

        let mut request = HookIngestRequest::new(
            "generic",
            "permission_requested",
            json!({
                "session_id": "session-1",
                "permission_prompt": "Allow shell command?",
                "tool": {"name": "Bash", "input": {"command": "cargo test"}}
            }),
        );
        request.blocking = true;
        request.request_id = "hook-request-1".to_string();
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
        assert_eq!(value["data"]["decision"], "ask_user");
        let pending_id = value["data"]["pending_request_id"].as_str().unwrap();

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("GET")
                .uri("/api/v1/hooks/pending")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"].as_array().unwrap().len(), 1);
        assert_eq!(value["data"][0]["status"], "pending");
        assert!(value["data"][0]["decision"].is_null());
        assert_eq!(value["data"][0]["prompt"], "Allow shell command?");

        let decision = PendingHookDecision {
            decision: HookDecision::Allow,
            response_text: None,
            note: Some("approved in test".to_string()),
        };
        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/hooks/pending/{pending_id}/decision"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&decision).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["status"], "resolved");
        assert_eq!(value["data"]["decision"], "allow");

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
        assert_eq!(value["data"][0]["status"], "running");
        assert!(value["data"][0]["pending_permission"].is_null());
    }

    #[tokio::test]
    async fn ask_user_policy_deduplicates_same_hook_request() {
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
        store::save_policy(&crate::hooks::policy::HookPolicy {
            global: crate::hooks::policy::HookPolicyMode::AskUser,
            ..crate::hooks::policy::HookPolicy::default()
        })
        .unwrap();

        let mut request = HookIngestRequest::new(
            "generic",
            "permission_requested",
            json!({"session_id": "session-dup", "tool_name": "Bash"}),
        );
        request.blocking = true;
        request.request_id = "duplicate-hook-request".to_string();
        let body = serde_json::to_vec(&request).unwrap();
        let mut pending_ids = Vec::new();
        for _ in 0..2 {
            let (status, value) = read_json(
                router(),
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hooks/ingest")
                    .header("content-type", "application/json")
                    .header("x-memorph-hook-token", endpoint.token.clone())
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            pending_ids.push(
                value["data"]["pending_request_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }
        assert_eq!(pending_ids[0], pending_ids[1]);
        assert_eq!(store::load_pending_requests().unwrap().requests.len(), 1);
    }

    #[tokio::test]
    async fn ask_user_policy_deduplicates_same_provider_request_across_bridge_retries() {
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
        store::save_policy(&crate::hooks::policy::HookPolicy {
            global: crate::hooks::policy::HookPolicyMode::AskUser,
            ..crate::hooks::policy::HookPolicy::default()
        })
        .unwrap();

        let mut pending_ids = Vec::new();
        for bridge_request_id in ["bridge-request-1", "bridge-request-2"] {
            let mut request = HookIngestRequest::new(
                "generic",
                "permission_requested",
                json!({
                    "session_id": "session-provider-retry",
                    "permission_id": "provider-permission-1",
                    "tool": {"id": "tool-1", "name": "Bash"}
                }),
            );
            request.blocking = true;
            request.request_id = bridge_request_id.to_string();
            let (status, value) = read_json(
                router(),
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/hooks/ingest")
                    .header("content-type", "application/json")
                    .header("x-memorph-hook-token", endpoint.token.clone())
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            pending_ids.push(
                value["data"]["pending_request_id"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            );
        }

        let pending = store::load_pending_requests().unwrap();
        assert_eq!(pending_ids[0], pending_ids[1]);
        assert_eq!(pending.requests.len(), 1);
        assert_eq!(
            pending.requests[0].provider_request_id.as_deref(),
            Some("provider-permission-1")
        );
    }

    #[tokio::test]
    async fn cleanup_expires_stale_pending_request_and_clears_runtime_waiting_state() {
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
        store::save_policy(&crate::hooks::policy::HookPolicy {
            global: crate::hooks::policy::HookPolicyMode::AskUser,
            ..crate::hooks::policy::HookPolicy::default()
        })
        .unwrap();

        let mut request = HookIngestRequest::new(
            "generic",
            "permission_requested",
            json!({"session_id": "session-expire", "tool_name": "Bash"}),
        );
        request.blocking = true;
        request.request_id = "expire-hook-request".to_string();
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
        let pending_id = value["data"]["pending_request_id"].as_str().unwrap();

        {
            let mut pending = pending_state().write().unwrap();
            let stale = pending
                .requests
                .iter_mut()
                .find(|request| request.id == pending_id)
                .unwrap();
            stale.created_at = Utc::now() - chrono::Duration::seconds(10);
            stale.updated_at = stale.created_at;
            persist_pending_state(&pending).unwrap();
        }

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/cleanup?pending_request_timeout_seconds=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["expired_pending"], 1);

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/hooks/pending/{pending_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["status"], "expired");
        assert_eq!(value["data"]["decision"], "provider_default");

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("GET")
                .uri("/api/v1/hooks/pending")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(value["data"].as_array().unwrap().is_empty());

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
        assert_eq!(value["data"][0]["status"], "running");
        assert!(value["data"][0]["pending_permission"].is_null());
    }

    #[tokio::test]
    async fn provider_progress_finalizes_existing_pending_request() {
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
        store::save_policy(&crate::hooks::policy::HookPolicy {
            global: crate::hooks::policy::HookPolicyMode::AskUser,
            ..crate::hooks::policy::HookPolicy::default()
        })
        .unwrap();

        let mut permission = HookIngestRequest::new(
            "generic",
            "permission_requested",
            json!({"session_id": "session-progress", "tool_name": "Bash"}),
        );
        permission.blocking = true;
        permission.request_id = "progress-permission-request".to_string();
        let (_, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/ingest")
                .header("content-type", "application/json")
                .header("x-memorph-hook-token", endpoint.token.clone())
                .body(Body::from(serde_json::to_vec(&permission).unwrap()))
                .unwrap(),
        )
        .await;
        let pending_id = value["data"]["pending_request_id"].as_str().unwrap();

        let progress = HookIngestRequest::new(
            "generic",
            "tool_started",
            json!({"session_id": "session-progress", "tool": {"name": "Bash"}}),
        );
        let (status, _) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/ingest")
                .header("content-type", "application/json")
                .header("x-memorph-hook-token", endpoint.token)
                .body(Body::from(serde_json::to_vec(&progress).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("GET")
                .uri(format!("/api/v1/hooks/pending/{pending_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["data"]["status"], "resolved");
        assert_eq!(value["data"]["decision"], "provider_default");
        assert_eq!(
            value["data"]["note"],
            "Provider continued before memorph user decision"
        );

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("GET")
                .uri("/api/v1/hooks/pending")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(value["data"].as_array().unwrap().is_empty());

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
        assert_eq!(value["data"][0]["status"], "running");
        assert!(value["data"][0]["pending_permission"].is_null());
    }

    #[tokio::test]
    async fn resolved_pending_request_cannot_be_resolved_again() {
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
        store::save_policy(&crate::hooks::policy::HookPolicy {
            global: crate::hooks::policy::HookPolicyMode::AskUser,
            ..crate::hooks::policy::HookPolicy::default()
        })
        .unwrap();

        let mut request = HookIngestRequest::new(
            "generic",
            "permission_requested",
            json!({"session_id": "session-double", "tool_name": "Bash"}),
        );
        request.blocking = true;
        request.request_id = "double-resolve-hook-request".to_string();
        let (_, value) = read_json(
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
        let pending_id = value["data"]["pending_request_id"].as_str().unwrap();
        let decision = PendingHookDecision {
            decision: HookDecision::Allow,
            response_text: None,
            note: None,
        };

        let (status, _) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/hooks/pending/{pending_id}/decision"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&decision).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri(format!("/api/v1/hooks/pending/{pending_id}/decision"))
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&decision).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(value["ok"], false);
    }
}
