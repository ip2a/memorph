use super::*;
use crate::core::active_compression::{
    apply_active_compression_with_archive_dir, ActiveCompressionApplyParams, ActiveCompressionMode,
    ActiveCompressionPolicy,
};
use crate::core::session_management;
use crate::storage::artifact_store::{ArtifactManifestKind, ArtifactStorageKind};
use serde_json::json;
use tempfile::{tempdir, NamedTempFile};

static TEST_CODEX_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

struct TestCodexDirGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for TestCodexDirGuard {
    fn drop(&mut self) {
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        set_test_codex_mutation_failure(None);
        set_test_codex_dir(None);
    }
}

fn use_test_codex_dir(path: PathBuf) -> TestCodexDirGuard {
    let lock = TEST_CODEX_TEST_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .expect("test Codex serial lock");
    set_test_codex_dir(Some(path));
    crate::cache::global_cache().invalidate(PROVIDER_ID);
    TestCodexDirGuard { _lock: lock }
}

struct NativeCodexFixture {
    index_path: PathBuf,
    rollout_path: PathBuf,
    original_index_bytes: Vec<u8>,
    original_rollout_bytes: Vec<u8>,
}

#[test]
fn scan_sessions_includes_sqlite_threads_missing_from_session_index() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).unwrap();
    let _guard = use_test_codex_dir(codex_dir.clone());
    let rollout_path = codex_dir.join("sessions/rollout-unindexed.jsonl");
    std::fs::create_dir_all(rollout_path.parent().unwrap()).unwrap();
    std::fs::write(
        &rollout_path,
        serde_json::to_string(&json!({
            "type": "session_meta",
            "payload": {
                "id": "unindexed-session",
                "cwd": "/tmp/rollout-project",
                "title": "Rollout title"
            }
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();

    let conn = Connection::open(codex_dir.join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
    conn.execute_batch(
        "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT, title TEXT, rollout_path TEXT);",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO threads (id, cwd, title, rollout_path) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![
            "unindexed-session",
            "/tmp/project",
            "Current Codex session",
            rollout_path.to_string_lossy().as_ref()
        ],
    )
    .unwrap();

    let sessions = CodexProvider.scan_sessions().unwrap();

    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, "unindexed-session");
    assert_eq!(sessions[0].project_dir.as_deref(), Some("/tmp/project"));
    assert_eq!(sessions[0].title.as_deref(), Some("Current Codex session"));
    assert_eq!(sessions[0].source_path.as_deref(), rollout_path.to_str());
}

#[test]
fn rollout_summary_tolerates_multibyte_character_at_head_window_boundary() {
    let temp = tempdir().unwrap();
    let rollout_path = temp.path().join("rollout.jsonl");
    let session_meta = serde_json::to_vec(&json!({
        "timestamp": "2026-08-01T00:00:00Z",
        "type": "session_meta",
        "payload": {
            "id": "boundary-session",
            "cwd": "/tmp/project",
            "title": "Boundary session"
        }
    }))
    .unwrap();
    let mut bytes = Vec::new();
    for prefix_len in 0..=3 {
        let user_event = serde_json::to_vec(&json!({
            "timestamp": "2026-08-01T00:01:00Z",
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": format!("{}{}", "x".repeat(prefix_len), "界".repeat(30_000))
            }
        }))
        .unwrap();
        bytes = session_meta.clone();
        bytes.push(b'\n');
        bytes.extend_from_slice(&user_event);
        bytes.push(b'\n');
        bytes.extend_from_slice(
            b"{\"timestamp\":\"2026-08-01T00:02:00Z\",\"type\":\"event_msg\"}\n",
        );
        if std::str::from_utf8(&bytes[..64 * 1024]).is_err() {
            break;
        }
    }
    assert!(std::str::from_utf8(&bytes[..64 * 1024]).is_err());
    std::fs::write(&rollout_path, bytes).unwrap();

    let summary = read_codex_rollout_summary(&rollout_path)
        .unwrap()
        .expect("rollout summary");
    assert_eq!(summary.session_id, "boundary-session");
    assert_eq!(summary.created_at.as_deref(), Some("2026-08-01T00:00:00Z"));
    assert_eq!(summary.updated_at.as_deref(), Some("2026-08-01T00:02:00Z"));
}

#[test]
fn discover_codex_rollouts_skips_invalid_utf8_file() {
    let temp = tempdir().unwrap();
    let sessions_dir = temp.path().join("sessions/2026/08/01");
    std::fs::create_dir_all(&sessions_dir).unwrap();

    let valid_path = sessions_dir.join("valid.jsonl");
    std::fs::write(
        &valid_path,
        serde_json::to_string(&json!({
            "timestamp": "2026-08-01T00:00:00Z",
            "type": "session_meta",
            "payload": {"id": "valid-session", "cwd": "/tmp/project"}
        }))
        .unwrap()
            + "\n"
            + &serde_json::to_string(&json!({
                "timestamp": "2026-08-01T00:01:00Z",
                "type": "event_msg",
                "payload": {"type": "user_message", "message": "hello"}
            }))
            .unwrap()
            + "\n",
    )
    .unwrap();

    let invalid_path = sessions_dir.join("invalid.jsonl");
    std::fs::write(
        &invalid_path,
        b"\xff\xfe{\"timestamp\":\"2026-08-01T00:00:00Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"invalid-session\"}}\n",
    )
    .unwrap();

    let rollouts = discover_codex_rollouts(temp.path()).unwrap();
    assert_eq!(rollouts.len(), 1);
    assert_eq!(rollouts[0].1.session_id, "valid-session");
    assert_eq!(rollouts[0].0, valid_path);
}

fn write_native_codex_fixture(codex_dir: &Path, session_id: &str) -> NativeCodexFixture {
    let sessions_dir = codex_dir.join("sessions/2026/07/09");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let index_path = codex_dir.join("session_index.jsonl");
    let rollout_path = sessions_dir.join(format!("rollout-2026-07-09T12-00-00-{session_id}.jsonl"));
    let original_index_bytes = format!(
        "{{\"id\":\"{session_id}\",\"thread_name\":\"Before\",\"updated_at\":\"2026-07-09T12:00:00Z\"}}\n\n{{\"id\":\"session-other\",\"thread_name\":\"Other\",\"updated_at\":\"2026-07-09T13:00:00Z\"}}\n"
    )
    .into_bytes();
    let original_rollout_bytes = [
        serde_json::to_string(&json!({
            "timestamp": "2026-07-09T12:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": session_id,
                "timestamp": "2026-07-09T12:00:00Z",
                "cwd": "/tmp/project",
                "title": "Before"
            }
        }))
        .unwrap(),
        serde_json::to_string(&json!({
            "timestamp": "2026-07-09T12:01:00Z",
            "type": "event_msg",
            "payload": {
                "type": "user_message",
                "message": "hello"
            }
        }))
        .unwrap(),
    ]
    .join("\n")
    .into_bytes();
    let mut original_rollout_bytes = original_rollout_bytes;
    original_rollout_bytes.push(b'\n');
    std::fs::write(&index_path, &original_index_bytes).unwrap();
    std::fs::write(&rollout_path, &original_rollout_bytes).unwrap();

    let conn = Connection::open(codex_dir.join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            cwd TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            unrelated_note TEXT NOT NULL
        );
        CREATE TABLE thread_dynamic_tools (
            thread_id TEXT NOT NULL,
            position INTEGER NOT NULL,
            tool_name TEXT NOT NULL,
            payload BLOB,
            PRIMARY KEY (thread_id, position)
        );
        CREATE TABLE thread_goals (
            thread_id TEXT NOT NULL,
            goal_id TEXT NOT NULL,
            objective TEXT NOT NULL,
            PRIMARY KEY (thread_id, goal_id)
        );
        CREATE TABLE thread_spawn_edges (
            parent_thread_id TEXT NOT NULL,
            child_thread_id TEXT NOT NULL PRIMARY KEY,
            status TEXT NOT NULL
        );
        CREATE TABLE stage1_outputs (
            thread_id TEXT NOT NULL,
            output_id TEXT NOT NULL,
            output TEXT NOT NULL,
            PRIMARY KEY (thread_id, output_id)
        );
        CREATE TABLE agent_job_items (
            job_id TEXT NOT NULL,
            item_id TEXT NOT NULL,
            assigned_thread_id TEXT,
            payload TEXT NOT NULL,
            status TEXT NOT NULL,
            PRIMARY KEY (job_id, item_id)
        );
        ",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO threads (id, title, cwd, updated_at, unrelated_note)
         VALUES (?1, 'Before', '/tmp/project', 100, 'preserve target columns')",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO threads (id, title, cwd, updated_at, unrelated_note)
         VALUES ('session-other', 'Other', '/tmp/other', 200, 'unrelated thread')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO thread_dynamic_tools (thread_id, position, tool_name, payload)
         VALUES (?1, 0, 'shell', ?2)",
        rusqlite::params![session_id, vec![0_u8, 1, 127, 128, 255]],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO thread_goals (thread_id, goal_id, objective)
         VALUES (?1, 'goal-1', 'finish exact restore')",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
         VALUES (?1, 'session-other', 'completed')",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO stage1_outputs (thread_id, output_id, output)
         VALUES (?1, 'output-1', 'captured output')",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agent_job_items (
            job_id, item_id, assigned_thread_id, payload, status
         ) VALUES ('job-1', 'item-1', ?1, 'keep payload', 'running')",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO agent_job_items (
            job_id, item_id, assigned_thread_id, payload, status
         ) VALUES ('job-2', 'item-2', 'session-other', 'other payload', 'queued')",
        [],
    )
    .unwrap();

    NativeCodexFixture {
        index_path,
        rollout_path,
        original_index_bytes,
        original_rollout_bytes,
    }
}

