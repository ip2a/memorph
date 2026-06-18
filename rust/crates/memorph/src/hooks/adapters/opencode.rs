//! OpenCode hook adapter.
//!
//! OpenCode integration is plugin-based. The memorph plugin forwards mapped
//! lifecycle events with CodeIsland-compatible field names, plus a few native
//! OpenCode ids for permissions/questions. This adapter normalizes those
//! payloads into memorph runtime events.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::PathBuf;

use crate::hooks::model::{
    HookEvent, HookEventType, HookMessage, HookToolCall, PermissionRequest, QuestionRequest,
};
use crate::hooks::normalizer::HookAdapter;
use crate::hooks::protocol::HookIngestRequest;

pub struct OpenCodeHookAdapter;

impl HookAdapter for OpenCodeHookAdapter {
    fn provider_id(&self) -> &'static str {
        "opencode"
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
                "type",
                "event",
            ],
        )
        .unwrap_or_else(|| request.event_name.clone());
        let mut event = HookEvent::new("opencode", opencode_event_type(&event_name), raw.clone());

        event.provider_session_id =
            string_at(&raw, &["session_id", "sessionId", "sessionID", "session"]);
        event.run_id = string_at(
            &raw,
            &[
                "_opencode_request_id",
                "opencode_request_id",
                "request_id",
                "requestId",
                "tool_use_id",
                "toolUseId",
            ],
        )
        .or_else(|| Some(request.request_id.clone()));
        event.cwd = path_at(
            &raw,
            &[
                "cwd",
                "directory",
                "workspace",
                "workspace_dir",
                "workspaceDir",
                "project_dir",
                "projectDir",
            ],
        )
        .or_else(|| first_path_at(&raw, &["workspace_roots", "workspaceRoots"]))
        .or_else(|| request.environment.cwd.clone());
        event.pid = u32_at(&raw, &["pid", "_pid"]).or(request.environment.pid);
        event.parent_pid = u32_at(&raw, &["parent_pid", "parentPid", "ppid", "_ppid"])
            .or(request.environment.parent_pid);
        event.tty = string_at(&raw, &["tty", "_tty", "terminal_tty", "terminalTty"])
            .or_else(|| request.environment.tty.clone());
        event.timestamp = timestamp_at(&raw).unwrap_or(request.received_at);

        let tool = tool_at(&raw);
        event.tool = tool.clone();
        event.message = message_at(&event_name, &raw);
        if is_ask_user_question(tool.as_ref()) {
            event.event_type = HookEventType::QuestionRequested;
            event.question = question_from_tool_or_raw(&raw, tool.as_ref());
        } else if event.event_type == HookEventType::PermissionRequested {
            event.permission = Some(permission_at(&raw, tool));
        }

        Ok(vec![event])
    }
}

