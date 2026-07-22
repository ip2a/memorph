pub mod adapter;
pub mod hook;
use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ImportedSession, MappingDirection, MappingDisposition, MappingReport,
    ProviderSessionRef, SessionContext, SessionEvent, SessionEventKind, SessionIdentity,
    SessionProvenance,
};
use crate::provider::{
    PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionSummary, ProviderSourceFingerprint, ProviderWriteRisk,
    ResumeQuality, ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

pub struct HermesProvider;
const PROVIDER_ID: &str = "hermes";

impl Provider for HermesProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "Hermes"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            resume: true,
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
            resume_quality: ResumeQuality::Native,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: false,
                sqlite: true,
                sidecar_files: true,
                index_repair: false,
            },
            backup_support: ProviderBackupSupport {
                before_write: false,
                restore: false,
                sync_only: false,
            },
            activity_support: ProviderActivitySupport {
                hook_events: false,
                runtime_endpoint: false,
                session_activity: false,
            },
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        scan_sessions_from_db(&state_db_path())
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let (db, id) = parse_source_locator(source_path)?;
        let conn = open_read_only(&db)?;
        let meta = conn
            .query_row(
                "SELECT id, title, cwd, model, started_at FROM sessions WHERE id = ?1",
                [&id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, f64>(4)?,
                    ))
                },
            )
            .with_context(|| format!("Hermes session not found: {id}"))?;
        let mut stmt = conn.prepare(
            "SELECT id, role, content, tool_call_id, tool_calls, tool_name, effect_disposition, timestamp, reasoning, reasoning_content, reasoning_details, compacted, active, api_content FROM messages WHERE session_id = ?1 ORDER BY id",
        )?;
        let mut events = Vec::new();
        let mut rows = stmt.query([&id])?;
        while let Some(row) = rows.next()? {
            let active: i64 = row.get(12)?;
            if active == 0 {
                continue;
            }
            let message_id: i64 = row.get(0)?;
            let role: String = row.get(1)?;
            let content: Option<String> = row.get(2)?;
            let tool_call_id: Option<String> = row.get(3)?;
            let tool_calls: Option<String> = row.get(4)?;
            let tool_name: Option<String> = row.get(5)?;
            let effect_disposition: Option<String> = row.get(6)?;
            let timestamp: f64 = row.get(7)?;
            let reasoning: Option<String> = row.get(8)?;
            let reasoning_content: Option<String> = row.get(9)?;
            let reasoning_details: Option<String> = row.get(10)?;
            let compacted: i64 = row.get(11)?;
            let api_content: Option<String> = row.get(13)?;
            let raw_message = serde_json::json!({
                "id": message_id,
                "role": role,
                "content": content,
                "tool_call_id": tool_call_id,
                "tool_calls": tool_calls.as_deref().and_then(|raw| serde_json::from_str::<Value>(raw).ok()),
                "tool_name": tool_name,
                "effect_disposition": effect_disposition,
                "timestamp": timestamp,
                "reasoning": reasoning,
                "reasoning_content": reasoning_content,
                "reasoning_details": reasoning_details,
                "compacted": compacted != 0,
                "active": active != 0,
                "api_content": api_content,
            });
            let event_role = match role.as_str() {
                "user" => EventRole::User,
                "assistant" => EventRole::Assistant,
                "tool" => EventRole::Tool,
                "system" => EventRole::System,
                _ => EventRole::Unknown,
            };
            let mut blocks = Vec::new();
            if tool_name.is_none() {
                if let Some(text) = content.clone().filter(|v| !v.trim().is_empty()) {
                    blocks.push(EventBlock::Text { text });
                }
            }
            if let Some(text) = reasoning
                .clone()
                .or(reasoning_content.clone())
                .or(reasoning_details.clone())
                .filter(|v| !v.trim().is_empty())
            {
                blocks.push(EventBlock::Thinking {
                    text,
                    signature: None,
                });
            }
            if let Some(name) = tool_name.clone() {
                blocks.push(EventBlock::ToolResult {
                    tool_call_id: tool_call_id
                        .clone()
                        .unwrap_or_else(|| message_id.to_string()),
                    content: content.clone().unwrap_or_default(),
                    is_error: false,
                });
                let _ = name;
            }
            if let Some(raw) = tool_calls.as_deref() {
                if let Ok(value) = serde_json::from_str::<Value>(raw) {
                    if let Some(items) = value.as_array() {
                        for item in items {
                            let function = item.get("function").unwrap_or(item);
                            let name = function
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("tool")
                                .to_string();
                            let id = item
                                .get("id")
                                .and_then(Value::as_str)
                                .or(tool_call_id.as_deref())
                                .unwrap_or("unknown")
                                .to_string();
                            let input = function
                                .get("arguments")
                                .cloned()
                                .or_else(|| function.get("input").cloned())
                                .and_then(|value| match value {
                                    Value::String(raw) => {
                                        serde_json::from_str(&raw).ok().or(Some(Value::String(raw)))
                                    }
                                    value => Some(value),
                                });
                            blocks.push(EventBlock::ToolCall {
                                tool_call_id: id,
                                name,
                                input,
                            });
                        }
                    }
                }
            }
            let kind = if tool_name.is_some() {
                SessionEventKind::ToolResult
            } else if tool_calls.is_some() {
                SessionEventKind::ToolCall
            } else {
                SessionEventKind::Message
            };
            events.push(SessionEvent {
                id: message_id.to_string(),
                kind,
                role: event_role,
                timestamp: timestamp_ms(timestamp)
                    .map(datetime_from_ms)
                    .unwrap_or_else(Utc::now),
                links: EventLinks::default(),
                blocks,
                metadata: EventMetadata {
                    source: EventSource {
                        provider_id: PROVIDER_ID.to_string(),
                        original_id: Some(message_id.to_string()),
                        original_role: Some(role.clone()),
                        phase: None,
                    },
                    model: meta.3.clone(),
                    usage: None,
                    fidelity: MappingDisposition::Preserved,
                    provider_ext: {
                        let mut ext = BTreeMap::new();
                        ext.insert("hermes_message".into(), raw_message);
                        ext
                    },
                },
            });
        }
        let created = timestamp_ms(meta.4).map(datetime_from_ms);
        let last = events.last().map(|event| event.timestamp).or(created);
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        report.push_issue(crate::canonical::MappingIssue {
            level: crate::canonical::MappingIssueLevel::Info,
            disposition: MappingDisposition::Preserved,
            code: "hermes-sqlite-source".into(),
            message: "Imported from Hermes state.db sessions/messages tables".into(),
            path: Some("source".into()),
            raw: None,
        });
        Ok(ImportedSession {
            session: CanonicalSession {
                schema: CanonicalSchema::default(),
                identity: SessionIdentity {
                    canonical_id: meta.0.clone(),
                    source_title: meta.1,
                },
                provenance: SessionProvenance {
                    imported_at: Utc::now(),
                    imported_by: Some("memorph-cli".into()),
                    primary_source: ProviderSessionRef {
                        provider_id: PROVIDER_ID.into(),
                        session_id: meta.0,
                        source_path: Some(source_path.into()),
                    },
                    aliases: Vec::new(),
                },
                context: SessionContext {
                    workspace_dir: meta.2,
                    created_at: created,
                    last_active_at: last,
                    tags: Vec::new(),
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
        source_fingerprint(source_path)
    }
    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("hermes --resume {session_id}"))
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        vec![state_db_path()]
    }
}