fn codex_session_row_counts(codex_dir: &Path, session_id: &str) -> Vec<i64> {
    let conn = Connection::open(codex_dir.join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
    [
        ("threads", "id = ?1"),
        ("thread_dynamic_tools", "thread_id = ?1"),
        ("thread_goals", "thread_id = ?1"),
        (
            "thread_spawn_edges",
            "parent_thread_id = ?1 OR child_thread_id = ?1",
        ),
        ("stage1_outputs", "thread_id = ?1"),
        ("agent_job_items", "assigned_thread_id = ?1"),
    ]
    .into_iter()
    .map(|(table, where_clause)| {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE {where_clause}"),
            [session_id],
            |row| row.get(0),
        )
        .unwrap()
    })
    .collect()
}

fn test_sync_context(codex_dir: &Path, workspace: &Path) -> (Connection, PathBuf, String) {
    let mut conn = Connection::open_in_memory().unwrap();
    local_store::configure_connection(&conn).unwrap();
    local_store::apply_schema(&mut conn).unwrap();
    let activity_id = ActivityStore::new(&conn)
        .start(NewActivity {
            provider_id: Some(PROVIDER_ID.to_string()),
            provider_session_id: None,
            workspace_dir: Some(workspace.to_string_lossy().to_string()),
            operation_kind: ActivityOperationKind::Sync,
            actor: ActivityActor::System,
            summary: "Synchronizing Codex workspace sessions".to_string(),
            details: serde_json::json!({}),
        })
        .unwrap();
    let backup_root = codex_dir
        .parent()
        .unwrap()
        .join("memorph-artifacts")
        .join("backups")
        .join("codex-sync");
    (conn, backup_root, activity_id)
}

fn run_test_workspace_sync(
    codex_dir: &Path,
    workspace: &Path,
    keep_backups: usize,
) -> (CodexWorkspaceRepairReport, Connection, PathBuf, String) {
    let (mut conn, backup_root, activity_id) = test_sync_context(codex_dir, workspace);
    let report = sync_workspace_sessions_in_codex_home(
        &mut conn,
        &activity_id,
        &backup_root,
        codex_dir,
        Some(workspace.to_str().unwrap()),
        keep_backups,
    )
    .unwrap();
    (report, conn, backup_root, activity_id)
}

