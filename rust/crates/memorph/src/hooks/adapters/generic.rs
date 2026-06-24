//! Generic JSON hook adapter.
//!
//! This adapter accepts memorph-shaped JSON and simple provider-neutral aliases.
//! It exists to validate the hook pipeline and support user-defined providers
//! without hiding missing first-class provider adapters.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::PathBuf;

use crate::hooks::contract::HookAdapter;
use crate::hooks::model::{
    HookEvent, HookEventType, HookMessage, HookToolCall, PermissionRequest, QuestionRequest,
};
use crate::hooks::protocol::HookIngestRequest;

pub struct GenericHookAdapter;

impl HookAdapter for GenericHookAdapter {
    fn provider_id(&self) -> &'static str {
        "generic"
    }

    fn normalize(&self, request: &HookIngestRequest) -> Result<Vec<HookEvent>> {
        let raw = request.raw.clone();
        let provider =
            string_at(&raw, &["provider", "source"]).unwrap_or_else(|| request.provider.clone());
        let event_name = string_at(&raw, &["event_type", "eventType", "type", "event"])
            .unwrap_or_else(|| request.event_name.clone());
        let mut event = HookEvent::new(
            provider,
            HookEventType::from_provider_name(&event_name),
            raw.clone(),
        );

        event.event_id = string_at(&raw, &["event_id", "eventId", "id"]).unwrap_or(event.event_id);
        event.provider_session_id = string_at(
            &raw,
            &[
                "provider_session_id",
                "session_id",
                "sessionId",
                "conversation_id",
            ],
        );
        event.run_id = string_at(&raw, &["run_id", "runId", "request_id", "requestId"])
            .or_else(|| Some(request.request_id.clone()));
        event.cwd = path_at(&raw, &["cwd", "workspace", "workspace_dir", "project_dir"])
            .or_else(|| request.environment.cwd.clone());
        event.pid = u32_at(&raw, &["pid"]).or(request.environment.pid);
        event.parent_pid =
            u32_at(&raw, &["parent_pid", "parentPid", "ppid"]).or(request.environment.parent_pid);
        event.tty =
            string_at(&raw, &["tty", "terminal_tty"]).or_else(|| request.environment.tty.clone());
        event.timestamp = timestamp_at(&raw).unwrap_or(request.received_at);
        event.tool = tool_at(&raw);
        event.message = message_at(&raw);
        event.permission = permission_at(&raw, event.tool.clone());
        event.question = question_at(&raw);

        Ok(vec![event])
    }
}

fn string_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|raw| match raw {
            Value::String(s) if !s.trim().is_empty() => Some(s.to_string()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
    })
}

fn u32_at(value: &Value, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|raw| match raw {
            Value::Number(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
            Value::String(s) => s.parse::<u32>().ok(),
            _ => None,
        })
    })
}

fn path_at(value: &Value, keys: &[&str]) -> Option<PathBuf> {
    string_at(value, keys).map(PathBuf::from)
}

fn timestamp_at(value: &Value) -> Option<DateTime<Utc>> {
    let raw = value.get("timestamp").or_else(|| value.get("created_at"))?;
    match raw {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|dt| dt.with_timezone(&Utc))
            .or_else(|| s.parse::<i64>().ok().and_then(unix_timestamp)),
        Value::Number(n) => n.as_i64().and_then(unix_timestamp),
        _ => None,
    }
}

fn unix_timestamp(value: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_opt(value, 0).single()
}

fn tool_at(value: &Value) -> Option<HookToolCall> {
    if let Some(tool) = value.get("tool").filter(|tool| tool.is_object()) {
        let name = string_at(tool, &["name", "tool_name", "toolName"])?;
        let id = string_at(tool, &["id", "tool_use_id", "toolUseId"]);
        let input = tool
            .get("input")
            .cloned()
            .or_else(|| tool.get("arguments").cloned())
            .unwrap_or(Value::Null);
        return Some(HookToolCall { id, name, input });
    }

    string_at(value, &["tool_name", "toolName", "tool"]).map(|name| HookToolCall {
        id: string_at(value, &["tool_use_id", "toolUseId"]),
        input: value
            .get("tool_input")
            .cloned()
            .or_else(|| value.get("toolInput").cloned())
            .unwrap_or(Value::Null),
        name,
    })
}

fn message_at(value: &Value) -> Option<HookMessage> {
    let text = string_at(value, &["message", "text", "content"])?;
    Some(HookMessage {
        role: string_at(value, &["role"]),
        text,
    })
}

fn permission_at(value: &Value, tool: Option<HookToolCall>) -> Option<PermissionRequest> {
    let prompt = string_at(value, &["permission_prompt", "prompt", "reason"]);
    if tool.is_none() && prompt.is_none() && value.get("permission").is_none() {
        return None;
    }
    Some(PermissionRequest {
        request_id: string_at(
            value,
            &["permission_id", "permissionId", "request_id", "requestId"],
        ),
        tool,
        prompt,
    })
}

fn question_at(value: &Value) -> Option<QuestionRequest> {
    let prompt = string_at(value, &["question", "question_prompt"])?;
    Some(QuestionRequest {
        request_id: string_at(
            value,
            &["question_id", "questionId", "request_id", "requestId"],
        ),
        prompt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::protocol::HookBridgeEnvironment;
    use serde_json::json;

    #[test]
    fn maps_simple_tool_payload() {
        let request = HookIngestRequest::new(
            "generic",
            "tool_started",
            json!({
                "session_id": "s1",
                "cwd": "/tmp/project",
                "tool": {"id": "t1", "name": "Bash", "input": {"command": "cargo check"}}
            }),
        );
        let event = GenericHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::ToolStarted);
        assert_eq!(event.provider_session_id.as_deref(), Some("s1"));
        assert_eq!(event.tool.as_ref().unwrap().id.as_deref(), Some("t1"));
        assert_eq!(event.tool.as_ref().unwrap().name, "Bash");
    }

    #[test]
    fn falls_back_to_bridge_environment() {
        let mut request = HookIngestRequest::new("generic", "heartbeat", json!({}));
        request.environment = HookBridgeEnvironment {
            cwd: Some(PathBuf::from("/tmp/project")),
            pid: Some(7),
            parent_pid: Some(6),
            pid_start_time: None,
            tty: Some("/dev/ttys001".to_string()),
            shell: None,
            process_ancestry: Vec::new(),
            vars: Default::default(),
        };
        let event = GenericHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(
            event.cwd.as_deref(),
            Some(std::path::Path::new("/tmp/project"))
        );
        assert_eq!(event.pid, Some(7));
        assert_eq!(event.parent_pid, Some(6));
        assert_eq!(event.tty.as_deref(), Some("/dev/ttys001"));
    }
}
