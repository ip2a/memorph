use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ImportedSession, MappingDirection, MappingDisposition, MappingIssue,
    MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext, SessionEvent,
    SessionEventKind, SessionIdentity, SessionProvenance, UsageStats,
};
use crate::provider::ProviderSessionSummary;
use crate::utils::{extract_text, parse_timestamp_to_ms, truncate_summary};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

const TITLE_MAX_CHARS: usize = 80;
const MAX_SCAN_DEPTH: usize = 8;

#[derive(Debug, Clone, Copy)]
pub struct JsonProviderSpec {
    pub provider_id: &'static str,
    pub extension_key: &'static str,
    pub roots: fn() -> Vec<PathBuf>,
}

pub fn scan_sessions(spec: JsonProviderSpec) -> Result<Vec<ProviderSessionSummary>> {
    let mut sessions = Vec::new();
    let mut seen = HashSet::new();

    for root in (spec.roots)() {
        if !root.exists() {
            continue;
        }

        for entry in WalkDir::new(&root)
            .max_depth(MAX_SCAN_DEPTH)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if !is_json_session_candidate(path) {
                continue;
            }
            let Ok(value) = read_session_value(path) else {
                continue;
            };
            if !looks_like_session(&value) {
                continue;
            }
            let summary = session_summary_from_value(path, &value);
            if seen.insert(summary.session_id.clone()) {
                sessions.push(summary);
            }
        }
    }

    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_active_at.unwrap_or(0)));
    Ok(sessions)
}

pub fn import_session(spec: JsonProviderSpec, source_path: &str) -> Result<ImportedSession> {
    let path = Path::new(source_path);
    let value = read_session_value(path).with_context(|| {
        format!(
            "Failed to read {} session: {}",
            spec.provider_id, source_path
        )
    })?;
    import_session_from_value(spec, path, value)
}