#[test]
fn delete_backup_restores_exact_codex_files_and_database_rows() {
    let codex_dir = tempdir().unwrap();
    let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
    let session_id = "session-delete-backup";
    let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
    let backup = create_codex_session_backup(
        ProviderSourceMutation::Delete,
        "operation-delete-1",
        session_id,
        &codex_dir.path().join("backups"),
    )
    .unwrap();

    delete_codex_session(session_id).unwrap();
    assert!(!fixture.rollout_path.exists());
    assert!(!std::fs::read(&fixture.index_path)
        .unwrap()
        .windows(session_id.len())
        .any(|window| window == session_id.as_bytes()));
    assert_eq!(
        codex_session_row_counts(codex_dir.path(), session_id),
        vec![0, 0, 0, 0, 0, 0]
    );

    restore_codex_session_backup(&backup).unwrap();

    assert_eq!(
        std::fs::read(&fixture.index_path).unwrap(),
        fixture.original_index_bytes
    );
    assert_eq!(
        std::fs::read(&fixture.rollout_path).unwrap(),
        fixture.original_rollout_bytes
    );
    assert_eq!(
        codex_session_row_counts(codex_dir.path(), session_id),
        vec![1, 1, 1, 1, 1, 1]
    );
    let conn = Connection::open(codex_dir.path().join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
    let payload: Vec<u8> = conn
        .query_row(
            "SELECT payload FROM thread_dynamic_tools
             WHERE thread_id = ?1 AND position = 0",
            [session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(payload, vec![0_u8, 1, 127, 128, 255]);
    let job: (Option<String>, String, String) = conn
        .query_row(
            "SELECT assigned_thread_id, payload, status
             FROM agent_job_items
             WHERE job_id = 'job-1' AND item_id = 'item-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        job,
        (
            Some(session_id.to_string()),
            "keep payload".to_string(),
            "running".to_string()
        )
    );

    let metadata: CodexSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(backup.backup_path.join("metadata.json")).unwrap())
            .unwrap();
    let tables = metadata
        .sqlite_tables
        .iter()
        .map(|manifest| manifest.table.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        tables,
        HashSet::from([
            "threads",
            "thread_dynamic_tools",
            "thread_goals",
            "thread_spawn_edges",
            "stage1_outputs",
            "agent_job_items",
        ])
    );
    assert_eq!(
        metadata
            .sqlite_tables
            .iter()
            .find(|manifest| manifest.table == "agent_job_items")
            .unwrap()
            .columns,
        vec!["job_id", "item_id", "assigned_thread_id"]
    );
}

#[test]
fn native_replace_preserves_codex_session_identity_and_path() {
    let codex_dir = tempdir().unwrap();
    let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
    let session_id = "session-native-replace";
    let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
    let original_index = std::fs::read(&fixture.index_path).unwrap();
    let mut session = import_canonical_session(&fixture.rollout_path)
        .unwrap()
        .session;
    session.events.clear();

    CodexProvider.replace_session(session_id, &session).unwrap();

    assert!(fixture.rollout_path.exists());
    assert_eq!(std::fs::read(&fixture.index_path).unwrap(), original_index);
    assert_eq!(
        import_canonical_session(&fixture.rollout_path)
            .unwrap()
            .session
            .identity
            .id,
        session_id
    );
    assert_eq!(
        codex_session_row_counts(codex_dir.path(), session_id),
        vec![1, 1, 1, 1, 1, 1]
    );
}

#[test]
fn replace_backup_restores_exact_codex_source() {
    let codex_dir = tempdir().unwrap();
    let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
    let session_id = "session-replace-backup";
    let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
    let backup = create_codex_session_backup(
        ProviderSourceMutation::Replace,
        "operation-replace-1",
        session_id,
        &codex_dir.path().join("backups"),
    )
    .unwrap();

    delete_codex_session(session_id).unwrap();
    restore_codex_session_backup(&backup).unwrap();

    assert_eq!(
        std::fs::read(&fixture.index_path).unwrap(),
        fixture.original_index_bytes
    );
    assert_eq!(
        std::fs::read(&fixture.rollout_path).unwrap(),
        fixture.original_rollout_bytes
    );
    assert_eq!(
        codex_session_row_counts(codex_dir.path(), session_id),
        vec![1, 1, 1, 1, 1, 1]
    );
}

#[test]
fn rename_backup_restores_only_codex_title_owned_state() {
    let codex_dir = tempdir().unwrap();
    let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
    let session_id = "session-rename-backup";
    let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
    let backup = create_codex_session_backup(
        ProviderSourceMutation::Rename,
        "operation-rename-1",
        session_id,
        &codex_dir.path().join("backups"),
    )
    .unwrap();

    rename_codex_session(session_id, "After").unwrap();
    let conn = Connection::open(codex_dir.path().join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
    conn.execute(
        "UPDATE threads
         SET cwd = '/tmp/changed', updated_at = 999, unrelated_note = 'changed independently'
         WHERE id = ?1",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "UPDATE agent_job_items
         SET payload = 'changed job payload', status = 'completed'
         WHERE job_id = 'job-1' AND item_id = 'item-1'",
        [],
    )
    .unwrap();
    drop(conn);

    restore_codex_session_backup(&backup).unwrap();

    assert_eq!(
        std::fs::read(&fixture.index_path).unwrap(),
        fixture.original_index_bytes
    );
    assert_eq!(
        std::fs::read(&fixture.rollout_path).unwrap(),
        fixture.original_rollout_bytes
    );
    let conn = Connection::open(codex_dir.path().join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
    let thread: (String, String, i64, String) = conn
        .query_row(
            "SELECT title, cwd, updated_at, unrelated_note
             FROM threads
             WHERE id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        thread,
        (
            "Before".to_string(),
            "/tmp/changed".to_string(),
            999,
            "changed independently".to_string()
        )
    );
    let job: (String, String) = conn
        .query_row(
            "SELECT payload, status
             FROM agent_job_items
             WHERE job_id = 'job-1' AND item_id = 'item-1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(
        job,
        ("changed job payload".to_string(), "completed".to_string())
    );

    let metadata: CodexSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(backup.backup_path.join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(metadata.sqlite_tables.len(), 1);
    assert_eq!(metadata.sqlite_tables[0].table, "threads");
    assert_eq!(metadata.sqlite_tables[0].columns, vec!["id", "title"]);
    assert_eq!(
        metadata.sqlite_tables[0].restore_mode,
        CodexSqliteRestoreMode::ThreadTitle
    );
}

#[test]
fn codex_backup_contract_and_capabilities_are_truthful() {
    let codex_dir = tempdir().unwrap();
    let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
    let session_id = "session-backup-contract";
    write_native_codex_fixture(codex_dir.path(), session_id);
    let backup = create_codex_session_backup(
        ProviderSourceMutation::Delete,
        "operation-contract-1",
        session_id,
        &codex_dir.path().join("backups"),
    )
    .unwrap();

    let capabilities = CodexProvider.capabilities();
    assert!(capabilities.backup_support.before_write);
    assert!(capabilities.backup_support.restore);
    assert!(!capabilities.backup_support.sync_only);
    assert_eq!(backup.mutation, ProviderSourceMutation::Delete);
    assert_eq!(backup.operation_id, "operation-contract-1");
    assert_eq!(backup.provider_session_id, session_id);
    assert_eq!(backup.source_path, codex_dir.path().canonicalize().unwrap());
    assert_eq!(backup.format, CODEX_SESSION_BACKUP_FORMAT);
    assert_eq!(backup.mime_type, CODEX_SESSION_BACKUP_MIME);
    assert_eq!(
        backup
            .restore_metadata
            .get("restore_mode")
            .and_then(Value::as_str),
        Some("codex_session_restore")
    );
}

#[test]
fn backup_registration_failure_prevents_codex_provider_write() {
    let codex_dir = tempdir().unwrap();
    let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
    let session_id = "session-registration-failure";
    let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
    let backup_root = codex_dir.path().join("backups");
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
        std::fs::read(&fixture.index_path).unwrap(),
        fixture.original_index_bytes
    );
    assert_eq!(
        std::fs::read(&fixture.rollout_path).unwrap(),
        fixture.original_rollout_bytes
    );
    assert_eq!(
        codex_session_row_counts(codex_dir.path(), session_id),
        vec![1, 1, 1, 1, 1, 1]
    );
    assert!(backup_root
        .join(PROVIDER_ID)
        .join("operation-registration-failure")
        .exists());
}

#[test]
fn partial_codex_delete_failure_restores_registered_backup() {
    let codex_dir = tempdir().unwrap();
    let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
    let session_id = "session-partial-delete";
    let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
    let backup_root = codex_dir.path().join("backups");
    let mut artifact_conn = Connection::open_in_memory().unwrap();
    local_store::configure_connection(&artifact_conn).unwrap();
    local_store::apply_schema(&mut artifact_conn).unwrap();
    set_test_codex_mutation_failure(Some(ProviderSourceMutation::Delete));

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
    assert_eq!(
        std::fs::read(&fixture.index_path).unwrap(),
        fixture.original_index_bytes
    );
    assert_eq!(
        std::fs::read(&fixture.rollout_path).unwrap(),
        fixture.original_rollout_bytes
    );
    assert_eq!(
        codex_session_row_counts(codex_dir.path(), session_id),
        vec![1, 1, 1, 1, 1, 1]
    );
}

#[test]
fn partial_codex_rename_failure_restores_registered_backup() {
    let codex_dir = tempdir().unwrap();
    let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
    let session_id = "session-partial-rename";
    let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
    let backup_root = codex_dir.path().join("backups");
    let mut artifact_conn = Connection::open_in_memory().unwrap();
    local_store::configure_connection(&artifact_conn).unwrap();
    local_store::apply_schema(&mut artifact_conn).unwrap();
    set_test_codex_mutation_failure(Some(ProviderSourceMutation::Rename));

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
    assert_eq!(
        std::fs::read(&fixture.index_path).unwrap(),
        fixture.original_index_bytes
    );
    assert_eq!(
        std::fs::read(&fixture.rollout_path).unwrap(),
        fixture.original_rollout_bytes
    );
    let conn = Connection::open(codex_dir.path().join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
    let title: String = conn
        .query_row(
            "SELECT title FROM threads WHERE id = ?1",
            [session_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(title, "Before");
}

#[test]
fn project_session_title_uses_codex_native_precedence_and_prompt_fallback() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let sessions_dir = codex_dir.join("sessions/2026/07/15");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let session_id = "codex-title-precedence";
    let source_path = sessions_dir.join(format!("rollout-{session_id}.jsonl"));
    let write_rollout = |title: &str| {
        std::fs::write(
            &source_path,
            [
                serde_json::to_string(&json!({
                    "timestamp": "2026-07-15T10:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": session_id,
                        "cwd": "/tmp/project",
                        "title": title
                    }
                }))
                .unwrap(),
                serde_json::to_string(&json!({
                    "timestamp": "2026-07-15T10:00:01Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": "Prompt title"
                            }
                        ]
                    }
                }))
                .unwrap(),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();
    };
    let write_index = |title: &str| {
        std::fs::write(
            codex_dir.join("session_index.jsonl"),
            serde_json::to_string(&json!({
                "id": session_id,
                "thread_name": title,
                "updated_at": "2026-07-15T10:00:01Z"
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();
    };

    write_rollout("Rollout title");
    write_index("Index title");
    let sqlite = Connection::open(codex_dir.join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
    sqlite
        .execute("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT)", [])
        .unwrap();
    sqlite
        .execute(
            "INSERT INTO threads (id, title) VALUES (?1, ?2)",
            rusqlite::params![session_id, "SQLite title"],
        )
        .unwrap();

    let imported_title = || -> String {
        let imported = CodexProvider
            .import_session(source_path.to_string_lossy().as_ref())
            .unwrap();
        session_title(&imported.session)
    };

    assert_eq!(imported_title(), "Index title");

    write_index(session_id);
    assert_eq!(imported_title(), "SQLite title");

    sqlite
        .execute(
            "UPDATE threads SET title = ?1 WHERE id = ?2",
            rusqlite::params![session_id, session_id],
        )
        .unwrap();
    assert_eq!(imported_title(), "Rollout title");

    write_rollout(session_id);
    assert_eq!(imported_title(), "Prompt title");
}

#[test]
fn codex_import_and_event_index_use_stable_source_order_for_missing_timestamps() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "type": "session_meta",
            "payload": {
                "id": "session-stable",
                "cwd": "/tmp/project"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "not-a-timestamp",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{ "type": "input_text", "text": "Build this" }]
            }
        })
    )
    .unwrap();
    file.flush().unwrap();

    let first = import_canonical_session(file.path()).unwrap();
    let second = import_canonical_session(file.path()).unwrap();
    let fingerprint = event_index::source_file_fingerprint(file.path()).unwrap();
    let (first_index, first_locations) = build_codex_event_index(file.path(), fingerprint).unwrap();
    let (second_index, second_locations) =
        build_codex_event_index(file.path(), fingerprint).unwrap();

    assert_eq!(
        serde_json::to_value(&first.session.events).unwrap(),
        serde_json::to_value(&second.session.events).unwrap()
    );
    assert_eq!(first.session.events[0].timestamp.timestamp_millis(), 1);
    assert_eq!(first.session.events[1].timestamp.timestamp_millis(), 2);
    assert_eq!(first_index, second_index);
    assert_eq!(first_locations, second_locations);
    assert_eq!(first_index.last_active_at_ms, Some(2));
}

#[test]
fn import_canonical_session_preserves_codex_runtime_and_message_events() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "session-1",
                "timestamp": "2026-05-21T10:00:00Z",
                "cwd": "/tmp/project",
                "base_instructions": { "text": "Be careful." },
                "model": "gpt-5.3-codex"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:01Z",
            "type": "turn_context",
            "payload": {
                "turn_id": "turn-1",
                "cwd": "/tmp/project",
                "current_date": "2026-05-21",
                "timezone": "Asia/Shanghai"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:02Z",
            "type": "event_msg",
            "payload": {
                "type": "task_started",
                "turn_id": "turn-1",
                "started_at": 1747821602
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:03Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "developer",
                "content": [
                    { "type": "input_text", "text": "# AGENTS.md instructions" }
                ]
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:04Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "phase": "commentary",
                "content": [
                    { "type": "output_text", "text": "Thinking out loud" }
                ]
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:05Z",
            "type": "response_item",
            "payload": {
                "type": "function_call",
                "name": "shell",
                "call_id": "call_1",
                "arguments": "{\"cmd\":\"echo hello\"}"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:05Z",
            "type": "response_item",
            "payload": {
                "type": "function_call_output",
                "call_id": "call_1",
                "output": "hello"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:06Z",
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": "turn-1",
                "last_agent_message": "Done."
            }
        })
    )
    .unwrap();

    let imported = import_canonical_session(file.path()).unwrap();
    let events = &imported.session.events;

    assert_eq!(imported.session.identity.id, "session-1");
    assert_eq!(
        imported.session.context.workspace.as_deref(),
        Some("/tmp/project")
    );
    assert!(events.iter().any(|event| {
        event.role == Role::System
            && matches!(
                event.blocks.first(),
                Some(Block::Text { text }) if text == "Be careful."
            )
    }));
    assert!(events.iter().any(|event| {
        event.role == Role::Developer
            && matches!(
                event.blocks.first(),
                Some(Block::Text { text }) if text == "# AGENTS.md instructions"
            )
    }));
    assert!(events.iter().any(|event| {
        event.role == Role::Assistant
            && matches!(
                event.blocks.first(),
                Some(Block::Thinking { text, .. }) if text == "Thinking out loud"
            )
    }));
    assert!(events.iter().any(|event| {
        event.id == "codex:response_item:6"
            && event.role == Role::Assistant
            && matches!(
                event.blocks.first(),
                Some(Block::ToolCall { name, tool_call_id, .. })
                    if name == "shell" && tool_call_id == "call_1"
            )
    }));
    assert!(events.iter().any(|event| {
        event.id == "codex:response_item:7"
            && matches!(
                event.blocks.first(),
                Some(Block::ToolResult { content, tool_call_id, .. })
                    if content == "hello" && tool_call_id == "call_1"
            )
    }));
    assert!(events.iter().any(|event| {
        event.id == "codex:event_msg:task_complete:8"
            && matches!(
                event.blocks.first(),
                Some(Block::Text { text }) if text == "Done."
            )
    }));
    let started = events
        .iter()
        .find(|event| event.id == "codex:event_msg:task_started:3")
        .unwrap();
    let completed = events
        .iter()
        .find(|event| event.id == "codex:event_msg:task_complete:8")
        .unwrap();
    assert_eq!(started.links.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(started.links.turn_outcome, None);
    assert_eq!(completed.links.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(completed.links.turn_outcome, Some(TurnOutcome::Completed));
    assert!(events
        .iter()
        .filter(|event| event.id != "codex:base_instructions:1")
        .all(|event| event.links.turn_id.as_deref() == Some("turn-1")));
}

#[test]
fn paged_import_preserves_native_turn_context_when_page_starts_mid_turn() {
    assert_eq!(
        CodexProvider.capabilities().page_strategy,
        PageStrategy::IndexedPage
    );
    let home = tempdir().unwrap();
    crate::config::set_test_home_dir(home.path().to_path_buf());
    let mut file = NamedTempFile::new().unwrap();
    for line in [
        json!({
            "timestamp": "2026-05-21T10:00:00Z",
            "type": "session_meta",
            "payload": {"id": "paged-turn", "cwd": "/tmp/project"}
        }),
        json!({
            "timestamp": "2026-05-21T10:00:01Z",
            "type": "turn_context",
            "payload": {"turn_id": "turn-page", "cwd": "/tmp/project"}
        }),
        json!({
            "timestamp": "2026-05-21T10:00:02Z",
            "type": "event_msg",
            "payload": {"type": "task_started", "turn_id": "turn-page"}
        }),
        json!({
            "timestamp": "2026-05-21T10:00:03Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Working"}]
            }
        }),
        json!({
            "timestamp": "2026-05-21T10:00:04Z",
            "type": "event_msg",
            "payload": {"type": "task_complete", "turn_id": "turn-page"}
        }),
    ] {
        writeln!(file, "{}", line).unwrap();
    }

    let page = import_canonical_session_page(file.path(), 3, Some(1)).unwrap();
    crate::config::reset_test_home_dir();

    assert_eq!(page.imported.session.events.len(), 1);
    assert_eq!(page.turn_count, None);
    let event = &page.imported.session.events[0];
    assert_eq!(event.links.turn_id.as_deref(), Some("turn-page"));
    assert_eq!(event.links.turn_outcome, None);
}

#[test]
fn import_canonical_session_hides_turn_aborted_and_internal_developer_controls() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "session-hidden-controls",
                "timestamp": "2026-05-21T10:00:00Z",
                "cwd": "/tmp/project"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:00.500Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "# AGENTS.md instructions for /tmp/project\n\n<INSTRUCTIONS>\nBe careful.\n</INSTRUCTIONS>"
                    },
                    {
                        "type": "input_text",
                        "text": "<environment_context>\n  <cwd>/tmp/project</cwd>\n  <shell>zsh</shell>\n  <current_date>2026-05-21</current_date>\n  <timezone>Asia/Shanghai</timezone>\n</environment_context>"
                    }
                ]
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:00.750Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "<codex_internal_context source=\"goal\">\nContinue working toward the active thread goal.\n</codex_internal_context>"
                    }
                ]
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_text",
                        "text": "<turn_aborted>\nInterrupted.\n</turn_aborted>"
                    }
                ]
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:02Z",
            "type": "event_msg",
            "payload": {
                "type": "turn_aborted",
                "turn_id": "turn-1",
                "reason": "interrupted"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:03Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "developer",
                "content": [
                    {
                        "type": "input_text",
                        "text": "<model_switch>\nSwitch context\n</model_switch>"
                    },
                    {
                        "type": "input_text",
                        "text": "<collaboration_mode># Collaboration Mode: Default\n</collaboration_mode>"
                    }
                ]
            }
        })
    )
    .unwrap();

    let imported = import_canonical_session(file.path()).unwrap();
    let events = &imported.session.events;

    assert!(!events.iter().any(|event| {
        event.role == Role::User
            && event.blocks.iter().any(
                |block| matches!(block, Block::Text { text } if text.contains("<turn_aborted>")),
            )
    }));
    assert!(!events.iter().any(|event| {
        event.role == Role::Developer
            && event.blocks.iter().any(
                |block| matches!(block, Block::Text { text } if text.contains("<model_switch>")),
            )
    }));
    assert!(!events.iter().any(|event| {
        event.role == Role::User
            && event.blocks.iter().any(|block| {
                matches!(block, Block::Text { text } if text.contains("<environment_context>") || text.contains("# AGENTS.md instructions") || text.contains("<codex_internal_context"))
            })
    }));
    assert!(events.iter().any(|event| {
        event.kind == EventKind::Lifecycle
            && event.role == Role::System
            && matches!(event.blocks.first(), Some(Block::Other { .. }))
    }));
    assert!(events.iter().any(|event| {
        event.kind == EventKind::Lifecycle
            && event.role == Role::System
            && matches!(event.blocks.first(), Some(Block::Other { .. }))
    }));
    assert!(events.iter().any(|event| {
        event.kind == EventKind::Lifecycle
            && event.role == Role::System
            && matches!(event.blocks.first(), Some(Block::Other { .. }))
    }));
    let aborted = events
        .iter()
        .find(|event| event.id == "codex:event_msg:turn_aborted:5")
        .unwrap();
    assert_eq!(aborted.links.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(aborted.links.turn_outcome, Some(TurnOutcome::Interrupted));
}

