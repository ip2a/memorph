use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::provider::{
    canonical_event_role_label, canonical_event_text, canonical_export_result,
    canonical_session_title, Provider, ProviderCapabilities, ProviderSessionSummary,
};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct DeepseekProvider;

const PROVIDER_ID: &str = "deepseek";

impl Provider for DeepseekProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "DeepSeek"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::full_session_management()
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(Vec::new());
        }
        let conn = Connection::open(&db_path).with_context(|| {
            format!("failed to open DeepSeek state db at {}", db_path.display())
        })?;

        let mut stmt = conn.prepare(
            "SELECT id, preview, cwd, title, created_at, updated_at FROM threads WHERE archived = 0 ORDER BY updated_at DESC"
        )?;
        let rows = stmt.query_map([], |row| {
            let id: String = row.get(0)?;
            let preview: String = row.get(1)?;
            let cwd: String = row.get(2)?;
            let title: Option<String> = row.get(3)?;
            let _created: i64 = row.get(4)?;
            let updated: i64 = row.get(5)?;
            Ok(ProviderSessionSummary {
                session_id: id.clone(),
                title: title.or_else(|| {
                    let p = preview.trim();
                    if p.is_empty() {
                        None
                    } else {
                        Some(p.to_string())
                    }
                }),
                project_dir: Some(cwd),
                last_active_at: Some(updated),
                source_path: Some(id),
            })
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            if let Ok(s) = row {
                sessions.push(s);
            }
        }
        Ok(sessions)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let db_path = get_state_db_path();
        let conn = Connection::open(&db_path).with_context(|| {
            format!("failed to open DeepSeek state db at {}", db_path.display())
        })?;
        import_canonical_session_from_connection(&conn, source_path)
    }

    fn export_session(
        &self,
        session: &CanonicalSession,
        target_dir: &Path,
    ) -> Result<ExportedSession> {
        let session_id = export_canonical_session(session, target_dir)?;
        Ok(canonical_export_result(
            PROVIDER_ID,
            session_id.clone(),
            self.resume_command(&session_id),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(());
        }
        let conn = Connection::open(&db_path)?;
        conn.execute("DELETE FROM messages WHERE thread_id = ?1", [session_id])?;
        conn.execute("DELETE FROM checkpoints WHERE thread_id = ?1", [session_id])?;
        conn.execute(
            "DELETE FROM thread_dynamic_tools WHERE thread_id = ?1",
            [session_id],
        )?;
        conn.execute("DELETE FROM threads WHERE id = ?1", [session_id])?;
        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(());
        }
        let conn = Connection::open(&db_path)?;
        let now = Utc::now().timestamp();
        conn.execute(
            "UPDATE threads SET title = ?1, preview = ?1, updated_at = ?2 WHERE id = ?3",
            [new_title, &now.to_string(), session_id],
        )?;

        // Also update session_index.jsonl
        append_session_index(session_id, Some(new_title), now, None)?;
        Ok(())
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("deepseek resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(0);
        }
        let conn = Connection::open(&db_path)?;
        deepseek_session_size_with_conn(&conn, session_id)
    }

    fn session_sizes(&self, session_ids: &[&str]) -> HashMap<String, u64> {
        let db_path = get_state_db_path();
        let Ok(conn) = Connection::open(&db_path) else {
            return HashMap::new();
        };
        session_ids
            .iter()
            .filter_map(|session_id| {
                deepseek_session_size_with_conn(&conn, session_id)
                    .ok()
                    .filter(|size| *size > 0)
                    .map(|size| ((*session_id).to_string(), size))
            })
            .collect()
    }
}

