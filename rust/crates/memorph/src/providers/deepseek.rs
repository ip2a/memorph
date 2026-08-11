use crate::provider::{
    event_role_label, event_visible_message_role, event_visible_message_text, export_result,
    session_title, PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport,
    ProviderCapabilities, ProviderContentFidelity, ProviderSessionBackup, ProviderSessionSummary,
    ProviderSourceFingerprint, ProviderSourceMutation, ProviderWriteRisk, ResumeQuality,
    ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::session::{
    Block, Context, Event, EventKind, EventMeta, EventSource, ExportedSession, Fidelity, Identity,
    ImportedSession, Links, MappingDirection, MappingIssue, MappingIssueLevel, MappingReport,
    Metadata, Provenance, ProviderRef, Role, Schema, Session,
};
use anyhow::{Context as _, Result};
use chrono::Utc;
use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
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
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: true,
            lightweight_scan: false,
            single_session_lookup: true,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Sqlite,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Unsupported),
                tool_call: Some(Fidelity::Preserved),
                tool_result: Some(Fidelity::Preserved),
                patch: Some(Fidelity::Unsupported),
                image: Some(Fidelity::Unsupported),
                file: Some(Fidelity::Unsupported),
                compressed: Some(Fidelity::Unsupported),
                provider_payload: Some(Fidelity::Preserved),
            },
            export_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Unsupported),
                tool_call: Some(Fidelity::Downgraded),
                tool_result: Some(Fidelity::Downgraded),
                patch: Some(Fidelity::Unsupported),
                image: Some(Fidelity::Unsupported),
                file: Some(Fidelity::Unsupported),
                compressed: Some(Fidelity::Unsupported),
                provider_payload: Some(Fidelity::Dropped),
            },
            resume_quality: ResumeQuality::Native,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: true,
                sidecar_files: true,
                index_repair: true,
            },
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
            },
            activity_support: ProviderActivitySupport {
                hook_events: false,
                runtime_endpoint: false,
                session_activity: false,
            },
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
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (id, preview, cwd, title, _created, updated) = row?;
            sessions.push(ProviderSessionSummary {
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
                created_at: None,
                last_active_at: Some(updated),
                source_path: Some(deepseek_source_locator(&id)?),
            });
        }
        Ok(sessions)
    }

    fn find_session_by_id(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
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
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .optional()?;

        meta.map(|(id, preview, cwd, title, updated)| {
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
                created_at: None,
                last_active_at: Some(updated),
                source_path: Some(deepseek_source_locator(&id)?),
            })
        })
        .transpose()
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let (db_path, thread_id) = parse_deepseek_source_locator(source_path)?;
        let conn = open_deepseek_read_only_db(&db_path)?;
        import_canonical_session_from_connection(&conn, &thread_id, source_path)
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        deepseek_source_fingerprint(source_path)
    }

    fn export_session(&self, session: &Session, target_dir: &Path) -> Result<ExportedSession> {
        let session_id = export_canonical_session(session, target_dir)?;
        Ok(export_result(
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

fn deepseek_source_fingerprint(source_locator: &str) -> Result<Option<ProviderSourceFingerprint>> {
    let (db_path, thread_id) = parse_deepseek_source_locator(source_locator)?;
    let Some(db_metadata) = std::fs::metadata(&db_path).ok() else {
        return Ok(None);
    };
    let conn = open_deepseek_read_only_db(&db_path)?;
    let thread_exists = conn
        .query_row("SELECT 1 FROM threads WHERE id = ?1", [&thread_id], |_| {
            Ok(())
        })
        .optional()?;
    if thread_exists.is_none() {
        return Ok(None);
    }

    let mut hasher = Sha256::new();
    hasher.update(b"deepseek-source-v1\0");
    hash_deepseek_bytes(&mut hasher, b"thread", thread_id.as_bytes());
    hash_deepseek_table_rows(&conn, &mut hasher, "threads", "id", &thread_id)?;
    for table in ["messages", "checkpoints", "thread_dynamic_tools"] {
        hash_deepseek_table_rows(&conn, &mut hasher, table, "thread_id", &thread_id)?;
    }

    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let wal_metadata = std::fs::metadata(&wal_path).ok();
    let modified_at_ms = deepseek_metadata_modified_at_ms(&db_metadata).max(
        wal_metadata
            .as_ref()
            .map(deepseek_metadata_modified_at_ms)
            .unwrap_or(0),
    );
    let size_bytes = i64::try_from(db_metadata.len())
        .unwrap_or(i64::MAX)
        .saturating_add(
            wal_metadata
                .as_ref()
                .map(|metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
                .unwrap_or(0),
        );
    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes,
        value: format!("sqlite-rows-v1:{:x}", hasher.finalize()),
    }))
}

fn hash_deepseek_table_rows(
    conn: &Connection,
    hasher: &mut Sha256,
    table: &str,
    selection_column: &str,
    thread_id: &str,
) -> Result<()> {
    let columns = deepseek_table_columns(conn, table)?;
    anyhow::ensure!(
        columns.iter().any(|column| column == selection_column),
        "DeepSeek table {table} must contain selection column {selection_column}"
    );
    let selected_columns = columns
        .iter()
        .map(|column| quote_deepseek_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let order_columns = selected_columns.clone();
    let query = format!(
        "SELECT {selected_columns} FROM {} WHERE {} = ?1 ORDER BY {order_columns}",
        quote_deepseek_identifier(table),
        quote_deepseek_identifier(selection_column),
    );
    let mut stmt = conn.prepare(&query)?;
    let mut rows = stmt.query([thread_id])?;
    hash_deepseek_bytes(hasher, b"table", table.as_bytes());
    for column in &columns {
        hash_deepseek_bytes(hasher, b"column", column.as_bytes());
    }
    while let Some(row) = rows.next()? {
        hasher.update(b"row\0");
        for index in 0..columns.len() {
            hash_deepseek_value(hasher, row.get_ref(index)?);
        }
    }
    Ok(())
}

fn deepseek_table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "PRAGMA table_info({})",
        quote_deepseek_identifier(table)
    ))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    anyhow::ensure!(
        !columns.is_empty(),
        "DeepSeek managed table does not exist: {table}"
    );
    Ok(columns)
}