pub fn import_session_from_value(
    spec: JsonProviderSpec,
    path: &Path,
    value: Value,
) -> Result<ImportedSession> {
    let mut report = MappingReport::new(spec.provider_id, MappingDirection::Import);
    let session_id = session_id_from_value(path, &value);
    let title = title_from_value(&value);
    let workspace_dir = workspace_from_value(&value);
    let created_at = datetime_from_value_keys(&value, &["startTime", "createdAt", "created_at"])
        .or_else(|| path_mtime_datetime(path))
        .unwrap_or_else(Utc::now);
    let last_active_at = datetime_from_value_keys(
        &value,
        &["lastUpdated", "updatedAt", "updated_at", "timestamp"],
    )
    .or_else(|| path_mtime_datetime(path))
    .unwrap_or(created_at);

    let events = extract_message_items(&value)
        .into_iter()
        .enumerate()
        .filter_map(|(index, item)| event_from_message(spec.provider_id, index, item, &mut report))
        .collect();

    let mut extensions = BTreeMap::new();
    extensions.insert(spec.extension_key.to_string(), value);

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: session_id.clone(),
                source_title: title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: spec.provider_id.to_string(),
                    session_id,
                    source_path: Some(path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir,
                created_at: Some(created_at),
                last_active_at: Some(last_active_at),
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn read_json(path: &Path) -> Result<Value> {
    let raw = std::fs::read_to_string(path)?;
    serde_json::from_str(&raw).with_context(|| format!("Failed to parse JSON: {}", path.display()))
}

/// Read a session file that may be either a single JSON object or JSONL.
///
/// For `.jsonl`, each non-empty line is parsed independently. Lines that look like
/// session metadata (carry `sessionId`/`session_id`/`cwd`/timestamp fields) are
/// merged into the returned object's top level; all other lines are collected
/// under a synthesized `messages` array so the downstream `extract_message_items`
/// pipeline can process them uniformly.
fn read_session_value(path: &Path) -> Result<Value> {
    let is_jsonl = path
        .extension()
        .and_then(|value| value.to_str())
        .map(|ext| ext == "jsonl")
        .unwrap_or(false);
    if !is_jsonl {
        return read_json(path);
    }
    read_jsonl_as_session_value(path)
}

fn read_jsonl_as_session_value(path: &Path) -> Result<Value> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read JSONL: {}", path.display()))?;
    let mut map = serde_json::Map::new();
    let mut messages = Vec::<Value>::new();
    let metadata_keys = [
        "sessionId",
        "session_id",
        "id",
        "conversationId",
        "conversation_id",
        "cwd",
        "workspace",
        "project",
        "projectDir",
        "model",
        "startTime",
        "createdAt",
        "created_at",
        "lastUpdated",
        "updatedAt",
        "updated_at",
    ];
    for (line_index, raw_line) in raw.lines().enumerate() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(line) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };

        let is_meta_line = line.get("type").and_then(value_to_string).is_some_and(|ty| {
            matches!(
                ty.as_str(),
                "session_meta" | "sessionMeta" | "metadata" | "meta"
            )
        }) || line.get("isMeta").and_then(Value::as_bool).is_some_and(|v| v);

        if is_meta_line {
            if let Some(obj) = line.as_object() {
                for (key, value) in obj {
                    if key == "type" || key == "isMeta" {
                        continue;
                    }
                    if metadata_keys.contains(&key.as_str()) {
                        map.entry(key.clone()).or_insert(value.clone());
                    }
                }
            }
            // Prefer the nested payload block if present (codex-style {type:"session_meta", payload:{...}}).
            if let Some(payload) = line.get("payload") {
                if let Some(obj) = payload.as_object() {
                    for (key, value) in obj {
                        if metadata_keys.contains(&key.as_str()) {
                            map.entry(key.clone()).or_insert(value.clone());
                        }
                    }
                }
            }
            continue;
        }

        // Promote metadata-looking keys from any line into the top-level map, but only
        // when they haven't been seen yet. This lets a Claude-style jsonl where every
        // row carries `sessionId`/`cwd` still surface those fields.
        if let Some(obj) = line.as_object() {
            for key in metadata_keys {
                if let Some(value) = obj.get(key) {
                    map.entry(key.to_string()).or_insert(value.clone());
                }
            }
        }

        // Anything that isn't a pure metadata marker becomes a message candidate.
        // Skip lines that are obviously control records without message content.
        let looks_like_control = line.get("type").and_then(value_to_string).is_some_and(|ty| {
            matches!(
                ty.as_str(),
                "turn_context" | "compacted" | "summary" | "custom-title" | "ai-title" | "tag"
            )
        });
        if looks_like_control {
            continue;
        }

        let mut entry = line.clone();
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("__line__".to_string(), Value::Number(line_index.into()));
        }
        messages.push(entry);
    }

    map.insert("messages".to_string(), Value::Array(messages));
    Ok(Value::Object(map))
}

fn is_json_session_candidate(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|value| value.to_str())
            .map(|ext| matches!(ext, "json" | "jsonl"))
            .unwrap_or(false)
        && path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|name| {
                let name = name.to_ascii_lowercase();
                name.starts_with("session")
                    || name.contains("chat")
                    || name.contains("conversation")
                    || name.contains("history")
                    || name.contains("thread")
                    || name.contains("rollout")
                    || name.contains("transcript")
                    || name.contains("events")
            })
            .unwrap_or(false)
}

fn looks_like_session(value: &Value) -> bool {
    !extract_message_items(value).is_empty()
        || value.get("sessionId").is_some()
        || value.get("session_id").is_some()
        || value.get("conversationId").is_some()
}

fn session_summary_from_value(path: &Path, value: &Value) -> ProviderSessionSummary {
    let session_id = session_id_from_value(path, value);
    let title = title_from_value(value).or_else(|| first_message_title(value));
    let project_dir = workspace_from_value(value);
    let last_active_at = timestamp_from_value_keys(
        value,
        &["lastUpdated", "updatedAt", "updated_at", "timestamp"],
    )
    .or_else(|| path_mtime_ms(path));

    ProviderSessionSummary {
        session_id,
        title,
        project_dir,
        last_active_at,
        source_path: Some(path.to_string_lossy().to_string()),
    }
}

fn session_id_from_value(path: &Path, value: &Value) -> String {
    for key in [
        "sessionId",
        "session_id",
        "id",
        "conversationId",
        "conversation_id",
        "threadId",
        "thread_id",
        "chatId",
        "chat_id",
    ] {
        if let Some(raw) = value.get(key).and_then(value_to_string) {
            if !raw.trim().is_empty() {
                return raw;
            }
        }
    }
    path.file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("unknown-session")
        .to_string()
}

