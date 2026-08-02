//! Process-local hook runtime state and endpoint publication.

use anyhow::{Context as _, Result};
use chrono::Utc;
use std::sync::{OnceLock, RwLock};
use uuid::Uuid;

use crate::hooks::model::RuntimeSession;
use crate::hooks::protocol::HookRuntimeEndpoint;
use crate::hooks::runtime::RuntimeState;
use crate::hooks::store;

static RUNTIME_STATE: OnceLock<RwLock<RuntimeState>> = OnceLock::new();
static RUNTIME_ENDPOINT: OnceLock<RwLock<Option<HookRuntimeEndpoint>>> = OnceLock::new();

pub fn publish_runtime_endpoint(endpoint: &str) -> Result<HookRuntimeEndpoint> {
    let endpoint = HookRuntimeEndpoint {
        endpoint: endpoint.trim_end_matches('/').to_string(),
        token: current_or_new_token(),
        pid: std::process::id(),
        started_at: Utc::now(),
    };
    *runtime_endpoint_cell().write().unwrap() = Some(endpoint.clone());
    store::save_server_runtime(&endpoint).context("Failed to save hook server runtime endpoint")?;
    Ok(endpoint)
}

pub fn current_runtime_endpoint() -> Option<HookRuntimeEndpoint> {
    runtime_endpoint_cell().read().unwrap().clone()
}

pub fn runtime_sessions_snapshot() -> Vec<RuntimeSession> {
    let state = runtime_state().read().unwrap();
    state.sessions.values().cloned().collect()
}

pub fn runtime_state() -> &'static RwLock<RuntimeState> {
    RUNTIME_STATE.get_or_init(|| RwLock::new(RuntimeState::default()))
}

fn current_or_new_token() -> String {
    current_runtime_endpoint()
        .map(|endpoint| endpoint.token)
        .filter(|token| !token.trim().is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string())
}

fn runtime_endpoint_cell() -> &'static RwLock<Option<HookRuntimeEndpoint>> {
    RUNTIME_ENDPOINT.get_or_init(|| RwLock::new(store::load_server_runtime().unwrap_or(None)))
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