fn hash_deepseek_value(hasher: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hasher.update(b"null\0"),
        ValueRef::Integer(value) => {
            hasher.update(b"integer\0");
            hasher.update(value.to_le_bytes());
        }
        ValueRef::Real(value) => {
            hasher.update(b"real\0");
            hasher.update(value.to_bits().to_le_bytes());
        }
        ValueRef::Text(value) => hash_deepseek_bytes(hasher, b"text", value),
        ValueRef::Blob(value) => hash_deepseek_bytes(hasher, b"blob", value),
    }
}

fn hash_deepseek_bytes(hasher: &mut Sha256, kind: &[u8], value: &[u8]) {
    hasher.update(kind);
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value);
    hasher.update(b"\0");
}

fn quote_deepseek_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn deepseek_metadata_modified_at_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
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

fn deepseek_source_locator(thread_id: &str) -> Result<String> {
    anyhow::ensure!(!thread_id.is_empty(), "DeepSeek thread ID cannot be empty");
    anyhow::ensure!(
        !thread_id.contains('#'),
        "DeepSeek thread ID cannot contain locator separators"
    );
    Ok(format!(
        "{}#thread={thread_id}",
        get_state_db_path().display()
    ))
}

fn parse_deepseek_source_locator(source_locator: &str) -> Result<(PathBuf, String)> {
    let (database_path, thread_id) = source_locator
        .rsplit_once("#thread=")
        .context("DeepSeek source locator must use '<database>#thread=<threadId>'")?;
    anyhow::ensure!(
        !database_path.is_empty() && !thread_id.is_empty(),
        "DeepSeek source locator must include a database path and thread ID"
    );
    anyhow::ensure!(
        !thread_id.contains('#'),
        "DeepSeek source locator contains an invalid thread ID"
    );
    Ok((PathBuf::from(database_path), thread_id.to_string()))
}

