//! Canonical hook event and runtime data models.
//!
//! These types are provider-neutral. Provider-specific adapters should preserve
//! raw payloads while mapping the stable fields memorph needs for ingestion
//! and lightweight runtime state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use uuid::Uuid;

use crate::hooks::protocol::HookProcessInfo;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookEventType {
    SessionStarted,
    MessageCreated,
    ToolStarted,
    ToolFinished,
    PermissionRequested,
    QuestionRequested,
    SessionCompleted,
    SessionFailed,
    Heartbeat,
    Unknown,
}

impl HookEventType {
    pub fn from_provider_name(value: &str) -> Self {
        let normalized = value.trim().replace(['-', ' '], "_").to_ascii_lowercase();
        match normalized.as_str() {
            "session_started" | "session_start" | "start" | "started" => Self::SessionStarted,
            "message_created" | "message" | "assistant_message" | "user_message" => {
                Self::MessageCreated
            }
            "tool_started" | "tool_start" | "tool_call" | "pre_tool_use" | "pretooluse" => {
                Self::ToolStarted
            }
            "tool_finished" | "tool_finish" | "tool_end" | "post_tool_use" | "posttooluse" => {
                Self::ToolFinished
            }
            "permission_requested" | "permission_request" | "permission" => {
                Self::PermissionRequested
            }
            "question_requested" | "question_request" | "ask_user" | "askuserquestion" => {
                Self::QuestionRequested
            }
            "session_completed" | "session_complete" | "completed" | "stop" => {
                Self::SessionCompleted
            }
            "session_failed" | "session_fail" | "failed" | "error" => Self::SessionFailed,
            "heartbeat" | "ping" => Self::Heartbeat,
            _ => Self::Unknown,
        }
    }

    pub fn is_blocking(&self) -> bool {
        matches!(self, Self::PermissionRequested | Self::QuestionRequested)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub input: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PermissionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<HookToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct RuntimeSessionId(pub String);

impl RuntimeSessionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionFingerprint {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid_start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub terminal_vars: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_ancestry: Vec<HookProcessInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HookEvent {
    pub event_id: String,
    pub provider: String,
    pub event_type: HookEventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid_start_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub terminal_vars: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_ancestry: Vec<HookProcessInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<HookToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<HookMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<PermissionRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question: Option<QuestionRequest>,
    pub timestamp: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub raw: Value,
}

impl HookEvent {
    pub fn new(provider: impl Into<String>, event_type: HookEventType, raw: Value) -> Self {
        Self {
            event_id: Uuid::new_v4().to_string(),
            provider: provider.into(),
            event_type,
            provider_session_id: None,
            run_id: None,
            cwd: None,
            pid: None,
            parent_pid: None,
            pid_start_time: None,
            tty: None,
            terminal_vars: BTreeMap::new(),
            process_ancestry: Vec::new(),
            tool: None,
            message: None,
            permission: None,
            question: None,
            timestamp: Utc::now(),
            raw,
        }
    }

    pub fn fingerprint(&self) -> SessionFingerprint {
        SessionFingerprint {
            provider: self.provider.clone(),
            provider_session_id: self.provider_session_id.clone(),
            run_id: self.run_id.clone(),
            cwd: self.cwd.clone(),
            pid: self.pid,
            parent_pid: self.parent_pid,
            pid_start_time: self.pid_start_time.clone(),
            tty: self.tty.clone(),
            terminal_vars: self.terminal_vars.clone(),
            process_ancestry: self.process_ancestry.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSessionStatus {
    Running,
    WaitingPermission,
    WaitingUser,
    Completed,
    Failed,
    Orphaned,
    Idle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeSession {
    pub runtime_id: RuntimeSessionId,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    pub status: RuntimeSessionStatus,
    pub started_at: DateTime<Utc>,
    pub last_event_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HookHealthStatus {
    Unsupported,
    NotInstalled,
    InstalledDisabled,
    InstalledOk,
    InstalledStaleBinary,
    InstalledStaleEndpoint,
    InstalledBrokenConfig,
    InstalledConflict,
    Repairable,
    NeedsUserAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookInstallStatus {
    pub provider: String,
    pub status: HookHealthStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HookOperationReport {
    pub provider: String,
    pub operation: String,
    pub changed: bool,
    pub status: HookInstallStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_common_provider_event_names() {
        assert_eq!(
            HookEventType::from_provider_name("PreToolUse"),
            HookEventType::ToolStarted
        );
        assert_eq!(
            HookEventType::from_provider_name("permission_request"),
            HookEventType::PermissionRequested
        );
        assert_eq!(
            HookEventType::from_provider_name("stop"),
            HookEventType::SessionCompleted
        );
    }

    #[test]
    fn event_builds_fingerprint() {
        let mut event = HookEvent::new("sample", HookEventType::ToolStarted, Value::Null);
        event.provider_session_id = Some("abc".to_string());
        event.pid = Some(42);
        let fingerprint = event.fingerprint();
        assert_eq!(fingerprint.provider, "sample");
        assert_eq!(fingerprint.provider_session_id.as_deref(), Some("abc"));
        assert_eq!(fingerprint.pid, Some(42));
    }
}