fn deepseek_session_size_with_conn(conn: &Connection, session_id: &str) -> Result<u64> {
    let mut total: u64 = 0;

    if let Ok(size) = conn.query_row(
        "SELECT COALESCE(length(id), 0) + COALESCE(length(preview), 0) + COALESCE(length(cwd), 0) + COALESCE(length(title), 0) + COALESCE(length(status), 0) + COALESCE(length(model_provider), 0) + COALESCE(length(cli_version), 0) + COALESCE(length(source), 0) FROM threads WHERE id = ?1",
        [session_id],
        |row| row.get::<_, i64>(0),
    ) {
        total += size as u64;
    }

    let mut stmt = conn.prepare("SELECT COALESCE(length(role), 0) + COALESCE(length(content), 0) + COALESCE(length(item_json), 0) FROM messages WHERE thread_id = ?1")?;
    let rows = stmt.query_map([session_id], |row| row.get::<_, i64>(0))?;
    for row in rows.flatten() {
        total += row as u64;
    }

    Ok(total)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn get_state_db_path() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".deepseek").join("state.db"))
        .unwrap_or_else(|| PathBuf::from(".deepseek").join("state.db"))
}

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    let db_path = get_state_db_path();
    if !db_path.exists() {
        anyhow::bail!(
            "DeepSeek state database does not exist at {}. Please launch DeepSeek TUI once to initialize storage before importing.",
            db_path.display()
        );
    }
    let mut conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open DeepSeek state db at {}", db_path.display()))?;

    let thread_id = format!("thread-{}", Uuid::new_v4());
    let now = Utc::now().timestamp();
    let cwd = target_dir.to_string_lossy().to_string();
    let title = canonical_session_title(session);
    let tx = conn.transaction()?;

    tx.execute(
        "INSERT INTO threads (id, preview, ephemeral, model_provider, created_at, updated_at, status, cwd, cli_version, source, title, archived) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        [
            &thread_id,
            &title,
            "0",
            "deepseek",
            &now.to_string(),
            &now.to_string(),
            "idle",
            &cwd,
            env!("CARGO_PKG_VERSION"),
            "interactive",
            &title,
            "0",
        ],
    )
    .context("failed to insert thread")?;

    for event in &session.events {
        let content = canonical_event_text(event);
        if content.trim().is_empty() {
            continue;
        }
        let role = match event.role {
            EventRole::Assistant => "assistant",
            EventRole::Tool => "tool",
            EventRole::System => "system",
            EventRole::Developer => "developer",
            EventRole::User | EventRole::Unknown => "user",
        };
        let item_json = serde_json::json!({
            "source": "memorph-canonical",
            "event_id": event.id,
            "event_kind": event.kind,
            "event_role": canonical_event_role_label(event.role),
            "blocks": event.blocks,
        });
        let created_at = event.timestamp.timestamp();
        tx.execute(
            "INSERT INTO messages (thread_id, role, content, item_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            [
                &thread_id,
                role,
                &content,
                &serde_json::to_string(&item_json)?,
                &created_at.to_string(),
            ],
        )
        .context("failed to insert message")?;
    }

    tx.commit()?;
    append_session_index(&thread_id, Some(&title), now, None)?;
    Ok(thread_id)
}

#[derive(Debug)]
struct ThreadRow {
    id: String,
    preview: String,
    cwd: String,
    title: Option<String>,
    created_at: i64,
    updated_at: i64,
    model_provider: String,
}

#[derive(Debug)]
struct MessageRow {
    id: i64,
    role: String,
    content: String,
    item_json: Option<String>,
    created_at: i64,
}

