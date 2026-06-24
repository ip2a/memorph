//! Claude Code hook adapter.
//!
//! Claude emits several subtly different JSON shapes depending on hook type
//! and version. This adapter accepts the common direct keys and the nested
//! payload shapes used by CodeIsland-compatible hook plugins, then maps them
//! into memorph's canonical runtime event model.

use anyhow::Result;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::PathBuf;

use crate::hooks::contract::HookAdapter;
use crate::hooks::model::{
    HookEvent, HookEventType, HookMessage, HookToolCall, PermissionRequest, QuestionRequest,
};
use crate::hooks::protocol::{HookIngestRequest, HookIngestResponse};

pub struct ClaudeHookAdapter;

impl HookAdapter for ClaudeHookAdapter {
    fn provider_id(&self) -> &'static str {
        "claude"
    }

    fn blocking_response_json(
        &self,
        event_name: &str,
        response: &HookIngestResponse,
    ) -> Option<Value> {
        crate::hooks::contract::hook_specific_output_response_json(event_name, response)
    }

    fn normalize(&self, request: &HookIngestRequest) -> Result<Vec<HookEvent>> {
        let raw = request.raw.clone();
        let event_name = string_deep(
            &raw,
            &[],
            &[
                "hook_event_name",
                "hookEventName",
                "event_name",
                "eventName",
            ],
        )
        .unwrap_or_else(|| request.event_name.clone());

        let mut event = HookEvent::new("claude", claude_event_type(&event_name), raw.clone());
        event.provider_session_id = string_deep(&raw, &[], &["session_id", "sessionId"]);
        event.run_id = string_deep(
            &raw,
            &[],
            &[
                "run_id",
                "runId",
                "request_id",
                "requestId",
                "transcript_path",
                "transcriptPath",
            ],
        )
        .or_else(|| Some(request.request_id.clone()));
        event.cwd = path_deep(
            &raw,
            &[],
            &[
                "cwd",
                "workspace",
                "workspace_dir",
                "workspaceDir",
                "project_dir",
                "projectDir",
            ],
        )
        .or_else(|| first_path_deep(&raw, &[], &["workspace_roots", "workspaceRoots"]))
        .or_else(|| request.environment.cwd.clone());
        event.pid = u32_deep(&raw, &[], &["pid"]).or(request.environment.pid);
        event.parent_pid = u32_deep(&raw, &[], &["parent_pid", "parentPid", "ppid"])
            .or(request.environment.parent_pid);
        event.tty = string_deep(&raw, &[], &["tty", "terminal_tty", "terminalTty"])
            .or_else(|| request.environment.tty.clone());
        event.timestamp = timestamp_at(&raw).unwrap_or(request.received_at);

        let tool = tool_at(&raw);
        event.tool = tool.clone();
        event.message = message_at(&event_name, &raw);

        match event.event_type {
            HookEventType::PermissionRequested => {
                if is_ask_user_question(tool.as_ref()) {
                    if let Some(question) = question_from_tool_or_raw(&raw, tool.as_ref()) {
                        event.event_type = HookEventType::QuestionRequested;
                        event.question = Some(question);
                    }
                } else {
                    event.permission = Some(permission_at(&raw, tool));
                }
            }
            HookEventType::QuestionRequested => {
                event.question = question_at(&raw);
            }
            _ => {}
        }

        Ok(vec![event])
    }
}