fn title_from_value(value: &Value) -> Option<String> {
    for key in ["title", "name", "summary", "preview"] {
        let Some(raw) = value.get(key).and_then(value_to_string) else {
            continue;
        };
        let title = truncate_summary(&raw, TITLE_MAX_CHARS);
        if !title.is_empty() {
            return Some(title);
        }
    }
    None
}

fn first_message_title(value: &Value) -> Option<String> {
    extract_message_items(value).into_iter().find_map(|item| {
        let text = message_text(item);
        let title = truncate_summary(&text, TITLE_MAX_CHARS);
        (!title.is_empty()).then_some(title)
    })
}

fn workspace_from_value(value: &Value) -> Option<String> {
    for key in [
        "workspaceDirectory",
        "workspaceDir",
        "workspace_dir",
        "projectDir",
        "project_dir",
        "cwd",
        "rootPath",
    ] {
        let Some(raw) = value.get(key).and_then(value_to_string) else {
            continue;
        };
        let raw = raw.trim();
        if !raw.is_empty() {
            return Some(raw.to_string());
        }
    }
    value
        .get("context")
        .and_then(workspace_from_value)
        .or_else(|| value.get("workspace").and_then(workspace_from_value))
}

fn extract_message_items(value: &Value) -> Vec<&Value> {
    if let Some(items) = value.get("messages").and_then(|value| value.as_array()) {
        return items.iter().collect();
    }
    if let Some(items) = value.get("history").and_then(|value| value.as_array()) {
        return items.iter().collect();
    }
    if let Some(items) = value.get("entries").and_then(|value| value.as_array()) {
        return items.iter().collect();
    }
    if let Some(items) = value.get("turns").and_then(|value| value.as_array()) {
        return items.iter().collect();
    }
    if let Some(conversation) = value.get("conversation") {
        let nested = extract_message_items(conversation);
        if !nested.is_empty() {
            return nested;
        }
    }
    Vec::new()
}

fn event_from_message(
    provider_id: &str,
    index: usize,
    item: &Value,
    report: &mut MappingReport,
) -> Option<SessionEvent> {
    let message = item.get("message").unwrap_or(item);
    let blocks = message_blocks(message, item, index, report);
    if blocks.is_empty() {
        return None;
    }
    let role_raw = role_string(message);
    let role = role_from_string(&role_raw);
    let timestamp = datetime_from_value_keys(message, &["timestamp", "createdAt", "created_at"])
        .unwrap_or_else(Utc::now);
    let original_id = message_id(message);

    Some(SessionEvent {
        id: original_id
            .clone()
            .unwrap_or_else(|| format!("{}:message:{}", provider_id, index)),
        kind: event_kind(&blocks),
        role,
        timestamp,
        links: EventLinks {
            parent_event_id: None,
            provider_parent_id: None,
            provider_turn_id: None,
            turn_index: Some(index as u32),
            turn_boundary: None,
            related_event_ids: Vec::new(),
        },
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: provider_id.to_string(),
                original_id,
                original_role: Some(role_raw),
                phase: None,
            },
            model: message.get("model").and_then(value_to_string),
            usage: usage_from_message(message),
            fidelity: MappingDisposition::Normalized,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("raw_message".to_string(), item.clone());
                ext
            },
        },
    })
}