fn open_deepseek_read_only_db(path: &Path) -> Result<Connection> {
    if !path.is_file() {
        anyhow::bail!("DeepSeek state database not found: {}", path.display());
    }
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open DeepSeek state db at {}", path.display()))
}

fn get_session_index_path() -> PathBuf {
    get_deepseek_dir().join("session_index.jsonl")
}

fn export_canonical_session(session: &Session, target_dir: &Path) -> Result<String> {
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
    let title = session_title(session);
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
        let Some(visible_role) = event_visible_message_role(event) else {
            continue;
        };
        let Some(content) = deepseek_message_content(event) else {
            continue;
        };
        let role = match visible_role {
            Role::Assistant => "assistant",
            Role::Tool => "tool",
            Role::User => "user",
            Role::System | Role::Developer | _ => continue,
        };
        let item_json = deepseek_item_json(event);
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

fn deepseek_message_content(event: &Event) -> Option<String> {
    event_visible_message_text(event)
}

/// Build the item_json payload for a DeepSeek message row.
///
/// The import parser (`blocks_from_message`) looks for specific keys in
/// item_json: `tool_name`/`call_id`/`arguments` for tool calls, and
/// `output`/`tool_use_id`/`is_error` for tool results. Writing blocks in
/// this shape lets DeepSeek exports round-trip tool structure instead of
/// silently degrading to text.
fn deepseek_item_json(event: &Event) -> Value {
    for block in &event.blocks {
        match block {
            Block::ToolCall {
                tool_call_id,
                name,
                input,
            } => {
                return serde_json::json!({
                    "source": "memorph-canonical",
                    "tool_name": name,
                    "call_id": tool_call_id,
                    "arguments": input.clone().unwrap_or(Value::Null),
                });
            }
            Block::ToolResult {
                tool_call_id,
                content,
                outcome,
            } => {
                return serde_json::json!({
                    "source": "memorph-canonical",
                    "output": content,
                    "tool_use_id": tool_call_id,
                    "is_error": crate::session::execution_outcome_is_error(*outcome),
                });
            }
            _ => {}
        }
    }
    serde_json::json!({
        "source": "memorph-canonical",
        "event_id": event.id,
        "event_kind": event.kind,
        "event_role": event_role_label(event.role),
    })
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
    thread_id: &str,
    source_path: &str,
) -> Result<ImportedSession> {
    let thread = load_thread_row(conn, thread_id)?;
    let messages = load_message_rows(conn, thread_id)?;
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut event_meta = Vec::new();

    for message in messages {
        let raw_message = deepseek_message_value(&message);
        let timestamp =
            chrono::DateTime::from_timestamp(message.created_at, 0).unwrap_or_else(Utc::now);
        let role = deepseek_event_role(&message.role, &raw_message, &mut report);
        let (blocks, fidelity) = blocks_from_message(&message, &raw_message, &mut report);
        let mut provider_ext = BTreeMap::new();
        provider_ext.insert("deepseek_message".to_string(), raw_message);
        if !thread.model_provider.trim().is_empty() {
            provider_ext.insert(
                "model_provider".to_string(),
                Value::String(thread.model_provider.clone()),
            );
        }

        events.push(Event {
            id: format!("deepseek:message:{}", message.id),
            kind: deepseek_event_kind(&message.role, &blocks),
            role,
            timestamp,
            links: Links::default(),
            blocks,
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        });
        event_meta.push(EventMeta {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: Some(message.id.to_string()),
                original_role: Some(message.role),
                phase: None,
            },
            fidelity,
            provider_ext,
        });
    }

    let title = deepseek_thread_title(&thread);
    let mut extensions = BTreeMap::new();
    extensions.insert(
        "deepseek_thread".to_string(),
        deepseek_thread_value(&thread),
    );

    Ok(ImportedSession {
        session: Session {
            lineage: Vec::new(),
            schema: Schema::default(),
            identity: Identity {
                id: thread.id.clone(),
                title,
            },
            context: Context {
                workspace: Some(thread.cwd.clone()),
                created_at: chrono::DateTime::from_timestamp(thread.created_at, 0),
                last_active_at: chrono::DateTime::from_timestamp(thread.updated_at, 0),
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
                session_id: thread.id.clone(),
                source_path: Some(source_path.to_string()),
            },
            aliases: Vec::new(),
        },
        event_meta,
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

fn deepseek_event_role(role: &str, raw_message: &Value, report: &mut MappingReport) -> Role {
    match role {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "tool" => Role::Tool,
        "system" => Role::System,
        "developer" => Role::Developer,
        "history" => Role::System,
        other => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: Fidelity::Normalized,
                code: "unknown_role_normalized".to_string(),
                message: format!("Normalized unknown DeepSeek role '{}'", other),
                path: None,
                raw: Some(raw_message.clone()),
            });
            Role::Other
        }
    }
}