#[test]
fn import_canonical_session_decodes_input_image_data_uri() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "session-2",
                "timestamp": "2026-05-21T10:00:00Z",
                "cwd": "/tmp/project"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [
                    {
                        "type": "input_image",
                        "mime_type": "image/png",
                        "image_url": "data:image/png;base64,QUJD"
                    }
                ]
            }
        })
    )
    .unwrap();

    let imported = import_canonical_session(file.path()).unwrap();
    let image_block = imported
        .session
        .events
        .iter()
        .flat_map(|event| event.blocks.iter())
        .find_map(|block| match block {
            Block::Image {
                mime_type,
                data,
                path,
            } => Some((mime_type, data, path)),
            _ => None,
        })
        .expect("expected image block");

    assert_eq!(image_block.0, "image/png");
    assert_eq!(image_block.1.as_deref(), Some("QUJD"));
    assert_eq!(image_block.2, &None);
}

#[test]
fn codex_response_blocks_preserve_reasoning_and_json_tool_output() {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let output = codex_response_item_event(
        &json!({
            "type": "function_call_output",
            "call_id": "call-1",
            "output": {"status": "ok", "items": [1, 2]}
        }),
        Utc::now(),
        1,
        json!({}),
        &mut report,
    );
    assert!(matches!(
        output.blocks.as_slice(),
        [Block::ToolResult { content, .. }]
            if content == r#"{"items":[1,2],"status":"ok"}"#
    ));

    let reasoning = codex_response_item_event(
        &json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "reasoning", "summary": "internal"}]
        }),
        Utc::now(),
        2,
        json!({}),
        &mut report,
    );
    assert!(matches!(
        reasoning.blocks.as_slice(),
        [Block::Other { raw }]
            if raw == &json!({"type": "reasoning", "summary": "internal"})
    ));
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == "codex_reasoning_preserved_as_provider_payload"));
}

#[test]
fn codex_text_block_without_text_is_not_silently_dropped() {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let event = codex_response_item_event(
        &json!({
            "type": "message",
            "role": "assistant",
            "content": [{"type": "output_text"}]
        }),
        Utc::now(),
        3,
        json!({}),
        &mut report,
    );

    assert!(matches!(
        event.blocks.as_slice(),
        [Block::Other { raw }]
            if raw.get("type").and_then(Value::as_str) == Some("output_text")
    ));
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == "codex_text_block_missing_text"));
}

#[test]
fn import_canonical_session_maps_native_compacted_to_compressed_block() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "session-compacted",
                "timestamp": "2026-05-21T10:00:00Z",
                "cwd": "/tmp/project"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:01Z",
            "type": "compacted",
            "payload": {
                "message": "compressed summary",
                "replacement_history": [
                    {
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": "[Compressed session segment from claude]\ncompressed summary\nSource event count: 2\nArchive: memorph-archive://s1/archive.json.gz"
                            }
                        ]
                    }
                ],
                "memorph": {
                    "source_provider_id": "claude",
                    "summary": "compressed summary",
                    "source_event_ids": ["old-event-1", "old-event-2"],
                    "source_event_count": 2,
                    "archive_ref": "memorph-archive://s1/archive.json.gz"
                }
            }
        })
    )
    .unwrap();

    let imported = import_canonical_session(file.path()).unwrap();
    let compressed = imported
        .session
        .events
        .iter()
        .find_map(compression::compressed_segment)
        .expect("expected compressed segment");

    assert_eq!(compressed.source_provider_id, "claude");
    assert_eq!(compressed.summary, "compressed summary");
    assert_eq!(compressed.source_event_ids, ["old-event-1", "old-event-2"]);
    assert_eq!(compressed.source_event_count, Some(2));
    assert_eq!(
        compressed.archive_ref.as_deref(),
        Some("memorph-archive://s1/archive.json.gz")
    );
}

