use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::provider::{
    canonical_event_role_label, canonical_event_visible_message_role,
    canonical_event_visible_message_text, canonical_export_result, canonical_session_title,
    Provider, ProviderBackupSupport, ProviderCapabilities, ProviderSessionBackup,
    ProviderSessionSummary, ProviderSourceMutation,
};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use uuid::Uuid;

mod backup;

pub struct DeepseekProvider;

const PROVIDER_ID: &str = "deepseek";

#[cfg(test)]
static TEST_DEEPSEEK_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_DEEPSEEK_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

impl Provider for DeepseekProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "DeepSeek"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
            },
            ..ProviderCapabilities::full_session_management()
        }
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

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(None);
        }
        let conn = Connection::open(&db_path).with_context(|| {
            format!("failed to open DeepSeek state db at {}", db_path.display())
        })?;

        let meta = conn
            .query_row(
                "SELECT id, preview, cwd, title, updated_at FROM threads WHERE id = ?1 AND archived = 0",
                [session_id],
                |row| {
                    let id: String = row.get(0)?;
                    let preview: String = row.get(1)?;
                    let cwd: String = row.get(2)?;
                    let title: Option<String> = row.get(3)?;
                    let updated: i64 = row.get(4)?;
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
                },
            )
            .optional()?;

        Ok(meta)
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
            session,
            self.capabilities(),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(());
        }
        let mut conn = Connection::open(&db_path)?;
        backup::validate_mutation_source(&conn, ProviderSourceMutation::Delete, session_id)?;
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM messages WHERE thread_id = ?1", [session_id])?;
        tx.execute("DELETE FROM checkpoints WHERE thread_id = ?1", [session_id])?;
        tx.execute(
            "DELETE FROM thread_dynamic_tools WHERE thread_id = ?1",
            [session_id],
        )?;
        let deleted = tx.execute("DELETE FROM threads WHERE id = ?1", [session_id])?;
        if deleted != 1 {
            anyhow::bail!("DeepSeek thread not found: {session_id}");
        }
        tx.commit()?;
        fail_deepseek_mutation_after_write(ProviderSourceMutation::Delete)?;
        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let db_path = get_state_db_path();
        if !db_path.exists() {
            return Ok(());
        }
        let mut conn = Connection::open(&db_path)?;
        backup::validate_mutation_source(&conn, ProviderSourceMutation::Rename, session_id)?;
        let now = Utc::now().timestamp();
        let tx = conn.transaction()?;
        let updated = tx.execute(
            "UPDATE threads SET title = ?1, preview = ?1, updated_at = ?2 WHERE id = ?3",
            [new_title, &now.to_string(), session_id],
        )?;
        if updated != 1 {
            anyhow::bail!("DeepSeek thread not found: {session_id}");
        }
        tx.commit()?;

        append_session_index(session_id, Some(new_title), now, None)?;
        fail_deepseek_mutation_after_write(ProviderSourceMutation::Rename)?;
        Ok(())
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        backup::create_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        backup::restore_session_backup(backup)
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

    fn data_source_paths(&self) -> Vec<PathBuf> {
        get_state_db_path()
            .parent()
            .map(PathBuf::from)
            .into_iter()
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

fn get_deepseek_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_DEEPSEEK_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test DeepSeek dir lock")
        .clone()
    {
        return path;
    }

    dirs::home_dir()
        .map(|h| h.join(".deepseek"))
        .unwrap_or_else(|| PathBuf::from(".deepseek"))
}

fn get_state_db_path() -> PathBuf {
    get_deepseek_dir().join("state.db")
}

