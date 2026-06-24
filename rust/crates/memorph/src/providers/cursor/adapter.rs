//! Cursor hook adapter.
//!
//! Cursor exposes a flat hook shape in `~/.cursor/hooks.json`. The hook names
//! are lower-camel events such as `beforeShellExecution` and
//! `afterAgentResponse`; this adapter maps those events into memorph's canonical
//! runtime event model while preserving the original payload.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::PathBuf;

use crate::hooks::contract::HookAdapter;
use crate::hooks::model::{HookEvent, HookEventType, HookMessage, HookToolCall, PermissionRequest};
use crate::hooks::protocol::HookIngestRequest;

pub struct CursorHookAdapter;

impl HookAdapter for CursorHookAdapter {
    fn provider_id(&self) -> &'static str {
        "cursor"
    }

    fn normalize(&self, request: &HookIngestRequest) -> Result<Vec<HookEvent>> {
        let raw = request.raw.clone();
        let cursor_event = string_at(&raw, &["hook_event_name", "event_name", "event", "type"])
            .unwrap_or_else(|| request.event_name.clone());
        let mut event = HookEvent::new("cursor", cursor_event_type(&cursor_event), raw.clone());

        event.event_id = string_at(&raw, &["event_id", "eventId", "id"]).unwrap_or(event.event_id);
        event.provider_session_id = string_at(
            &raw,
            &[
                "session_id",
                "sessionId",
                "composer_id",
                "composerId",
                "conversation_id",
                "conversationId",
            ],
        );
        event.run_id = string_at(&raw, &["run_id", "runId", "request_id", "requestId"])
            .or_else(|| Some(request.request_id.clone()));
        event.cwd = path_at(&raw, &["cwd", "workspace", "workspace_dir", "project_dir"])
            .or_else(|| first_path_at(&raw, &["workspace_roots", "workspaceRoots"]))
            .or_else(|| request.environment.cwd.clone());
        event.pid = u32_at(&raw, &["pid"]).or(request.environment.pid);
        event.parent_pid =
            u32_at(&raw, &["parent_pid", "parentPid", "ppid"]).or(request.environment.parent_pid);
        event.tty =
            string_at(&raw, &["tty", "terminal_tty"]).or_else(|| request.environment.tty.clone());
        event.timestamp = timestamp_at(&raw).unwrap_or(request.received_at);
        event.tool = tool_for_cursor_event(&cursor_event, &raw);
        event.message = message_for_cursor_event(&cursor_event, &raw);
        event.permission = permission_for_cursor_event(&event.event_type, event.tool.clone(), &raw);

        Ok(vec![event])
    }
}

fn cursor_event_type(event: &str) -> HookEventType {
    match normalize_event_name(event).as_str() {
        "beforesubmitprompt" => HookEventType::MessageCreated,
        "beforeshellexecution" | "beforereadfile" | "beforemcpexecution" => {
            HookEventType::ToolStarted
        }
        "aftershellexecution" | "afterfileedit" | "aftermcpexecution" => {
            HookEventType::ToolFinished
        }
        "afteragentthought" | "afteragentresponse" => HookEventType::MessageCreated,
        "stop" | "sessionend" => HookEventType::SessionCompleted,
        other => HookEventType::from_provider_name(other),
    }
}

