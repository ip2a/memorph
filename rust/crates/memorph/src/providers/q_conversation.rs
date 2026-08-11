//! Shared ConversationState parser for Amazon Q CLI / Kiro CLI lineage.
//!
//! Both store `{ "history": [ { "user": {...}, "assistant": {...} }, ... ] }`
//! in a SQLite `value` column. This module converts that JSON into memorph Events.

use crate::session::{
    Block, Event, EventKind, Fidelity, Links, MappingIssue, MappingIssueLevel, MappingReport,
    Metadata, Role,
};
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Parse a ConversationState JSON string into memorph Events.
pub(crate) fn parse_history(
    provider_id: &str,
    value_json: &str,
    session_id: &str,
    report: &mut MappingReport,
) -> Vec<Event> {
    let Ok(json) = serde_json::from_str::<Value>(value_json) else {
        return Vec::new();
    };
    parse_history_value(provider_id, &json, session_id, report)
}

/// Parse from an already-deserialized Value.
pub(crate) fn parse_history_value(
    provider_id: &str,
    json: &Value,
    session_id: &str,
    report: &mut MappingReport,
) -> Vec<Event> {
    let Some(history) = json.get("history").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    for (idx, entry) in history.iter().enumerate() {
        if let Some(user) = entry.get("user") {
            if let Some(ev) = convert_user(provider_id, user, session_id, idx, report) {
                events.push(ev);
            }
        }
        if let Some(assistant) = entry.get("assistant") {
            if let Some(ev) = convert_assistant(provider_id, assistant, session_id, idx, report) {
                events.push(ev);
            }
        }
    }
    events
}

/// Extract the first user prompt text (for session title).
pub(crate) fn first_prompt_text(value_json: &str, max_chars: usize) -> Option<String> {
    let json: Value = serde_json::from_str(value_json).ok()?;
    first_prompt_text_value(&json, max_chars)
}

pub(crate) fn first_prompt_text_value(json: &Value, max_chars: usize) -> Option<String> {
    let history = json.get("history").and_then(Value::as_array)?;
    history.iter().find_map(|e| {
        e.get("user")?
            .get("content")?
            .get("Prompt")?
            .get("prompt")?
            .as_str()
            .map(|s| s.chars().take(max_chars).collect::<String>())
    })
}

/// Extract (first_timestamp, last_timestamp) from user turns.
pub(crate) fn history_time_bounds(
    value_json: &str,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let Ok(json) = serde_json::from_str::<Value>(value_json) else {
        return (None, None);
    };
    history_time_bounds_value(&json)
}

pub(crate) fn history_time_bounds_value(
    json: &Value,
) -> (Option<DateTime<Utc>>, Option<DateTime<Utc>>) {
    let Some(history) = json.get("history").and_then(Value::as_array) else {
        return (None, None);
    };
    let timestamps: Vec<DateTime<Utc>> = history
        .iter()
        .filter_map(|e| {
            e.get("user")?
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(parse_timestamp)
        })
        .collect();
    let first = timestamps.first().copied();
    let last = timestamps.last().copied().or(first);
    (first, last)
}

fn parse_timestamp(s: &str) -> Option<DateTime<Utc>> {
    if s.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

fn convert_user(
    provider_id: &str,
    user: &Value,
    session_id: &str,
    idx: usize,
    report: &mut MappingReport,
) -> Option<Event> {
    let timestamp = user
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
        .unwrap_or_else(Utc::now);

    let content_obj = user.get("content")?;
    let mut blocks = Vec::new();

    if let Some(prompt) = content_obj.get("Prompt") {
        if let Some(text) = prompt.get("prompt").and_then(Value::as_str) {
            if !text.is_empty() {
                blocks.push(Block::Text {
                    text: text.to_string(),
                });
            }
        }
    } else if let Some(tool_results) = content_obj.get("ToolUseResults") {
        push_tool_results(&mut blocks, tool_results);
    } else if let Some(cancelled) = content_obj.get("CancelledToolUses") {
        if let Some(text) = cancelled.get("prompt").and_then(Value::as_str) {
            if !text.is_empty() {
                blocks.push(Block::Text {
                    text: text.to_string(),
                });
            }
        }
        push_tool_results(&mut blocks, cancelled);
    }

    if blocks.is_empty() {
        return None;
    }

    let kind = if blocks.iter().any(|b| matches!(b, Block::ToolResult { .. })) {
        EventKind::Observation
    } else {
        EventKind::Message
    };

    report.push_issue(MappingIssue {
        level: MappingIssueLevel::Info,
        disposition: Fidelity::Preserved,
        code: format!("{provider_id}-q-user"),
        message: "Mapped Q conversation user turn".into(),
        path: Some(format!("history[{idx}].user")),
        raw: None,
    });

    Some(Event {
        id: format!("{provider_id}:{session_id}:user:{idx}"),
        kind,
        role: Role::User,
        timestamp,
        links: Links::default(),
        blocks,
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    })
}

fn push_tool_results(blocks: &mut Vec<Block>, holder: &Value) {
    let Some(results) = holder.get("tool_use_results").and_then(Value::as_array) else {
        return;
    };
    for tr in results {
        let tool_use_id = tr
            .get("tool_use_id")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let text = tr
            .get("content")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("Text").and_then(Value::as_str))
            .unwrap_or("")
            .to_string();
        blocks.push(Block::ToolResult {
            tool_call_id: tool_use_id,
            content: text,
            outcome: crate::session::ExecutionOutcome::Succeeded,
        });
    }
}