#[test]
fn compressed_segment_exports_as_native_codex_compacted_rollout() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    std::fs::create_dir_all(&codex_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        codex_dir.join("config.toml"),
        "model_provider = \"test-provider\"\n",
    )
    .unwrap();

    let session = Session {
        lineage: Vec::new(),
        schema: Schema::default(),
        identity: Identity {
            id: "session-native-compact".to_string(),
            title: Some("Native Compact".to_string()),
        },
        context: Context {
            workspace: Some(workspace.to_string_lossy().to_string()),
            created_at: None,
            last_active_at: None,
            tags: Vec::new(),
        },
        events: vec![
            Event {
                id: "compressed-source".to_string(),
                kind: EventKind::Message,
                role: Role::Assistant,
                timestamp: Utc::now(),
                links: Links::default(),
                blocks: vec![Block::Compressed {
                    raw: json!({
                        "format": "memorph.compressed.v1",
                        "source_provider_id": "claude",
                        "summary": "compressed summary",
                        "source_event_ids": ["old-user"],
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
            },
            Event {
                id: "tail-user".to_string(),
                kind: EventKind::Message,
                role: Role::User,
                timestamp: Utc::now(),
                links: Links::default(),
                blocks: vec![Block::Text {
                    text: "latest request".to_string(),
                }],
                tags: Vec::new(),
                extensions: Default::default(),
                metadata: Metadata {
                    model: None,
                    usage: None,
                },
            },
        ],
        extensions: BTreeMap::new(),
    };

    let session_id =
        export_canonical_session_in_codex_dir(&session, &workspace, &codex_dir).unwrap();
    let rollout_path = WalkDir::new(codex_dir.join("sessions"))
        .into_iter()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
        .find(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("jsonl")
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.contains(&session_id))
        })
        .expect("exported rollout");
    let lines = std::fs::read_to_string(&rollout_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    let compacted = lines
        .iter()
        .find(|line| line.get("type").and_then(Value::as_str) == Some("compacted"))
        .expect("native compacted line");
    let payload = compacted.get("payload").expect("compacted payload");
    assert_eq!(
        payload.get("message").and_then(Value::as_str),
        Some("compressed summary")
    );
    assert_eq!(
        payload
            .pointer("/memorph/source_provider_id")
            .and_then(Value::as_str),
        Some("claude")
    );
    assert_eq!(
        payload
            .pointer("/memorph/source_event_count")
            .and_then(Value::as_u64),
        Some(3)
    );
    let model_visible_text = payload
        .pointer("/replacement_history/0/content/0/text")
        .and_then(Value::as_str)
        .expect("replacement history text");
    assert!(model_visible_text.contains("[Compressed session segment from claude]"));
    assert!(model_visible_text.contains("compressed summary"));
    assert!(model_visible_text.contains("Source event count: 3"));
    assert!(model_visible_text.contains("Archive: memorph-archive://s1/archive.json.gz"));
    assert!(model_visible_text.contains("memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"));
    assert!(!model_visible_text.contains("old-event-1"));

    let compressed_response_item = lines.iter().any(|line| {
        line.get("type").and_then(Value::as_str) == Some("response_item")
            && line
                .to_string()
                .contains("[Compressed session segment from claude]")
    });
    assert!(!compressed_response_item);
    assert!(lines.iter().any(|line| {
        line.get("type").and_then(Value::as_str) == Some("response_item")
            && line.to_string().contains("latest request")
    }));
}

#[test]
fn active_compression_export_round_trips_as_native_codex_compacted_rollout() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    let archive_dir = temp.path().join("archives");
    std::fs::create_dir_all(&codex_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        codex_dir.join("config.toml"),
        "model_provider = \"test-provider\"\n",
    )
    .unwrap();

    let now = Utc::now();
    let mut source_session = Session {
        lineage: Vec::new(),
        schema: Schema::default(),
        identity: Identity {
            id: "active-to-codex".to_string(),
            title: Some("Active to Codex".to_string()),
        },
        context: Context {
            workspace: Some(workspace.to_string_lossy().to_string()),
            created_at: None,
            last_active_at: None,
            tags: Vec::new(),
        },
        events: Vec::new(),
        extensions: BTreeMap::new(),
    };
    source_session.events.push(Event {
        id: "old-user".to_string(),
        kind: EventKind::Message,
        role: Role::User,
        timestamp: now,
        links: Links::default(),
        blocks: vec![Block::Text {
            text: "historical context that should be archived ".repeat(80),
        }],
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    });
    source_session.events.push(Event {
        id: "recent-user".to_string(),
        kind: EventKind::Message,
        role: Role::User,
        timestamp: now,
        links: Links::default(),
        blocks: vec![Block::Text {
            text: "latest request".to_string(),
        }],
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    });

    let applied = apply_active_compression_with_archive_dir(
        &source_session,
        ActiveCompressionApplyParams {
            source_provider_id: "claude".to_string(),
            target_provider_id: "codex".to_string(),
            policy: ActiveCompressionPolicy {
                protect_recent_message_events: 1,
                min_candidate_bytes: 16,
                min_savings_ratio_percent: 20,
                mode: ActiveCompressionMode::Auto,
            },
            candidate_ids: Vec::new(),
        },
        &archive_dir,
    )
    .unwrap();
    assert_eq!(applied.report.archive_refs.len(), 1);
    let archive_ref = applied.report.archive_refs[0].clone();

    let session_id =
        export_canonical_session_in_codex_dir(&applied.session, &workspace, &codex_dir).unwrap();
    let rollout_path = WalkDir::new(codex_dir.join("sessions"))
        .into_iter()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
        .find(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("jsonl")
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.contains(&session_id))
        })
        .expect("exported rollout");
    let lines = std::fs::read_to_string(&rollout_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();

    let compacted = lines
        .iter()
        .find(|line| line.get("type").and_then(Value::as_str) == Some("compacted"))
        .expect("native compacted line");
    let payload = compacted.get("payload").expect("compacted payload");
    assert_eq!(
        payload
            .pointer("/memorph/source_provider_id")
            .and_then(Value::as_str),
        Some("claude")
    );
    assert_eq!(
        payload
            .pointer("/memorph/source_event_ids/0")
            .and_then(Value::as_str),
        Some("old-user")
    );
    assert_eq!(
        payload
            .pointer("/memorph/archive_ref")
            .and_then(Value::as_str),
        Some(archive_ref.as_str())
    );
    let model_visible_text = payload
        .pointer("/replacement_history/0/content/0/text")
        .and_then(Value::as_str)
        .expect("replacement history text");
    assert!(model_visible_text.contains("[Compressed session segment from claude]"));
    assert!(model_visible_text.contains(&format!("Archive: {archive_ref}")));
    assert!(model_visible_text.contains(&format!(
        "memorph compression retrieve {archive_ref} --query <terms> --max-results 5"
    )));

    let old_source_response_item = lines.iter().any(|line| {
        line.get("type").and_then(Value::as_str) == Some("response_item")
            && line
                .to_string()
                .contains("historical context that should be archived")
    });
    assert!(!old_source_response_item);

    let imported = import_canonical_session(&rollout_path).unwrap();
    let imported_compressed = imported
        .session
        .events
        .iter()
        .find_map(compression::compressed_segment)
        .expect("imported compressed segment");
    assert_eq!(imported_compressed.source_provider_id, "claude");
    assert_eq!(imported_compressed.source_event_ids, ["old-user"]);
    assert_eq!(
        imported_compressed.archive_ref.as_deref(),
        Some(archive_ref.as_str())
    );
}

