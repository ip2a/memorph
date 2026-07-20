use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ImportedSession, MappingDirection, MappingDisposition, MappingIssue,
    MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext, SessionEvent,
    SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::provider::{
    PageStrategy, Provider, ProviderCapabilities, ProviderContentFidelity, ProviderSessionSummary,
    ProviderSourceFingerprint, ScanStrategy, StorageShape, TurnQuality,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

pub struct OpenClawProvider;
const PROVIDER_ID: &str = "openclaw";
const DATABASE_NAME: &str = "openclaw-agent.sqlite";

impl Provider for OpenClawProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "OpenClaw"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            storage_shape: StorageShape::Sqlite,
            scan_strategy: ScanStrategy::Indexed,
            page_strategy: PageStrategy::FullImport,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Preserved),
                tool_call: Some(MappingDisposition::Preserved),
                tool_result: Some(MappingDisposition::Preserved),
                provider_payload: Some(MappingDisposition::Preserved),
                ..ProviderContentFidelity::unknown()
            },
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        scan_root(&openclaw_agents_dir())
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let (db, agent_id, session_id) = parse_locator(source_path)?;
        let conn = open_read_only(&db)?;
        let metadata = conn.query_row(
            "SELECT session_key, display_name, model, created_at, updated_at FROM sessions WHERE session_id = ?1",
            [&session_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?)),
        ).with_context(|| format!("OpenClaw session not found: {session_id}"))?;

        let mut stmt = conn.prepare(
            "SELECT seq, event_json, created_at FROM transcript_events WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([&session_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?;
        let mut events = Vec::new();
        for row in rows {
            let (seq, raw, created_at) = row?;
            let value: Value = serde_json::from_str(&raw)
                .with_context(|| format!("invalid OpenClaw transcript event at sequence {seq}"))?;
            if let Some(event) = event_from_value(seq, created_at, value) {
                events.push(event);
            }
        }

        let title = metadata.1.or_else(|| first_user_text(&events));
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: MappingDisposition::Preserved,
            code: "openclaw-sqlite-source".into(),
            message: "Imported from OpenClaw sessions and transcript_events tables".into(),
            path: Some("source".into()),
            raw: None,
        });
        Ok(ImportedSession {
            session: CanonicalSession {
                schema: CanonicalSchema::default(),
                identity: SessionIdentity {
                    canonical_id: session_id.clone(),
                    source_title: title,
                },
                provenance: SessionProvenance {
                    imported_at: Utc::now(),
                    imported_by: Some("memorph-cli".into()),
                    primary_source: ProviderSessionRef {
                        provider_id: PROVIDER_ID.into(),
                        session_id: session_id.clone(),
                        source_path: Some(source_path.into()),
                    },
                    aliases: Vec::new(),
                },
                context: SessionContext {
                    workspace_dir: None,
                    created_at: Some(datetime_from_ms(metadata.3)),
                    last_active_at: Some(datetime_from_ms(metadata.4)),
                    tags: vec![format!("agent:{agent_id}")],
                },
                events,
                artifacts: Vec::new(),
                extensions: BTreeMap::new(),
            },
            report,
        })
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        fingerprint(source_path)
    }
}

fn scan_root(root: &Path) -> Result<Vec<ProviderSessionSummary>> {
    let mut sessions = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return Ok(sessions);
    };
    for entry in entries.flatten() {
        let agent_id = entry.file_name().to_string_lossy().into_owned();
        let db = entry.path().join("agent").join(DATABASE_NAME);
        if !db.is_file() {
            continue;
        }
        let conn = match open_read_only(&db) {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        let mut stmt = match conn.prepare("SELECT session_id, session_key, display_name, updated_at FROM sessions ORDER BY updated_at DESC") {
            Ok(stmt) => stmt,
            Err(_) => continue,
        };
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;
        for row in rows {
            let (session_id, _session_key, title, updated_at) = row?;
            sessions.push(ProviderSessionSummary {
                source_path: Some(locator(&db, &agent_id, &session_id)),
                session_id,
                title,
                project_dir: None,
                last_active_at: Some(updated_at),
            });
        }
    }
    sessions.sort_by_key(|session| std::cmp::Reverse(session.last_active_at.unwrap_or(0)));
    Ok(sessions)
}

fn event_from_value(seq: i64, created_at: i64, value: Value) -> Option<SessionEvent> {
    if value.get("type").and_then(Value::as_str) != Some("message") {
        return None;
    }
    let message = value.get("message")?;
    let role_text = message
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let role = match role_text {
        "user" => EventRole::User,
        "assistant" => EventRole::Assistant,
        "tool" => EventRole::Tool,
        "system" => EventRole::System,
        _ => EventRole::Unknown,
    };
    let mut blocks = Vec::new();
    match message.get("content") {
        Some(Value::String(text)) if !text.is_empty() => {
            blocks.push(EventBlock::Text { text: text.clone() })
        }
        Some(Value::Array(items)) => {
            for item in items {
                match item.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(text) = item.get("text").and_then(Value::as_str) {
                            blocks.push(EventBlock::Text { text: text.into() });
                        }
                    }
                    Some("thinking") => {
                        if let Some(text) = item
                            .get("thinking")
                            .or_else(|| item.get("text"))
                            .and_then(Value::as_str)
                        {
                            blocks.push(EventBlock::Thinking {
                                text: text.into(),
                                signature: item
                                    .get("signature")
                                    .and_then(Value::as_str)
                                    .map(str::to_owned),
                            });
                        }
                    }
                    Some("tool_use") => blocks.push(EventBlock::ToolCall {
                        tool_call_id: item
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .into(),
                        name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool")
                            .into(),
                        input: item.get("input").cloned(),
                    }),
                    Some("tool_result") => blocks.push(EventBlock::ToolResult {
                        tool_call_id: item
                            .get("tool_use_id")
                            .or_else(|| item.get("toolCallId"))
                            .and_then(Value::as_str)
                            .unwrap_or("unknown")
                            .into(),
                        content: text_content(item.get("content")),
                        is_error: item
                            .get("is_error")
                            .or_else(|| item.get("isError"))
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    }),
                    _ => blocks.push(EventBlock::ProviderPayload {
                        kind: "openclaw-content".into(),
                        payload: item.clone(),
                    }),
                }
            }
        }
        _ => {}
    }
    if blocks.is_empty() {
        blocks.push(EventBlock::ProviderPayload {
            kind: "openclaw-message".into(),
            payload: message.clone(),
        });
    }
    let kind = if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolResult { .. }))
    {
        SessionEventKind::ToolResult
    } else if blocks
        .iter()
        .any(|block| matches!(block, EventBlock::ToolCall { .. }))
    {
        SessionEventKind::ToolCall
    } else {
        SessionEventKind::Message
    };
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .unwrap_or_else(|| seq.to_string());
    Some(SessionEvent {
        id: id.clone(),
        kind,
        role,
        timestamp: datetime_from_ms(created_at),
        links: EventLinks::default(),
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.into(),
                original_id: Some(id),
                original_role: Some(role_text.into()),
                phase: None,
            },
            model: message
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_owned),
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: BTreeMap::new(),
        },
    })
}