fn import_canonical_session_from_connection(
    conn: &Connection,
    source_path: &str,
) -> Result<ImportedSession> {
    let thread = load_thread_row(conn, source_path)?;
    let messages = load_message_rows(conn, source_path)?;
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();

    for message in messages {
        let raw_message = deepseek_message_value(&message);
        let timestamp =
            chrono::DateTime::from_timestamp(message.created_at, 0).unwrap_or_else(Utc::now);
        let role = deepseek_event_role(&message.role, &raw_message, &mut report);
        let (blocks, fidelity) = canonical_blocks_from_message(&message, &raw_message, &mut report);

        events.push(SessionEvent {
            id: format!("deepseek:message:{}", message.id),
            kind: deepseek_event_kind(&message.role, &blocks),
            role,
            timestamp,
            links: EventLinks::default(),
            blocks,
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: PROVIDER_ID.to_string(),
                    original_id: Some(message.id.to_string()),
                    original_role: Some(message.role.clone()),
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity,
                provider_ext: {
                    let mut ext = BTreeMap::new();
                    ext.insert("deepseek_message".to_string(), raw_message);
                    if !thread.model_provider.trim().is_empty() {
                        ext.insert(
                            "model_provider".to_string(),
                            Value::String(thread.model_provider.clone()),
                        );
                    }
                    ext
                },
            },
        });
    }

    let source_title = deepseek_thread_title(&thread);
    let mut extensions = BTreeMap::new();
    extensions.insert(
        "deepseek_thread".to_string(),
        deepseek_thread_value(&thread),
    );

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: thread.id.clone(),
                source_title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: thread.id.clone(),
                    source_path: Some(source_path.to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: Some(thread.cwd.clone()),
                created_at: chrono::DateTime::from_timestamp(thread.created_at, 0),
                last_active_at: chrono::DateTime::from_timestamp(thread.updated_at, 0),
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn load_thread_row(conn: &Connection, source_path: &str) -> Result<ThreadRow> {
    let thread = conn
        .query_row(
            "SELECT id, preview, cwd, title, created_at, updated_at, model_provider FROM threads WHERE id = ?1",
            [source_path],
            |row| {
                Ok(ThreadRow {
                    id: row.get(0)?,
                    preview: row.get(1)?,
                    cwd: row.get(2)?,
                    title: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    model_provider: row.get(6)?,
                })
            },
        )
        .optional()
        .context("failed to read thread")?;

    thread.with_context(|| format!("thread not found: {}", source_path))
}

fn load_message_rows(conn: &Connection, source_path: &str) -> Result<Vec<MessageRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, role, content, item_json, created_at FROM messages WHERE thread_id = ?1 ORDER BY created_at ASC"
    )?;
    let rows = stmt.query_map([source_path], |row| {
        Ok(MessageRow {
            id: row.get(0)?,
            role: row.get(1)?,
            content: row.get(2)?,
            item_json: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;

    let mut messages = Vec::new();
    for row in rows {
        messages.push(row?);
    }
    Ok(messages)
}

fn deepseek_thread_title(thread: &ThreadRow) -> Option<String> {
    thread.title.clone().or_else(|| {
        let preview = thread.preview.trim();
        if preview.is_empty() {
            None
        } else {
            Some(preview.to_string())
        }
    })
}

fn deepseek_event_role(role: &str, raw_message: &Value, report: &mut MappingReport) -> EventRole {
    match role {
        "user" => EventRole::User,
        "assistant" => EventRole::Assistant,
        "tool" => EventRole::Tool,
        "system" => EventRole::System,
        "developer" => EventRole::Developer,
        "history" => EventRole::System,
        other => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Normalized,
                code: "unknown_role_normalized".to_string(),
                message: format!("Normalized unknown DeepSeek role '{}'", other),
                path: None,
                raw: Some(raw_message.clone()),
            });
            EventRole::Unknown
        }
    }
}

fn canonical_blocks_from_message(
    message: &MessageRow,
    raw_message: &Value,
    report: &mut MappingReport,
) -> (Vec<EventBlock>, MappingDisposition) {
    let mut blocks = Vec::new();
    let mut fidelity = MappingDisposition::Preserved;
    let content = message.content.trim();

    match message.item_json.as_deref() {
        Some(raw_item) => match serde_json::from_str::<Value>(raw_item) {
            Ok(item) => {
                if message.role == "history" && !content.is_empty() {
                    blocks.push(EventBlock::Text {
                        text: message.content.clone(),
                    });
                }

                if let Some(tool_name) = item.get("tool_name").and_then(|value| value.as_str()) {
                    blocks.push(EventBlock::ToolCall {
                        tool_call_id: item
                            .get("call_id")
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| message.id.to_string()),
                        name: tool_name.to_string(),
                        input: item.get("arguments").cloned(),
                    });
                    if !content.is_empty() {
                        blocks.push(EventBlock::Text {
                            text: message.content.clone(),
                        });
                    }
                } else if let Some(output) = item.get("output").and_then(|value| value.as_str()) {
                    blocks.push(EventBlock::ToolResult {
                        tool_call_id: item
                            .get("tool_use_id")
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| message.id.to_string()),
                        content: output.to_string(),
                        is_error: item
                            .get("is_error")
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false),
                    });
                    if !content.is_empty() && content != output {
                        blocks.push(EventBlock::Text {
                            text: message.content.clone(),
                        });
                    }
                } else if !content.is_empty() {
                    blocks.push(EventBlock::Text {
                        text: message.content.clone(),
                    });
                }

                blocks.push(EventBlock::ProviderPayload {
                    kind: "message_item".to_string(),
                    payload: item,
                });
            }
            Err(error) => {
                fidelity = MappingDisposition::Normalized;
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Normalized,
                    code: "invalid_item_json".to_string(),
                    message: format!("Failed to parse DeepSeek item_json: {}", error),
                    path: Some(format!("message:{}", message.id)),
                    raw: Some(raw_message.clone()),
                });
                if !content.is_empty() {
                    blocks.push(EventBlock::Text {
                        text: message.content.clone(),
                    });
                }
                blocks.push(EventBlock::ProviderPayload {
                    kind: "message_item_raw".to_string(),
                    payload: Value::String(raw_item.to_string()),
                });
            }
        },
        None if !content.is_empty() => {
            blocks.push(EventBlock::Text {
                text: message.content.clone(),
            });
        }
        None => {}
    }

    if blocks.is_empty() {
        fidelity = MappingDisposition::Normalized;
        blocks.push(EventBlock::Unknown {
            raw: raw_message.clone(),
        });
    }

    (blocks, fidelity)
}

