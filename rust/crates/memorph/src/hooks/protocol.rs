//! Wire protocol between provider hooks, the internal bridge, and the integrated hook API.
//!
//! These payloads are intentionally transport-neutral. The first transport will
//! be the existing Axum API, but the same request/response shapes can be reused
//! by a Unix socket transport later if needed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HookProcessInfo {
    pub pid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_time: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct HookBridgeEnvironment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_ancestry: Vec<HookProcessInfo>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub vars: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookIngestRequest {
    pub request_id: String,
    pub provider: String,
    pub event_name: String,
    #[serde(default)]
    pub blocking: bool,
    #[serde(default)]
    pub environment: HookBridgeEnvironment,
    pub raw: Value,
    pub received_at: DateTime<Utc>,
}

impl HookIngestRequest {
    pub fn new(provider: impl Into<String>, event_name: impl Into<String>, raw: Value) -> Self {
        Self {
            request_id: Uuid::new_v4().to_string(),
            provider: provider.into(),
            event_name: event_name.into(),
            blocking: false,
            environment: HookBridgeEnvironment::default(),
            raw,
            received_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookDecision {
    Allow,
    Deny,
    AskUser,
    Ignore,
    RecordOnly,
    ProviderDefault,
}

impl Default for HookDecision {
    fn default() -> Self {
        Self::RecordOnly
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookIngestResponse {
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<HookDecision>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl HookIngestResponse {
    pub fn accepted(event_ids: Vec<String>) -> Self {
        Self {
            accepted: true,
            event_ids,
            decision: None,
            pending_request_id: None,
            response_text: None,
            message: None,
        }
    }

    pub fn with_decision(event_ids: Vec<String>, decision: HookDecision) -> Self {
        Self {
            accepted: true,
            event_ids,
            decision: Some(decision),
            pending_request_id: None,
            response_text: None,
            message: None,
        }
    }

    pub fn waiting_for_user(event_ids: Vec<String>, pending_request_id: String) -> Self {
        Self {
            accepted: true,
            event_ids,
            decision: Some(HookDecision::AskUser),
            pending_request_id: Some(pending_request_id),
            response_text: None,
            message: Some("Waiting for memorph user decision".to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookRuntimeEndpoint {
    pub endpoint: String,
    pub token: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn request_constructor_sets_required_fields() {
        let request = HookIngestRequest::new("claude", "pre_tool_use", json!({"tool": "Bash"}));
        assert_eq!(request.provider, "claude");
        assert_eq!(request.event_name, "pre_tool_use");
        assert!(!request.blocking);
        assert!(!request.request_id.is_empty());
    }
}