#[test]
fn compressed_segment_content_fallback_stays_portable_for_non_native_paths() {
    let event = Event {
        id: "compressed-source".to_string(),
        kind: EventKind::Message,
        role: Role::Assistant,
        timestamp: Utc::now(),
        links: Links::default(),
        blocks: vec![Block::Compressed {
            raw: json!({
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

    let content = event_to_codex_content(&event);

    assert_eq!(content.len(), 1);
    assert_eq!(
        content[0].get("type").and_then(Value::as_str),
        Some("output_text")
    );
    let text = content[0]
        .get("text")
        .and_then(Value::as_str)
        .expect("portable compressed text");
    assert!(text.contains("[Compressed session segment from opencode]"));
    assert!(text.contains("compressed summary"));
    assert!(text.contains("Source event count: 3"));
    assert!(text.contains("Archive: memorph-archive://s1/archive.json.gz"));
    assert!(text.contains("memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"));
    assert!(!text.contains("old-event-1"));
    assert!(!text.contains("old-event-2"));
    assert!(!text.contains("old-event-3"));
}

#[test]
fn first_user_message_skips_empty_user_events_but_has_user_event_stays_true() {
    let session = Session {
        lineage: Vec::new(),
        schema: Schema::default(),
        identity: Identity {
            id: "session-3".to_string(),
            title: None,
        },
        context: Context {
            workspace: None,
            created_at: None,
            last_active_at: None,
            tags: Vec::new(),
        },
        events: vec![
            Event {
                id: "user-empty".to_string(),
                kind: EventKind::Message,
                role: Role::User,
                timestamp: Utc::now(),
                links: Links::default(),
                blocks: vec![Block::Text {
                    text: "   ".to_string(),
                }],
                tags: Vec::new(),
                extensions: Default::default(),
                metadata: Metadata {
                    model: None,
                    usage: None,
                },
            },
            Event {
                id: "user-real".to_string(),
                kind: EventKind::Message,
                role: Role::User,
                timestamp: Utc::now(),
                links: Links::default(),
                blocks: vec![Block::Text {
                    text: "real prompt".to_string(),
                }],
                tags: Vec::new(),
                extensions: Default::default(),
                metadata: Metadata {
                    model: None,
                    usage: None,
                },
            },
        ],
        extensions: BTreeMap::new(),
    };

    assert!(has_user_event(&session));
    assert_eq!(first_user_message(&session).as_deref(), Some("real prompt"));
}

#[test]
fn update_codex_global_state_file_remembers_workspace_without_switching_active_root() {
    let dir = tempdir().unwrap();
    let path = dir.path().join(".codex-global-state.json");
    let workspace_a = "/tmp/a";
    let workspace_b = "/tmp/b";
    std::fs::write(
        &path,
        serde_json::to_string(&json!({
            "electron-saved-workspace-roots": [workspace_a],
            "active-workspace-roots": [workspace_a],
            "project-order": [workspace_a],
        }))
        .unwrap(),
    )
    .unwrap();

    update_codex_global_state_file(&path, Path::new(workspace_b)).unwrap();

    let updated: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        updated["electron-saved-workspace-roots"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>(),
        vec![workspace_a, workspace_b]
    );
    assert_eq!(
        updated["project-order"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>(),
        vec![workspace_a, workspace_b]
    );
    assert_eq!(
        updated["active-workspace-roots"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>(),
        vec![workspace_a]
    );
}

#[test]
fn sync_workspace_sessions_registers_prewrite_backup_with_activity_identity() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    let sessions_dir = codex_dir.join("sessions/2026/05/27");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        codex_dir.join("config.toml"),
        "model_provider = \"custom-provider\"\n",
    )
    .unwrap();
    std::fs::write(
        codex_dir.join(".codex-global-state.json"),
        serde_json::to_string(&json!({
            "electron-saved-workspace-roots": [],
            "project-order": [],
            "active-workspace-roots": [],
        }))
        .unwrap(),
    )
    .unwrap();

    let session_path = sessions_dir.join("rollout-2026-05-27T12-00-00-session-1.jsonl");
    std::fs::write(
        &session_path,
        [
            serde_json::to_string(&json!({
                "timestamp": "2026-05-27T12:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-1",
                    "timestamp": "2026-05-27T12:00:00Z",
                    "cwd": workspace.to_string_lossy(),
                    "model_provider": "openai",
                    "title": "Repair me"
                }
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "timestamp": "2026-05-27T12:05:00Z",
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "hello"
                }
            }))
            .unwrap(),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let (report, mut activity_conn, backup_root, activity_id) =
        run_test_workspace_sync(&codex_dir, &workspace, DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT);

    assert_eq!(report.current_model_provider, "custom-provider");
    assert_eq!(report.workspace_session_count, 1);
    assert_eq!(report.hidden_session_count, 1);
    assert_eq!(report.repaired_session_count, 1);
    assert_eq!(report.reindexed_session_count, 1);
    assert_eq!(report.sqlite_rows_updated, 0);
    assert!(report.backup_dir.is_some());
    assert_eq!(report.touched_sessions.len(), 1);
    assert_eq!(
        report.touched_sessions[0]
            .previous_model_provider
            .as_deref(),
        Some("openai")
    );

    let backup_id = report.backup_id.as_deref().unwrap();
    let backup = ArtifactStore::new(&mut activity_conn)
        .get_backup(backup_id)
        .unwrap()
        .unwrap();
    let canonical_codex_dir = codex_dir.canonicalize().unwrap();
    let canonical_workspace = workspace.canonicalize().unwrap();
    let canonical_backup_root = backup_root.canonicalize().unwrap();
    assert_eq!(backup.operation_id.as_deref(), Some(activity_id.as_str()));
    assert_eq!(
        backup.artifact.operation_id.as_deref(),
        Some(activity_id.as_str())
    );
    assert_eq!(
        backup.artifact.artifact_kind,
        ArtifactManifestKind::SessionBackup
    );
    assert_eq!(backup.artifact.storage_kind, ArtifactStorageKind::Directory);
    assert!(backup.artifact.content_hash.starts_with("sha256-tree-v1:"));
    assert_eq!(
        backup.source_path.as_deref(),
        Some(canonical_codex_dir.as_path())
    );
    assert!(backup.artifact.path.starts_with(&canonical_backup_root));
    assert_eq!(
        backup.artifact.mime_type.as_deref(),
        Some("application/vnd.memorph.codex-sync-backup")
    );
    assert_eq!(
        backup.artifact.format.as_deref(),
        Some("codex-sync-backup-v1")
    );
    assert_eq!(
        backup.artifact.metadata,
        json!({
            "role": "codex_prewrite_sync_backup",
            "workspace_dir": canonical_workspace.to_string_lossy(),
            "target_provider": "custom-provider",
            "provider_session_ids": ["session-1"],
        })
    );
    assert_eq!(
        backup.metadata,
        json!({
            "restore_mode": "codex_sync_restore",
            "metadata_file": "metadata.json",
        })
    );
    assert_eq!(
        report.backup_artifact_id.as_deref(),
        Some(backup.artifact.id.as_str())
    );
    assert_eq!(report.backup_id.as_deref(), Some(backup.id.as_str()));

    let updated_rollout = std::fs::read_to_string(&session_path).unwrap();
    assert!(updated_rollout.contains("\"model_provider\":\"custom-provider\""));

    let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
    assert!(index.contains("\"id\":\"session-1\""));

    let global_state: Value = serde_json::from_str(
        &std::fs::read_to_string(codex_dir.join(".codex-global-state.json")).unwrap(),
    )
    .unwrap();
    let saved = global_state["electron-saved-workspace-roots"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert_eq!(saved, vec![canonical_workspace.to_string_lossy().as_ref()]);
}

#[test]
fn codex_sync_backup_registration_conflict_keeps_backup_and_source_unchanged() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    let sessions_dir = codex_dir.join("sessions/2026/05/27");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    let session_path = sessions_dir.join("rollout-2026-05-27T12-00-00-session-1.jsonl");
    let original_rollout = serde_json::to_string(&json!({
        "timestamp": "2026-05-27T12:00:00Z",
        "type": "session_meta",
        "payload": {
            "id": "session-1",
            "timestamp": "2026-05-27T12:00:00Z",
            "cwd": workspace.to_string_lossy(),
            "model_provider": "openai",
            "title": "Unchanged"
        }
    }))
    .unwrap()
        + "\n";
    std::fs::write(&session_path, &original_rollout).unwrap();

    let (mut activity_conn, backup_root, activity_id) = test_sync_context(&codex_dir, &workspace);
    let canonical_workspace = workspace.canonicalize().unwrap();
    let backup_dir = create_codex_sync_backup(
        &backup_root,
        &activity_id,
        &codex_dir,
        canonical_workspace.to_string_lossy().as_ref(),
        "custom-provider",
        std::slice::from_ref(&session_path),
    )
    .unwrap();
    register_codex_sync_backup(
        &mut activity_conn,
        &activity_id,
        &codex_dir,
        &backup_dir,
        canonical_workspace.to_string_lossy().as_ref(),
        "custom-provider",
        &["different-session".to_string()],
    )
    .unwrap();

    let error = register_codex_sync_backup(
        &mut activity_conn,
        &activity_id,
        &codex_dir,
        &backup_dir,
        canonical_workspace.to_string_lossy().as_ref(),
        "custom-provider",
        &["session-1".to_string()],
    )
    .unwrap_err();

    assert!(format!("{error:#}")
        .contains("Artifact path was already registered with conflicting context"));
    assert!(backup_dir.exists());
    assert_eq!(
        std::fs::read_to_string(&session_path).unwrap(),
        original_rollout
    );
}

#[test]
fn sync_workspace_sessions_reindexes_with_sqlite_title_when_rollout_has_none() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    let sessions_dir = codex_dir.join("sessions/2026/05/27");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        codex_dir.join("config.toml"),
        "model_provider = \"custom-provider\"\n",
    )
    .unwrap();

    let session_path = sessions_dir.join("rollout-2026-05-27T12-00-00-sqlite-title-session.jsonl");
    std::fs::write(
        &session_path,
        [
            serde_json::to_string(&json!({
                "timestamp": "2026-05-27T12:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "sqlite-title-session",
                    "timestamp": "2026-05-27T12:00:00Z",
                    "cwd": workspace.to_string_lossy(),
                    "model_provider": "openai"
                }
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "timestamp": "2026-05-27T12:05:00Z",
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "hello"
                }
            }))
            .unwrap(),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
    let conn = Connection::open(&sqlite_path).unwrap();
    conn.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            model_provider TEXT,
            cwd TEXT,
            has_user_event INTEGER,
            title TEXT
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO threads (id, model_provider, cwd, has_user_event, title) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "sqlite-title-session",
            "openai",
            workspace.to_string_lossy().to_string(),
            0,
            "SQLite title"
        ],
    )
    .unwrap();

    let (report, _, _, _) =
        run_test_workspace_sync(&codex_dir, &workspace, DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT);

    assert_eq!(report.reindexed_session_count, 1);
    assert_eq!(
        report.touched_sessions[0].title.as_deref(),
        Some("SQLite title")
    );
    let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
    assert!(index.contains("\"id\":\"sqlite-title-session\""));
    assert!(index.contains("\"thread_name\":\"SQLite title\""));
    assert!(!index.contains("\"thread_name\":\"sqlite-title-session\""));
}

#[test]
fn sync_workspace_sessions_updates_archived_rollouts_sqlite_and_prunes_backups() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    let sessions_dir = codex_dir.join("sessions/2026/05/27");
    let archived_dir = codex_dir.join("archived_sessions/2026/05/20");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&archived_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        codex_dir.join("config.toml"),
        "model_provider = \"custom-provider\"\n",
    )
    .unwrap();
    std::fs::write(
        codex_dir.join(CODEX_GLOBAL_STATE_FILE_BASENAME),
        serde_json::to_string(&json!({
            "electron-saved-workspace-roots": [],
            "project-order": [],
            "active-workspace-roots": [],
        }))
        .unwrap(),
    )
    .unwrap();
    std::fs::write(
        codex_dir.join("session_index.jsonl"),
        serde_json::to_string(&json!({
            "id": "session-active",
            "thread_name": "Existing index",
            "updated_at": "2026-05-27T12:05:00Z",
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();

    let active_path = sessions_dir.join("rollout-2026-05-27T12-00-00-session-active.jsonl");
    std::fs::write(
        &active_path,
        [
            serde_json::to_string(&json!({
                "timestamp": "2026-05-27T12:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-active",
                    "timestamp": "2026-05-27T12:00:00Z",
                    "cwd": workspace.to_string_lossy(),
                    "model_provider": "openai",
                    "title": "Active hidden"
                }
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "timestamp": "2026-05-27T12:05:00Z",
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "hello"
                }
            }))
            .unwrap(),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let archived_path = archived_dir.join("rollout-2026-05-20T08-00-00-session-archived.jsonl");
    std::fs::write(
        &archived_path,
        [
            serde_json::to_string(&json!({
                "timestamp": "2026-05-20T08:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-archived",
                    "timestamp": "2026-05-20T08:00:00Z",
                    "cwd": workspace.to_string_lossy(),
                    "model_provider": "openai",
                    "title": "Archived hidden"
                }
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "timestamp": "2026-05-20T08:01:00Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "need sync" }
                    ]
                }
            }))
            .unwrap(),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
    let conn = Connection::open(&sqlite_path).unwrap();
    conn.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            model_provider TEXT,
            cwd TEXT,
            has_user_event INTEGER
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO threads (id, model_provider, cwd, has_user_event) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["session-active", "openai", "/tmp/other", 0],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO threads (id, model_provider, cwd, has_user_event) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params!["session-archived", "openai", "/tmp/other", 0],
    )
    .unwrap();

    let (mut activity_conn, backup_root, current_activity_id) =
        test_sync_context(&codex_dir, &workspace);
    let stale_activity_id = ActivityStore::new(&activity_conn)
        .start(NewActivity {
            provider_id: Some(PROVIDER_ID.to_string()),
            provider_session_id: None,
            workspace_dir: Some(workspace.to_string_lossy().to_string()),
            operation_kind: ActivityOperationKind::Sync,
            actor: ActivityActor::System,
            summary: "Previous Codex workspace sync".to_string(),
            details: serde_json::json!({}),
        })
        .unwrap();
    let canonical_workspace = workspace.canonicalize().unwrap();
    let stale_backup_dir = create_codex_sync_backup(
        &backup_root,
        &stale_activity_id,
        &codex_dir,
        canonical_workspace.to_string_lossy().as_ref(),
        "openai",
        &[],
    )
    .unwrap();
    let stale_backup = register_codex_sync_backup(
        &mut activity_conn,
        &stale_activity_id,
        &codex_dir,
        &stale_backup_dir,
        canonical_workspace.to_string_lossy().as_ref(),
        "openai",
        &[],
    )
    .unwrap();
    let report = sync_workspace_sessions_in_codex_home(
        &mut activity_conn,
        &current_activity_id,
        &backup_root,
        &codex_dir,
        Some(workspace.to_str().unwrap()),
        1,
    )
    .unwrap();

    assert_eq!(report.scanned_rollouts, 2);
    assert_eq!(report.workspace_session_count, 2);
    assert_eq!(report.hidden_session_count, 2);
    assert_eq!(report.repaired_session_count, 2);
    assert_eq!(report.reindexed_session_count, 1);
    assert_eq!(report.sqlite_provider_rows_updated, 2);
    assert_eq!(report.sqlite_user_event_rows_updated, 2);
    assert_eq!(report.sqlite_cwd_rows_updated, 2);
    assert_eq!(report.sqlite_rows_updated, 6);
    assert_eq!(report.pruned_backup_count, 1);
    assert!(report.skipped_rollout_files.is_empty());

    let backup_dir = PathBuf::from(report.backup_dir.clone().unwrap());
    assert!(backup_dir.exists());
    assert!(!stale_backup_dir.exists());
    assert!(ArtifactStore::new(&mut activity_conn)
        .get_backup(&stale_backup.id)
        .unwrap()
        .is_none());
    assert!(ArtifactStore::new(&mut activity_conn)
        .get(&stale_backup.artifact.id)
        .unwrap()
        .is_none());
    assert!(ArtifactStore::new(&mut activity_conn)
        .get_backup(report.backup_id.as_deref().unwrap())
        .unwrap()
        .is_some());

    let active_rollout = std::fs::read_to_string(&active_path).unwrap();
    assert!(active_rollout.contains("\"model_provider\":\"custom-provider\""));
    let archived_rollout = std::fs::read_to_string(&archived_path).unwrap();
    assert!(archived_rollout.contains("\"model_provider\":\"custom-provider\""));

    let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
    assert!(index.contains("\"id\":\"session-active\""));
    assert!(index.contains("\"id\":\"session-archived\""));

    let verify_conn = Connection::open(&sqlite_path).unwrap();
    let mut stmt = verify_conn
        .prepare("SELECT model_provider, cwd, has_user_event FROM threads WHERE id = ?1")
        .unwrap();
    let active_row = stmt
        .query_row(rusqlite::params!["session-active"], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .unwrap();
    assert_eq!(active_row.0, "custom-provider");
    assert_eq!(active_row.1, workspace.to_string_lossy().to_string());
    assert_eq!(active_row.2, 1);
    let archived_row = stmt
        .query_row(rusqlite::params!["session-archived"], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .unwrap();
    assert_eq!(archived_row.0, "custom-provider");
    assert_eq!(archived_row.1, workspace.to_string_lossy().to_string());
    assert_eq!(archived_row.2, 1);

    let backup_entries = std::fs::read_dir(&backup_root)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .collect::<Vec<_>>();
    assert_eq!(backup_entries.len(), 1);
}

#[test]
fn sync_workspace_sessions_fixes_index_title_equal_to_session_id() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    let sessions_dir = codex_dir.join("sessions/2026/06/08");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        codex_dir.join("config.toml"),
        "model_provider = \"custom-provider\"\n",
    )
    .unwrap();

    let session_id = "019ea6e7-session-id-as-title";
    let session_path =
        sessions_dir.join(format!("rollout-2026-06-08T19-03-56-{}.jsonl", session_id));
    std::fs::write(
        &session_path,
        [
            serde_json::to_string(&json!({
                "timestamp": "2026-06-08T19:03:56Z",
                "type": "session_meta",
                "payload": {
                    "id": session_id,
                    "timestamp": "2026-06-08T19:03:56Z",
                    "cwd": workspace.to_string_lossy(),
                    "model_provider": "custom-provider"
                }
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "timestamp": "2026-06-08T19:04:00Z",
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "hello"
                }
            }))
            .unwrap(),
        ]
        .join("\n")
            + "\n",
    )
    .unwrap();

    std::fs::write(
        codex_dir.join("session_index.jsonl"),
        serde_json::to_string(&json!({
            "id": session_id,
            "thread_name": session_id,
            "updated_at": "2026-06-08T19:04:01Z",
        }))
        .unwrap()
            + "\n",
    )
    .unwrap();

    let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
    let conn = Connection::open(&sqlite_path).unwrap();
    conn.execute(
        "CREATE TABLE threads (
            id TEXT PRIMARY KEY,
            model_provider TEXT,
            cwd TEXT,
            has_user_event INTEGER,
            title TEXT
        )",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO threads (id, model_provider, cwd, has_user_event, title) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            session_id,
            "custom-provider",
            workspace.to_string_lossy().to_string(),
            1,
            "Real Title"
        ],
    )
    .unwrap();

    let (report, _, _, _) =
        run_test_workspace_sync(&codex_dir, &workspace, DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT);

    assert_eq!(report.workspace_session_count, 1);
    assert_eq!(report.repaired_session_count, 0);
    assert_eq!(report.reindexed_session_count, 0);
    assert_eq!(report.retitled_session_count, 1);
    assert_eq!(report.touched_sessions.len(), 1);
    assert!(report.touched_sessions[0].updated_index_title);
    assert!(!report.touched_sessions[0].added_to_index);

    let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
    assert!(index.contains("\"id\":\"019ea6e7-session-id-as-title\""));
    assert!(index.contains("\"thread_name\":\"Real Title\""));
    assert!(!index.contains("\"thread_name\":\"019ea6e7-session-id-as-title\""));
}

