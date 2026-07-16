pub mod adapter;
mod backup;
mod db;
pub mod hook;
mod load;
mod scan;
mod write;

use crate::canonical::{CanonicalSession, ExportedSession, ImportedSession};
use crate::provider::{
    canonical_export_result, PageStrategy, Provider, ProviderBackupSupport, ProviderCapabilities,
    ProviderSessionBackup, ProviderSessionSummary, ProviderSourceMutation, StorageShape,
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
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Sqlite,
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
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
    use crate::{core::session_management, storage::local_store};
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
    struct StoredCursorRow {
        value: SqliteValue,
        owner: String,
        revision: i64,
    }

    struct NativeCursorFixture {
        target_rows: Vec<(String, StoredCursorRow)>,
    }

    fn write_native_cursor_fixture(
        db_path: &Path,
        session_id: &str,
        include_index: bool,
    ) -> NativeCursorFixture {
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE cursorDiskKV (
                key TEXT UNIQUE ON CONFLICT REPLACE,
                value BLOB,
                owner TEXT NOT NULL,
                revision INTEGER NOT NULL
            );
            CREATE TABLE ItemTable (
                key TEXT UNIQUE ON CONFLICT REPLACE,
                value BLOB,
                owner TEXT NOT NULL,
                revision INTEGER NOT NULL
            );
            ",
        )
        .unwrap();

        let composer_key = format!("composerData:{session_id}");
        let bubble_blob_key = format!("bubbleId:{session_id}:bubble-blob");
        let bubble_text_key = format!("bubbleId:{session_id}:bubble-text");
        let rows = vec![
            (
                composer_key,
                SqliteValue::Text(
                    serde_json::to_string(&json!({
                        "composerId": session_id,
                        "status": "completed",
                        "text": "Before",
                        "name": "Before",
                        "workspaceIdentifier": {
                            "id": "workspace-1",
                            "uri": {"fsPath": "/tmp/cursor-project"}
                        },
                        "createdAt": 100,
                        "isAgentic": true,
                        "independentComposerField": "original"
                    }))
                    .unwrap(),
                ),
                "target-composer",
                10,
            ),
            (
                bubble_blob_key,
                SqliteValue::Blob(
                    serde_json::to_vec(&json!({
                        "bubbleId": "bubble-blob",
                        "type": 1,
                        "text": "blob message"
                    }))
                    .unwrap(),
                ),
                "target-bubble-blob",
                11,
            ),
            (
                bubble_text_key,
                SqliteValue::Text(
                    serde_json::to_string(&json!({
                        "bubbleId": "bubble-text",
                        "type": 2,
                        "text": "text message"
                    }))
                    .unwrap(),
                ),
                "target-bubble-text",
                12,
            ),
            (
                "composerData:other-session".to_string(),
                SqliteValue::Blob(
                    serde_json::to_vec(&json!({
                        "composerId": "other-session",
                        "name": "Other"
                    }))
                    .unwrap(),
                ),
                "unrelated-composer",
                20,
            ),
        ];
        for (key, value, owner, revision) in &rows {
            conn.execute(
                "INSERT INTO cursorDiskKV (key, value, owner, revision)
                 VALUES (?1, ?2, ?3, ?4)",
                params![key, value, owner, revision],
            )
            .unwrap();
        }

        if include_index {
            let index = json!({
                "allComposers": [
                    {
                        "composerId": session_id,
                        "name": "Before",
                        "subtitle": "target original",
                        "targetOnly": true
                    },
                    {
                        "composerId": "other-session",
                        "name": "Other",
                        "subtitle": "other original"
                    }
                ],
                "indexGeneration": "original"
            });
            conn.execute(
                "INSERT INTO ItemTable (key, value, owner, revision)
                 VALUES ('composer.composerHeaders', ?1, 'shared-index', 30)",
                [SqliteValue::Blob(serde_json::to_vec(&index).unwrap())],
            )
            .unwrap();
        }

        let target_rows = rows
            .into_iter()
            .take(3)
            .map(|(key, value, owner, revision)| {
                (
                    key,
                    StoredCursorRow {
                        value,
                        owner: owner.to_string(),
                        revision,
                    },
                )
            })
            .collect();
        NativeCursorFixture { target_rows }
    }

    fn write_current_cursor_rename_fixture(db_path: &Path, session_id: &str) {
        let conn = Connection::open(db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE cursorDiskKV (
                key TEXT UNIQUE ON CONFLICT REPLACE,
                value BLOB
            );
            CREATE TABLE ItemTable (
                key TEXT UNIQUE ON CONFLICT REPLACE,
                value BLOB
            );
            CREATE TABLE composerHeaders (
                composerId TEXT PRIMARY KEY,
                workspaceId TEXT,
                createdAt INTEGER,
                lastUpdatedAt INTEGER,
                isArchived INTEGER,
                isSubagent INTEGER,
                recency INTEGER,
                checkpointAt INTEGER,
                value TEXT
            );
            ",
        )
        .unwrap();

        let header = json!({
            "composerId": session_id,
            "name": "Before",
            "workspaceIdentifier": {
                "id": "workspace-1",
                "uri": {"fsPath": "/tmp/cursor-project"}
            }
        });
        conn.execute(
            "INSERT INTO composerHeaders
             (composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent,
              recency, checkpointAt, value)
             VALUES (?1, 'workspace-1', 100, 100, 0, 0, 100, NULL, ?2)",
            params![session_id, serde_json::to_string(&header).unwrap()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO cursorDiskKV (key, value) VALUES (?1, ?2)",
            params![
                format!("composerData:{session_id}"),
                serde_json::to_vec(&json!({
                    "composerId": session_id,
                    "name": "Before",
                    "createdAt": 100
                }))
                .unwrap(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES ('composer.composerHeaders', ?1)",
            [serde_json::to_vec(&json!({
                "allComposers": [{"composerId": session_id, "name": "Before"}],
                "indexGeneration": "original"
            }))
            .unwrap()],
        )
        .unwrap();
    }

    fn stored_row(conn: &Connection, table: &str, key: &str) -> Option<StoredCursorRow> {
        conn.query_row(
            &format!("SELECT value, owner, revision FROM {table} WHERE key = ?1"),
            [key],
            |row| {
                Ok(StoredCursorRow {
                    value: row.get(0)?,
                    owner: row.get(1)?,
                    revision: row.get(2)?,
                })
            },
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

    fn write_json_value(conn: &Connection, table: &str, key: &str, value: &Value, as_text: bool) {
        let stored = if as_text {
            SqliteValue::Text(serde_json::to_string(value).unwrap())
        } else {
            SqliteValue::Blob(serde_json::to_vec(value).unwrap())
        };
        conn.execute(
            &format!("UPDATE {table} SET value = ?1 WHERE key = ?2"),
            params![stored, key],
        )
        .unwrap();
    }

    fn composer_index(conn: &Connection) -> (Value, String, i64, &'static str) {
        let row = stored_row(conn, "ItemTable", "composer.composerHeaders").unwrap();
        let storage = match row.value {
            SqliteValue::Text(_) => "text",
            SqliteValue::Blob(_) => "blob",
            _ => "other",
        };
        (
            parse_stored_json(&row.value),
            row.owner,
            row.revision,
            storage,
        )
    }

    fn target_index_entry<'a>(index: &'a Value, session_id: &str) -> &'a Value {
        index["allComposers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["composerId"] == session_id)
            .unwrap()
    }

    #[test]
    fn delete_backup_restores_exact_cursor_rows_and_preserves_shared_state() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-delete-backup";
        let fixture = write_native_cursor_fixture(&db_path, session_id, true);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-delete-1",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        write::delete_session(session_id).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        for (key, _) in &fixture.target_rows {
            assert!(stored_row(&conn, "cursorDiskKV", key).is_none());
        }
        conn.execute(
            "UPDATE cursorDiskKV
             SET owner = 'unrelated-changed', revision = 99
             WHERE key = 'composerData:other-session'",
            [],
        )
        .unwrap();
        let mut index = composer_index(&conn).0;
        let entries = index["allComposers"].as_array_mut().unwrap();
        entries[0]["name"] = json!("Other changed");
        entries.push(json!({
            "composerId": "new-session",
            "name": "New during restore window"
        }));
        index["indexGeneration"] = json!("current");
        write_json_value(&conn, "ItemTable", "composer.composerHeaders", &index, true);
        conn.execute(
            "UPDATE ItemTable
             SET owner = 'shared-index-current', revision = 77
             WHERE key = 'composer.composerHeaders'",
            [],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();
        CursorProvider.restore_session_backup(&backup).unwrap();

        let conn = Connection::open(&db_path).unwrap();
        for (key, original) in &fixture.target_rows {
            assert_eq!(
                stored_row(&conn, "cursorDiskKV", key).as_ref(),
                Some(original)
            );
        }
        let unrelated = stored_row(&conn, "cursorDiskKV", "composerData:other-session").unwrap();
        assert_eq!(unrelated.owner, "unrelated-changed");
        assert_eq!(unrelated.revision, 99);

        let (restored_index, owner, revision, storage) = composer_index(&conn);
        assert_eq!(
            target_index_entry(&restored_index, session_id),
            &json!({
                "composerId": session_id,
                "name": "Before",
                "subtitle": "target original",
                "targetOnly": true
            })
        );
        assert_eq!(
            target_index_entry(&restored_index, "other-session")["name"],
            "Other changed"
        );
        assert_eq!(
            target_index_entry(&restored_index, "new-session")["name"],
            "New during restore window"
        );
        assert_eq!(restored_index["indexGeneration"], "current");
        assert_eq!(owner, "shared-index-current");
        assert_eq!(revision, 77);
        assert_eq!(storage, "text");

        let backup_conn =
            Connection::open(backup.backup_path.join("sqlite/cursor-session.db")).unwrap();
        let disk_count: i64 = backup_conn
            .query_row("SELECT COUNT(*) FROM cursorDiskKV", [], |row| row.get(0))
            .unwrap();
        let item_count: i64 = backup_conn
            .query_row("SELECT COUNT(*) FROM ItemTable", [], |row| row.get(0))
            .unwrap();
        assert_eq!(disk_count, 3);
        assert_eq!(item_count, 1);
        assert!(stored_row(&backup_conn, "cursorDiskKV", "composerData:other-session").is_none());
    }

    #[test]
    fn rename_backup_restores_only_cursor_owned_name_fields() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-rename-backup";
        write_native_cursor_fixture(&db_path, session_id, true);
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
        let composer_key = format!("composerData:{session_id}");
        let composer = stored_row(&conn, "cursorDiskKV", &composer_key).unwrap();
        assert!(matches!(composer.value, SqliteValue::Text(_)));
        assert_eq!(parse_stored_json(&composer.value)["name"], "After");
        assert_eq!(composer_index(&conn).3, "blob");

        let mut current_composer = parse_stored_json(&composer.value);
        current_composer["independentComposerField"] = json!("changed independently");
        current_composer["newComposerField"] = json!(42);
        write_json_value(
            &conn,
            "cursorDiskKV",
            &composer_key,
            &current_composer,
            false,
        );
        conn.execute(
            "UPDATE cursorDiskKV
             SET owner = 'composer-current', revision = 88
             WHERE key = ?1",
            [&composer_key],
        )
        .unwrap();

        let mut current_index = composer_index(&conn).0;
        target_index_entry(&current_index, session_id);
        let entries = current_index["allComposers"].as_array_mut().unwrap();
        entries[0]["subtitle"] = json!("target changed independently");
        entries[0]["newIndexField"] = json!(true);
        entries[1]["name"] = json!("Other changed");
        current_index["indexGeneration"] = json!("current");
        write_json_value(
            &conn,
            "ItemTable",
            "composer.composerHeaders",
            &current_index,
            false,
        );
        conn.execute(
            "UPDATE ItemTable
             SET owner = 'index-current', revision = 89
             WHERE key = 'composer.composerHeaders'",
            [],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();
        CursorProvider.restore_session_backup(&backup).unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let composer = stored_row(&conn, "cursorDiskKV", &composer_key).unwrap();
        let composer_json = parse_stored_json(&composer.value);
        assert_eq!(composer_json["name"], "Before");
        assert_eq!(
            composer_json["independentComposerField"],
            "changed independently"
        );
        assert_eq!(composer_json["newComposerField"], 42);
        assert_eq!(composer.owner, "composer-current");
        assert_eq!(composer.revision, 88);
        assert!(matches!(composer.value, SqliteValue::Blob(_)));

        let (index, owner, revision, storage) = composer_index(&conn);
        let target = target_index_entry(&index, session_id);
        assert_eq!(target["name"], "Before");
        assert_eq!(target["subtitle"], "target changed independently");
        assert_eq!(target["newIndexField"], true);
        assert_eq!(
            target_index_entry(&index, "other-session")["name"],
            "Other changed"
        );
        assert_eq!(index["indexGeneration"], "current");
        assert_eq!(owner, "index-current");
        assert_eq!(revision, 89);
        assert_eq!(storage, "blob");
    }

    #[test]
    fn cursor_backup_contract_and_capabilities_are_truthful() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-backup-contract";
        write_native_cursor_fixture(&db_path, session_id, true);

        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-contract-1",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();

        let capabilities = CursorProvider.capabilities();
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert_eq!(backup.mutation, ProviderSourceMutation::Delete);
        assert_eq!(backup.operation_id, "operation-contract-1");
        assert_eq!(backup.provider_session_id, session_id);
        assert_eq!(backup.source_path, db_path.canonicalize().unwrap());
        assert_eq!(backup.format, "cursor-session-backup-v1");
        assert_eq!(
            backup.mime_type,
            "application/vnd.memorph.cursor-session-backup"
        );
        assert_eq!(
            backup
                .restore_metadata
                .get("restore_mode")
                .and_then(Value::as_str),
            Some("cursor_session_restore")
        );
        assert!(backup.backup_path.join("metadata.json").is_file());
        assert!(backup
            .backup_path
            .join("sqlite/cursor-session.db")
            .is_file());
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
        let fixture = write_native_cursor_fixture(&db_path, session_id, true);
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
        for (key, original) in &fixture.target_rows {
            assert_eq!(
                stored_row(&conn, "cursorDiskKV", key).as_ref(),
                Some(original)
            );
        }
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
        let fixture = write_native_cursor_fixture(&db_path, session_id, true);
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
        for (key, original) in &fixture.target_rows {
            assert_eq!(
                stored_row(&conn, "cursorDiskKV", key).as_ref(),
                Some(original)
            );
        }
        assert_eq!(
            target_index_entry(&composer_index(&conn).0, session_id)["name"],
            "Before"
        );
    }

    #[test]
    fn partial_cursor_rename_failure_restores_registered_backup() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-partial-rename";
        write_current_cursor_rename_fixture(&db_path, session_id);
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
        let composer: SqliteValue = conn
            .query_row(
                "SELECT value FROM cursorDiskKV WHERE key = ?1",
                [format!("composerData:{session_id}")],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(parse_stored_json(&composer)["name"], "Before");
        let index: SqliteValue = conn
            .query_row(
                "SELECT value FROM ItemTable WHERE key = 'composer.composerHeaders'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            parse_stored_json(&index)["allComposers"][0]["name"],
            "Before"
        );
    }

    #[test]
    fn cursor_backup_rejects_missing_or_non_unique_key_schema_before_mutation() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let backup_root = dir.path().join("backups");

        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE cursorDiskKV (id TEXT, value BLOB);
            CREATE TABLE ItemTable (key TEXT UNIQUE, value BLOB);
            INSERT INTO cursorDiskKV (id, value)
            VALUES ('composerData:invalid-schema', '{}');
            ",
        )
        .unwrap();
        drop(conn);
        let error = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-missing-key",
                "invalid-schema",
                &backup_root,
            )
            .unwrap_err();
        assert!(error.to_string().contains("must contain key and value"));
        assert!(!backup_root.exists());

        std::fs::remove_file(&db_path).unwrap();
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE cursorDiskKV (key TEXT, value BLOB);
            CREATE TABLE ItemTable (key TEXT UNIQUE, value BLOB);
            INSERT INTO cursorDiskKV (key, value)
            VALUES ('composerData:invalid-schema', '{}');
            ",
        )
        .unwrap();
        drop(conn);
        let error = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-non-unique-key",
                "invalid-schema",
                &backup_root,
            )
            .unwrap_err();
        assert!(error.to_string().contains("does not enforce a unique key"));
        assert!(!backup_root.exists());
    }

    #[test]
    fn cursor_backup_rejects_duplicate_target_index_entries() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-duplicate-index";
        write_native_cursor_fixture(&db_path, session_id, true);
        let conn = Connection::open(&db_path).unwrap();
        let mut index = composer_index(&conn).0;
        index["allComposers"]
            .as_array_mut()
            .unwrap()
            .push(json!({"composerId": session_id, "name": "Duplicate"}));
        write_json_value(
            &conn,
            "ItemTable",
            "composer.composerHeaders",
            &index,
            false,
        );
        drop(conn);

        let error = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-duplicate-index",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err();

        assert!(error.to_string().contains("contains duplicate entries"));
        assert!(!dir
            .path()
            .join("backups/cursor/operation-duplicate-index")
            .exists());
    }

    #[test]
    fn delete_restore_preserves_concurrently_created_nonempty_index() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-index-absent";
        write_native_cursor_fixture(&db_path, session_id, false);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-index-absent",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        write::delete_session(session_id).unwrap();

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value, owner, revision)
             VALUES ('composer.composerHeaders', ?1, 'created-current', 55)",
            [SqliteValue::Text(
                serde_json::to_string(&json!({
                    "allComposers": [{
                        "composerId": "new-session",
                        "name": "Created during restore window"
                    }],
                    "indexGeneration": "current"
                }))
                .unwrap(),
            )],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let (index, owner, revision, storage) = composer_index(&conn);
        assert_eq!(index["allComposers"].as_array().unwrap().len(), 1);
        assert_eq!(
            target_index_entry(&index, "new-session")["name"],
            "Created during restore window"
        );
        assert_eq!(index["indexGeneration"], "current");
        assert_eq!(owner, "created-current");
        assert_eq!(revision, 55);
        assert_eq!(storage, "text");
    }

    #[test]
    fn rename_restore_preserves_index_created_after_indexless_backup() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-rename-index-absent";
        write_native_cursor_fixture(&db_path, session_id, false);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Rename,
                "operation-rename-index-absent",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        write::rename_session(session_id, "After").unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let current_index = json!({
            "allComposers": [
                {
                    "composerId": session_id,
                    "name": "Created independently"
                },
                {
                    "composerId": "new-session",
                    "name": "New session"
                }
            ],
            "indexGeneration": "current"
        });
        conn.execute(
            "INSERT INTO ItemTable (key, value, owner, revision)
             VALUES ('composer.composerHeaders', ?1, 'created-current', 61)",
            [SqliteValue::Text(
                serde_json::to_string(&current_index).unwrap(),
            )],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let composer =
            stored_row(&conn, "cursorDiskKV", &format!("composerData:{session_id}")).unwrap();
        assert_eq!(parse_stored_json(&composer.value)["name"], "Before");
        let (index, owner, revision, storage) = composer_index(&conn);
        assert_eq!(
            target_index_entry(&index, session_id)["name"],
            "Created independently"
        );
        assert_eq!(
            target_index_entry(&index, "new-session")["name"],
            "New session"
        );
        assert_eq!(index["indexGeneration"], "current");
        assert_eq!(owner, "created-current");
        assert_eq!(revision, 61);
        assert_eq!(storage, "text");
    }

    #[test]
    fn rename_restore_does_not_recreate_concurrently_deleted_rows() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-rename-concurrent-delete";
        write_native_cursor_fixture(&db_path, session_id, true);
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
            "DELETE FROM cursorDiskKV WHERE key = ?1",
            [format!("composerData:{session_id}")],
        )
        .unwrap();
        conn.execute(
            "DELETE FROM ItemTable WHERE key = 'composer.composerHeaders'",
            [],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();

        let conn = Connection::open(&db_path).unwrap();
        assert!(stored_row(&conn, "cursorDiskKV", &format!("composerData:{session_id}")).is_none());
        assert!(stored_row(&conn, "ItemTable", "composer.composerHeaders").is_none());
    }

    #[test]
    fn delete_restore_does_not_resurrect_unrelated_entries_from_missing_current_index() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("state.vscdb");
        let _guard = use_test_cursor_db(db_path.clone());
        let session_id = "cursor-delete-index-missing";
        write_native_cursor_fixture(&db_path, session_id, true);
        let backup = CursorProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-delete-index-missing",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap();
        write::delete_session(session_id).unwrap();

        let conn = Connection::open(&db_path).unwrap();
        conn.execute(
            "DELETE FROM ItemTable WHERE key = 'composer.composerHeaders'",
            [],
        )
        .unwrap();
        drop(conn);

        CursorProvider.restore_session_backup(&backup).unwrap();

        let conn = Connection::open(&db_path).unwrap();
        let (index, _, _, _) = composer_index(&conn);
        let entries = index["allComposers"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["composerId"], session_id);
        assert_eq!(entries[0]["name"], "Before");
    }
}