fn get_session_index_path() -> PathBuf {
    get_deepseek_dir().join("session_index.jsonl")
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
        let Some(visible_role) = canonical_event_visible_message_role(event) else {
            continue;
        };
        let Some(content) = deepseek_message_content(event) else {
            continue;
        };
        let role = match visible_role {
            EventRole::Assistant => "assistant",
            EventRole::Tool => "tool",
            EventRole::User => "user",
            EventRole::System | EventRole::Developer | EventRole::Unknown => continue,
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

fn deepseek_message_content(event: &SessionEvent) -> Option<String> {
    canonical_event_visible_message_text(event)
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
    let index_path = get_session_index_path();

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
fn set_test_deepseek_dir(path: Option<PathBuf>) {
    *TEST_DEEPSEEK_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test DeepSeek dir lock") = path;
}

#[cfg(test)]
fn set_test_deepseek_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_DEEPSEEK_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test DeepSeek mutation failure lock") = mutation;
}

#[cfg(test)]
fn fail_deepseek_mutation_after_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_DEEPSEEK_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test DeepSeek mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected DeepSeek mutation failure after provider write");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_deepseek_mutation_after_write(_mutation: ProviderSourceMutation) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{core::session_management, storage::local_store};
    use rusqlite::params;
    use rusqlite::types::Value as SqliteValue;
    use serde_json::json;
    use tempfile::tempdir;

    static TEST_DEEPSEEK_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    struct TestDeepseekDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestDeepseekDirGuard {
        fn drop(&mut self) {
            crate::cache::global_cache().invalidate(PROVIDER_ID);
            backup::set_test_backup_failure(false);
            set_test_deepseek_mutation_failure(None);
            set_test_deepseek_dir(None);
        }
    }

    fn use_test_deepseek_dir(path: PathBuf) -> TestDeepseekDirGuard {
        let lock = TEST_DEEPSEEK_TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        set_test_deepseek_dir(Some(path));
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        TestDeepseekDirGuard { _lock: lock }
    }

    struct NativeDeepseekFixture {
        index_path: PathBuf,
        original_index_bytes: Vec<u8>,
    }

    fn write_native_deepseek_fixture(
        deepseek_dir: &Path,
        session_id: &str,
    ) -> NativeDeepseekFixture {
        std::fs::create_dir_all(deepseek_dir).unwrap();
        let db_path = deepseek_dir.join("state.db");
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
                archived INTEGER NOT NULL DEFAULT 0,
                unrelated_note TEXT NOT NULL,
                native_blob BLOB
            );
            CREATE TABLE messages (
                id INTEGER PRIMARY KEY,
                thread_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                item_json TEXT,
                created_at INTEGER NOT NULL,
                native_blob BLOB,
                FOREIGN KEY (thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );
            CREATE TABLE checkpoints (
                id TEXT PRIMARY KEY,
                thread_id TEXT NOT NULL,
                payload BLOB,
                note TEXT NOT NULL,
                FOREIGN KEY (thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );
            CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                tool_name TEXT NOT NULL,
                payload BLOB,
                PRIMARY KEY (thread_id, position),
                FOREIGN KEY (thread_id) REFERENCES threads(id) ON DELETE CASCADE
            );
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (
                id, preview, cwd, title, created_at, updated_at, model_provider,
                archived, unrelated_note, native_blob
             ) VALUES (?1, 'Before preview', '/tmp/deepseek', 'Before', 100, 200,
                'deepseek', 0, 'target note', ?2)",
            params![session_id, vec![0_u8, 1, 127, 128, 255]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (
                id, preview, cwd, title, created_at, updated_at, model_provider,
                archived, unrelated_note, native_blob
             ) VALUES ('thread-other', 'Other preview', '/tmp/other', 'Other', 300,
                400, 'deepseek', 0, 'other note', X'1020')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (
                id, thread_id, role, content, item_json, created_at, native_blob
             ) VALUES (1, ?1, 'user', 'hello', '{\"kind\":\"message\"}', 101, ?2)",
            params![session_id, vec![255_u8, 0, 128]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (
                id, thread_id, role, content, item_json, created_at, native_blob
             ) VALUES (2, 'thread-other', 'user', 'other', NULL, 301, X'22')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO checkpoints (id, thread_id, payload, note)
             VALUES ('checkpoint-1', ?1, ?2, 'target checkpoint')",
            params![session_id, vec![1_u8, 2, 3, 250]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO checkpoints (id, thread_id, payload, note)
             VALUES ('checkpoint-other', 'thread-other', X'33', 'other checkpoint')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO thread_dynamic_tools (thread_id, position, tool_name, payload)
             VALUES (?1, 0, 'shell', ?2)",
            params![session_id, vec![4_u8, 5, 200]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO thread_dynamic_tools (thread_id, position, tool_name, payload)
             VALUES ('thread-other', 0, 'read', X'44')",
            [],
        )
        .unwrap();

        let index_path = deepseek_dir.join("session_index.jsonl");
        let original_index_bytes = [
            serde_json::to_string(&json!({
                "thread_id": session_id,
                "thread_name": "Before",
                "updated_at": 200,
                "rollout_path": null
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "thread_id": "thread-other",
                "thread_name": "Other",
                "updated_at": 400,
                "rollout_path": null
            }))
            .unwrap(),
        ]
        .join("\n")
        .into_bytes();
        let mut original_index_bytes = original_index_bytes;
        original_index_bytes.push(b'\n');
        std::fs::write(&index_path, &original_index_bytes).unwrap();

        NativeDeepseekFixture {
            index_path,
            original_index_bytes,
        }
    }

    fn deepseek_session_row_counts(deepseek_dir: &Path, session_id: &str) -> Vec<i64> {
        let conn = Connection::open(deepseek_dir.join("state.db")).unwrap();
        [
            ("threads", "id"),
            ("messages", "thread_id"),
            ("checkpoints", "thread_id"),
            ("thread_dynamic_tools", "thread_id"),
        ]
        .into_iter()
        .map(|(table, column)| {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?1"),
                [session_id],
                |row| row.get(0),
            )
            .unwrap()
        })
        .collect()
    }

    fn target_thread_values(
        deepseek_dir: &Path,
        session_id: &str,
    ) -> Option<(String, String, i64, String, SqliteValue)> {
        Connection::open(deepseek_dir.join("state.db"))
            .unwrap()
            .query_row(
                "SELECT title, preview, updated_at, unrelated_note, native_blob
                 FROM threads WHERE id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()
            .unwrap()
    }

    #[test]
    fn delete_backup_restores_exact_deepseek_rows_and_preserves_unrelated_rows() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-delete-backup";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let original_thread = target_thread_values(&get_deepseek_dir(), session_id).unwrap();
        let backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-delete-1",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        DeepseekProvider.delete_session(session_id).unwrap();
        assert_eq!(
            deepseek_session_row_counts(&get_deepseek_dir(), session_id),
            vec![0, 0, 0, 0]
        );
        let conn = Connection::open(get_state_db_path()).unwrap();
        conn.execute(
            "UPDATE threads
             SET unrelated_note = 'other changed', native_blob = X'AABB'
             WHERE id = 'thread-other'",
            [],
        )
        .unwrap();
        drop(conn);

        DeepseekProvider.restore_session_backup(&backup).unwrap();
        DeepseekProvider.restore_session_backup(&backup).unwrap();

        assert_eq!(
            deepseek_session_row_counts(&get_deepseek_dir(), session_id),
            vec![1, 1, 1, 1]
        );
        assert_eq!(
            target_thread_values(&get_deepseek_dir(), session_id).as_ref(),
            Some(&original_thread)
        );
        let conn = Connection::open(get_state_db_path()).unwrap();
        let other: (String, Vec<u8>) = conn
            .query_row(
                "SELECT unrelated_note, native_blob FROM threads WHERE id = 'thread-other'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(other, ("other changed".to_string(), vec![0xAA, 0xBB]));
        let message_blob: Vec<u8> = conn
            .query_row(
                "SELECT native_blob FROM messages WHERE thread_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(message_blob, vec![255_u8, 0, 128]);
    }

    #[test]
    fn rename_backup_restores_owned_fields_and_removes_only_target_index_append() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-rename-backup";
        let fixture = write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-rename-1",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        DeepseekProvider
            .rename_session(session_id, "After")
            .unwrap();
        let conn = Connection::open(get_state_db_path()).unwrap();
        conn.execute(
            "UPDATE threads
             SET cwd = '/tmp/changed', unrelated_note = 'changed independently',
                 native_blob = X'FEED'
             WHERE id = ?1",
            [session_id],
        )
        .unwrap();
        drop(conn);
        append_session_index("thread-concurrent", Some("Concurrent"), 999, None).unwrap();

        DeepseekProvider.restore_session_backup(&backup).unwrap();
        DeepseekProvider.restore_session_backup(&backup).unwrap();

        let conn = Connection::open(get_state_db_path()).unwrap();
        let thread: (String, String, i64, String, String, Vec<u8>) = conn
            .query_row(
                "SELECT title, preview, updated_at, cwd, unrelated_note, native_blob
                 FROM threads WHERE id = ?1",
                [session_id],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            thread,
            (
                "Before".to_string(),
                "Before preview".to_string(),
                200,
                "/tmp/changed".to_string(),
                "changed independently".to_string(),
                vec![0xFE, 0xED],
            )
        );
        let current_index = std::fs::read(&fixture.index_path).unwrap();
        assert!(current_index.starts_with(&fixture.original_index_bytes));
        let suffix = &current_index[fixture.original_index_bytes.len()..];
        assert_eq!(
            std::str::from_utf8(suffix)
                .unwrap()
                .lines()
                .map(
                    |line| serde_json::from_str::<Value>(line).unwrap()["thread_id"]
                        .as_str()
                        .unwrap()
                        .to_string()
                )
                .collect::<Vec<_>>(),
            vec!["thread-concurrent".to_string()]
        );
    }

    #[test]
    fn rename_restore_does_not_recreate_concurrently_deleted_thread() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-concurrent-delete";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-concurrent-delete",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        DeepseekProvider
            .rename_session(session_id, "After")
            .unwrap();
        Connection::open(get_state_db_path())
            .unwrap()
            .execute("DELETE FROM threads WHERE id = ?1", [session_id])
            .unwrap();

        DeepseekProvider.restore_session_backup(&backup).unwrap();

        assert!(target_thread_values(&get_deepseek_dir(), session_id).is_none());
    }

    #[test]
    fn deepseek_backup_contract_and_capabilities_are_truthful() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-backup-contract";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-contract-1",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        let capabilities = DeepseekProvider.capabilities();
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert_eq!(backup.mutation, ProviderSourceMutation::Rename);
        assert_eq!(backup.operation_id, "operation-contract-1");
        assert_eq!(backup.provider_session_id, session_id);
        assert_eq!(
            backup.source_path,
            get_state_db_path().canonicalize().unwrap()
        );
        assert_eq!(backup.format, "deepseek-session-backup-v1");
        assert_eq!(
            backup.mime_type,
            "application/vnd.memorph.deepseek-session-backup"
        );
        assert_eq!(
            backup
                .restore_metadata
                .get("restore_mode")
                .and_then(Value::as_str),
            Some("deepseek_session_restore")
        );
        assert!(backup.backup_path.join("metadata.json").is_file());
        assert!(backup
            .backup_path
            .join("sqlite/deepseek-session.db")
            .is_file());
        assert!(backup
            .backup_path
            .join("files/session_index.jsonl")
            .is_file());
    }

    #[test]
    fn backup_registration_failure_prevents_deepseek_provider_write() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-registration-failure";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let backup_root = dir.path().join("backups");
        let mut artifact_conn = Connection::open_in_memory().unwrap();

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-registration-failure".to_string()],
            &backup_root,
            &mut artifact_conn,
        );

        let error = results.into_iter().next().unwrap().unwrap_err();
        assert!(error
            .to_string()
            .contains("Delete cancelled before provider write"));
        assert_eq!(
            deepseek_session_row_counts(&get_deepseek_dir(), session_id),
            vec![1, 1, 1, 1]
        );
        assert!(backup_root
            .join(PROVIDER_ID)
            .join("operation-registration-failure")
            .exists());
    }

    #[test]
    fn partial_deepseek_delete_failure_restores_registered_backup() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-partial-delete";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let mut artifact_conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();
        set_test_deepseek_mutation_failure(Some(ProviderSourceMutation::Delete));

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-partial-delete".to_string()],
            &dir.path().join("backups"),
            &mut artifact_conn,
        );

        let error = results.into_iter().next().unwrap().unwrap_err();
        assert!(error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            deepseek_session_row_counts(&get_deepseek_dir(), session_id),
            vec![1, 1, 1, 1]
        );
    }

    #[test]
    fn partial_deepseek_rename_failure_restores_registered_backup() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-partial-rename";
        let fixture = write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let mut artifact_conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();
        set_test_deepseek_mutation_failure(Some(ProviderSourceMutation::Rename));

        let error = session_management::rename_session(
            PROVIDER_ID,
            session_id,
            "After",
            "operation-partial-rename",
            &dir.path().join("backups"),
            &mut artifact_conn,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            target_thread_values(&get_deepseek_dir(), session_id)
                .unwrap()
                .0,
            "Before"
        );
        assert_eq!(
            std::fs::read(&fixture.index_path).unwrap(),
            fixture.original_index_bytes
        );
    }

    #[test]
    fn deepseek_backup_rejects_unsafe_schema_before_mutation() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        std::fs::create_dir_all(get_deepseek_dir()).unwrap();
        let conn = Connection::open(get_state_db_path()).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE threads (
                id TEXT,
                title TEXT,
                preview TEXT,
                updated_at INTEGER
            );
            CREATE TABLE messages (thread_id TEXT);
            CREATE TABLE checkpoints (thread_id TEXT);
            CREATE TABLE thread_dynamic_tools (thread_id TEXT);
            INSERT INTO threads (id, title, preview, updated_at)
            VALUES ('unsafe-thread', 'Before', 'Before', 1);
            ",
        )
        .unwrap();
        drop(conn);

        let error = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-unsafe-schema",
                "unsafe-thread",
                &dir.path().join("backups"),
            )
            .unwrap_err();

        assert!(error.to_string().contains("does not enforce a unique id"));
        assert!(!dir
            .path()
            .join("backups/deepseek/operation-unsafe-schema")
            .exists());
        assert_eq!(
            Connection::open(get_state_db_path())
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM threads WHERE id = 'unsafe-thread'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );

        std::fs::remove_file(get_state_db_path()).unwrap();
        let conn = Connection::open(get_state_db_path()).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                title TEXT,
                preview TEXT,
                updated_at INTEGER
            );
            CREATE TABLE messages (thread_id TEXT);
            CREATE TABLE thread_dynamic_tools (thread_id TEXT);
            INSERT INTO threads (id, title, preview, updated_at)
            VALUES ('missing-table-thread', 'Before', 'Before', 1);
            ",
        )
        .unwrap();
        drop(conn);
        let error = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-missing-table",
                "missing-table-thread",
                &dir.path().join("backups"),
            )
            .unwrap_err();
        assert!(error.to_string().contains("table does not exist"));
        assert!(!dir
            .path()
            .join("backups/deepseek/operation-missing-table")
            .exists());
    }

    #[test]
    fn deepseek_restore_rejects_manifest_selection_and_schema_tampering() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-tampering";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let backup_root = dir.path().join("backups");

        let manifest_backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-manifest-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        let metadata_path = manifest_backup.backup_path.join("metadata.json");
        let mut metadata: Value =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        metadata["sqlite_tables"][0]["row_count"] = json!(99);
        std::fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
        assert!(DeepseekProvider
            .restore_session_backup(&manifest_backup)
            .unwrap_err()
            .to_string()
            .contains("row count"));

        let selection_backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-selection-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        Connection::open(
            selection_backup
                .backup_path
                .join("sqlite/deepseek-session.db"),
        )
        .unwrap()
        .execute("UPDATE threads SET id = 'thread-outside-selection'", [])
        .unwrap();
        assert!(DeepseekProvider
            .restore_session_backup(&selection_backup)
            .unwrap_err()
            .to_string()
            .contains("outside the target session"));

        let schema_backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-schema-tamper",
                session_id,
                &backup_root,
            )
            .unwrap();
        Connection::open(get_state_db_path())
            .unwrap()
            .execute("ALTER TABLE messages ADD COLUMN schema_drift TEXT", [])
            .unwrap();
        assert!(DeepseekProvider
            .restore_session_backup(&schema_backup)
            .unwrap_err()
            .to_string()
            .contains("schema does not match"));
    }

    #[test]
    fn deepseek_index_restore_rejects_ambiguous_target_appends() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-ambiguous-index";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-ambiguous-index",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        DeepseekProvider
            .rename_session(session_id, "After")
            .unwrap();
        append_session_index(session_id, Some("Concurrent target"), 999, None).unwrap();

        let error = DeepseekProvider
            .restore_session_backup(&backup)
            .unwrap_err();

        assert!(
            error.to_string().contains("multiple target records"),
            "{error:#}"
        );
    }

    #[test]
    fn rename_restore_removes_new_index_file_when_target_append_is_only_content() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-index-absent";
        let fixture = write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        std::fs::remove_file(&fixture.index_path).unwrap();
        let backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-index-absent",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        DeepseekProvider
            .rename_session(session_id, "After")
            .unwrap();

        DeepseekProvider.restore_session_backup(&backup).unwrap();
        DeepseekProvider.restore_session_backup(&backup).unwrap();

        assert!(!fixture.index_path.exists());
    }

    #[test]
    fn failed_deepseek_backup_creation_removes_operation_directory() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-backup-cleanup";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let backup_root = dir.path().join("backups");
        backup::set_test_backup_failure(true);

        let error = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-backup-cleanup",
                session_id,
                &backup_root,
            )
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("injected DeepSeek backup failure"));
        assert!(!backup_root
            .join(PROVIDER_ID)
            .join("operation-backup-cleanup")
            .exists());
    }

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

    #[test]
    fn compressed_segment_exports_as_portable_deepseek_message_content() {
        let event = SessionEvent {
            id: "compressed-source".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Compressed {
                source_provider_id: "opencode".to_string(),
                summary: "compressed summary".to_string(),
                source_event_ids: vec![
                    "old-event-1".to_string(),
                    "old-event-2".to_string(),
                    "old-event-3".to_string(),
                ],
                source_event_count: None,
                archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "memorph".to_string(),
                    original_id: None,
                    original_role: Some("assistant".to_string()),
                    phase: Some("compression".to_string()),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        let content = deepseek_message_content(&event).expect("visible compressed content");

        assert!(content.contains("[Compressed session segment from opencode]"));
        assert!(content.contains("compressed summary"));
        assert!(content.contains("Source event count: 3"));
        assert!(content.contains("Archive: memorph-archive://s1/archive.json.gz"));
        assert!(content.contains("memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"));
        assert!(!content.contains("old-event-1"));
        assert!(!content.contains("old-event-2"));
        assert!(!content.contains("old-event-3"));
    }

    #[test]
    fn internal_events_do_not_export_as_deepseek_message_content() {
        let event = SessionEvent {
            id: "internal".to_string(),
            kind: SessionEventKind::Lifecycle,
            role: EventRole::System,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: "internal context".to_string(),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "codex".to_string(),
                    original_id: None,
                    original_role: Some("user".to_string()),
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        assert!(deepseek_message_content(&event).is_none());
    }
}