fn convert_assistant(
    provider_id: &str,
    assistant: &Value,
    session_id: &str,
    idx: usize,
    report: &mut MappingReport,
) -> Option<Event> {
    let mut blocks = Vec::new();

    if let Some(response) = assistant.get("Response") {
        let text = response
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !text.is_empty() {
            blocks.push(Block::Text {
                text: text.to_string(),
            });
        }
    } else if let Some(tool_use) = assistant.get("ToolUse") {
        let text = tool_use
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !text.is_empty() {
            blocks.push(Block::Text {
                text: text.to_string(),
            });
        }
        if let Some(tools) = tool_use.get("tool_uses").and_then(Value::as_array) {
            for tool in tools {
                let id = tool
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_string();
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool")
                    .to_string();
                let input = tool.get("args").cloned();
                blocks.push(Block::ToolCall {
                    tool_call_id: id,
                    name,
                    input,
                });
            }
        }
    }

    if blocks.is_empty() {
        return None;
    }

    let kind = if blocks.iter().any(|b| matches!(b, Block::ToolCall { .. })) {
        EventKind::Action
    } else {
        EventKind::Message
    };

    report.push_issue(MappingIssue {
        level: MappingIssueLevel::Info,
        disposition: Fidelity::Preserved,
        code: format!("{provider_id}-q-assistant"),
        message: "Mapped Q conversation assistant turn".into(),
        path: Some(format!("history[{idx}].assistant")),
        raw: None,
    });

    Some(Event {
        id: format!("{provider_id}:{session_id}:assistant:{idx}"),
        kind,
        role: Role::Assistant,
        timestamp: Utc::now(),
        links: Links::default(),
        blocks,
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::MappingDirection;

    #[test]
    fn parses_prompt_and_response() {
        let value = serde_json::json!({
            "history": [{
                "user": {
                    "content": {"Prompt": {"prompt": "hello"}},
                    "timestamp": "2025-10-08T10:00:00Z"
                },
                "assistant": {"Response": {"message_id": "m1", "content": "world"}}
            }]
        })
        .to_string();
        let mut report = MappingReport::new("test", MappingDirection::Import);
        let events = parse_history("test", &value, "sess-1", &mut report);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].role, Role::User);
        assert!(matches!(&events[0].blocks[0], Block::Text { text } if text == "hello"));
        assert_eq!(events[1].role, Role::Assistant);
        assert!(matches!(&events[1].blocks[0], Block::Text { text } if text == "world"));
    }

    #[test]
    fn parses_tool_use_and_results() {
        let value = serde_json::json!({
            "history": [{
                "user": {
                    "content": {"ToolUseResults": {"tool_use_results": [
                        {"tool_use_id": "t1", "content": [{"Text": "output"}]}
                    ]}},
                    "timestamp": "2025-10-08T10:01:00Z"
                },
                "assistant": {"ToolUse": {
                    "message_id": "m2",
                    "content": "Running command",
                    "tool_uses": [{"id": "t2", "name": "execute_bash", "args": {"command": "ls"}}]
                }}
            }]
        })
        .to_string();
        let mut report = MappingReport::new("test", MappingDirection::Import);
        let events = parse_history("test", &value, "sess-1", &mut report);
        assert_eq!(events.len(), 2);
        assert!(
            matches!(&events[0].blocks[0], Block::ToolResult { tool_call_id, .. } if tool_call_id == "t1")
        );
        assert_eq!(events[0].kind, EventKind::Observation);
        assert!(
            matches!(&events[1].blocks[1], Block::ToolCall { name, .. } if name == "execute_bash")
        );
        assert_eq!(events[1].kind, EventKind::Action);
    }

    #[test]
    fn first_prompt_and_time_bounds() {
        let value = serde_json::json!({
            "history": [
                {
                    "user": {"content": {"Prompt": {"prompt": "first msg"}}, "timestamp": "2025-01-01T00:00:00Z"},
                    "assistant": {"Response": {"content": "ok"}}
                },
                {
                    "user": {"content": {"Prompt": {"prompt": "second"}}, "timestamp": "2025-01-02T00:00:00Z"},
                    "assistant": {"Response": {"content": "ok2"}}
                }
            ]
        })
        .to_string();
        assert_eq!(first_prompt_text(&value, 80).as_deref(), Some("first msg"));
        let (first, last) = history_time_bounds(&value);
        assert!(first.is_some());
        assert!(last.is_some());
        assert!(last.unwrap() > first.unwrap());
    }
}
