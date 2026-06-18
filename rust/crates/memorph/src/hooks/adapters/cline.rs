//! Cline hook adapter.
//!
//! Cline invokes executable files in `~/Documents/Cline/Hooks/<EventName>` and
//! sends JSON on stdin. This adapter maps those file-based hook events into
//! memorph's canonical runtime model.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::PathBuf;

use crate::hooks::model::{HookEvent, HookEventType, HookMessage, HookToolCall, PermissionRequest};
use crate::hooks::normalizer::HookAdapter;
use crate::hooks::protocol::HookIngestRequest;

pub struct ClineHookAdapter;

impl HookAdapter for ClineHookAdapter {
    fn provider_id(&self) -> &'static str {
        "cline"
    }

    fn normalize(&self, request: &HookIngestRequest) -> Result<Vec<HookEvent>> {
        let raw = request.raw.clone();
        let event_name = string_at(
            &raw,
            &[
                "hook_event_name",
                "hookEventName",
                "event_name",
                "eventName",
                "event",
                "type",
            ],
        )
        .unwrap_or_else(|| request.event_name.clone());
        let mut event = HookEvent::new("cline", cline_event_type(&event_name), raw.clone());

        event.event_id = string_at(&raw, &["event_id", "eventId", "id"]).unwrap_or(event.event_id);
        event.provider_session_id = string_at(
            &raw,
            &[
                "session_id",
                "sessionId",
                "task_id",
                "taskId",
                "conversation_id",
                "conversationId",
            ],
        );
        event.run_id = string_at(
            &raw,
            &[
                "run_id",
                "runId",
                "request_id",
                "requestId",
                "task_id",
                "taskId",
            ],
        )
        .or_else(|| Some(request.request_id.clone()));
        event.cwd = path_at(
            &raw,
            &[
                "cwd",
                "workspace",
                "workspace_dir",
                "workspaceDir",
                "project_dir",
                "projectDir",
            ],
        )
        .or_else(|| first_path_at(&raw, &["workspace_roots", "workspaceRoots"]))
        .or_else(|| request.environment.cwd.clone());
        event.pid = u32_at(&raw, &["pid"]).or(request.environment.pid);
        event.parent_pid =
            u32_at(&raw, &["parent_pid", "parentPid", "ppid"]).or(request.environment.parent_pid);
        event.tty = string_at(&raw, &["tty", "terminal_tty", "terminalTty"])
            .or_else(|| request.environment.tty.clone());
        event.timestamp = timestamp_at(&raw).unwrap_or(request.received_at);

        let tool = tool_at(&raw);
        event.tool = tool.clone();
        event.message = message_at(&raw, &event_name);
        if event.event_type == HookEventType::PermissionRequested {
            event.permission = Some(permission_at(&raw, tool));
        }

        Ok(vec![event])
    }
}

fn cline_event_type(event_name: &str) -> HookEventType {
    match normalize_name(event_name).as_str() {
        "taskstart" => HookEventType::SessionStarted,
        "taskresume" => HookEventType::Heartbeat,
        "userpromptsubmit" => HookEventType::MessageCreated,
        "pretooluse" => HookEventType::ToolStarted,
        "posttooluse" => HookEventType::ToolFinished,
        "permissionrequest" => HookEventType::PermissionRequested,
        "taskend" | "taskcomplete" | "stop" => HookEventType::SessionCompleted,
        _ => HookEventType::from_provider_name(event_name),
    }
}

fn tool_at(value: &Value) -> Option<HookToolCall> {
    if let Some(tool) = value.get("tool").filter(|tool| tool.is_object()) {
        let name = string_at(tool, &["name", "tool_name", "toolName"])?;
        return Some(HookToolCall {
            id: string_at(tool, &["id", "tool_use_id", "toolUseId"]),
            name,
            input: value_at(tool, &["input", "arguments", "args", "params"]).unwrap_or(Value::Null),
        });
    }

    let name = string_at(value, &["tool_name", "toolName", "tool", "name"])?;
    Some(HookToolCall {
        id: string_at(value, &["tool_use_id", "toolUseId", "id"]),
        name,
        input: value_at(
            value,
            &[
                "tool_input",
                "toolInput",
                "input",
                "arguments",
                "args",
                "params",
            ],
        )
        .unwrap_or(Value::Null),
    })
}