fn text_content(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => String::new(),
    }
}
fn first_user_text(events: &[SessionEvent]) -> Option<String> {
    events
        .iter()
        .find(|event| event.role == EventRole::User)
        .and_then(|event| {
            event.blocks.iter().find_map(|block| match block {
                EventBlock::Text { text } => Some(text.chars().take(80).collect()),
                _ => None,
            })
        })
}
fn openclaw_agents_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".openclaw/agents")
}
fn locator(db: &Path, agent_id: &str, session_id: &str) -> String {
    format!(
        "{}#agent={agent_id}&session={session_id}",
        db.to_string_lossy()
    )
}
fn parse_locator(source: &str) -> Result<(PathBuf, String, String)> {
    let (path, fragment) = source
        .split_once('#')
        .context("OpenClaw source must contain agent and session ids")?;
    let mut agent = None;
    let mut session = None;
    for item in fragment.split('&') {
        if let Some(value) = item.strip_prefix("agent=") {
            agent = Some(value.to_string());
        }
        if let Some(value) = item.strip_prefix("session=") {
            session = Some(value.to_string());
        }
    }
    Ok((
        PathBuf::from(path),
        agent
            .filter(|v| !v.is_empty())
            .context("OpenClaw source agent id is empty")?,
        session
            .filter(|v| !v.is_empty())
            .context("OpenClaw source session id is empty")?,
    ))
}
fn open_read_only(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open OpenClaw database at {}", path.display()))
}
fn datetime_from_ms(value: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_millis(value.max(0) as u64))
}
fn fingerprint(source: &str) -> Result<Option<ProviderSourceFingerprint>> {
    let (db, _, session_id) = parse_locator(source)?;
    let metadata = match std::fs::metadata(&db) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let conn = open_read_only(&db)?;
    let updated = conn
        .query_row(
            "SELECT updated_at FROM sessions WHERE session_id = ?1",
            [&session_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?;
    let Some(updated) = updated else {
        return Ok(None);
    };
    let mut digest = Sha256::new();
    digest.update(updated.to_le_bytes());
    let mut stmt = conn.prepare("SELECT seq, event_json, created_at FROM transcript_events WHERE session_id = ?1 ORDER BY seq")?;
    let rows = stmt.query_map([&session_id], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (seq, event, created) = row?;
        digest.update(seq.to_le_bytes());
        digest.update(event);
        digest.update(created.to_le_bytes());
    }
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes: metadata.len().min(i64::MAX as u64) as i64,
        value: format!("openclaw-sqlite-v1:{session_id}:{:x}", digest.finalize()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sqlite_scan_import_and_fingerprint() {
        let root = tempfile::tempdir().unwrap();
        let agent_dir = root.path().join("main/agent");
        std::fs::create_dir_all(&agent_dir).unwrap();
        let db = agent_dir.join(DATABASE_NAME);
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE sessions (session_id TEXT PRIMARY KEY, session_key TEXT NOT NULL, display_name TEXT, model TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL); CREATE TABLE transcript_events (session_id TEXT NOT NULL, seq INTEGER NOT NULL, event_json TEXT NOT NULL, created_at INTEGER NOT NULL, PRIMARY KEY(session_id, seq)); INSERT INTO sessions VALUES ('s1','agent:main:main','Fixture','model-x',1000,3000); INSERT INTO transcript_events VALUES ('s1',1,'{\"type\":\"message\",\"id\":\"m1\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}',1000); INSERT INTO transcript_events VALUES ('s1',2,'{\"type\":\"message\",\"id\":\"m2\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"tool_use\",\"id\":\"c1\",\"name\":\"exec\",\"input\":{\"cmd\":\"pwd\"}}]}}',2000);").unwrap();
        drop(conn);
        let sessions = scan_root(root.path()).unwrap();
        assert_eq!(sessions.len(), 1);
        let imported = OpenClawProvider
            .import_session(sessions[0].source_path.as_deref().unwrap())
            .unwrap();
        assert_eq!(imported.session.events.len(), 2);
        assert!(
            matches!(imported.session.events[1].blocks[0], EventBlock::ToolCall { ref name, .. } if name == "exec")
        );
        assert!(fingerprint(sessions[0].source_path.as_deref().unwrap())
            .unwrap()
            .is_some());
    }
}
