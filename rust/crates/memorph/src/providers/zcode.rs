//! ZCode (Z.ai's GLM harness, desktop app).
//!
//! The agent runtime `~/.zcode/cli/` is an OpenCode derivative: `cli/db/db.sqlite`
//! has the same session / message / part tables, and part JSON (text / reasoning /
//! tool{callID,tool,state} / step-start / step-finish) is field-compatible —
//! event conversion is reused from the OpenCode provider
//! ([`crate::providers::opencode::load::imported_session_from_data`]).
//!
//! Differences handled here (semantics verified against the zcode open-source
//! tree by the Wake reference implementation):
//! - session table has no model / token columns; model lives per-message in
//!   `message.data.modelId` (note the lowercase `d`) and is normalized to the
//!   OpenCode `modelID` shape before conversion.
//! - `task_type` is ALTER-added: PRAGMA-probe per open; when present, only
//!   interactive / fork / selection_side_chat are user sessions
//!   (subagent_child / workflow_child are runtime noise); when absent, fall
//!   back to `parent_id IS NULL` (OpenCode semantics).
//! - `v2/tasks-index.sqlite` (`tasks.task_id` = `session.id`) supplies two
//!   filters: `deleted` (desktop soft-delete) and `migration_source`
//!   (first-run import from Claude Code — already listed once there). Read
//!   failure degrades to no filtering.
//! - message ordering prefers the ALTER-added `sequence` column, falling back
//!   to OpenCode's `time_created, id`.
//!
//! Import-only: the desktop app holds a write lock on the runtime, so
//! resume / export are not offered (`resume` is only a CLI affordance in
//! ZCode's case anyway — there is no `zcode` CLI on PATH).