fn claude_event_type(event_name: &str) -> HookEventType {
    match normalize_name(event_name).as_str() {
        "pretooluse" => HookEventType::ToolStarted,
        "posttooluse" | "posttoolusefailure" => HookEventType::ToolFinished,
        "permissionrequest" => HookEventType::PermissionRequested,
        "userpromptsubmit" | "notification" => HookEventType::MessageCreated,
        "sessionstart" => HookEventType::SessionStarted,
        "stop" | "sessionend" => HookEventType::SessionCompleted,
        "subagentstart" | "subagentstop" | "precompact" => HookEventType::Heartbeat,
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
    let direct_name = string_deep(value, &[], &["tool_name", "toolName", "tool", "name"]);
    let nested_name = string_deep(
        value,
        &["tool", "payload", "data"],
        &["name", "tool_name", "toolName"],
    )
    .or_else(|| string_deep(value, &["tool"], &["name", "tool_name", "toolName"]));
    let name = direct_name.or(nested_name)?;

    let id = string_deep(value, &[], &["tool_use_id", "toolUseId"])
        .or_else(|| {
            string_deep(
                value,
                &["tool", "tool_use", "toolUse", "payload", "data"],
                &["id", "tool_use_id", "toolUseId"],
            )
        })
        .or_else(|| {
            string_deep(
                value,
                &["tool", "payload", "data"],
                &["id", "tool_use_id", "toolUseId"],
            )
        })
        .or_else(|| string_deep(value, &["tool"], &["id", "tool_use_id", "toolUseId"]));

    let input = value_at(
        value,
        &[],
        &[
            "tool_input",
            "toolInput",
            "input",
            "arguments",
            "args",
            "params",
        ],
    )
    .or_else(|| {
        value_at(
            value,
            &["tool", "payload", "data"],
            &[
                "input",
                "arguments",
                "args",
                "params",
                "tool_input",
                "toolInput",
            ],
        )
    })
    .or_else(|| {
        value_at(
            value,
            &["tool"],
            &[
                "input",
                "arguments",
                "args",
                "params",
                "tool_input",
                "toolInput",
            ],
        )
    })
    .unwrap_or(Value::Null);

    Some(HookToolCall { id, name, input })
}

fn permission_at(value: &Value, tool: Option<HookToolCall>) -> PermissionRequest {
    PermissionRequest {
        request_id: string_deep(
            value,
            &[],
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
        prompt: string_deep(
            value,
            &[],
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

fn question_at(value: &Value) -> Option<QuestionRequest> {
    let prompt = string_deep(
        value,
        &[],
        &[
            "question",
            "question_prompt",
            "questionPrompt",
            "prompt",
            "message",
        ],
    )?;
    Some(QuestionRequest {
        request_id: string_deep(
            value,
            &[],
            &["question_id", "questionId", "request_id", "requestId"],
        ),
        prompt,
    })
}

fn question_from_tool_or_raw(
    value: &Value,
    tool: Option<&HookToolCall>,
) -> Option<QuestionRequest> {
    if let Some(question) = question_at(value) {
        return Some(question);
    }

    let input = &tool?.input;
    let prompt = string_deep(
        input,
        &[],
        &["question", "prompt", "message", "text", "content"],
    )?;
    Some(QuestionRequest {
        request_id: tool.and_then(|tool| tool.id.clone()),
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
        _ => (
            string_deep(value, &[], &["role"]),
            &["message", "text", "content"],
        ),
    };
    let text = string_deep(value, &[], keys)?;
    Some(HookMessage { role, text })
}

fn string_deep(value: &Value, path: &[&str], keys: &[&str]) -> Option<String> {
    let target = value_at_path(value, path)?;
    keys.iter().find_map(|key| match target.get(*key) {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.to_string()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    })
}

fn u32_deep(value: &Value, path: &[&str], keys: &[&str]) -> Option<u32> {
    let target = value_at_path(value, path)?;
    keys.iter().find_map(|key| match target.get(*key) {
        Some(Value::Number(n)) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
        Some(Value::String(s)) => s.parse().ok(),
        _ => None,
    })
}

fn path_deep(value: &Value, path: &[&str], keys: &[&str]) -> Option<PathBuf> {
    string_deep(value, path, keys).map(PathBuf::from)
}

fn first_path_deep(value: &Value, path: &[&str], keys: &[&str]) -> Option<PathBuf> {
    let target = value_at_path(value, path)?;
    keys.iter().find_map(|key| match target.get(*key) {
        Some(Value::Array(items)) => items.iter().find_map(|item| match item {
            Value::String(path) if !path.trim().is_empty() => Some(PathBuf::from(path.trim())),
            _ => None,
        }),
        Some(Value::String(path)) if !path.trim().is_empty() => Some(PathBuf::from(path.trim())),
        _ => None,
    })
}

fn value_at(value: &Value, path: &[&str], keys: &[&str]) -> Option<Value> {
    let target = value_at_path(value, path)?;
    keys.iter().find_map(|key| target.get(*key).cloned())
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
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
    use crate::hooks::protocol::HookBridgeEnvironment;
    use serde_json::json;

    #[test]
    fn maps_pre_tool_use_with_direct_claude_fields() {
        let request = HookIngestRequest::new(
            "claude",
            "PreToolUse",
            json!({
                "hook_event_name": "PreToolUse",
                "session_id": "claude-session-1",
                "cwd": "/tmp/work",
                "tool_name": "Bash",
                "tool_use_id": "tool-1",
                "tool_input": {"command": "cargo test"}
            }),
        );
        let event = ClaudeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.provider, "claude");
        assert_eq!(event.event_type, HookEventType::ToolStarted);
        assert_eq!(
            event.provider_session_id.as_deref(),
            Some("claude-session-1")
        );
        assert_eq!(event.tool.as_ref().unwrap().id.as_deref(), Some("tool-1"));
        assert_eq!(event.tool.as_ref().unwrap().name, "Bash");
    }

    #[test]
    fn maps_nested_permission_ask_user_tool_to_question() {
        let request = HookIngestRequest::new(
            "claude",
            "PermissionRequest",
            json!({
                "hookEventName": "PermissionRequest",
                "sessionId": "claude-session-2",
                "tool": {
                    "payload": {
                        "data": {
                            "name": "AskUserQuestion",
                            "id": "q-tool",
                            "input": {"question": "Which branch should I use?"}
                        }
                    }
                }
            }),
        );
        let event = ClaudeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::QuestionRequested);
        assert_eq!(
            event.question.as_ref().unwrap().prompt,
            "Which branch should I use?"
        );
    }

    #[test]
    fn falls_back_to_bridge_environment_for_runtime_identity() {
        let mut request = HookIngestRequest::new("claude", "SessionStart", json!({}));
        request.environment = HookBridgeEnvironment {
            cwd: Some(PathBuf::from("/tmp/work")),
            pid: Some(10),
            parent_pid: Some(9),
            pid_start_time: None,
            tty: Some("/dev/ttys001".to_string()),
            shell: None,
            process_ancestry: Vec::new(),
            vars: Default::default(),
        };
        let event = ClaudeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::SessionStarted);
        assert_eq!(
            event.cwd.as_deref(),
            Some(std::path::Path::new("/tmp/work"))
        );
        assert_eq!(event.pid, Some(10));
        assert_eq!(event.parent_pid, Some(9));
        assert_eq!(event.tty.as_deref(), Some("/dev/ttys001"));
    }

    #[test]
    fn maps_user_prompt_submit_to_user_message() {
        let request = HookIngestRequest::new(
            "claude",
            "UserPromptSubmit",
            json!({"prompt": "summarize this workspace"}),
        );
        let event = ClaudeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::MessageCreated);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("user")
        );
        assert_eq!(
            event.message.as_ref().unwrap().text,
            "summarize this workspace"
        );
    }

    #[test]
    fn maps_stop_to_assistant_message() {
        let request = HookIngestRequest::new(
            "claude",
            "Stop",
            json!({"last_assistant_message": "implementation finished"}),
        );
        let event = ClaudeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(event.event_type, HookEventType::SessionCompleted);
        assert_eq!(
            event.message.as_ref().unwrap().role.as_deref(),
            Some("assistant")
        );
        assert_eq!(
            event.message.as_ref().unwrap().text,
            "implementation finished"
        );
    }

    #[test]
    fn uses_first_workspace_root_as_cwd() {
        let request = HookIngestRequest::new(
            "claude",
            "SessionStart",
            json!({"workspace_roots": ["/tmp/root-a", "/tmp/root-b"]}),
        );
        let event = ClaudeHookAdapter.normalize(&request).unwrap().remove(0);
        assert_eq!(
            event.cwd.as_deref(),
            Some(std::path::Path::new("/tmp/root-a"))
        );
    }
}
