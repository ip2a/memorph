//! Hook diagnostics and support bundle generation.
//!
//! Reports hook server status, provider hook health, recent events, recent
//! errors, and persisted runtime sessions. This is intentionally read-only and
//! redacts the hook ingest token.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::hooks::capabilities::HookProviderCapabilities;
use crate::hooks::model::{HookEvent, HookInstallStatus, RuntimeSession};
use crate::hooks::protocol::HookRuntimeEndpoint;
use crate::hooks::store::{self, HookErrorRecord};
use crate::hooks::strategies::HookConfigStrategyKind;

const DEFAULT_EVENT_LIMIT: usize = 100;
const DEFAULT_ERROR_LIMIT: usize = 50;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookDiagnosticsReport {
    pub generated_at: DateTime<Utc>,
    pub store: HookDiagnosticsStorePaths,
    pub server: Option<HookDiagnosticsServer>,
    pub providers: Vec<HookDiagnosticsProvider>,
    pub runtime_sessions: Vec<RuntimeSession>,
    pub recent_events: Vec<HookEvent>,
    pub recent_errors: Vec<HookErrorRecord>,
    pub counts: HookDiagnosticsCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookDiagnosticsStorePaths {
    pub root: String,
    pub events: String,
    pub errors: String,
    pub runtime_sessions: String,
    pub server_runtime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookDiagnosticsServer {
    pub endpoint: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub token: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookDiagnosticsProvider {
    #[serde(flatten)]
    pub status: HookInstallStatus,
    pub strategy_kind: HookConfigStrategyKind,
    pub capabilities: HookProviderCapabilities,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookDiagnosticsCounts {
    pub providers: usize,
    pub runtime_sessions: usize,
    pub active_runtime_sessions: usize,
    pub recent_events: usize,
    pub recent_errors: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookDiagnosticsOptions {
    pub event_limit: usize,
    pub error_limit: usize,
}

impl Default for HookDiagnosticsOptions {
    fn default() -> Self {
        Self {
            event_limit: DEFAULT_EVENT_LIMIT,
            error_limit: DEFAULT_ERROR_LIMIT,
        }
    }
}

pub fn collect(options: HookDiagnosticsOptions) -> Result<HookDiagnosticsReport> {
    let paths = store::hook_store_paths()?;
    let providers = crate::hooks::registry::all()
        .into_iter()
        .map(|descriptor| {
            crate::hooks::operations::status(descriptor.provider()).map(|status| {
                HookDiagnosticsProvider {
                    status,
                    strategy_kind: descriptor.strategy_kind,
                    capabilities: descriptor.capabilities,
                }
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let runtime_store = store::load_runtime_sessions()?;
    let recent_events = store::load_recent_events(options.event_limit.min(1000))?;
    let recent_errors = store::load_recent_errors(options.error_limit.min(1000))?;
    let active_runtime_sessions = runtime_store
        .sessions
        .iter()
        .filter(|session| {
            !matches!(
                session.status,
                crate::hooks::model::RuntimeSessionStatus::Completed
                    | crate::hooks::model::RuntimeSessionStatus::Failed
            )
        })
        .count();

    Ok(HookDiagnosticsReport {
        generated_at: Utc::now(),
        store: HookDiagnosticsStorePaths {
            root: paths.root.display().to_string(),
            events: paths.events.display().to_string(),
            errors: paths.errors.display().to_string(),
            runtime_sessions: paths.runtime_sessions.display().to_string(),
            server_runtime: paths.server_runtime.display().to_string(),
        },
        server: crate::hooks::server::current_runtime_endpoint().map(redact_endpoint),
        counts: HookDiagnosticsCounts {
            providers: providers.len(),
            runtime_sessions: runtime_store.sessions.len(),
            active_runtime_sessions,
            recent_events: recent_events.len(),
            recent_errors: recent_errors.len(),
        },
        providers,
        runtime_sessions: runtime_store.sessions,
        recent_events,
        recent_errors,
    })
}

fn redact_endpoint(endpoint: HookRuntimeEndpoint) -> HookDiagnosticsServer {
    HookDiagnosticsServer {
        endpoint: endpoint.endpoint,
        pid: endpoint.pid,
        started_at: endpoint.started_at,
        token: "redacted".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_server_redacts_token() {
        let server = redact_endpoint(HookRuntimeEndpoint {
            endpoint: "http://127.0.0.1:3737".to_string(),
            token: "secret-token".to_string(),
            pid: 42,
            started_at: Utc::now(),
        });
        assert_eq!(server.token, "redacted");
        assert_eq!(server.pid, 42);
    }
}
