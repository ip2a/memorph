use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ImportedSession, MappingDirection, MappingDisposition, MappingReport,
    ProviderSessionRef, SessionContext, SessionEvent, SessionEventKind, SessionIdentity,
    SessionProvenance,
};
use crate::providers::cursor::db::{BubbleData, ComposerData, list_bubbles, list_composers};
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::BTreeMap;

pub fn import_session(composer_id: &str) -> Result<ImportedSession> {
    let composer = list_composers()?
        .into_iter()
        .find(|composer| composer.composer_id == composer_id);
    let bubbles = list_bubbles(composer_id)?;
    Ok(imported_session_from_cursor(composer_id, composer, bubbles))
}

fn imported_session_from_cursor(
    composer_id: &str,
    composer: Option<ComposerData>,
    mut bubbles: Vec<BubbleData>,
) -> ImportedSession {
    bubbles.sort_by(|a, b| {
        let a_ts = a.created_at.as_deref().unwrap_or("");
        let b_ts = b.created_at.as_deref().unwrap_or("");
        a_ts.cmp(b_ts)
    });

    let mut events = Vec::new();
    for bubble in bubbles {
        let role = bubble_role(bubble.bubble_type);
        let timestamp = bubble
            .created_at
            .as_deref()
            .and_then(parse_timestamp)
            .unwrap_or_else(Utc::now);
        let mut blocks = Vec::new();
        if let Some(text) = bubble
            .text
            .as_deref()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            blocks.push(EventBlock::Text {
                text: text.to_string(),
            });
        }
        if blocks.is_empty() {
            blocks.push(EventBlock::ProviderPayload {
                kind: "cursor_bubble".to_string(),
                payload: bubble_payload(&bubble),
            });
        }

        let mut provider_ext = BTreeMap::new();
        if let Some(request_id) = bubble.request_id.as_ref() {
            provider_ext.insert("request_id".to_string(), Value::String(request_id.clone()));
        }
        if let Some(model_info) = bubble.model_info.as_ref() {
            provider_ext.insert("model_info".to_string(), model_info.clone());
        }
        provider_ext.insert("cursor_bubble".to_string(), bubble_payload(&bubble));

        events.push(SessionEvent {
            id: bubble.bubble_id.clone(),
            kind: SessionEventKind::Message,
            role,
            timestamp,
            links: EventLinks::default(),
            blocks,
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "cursor".to_string(),
                    original_id: Some(bubble.bubble_id),
                    original_role: Some(bubble.bubble_type.to_string()),
                    phase: None,
                },
                model: bubble_model_name(bubble.model_info.as_ref()),
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext,
            },
        });
    }

    let created_at = composer
        .as_ref()
        .and_then(|composer| composer.created_at)
        .and_then(chrono::DateTime::from_timestamp_millis)
        .or_else(|| events.first().map(|event| event.timestamp));
    let last_active_at = events.last().map(|event| event.timestamp).or(created_at);
    let source_title = composer
        .as_ref()
        .and_then(cursor_source_title)
        .or_else(|| infer_title_from_events(&events));
    let workspace_dir = composer
        .as_ref()
        .and_then(|composer| composer.workspace_identifier.as_ref())
        .map(|workspace| workspace.uri.fs_path.clone());

    let mut extensions = BTreeMap::new();
    if let Some(composer) = composer.as_ref() {
        extensions.insert(
            "cursor_composer".to_string(),
            serde_json::to_value(composer).unwrap_or(Value::Null),
        );
    }

    ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: composer_id.to_string(),
                source_title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: "cursor".to_string(),
                    session_id: composer_id.to_string(),
                    source_path: Some(composer_id.to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir,
                created_at,
                last_active_at,
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report: MappingReport::new("cursor", MappingDirection::Import),
    }
}

fn bubble_role(bubble_type: i32) -> EventRole {
    match bubble_type {
        1 => EventRole::User,
        2 => EventRole::Assistant,
        _ => EventRole::System,
    }
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn bubble_model_name(model_info: Option<&Value>) -> Option<String> {
    let model_info = model_info?;
    ["name", "modelName", "model", "id"]
        .iter()
        .find_map(|key| model_info.get(*key).and_then(Value::as_str))
        .map(str::to_string)
}

fn cursor_source_title(composer: &ComposerData) -> Option<String> {
    composer
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .or_else(|| {
            composer
                .name
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
        })
}

fn infer_title_from_events(events: &[SessionEvent]) -> Option<String> {
    events.iter().find_map(|event| {
        event.blocks.iter().find_map(|block| match block {
            EventBlock::Text { text } => {
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

fn bubble_payload(bubble: &BubbleData) -> Value {
    serde_json::to_value(bubble).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::cursor::db::{WorkspaceIdentifier, WorkspaceUri};

    #[test]
    fn imported_cursor_session_preserves_model_info_and_workspace() {
        let composer = ComposerData {
            composer_id: "composer-1".to_string(),
            status: None,
            text: Some("Workspace task".to_string()),
            name: None,
            workspace_identifier: Some(WorkspaceIdentifier {
                id: "workspace-1".to_string(),
                uri: WorkspaceUri {
                    fs_path: "/tmp/project".to_string(),
                },
            }),
            created_at: Some(1_700_000_000_000),
            is_agentic: Some(true),
        };
        let bubbles = vec![BubbleData {
            bubble_id: "bubble-1".to_string(),
            bubble_type: 2,
            text: Some("Hello from Cursor".to_string()),
            created_at: Some("2024-01-01T00:00:00Z".to_string()),
            request_id: Some("request-1".to_string()),
            model_info: Some(serde_json::json!({
                "modelName": "gpt-4.1"
            })),
        }];

        let imported = imported_session_from_cursor("composer-1", Some(composer), bubbles);
        let event = &imported.session.events[0];

        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some("/tmp/project")
        );
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Workspace task")
        );
        assert_eq!(event.metadata.model.as_deref(), Some("gpt-4.1"));
        assert_eq!(
            event
                .metadata
                .provider_ext
                .get("request_id")
                .and_then(Value::as_str),
            Some("request-1")
        );
        assert!(matches!(event.blocks[0], EventBlock::Text { .. }));
    }
}