#[test]
fn import_canonical_session_drops_token_count() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "session-tc",
                "timestamp": "2026-05-21T10:00:00Z",
                "cwd": "/tmp/project"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "assistant",
                "content": [
                    { "type": "output_text", "text": "Hello" }
                ]
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:02Z",
            "type": "response_item",
            "payload": {
                "type": "token_count",
                "info": {
                    "input_tokens": 100,
                    "output_tokens": 50
                }
            }
        })
    )
    .unwrap();

    let imported = import_canonical_session(file.path()).unwrap();
    // session_meta + message = 2 events; token_count is dropped
    assert_eq!(imported.session.events.len(), 2);
    assert!(!imported.session.events.iter().any(|event| {
        event.blocks.iter().any(
            |block| matches!(block, Block::Other { raw } if raw.get("type").and_then(Value::as_str) == Some("token_count")),
        )
    }));
}

#[test]
fn import_canonical_session_dedupes_last_agent_message() {
    let mut file = NamedTempFile::new().unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "session-dedup",
                "timestamp": "2026-05-21T10:00:00Z",
                "cwd": "/tmp/project"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:01Z",
            "type": "event_msg",
            "payload": {
                "type": "agent_message",
                "message": "Same text",
                "last_agent_message": "Same text",
                "phase": "final_answer"
            }
        })
    )
    .unwrap();
    writeln!(
        file,
        "{}",
        json!({
            "timestamp": "2026-05-21T10:00:02Z",
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": "turn-1",
                "last_agent_message": "Different text"
            }
        })
    )
    .unwrap();

    let imported = import_canonical_session(file.path()).unwrap();
    let events: Vec<_> = imported.session.events.iter().collect();

    let agent_msg = events
        .iter()
        .find(|e| e.id == "codex:event_msg:agent_message:2")
        .unwrap();
    let text_blocks: Vec<_> = agent_msg
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks, vec!["Same text"]);

    let complete_msg = events
        .iter()
        .find(|e| e.id == "codex:event_msg:task_complete:3")
        .unwrap();
    let text_blocks: Vec<_> = complete_msg
        .blocks
        .iter()
        .filter_map(|b| match b {
            Block::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text_blocks, vec!["Different text"]);
}

