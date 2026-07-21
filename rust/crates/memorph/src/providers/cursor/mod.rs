pub mod adapter;
mod backup;
mod db;
pub mod hook;
mod load;
mod scan;
mod write;

use crate::canonical::{CanonicalSession, ExportedSession, ImportedSession};
use crate::provider::{
    canonical_export_result, PageStrategy, Provider, ProviderActivitySupport,
    ProviderBackupSupport, ProviderCapabilities, ProviderSessionBackup, ProviderSessionSummary,
    ProviderSourceMutation, ProviderWriteRisk, ScanStrategy, StorageShape, TurnQuality,
    WriteRiskLevel,
};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct CursorProvider;

const PROVIDER_ID: &str = "cursor";

impl Provider for CursorProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Cursor"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: false,
            scan_strategy: ScanStrategy::Indexed,
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Sqlite,
            turn_quality: TurnQuality::Inferred,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: false,
                sqlite: true,
                sidecar_files: false,
                index_repair: false,
            },
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
            },
            activity_support: ProviderActivitySupport {
                hook_events: true,
                runtime_endpoint: true,
                session_activity: true,
            },
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        scan::scan_sessions(None)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        load::import_session(source_path)
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<crate::provider::ProviderSourceFingerprint>> {
        db::source_fingerprint(source_path)
    }

    fn export_session(
        &self,
        session: &CanonicalSession,
        target_dir: &Path,
    ) -> Result<ExportedSession> {
        let session_id = write::export_session(session, target_dir)?;
        Ok(canonical_export_result(
            PROVIDER_ID,
            session_id.clone(),
            self.resume_command(&session_id),
            session,
            self.capabilities(),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        write::delete_session(session_id)
    }

    fn delete_sessions(&self, session_ids: &[&str]) -> Vec<Result<()>> {
        write::delete_sessions(session_ids)
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        write::rename_session(session_id, new_title)
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

    fn session_size(&self, session_id: &str) -> Result<u64> {
        db::composer_size(session_id)
    }

    fn session_sizes(&self, session_ids: &[&str]) -> HashMap<String, u64> {
        db::composer_sizes(session_ids).unwrap_or_default()
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        db::global_state_db_path().ok().into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::session_management,
        storage::{
            activity_store::{ActivityActor, ActivityOperationKind, ActivityQuery, ActivityStatus},
            artifact_store::{ArtifactVerificationStatus, BackupQuery, BackupRestoreStatus},
            local_store,
        },
    };
    use rusqlite::types::Value as SqliteValue;
    use rusqlite::{params, Connection, OptionalExtension};
    use serde_json::{json, Value};
    use tempfile::tempdir;

    static TEST_CURSOR_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    struct TestCursorDbGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
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

    impl Drop for TestCursorDbGuard {
        fn drop(&mut self) {
            crate::cache::global_cache().invalidate(PROVIDER_ID);
            write::set_test_cursor_mutation_failure(None);
            db::set_test_cursor_db_path(None);
        }
    }

    fn use_test_cursor_db(path: PathBuf) -> TestCursorDbGuard {
        let lock = TEST_CURSOR_TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        db::set_test_cursor_db_path(Some(path));
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        TestCursorDbGuard { _lock: lock }
    }

    fn cursor_audit_fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/providers/cursor/fixtures/v3_11_19")
    }

    #[test]
    fn cursor_3_11_19_audit_fixture_matches_current_sqlite_contract() {
        let root = cursor_audit_fixture_root();
        let manifest: Value =
            serde_json::from_str(&std::fs::read_to_string(root.join("fixture.json")).unwrap())
                .unwrap();

        assert_eq!(manifest["provider"], "cursor");
        assert_eq!(manifest["observed_cursor_version"], "3.11.19");
        assert_eq!(manifest["captured_on"], "2026-07-16");
        assert_eq!(manifest["provenance"], "sanitized-local-source");
        assert_eq!(manifest["raw_user_content_committed"], false);
        assert_eq!(manifest["raw_user_identifiers_committed"], false);
        assert_eq!(manifest["live_source_mutated"], false);
        assert_eq!(manifest["journal_mode"], "wal");
        assert_eq!(
            manifest["session_identity"]["provider_session_id"],
            "composerId"
        );
        assert_eq!(
            manifest["session_identity"]["composer_id_only_is_physical_locator"],
            false
        );
        assert_eq!(
            manifest["pagination_finding"]["native_cursor_proven"],
            false
        );
        assert_eq!(
            manifest["pagination_finding"]["safe_initial_strategy"],
            "FullImport"
        );

        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(&std::fs::read_to_string(root.join("schema.sql")).unwrap())
            .unwrap();
        let table_columns = |table: &str| {
            let mut stmt = conn
                .prepare(&format!("PRAGMA table_info({table})"))
                .unwrap();
            stmt.query_map([], |row| row.get::<_, String>(1))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };

        assert_eq!(table_columns("ItemTable"), ["key", "value"]);
        assert_eq!(table_columns("cursorDiskKV"), ["key", "value"]);
        assert_eq!(
            table_columns("composerHeaders"),
            [
                "composerId",
                "workspaceId",
                "createdAt",
                "lastUpdatedAt",
                "isArchived",
                "isSubagent",
                "recency",
                "checkpointAt",
                "value",
            ]
        );
    }

    #[test]
    fn cursor_3_11_19_audit_fixture_is_structural_and_records_source_gaps() {
        let root = cursor_audit_fixture_root();
        let manifest_text = std::fs::read_to_string(root.join("fixture.json")).unwrap();
        let inventory_text = std::fs::read_to_string(root.join("field_inventory.json")).unwrap();
        let manifest: Value = serde_json::from_str(&manifest_text).unwrap();
        let inventory: Value = serde_json::from_str(&inventory_text).unwrap();

        assert!(!manifest_text.contains("/Users/"));
        assert!(!inventory_text.contains("/Users/"));
        assert!(!inventory_text.contains("file:///"));
        assert_eq!(
            inventory["content_policy"],
            "field names, JSON types, and aggregate presence counts only; no source values"
        );
        assert_eq!(inventory["composerData"]["rows"], 28);
        assert_eq!(inventory["composerData"]["invalid_json_rows"], 1);
        assert_eq!(inventory["bubbleId"]["rows"], 4633);
        assert_eq!(inventory["bubbleId"]["invalid_json_rows"], 0);
        assert_eq!(inventory["composerHeaders_value"]["rows"], 37);

        for field in [
            "composerId",
            "workspaceIdentifier",
            "createdAt",
            "lastUpdatedAt",
            "fullConversationHeadersOnly",
        ] {
            assert!(inventory["composerData"]["top_level_fields"][field].is_object());
        }
        for field in [
            "bubbleId",
            "type",
            "createdAt",
            "requestId",
            "toolFormerData",
            "toolResults",
        ] {
            assert!(inventory["bubbleId"]["top_level_fields"][field].is_object());
        }

        assert_eq!(
            manifest["observed_relationships"]
                ["fullConversationHeadersOnly_exactly_covers_bubble_rows"],
            "15/27 sessions"
        );
        assert_eq!(
            manifest["mutation_boundary_finding"]
                ["existing_synthetic_fixture_columns_not_observed"],
            json!(["owner", "revision"])
        );
        assert_eq!(
            manifest["mutation_boundary_finding"]["current_index_table"],
            "composerHeaders"
        );
        assert_eq!(
            manifest["source_plane_findings"]["global_state_database"],
            "only observed plane containing composerHeaders, composerData, and bubbleId session records"
        );
    }

    #[test]
    fn cursor_current_locator_and_fingerprint_cover_database_and_session_rows() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "composer-fingerprint";

        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("fixtures/v3_11_19/schema.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, NULL, ?6)",
            params![
                session_id,
                "workspace-fingerprint",
                1_700_000_000_000_i64,
                1_700_000_001_000_i64,
                1_700_000_001_000_i64,
                json!({"composerId": session_id, "name": "Sanitized"}).to_string(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                format!("composerData:{session_id}"),
                json!({
                    "composerId": session_id,
                    "text": "Sanitized session",
                    "createdAt": 1_700_000_000_000_i64,
                    "lastUpdatedAt": 1_700_000_001_000_i64,
                    "workspaceIdentifier": {
                        "id": "workspace-fingerprint",
                        "uri": {"fsPath": "/workspace/sanitized"}
                    }
                })
                .to_string(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                format!("bubbleId:{session_id}:bubble-1"),
                json!({
                    "bubbleId": "bubble-1",
                    "type": 1,
                    "createdAt": "2024-01-01T00:00:00Z",
                    "requestId": "request-1",
                    "text": "Sanitized message"
                })
                .to_string(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            params!["composer.composerHeaders.migratedToTable", "true"],
        )
        .unwrap();
        drop(conn);

        let locator = db::cursor_source_locator(session_id).unwrap();
        assert_eq!(
            db::parse_cursor_source_locator(&locator).unwrap(),
            (db_path.clone(), session_id.to_string())
        );
        assert!(db::parse_cursor_source_locator(session_id).is_err());

        let first = CursorProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .expect("current session source");
        assert!(first.value.starts_with("sqlite-rows-v1:"));
        let imported = CursorProvider.import_session(&locator).unwrap();
        assert_eq!(
            imported
                .session
                .provenance
                .primary_source
                .source_path
                .as_deref(),
            Some(locator.as_str())
        );

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE cursorDiskKV SET value = ?1 WHERE key = ?2",
            params![
                json!({
                    "bubbleId": "bubble-1",
                    "type": 1,
                    "createdAt": "2024-01-01T00:00:00Z",
                    "requestId": "request-1",
                    "text": "Changed sanitized message"
                })
                .to_string(),
                format!("bubbleId:{session_id}:bubble-1"),
            ],
        )
        .unwrap();
        drop(conn);
        let bubble_changed = CursorProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .expect("updated current session source");
        assert_ne!(first.value, bubble_changed.value);

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE composerHeaders SET lastUpdatedAt = lastUpdatedAt + 1 WHERE composerId = ?1",
            [session_id],
        )
        .unwrap();
        drop(conn);
        let header_changed = CursorProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .expect("header-backed current session source");
        assert_ne!(bubble_changed.value, header_changed.value);

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE cursorDiskKV SET value = ?1 WHERE key = ?2",
            params![
                json!({
                    "composerId": session_id,
                    "text": "Changed sanitized session",
                    "createdAt": 1_700_000_000_000_i64,
                    "lastUpdatedAt": 1_700_000_001_000_i64,
                    "workspaceIdentifier": {
                        "id": "workspace-fingerprint",
                        "uri": {"fsPath": "/workspace/sanitized"}
                    }
                })
                .to_string(),
                format!("composerData:{session_id}"),
            ],
        )
        .unwrap();
        drop(conn);
        let data_changed = CursorProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .expect("data-backed current session source");
        assert_ne!(header_changed.value, data_changed.value);

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "UPDATE ItemTable SET value = 'false'
             WHERE key = 'composer.composerHeaders.migratedToTable'",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES ('header-only', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                "composerData:data-only",
                json!({"composerId": "data-only"}).to_string()
            ],
        )
        .unwrap();
        drop(conn);
        let marker_changed = CursorProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .expect("current session source after migration marker update");
        assert_ne!(data_changed.value, marker_changed.value);
        assert!(CursorProvider
            .session_source_fingerprint(&db::cursor_source_locator("header-only").unwrap())
            .unwrap()
            .is_some());
        assert!(CursorProvider
            .session_source_fingerprint(&db::cursor_source_locator("data-only").unwrap())
            .unwrap()
            .is_some());

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "DELETE FROM composerHeaders WHERE composerId = ?1",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM cursorDiskKV WHERE key = ?1 OR key LIKE ?2",
            params![
                format!("composerData:{session_id}"),
                format!("bubbleId:{session_id}:%"),
            ],
        )
        .unwrap();
        drop(conn);
        assert!(CursorProvider
            .session_source_fingerprint(&locator)
            .unwrap()
            .is_none());
    }

    #[test]
    fn cursor_current_discovery_excludes_empty_state_draft_and_keeps_partial_sessions() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("fixtures/v3_11_19/schema.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES ('header-only', NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?1)",
            [json!({"composerId": "header-only", "name": "Header only"}).to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES
             ('composerData:data-only', ?1),
             ('composerData:empty-state-draft', NULL)",
            [json!({"composerId": "data-only", "name": "Data only"}).to_string()],
        )
        .unwrap();
        drop(conn);

        let sessions = db::list_session_metadata().unwrap();
        let ids = sessions
            .iter()
            .map(|session| session.composer_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["data-only", "header-only"]);
        assert!(db::load_source(&db::cursor_source_locator("empty-state-draft").unwrap()).is_err());
    }

    #[derive(Clone, Debug, PartialEq)]
    struct CurrentHeaderRow {
        composer_id: String,
        workspace_id: Option<String>,
        created_at: Option<i64>,
        last_updated_at: Option<i64>,
        is_archived: Option<i64>,
        is_subagent: Option<i64>,
        recency: Option<i64>,
        checkpoint_at: Option<i64>,
        value: Option<String>,
    }

    #[derive(Clone, Debug, PartialEq)]
    struct CurrentDiskRow {
        key: String,
        value: SqliteValue,
    }

    struct CurrentCursorFixture {
        target_header: CurrentHeaderRow,
        target_disk_rows: Vec<CurrentDiskRow>,
        unrelated_header: CurrentHeaderRow,
        unrelated_disk_rows: Vec<CurrentDiskRow>,
        item_rows: Vec<CurrentDiskRow>,
    }

    fn write_current_cursor_management_fixture(
        db_path: &Path,
        session_id: &str,
    ) -> CurrentCursorFixture {
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(include_str!("fixtures/v3_11_19/schema.sql"))
            .unwrap();
        let other_session_id = "other-session";
        let target_header = json!({
            "composerId": session_id,
            "name": "Before",
            "workspaceIdentifier": {
                "id": "workspace-1",
                "uri": {"fsPath": "/tmp/cursor-project"}
            },
            "independentHeaderField": "original"
        });
        let other_header = json!({
            "composerId": other_session_id,
            "name": "Other",
            "workspaceIdentifier": {
                "id": "workspace-2",
                "uri": {"fsPath": "/tmp/other-project"}
            }
        });
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            params![
                "composer.composerHeaders.migratedToTable",
                SqliteValue::Text("true".to_string())
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            params![
                "composer.composerHeaders",
                SqliteValue::Blob(
                    serde_json::to_vec(&json!({
                        "allComposers": [{"composerId": session_id, "name": "Stale legacy"}],
                        "sentinel": "must-not-change"
                    }))
                    .unwrap()
                )
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            params![
                "unrelated.config",
                SqliteValue::Text("unrelated-value".to_string())
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES (?1, 'workspace-1', 100, 300, 0, 0, 300, NULL, ?2)",
            params![session_id, target_header.to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES (?1, 'workspace-2', 400, 500, 0, 0, 500, NULL, ?2)",
            params![other_session_id, other_header.to_string()],
        )
        .unwrap();

        let target_composer = json!({
            "composerId": session_id,
            "name": "Before",
            "createdAt": 100,
            "lastUpdatedAt": 300,
            "fullConversationHeadersOnly": [
                {"bubbleId": "user-1", "type": 1},
                {"bubbleId": "assistant-1", "type": 2}
            ],
            "independentComposerField": "original"
        });
        let target_user_bubble = json!({
            "bubbleId": "user-1",
            "type": 1,
            "createdAt": "2026-07-16T01:00:00Z",
            "text": "hello"
        });
        let target_assistant_bubble = json!({
            "bubbleId": "assistant-1",
            "type": 2,
            "createdAt": "2026-07-16T01:00:01Z",
            "requestId": "request-1",
            "text": "world"
        });
        let other_composer = json!({
            "composerId": other_session_id,
            "name": "Other",
            "createdAt": 400,
            "lastUpdatedAt": 500
        });
        let rows = [
            (
                format!("composerData:{session_id}"),
                SqliteValue::Blob(serde_json::to_vec(&target_composer).unwrap()),
            ),
            (
                format!("bubbleId:{session_id}:user-1"),
                SqliteValue::Blob(serde_json::to_vec(&target_user_bubble).unwrap()),
            ),
            (
                format!("bubbleId:{session_id}:assistant-1"),
                SqliteValue::Text(target_assistant_bubble.to_string()),
            ),
            (
                format!("composerData:{other_session_id}"),
                SqliteValue::Text(other_composer.to_string()),
            ),
            (
                format!("bubbleId:{other_session_id}:other-1"),
                SqliteValue::Text(
                    json!({
                        "bubbleId": "other-1",
                        "type": 1,
                        "createdAt": "2026-07-16T02:00:00Z",
                        "text": "other"
                    })
                    .to_string(),
                ),
            ),
        ];
        for (key, value) in rows {
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .unwrap();
        }

        CurrentCursorFixture {
            target_header: current_header_row(&conn, session_id).unwrap(),
            target_disk_rows: current_session_disk_rows(&conn, session_id),
            unrelated_header: current_header_row(&conn, other_session_id).unwrap(),
            unrelated_disk_rows: current_session_disk_rows(&conn, other_session_id),
            item_rows: current_table_rows(&conn, "ItemTable"),
        }
    }

    fn current_header_row(conn: &Connection, session_id: &str) -> Option<CurrentHeaderRow> {
        conn.query_row(
            "SELECT composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
                    recency, checkpointAt, value
             FROM composerHeaders WHERE composerId = ?1",
            [session_id],
            |row| {
                Ok(CurrentHeaderRow {
                    composer_id: row.get(0)?,
                    workspace_id: row.get(1)?,
                    created_at: row.get(2)?,
                    last_updated_at: row.get(3)?,
                    is_archived: row.get(4)?,
                    is_subagent: row.get(5)?,
                    recency: row.get(6)?,
                    checkpoint_at: row.get(7)?,
                    value: row.get(8)?,
                })
            },
        )
        .optional()
        .unwrap()
    }

    fn current_table_rows(conn: &Connection, table: &str) -> Vec<CurrentDiskRow> {
        let mut stmt = conn
            .prepare(&format!("SELECT key, value FROM {table} ORDER BY key ASC"))
            .unwrap();
        stmt.query_map([], |row| {
            Ok(CurrentDiskRow {
                key: row.get(0)?,
                value: row.get(1)?,
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    }

    fn current_session_disk_rows(conn: &Connection, session_id: &str) -> Vec<CurrentDiskRow> {
        let composer_key = format!("composerData:{session_id}");
        let bubble_prefix = format!("bubbleId:{session_id}:");
        let mut stmt = conn
            .prepare(
                "SELECT key, value FROM cursorDiskKV
                 WHERE key = ?1 OR key LIKE ?2
                 ORDER BY key ASC",
            )
            .unwrap();
        stmt.query_map(params![composer_key, format!("{bubble_prefix}%")], |row| {
            Ok(CurrentDiskRow {
                key: row.get(0)?,
                value: row.get(1)?,
            })
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
    }

    fn current_disk_value(conn: &Connection, key: &str) -> Option<SqliteValue> {
        conn.query_row(
            "SELECT value FROM cursorDiskKV WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()
        .unwrap()
    }

    fn parse_stored_json(value: &SqliteValue) -> Value {
        match value {
            SqliteValue::Text(text) => serde_json::from_str(text).unwrap(),
            SqliteValue::Blob(bytes) => serde_json::from_slice(bytes).unwrap(),
            _ => panic!("expected TEXT or BLOB JSON"),
        }
    }

    fn update_current_json_value(
        conn: &Connection,
        table: &str,
        identity_column: &str,
        identity: &str,
        value: &Value,
    ) {
        let stored: SqliteValue = conn
            .query_row(
                &format!("SELECT value FROM {table} WHERE {identity_column} = ?1"),
                [identity],
                |row| row.get(0),
            )
            .unwrap();
        let updated = match stored {
            SqliteValue::Text(_) => SqliteValue::Text(value.to_string()),
            SqliteValue::Blob(_) => SqliteValue::Blob(serde_json::to_vec(value).unwrap()),
            _ => panic!("expected TEXT or BLOB JSON"),
        };
        conn.execute(
            &format!("UPDATE {table} SET value = ?1 WHERE {identity_column} = ?2"),
            params![updated, identity],
        )
        .unwrap();
    }

    #[test]
    fn cursor_current_export_writes_discoverable_header_data_and_bubbles() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-export-source";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
        let source = db::cursor_source_locator(session_id).unwrap();
        let canonical = CursorProvider.import_session(&source).unwrap().session;

        let exported = CursorProvider
            .export_session(&canonical, &dir.path().join("workspace"))
            .unwrap();
        let exported_id = exported.session_id;
        assert_ne!(exported_id, session_id);
        let scanned = CursorProvider.scan_sessions().unwrap();
        assert!(scanned
            .iter()
            .any(|session| session.session_id == exported_id));
        let imported = CursorProvider
            .import_session(&db::cursor_source_locator(&exported_id).unwrap())
            .unwrap();
        assert_eq!(imported.session.events.len(), canonical.events.len());

        let conn = Connection::open(&db_path).unwrap();
        let header = current_header_row(&conn, &exported_id).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(header.value.as_deref().unwrap()).unwrap()["composerId"],
            exported_id
        );
        assert!(current_disk_value(&conn, &format!("composerData:{exported_id}")).is_some());
        assert_eq!(current_session_disk_rows(&conn, &exported_id).len(), 3);
        assert_eq!(current_table_rows(&conn, "ItemTable"), fixture.item_rows);
    }

    #[test]
    fn cursor_current_delete_backup_restores_exact_target_rows_and_preserves_unrelated_state() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-delete-backup";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-delete-1",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        assert_eq!(backup.format, "cursor-session-backup-v2");
        let backup_conn =
            Connection::open(backup.backup_path.join("sqlite/cursor-session.db")).unwrap();
        let backup_tables = backup_conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(backup_tables, ["composerHeaders", "cursorDiskKV"]);
        drop(backup_conn);

        write::delete_session(session_id).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        assert!(current_header_row(&conn, session_id).is_none());
        assert!(current_session_disk_rows(&conn, session_id).is_empty());
        assert!(!CursorProvider
            .scan_sessions()
            .unwrap()
            .iter()
            .any(|session| session.session_id == session_id));
        conn.execute(
            "UPDATE composerHeaders SET recency = 777 WHERE composerId = 'other-session'",
            [],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();
        CursorProvider.restore_session_backup(&backup).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            current_header_row(&conn, session_id),
            Some(fixture.target_header)
        );
        assert_eq!(
            current_session_disk_rows(&conn, session_id),
            fixture.target_disk_rows
        );
        let mut current_other = current_header_row(&conn, "other-session").unwrap();
        let mut expected_other = fixture.unrelated_header;
        current_other.recency = Some(500);
        expected_other.recency = Some(500);
        assert_eq!(current_other, expected_other);
        assert_eq!(
            current_header_row(&conn, "other-session").unwrap().recency,
            Some(777)
        );
        assert_eq!(
            current_session_disk_rows(&conn, "other-session"),
            fixture.unrelated_disk_rows
        );
        assert_eq!(current_table_rows(&conn, "ItemTable"), fixture.item_rows);
    }

    #[test]
    fn cursor_current_rename_backup_restores_only_cursor_owned_name_fields() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-rename-backup";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
        let original_bubbles = fixture
            .target_disk_rows
            .iter()
            .filter(|row| row.key.starts_with("bubbleId:"))
            .cloned()
            .collect::<Vec<_>>();
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-rename-1",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        write::rename_session(session_id, "After").unwrap();
        let conn = Connection::open(&db_path).unwrap();
        let mut header_json: Value = serde_json::from_str(
            current_header_row(&conn, session_id)
                .unwrap()
                .value
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        let composer_key = format!("composerData:{session_id}");
        let mut composer_json =
            parse_stored_json(&current_disk_value(&conn, &composer_key).unwrap());
        assert_eq!(header_json["name"], "After");
        assert_eq!(composer_json["name"], "After");
        assert_eq!(
            CursorProvider
                .scan_sessions()
                .unwrap()
                .into_iter()
                .find(|session| session.session_id == session_id)
                .unwrap()
                .title,
            Some("After".to_string())
        );

        header_json["independentHeaderField"] = json!("changed independently");
        header_json["newHeaderField"] = json!(true);
        composer_json["independentComposerField"] = json!("changed independently");
        composer_json["newComposerField"] = json!(42);
        update_current_json_value(
            &conn,
            "composerHeaders",
            "composerId",
            session_id,
            &header_json,
        );
        update_current_json_value(&conn, "cursorDiskKV", "key", &composer_key, &composer_json);
        conn.execute(
            "UPDATE composerHeaders SET recency = 888 WHERE composerId = ?1",
            [session_id],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();
        CursorProvider.restore_session_backup(&backup).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        let header = current_header_row(&conn, session_id).unwrap();
        let header_json: Value = serde_json::from_str(header.value.as_deref().unwrap()).unwrap();
        let composer_json = parse_stored_json(&current_disk_value(&conn, &composer_key).unwrap());
        assert_eq!(header_json["name"], "Before");
        assert_eq!(
            header_json["independentHeaderField"],
            "changed independently"
        );
        assert_eq!(header_json["newHeaderField"], true);
        assert_eq!(header.recency, Some(888));
        assert_eq!(composer_json["name"], "Before");
        assert_eq!(
            composer_json["independentComposerField"],
            "changed independently"
        );
        assert_eq!(composer_json["newComposerField"], 42);
        assert_eq!(
            current_session_disk_rows(&conn, session_id)
                .into_iter()
                .filter(|row| row.key.starts_with("bubbleId:"))
                .collect::<Vec<_>>(),
            original_bubbles
        );
        assert_eq!(current_table_rows(&conn, "ItemTable"), fixture.item_rows);
    }

    #[test]
    fn cursor_backup_contract_and_capabilities_are_truthful() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-backup-contract";
        write_current_cursor_management_fixture(&db_path, session_id);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-contract-1",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        let capabilities = CursorProvider.capabilities();
        assert_eq!(capabilities.scan_strategy, ScanStrategy::Indexed);
        assert_eq!(capabilities.turn_quality, TurnQuality::Inferred);
        assert_eq!(capabilities.write_risk.level, WriteRiskLevel::High);
        assert!(capabilities.write_risk.sqlite);
        assert!(!capabilities.write_risk.multiple_files);
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert!(capabilities.activity_support.hook_events);
        assert!(capabilities.activity_support.runtime_endpoint);
        assert!(capabilities.activity_support.session_activity);
        assert_eq!(backup.mutation, ProviderSourceMutation::Delete);
        assert_eq!(backup.operation_id, "operation-contract-1");
        assert_eq!(backup.provider_session_id, session_id);
        assert_eq!(backup.source_path, db_path.canonicalize().unwrap());
        assert_eq!(backup.format, "cursor-session-backup-v2");
        assert_eq!(
            backup.mime_type,
            "application/vnd.memorph.cursor-session-backup"
        );
        let metadata: Value = serde_json::from_slice(
            &std::fs::read(backup.backup_path.join("metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["version"], 2);
        assert_eq!(metadata["sqlite_tables"].as_array().unwrap().len(), 2);
        assert_eq!(
            backup
                .restore_metadata
                .get("restore_mode")
                .and_then(Value::as_str),
            Some("cursor_session_restore")
        );
    }

    #[test]
    fn cursor_full_import_pages_keep_total_counts_and_project_only_page_events() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-pagination";
        let created_at = 1_700_000_000_000_i64;

        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(include_str!("fixtures/v3_11_19/schema.sql"))
            .unwrap();
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, NULL, ?6)",
            params![
                session_id,
                "workspace-pagination",
                created_at,
                created_at + 3_000,
                created_at + 3_000,
                json!({"composerId": session_id, "name": "Pagination"}).to_string(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                format!("composerData:{session_id}"),
                json!({
                    "composerId": session_id,
                    "name": "Pagination",
                    "createdAt": created_at,
                    "lastUpdatedAt": created_at + 3_000
                })
                .to_string(),
            ],
        )
        .unwrap();

        let bubbles = [
            ("a", 1, created_at, None, "first"),
            ("b", 2, created_at, Some("turn-1"), "second"),
            ("c", 1, created_at + 1_000, None, "third"),
            ("d", 2, created_at + 2_000, Some("turn-2"), "fourth"),
        ];
        for (bubble_id, bubble_type, bubble_created_at, request_id, text) in bubbles {
            let mut bubble = json!({
                "bubbleId": bubble_id,
                "type": bubble_type,
                "createdAt": chrono::DateTime::from_timestamp_millis(bubble_created_at)
                    .unwrap()
                    .to_rfc3339(),
                "text": text,
            });
            if let Some(request_id) = request_id {
                bubble["requestId"] = Value::String(request_id.to_string());
            }
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
                params![
                    format!("bubbleId:{session_id}:{bubble_id}"),
                    bubble.to_string(),
                ],
            )
            .unwrap();
        }
        drop(conn);

        let capabilities = CursorProvider.capabilities();
        assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);

        let locator = db::cursor_source_locator(session_id).unwrap();
        let full = CursorProvider
            .import_session_page(&locator, 0, None)
            .unwrap();
        assert_eq!(full.imported.session.events.len(), 4);
        assert_eq!(full.imported.session.events[0].id, "a");
        assert_eq!(full.imported.session.events[1].id, "b");
        assert_eq!(full.event_count, 4);
        assert_eq!(full.message_count, 4);
        assert_eq!(full.turn_count, Some(full.turns.len()));

        let page = CursorProvider
            .import_session_page(&locator, 1, Some(2))
            .unwrap();
        assert_eq!(page.imported.session.events.len(), 2);
        assert_eq!(
            page.imported
                .session
                .events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert_eq!(page.event_count, full.event_count);
        assert_eq!(page.message_count, full.message_count);
        assert_eq!(page.turn_count, full.turn_count);
        assert!(page.turns.iter().all(|turn| {
            turn.source_range
                .start_cursor
                .as_deref()
                .is_some_and(|cursor| ["b", "c"].contains(&cursor))
                && turn
                    .source_range
                    .end_cursor
                    .as_deref()
                    .is_some_and(|cursor| ["b", "c"].contains(&cursor))
        }));

        let empty = CursorProvider
            .import_session_page(&locator, 0, Some(0))
            .unwrap();
        assert!(empty.imported.session.events.is_empty());
        assert!(empty.turns.is_empty());
        assert_eq!(empty.event_count, full.event_count);
        assert_eq!(empty.message_count, full.message_count);
        assert_eq!(empty.turn_count, full.turn_count);

        let beyond_end = CursorProvider
            .import_session_page(&locator, full.event_count + 10, Some(5))
            .unwrap();
        assert!(beyond_end.imported.session.events.is_empty());
        assert!(beyond_end.turns.is_empty());
        assert_eq!(beyond_end.event_count, full.event_count);
        assert_eq!(beyond_end.message_count, full.message_count);
        assert_eq!(beyond_end.turn_count, full.turn_count);
    }

    #[test]
    fn cursor_current_index_and_detail_are_idempotent_source_backed_and_bodyless() -> Result<()> {
        let dir = tempdir()?;
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home)?;
        let _home_guard = TestConfigHomeGuard::new(&home);
        let db_path = dir.path().join("state.vscdb");
        let _db_guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-bodyless";
        let created_at = 1_700_000_000_000_i64;

        let conn = Connection::open(&db_path)?;
        conn.execute_batch(include_str!("fixtures/v3_11_19/schema.sql"))?;
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, NULL, ?6)",
            params![
                session_id,
                "workspace-bodyless",
                created_at,
                created_at + 1_000,
                created_at + 1_000,
                json!({"composerId": session_id, "name": "Bodyless"}).to_string(),
            ],
        )?;
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                format!("composerData:{session_id}"),
                json!({
                    "composerId": session_id,
                    "name": "Bodyless",
                    "createdAt": created_at,
                    "lastUpdatedAt": created_at + 1_000
                })
                .to_string(),
            ],
        )?;
        for (bubble_id, bubble_type, timestamp, text) in [
            ("user-1", 1, "2023-11-14T22:13:20Z", "hello"),
            ("assistant-1", 2, "2023-11-14T22:13:21Z", "answer"),
        ] {
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
                params![
                    format!("bubbleId:{session_id}:{bubble_id}"),
                    json!({
                        "bubbleId": bubble_id,
                        "type": bubble_type,
                        "createdAt": timestamp,
                        "text": text
                    })
                    .to_string(),
                ],
            )?;
        }
        drop(conn);

        let capabilities = CursorProvider.capabilities();
        assert_eq!(capabilities.storage_shape, StorageShape::Sqlite);
        let summary = CursorProvider
            .scan_sessions()?
            .into_iter()
            .find(|summary| summary.session_id == session_id)
            .expect("current Cursor session summary");
        let source_path = summary.source_path.clone().expect("Cursor locator");
        let fingerprint = CursorProvider
            .session_source_fingerprint(&source_path)?
            .expect("current Cursor source");
        let full = CursorProvider.import_session_page(&source_path, 0, None)?;

        let mut conn = local_store::open_database()?;
        let first = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(PROVIDER_ID, &summary, capabilities, &fingerprint)?;
        let second = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(PROVIDER_ID, &summary, capabilities, &fingerprint)?;
        assert_eq!(first, second);

        let counts: (i64, i64, i64, i64) = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'cursor'),
                (SELECT COUNT(*) FROM sessions WHERE provider_id = 'cursor'),
                (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'cursor'),
                (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'cursor')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(counts, (1, 1, 1, 2));

        let indexed_source: (String, String, String) = conn.query_row(
            "SELECT source_path, storage_shape, source_cursor
             FROM session_sources WHERE id = ?1",
            [&first.source_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_eq!(indexed_source.0, source_path);
        assert_eq!(indexed_source.1, "sqlite");
        assert_eq!(indexed_source.2, fingerprint.value);

        let snapshot_json: String = conn.query_row(
            "SELECT snapshot_json FROM session_snapshots WHERE session_id = ?1",
            [&first.canonical_session_id],
            |row| row.get(0),
        )?;
        let snapshot_json: Value = serde_json::from_str(&snapshot_json)?;
        assert_eq!(
            snapshot_json
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            std::collections::BTreeSet::from(["index_version", "source_fingerprint"]),
        );
        let body_table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(body_table_count, 0);
        drop(conn);

        let detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        assert!(detail.events.is_empty());
        assert!(detail.turns.is_empty());
        assert_eq!(detail.event_count, full.event_count);
        assert_eq!(detail.message_count, full.message_count);
        assert!(!detail.stale);
        assert_eq!(detail.source_path.as_deref(), Some(source_path.as_str()));
        assert_eq!(
            detail.projection_report.as_ref().unwrap().id,
            format!("source-read:{PROVIDER_ID}:{session_id}")
        );

        let conn = local_store::open_database()?;
        let cached_counts: (i64, i64, i64, i64) = conn.query_row(
            "SELECT event_count, message_count, turn_count, counts_complete
             FROM session_snapshots WHERE session_id = ?1",
            [&first.canonical_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(cached_counts.0, full.event_count as i64);
        assert_eq!(cached_counts.1, full.message_count as i64);
        assert_eq!(cached_counts.2, full.turn_count.unwrap() as i64);
        assert_eq!(cached_counts.3, 1);
        drop(conn);

        let conn = Connection::open(&db_path)?;
        conn.execute(
            "UPDATE cursorDiskKV SET value = ?1 WHERE key = ?2",
            params![
                json!({
                    "bubbleId": "assistant-1",
                    "type": 2,
                    "createdAt": "2023-11-14T22:13:21Z",
                    "text": "changed"
                })
                .to_string(),
                format!("bubbleId:{session_id}:assistant-1"),
            ],
        )?;
        drop(conn);
        let stale_detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        assert!(stale_detail.stale);

        std::fs::remove_file(&db_path)?;
        let error = crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(1))
            .unwrap_err();
        assert!(format!("{error:#}").contains("Session source is missing"));
        Ok(())
    }

    #[test]
    fn cursor_current_bootstrap_stale_and_system_sync_are_incremental_and_bodyless() -> Result<()> {
        let dir = tempdir()?;
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home)?;
        let _home_guard = TestConfigHomeGuard::new(&home);
        let db_path = dir.path().join("state.vscdb");
        let _db_guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-bootstrap";
        let created_at = 1_700_000_000_000_i64;

        let conn = Connection::open(&db_path)?;
        conn.execute_batch(include_str!("fixtures/v3_11_19/schema.sql"))?;
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES (?1, ?2, ?3, ?4, 0, 0, ?5, NULL, ?6)",
            params![
                session_id,
                "workspace-bootstrap",
                created_at,
                created_at + 1_000,
                created_at + 1_000,
                json!({
                    "composerId": session_id,
                    "name": "Cursor bootstrap",
                    "workspaceIdentifier": {
                        "id": "workspace-bootstrap",
                        "uri": {"fsPath": "/workspace/cursor-bootstrap"}
                    }
                })
                .to_string(),
            ],
        )?;
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                format!("composerData:{session_id}"),
                json!({
                    "composerId": session_id,
                    "name": "Cursor bootstrap",
                    "createdAt": created_at,
                    "lastUpdatedAt": created_at + 1_000,
                    "workspaceIdentifier": {
                        "id": "workspace-bootstrap",
                        "uri": {"fsPath": "/workspace/cursor-bootstrap"}
                    }
                })
                .to_string(),
            ],
        )?;
        for (bubble_id, bubble_type, timestamp, text) in [
            ("user-1", 1, "2023-11-14T22:13:20Z", "hello"),
            ("assistant-1", 2, "2023-11-14T22:13:21Z", "answer"),
        ] {
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
                params![
                    format!("bubbleId:{session_id}:{bubble_id}"),
                    json!({
                        "bubbleId": bubble_id,
                        "type": bubble_type,
                        "createdAt": timestamp,
                        "text": text
                    })
                    .to_string(),
                ],
            )?;
        }
        drop(conn);

        let first = crate::core::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::Cli,
        )?;
        assert_eq!(first.scanned_providers, 1);
        assert_eq!(first.discovered_sessions, 1);
        assert_eq!(first.projected_sessions, 1);
        assert_eq!(first.unchanged_sessions, 0);
        assert!(first.failures.is_empty());

        let detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        assert!(detail.events.is_empty());
        assert!(detail.turns.is_empty());
        assert!(!detail.stale);

        let conn = local_store::open_database()?;
        let initial: (String, String, i64, i64, i64, i64) = conn.query_row(
            "SELECT ss.title, ss.workspace_dir, ss.counts_complete, ss.stale,
                    src.scan_generation,
                    (SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'table'
                       AND name IN ('session_turns', 'session_events', 'session_event_blocks'))
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'cursor' AND s.provider_session_id = ?1",
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
        )?;
        let initial_fingerprint: String = conn.query_row(
            "SELECT source_cursor FROM session_sources
             WHERE provider_id = 'cursor' AND provider_session_id = ?1",
            [session_id],
            |row| row.get(0),
        )?;
        assert_eq!(initial.0, "Cursor bootstrap");
        assert_eq!(initial.1, "/workspace/cursor-bootstrap");
        assert_eq!(initial.2, 1);
        assert_eq!(initial.3, 0);
        assert_eq!(initial.4, 1);
        assert_eq!(initial.5, 0);
        drop(conn);

        let unchanged = crate::core::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(unchanged.scanned_providers, 1);
        assert_eq!(unchanged.discovered_sessions, 1);
        assert_eq!(unchanged.projected_sessions, 0);
        assert_eq!(unchanged.unchanged_sessions, 1);
        assert!(unchanged.failures.is_empty());

        let conn = local_store::open_database()?;
        let unchanged_state: (i64, i64) = conn.query_row(
            "SELECT src.scan_generation, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'cursor' AND s.provider_session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(unchanged_state, (1, 1));
        drop(conn);

        let conn = Connection::open(&db_path)?;
        conn.execute(
            "UPDATE cursorDiskKV SET value = ?1 WHERE key = ?2",
            params![
                json!({
                    "bubbleId": "assistant-1",
                    "type": 2,
                    "createdAt": "2023-11-14T22:13:21Z",
                    "text": "changed answer"
                })
                .to_string(),
                format!("bubbleId:{session_id}:assistant-1"),
            ],
        )?;
        drop(conn);

        let stale = crate::core::refresh_projected_session_staleness(
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(stale.checked_sources, 1);
        assert_eq!(stale.fresh_snapshots, 0);
        assert_eq!(stale.stale_snapshots, 1);
        assert_eq!(stale.missing_sources, 0);

        let refreshed = crate::core::reproject_stale_sessions(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(refreshed.candidate_snapshots, 1);
        assert_eq!(refreshed.reprojected_snapshots, 1);
        assert_eq!(refreshed.missing_sources, 0);
        assert!(refreshed.failures.is_empty());

        let conn = local_store::open_database()?;
        let after_bubble: (String, i64, i64) = conn.query_row(
            "SELECT src.source_cursor, ss.stale, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'cursor' AND s.provider_session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        assert_ne!(after_bubble.0, initial_fingerprint);
        assert_eq!(after_bubble.1, 0);
        assert_eq!(after_bubble.2, 0);
        drop(conn);

        let detail =
            crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        assert!(!detail.stale);

        let conn = Connection::open(&db_path)?;
        conn.execute(
            "UPDATE composerHeaders SET recency = ?2, value = ?3 WHERE composerId = ?1",
            params![
                session_id,
                created_at + 2_000,
                json!({
                    "composerId": session_id,
                    "name": "Cursor bootstrap updated",
                    "workspaceIdentifier": {
                        "id": "workspace-bootstrap",
                        "uri": {"fsPath": "/workspace/cursor-bootstrap"}
                    }
                })
                .to_string(),
            ],
        )?;
        drop(conn);

        let header_sync = crate::core::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(header_sync.projected_sessions, 1);
        assert_eq!(header_sync.unchanged_sessions, 0);
        assert!(header_sync.failures.is_empty());

        let conn = local_store::open_database()?;
        let after_header: (String, i64, String, i64) = conn.query_row(
            "SELECT ss.title, ss.last_active_at_ms, src.source_cursor, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'cursor' AND s.provider_session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        assert_eq!(after_header.0, "Cursor bootstrap updated");
        assert_eq!(after_header.1, created_at + 2_000);
        assert_ne!(after_header.2, after_bubble.0);
        assert_eq!(after_header.3, 0);
        drop(conn);

        crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
        let conn = Connection::open(&db_path)?;
        conn.execute(
            "UPDATE cursorDiskKV SET value = ?1 WHERE key = ?2",
            params![
                json!({
                    "composerId": session_id,
                    "name": "Cursor bootstrap",
                    "text": "composer data changed",
                    "createdAt": created_at,
                    "lastUpdatedAt": created_at + 3_000,
                    "workspaceIdentifier": {
                        "id": "workspace-bootstrap",
                        "uri": {"fsPath": "/workspace/cursor-bootstrap"}
                    }
                })
                .to_string(),
                format!("composerData:{session_id}"),
            ],
        )?;
        drop(conn);

        let composer_sync = crate::core::bootstrap_session_projections(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(composer_sync.projected_sessions, 1);
        assert_eq!(composer_sync.unchanged_sessions, 0);
        assert!(composer_sync.failures.is_empty());

        let conn = local_store::open_database()?;
        let after_composer: (String, i64) = conn.query_row(
            "SELECT src.source_cursor, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'cursor' AND s.provider_session_id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_ne!(after_composer.0, after_header.2);
        assert_eq!(after_composer.1, 0);
        drop(conn);

        std::fs::remove_file(&db_path)?;
        let missing = crate::core::refresh_projected_session_staleness(
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(missing.checked_sources, 0);
        assert_eq!(missing.fresh_snapshots, 0);
        assert_eq!(missing.missing_sources, 1);
        assert_eq!(missing.stale_snapshots, 1);

        let missing_reprojection = crate::core::reproject_stale_sessions(
            Some(PROVIDER_ID),
            crate::storage::activity_store::ActivityActor::System,
        )?;
        assert_eq!(missing_reprojection.candidate_snapshots, 1);
        assert_eq!(missing_reprojection.reprojected_snapshots, 0);
        assert_eq!(missing_reprojection.missing_sources, 1);

        let groups = crate::core::list_sessions(&crate::core::SessionListParams {
            all: true,
            providers: vec![PROVIDER_ID.to_string()],
            cwd: None,
            include_message_counts: true,
            limit: None,
            offset: None,
            sort: crate::core::SessionListSort::Recent,
            hook_filter: crate::core::SessionHookFilter::All,
        })?;
        let session = groups
            .iter()
            .flat_map(|group| &group.sessions)
            .find(|session| session.session_id == session_id)
            .unwrap();
        assert!(session.stale);

        let error = crate::core::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(1))
            .unwrap_err();
        assert!(format!("{error:#}").contains("Session source is missing"));

        let conn = local_store::open_database()?;
        let system_scan_activities: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_activity
             WHERE actor = 'system' AND operation_kind = 'scan' AND status != 'running'",
            [],
            |row| row.get(0),
        )?;
        assert!(system_scan_activities >= 7);
        let body_table_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(body_table_count, 0);
        Ok(())
    }

    #[test]
    fn backup_registration_failure_prevents_cursor_provider_write() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-registration-failure";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
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
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            current_header_row(&conn, session_id),
            Some(fixture.target_header)
        );
        assert_eq!(
            current_session_disk_rows(&conn, session_id),
            fixture.target_disk_rows
        );
        assert_eq!(current_table_rows(&conn, "ItemTable"), fixture.item_rows);
        assert!(backup_root
            .join(PROVIDER_ID)
            .join("operation-registration-failure")
            .exists());
    }

    #[test]
    fn partial_cursor_delete_failure_restores_registered_backup() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-partial-delete";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
        let backup_root = dir.path().join("backups");
        let mut artifact_conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();
        write::set_test_cursor_mutation_failure(Some(ProviderSourceMutation::Delete));

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-partial-delete".to_string()],
            &backup_root,
            &mut artifact_conn,
        );

        let error = results.into_iter().next().unwrap().unwrap_err();
        assert!(error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            current_header_row(&conn, session_id),
            Some(fixture.target_header)
        );
        assert_eq!(
            current_session_disk_rows(&conn, session_id),
            fixture.target_disk_rows
        );
        assert_eq!(current_table_rows(&conn, "ItemTable"), fixture.item_rows);
    }

    #[test]
    fn partial_cursor_rename_failure_restores_registered_backup() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-partial-rename";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
        let backup_root = dir.path().join("backups");
        let mut artifact_conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();
        write::set_test_cursor_mutation_failure(Some(ProviderSourceMutation::Rename));

        let error = session_management::rename_session(
            PROVIDER_ID,
            session_id,
            "After",
            "operation-partial-rename",
            &backup_root,
            &mut artifact_conn,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            current_header_row(&conn, session_id),
            Some(fixture.target_header)
        );
        assert_eq!(
            current_session_disk_rows(&conn, session_id),
            fixture.target_disk_rows
        );
        assert_eq!(current_table_rows(&conn, "ItemTable"), fixture.item_rows);
    }

    #[test]
    fn cursor_backup_rejects_non_current_or_non_primary_header_schema_before_mutation() {
        let dir = tempdir().unwrap();
        let backup_root = dir.path().join("backups");
        {
            let db_path = dir.path().join("missing-header.vscdb");
            let guard = use_test_cursor_db(db_path.clone());
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE ItemTable (
                     key TEXT UNIQUE ON CONFLICT REPLACE,
                     value BLOB
                 );
                 CREATE TABLE cursorDiskKV (
                     key TEXT UNIQUE ON CONFLICT REPLACE,
                     value BLOB
                 );",
            )
            .unwrap();
            drop(conn);
            let error = CursorProvider
                .create_session_backup(
                    ProviderSourceMutation::Delete,
                    "operation-missing-header",
                    "missing-header",
                    &backup_root,
                )
                .unwrap_err();
            assert!(error.to_string().contains("composerHeaders"));
            drop(guard);
        }
        {
            let db_path = dir.path().join("non-primary-header.vscdb");
            let guard = use_test_cursor_db(db_path.clone());
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE ItemTable (
                     key TEXT UNIQUE ON CONFLICT REPLACE,
                     value BLOB
                 );
                 CREATE TABLE cursorDiskKV (
                     key TEXT UNIQUE ON CONFLICT REPLACE,
                     value BLOB
                 );
                 CREATE TABLE composerHeaders (
                     composerId TEXT,
                     workspaceId TEXT,
                     createdAt INTEGER,
                     lastUpdatedAt INTEGER,
                     isArchived INTEGER,
                     isSubagent INTEGER,
                     recency INTEGER,
                     checkpointAt INTEGER,
                     value TEXT
                 );",
            )
            .unwrap();
            drop(conn);
            let error = CursorProvider
                .create_session_backup(
                    ProviderSourceMutation::Rename,
                    "operation-non-primary-header",
                    "non-primary-header",
                    &backup_root,
                )
                .unwrap_err();
            assert!(error
                .to_string()
                .contains("must use composerId as its primary key"));
            drop(guard);
        }
    }

    #[test]
    fn cursor_restore_rejects_tampered_current_backup_selection() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-tampered-backup";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-tampered-backup",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        let backup_db = backup.backup_path.join("sqlite/cursor-session.db");
        let conn = Connection::open(&backup_db).unwrap();
        conn.execute(
            "UPDATE composerHeaders
             SET composerId = 'outside-session', value = ?1",
            [json!({"composerId": "outside-session", "name": "Outside"}).to_string()],
        )
        .unwrap();
        drop(conn);

        let error = CursorProvider.restore_session_backup(&backup).unwrap_err();
        assert!(error
            .to_string()
            .contains("composer header outside the target session"));
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            current_header_row(&conn, session_id),
            Some(fixture.target_header)
        );
        assert_eq!(
            current_session_disk_rows(&conn, session_id),
            fixture.target_disk_rows
        );
    }

    #[test]
    fn cursor_restore_rejects_tampered_backup_row_count() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-tampered-row-count";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-tampered-row-count",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        let metadata_path = backup.backup_path.join("metadata.json");
        let mut metadata: Value =
            serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
        let disk_manifest = metadata["sqlite_tables"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|manifest| manifest["table"] == "cursorDiskKV")
            .unwrap();
        disk_manifest["row_count"] = json!(2);
        std::fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();

        let error = CursorProvider.restore_session_backup(&backup).unwrap_err();
        assert!(error
            .to_string()
            .contains("backup row count does not match manifest for cursorDiskKV"));
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(
            current_header_row(&conn, session_id),
            Some(fixture.target_header)
        );
        assert_eq!(
            current_session_disk_rows(&conn, session_id),
            fixture.target_disk_rows
        );
    }

    #[test]
    fn rename_restore_does_not_recreate_concurrently_deleted_current_rows() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-rename-concurrent-delete";
        let fixture = write_current_cursor_management_fixture(&db_path, session_id);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-rename-concurrent-delete",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        write::rename_session(session_id, "After").unwrap();

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "DELETE FROM composerHeaders WHERE composerId = ?1",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM cursorDiskKV WHERE key = ?1",
            [format!("composerData:{session_id}")],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        assert!(current_header_row(&conn, session_id).is_none());
        assert!(current_disk_value(&conn, &format!("composerData:{session_id}")).is_none());
        assert_eq!(
            current_session_disk_rows(&conn, session_id)
                .into_iter()
                .filter(|row| row.key.starts_with("bubbleId:"))
                .collect::<Vec<_>>(),
            fixture
                .target_disk_rows
                .into_iter()
                .filter(|row| row.key.starts_with("bubbleId:"))
                .collect::<Vec<_>>()
        );
        assert_eq!(current_table_rows(&conn, "ItemTable"), fixture.item_rows);
    }

    #[test]
    fn core_cursor_mutations_register_backups_restore_failures_and_finish_activity() {
        let dir = tempdir().unwrap();
        let home = dir.path().join("home");
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let _home_guard = TestConfigHomeGuard::new(&home);
        let session_id = "cursor-core-management";
        write_current_cursor_management_fixture(&db_path, session_id);

        let renamed = crate::core::rename_session(
            PROVIDER_ID,
            session_id,
            "Renamed through core",
            ActivityActor::Cli,
        )
        .unwrap();
        assert!(renamed.native_updated);
        let conn = Connection::open(&db_path).unwrap();
        let renamed_header = current_header_row(&conn, session_id).unwrap();
        let renamed_disk_rows = current_session_disk_rows(&conn, session_id);
        assert_eq!(
            serde_json::from_str::<Value>(renamed_header.value.as_deref().unwrap()).unwrap()
                ["name"],
            "Renamed through core"
        );
        drop(conn);

        let rename_activity = crate::core::management::list_management_activity(&ActivityQuery {
            session_id: Some(session_id.to_string()),
            provider_id: Some(PROVIDER_ID.to_string()),
            operation_kind: Some(ActivityOperationKind::Rename),
            ..ActivityQuery::default()
        })
        .unwrap();
        assert_eq!(rename_activity.len(), 1);
        assert_eq!(rename_activity[0].status, ActivityStatus::Success);
        assert!(rename_activity[0].finished_at_ms.is_some());

        write::set_test_cursor_mutation_failure(Some(ProviderSourceMutation::Delete));
        let error =
            crate::core::delete_session(PROVIDER_ID, session_id, ActivityActor::Cli).unwrap_err();
        assert!(
            format!("{error:#}").contains("Provider source was restored from registered backup")
        );
        let conn = Connection::open(&db_path).unwrap();
        assert_eq!(current_header_row(&conn, session_id), Some(renamed_header));
        assert_eq!(
            current_session_disk_rows(&conn, session_id),
            renamed_disk_rows
        );
        drop(conn);

        let delete_activity = crate::core::management::list_management_activity(&ActivityQuery {
            session_id: Some(session_id.to_string()),
            provider_id: Some(PROVIDER_ID.to_string()),
            operation_kind: Some(ActivityOperationKind::Delete),
            ..ActivityQuery::default()
        })
        .unwrap();
        assert_eq!(delete_activity.len(), 1);
        assert_eq!(delete_activity[0].status, ActivityStatus::Failed);
        assert!(delete_activity[0].finished_at_ms.is_some());
        assert!(delete_activity[0].error.is_some());

        let backups = session_management::list_registered_backups(BackupQuery {
            provider_id: Some(PROVIDER_ID.to_string()),
            provider_session_id: Some(session_id.to_string()),
            ..BackupQuery::default()
        })
        .unwrap();
        assert_eq!(backups.len(), 2);
        assert!(backups
            .iter()
            .all(|backup| backup.verification.status == ArtifactVerificationStatus::Verified));
        let delete_backup = backups
            .iter()
            .find(|backup| backup.entry.backup.metadata["mutation"] == "delete")
            .expect("delete backup should be registered");
        let restore = session_management::restore_registered_backup(
            &delete_backup.entry.backup.id,
            ActivityActor::Cli,
        )
        .unwrap();
        assert_eq!(restore.status, BackupRestoreStatus::Success);
        assert!(restore.finished_at_ms.is_some());
        let repeated = session_management::restore_registered_backup(
            &delete_backup.entry.backup.id,
            ActivityActor::Cli,
        )
        .unwrap();
        assert_eq!(repeated.status, BackupRestoreStatus::Success);

        let activities = crate::core::management::list_management_activity(&ActivityQuery {
            session_id: Some(session_id.to_string()),
            provider_id: Some(PROVIDER_ID.to_string()),
            ..ActivityQuery::default()
        })
        .unwrap();
        assert_eq!(activities.len(), 2);
        assert!(activities
            .iter()
            .all(|activity| activity.status != ActivityStatus::Running
                && activity.finished_at_ms.is_some()));
    }
}