fn tool_for_cursor_event(event: &str, value: &Value) -> Option<HookToolCall> {
    if let Some(tool) = value.get("tool").filter(|tool| tool.is_object()) {
        let name = string_at(tool, &["name", "tool_name", "toolName"])?;
        return Some(HookToolCall {
            id: string_at(tool, &["id", "tool_use_id", "toolUseId"]),
            name,
            input: tool
                .get("input")
                .cloned()
                .or_else(|| tool.get("arguments").cloned())
                .unwrap_or(Value::Null),
        });
    }

    let normalized = normalize_event_name(event);
    let inferred = match normalized.as_str() {
        "beforeshellexecution" | "aftershellexecution" => "Shell",
        "beforereadfile" => "Read",
        "afterfileedit" => "Edit",
        "beforemcpexecution" | "aftermcpexecution" => "MCP",
        _ => {
            let name = string_at(value, &["tool_name", "toolName", "tool"])?;
            return Some(HookToolCall {
                id: string_at(
                    value,
                    &["tool_use_id", "toolUseId", "tool_call_id", "toolCallId"],
                ),
                name,
                input: tool_input(value),
            });
        }
    };

    Some(HookToolCall {
        id: string_at(
            value,
            &["tool_use_id", "toolUseId", "tool_call_id", "toolCallId"],
        ),
        name: string_at(value, &["tool_name", "toolName", "tool"])
            .unwrap_or_else(|| inferred.to_string()),
        input: tool_input(value),
    })
}

fn message_for_cursor_event(event: &str, value: &Value) -> Option<HookMessage> {
    let text = string_at(
        value,
        &[
            "prompt", "message", "text", "content", "response", "thought",
        ],
    )?;
    let role = match normalize_event_name(event).as_str() {
        "beforesubmitprompt" => Some("user".to_string()),
        "afteragentthought" | "afteragentresponse" => Some("assistant".to_string()),
        _ => string_at(value, &["role"]),
    };
    Some(HookMessage { role, text })
}

fn permission_for_cursor_event(
    event_type: &HookEventType,
    tool: Option<HookToolCall>,
    value: &Value,
) -> Option<PermissionRequest> {
    if *event_type != HookEventType::PermissionRequested {
        return None;
    }
    Some(PermissionRequest {
        request_id: string_at(
            value,
            &["permission_id", "permissionId", "request_id", "requestId"],
        ),
        tool,
        prompt: string_at(value, &["permission_prompt", "prompt", "reason", "message"]),
    })
}

fn tool_input(value: &Value) -> Value {
    value
        .get("tool_input")
        .cloned()
        .or_else(|| value.get("toolInput").cloned())
        .or_else(|| value.get("input").cloned())
        .or_else(|| {
            let command = string_at(value, &["command", "shell_command", "shellCommand"])?;
            Some(serde_json::json!({ "command": command }))
        })
        .or_else(|| {
            let path = string_at(value, &["file_path", "filePath", "path"])?;
            Some(serde_json::json!({ "file_path": path }))
        })
        .unwrap_or(Value::Null)
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

fn normalize_event_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_cursor_shell_execution_to_tool_started() {
        let request = HookIngestRequest::new(
            "cursor",
            "beforeShellExecution",
            json!({
                "session_id": "composer-1",
                "command": "cargo check",
                "cwd": "/tmp/project"
            }),
        );
        let event = CursorHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.provider, "cursor");
        assert_eq!(event.event_type, HookEventType::ToolStarted);
        assert_eq!(event.provider_session_id.as_deref(), Some("composer-1"));
        assert_eq!(event.tool.as_ref().unwrap().name, "Shell");
        assert_eq!(event.tool.as_ref().unwrap().input["command"], "cargo check");
    }

    #[test]
    fn maps_cursor_agent_response_to_assistant_message() {
        let request = HookIngestRequest::new(
            "cursor",
            "afterAgentResponse",
            json!({"composer_id": "composer-1", "response": "done"}),
        );
        let event = CursorHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::MessageCreated);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("assistant")
        );
        assert_eq!(event.message.as_ref().unwrap().text, "done");
    }

    #[test]
    fn uses_first_workspace_root_as_cwd() {
        let request = HookIngestRequest::new(
            "cursor",
            "beforeSubmitPrompt",
            json!({"workspace_roots": ["/tmp/cursor-a", "/tmp/cursor-b"]}),
        );
        let event = CursorHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(
            event.cwd.as_deref(),
            Some(std::path::Path::new("/tmp/cursor-a"))
        );
    }
}