#[test]
fn provider_payload_block_is_skipped_in_codex_export() {
    let event = Event {
        id: "test".to_string(),
        kind: EventKind::Message,
        role: Role::Assistant,
        timestamp: Utc::now(),
        links: Links::default(),
        blocks: vec![
            Block::Text {
                text: "Hello".to_string(),
            },
            Block::Other {
                raw: serde_json::json!({"type": "task_complete"}),
            },
        ],
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    };

    let content = event_to_codex_content(&event);
    assert_eq!(content.len(), 1);
    assert_eq!(
        content[0].get("text").and_then(Value::as_str),
        Some("Hello")
    );
}

#[test]
fn codex_base_instructions_use_instruction_context_not_lifecycle() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    std::fs::create_dir_all(&codex_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(codex_dir.join("config.toml"), "model_provider = \"test\"\n").unwrap();

    let session = Session {
        lineage: Vec::new(),
        schema: Schema::default(),
        identity: Identity {
            id: "session-base-instructions".to_string(),
            title: Some("Base Instructions".to_string()),
        },
        context: Context {
            workspace: Some(workspace.to_string_lossy().to_string()),
            created_at: None,
            last_active_at: None,
            tags: Vec::new(),
        },
        events: vec![
            codex_test_event(
                "system",
                EventKind::Message,
                Role::System,
                vec![Block::Text {
                    text: "system instructions".to_string(),
                }],
            ),
            codex_test_event(
                "runtime",
                EventKind::Lifecycle,
                Role::System,
                vec![Block::Text {
                    text: "runtime context".to_string(),
                }],
            ),
            codex_test_event(
                "developer",
                EventKind::Message,
                Role::Developer,
                vec![Block::Text {
                    text: "developer instructions".to_string(),
                }],
            ),
            codex_test_event(
                "payload",
                EventKind::Message,
                Role::System,
                vec![Block::Other {
                    raw: serde_json::json!({"text": "provider payload"}),
                }],
            ),
            codex_test_event(
                "user",
                EventKind::Message,
                Role::User,
                vec![Block::Text {
                    text: "real prompt".to_string(),
                }],
            ),
        ],
        extensions: BTreeMap::new(),
    };

    let session_id =
        export_canonical_session_in_codex_dir(&session, &workspace, &codex_dir).unwrap();
    let rollout_path = WalkDir::new(codex_dir.join("sessions"))
        .into_iter()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
        .find(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("jsonl")
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.contains(&session_id))
        })
        .expect("exported rollout");
    let session_meta = std::fs::read_to_string(&rollout_path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|line| line.get("type").and_then(Value::as_str) == Some("session_meta"))
        .expect("session_meta line");
    let instructions = session_meta
        .pointer("/payload/base_instructions/text")
        .and_then(Value::as_str)
        .expect("base instructions");

    assert_eq!(
        instructions,
        "system instructions\n\ndeveloper instructions"
    );
    assert!(!instructions.contains("runtime context"));
    assert!(!instructions.contains("provider payload"));
    assert!(!instructions.contains("real prompt"));
}

fn codex_test_event(id: &str, kind: EventKind, role: Role, blocks: Vec<Block>) -> Event {
    Event {
        id: id.to_string(),
        kind,
        role,
        timestamp: Utc::now(),
        links: Links::default(),
        blocks,
        tags: Vec::new(),
        extensions: Default::default(),
        metadata: Metadata {
            model: None,
            usage: None,
        },
    }
}

#[test]
fn codex_export_emits_native_function_call_and_output_for_tool_blocks() {
    let assistant_event = codex_test_event(
        "call-1",
        EventKind::Action,
        Role::Assistant,
        vec![Block::ToolCall {
            tool_call_id: "call-1".to_string(),
            name: "shell".to_string(),
            input: Some(json!({"command": "ls"})),
        }],
    );
    let tool_event = codex_test_event(
        "call-1-result",
        EventKind::Observation,
        Role::Tool,
        vec![Block::ToolResult {
            tool_call_id: "call-1".to_string(),
            content: "file.txt".to_string(),
            outcome: crate::session::execution_outcome(false),
        }],
    );

    let assistant_lines = super::write::codex_tool_response_items(&assistant_event);
    assert_eq!(assistant_lines.len(), 1);
    let payload = &assistant_lines[0]["payload"];
    assert_eq!(payload["type"], json!("function_call"));
    assert_eq!(payload["name"], json!("shell"));
    assert_eq!(payload["call_id"], json!("call-1"));
    assert_eq!(payload["phase"], json!("final_answer"));

    let tool_lines = super::write::codex_tool_response_items(&tool_event);
    assert_eq!(tool_lines.len(), 1);
    let payload = &tool_lines[0]["payload"];
    assert_eq!(payload["type"], json!("function_call_output"));
    assert_eq!(payload["call_id"], json!("call-1"));
    assert_eq!(payload["output"], json!("file.txt"));
    assert_eq!(payload["is_error"], json!(false));
}

#[test]
fn codex_export_then_import_round_trips_tool_call_and_result() {
    let temp = tempdir().unwrap();
    let codex_dir = temp.path().join(".codex");
    let workspace = temp.path().join("repo");
    std::fs::create_dir_all(&codex_dir).unwrap();
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::write(
        codex_dir.join("config.toml"),
        "model_provider = \"test-provider\"\n",
    )
    .unwrap();
    let _guard = use_test_codex_dir(codex_dir.clone());

    let session = Session {
        lineage: Vec::new(),
        schema: Schema::default(),
        identity: Identity {
            id: "rt-source".to_string(),
            title: Some("Codex Round Trip".to_string()),
        },
        context: Context {
            workspace: Some(workspace.to_string_lossy().to_string()),
            created_at: None,
            last_active_at: None,
            tags: Vec::new(),
        },
        events: vec![
            codex_test_event(
                "rt-user",
                EventKind::Message,
                Role::User,
                vec![Block::Text {
                    text: "list files".to_string(),
                }],
            ),
            codex_test_event(
                "rt-assistant",
                EventKind::Action,
                Role::Assistant,
                vec![
                    Block::Text {
                        text: "running ls".to_string(),
                    },
                    Block::ToolCall {
                        tool_call_id: "call-rt1".to_string(),
                        name: "shell".to_string(),
                        input: Some(json!({"command": "ls"})),
                    },
                ],
            ),
            codex_test_event(
                "rt-result",
                EventKind::Observation,
                Role::Tool,
                vec![Block::ToolResult {
                    tool_call_id: "call-rt1".to_string(),
                    content: "Cargo.toml".to_string(),
                    outcome: crate::session::execution_outcome(false),
                }],
            ),
        ],
        extensions: BTreeMap::new(),
    };

    let session_id =
        export_canonical_session_in_codex_dir(&session, &workspace, &codex_dir).unwrap();

    let rollout_path = WalkDir::new(codex_dir.join("sessions"))
        .into_iter()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.into_path())
        .find(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("jsonl")
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .is_some_and(|value| value.contains(&session_id))
        })
        .expect("exported rollout file");

    let imported = super::load::import_canonical_session(&rollout_path).unwrap();

    // ToolCall survives.
    let assistant = imported
        .session
        .events
        .iter()
        .find(|e| e.role == Role::Assistant)
        .expect("assistant event survived round-trip");
    assert!(
        assistant.blocks.iter().any(|b| matches!(
            b,
            Block::ToolCall { tool_call_id, name, .. }
                if tool_call_id == "call-rt1" && name == "shell"
        )),
        "ToolCall did not round-trip; blocks = {:?}",
        assistant.blocks
    );

    // ToolResult survives.
    assert!(
        imported.session.events.iter().any(|e| {
            e.blocks.iter().any(|b| matches!(
                b,
                Block::ToolResult { tool_call_id, content, .. }
                    if tool_call_id == "call-rt1" && content == "Cargo.toml"
            ))
        }),
        "ToolResult did not round-trip"
    );

    // User text survives.
    assert!(
        imported.session.events.iter().any(|e| {
            e.role == Role::User
                && e.blocks
                    .iter()
                    .any(|b| matches!(b, Block::Text { text } if text == "list files"))
        }),
        "user text did not round-trip"
    );
}