fn state_db_path() -> PathBuf {
    std::env::var_os("HERMES_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".hermes")))
        .unwrap_or_else(|| PathBuf::from(".hermes"))
        .join("state.db")
}
fn source_locator_for(db: &Path, id: &str) -> String {
    format!("{}#session={id}", db.display())
}
#[cfg(test)]
fn source_locator(id: &str) -> String {
    source_locator_for(&state_db_path(), id)
}
fn scan_sessions_from_db(db: &Path) -> Result<Vec<ProviderSessionSummary>> {
    if !db.exists() {
        return Ok(Vec::new());
    }
    let conn = open_read_only(db)?;
    let mut stmt = conn.prepare(
        "SELECT id, title, cwd, started_at FROM sessions WHERE archived = 0 ORDER BY started_at DESC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, f64>(3)?,
        ))
    })?;
    rows.map(|row| {
        let (id, title, cwd, started) = row?;
        Ok(ProviderSessionSummary {
            session_id: id.clone(),
            title,
            project_dir: cwd,
            created_at: None,
            last_active_at: timestamp_ms(started),
            source_path: Some(source_locator_for(db, &id)),
        })
    })
    .collect()
}
fn parse_source_locator(source: &str) -> Result<(PathBuf, String)> {
    let (path, fragment) = source
        .split_once('#')
        .context("Hermes source must be state.db#session=<id>")?;
    let id = fragment
        .strip_prefix("session=")
        .filter(|id| !id.is_empty())
        .context("Hermes source must contain session id")?;
    Ok((PathBuf::from(path), id.to_string()))
}
fn open_read_only(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open Hermes state db at {}", path.display()))
}
fn timestamp_ms(value: f64) -> Option<i64> {
    if value.is_finite() {
        Some((value * 1000.0).round() as i64)
    } else {
        None
    }
}
fn datetime_from_ms(value: i64) -> DateTime<Utc> {
    DateTime::<Utc>::from(UNIX_EPOCH + Duration::from_millis(value.max(0) as u64))
}
fn source_fingerprint(source: &str) -> Result<Option<ProviderSourceFingerprint>> {
    let (path, id) = parse_source_locator(source)?;
    let metadata = match std::fs::metadata(&path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let conn = open_read_only(&path)?;
    let session = conn
        .query_row(
            "SELECT CAST(started_at AS TEXT), CAST(ended_at AS TEXT), CAST(message_count AS TEXT), CAST(tool_call_count AS TEXT), CAST(archived AS TEXT), title, cwd, model FROM sessions WHERE id = ?1",
            [&id],
            |row| (0..8).map(|index| row.get::<_, Option<String>>(index)).collect::<rusqlite::Result<Vec<_>>>(),
        )
        .optional()?;
    let Some(session) = session else {
        return Ok(None);
    };
    let mut digest = Sha256::new();
    for value in session {
        digest.update(value.unwrap_or_default());
        digest.update([0]);
    }
    let mut stmt = conn.prepare(
        "SELECT CAST(id AS TEXT), role, content, tool_call_id, tool_calls, tool_name, effect_disposition, CAST(timestamp AS TEXT), reasoning, reasoning_content, reasoning_details, CAST(compacted AS TEXT), CAST(active AS TEXT), api_content FROM messages WHERE session_id = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([&id], |row| {
        (0..14)
            .map(|index| row.get::<_, Option<String>>(index))
            .collect::<rusqlite::Result<Vec<_>>>()
    })?;
    for row in rows {
        for value in row? {
            digest.update(value.unwrap_or_default());
            digest.update([0]);
        }
    }
    let modified_at_ms = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    let digest = digest.finalize();
    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes: metadata.len().min(i64::MAX as u64) as i64,
        value: format!("hermes-sqlite-v1:{id}:{digest:x}"),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_db() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("state.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, model TEXT, started_at REAL NOT NULL, ended_at REAL, message_count INTEGER, tool_call_count INTEGER, archived INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT, tool_call_id TEXT, tool_calls TEXT, tool_name TEXT, effect_disposition TEXT, timestamp REAL NOT NULL, reasoning TEXT, reasoning_content TEXT, reasoning_details TEXT, compacted INTEGER NOT NULL DEFAULT 0, active INTEGER NOT NULL DEFAULT 1, api_content TEXT);
             INSERT INTO sessions VALUES ('active','Fixture','/tmp/project','model-x',1000,NULL,3,1,0);
             INSERT INTO sessions VALUES ('archived','Hidden',NULL,NULL,900,NULL,0,0,1);
             INSERT INTO messages (id,session_id,role,content,tool_call_id,tool_calls,tool_name,effect_disposition,timestamp,reasoning,reasoning_content,reasoning_details,compacted,active,api_content) VALUES (1,'active','user','hello',NULL,NULL,NULL,NULL,1000,NULL,NULL,NULL,0,1,NULL);
             INSERT INTO messages (id,session_id,role,content,tool_call_id,tool_calls,tool_name,effect_disposition,timestamp,reasoning,reasoning_content,reasoning_details,compacted,active,api_content) VALUES (2,'active','assistant',NULL,NULL,'[{\"id\":\"call-1\",\"type\":\"function\",\"function\":{\"name\":\"terminal\",\"arguments\":\"{\\\"cmd\\\":\\\"pwd\\\"}\"}}]',NULL,NULL,1001,'thinking',NULL,NULL,0,1,'wire-content');
             INSERT INTO messages (id,session_id,role,content,tool_call_id,tool_calls,tool_name,effect_disposition,timestamp,reasoning,reasoning_content,reasoning_details,compacted,active,api_content) VALUES (3,'active','tool','done','call-1',NULL,'terminal',NULL,1002,NULL,NULL,NULL,0,1,NULL);
             INSERT INTO messages (id,session_id,role,content,tool_call_id,tool_calls,tool_name,effect_disposition,timestamp,reasoning,reasoning_content,reasoning_details,compacted,active,api_content) VALUES (4,'active','assistant','inactive',NULL,NULL,NULL,NULL,1003,NULL,NULL,NULL,0,0,NULL);"
        ).unwrap();
        dir
    }

    #[test]
    fn native_sqlite_scan_import_and_fingerprint() {
        let dir = fixture_db();
        let db = dir.path().join("state.db");
        let sessions = scan_sessions_from_db(&db).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "active");
        assert_eq!(sessions[0].last_active_at, Some(1_000_000));

        let source = source_locator_for(&db, "active");
        let imported = HermesProvider.import_session(&source).unwrap();
        assert_eq!(imported.session.events.len(), 3);
        assert!(matches!(
            imported.session.events[1].blocks[0],
            EventBlock::Thinking { .. }
        ));
        assert!(
            matches!(imported.session.events[1].blocks[1], EventBlock::ToolCall { ref name, .. } if name == "terminal")
        );
        assert!(
            matches!(imported.session.events[2].blocks[0], EventBlock::ToolResult { ref content, .. } if content == "done")
        );
        assert_eq!(
            imported.session.events[1].metadata.provider_ext["hermes_message"]["tool_calls"],
            serde_json::json!([{
                "id": "call-1",
                "type": "function",
                "function": {
                    "name": "terminal",
                    "arguments": "{\"cmd\":\"pwd\"}"
                }
            }])
        );
        assert_eq!(
            imported.session.events[1].metadata.provider_ext["hermes_message"]["api_content"],
            "wire-content"
        );
        assert!(source_fingerprint(&source).unwrap().is_some());
        assert!(source_fingerprint(&source_locator_for(&db, "missing"))
            .unwrap()
            .is_none());
    }

    #[test]
    fn source_locator_round_trips() {
        let source = source_locator("abc");
        let (path, id) = parse_source_locator(&source).unwrap();
        assert_eq!(path, state_db_path());
        assert_eq!(id, "abc");
    }
}