fn message_blocks(
    message: &Value,
    raw_item: &Value,
    index: usize,
    report: &mut MappingReport,
) -> Vec<EventBlock> {
    let mut blocks = Vec::new();
    let text = message_text(message);
    if !text.trim().is_empty() {
        blocks.push(EventBlock::Text { text });
    }

    if let Some(thoughts) = message.get("thoughts").and_then(|value| value.as_array()) {
        for thought in thoughts {
            let text = thought
                .get("subject")
                .and_then(|value| value.as_str())
                .or_else(|| thought.get("description").and_then(|value| value.as_str()))
                .or_else(|| thought.get("text").and_then(|value| value.as_str()))
                .unwrap_or("");
            if !text.trim().is_empty() {
                blocks.push(EventBlock::Thinking {
                    text: text.to_string(),
                    signature: None,
                });
            }
        }
    }

    if let Some(tool_calls) = message.get("toolCalls").and_then(|value| value.as_array()) {
        for tool_call in tool_calls {
            let id = tool_call
                .get("id")
                .and_then(value_to_string)
                .unwrap_or_else(|| format!("tool-call-{}", blocks.len()));
            let name = tool_call
                .get("name")
                .or_else(|| tool_call.get("displayName"))
                .and_then(value_to_string)
                .unwrap_or_else(|| "tool".to_string());
            blocks.push(EventBlock::ToolCall {
                tool_call_id: id.clone(),
                name,
                input: tool_call.get("args").cloned(),
            });
            if let Some(result) = tool_call.get("result") {
                let content = extract_text(result);
                blocks.push(EventBlock::ToolResult {
                    tool_call_id: id,
                    content,
                    is_error: tool_call
                        .get("status")
                        .and_then(|value| value.as_str())
                        .map(|status| status.eq_ignore_ascii_case("error"))
                        .unwrap_or(false),
                });
            }
        }
    }

    if blocks.is_empty() && looks_like_non_text_message(message) {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: MappingDisposition::Preserved,
            code: "provider_message_preserved".to_string(),
            message: "Preserved provider message with unsupported block structure".to_string(),
            path: Some(format!("messages:{}", index)),
            raw: Some(raw_item.clone()),
        });
        blocks.push(EventBlock::ProviderPayload {
            kind: "message".to_string(),
            payload: message.clone(),
        });
    }

    blocks
}

fn message_text(message: &Value) -> String {
    for key in ["content", "displayContent", "text", "prompt", "response"] {
        let Some(value) = message.get(key) else {
            continue;
        };
        let text = extract_text(value);
        if !text.trim().is_empty() {
            return text;
        }
    }
    String::new()
}

fn looks_like_non_text_message(message: &Value) -> bool {
    message.get("toolCalls").is_some()
        || message.get("parts").is_some()
        || message.get("content").is_some()
        || message.get("message").is_some()
}

fn role_string(message: &Value) -> String {
    for key in ["role", "type", "author", "sender"] {
        if let Some(role) = message.get(key).and_then(value_to_string) {
            return role;
        }
    }
    "unknown".to_string()
}

fn role_from_string(role: &str) -> EventRole {
    match role.to_ascii_lowercase().as_str() {
        "user" | "human" => EventRole::User,
        "assistant" | "ai" | "agent" | "bot" | "gemini" | "copilot" => EventRole::Assistant,
        "tool" | "tool_result" | "tool-call" => EventRole::Tool,
        "system" => EventRole::System,
        "developer" => EventRole::Developer,
        _ => EventRole::Unknown,
    }
}

fn message_id(message: &Value) -> Option<String> {
    for key in ["id", "messageId", "message_id"] {
        if let Some(id) = message.get(key).and_then(value_to_string) {
            return Some(id);
        }
    }
    None
}

fn event_kind(blocks: &[EventBlock]) -> SessionEventKind {
    if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolResult { .. }))
    {
        SessionEventKind::ToolResult
    } else if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolCall { .. }))
    {
        SessionEventKind::ToolCall
    } else if blocks.iter().all(|block| {
        matches!(
            block,
            EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. }
        )
    }) {
        SessionEventKind::Unknown
    } else {
        SessionEventKind::Message
    }
}

fn usage_from_message(message: &Value) -> Option<UsageStats> {
    let tokens = message.get("tokens").or_else(|| message.get("usage"))?;
    Some(UsageStats {
        input_tokens: tokens.get("input").and_then(Value::as_u64),
        output_tokens: tokens
            .get("output")
            .or_else(|| tokens.get("candidates"))
            .and_then(Value::as_u64),
        total_tokens: tokens.get("total").and_then(Value::as_u64),
    })
}

fn datetime_from_value_keys(value: &Value, keys: &[&str]) -> Option<DateTime<Utc>> {
    timestamp_from_value_keys(value, keys).and_then(DateTime::<Utc>::from_timestamp_millis)
}