fn blocks_from_message(
    message: &MessageRow,
    raw_message: &Value,
    report: &mut MappingReport,
) -> (Vec<Block>, Fidelity) {
    let mut blocks = Vec::new();
    let mut fidelity = Fidelity::Preserved;
    let content = message.content.trim();

    match message.item_json.as_deref() {
        Some(raw_item) => match serde_json::from_str::<Value>(raw_item) {
            Ok(item) => {
                if message.role == "history" && !content.is_empty() {
                    blocks.push(Block::Text {
                        text: message.content.clone(),
                    });
                }

                if let Some(tool_name) = item.get("tool_name").and_then(|value| value.as_str()) {
                    blocks.push(Block::ToolCall {
                        tool_call_id: item
                            .get("call_id")
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| message.id.to_string()),
                        name: tool_name.to_string(),
                        input: item.get("arguments").cloned(),
                    });
                    if !content.is_empty() {
                        blocks.push(Block::Text {
                            text: message.content.clone(),
                        });
                    }
                } else if let Some(output) = item.get("output").and_then(|value| value.as_str()) {
                    blocks.push(Block::ToolResult {
                        tool_call_id: item
                            .get("tool_use_id")
                            .and_then(|value| value.as_str())
                            .map(str::to_string)
                            .unwrap_or_else(|| message.id.to_string()),
                        content: output.to_string(),
                        outcome: crate::session::execution_outcome(
                            item.get("is_error")
                                .and_then(|value| value.as_bool())
                                .unwrap_or(false),
                        ),
                    });
                    if !content.is_empty() && content != output {
                        blocks.push(Block::Text {
                            text: message.content.clone(),
                        });
                    }
                } else if !content.is_empty() {
                    blocks.push(Block::Text {
                        text: message.content.clone(),
                    });
                }

                blocks.push(Block::Other { raw: item });
            }
            Err(error) => {
                fidelity = Fidelity::Normalized;
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: Fidelity::Normalized,
                    code: "invalid_item_json".to_string(),
                    message: format!("Failed to parse DeepSeek item_json: {}", error),
                    path: Some(format!("message:{}", message.id)),
                    raw: Some(raw_message.clone()),
                });
                if !content.is_empty() {
                    blocks.push(Block::Text {
                        text: message.content.clone(),
                    });
                }
                blocks.push(Block::Other {
                    raw: Value::String(raw_item.to_string()),
                });
            }
        },
        None if !content.is_empty() => {
            blocks.push(Block::Text {
                text: message.content.clone(),
            });
        }
        None => {}
    }

    if blocks.is_empty() {
        fidelity = Fidelity::Normalized;
        blocks.push(Block::Other {
            raw: raw_message.clone(),
        });
    }

    (blocks, fidelity)
}