use crate::provider::{
    PageStrategy, Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionSummary, ProviderSourceFingerprint, ProviderWriteRisk,
    ResumeQuality, ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::providers::opencode::load::imported_session_from_data;
use crate::session::{Fidelity, ImportedSession};
use anyhow::{Context as _, Result};
use rusqlite::Connection;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub struct ZcodeProvider;
const PROVIDER_ID: &str = "zcode";

#[cfg(test)]
static TEST_ZCODE_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();
static TEST_ZCODE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

impl Provider for ZcodeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }
    fn name(&self) -> &'static str {
        "ZCode"
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
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Preserved),
                tool_result: Some(Fidelity::Preserved),
                provider_payload: Some(Fidelity::Dropped),
                ..ProviderContentFidelity::unknown()
            },
            resume_quality: ResumeQuality::None,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: true,
                sidecar_files: false,
                index_repair: true,
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
        let Some(db) = db_path() else {
            return Ok(Vec::new());
        };
        if !db.is_file() {
            return Ok(Vec::new());
        }
        let conn = Connection::open(&db)
            .with_context(|| format!("failed to open ZCode db at {}", db.display()))?;
        let has_task_type = table_has_column(&conn, "session", "task_type");
        let listed = if has_task_type {
            "task_type IN ('interactive', 'fork', 'selection_side_chat')"
        } else {
            "parent_id IS NULL"
        };
        let mut stmt = conn.prepare(&format!(
            "SELECT id, directory, title, time_created, time_updated,
                    (SELECT COALESCE(SUM(LENGTH(p.data)), 0) FROM part p WHERE p.session_id = session.id)
             FROM session WHERE {listed} ORDER BY time_updated DESC"
        ))?;
        let hidden = hidden_session_ids();
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, i64>(5)?,
            ))
        })?;
        let mut sessions = Vec::new();
        for row in rows.flatten() {
            let (id, directory, title, created, updated, content_len) = row;
            if content_len == 0 || hidden.contains(&id) {
                continue;
            }
            sessions.push(ProviderSessionSummary {
                archived: false,
                session_id: id.clone(),
                title: Some(title),
                project_dir: Some(directory),
                created_at: Some(created),
                last_active_at: Some(updated),
                source_path: Some(source_locator(&db, &id)),
            });
        }
        Ok(sessions)
    }
    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let (db, session_id) = parse_source_locator(source_path)?;
        let conn = Connection::open(&db)
            .with_context(|| format!("failed to open ZCode db at {}", db.display()))?;

        let mut session_stmt = conn.prepare(
            "SELECT id, directory, title, time_created, time_updated FROM session WHERE id = ?1",
        )?;
        let (id, directory, title, created, updated) = session_stmt.query_row(
            [&session_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )?;
        let session_json = serde_json::json!({
            "id": id,
            "directory": directory,
            "title": title,
            "time": { "created": created, "updated": updated },
        });

        let msg_order = if table_has_column(&conn, "message", "sequence") {
            "sequence, id"
        } else {
            "time_created, id"
        };
        let mut msg_stmt = conn.prepare(&format!(
            "SELECT id, time_created, data FROM message WHERE session_id = ?1 ORDER BY {msg_order}"
        ))?;
        let msg_rows = msg_stmt.query_map([&session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut messages = Vec::new();
        for row in msg_rows.flatten() {
            let (msg_id, time_created, data_str) = row;
            let mut data: Value =
                serde_json::from_str(&data_str).unwrap_or_else(|_| Value::Object(Default::default()));
            if let Value::Object(map) = &mut data {
                map.insert("id".to_string(), Value::String(msg_id));
                // ZCode writes `modelId`; the OpenCode converter reads `modelID`.
                if let Some(model) = map.get("modelId").cloned() {
                    map.insert("modelID".to_string(), model);
                }
            }
            messages.push((Some(time_created), data));
        }

        let part_order = if table_has_column(&conn, "part", "sequence") {
            "message_id, sequence, id"
        } else {
            "message_id, time_created, id"
        };
        let mut part_stmt = conn.prepare(&format!(
            "SELECT id, message_id, data FROM part WHERE session_id = ?1 ORDER BY {part_order}"
        ))?;
        let part_rows = part_stmt.query_map([&session_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        let mut parts_map: HashMap<String, Vec<Value>> = HashMap::new();
        for row in part_rows.flatten() {
            let (part_id, message_id, data_str) = row;
            let mut data: Value =
                serde_json::from_str(&data_str).unwrap_or_else(|_| Value::Object(Default::default()));
            if let Value::Object(map) = &mut data {
                map.insert("id".to_string(), Value::String(part_id));
            }
            parts_map.entry(message_id).or_default().push(data);
        }

        let mut imported = imported_session_from_data(
            &session_id,
            (session_json, messages, parts_map),
        )?;
        imported.provenance.primary_source = crate::session::ProviderRef {
            provider_id: PROVIDER_ID.into(),
            session_id: session_id.clone(),
            source_path: Some(source_path.to_string()),
        };
        imported.event_meta = (0..imported.session.events.len())
            .map(|_| crate::session::EventMeta::preserved(PROVIDER_ID))
            .collect();
        Ok(imported)
    }
    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        let (db, session_id) = parse_source_locator(source_path)?;
        if !db.is_file() {
            return Ok(None);
        }
        let meta = std::fs::metadata(&db)?;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0);
        Ok(Some(ProviderSourceFingerprint {
            modified_at_ms: modified,
            size_bytes: meta.len().min(i64::MAX as u64) as i64,
            value: format!("{PROVIDER_ID}-session-v1:{session_id}"),
        }))
    }
    fn data_source_paths(&self) -> Vec<PathBuf> {
        db_path().into_iter().collect()
    }
}

fn zcode_home() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(lock) = TEST_ZCODE_DIR.get_or_init(|| std::sync::Mutex::new(None)).lock().ok() {
        if let Some(dir) = lock.as_ref() {
            return Some(dir.clone());
        }
    }
    std::env::var_os("ZCODE_STORAGE_DIR")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".zcode")))
}

fn db_path() -> Option<PathBuf> {
    zcode_home().map(|home| home.join("cli/db/db.sqlite"))
}

fn tasks_db_path() -> Option<PathBuf> {
    zcode_home().map(|home| home.join("v2/tasks-index.sqlite"))
}

fn source_locator(db: &Path, session_id: &str) -> String {
    format!("{}#session={session_id}", db.to_string_lossy())
}

fn parse_source_locator(source_path: &str) -> Result<(PathBuf, String)> {
    let (db, session_id) = source_path
        .rsplit_once("#session=")
        .context("ZCode source must be a `<db>#session=<id>` locator")?;
    Ok((PathBuf::from(db), session_id.to_string()))
}

fn table_has_column(conn: &Connection, table: &str, column: &str) -> bool {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info({table})")) else {
        return false;
    };
    let found = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map(|rows| rows.flatten().any(|name| name == column))
        .unwrap_or(false);
    found
}

/// tasks-index soft-deleted or first-run Claude imports; read failure degrades
/// to an empty set (those sessions stay listed rather than disappearing).
fn hidden_session_ids() -> std::collections::HashSet<String> {
    let mut hidden = std::collections::HashSet::new();
    let Some(tasks_db) = tasks_db_path() else {
        return hidden;
    };
    if !tasks_db.is_file() {
        return hidden;
    }
    let Ok(conn) = Connection::open(&tasks_db) else {
        return hidden;
    };
    let Ok(mut stmt) =
        conn.prepare("SELECT task_id FROM tasks WHERE deleted = 1 OR migration_source IS NOT NULL")
    else {
        return hidden;
    };
    if let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0)) {
        hidden.extend(rows.flatten());
    }
    hidden
}