fn deepseek_event_kind(role: &str, blocks: &[EventBlock]) -> SessionEventKind {
    if role == "history" {
        SessionEventKind::Lifecycle
    } else if blocks
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

fn deepseek_thread_value(thread: &ThreadRow) -> Value {
    serde_json::json!({
        "id": thread.id,
        "preview": thread.preview,
        "cwd": thread.cwd,
        "title": thread.title,
        "created_at": thread.created_at,
        "updated_at": thread.updated_at,
        "model_provider": thread.model_provider,
    })
}

fn deepseek_message_value(message: &MessageRow) -> Value {
    serde_json::json!({
        "id": message.id,
        "role": message.role,
        "content": message.content,
        "item_json": message.item_json,
        "created_at": message.created_at,
    })
}

fn append_session_index(
    thread_id: &str,
    thread_name: Option<&str>,
    updated_at: i64,
    rollout_path: Option<&Path>,
) -> Result<()> {
    let index_path = dirs::home_dir()
        .map(|h| h.join(".deepseek").join("session_index.jsonl"))
        .unwrap_or_else(|| PathBuf::from(".deepseek").join("session_index.jsonl"));

    if let Some(parent) = index_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let entry = serde_json::json!({
        "thread_id": thread_id,
        "thread_name": thread_name,
        "updated_at": updated_at,
        "rollout_path": rollout_path.map(|p| p.to_string_lossy().to_string()),
    });

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&index_path)?;
    use std::io::Write;
    writeln!(file, "{}", entry)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_canonical_session_preserves_workspace_tool_payloads_and_history() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let conn = Connection::open(&db_path).unwrap();

        conn.execute_batch(
            "
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                preview TEXT NOT NULL,
                cwd TEXT NOT NULL,
                title TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                model_provider TEXT NOT NULL,
                archived INTEGER DEFAULT 0
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY,
                thread_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                item_json TEXT,
                created_at INTEGER NOT NULL
            );
            ",
        )
        .unwrap();

        conn.execute(
            "INSERT INTO threads (id, preview, cwd, title, created_at, updated_at, model_provider, archived) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)",
            (
                "thread-1",
                "preview title",
                "/tmp/workspace",
                "Named Thread",
                1710000000_i64,
                1710000100_i64,
                "deepseek",
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, thread_id, role, content, item_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                1_i64,
                "thread-1",
                "history",
                "forked from another thread",
                "{\"kind\":\"fork\"}",
                1710000001_i64,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, thread_id, role, content, item_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                2_i64,
                "thread-1",
                "assistant",
                "",
                "{\"tool_name\":\"read_file\",\"call_id\":\"call-1\",\"arguments\":{\"path\":\"Cargo.toml\"}}",
                1710000002_i64,
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (id, thread_id, role, content, item_json, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            (
                3_i64,
                "thread-1",
                "tool",
                "file contents",
                "{\"output\":\"file contents\",\"tool_use_id\":\"call-1\",\"is_error\":false}",
                1710000003_i64,
            ),
        )
        .unwrap();

        let imported = import_canonical_session_from_connection(&conn, "thread-1").unwrap();

        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some("/tmp/workspace")
        );
        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Named Thread")
        );
        assert_eq!(imported.session.events.len(), 3);
        assert_eq!(imported.session.events[0].kind, SessionEventKind::Lifecycle);
        assert!(matches!(
            imported.session.events[1].blocks.first(),
            Some(EventBlock::ToolCall {
                name,
                tool_call_id,
                ..
            }) if name == "read_file" && tool_call_id == "call-1"
        ));
        assert!(matches!(
            imported.session.events[2].blocks.first(),
            Some(EventBlock::ToolResult {
                tool_call_id,
                content,
                is_error
            }) if tool_call_id == "call-1" && content == "file contents" && !is_error
        ));
        assert!(imported.session.extensions.contains_key("deepseek_thread"));
    }
}