fn deepseek_event_kind(role: &str, blocks: &[Block]) -> EventKind {
    if role == "history" {
        EventKind::Lifecycle
    } else if blocks
        .iter()
        .any(|block| matches!(block, Block::ToolResult { .. }))
    {
        EventKind::Observation
    } else if blocks
        .iter()
        .any(|block| matches!(block, Block::ToolCall { .. }))
    {
        EventKind::Action
    } else if blocks
        .iter()
        .all(|block| matches!(block, Block::Other { .. }))
    {
        EventKind::Other
    } else {
        EventKind::Message
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

    struct TestConfigHomeGuard;

    impl TestConfigHomeGuard {
        fn new(path: &Path) -> Self {
            crate::config::set_test_home_dir(path.to_path_buf());
            Self
        }
    }

    impl Drop for TestConfigHomeGuard {
        fn drop(&mut self) {
            crate::config::reset_test_home_dir();
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
    fn deepseek_scan_locator_roundtrips_and_import_uses_the_locator() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-locator";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let db_path = get_state_db_path();
        let expected_locator = format!("{}#thread={session_id}", db_path.display());

        let sessions = DeepseekProvider.scan_sessions().unwrap();
        let summary = sessions
            .iter()
            .find(|session| session.session_id == session_id)
            .unwrap();
        assert_eq!(
            summary.source_path.as_deref(),
            Some(expected_locator.as_str())
        );
        assert_eq!(
            parse_deepseek_source_locator(summary.source_path.as_deref().unwrap()).unwrap(),
            (db_path.clone(), session_id.to_string())
        );
        assert!(parse_deepseek_source_locator(session_id).is_err());

        let meta = DeepseekProvider
            .get_session_meta(session_id)
            .unwrap()
            .unwrap();
        assert_eq!(meta.source_path, summary.source_path);
        let imported = DeepseekProvider
            .import_session(summary.source_path.as_deref().unwrap())
            .unwrap();
        assert_eq!(
            imported.provenance.primary_source.source_path.as_deref(),
            Some(expected_locator.as_str())
        );
        assert_eq!(imported.session.identity.id, session_id);
    }

    #[test]
    fn deepseek_capabilities_describe_sqlite_projection_and_management_boundaries() {
        let capabilities = DeepseekProvider.capabilities();

        assert!(capabilities.scan);
        assert!(capabilities.import);
        assert!(capabilities.export);
        assert!(capabilities.delete);
        assert!(capabilities.rename);
        assert!(capabilities.resume);
        assert_eq!(capabilities.scan_strategy, ScanStrategy::FullScan);
        assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);
        assert_eq!(capabilities.storage_shape, StorageShape::Sqlite);
        assert_eq!(capabilities.turn_quality, TurnQuality::Inferred);
        assert_eq!(capabilities.resume_quality, ResumeQuality::Native);
        assert_eq!(capabilities.write_risk.level, WriteRiskLevel::High);
        assert!(capabilities.write_risk.multiple_files);
        assert!(capabilities.write_risk.sqlite);
        assert!(capabilities.write_risk.sidecar_files);
        assert!(capabilities.write_risk.index_repair);
        assert!(!capabilities.activity_support.hook_events);
        assert!(!capabilities.activity_support.runtime_endpoint);
        assert!(!capabilities.activity_support.session_activity);
        assert_eq!(
            capabilities.import_fidelity.provider_payload,
            Some(Fidelity::Preserved)
        );
        assert_eq!(
            capabilities.export_fidelity.provider_payload,
            Some(Fidelity::Dropped)
        );
    }

    #[test]
    fn deepseek_projection_bootstrap_is_bodyless_and_detail_reads_locator_source() -> Result<()> {
        let source_root = tempdir()?;
        let home = tempdir()?;
        let _home_guard = TestConfigHomeGuard::new(home.path());
        let _deepseek_guard = use_test_deepseek_dir(source_root.path().join("deepseek"));
        let session_id = "thread-projection";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);

        let first = crate::core::projection::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(first.scanned_providers, 1);
        assert_eq!(first.discovered_sessions, 2);
        assert_eq!(first.projected_sessions, 2);
        assert_eq!(first.unchanged_sessions, 0);
        assert!(first.failures.is_empty());

        let conn = local_store::open_database()?;
        let stored: (String, String, String, String, i64) = conn.query_row(
            "SELECT s.id, src.source_path, src.storage_shape, src.source_cursor,
                    ss.stale
             FROM sessions s
             JOIN session_sources src ON src.id = s.primary_source_id
             JOIN session_snapshots ss ON ss.session_id = s.id
             WHERE s.provider_id = ?1 AND s.provider_session_id = ?2",
            params![PROVIDER_ID, session_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )?;
        let expected_locator = deepseek_source_locator(session_id)?;
        let expected_fingerprint = DeepseekProvider
            .session_source_fingerprint(&expected_locator)?
            .expect("fixture fingerprint");
        assert_eq!(stored.1, expected_locator);
        assert_eq!(stored.2, "sqlite");
        assert_eq!(stored.3, expected_fingerprint.value);
        assert_eq!(stored.4, 0);

        let body_table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(body_table_count, 0);
        drop(conn);

        let detail = crate::core::sessions::get_session_detail_view_page(
            PROVIDER_ID,
            session_id,
            0,
            Some(0),
        )?;
        assert!(detail.events.is_empty());
        assert!(detail.turns.is_empty());
        assert_eq!(detail.event_count, 1);
        assert_eq!(detail.message_count, 1);
        assert!(!detail.stale);
        assert_eq!(
            detail.source_path.as_deref(),
            Some(expected_locator.as_str())
        );
        assert_eq!(
            detail.projection_report.as_ref().unwrap().id,
            format!("source-read:{PROVIDER_ID}:{session_id}")
        );

        let conn = local_store::open_database()?;
        let cached_counts: (i64, i64, i64, i64) = conn.query_row(
            "SELECT event_count, message_count, turn_count, counts_complete
             FROM session_snapshots WHERE session_id = ?1",
            [stored.0.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(cached_counts, (1, 1, 1, 1));
        drop(conn);

        let second = crate::core::projection::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(second.projected_sessions, 0);
        assert_eq!(second.unchanged_sessions, 2);
        assert!(second.failures.is_empty());

        let conn = Connection::open(get_state_db_path())?;
        conn.execute(
            "UPDATE messages SET content = 'changed' WHERE thread_id = ?1",
            [session_id],
        )?;
        drop(conn);

        let stale_detail = crate::core::sessions::get_session_detail_view_page(
            PROVIDER_ID,
            session_id,
            0,
            Some(0),
        )?;
        assert!(stale_detail.stale);

        let refreshed = crate::core::projection::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(refreshed.projected_sessions, 1);
        assert_eq!(refreshed.unchanged_sessions, 1);
        assert!(refreshed.failures.is_empty());

        let fresh_detail = crate::core::sessions::get_session_detail_view_page(
            PROVIDER_ID,
            session_id,
            0,
            Some(0),
        )?;
        assert!(!fresh_detail.stale);
        assert_eq!(fresh_detail.message_count, 1);
        Ok(())
    }

    #[test]
    fn deepseek_locator_rejects_raw_thread_ids_and_reports_missing_sources() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-source-errors";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);

        assert!(DeepseekProvider.import_session(session_id).is_err());
        assert!(DeepseekProvider
            .session_source_fingerprint(session_id)
            .is_err());

        let missing_thread_locator = deepseek_source_locator("thread-missing").unwrap();
        assert!(DeepseekProvider
            .session_source_fingerprint(&missing_thread_locator)
            .unwrap()
            .is_none());
        let missing_db_locator = format!(
            "{}#thread={session_id}",
            dir.path().join("missing").join("state.db").display()
        );
        assert!(DeepseekProvider
            .session_source_fingerprint(&missing_db_locator)
            .unwrap()
            .is_none());
        assert!(DeepseekProvider
            .import_session(&missing_db_locator)
            .is_err());
    }

    #[test]
    fn deepseek_fingerprint_is_thread_scoped_and_allows_empty_message_sets() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek"));
        let session_id = "thread-fingerprint";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let locator = deepseek_source_locator(session_id).unwrap();

        let first = DeepseekProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .unwrap();
        let conn = Connection::open(get_state_db_path()).unwrap();
        conn.execute(
            "UPDATE threads SET unrelated_note = 'changed other' WHERE id = 'thread-other'",
            [],
        )
        .unwrap();
        drop(conn);
        let unrelated_change = DeepseekProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .unwrap();
        assert_eq!(first.value, unrelated_change.value);

        let conn = Connection::open(get_state_db_path()).unwrap();
        conn.execute("DELETE FROM messages WHERE thread_id = ?1", [session_id])
            .unwrap();
        drop(conn);
        let empty_messages = DeepseekProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .unwrap();
        assert_ne!(first.value, empty_messages.value);
        let imported = DeepseekProvider.import_session(&locator).unwrap();
        assert!(imported.session.events.is_empty());

        let conn = Connection::open(get_state_db_path()).unwrap();
        conn.execute(
            "UPDATE checkpoints SET note = 'changed target' WHERE thread_id = ?1",
            [session_id],
        )
        .unwrap();
        drop(conn);
        let target_change = DeepseekProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .unwrap();
        assert_ne!(empty_messages.value, target_change.value);
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
    fn restore_rejects_backup_when_current_state_database_changes() {
        let dir = tempdir().unwrap();
        let _guard = use_test_deepseek_dir(dir.path().join("deepseek-a"));
        let session_id = "thread-source-boundary";
        write_native_deepseek_fixture(&get_deepseek_dir(), session_id);
        let backup = DeepseekProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-source-boundary",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        let other_dir = dir.path().join("deepseek-b");
        write_native_deepseek_fixture(&other_dir, session_id);
        set_test_deepseek_dir(Some(other_dir.clone()));
        crate::cache::global_cache().invalidate(PROVIDER_ID);

        let error = DeepseekProvider
            .restore_session_backup(&backup)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("does not match the current state database"),
            "{error:#}"
        );
        assert_eq!(
            deepseek_session_row_counts(&other_dir, session_id),
            vec![1, 1, 1, 1]
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

        let imported =
            import_canonical_session_from_connection(&conn, "thread-1", "state.db#thread=thread-1")
                .unwrap();

        assert_eq!(
            imported.session.context.workspace.as_deref(),
            Some("/tmp/workspace")
        );
        assert_eq!(
            imported.session.identity.title.as_deref(),
            Some("Named Thread")
        );
        assert_eq!(imported.session.events.len(), 3);
        assert_eq!(imported.session.events[0].kind, EventKind::Lifecycle);
        assert!(matches!(
            imported.session.events[1].blocks.first(),
            Some(Block::ToolCall {
                name,
                tool_call_id,
                ..
            }) if name == "read_file" && tool_call_id == "call-1"
        ));
        assert!(matches!(
            imported.session.events[2].blocks.first(),
            Some(Block::ToolResult {
                tool_call_id,
                content,
                outcome
            }) if tool_call_id == "call-1" && content == "file contents" && *outcome == crate::session::ExecutionOutcome::Succeeded
        ));
        assert!(imported.session.extensions.contains_key("deepseek_thread"));
    }

    #[test]
    fn compressed_segment_exports_as_portable_deepseek_message_content() {
        let event = Event {
            id: "compressed-source".to_string(),
            kind: EventKind::Message,
            role: Role::Assistant,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::Compressed {
                raw: serde_json::json!({
                    "format": "memorph.compressed.v1",
                    "source_provider_id": "opencode",
                    "summary": "compressed summary",
                    "source_event_ids": ["old-event-1", "old-event-2", "old-event-3"],
                    "source_event_count": 3,
                    "archive_ref": "memorph-archive://s1/archive.json.gz",
                }),
            }],
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
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
        let event = Event {
            id: "internal".to_string(),
            kind: EventKind::Lifecycle,
            role: Role::System,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::Text {
                text: "internal context".to_string(),
            }],
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        };

        assert!(deepseek_message_content(&event).is_none());
    }

    #[test]
    fn deepseek_item_json_roundtrips_tool_call_and_result_through_blocks_from_message() {
        // Build a ToolCall event that export would process.
        let tool_call_event = Event {
            id: "evt-call-1".to_string(),
            kind: EventKind::Action,
            role: Role::Assistant,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::ToolCall {
                tool_call_id: "call-42".to_string(),
                name: "read_file".to_string(),
                input: Some(json!({"path": "src/lib.rs"})),
            }],
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        };

        let item_json_value = deepseek_item_json(&tool_call_event);
        assert_eq!(item_json_value["tool_name"], "read_file");
        assert_eq!(item_json_value["call_id"], "call-42");
        assert_eq!(item_json_value["arguments"]["path"], "src/lib.rs");

        // Simulate what export writes into the DB row.
        let call_row = MessageRow {
            id: 10,
            role: "assistant".to_string(),
            content: "[Tool use: read_file (call-42)]\n{\"path\":\"src/lib.rs\"}"
                .to_string(),
            item_json: Some(serde_json::to_string(&item_json_value).unwrap()),
            created_at: 1710000002,
        };
        let raw_call = serde_json::json!({"role": "assistant", "content": call_row.content});
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let (blocks, _fidelity) = blocks_from_message(&call_row, &raw_call, &mut report);

        // The first block must be a ToolCall, not degraded text.
        assert!(
            blocks.iter().any(|b| matches!(
                b,
                Block::ToolCall { name, tool_call_id, .. }
                    if name == "read_file" && tool_call_id == "call-42"
            )),
            "ToolCall did not round-trip; blocks = {:?}",
            blocks
        );

        // Now a ToolResult event.
        let tool_result_event = Event {
            id: "evt-result-1".to_string(),
            kind: EventKind::Observation,
            role: Role::Tool,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::ToolResult {
                tool_call_id: "call-42".to_string(),
                content: "file contents here".to_string(),
                outcome: crate::session::ExecutionOutcome::Succeeded,
            }],
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        };

        let result_item = deepseek_item_json(&tool_result_event);
        assert_eq!(result_item["output"], "file contents here");
        assert_eq!(result_item["tool_use_id"], "call-42");
        assert_eq!(result_item["is_error"], false);

        let result_row = MessageRow {
            id: 11,
            role: "tool".to_string(),
            content: "file contents here".to_string(),
            item_json: Some(serde_json::to_string(&result_item).unwrap()),
            created_at: 1710000003,
        };
        let raw_result = serde_json::json!({"role": "tool", "content": result_row.content});
        let mut report2 = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let (blocks2, _) = blocks_from_message(&result_row, &raw_result, &mut report2);

        assert!(
            blocks2.iter().any(|b| matches!(
                b,
                Block::ToolResult { tool_call_id, content, outcome }
                    if tool_call_id == "call-42"
                        && content == "file contents here"
                        && *outcome == crate::session::ExecutionOutcome::Succeeded
            )),
            "ToolResult did not round-trip; blocks = {:?}",
            blocks2
        );

        // An event with only Text falls back to the generic shape.
        let text_event = Event {
            id: "evt-text-1".to_string(),
            kind: EventKind::Message,
            role: Role::User,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::Text {
                text: "hello world".to_string(),
            }],
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: Metadata {
                model: None,
                usage: None,
            },
        };
        let text_item = deepseek_item_json(&text_event);
        assert!(text_item.get("tool_name").is_none());
        assert!(text_item.get("output").is_none());
        assert_eq!(text_item["source"], "memorph-canonical");
    }

}