fn opencode_event_type(event_name: &str) -> HookEventType {
    match normalize_name(event_name).as_str() {
        "sessionstart" => HookEventType::SessionStarted,
        "sessionend" | "stop" => HookEventType::SessionCompleted,
        "userpromptsubmit" | "notification" => HookEventType::MessageCreated,
        "pretooluse" => HookEventType::ToolStarted,
        "posttooluse" | "posttoolusefailure" => HookEventType::ToolFinished,
        "permissionrequest" => HookEventType::PermissionRequested,
        _ => HookEventType::from_provider_name(event_name),
    }
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
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
        id: string_at(
            value,
            &[
                "tool_use_id",
                "toolUseId",
                "_opencode_request_id",
                "opencode_request_id",
            ],
        ),
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

fn permission_at(value: &Value, tool: Option<HookToolCall>) -> PermissionRequest {
    PermissionRequest {
        request_id: string_at(
            value,
            &[
                "_opencode_request_id",
                "opencode_request_id",
                "permission_id",
                "permissionId",
                "request_id",
                "requestId",
            ],
        )
        .or_else(|| tool.as_ref().and_then(|tool| tool.id.clone())),
        tool,
        prompt: string_at(
            value,
            &[
                "permission_prompt",
                "permissionPrompt",
                "prompt",
                "reason",
                "message",
            ],
        ),
    }
}

fn question_from_tool_or_raw(
    value: &Value,
    tool: Option<&HookToolCall>,
) -> Option<QuestionRequest> {
    let prompt = string_at(
        value,
        &["question", "question_prompt", "questionPrompt", "prompt"],
    )
    .or_else(|| {
        let questions = tool?.input.get("questions")?.as_array()?;
        let first = questions.first()?;
        string_at(first, &["question", "prompt", "header"])
    })?;
    Some(QuestionRequest {
        request_id: string_at(
            value,
            &[
                "_opencode_request_id",
                "opencode_request_id",
                "question_id",
                "questionId",
                "request_id",
                "requestId",
            ],
        )
        .or_else(|| tool.and_then(|tool| tool.id.clone())),
        prompt,
    })
}

fn is_ask_user_question(tool: Option<&HookToolCall>) -> bool {
    tool.map(|tool| normalize_name(&tool.name) == "askuserquestion")
        .unwrap_or(false)
}

fn message_at(event_name: &str, value: &Value) -> Option<HookMessage> {
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
        "stop" | "sessionend" => (
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
        "notification" => (None, &["message", "text", "summary", "status", "detail"]),
        _ => (string_at(value, &["role"]), &["message", "text", "content"]),
    };
    let text = string_at(value, keys)?;
    Some(HookMessage { role, text })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_opencode_tool_event() {
        let request = HookIngestRequest::new(
            "opencode",
            "PreToolUse",
            json!({
                "session_id": "opencode-session-1",
                "cwd": "/tmp/work",
                "tool_name": "Bash",
                "tool_input": {"command": "cargo test"},
                "_ppid": 42,
                "_tty": "/dev/ttys003"
            }),
        );
        let event = OpenCodeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.provider, "opencode");
        assert_eq!(event.event_type, HookEventType::ToolStarted);
        assert_eq!(
            event.provider_session_id.as_deref(),
            Some("opencode-session-1")
        );
        assert_eq!(event.parent_pid, Some(42));
        assert_eq!(event.tty.as_deref(), Some("/dev/ttys003"));
    }

    #[test]
    fn maps_opencode_question_permission_to_question() {
        let request = HookIngestRequest::new(
            "opencode",
            "PermissionRequest",
            json!({
                "session_id": "opencode-session-2",
                "tool_name": "AskUserQuestion",
                "tool_input": {
                    "questions": [{"question": "Which package manager?"}]
                },
                "_opencode_request_id": "question-1"
            }),
        );
        let event = OpenCodeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::QuestionRequested);
        assert_eq!(
            event.question.as_ref().unwrap().prompt,
            "Which package manager?"
        );
        assert_eq!(
            event.question.as_ref().unwrap().request_id.as_deref(),
            Some("question-1")
        );
    }

    #[test]
    fn maps_user_prompt_submit_to_user_message() {
        let request = HookIngestRequest::new(
            "opencode",
            "UserPromptSubmit",
            json!({"lastUserMessage": "inspect runtime state"}),
        );
        let event = OpenCodeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::MessageCreated);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("user")
        );
        assert_eq!(
            event.message.as_ref().unwrap().text,
            "inspect runtime state"
        );
    }

    #[test]
    fn maps_stop_to_assistant_message() {
        let request =
            HookIngestRequest::new("opencode", "Stop", json!({"assistantMessage": "ready"}));
        let event = OpenCodeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::SessionCompleted);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("assistant")
        );
        assert_eq!(event.message.as_ref().unwrap().text, "ready");
    }

    #[test]
    fn uses_first_workspace_root_as_cwd() {
        let request = HookIngestRequest::new(
            "opencode",
            "SessionStart",
            json!({"workspace_roots": ["/tmp/opencode-a", "/tmp/opencode-b"]}),
        );
        let event = OpenCodeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(
            event.cwd.as_deref(),
            Some(std::path::Path::new("/tmp/opencode-a"))
        );
    }
}
