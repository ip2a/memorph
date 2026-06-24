//! Gemini hook adapter.
//!
//! Gemini CLI uses a nested hook configuration at `~/.gemini/settings.json`.
//! This adapter maps Gemini lifecycle/tool/agent hook names into memorph's
//! canonical runtime event model and keeps provider-specific payload aliases out
//! of the runtime reducer.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::PathBuf;

use crate::hooks::contract::HookAdapter;
use crate::hooks::model::{HookEvent, HookEventType, HookMessage, HookToolCall};
use crate::hooks::protocol::HookIngestRequest;

pub struct GeminiHookAdapter;

impl HookAdapter for GeminiHookAdapter {
    fn provider_id(&self) -> &'static str {
        "gemini"
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
        let mut event = HookEvent::new("gemini", gemini_event_type(&event_name), raw.clone());

        event.event_id = string_at(&raw, &["event_id", "eventId", "id"]).unwrap_or(event.event_id);
        event.provider_session_id = string_at(
            &raw,
            &[
                "session_id",
                "sessionId",
                "conversation_id",
                "conversationId",
                "chat_id",
                "chatId",
            ],
        );
        event.run_id = string_at(
            &raw,
            &[
                "run_id",
                "runId",
                "request_id",
                "requestId",
                "call_id",
                "callId",
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
        event.tool = tool_at(&raw);
        event.message = message_for_event(&event_name, &raw);

        Ok(vec![event])
    }
}

fn gemini_event_type(event_name: &str) -> HookEventType {
    match normalize_name(event_name).as_str() {
        "sessionstart" => HookEventType::SessionStarted,
        "sessionend" | "stop" => HookEventType::SessionCompleted,
        "beforetool" | "pretooluse" => HookEventType::ToolStarted,
        "aftertool" | "posttooluse" => HookEventType::ToolFinished,
        "beforeagent" | "afteragent" | "userpromptsubmit" => HookEventType::MessageCreated,
        _ => HookEventType::from_provider_name(event_name),
    }
}

fn tool_at(value: &Value) -> Option<HookToolCall> {
    if let Some(tool) = value.get("tool").filter(|tool| tool.is_object()) {
        let name = string_at(
            tool,
            &[
                "name",
                "tool_name",
                "toolName",
                "function_name",
                "functionName",
            ],
        )?;
        return Some(HookToolCall {
            id: string_at(
                tool,
                &[
                    "id",
                    "tool_use_id",
                    "toolUseId",
                    "call_id",
                    "callId",
                    "invocation_id",
                    "invocationId",
                ],
            ),
            name,
            input: value_at(tool, &["input", "arguments", "args", "params"]).unwrap_or(Value::Null),
        });
    }

    let name = string_at(
        value,
        &[
            "tool_name",
            "toolName",
            "tool",
            "name",
            "function_name",
            "functionName",
        ],
    )?;
    Some(HookToolCall {
        id: string_at(
            value,
            &[
                "tool_use_id",
                "toolUseId",
                "call_id",
                "callId",
                "invocation_id",
                "invocationId",
                "id",
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

fn message_for_event(event_name: &str, value: &Value) -> Option<HookMessage> {
    let normalized = normalize_name(event_name);
    let (role, keys): (Option<String>, &[&str]) = match normalized.as_str() {
        "userpromptsubmit" | "beforeagent" => (
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
        "afteragent" | "stop" | "sessionend" => (
            Some("assistant".to_string()),
            &[
                "response",
                "agent_message",
                "agentMessage",
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

fn normalize_name(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
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
    fn maps_gemini_before_tool_to_tool_started() {
        let request = HookIngestRequest::new(
            "gemini",
            "BeforeTool",
            json!({
                "session_id": "gemini-session-1",
                "tool_name": "run_shell_command",
                "args": {"command": "cargo check"},
                "cwd": "/tmp/project"
            }),
        );
        let event = GeminiHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.provider, "gemini");
        assert_eq!(event.event_type, HookEventType::ToolStarted);
        assert_eq!(
            event.provider_session_id.as_deref(),
            Some("gemini-session-1")
        );
        assert_eq!(event.tool.as_ref().unwrap().name, "run_shell_command");
        assert_eq!(event.tool.as_ref().unwrap().input["command"], "cargo check");
    }

    #[test]
    fn maps_gemini_after_agent_to_message() {
        let request = HookIngestRequest::new(
            "gemini",
            "AfterAgent",
            json!({"session_id": "s1", "response": "done"}),
        );
        let event = GeminiHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::MessageCreated);
        assert_eq!(event.message.as_ref().unwrap().text, "done");
    }

    #[test]
    fn maps_gemini_user_prompt_to_user_message() {
        let request = HookIngestRequest::new(
            "gemini",
            "UserPromptSubmit",
            json!({"userPrompt": "inspect adapters"}),
        );
        let event = GeminiHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::MessageCreated);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("user")
        );
        assert_eq!(event.message.as_ref().unwrap().text, "inspect adapters");
    }

    #[test]
    fn maps_gemini_stop_to_assistant_message() {
        let request = HookIngestRequest::new(
            "gemini",
            "Stop",
            json!({"last_assistant_message": "adapter work complete"}),
        );
        let event = GeminiHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::SessionCompleted);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("assistant")
        );
        assert_eq!(
            event.message.as_ref().unwrap().text,
            "adapter work complete"
        );
    }

    #[test]
    fn uses_first_workspace_root_as_cwd() {
        let request = HookIngestRequest::new(
            "gemini",
            "SessionStart",
            json!({"workspaceRoots": ["/tmp/gemini-a", "/tmp/gemini-b"]}),
        );
        let event = GeminiHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(
            event.cwd.as_deref(),
            Some(std::path::Path::new("/tmp/gemini-a"))
        );
    }
}
