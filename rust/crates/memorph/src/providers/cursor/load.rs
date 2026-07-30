use crate::providers::cursor::db::{load_source, CursorBubbleRecord, CursorLoadedSource};
use crate::session::{
    Block, Context, Event, EventKind, EventMeta, EventSource, Fidelity, Identity, ImportedSession,
    Links, MappingDirection, MappingIssue, MappingIssueLevel, MappingReport, Metadata, Provenance,
    ProviderRef, Role, Schema, Session, Usage,
};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::collections::BTreeMap;

const PROVIDER_ID: &str = "cursor";

pub fn import_session(source_locator: &str) -> Result<ImportedSession> {
    let source = load_source(source_locator)?;
    Ok(imported_session_from_cursor(source, source_locator))
}

fn imported_session_from_cursor(
    source: CursorLoadedSource,
    source_locator: &str,
) -> ImportedSession {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    if source.metadata.header.is_none() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: Fidelity::Normalized,
            code: "cursor_missing_composer_header".to_string(),
            message: "Imported a Cursor data-only session without composerHeaders metadata"
                .to_string(),
            path: Some(format!("composerData:{}", source.metadata.composer_id)),
            raw: None,
        });
    }
    if source.metadata.composer.is_none() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: Fidelity::Normalized,
            code: "cursor_missing_composer_data".to_string(),
            message: "Imported a Cursor header-only session without composerData metadata"
                .to_string(),
            path: Some(format!("composerHeaders:{}", source.metadata.composer_id)),
            raw: None,
        });
    }
    for invalid in &source.invalid_bubbles {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: Fidelity::Dropped,
            code: "cursor_invalid_bubble_row".to_string(),
            message: invalid.error.clone(),
            path: Some(invalid.key.clone()),
            raw: Some(invalid.raw.clone()),
        });
    }

    let timestamp_base = source
        .metadata
        .created_at_ms()
        .and_then(DateTime::from_timestamp_millis)
        .unwrap_or(DateTime::UNIX_EPOCH);
    let mut imported_events = Vec::new();
    let mut tool_payload_count = 0usize;
    let mut thinking_count = 0usize;
    let mut unmapped_structured_count = 0usize;
    for (source_order, bubble) in source.bubbles.iter().enumerate() {
        let imported = event_from_bubble(
            bubble,
            &source.metadata.composer_id,
            source_order,
            timestamp_base,
            &mut report,
        );
        tool_payload_count += usize::from(imported.has_tool_payload);
        thinking_count += usize::from(imported.has_thinking);
        unmapped_structured_count += usize::from(imported.has_unmapped_structured_payload);
        imported_events.push(imported);
    }
    imported_events.sort_by(|left, right| {
        left.event
            .timestamp
            .cmp(&right.event.timestamp)
            .then_with(|| left.event.id.cmp(&right.event.id))
    });
    let (events, event_meta): (Vec<_>, Vec<_>) = imported_events
        .into_iter()
        .map(|imported| (imported.event, imported.event_meta))
        .unzip();

    if tool_payload_count > 0 {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: Fidelity::Normalized,
            code: "cursor_tool_payload_normalized".to_string(),
            message: format!(
                "Mapped toolFormerData into canonical tool blocks for {tool_payload_count} Cursor bubbles"
            ),
            path: None,
            raw: None,
        });
    }
    if thinking_count > 0 {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: Fidelity::Normalized,
            code: "cursor_thinking_payload_normalized".to_string(),
            message: format!(
                "Mapped Cursor thinking objects into canonical thinking blocks for {thinking_count} bubbles"
            ),
            path: None,
            raw: None,
        });
    }
    if unmapped_structured_count > 0 {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: Fidelity::Normalized,
            code: "cursor_structured_payload_preserved".to_string(),
            message: format!(
                "Preserved non-empty Cursor structured fields in provider payloads for {unmapped_structured_count} bubbles"
            ),
            path: None,
            raw: None,
        });
    }

    let created_at = source
        .metadata
        .created_at_ms()
        .and_then(DateTime::from_timestamp_millis)
        .or_else(|| events.first().map(|event| event.timestamp));
    let last_active_at = source
        .metadata
        .last_active_at_ms()
        .and_then(DateTime::from_timestamp_millis)
        .or_else(|| events.last().map(|event| event.timestamp))
        .or(created_at);
    let source_title = source
        .metadata
        .title()
        .or_else(|| infer_title_from_events(&events));
    let workspace_dir = source.metadata.workspace_dir();

    let mut extensions = BTreeMap::new();
    if let Some(header) = source.metadata.header.as_ref() {
        extensions.insert("cursor_composer_header".to_string(), header.value.clone());
    }
    if let Some(composer) = source.metadata.composer.as_ref() {
        extensions.insert("cursor_composer".to_string(), composer.raw.clone());
    }

    ImportedSession {
        session: Session {
            schema: Schema::default(),
            identity: Identity {
                id: source.metadata.composer_id.clone(),
                title: source_title,
            },
            context: Context {
                workspace: workspace_dir,
                created_at,
                last_active_at,
                tags: Vec::new(),
            },
            events,
            extensions,
        },
        provenance: Provenance {
            imported_at: Utc::now(),
            imported_by: Some("memorph-cli".to_string()),
            primary_source: ProviderRef {
                provider_id: PROVIDER_ID.to_string(),
                session_id: source.metadata.composer_id,
                source_path: Some(source_locator.to_string()),
            },
            aliases: Vec::new(),
        },
        event_meta,
        report,
    }
}