fn message_at(value: &Value, event_name: &str) -> Option<HookMessage> {
    let normalized = normalize_name(event_name);
    let (role, keys): (Option<String>, &[&str]) = match normalized.as_str() {
        "userpromptsubmit" => (
            Some("user".to_string()),
            &[
                "prompt",
                "user_prompt",
                "userPrompt",
                "last_user_message",
                "lastUserMessage",
                "message",
                "text",
                "content",
            ],
        ),
        "taskend" | "taskcomplete" | "stop" => (
            Some("assistant".to_string()),
            &[
                "last_assistant_message",
                "lastAssistantMessage",
                "assistant_message",
                "assistantMessage",
                "summary",
                "message",
                "text",
                "content",
            ],
        ),
        _ => (string_at(value, &["role"]), &["message", "text", "content"]),
    };
    let text = string_at(value, keys)?;
    Some(HookMessage { role, text })
}

fn permission_at(value: &Value, tool: Option<HookToolCall>) -> PermissionRequest {
    PermissionRequest {
        request_id: string_at(
            value,
            &[
                "permission_id",
                "permissionId",
                "request_id",
                "requestId",
                "tool_use_id",
                "toolUseId",
            ],
        )
        .or_else(|| tool.as_ref().and_then(|tool| tool.id.clone())),
        tool,
        prompt: string_at(value, &["permission_prompt", "prompt", "reason", "message"]),
    }
}

fn string_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match value.get(*key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    })
}

fn u32_at(value: &Value, keys: &[&str]) -> Option<u32> {
    keys.iter().find_map(|key| match value.get(*key) {
        Some(Value::Number(n)) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    })
}

fn path_at(value: &Value, keys: &[&str]) -> Option<PathBuf> {
    string_at(value, keys).map(PathBuf::from)
}

fn first_path_at(value: &Value, keys: &[&str]) -> Option<PathBuf> {
    keys.iter().find_map(|key| match value.get(*key) {
        Some(Value::Array(items)) => items.iter().find_map(|item| match item {
            Value::String(path) if !path.trim().is_empty() => Some(PathBuf::from(path.trim())),
            _ => None,
        }),
        Some(Value::String(path)) if !path.trim().is_empty() => Some(PathBuf::from(path.trim())),
        _ => None,
    })
}

fn value_at(value: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter().find_map(|key| value.get(*key).cloned())
}

fn timestamp_at(value: &Value) -> Option<DateTime<Utc>> {
    let raw = value
        .get("timestamp")
        .or_else(|| value.get("created_at"))
        .or_else(|| value.get("createdAt"))?;
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

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::protocol::HookIngestRequest;
    use serde_json::json;

    #[test]
    fn maps_cline_pre_tool_use() {
        let request = HookIngestRequest::new(
            "cline",
            "PreToolUse",
            json!({
                "task_id": "task-1",
                "tool_name": "execute_command",
                "tool_input": {"command": "cargo check"}
            }),
        );
        let event = ClineHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.provider, "cline");
        assert_eq!(event.event_type, HookEventType::ToolStarted);
        assert_eq!(event.provider_session_id.as_deref(), Some("task-1"));
        assert_eq!(event.tool.as_ref().unwrap().name, "execute_command");
    }

    #[test]
    fn maps_cline_user_prompt_to_user_message() {
        let request = HookIngestRequest::new(
            "cline",
            "UserPromptSubmit",
            json!({"task_id": "task-1", "userPrompt": "continue"}),
        );
        let event = ClineHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::MessageCreated);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("user")
        );
        assert_eq!(event.message.as_ref().unwrap().text, "continue");
    }

    #[test]
    fn maps_cline_task_end_to_assistant_message() {
        let request = HookIngestRequest::new(
            "cline",
            "TaskEnd",
            json!({"task_id": "task-1", "lastAssistantMessage": "complete"}),
        );
        let event = ClineHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::SessionCompleted);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("assistant")
        );
        assert_eq!(event.message.as_ref().unwrap().text, "complete");
    }

    #[test]
    fn uses_first_workspace_root_as_cwd() {
        let request = HookIngestRequest::new(
            "cline",
            "TaskStart",
            json!({"workspaceRoots": ["/tmp/cline-a", "/tmp/cline-b"]}),
        );
        let event = ClineHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(
            event.cwd.as_deref(),
            Some(std::path::Path::new("/tmp/cline-a"))
        );
    }
}