#[cfg(test)]
mod tests {
    use super::*;

    // Single TEST_ZCODE_DIR slot is process-global; hold a dedicated mutex for
    // the test duration so parallel zcode tests can't overwrite each other's
    // dir (runtime reads take the slot lock only briefly, so it stays free).
    struct ZcodeDirGuard {
        _serial: std::sync::MutexGuard<'static, ()>,
    }

    impl ZcodeDirGuard {
        fn new(dir: PathBuf) -> Self {
            let _serial = TEST_ZCODE_LOCK
                .get_or_init(|| std::sync::Mutex::new(()))
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *TEST_ZCODE_DIR
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
                .unwrap() = Some(dir);
            ZcodeDirGuard { _serial }
        }
    }

    impl Drop for ZcodeDirGuard {
        fn drop(&mut self) {
            *TEST_ZCODE_DIR
                .get_or_init(|| std::sync::Mutex::new(None))
                .lock()
                .unwrap() = None;
        }
    }

    fn create_zcode_db(dir: &Path) -> PathBuf {
        let db = dir.join("cli/db/db.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT, title TEXT, task_type TEXT, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_archived INTEGER);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, time_created INTEGER NOT NULL, data TEXT NOT NULL);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT NOT NULL, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, data TEXT NOT NULL);
             INSERT INTO session VALUES ('sess-1', NULL, '/tmp/zc-project', 'Fix login flow', 'interactive', 1000, 2000, NULL);
             INSERT INTO session VALUES ('sess-child', 'sess-1', '/tmp/zc-project', 'subagent', 'subagent_child', 1000, 2000, NULL);
             INSERT INTO message VALUES ('msg-1', 'sess-1', 'user', 1100, '{\"role\":\"user\"}');
             INSERT INTO message VALUES ('msg-2', 'sess-1', 'assistant', 1200, '{\"role\":\"assistant\",\"modelId\":\"glm-5.3\"}');
             INSERT INTO part VALUES ('p-1', 'msg-1', 'sess-1', 1100, '{\"type\":\"text\",\"text\":\"hello zcode\"}');
             INSERT INTO part VALUES ('p-2', 'msg-2', 'sess-1', 1200, '{\"type\":\"text\",\"text\":\"hi back\"}');",
        )
        .unwrap();
        drop(conn);
        db
    }

    #[test]
    fn scans_only_user_task_types_and_imports_via_opencode_converter() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = ZcodeDirGuard::new(dir.path().to_path_buf());
        let db = create_zcode_db(dir.path());

        let conn = Connection::open(&db).unwrap();
        let has_task_type = table_has_column(&conn, "session", "task_type");
        drop(conn);
        assert!(has_task_type);

        let sessions = ZcodeProvider.scan_sessions().unwrap();
        assert_eq!(sessions.len(), 1, "subagent_child must not be listed");
        assert_eq!(sessions[0].session_id, "sess-1");
        assert_eq!(sessions[0].project_dir.as_deref(), Some("/tmp/zc-project"));

        let imported = ZcodeProvider
            .import_session(&sessions[0].source_path.clone().unwrap())
            .unwrap();
        assert_eq!(imported.session.identity.title.as_deref(), Some("Fix login flow"));
        assert_eq!(imported.session.context.workspace.as_deref(), Some("/tmp/zc-project"));
        assert_eq!(imported.session.events.len(), 2);
        assert_eq!(
            imported.session.events[1].metadata.model.as_deref(),
            Some("glm-5.3"),
            "modelId must be normalized to modelID"
        );
        assert_eq!(
            imported.provenance.primary_source.provider_id, PROVIDER_ID
        );
    }

    #[test]
    fn old_schema_without_task_type_falls_back_to_parent_id() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = ZcodeDirGuard::new(dir.path().to_path_buf());
        let db = dir.path().join("cli/db/db.sqlite");
        std::fs::create_dir_all(db.parent().unwrap()).unwrap();
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT, title TEXT, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_archived INTEGER);
             CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, role TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
             INSERT INTO session VALUES ('old-1', NULL, '/tmp/old', 'Old session', 1, 2, NULL);
             INSERT INTO message VALUES ('old-msg-1', 'old-1', 'user', 1, '{}');
             INSERT INTO part VALUES ('old-p-1', 'old-msg-1', 'old-1', 1, '{\"type\":\"text\",\"text\":\"old body\"}');",
        )
        .unwrap();
        drop(conn);

        let sessions = ZcodeProvider.scan_sessions().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "old-1");
    }
}