struct ImportedCursorEvent {
    event: Event,
    event_meta: EventMeta,
    has_tool_payload: bool,
    has_thinking: bool,
    has_unmapped_structured_payload: bool,
}

fn event_from_bubble(
    bubble: &CursorBubbleRecord,
    composer_id: &str,
    source_order: usize,
    timestamp_base: DateTime<Utc>,
    report: &mut MappingReport,
) -> ImportedCursorEvent {
    let key_prefix = format!("bubbleId:{composer_id}:");
    let key_bubble_id = bubble
        .key
        .strip_prefix(&key_prefix)
        .unwrap_or(bubble.key.as_str());
    let raw_bubble_id = bubble.raw.get("bubbleId").and_then(Value::as_str);
    let event_id = if raw_bubble_id == Some(key_bubble_id) {
        key_bubble_id.to_string()
    } else {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: Fidelity::Normalized,
            code: "cursor_bubble_identity_mismatch".to_string(),
            message: "Used the Cursor bubble key suffix as the canonical event ID".to_string(),
            path: Some(bubble.key.clone()),
            raw: bubble.raw.get("bubbleId").cloned(),
        });
        key_bubble_id.to_string()
    };

    let bubble_type = bubble.raw.get("type").and_then(Value::as_i64);
    let (role, mut fidelity) = match bubble_type {
        Some(1) => (Role::User, Fidelity::Preserved),
        Some(2) => (Role::Assistant, Fidelity::Preserved),
        other => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: Fidelity::Normalized,
                code: "cursor_unknown_bubble_type".to_string(),
                message: "Mapped an unknown Cursor bubble type to the canonical unknown role"
                    .to_string(),
                path: Some(bubble.key.clone()),
                raw: other.map(Value::from),
            });
            (Role::Other, Fidelity::Normalized)
        }
    };

    let timestamp = bubble
        .raw
        .get("createdAt")
        .and_then(Value::as_str)
        .and_then(parse_timestamp)
        .unwrap_or_else(|| {
            fidelity = fidelity.worst(Fidelity::Normalized);
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: Fidelity::Normalized,
                code: "cursor_invalid_bubble_timestamp".to_string(),
                message:
                    "Used the session creation time plus stable source order for a Cursor bubble"
                        .to_string(),
                path: Some(bubble.key.clone()),
                raw: bubble.raw.get("createdAt").cloned(),
            });
            timestamp_base + Duration::milliseconds(source_order as i64)
        });

    let mut blocks = Vec::new();
    if let Some(text) = bubble
        .raw
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        blocks.push(Block::Text {
            text: text.to_string(),
        });
    }

    let mut has_thinking = false;
    if let Some(thinking) = bubble.raw.get("thinking").and_then(Value::as_object) {
        if let Some(text) = thinking
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            has_thinking = true;
            fidelity = fidelity.worst(Fidelity::Normalized);
            blocks.push(Block::Thinking {
                text: text.to_string(),
                signature: thinking
                    .get("signature")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        }
    }

    let mut has_tool_payload = false;
    if let Some(tool) = bubble.raw.get("toolFormerData").and_then(Value::as_object) {
        let tool_call_id = tool.get("toolCallId").and_then(Value::as_str);
        let tool_name = tool.get("name").and_then(Value::as_str);
        if let (Some(tool_call_id), Some(tool_name)) = (tool_call_id, tool_name) {
            has_tool_payload = true;
            fidelity = fidelity.worst(Fidelity::Normalized);
            let input = tool
                .get("params")
                .or_else(|| tool.get("rawArgs"))
                .and_then(Value::as_str)
                .map(json_or_string);
            blocks.push(Block::ToolCall {
                tool_call_id: tool_call_id.to_string(),
                name: tool_name.to_string(),
                input,
            });
            let status = tool.get("status").and_then(Value::as_str);
            let result = tool
                .get("result")
                .and_then(Value::as_str)
                .map(|content| (content, false))
                .or_else(|| {
                    tool.get("error")
                        .and_then(Value::as_str)
                        .map(|content| (content, true))
                });
            if let Some((content, field_is_error)) = result {
                blocks.push(Block::ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    content: content.to_string(),
                    is_error: field_is_error || status == Some("error"),
                });
            }
        } else {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: Fidelity::Normalized,
                code: "cursor_incomplete_tool_payload".to_string(),
                message: "Preserved an incomplete Cursor toolFormerData object as provider payload"
                    .to_string(),
                path: Some(bubble.key.clone()),
                raw: Some(Value::Object(tool.clone())),
            });
            fidelity = fidelity.worst(Fidelity::Normalized);
        }
    }

    if blocks.is_empty() {
        blocks.push(Block::Other {
            raw: bubble.raw.clone(),
        });
    }

    let request_id = bubble
        .raw
        .get("requestId")
        .and_then(Value::as_str)
        .map(str::to_string);
    let model_info = bubble.raw.get("modelInfo");
    let mut provider_ext = BTreeMap::new();
    if let Some(request_id) = request_id.as_ref() {
        provider_ext.insert("request_id".to_string(), Value::String(request_id.clone()));
    }
    if let Some(model_info) = model_info {
        provider_ext.insert("model_info".to_string(), model_info.clone());
    }
    provider_ext.insert("cursor_bubble".to_string(), bubble.raw.clone());
    let has_unmapped_structured_payload = [
        "images",
        "codeBlocks",
        "gitDiffs",
        "interpreterResults",
        "fileDiffTrajectories",
        "humanChanges",
    ]
    .iter()
    .any(|key| value_is_nonempty(bubble.raw.get(*key)));

    ImportedCursorEvent {
        event: Event {
            id: event_id.clone(),
            kind: EventKind::Message,
            role,
            timestamp,
            links: Links {
                turn_id: request_id,
                ..Links::default()
            },
            blocks,
            metadata: Metadata {
                model: bubble_model_name(model_info),
                usage: bubble_usage(bubble.raw.get("tokenCount")),
            },
        },
        event_meta: EventMeta {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: raw_bubble_id.map(str::to_string),
                original_role: bubble_type.map(|value| value.to_string()),
                phase: None,
            },
            fidelity,
            provider_ext,
        },
        has_tool_payload,
        has_thinking,
        has_unmapped_structured_payload,
    }
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn bubble_model_name(model_info: Option<&Value>) -> Option<String> {
    let model_info = model_info?;
    ["name", "modelName", "model", "id"]
        .iter()
        .find_map(|key| model_info.get(*key).and_then(Value::as_str))
        .map(str::to_string)
}