fn timestamp_from_value_keys(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(parse_timestamp_to_ms))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn path_mtime_datetime(path: &Path) -> Option<DateTime<Utc>> {
    path_mtime_ms(path).and_then(DateTime::<Utc>::from_timestamp_millis)
}

fn path_mtime_ms(path: &Path) -> Option<i64> {
    std::fs::metadata(path)
        .ok()?
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    const JSONL_WITH_SESSION_META: &str = r#"{"type":"session_meta","payload":{"id":"sess-1","cwd":"/tmp/proj","model_provider":"codex"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hello"}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}}
"#;

    const JSONL_CLAUDE_STYLE: &str = r#"{"sessionId":"abc","cwd":"/work","type":"user","message":{"role":"user","content":"ping"}}
{"sessionId":"abc","cwd":"/work","type":"assistant","message":{"role":"assistant","content":"pong"}}
"#;

    const JSONL_WITH_CONTROL_LINES: &str = r#"{"sessionId":"ctrl","type":"session_meta","payload":{"id":"ctrl","cwd":"/x"}}
{"type":"turn_context","payload":{"turn_id":"t1"}}
{"type":"compacted","message":"summarized"}
{"sessionId":"ctrl","type":"user","message":{"role":"user","content":"hi"}}
"#;


    fn spec() -> JsonProviderSpec {
        JsonProviderSpec {
            provider_id: "example",
            extension_key: "example_session",
            roots: Vec::new,
        }
    }

    #[test]
    fn imports_gemini_style_conversation_record() {
        let value = json!({
            "sessionId": "abc",
            "projectHash": "proj",
            "startTime": "2026-06-08T01:00:00Z",
            "lastUpdated": "2026-06-08T01:02:00Z",
            "messages": [
                {"id": "u1", "type": "user", "timestamp": "2026-06-08T01:00:00Z", "content": "hello"},
                {"id": "g1", "type": "gemini", "timestamp": "2026-06-08T01:01:00Z", "content": [{"text": "hi"}], "tokens": {"input": 1, "output": 2, "total": 3}}
            ]
        });

        let imported = import_session_from_value(spec(), Path::new("session-abc.json"), value)
            .expect("import session");

        assert_eq!(imported.session.identity.canonical_id, "abc");
        assert_eq!(imported.session.events.len(), 2);
        assert_eq!(imported.session.events[1].role, EventRole::Assistant);
        assert_eq!(
            imported.session.events[1]
                .metadata
                .usage
                .as_ref()
                .and_then(|usage| usage.total_tokens),
            Some(3)
        );
    }

    #[test]
    fn ignores_plain_config_json_without_messages() {
        let value = json!({"theme": "dark", "enabled": true});
        assert!(!looks_like_session(&value));
    }

    #[test]
    fn read_jsonl_merges_session_meta_and_messages() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rollout-test.jsonl");
        std::fs::write(&file, JSONL_WITH_SESSION_META).unwrap();

        let value = read_session_value(&file).expect("read jsonl");
        assert_eq!(value.get("id").and_then(|v| v.as_str()), Some("sess-1"));
        assert_eq!(value.get("cwd").and_then(|v| v.as_str()), Some("/tmp/proj"));
        let messages = value.get("messages").and_then(|v| v.as_array()).unwrap();
        assert_eq!(messages.len(), 2, "two message lines preserved");
    }

    #[test]
    fn read_jsonl_promotes_metadata_from_repeated_rows() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session-abc.jsonl");
        std::fs::write(&file, JSONL_CLAUDE_STYLE).unwrap();

        let value = read_session_value(&file).expect("read");
        assert_eq!(value.get("sessionId").and_then(|v| v.as_str()), Some("abc"));
        assert_eq!(value.get("cwd").and_then(|v| v.as_str()), Some("/work"));
        assert_eq!(
            value.get("messages").and_then(|v| v.as_array()).unwrap().len(),
            2,
        );
    }

    #[test]
    fn read_jsonl_skips_control_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("session-ctrl.jsonl");
        std::fs::write(&file, JSONL_WITH_CONTROL_LINES).unwrap();

        let value = read_session_value(&file).expect("read");
        let messages = value.get("messages").and_then(|v| v.as_array()).unwrap();
        assert_eq!(messages.len(), 1, "only the user message survives");
    }

}
