//! Hook 运行时会话状态管理(核心层)。
//!
//! 运行时会话的快照、endpoint 发布、清理等纯逻辑。被核心模块
//! (agent_management / session_management / projection / augmentation /
//! diagnostics)直接调用;hooks HTTP handler(`hooks::server`)也共享同一份
//! 全局状态。本模块不依赖 axum,可独立留在核心 crate。

use anyhow::{Context as _, Result};
use chrono::Utc;
use std::sync::{OnceLock, RwLock};
use uuid::Uuid;

use crate::hooks::lifecycle::{self, RuntimeCleanupOptions};
use crate::hooks::model::RuntimeSession;
use crate::hooks::protocol::HookRuntimeEndpoint;
use crate::hooks::runtime::{RuntimeCleanupReport, RuntimeState};
use crate::hooks::store::{self, RuntimeSessionStore};

static RUNTIME_STATE: OnceLock<RwLock<RuntimeState>> = OnceLock::new();
static RUNTIME_ENDPOINT: OnceLock<RwLock<Option<HookRuntimeEndpoint>>> = OnceLock::new();

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
    let _ = cleanup_runtime_state(RuntimeCleanupOptions::default());
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

#[cfg(any(test, feature = "test-support"))]
pub fn linked_runtime_sessions_from_state(
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

pub fn runtime_state() -> &'static RwLock<RuntimeState> {
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

pub fn persist_runtime_state(state: &RuntimeState) -> Result<()> {
    store::save_runtime_sessions(&RuntimeSessionStore {
        version: 1,
        sessions: state.sessions.values().cloned().collect(),
    })
}

pub fn cleanup_runtime_state(options: RuntimeCleanupOptions) -> Result<RuntimeCleanupReport> {
    let mut state = runtime_state().write().unwrap();
    let report = state.cleanup_stale_sessions(
        Utc::now(),
        options.idle_after(),
        options.orphan_after(),
        lifecycle::pid_is_alive_with_start_time,
    );
    if report.idle > 0 || report.orphaned > 0 {
        persist_runtime_state(&state)?;
    }
    drop(state);
    Ok(report)
}

#[cfg(any(test, feature = "test-support"))]
pub fn reset_for_tests() {
    *runtime_state().write().unwrap() = RuntimeState::default();
    *runtime_endpoint_cell().write().unwrap() = None;
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_runtime_endpoint_for_tests(endpoint: HookRuntimeEndpoint) {
    *runtime_endpoint_cell().write().unwrap() = Some(endpoint);
}