fn bubble_usage(token_count: Option<&Value>) -> Option<Usage> {
    let token_count = token_count?.as_object()?;
    let input_tokens = token_count.get("inputTokens").and_then(Value::as_u64);
    let output_tokens = token_count.get("outputTokens").and_then(Value::as_u64);
    if input_tokens.is_none() && output_tokens.is_none() {
        return None;
    }
    Some(Usage {
        input_tokens,
        output_tokens,
        cache_read_tokens: None,
        cache_write_tokens: None,
        reasoning_tokens: None,
    })
}

fn json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
}

fn value_is_nonempty(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(values)) => !values.is_empty(),
        Some(Value::Object(values)) => !values.is_empty(),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Null) | None => false,
        Some(_) => true,
    }
}

fn infer_title_from_events(events: &[Event]) -> Option<String> {
    events.iter().find_map(|event| {
        event.blocks.iter().find_map(|block| match block {
            Block::Text { text } => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else if trimmed.chars().count() > 50 {
                    Some(format!(
                        "{}...",
                        trimmed.chars().take(50).collect::<String>()
                    ))
                } else {
                    Some(trimmed.to_string())
                }
            }
            _ => None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::cursor::db::{
        ComposerData, CursorComposerHeader, CursorComposerRecord, CursorInvalidRow,
        CursorSessionMetadata,
    };
    use crate::session::{Block, Fidelity, Role};
    use serde_json::json;

    fn composer_metadata(
        session_id: &str,
        header: Option<Value>,
        composer: Option<Value>,
    ) -> CursorSessionMetadata {
        CursorSessionMetadata {
            composer_id: session_id.to_string(),
            header: header.map(|value| CursorComposerHeader {
                created_at: value.get("createdAt").and_then(Value::as_i64),
                last_updated_at: value.get("lastUpdatedAt").and_then(Value::as_i64),
                recency: value.get("recency").and_then(Value::as_i64),
                value,
            }),
            composer: composer.map(|raw| CursorComposerRecord {
                data: serde_json::from_value::<ComposerData>(raw.clone()).unwrap(),
                raw,
            }),
        }
    }

    fn issue_codes(imported: &ImportedSession) -> Vec<&str> {
        imported
            .report
            .issues
            .iter()
            .map(|issue| issue.code.as_str())
            .collect()
    }

    #[test]
    fn full_current_source_maps_metadata_and_bubble_payloads() {
        let session_id = "composer-full";
        let created_at = 1_700_000_000_000i64;
        let header = json!({
            "composerId": session_id,
            "name": "Header title",
            "workspaceIdentifier": {"id": "workspace-1", "uri": {"fsPath": "/tmp/project"}},
            "createdAt": created_at,
            "lastUpdatedAt": created_at + 100,
            "recency": created_at + 200
        });
        let composer = json!({
            "composerId": session_id,
            "name": "Composer title",
            "text": "Composer text",
            "workspaceIdentifier": {"id": "workspace-1", "uri": {"fsPath": "/tmp/project"}},
            "createdAt": created_at,
            "lastUpdatedAt": created_at + 300
        });
        let source = CursorLoadedSource {
            metadata: composer_metadata(session_id, Some(header), Some(composer)),
            bubbles: vec![
                CursorBubbleRecord {
                    key: format!("bubbleId:{session_id}:user-1"),
                    raw: json!({
                        "bubbleId": "user-1",
                        "type": 1,
                        "text": "hello",
                        "createdAt": "2023-11-14T22:13:20Z"
                    }),
                },
                CursorBubbleRecord {
                    key: format!("bubbleId:{session_id}:assistant-1"),
                    raw: json!({
                        "bubbleId": "assistant-1",
                        "type": 2,
                        "text": "answer",
                        "createdAt": "invalid",
                        "thinking": {"text": "internal reasoning", "signature": "sig"},
                        "toolFormerData": {
                            "toolCallId": "call-1",
                            "name": "shell",
                            "params": "{\"command\":\"pwd\"}",
                            "result": "ok"
                        },
                        "tokenCount": {"inputTokens": 3, "outputTokens": 5},
                        "requestId": "turn-1",
                        "modelInfo": {"modelName": "gpt-test"},
                        "images": [{"id": "image-1"}],
                        "unknownField": true
                    }),
                },
            ],
            invalid_bubbles: Vec::new(),
        };

        let imported = imported_session_from_cursor(source, "db#composer=composer-full");

        assert_eq!(
            imported.session.identity.title.as_deref(),
            Some("Header title")
        );
        assert_eq!(
            imported.session.context.workspace.as_deref(),
            Some("/tmp/project")
        );
        assert_eq!(
            imported.session.context.last_active_at,
            DateTime::from_timestamp_millis(created_at + 200)
        );
        assert_eq!(imported.session.events.len(), 2);

        let user = imported
            .session
            .events
            .iter()
            .find(|event| event.id == "user-1")
            .unwrap();
        assert_eq!(user.role, Role::User);
        assert!(matches!(user.blocks[0], Block::Text { .. }));

        let assistant_index = imported
            .session
            .events
            .iter()
            .position(|event| event.id == "assistant-1")
            .unwrap();
        let assistant = &imported.session.events[assistant_index];
        let assistant_meta = &imported.event_meta[assistant_index];
        assert_eq!(assistant.role, Role::Assistant);
        assert_eq!(assistant.links.turn_id.as_deref(), Some("turn-1"));
        assert_eq!(assistant.metadata.model.as_deref(), Some("gpt-test"));
        assert_eq!(assistant_meta.fidelity, Fidelity::Normalized);
        assert_eq!(
            assistant.metadata.usage.as_ref().unwrap().input_tokens,
            Some(3)
        );
        assert_eq!(
            assistant.metadata.usage.as_ref().unwrap().output_tokens,
            Some(5)
        );
        assert!(assistant.blocks.iter().any(
            |block| matches!(block, Block::Thinking { text, .. } if text == "internal reasoning")
        ));
        assert!(assistant.blocks.iter().any(|block| matches!(
            block,
            Block::ToolCall { tool_call_id, name, .. }
                if tool_call_id == "call-1" && name == "shell"
        )));
        assert!(assistant.blocks.iter().any(|block| matches!(
            block,
            Block::ToolResult { tool_call_id, content, is_error }
                if tool_call_id == "call-1" && content == "ok" && !is_error
        )));
        assert_eq!(
            assistant_meta.provider_ext["cursor_bubble"]["unknownField"],
            true
        );

        let codes = issue_codes(&imported);
        assert!(codes.contains(&"cursor_invalid_bubble_timestamp"));
        assert!(codes.contains(&"cursor_tool_payload_normalized"));
        assert!(codes.contains(&"cursor_thinking_payload_normalized"));
        assert!(codes.contains(&"cursor_structured_payload_preserved"));
    }

    #[test]
    fn header_only_and_data_only_sources_are_imported_with_reports() {
        let session_id = "composer-partial";
        let header_only = imported_session_from_cursor(
            CursorLoadedSource {
                metadata: composer_metadata(
                    session_id,
                    Some(json!({
                        "composerId": session_id,
                        "name": "Header only",
                        "createdAt": 1_700_000_000_000i64
                    })),
                    None,
                ),
                bubbles: Vec::new(),
                invalid_bubbles: Vec::new(),
            },
            "db#composer=composer-partial",
        );
        assert_eq!(
            header_only.session.identity.title.as_deref(),
            Some("Header only")
        );
        assert!(issue_codes(&header_only).contains(&"cursor_missing_composer_data"));

        let data_only = imported_session_from_cursor(
            CursorLoadedSource {
                metadata: composer_metadata(
                    session_id,
                    None,
                    Some(json!({
                        "composerId": session_id,
                        "name": "Data only",
                        "createdAt": 1_700_000_000_000i64
                    })),
                ),
                bubbles: Vec::new(),
                invalid_bubbles: Vec::new(),
            },
            "db#composer=composer-partial",
        );
        assert_eq!(
            data_only.session.identity.title.as_deref(),
            Some("Data only")
        );
        assert!(issue_codes(&data_only).contains(&"cursor_missing_composer_header"));
    }

    #[test]
    fn bubble_key_is_canonical_and_unknown_or_invalid_fields_are_reported() {
        let session_id = "composer-normalization";
        let source = CursorLoadedSource {
            metadata: composer_metadata(
                session_id,
                None,
                Some(json!({
                    "composerId": session_id,
                    "createdAt": 1_700_000_000_000i64
                })),
            ),
            bubbles: vec![CursorBubbleRecord {
                key: format!("bubbleId:{session_id}:key-id"),
                raw: json!({
                    "bubbleId": "different-id",
                    "type": 99,
                    "createdAt": "not-a-timestamp"
                }),
            }],
            invalid_bubbles: vec![CursorInvalidRow {
                key: format!("bubbleId:{session_id}:invalid"),
                raw: json!("not an object"),
                error: "invalid bubble".to_string(),
            }],
        };

        let imported = imported_session_from_cursor(source, "db#composer=composer-normalization");
        assert_eq!(imported.session.events[0].id, "key-id");
        assert_eq!(imported.session.events[0].role, Role::Other);
        assert_eq!(
            imported.session.events[0].timestamp,
            DateTime::from_timestamp_millis(1_700_000_000_000).unwrap()
        );
        assert!(matches!(
            imported.session.events[0].blocks[0],
            Block::Other { .. }
        ));
        assert_eq!(imported.report.overall, Fidelity::Dropped);
        let codes = issue_codes(&imported);
        assert!(codes.contains(&"cursor_bubble_identity_mismatch"));
        assert!(codes.contains(&"cursor_unknown_bubble_type"));
        assert!(codes.contains(&"cursor_invalid_bubble_timestamp"));
        assert!(codes.contains(&"cursor_invalid_bubble_row"));
    }
}
