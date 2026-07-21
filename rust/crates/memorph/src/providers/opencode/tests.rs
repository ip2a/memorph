use super::*;
use crate::{core::session_management, storage::local_store};
use chrono::TimeZone;
use rusqlite::Connection;
use tempfile::tempdir;

fn write_multimessage_opencode_db(opencode_dir: &Path, session_id: &str) {
    std::fs::create_dir_all(opencode_dir).unwrap();
    let conn = Connection::open(opencode_dir.join("opencode.db")).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE project (
            id TEXT PRIMARY KEY,
            worktree TEXT NOT NULL
        );
        CREATE TABLE session (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            parent_id TEXT,
            slug TEXT NOT NULL,
            directory TEXT NOT NULL,
            title TEXT NOT NULL,
            version TEXT NOT NULL,
            share_url TEXT,
            summary_additions INTEGER,
            summary_deletions INTEGER,
            summary_files INTEGER,
            summary_diffs TEXT,
            revert TEXT,
            permission TEXT,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            time_compacting INTEGER,
            time_archived INTEGER,
            workspace_id TEXT,
            path TEXT,
            agent TEXT,
            model TEXT,
            FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
        );
        CREATE TABLE message (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
        );
        CREATE TABLE part (
            id TEXT PRIMARY KEY,
            message_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL,
            FOREIGN KEY (message_id) REFERENCES message(id) ON DELETE CASCADE
        );
        ",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project (id, worktree) VALUES ('p1', '/tmp/project')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session (
            id, project_id, parent_id, slug, directory, title, version,
            share_url, summary_additions, summary_deletions, summary_files,
            summary_diffs, revert, permission, time_created, time_updated,
            time_compacting, time_archived, workspace_id, path, agent, model
         ) VALUES (
            ?1, 'p1', NULL, 's', '/tmp/project', 'Multi', '1.0',
            NULL, NULL, NULL, NULL, NULL, NULL, NULL,
            1700000000000, 1700000000500, NULL, NULL, NULL, NULL, NULL, NULL
         )",
        [session_id],
    )
    .unwrap();

    let messages = [
        ("msg-a", 1700000000010_i64, "user", "Build feature"),
        ("msg-b", 1700000000020, "assistant", "On it"),
        ("msg-c", 1700000000030, "user", "Thanks"),
    ];
    for (msg_id, created, role, text) in messages {
        let data = serde_json::json!({ "role": role }).to_string();
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, time_updated, data)
             VALUES (?1, ?2, ?3, ?3, ?4)",
            rusqlite::params![msg_id, session_id, created, data],
        )
        .unwrap();
        let part_data = serde_json::json!({ "type": "text", "text": text }).to_string();
        conn.execute(
            "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data)
             VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            rusqlite::params![
                format!("{msg_id}-p1"),
                msg_id,
                session_id,
                created,
                part_data
            ],
        )
        .unwrap();
    }
}

#[test]
fn import_session_page_paginates_messages_and_keeps_full_counts() {
    assert_eq!(
        OpenCodeProvider.capabilities().page_strategy,
        PageStrategy::NativePage
    );
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    write_multimessage_opencode_db(opencode_dir.path(), "ses-paged");

    let locator = format!(
        "{}#session=ses-paged",
        opencode_dir.path().join("opencode.db").display()
    );

    // Full import baseline.
    let full_import = OpenCodeProvider.import_session(&locator).unwrap();
    assert_eq!(full_import.session.events.len(), 3);

    // Full page: counts match a full import, all events present.
    let full = import_opencode_session_page(&locator, 0, None).unwrap();
    assert_eq!(full.event_count, 3);
    assert_eq!(full.imported.session.events.len(), 3);
    let expected_visible = full_import
        .session
        .events
        .iter()
        .filter(|event| canonical_event_is_visible_message(event))
        .count();
    assert_eq!(full.message_count, expected_visible);
    assert_eq!(full.message_count, 3);
    assert_eq!(full.turn_count, Some(full.turns.len()));

    // Page with limit returns a strict subset but keeps total counts.
    let page1 = import_opencode_session_page(&locator, 0, Some(2)).unwrap();
    assert_eq!(page1.imported.session.events.len(), 2);
    assert_eq!(page1.event_count, 3);
    assert_eq!(page1.message_count, full.message_count);
    assert_eq!(page1.turn_count, None);
    assert_eq!(page1.imported.session.events[0].id, "msg-a");
    assert_eq!(page1.imported.session.events[1].id, "msg-b");

    // Second page starts at offset 2.
    let page2 = import_opencode_session_page(&locator, 2, Some(2)).unwrap();
    assert_eq!(page2.imported.session.events.len(), 1);
    assert_eq!(page2.event_count, 3);
    assert_eq!(page2.turn_count, None);
    assert_eq!(page2.imported.session.events[0].id, "msg-c");

    // Identity and title carry across pages.
    assert_eq!(page1.imported.session.identity.canonical_id, "ses-paged");
    assert_eq!(
        page1.imported.session.identity.source_title.as_deref(),
        Some("Multi")
    );
}

#[test]
fn opencode_malformed_parts_are_preserved_and_reported() {
    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut artifacts = Vec::new();
    let blocks = canonical_blocks_from_parts(
        "message-1",
        &[
            serde_json::json!({"type": "text"}),
            serde_json::json!({"type": "reasoning"}),
            serde_json::json!({
                "type": "file",
                "mime": "image/png",
                "filename": "image.png",
                "url": "data:image/png,not-valid"
            }),
            serde_json::json!({"type": "file", "filename": "missing.txt"}),
        ],
        &mut report,
        &mut artifacts,
    );

    assert_eq!(blocks.len(), 4);
    assert!(blocks
        .iter()
        .all(|block| matches!(block, EventBlock::ProviderPayload { .. })));
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == "opencode_text_part_missing_text"));
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == "opencode_reasoning_part_missing_text"));
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == "opencode_image_part_invalid_data_uri"));
    assert!(report
        .issues
        .iter()
        .any(|issue| issue.code == "opencode_file_part_missing_url"));
}

#[test]
fn opencode_message_without_parts_is_preserved_as_an_event() {
    let imported = imported_session_from_data(
        "session-1",
        (
            serde_json::json!({"id": "session-1", "title": "Empty message"}),
            vec![(
                Some(1),
                serde_json::json!({"id": "message-1", "role": "assistant"}),
            )],
            HashMap::new(),
        ),
    )
    .unwrap();

    assert!(matches!(
        imported.session.events[0].blocks.as_slice(),
        [EventBlock::ProviderPayload { kind, .. }]
            if kind == "message_without_mappable_parts"
    ));
    assert!(imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "opencode_message_without_mappable_parts"));
}

#[test]
fn maps_opencode_error_finish_to_failed_boundary() {
    let mut parts = HashMap::new();
    parts.insert(
        "user-1".to_string(),
        vec![serde_json::json!({"type": "text", "text": "Build it"})],
    );
    parts.insert(
        "assistant-1".to_string(),
        vec![serde_json::json!({"type": "text", "text": "Failed"})],
    );
    let imported = imported_session_from_data(
        "session-1",
        (
            serde_json::json!({"id": "session-1", "title": "Build it"}),
            vec![
                (Some(1), serde_json::json!({"id": "user-1", "role": "user"})),
                (
                    Some(2),
                    serde_json::json!({
                        "id": "assistant-1",
                        "role": "assistant",
                        "finish": "error"
                    }),
                ),
            ],
            parts,
        ),
    )
    .unwrap();

    let assistant = imported
        .session
        .events
        .iter()
        .find(|event| event.id == "assistant-1")
        .unwrap();
    assert_eq!(assistant.links.provider_turn_id, None);
    assert_eq!(assistant.links.turn_boundary, Some(TurnBoundary::Failed));
}

struct TestOpenCodeDirGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl Drop for TestOpenCodeDirGuard {
    fn drop(&mut self) {
        set_test_opencode_mutation_failure(None);
        set_test_opencode_dir(None);
    }
}

fn use_test_opencode_dir(path: PathBuf) -> TestOpenCodeDirGuard {
    let lock = lock_test_opencode_state();
    set_test_opencode_dir(Some(path));
    TestOpenCodeDirGuard { _lock: lock }
}

struct NativeOpenCodeFixture {
    session_path: PathBuf,
    message_path: PathBuf,
    part_path: PathBuf,
    orphan_part_path: PathBuf,
    original_session_bytes: Vec<u8>,
}

fn write_native_opencode_fixture(opencode_dir: &Path, session_id: &str) -> NativeOpenCodeFixture {
    std::fs::create_dir_all(opencode_dir).unwrap();
    let conn = Connection::open(opencode_dir.join("opencode.db")).unwrap();
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        CREATE TABLE project (
            id TEXT PRIMARY KEY,
            worktree TEXT NOT NULL
        );
        CREATE TABLE session (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            parent_id TEXT,
            slug TEXT NOT NULL,
            directory TEXT NOT NULL,
            title TEXT NOT NULL,
            version TEXT NOT NULL,
            share_url TEXT,
            summary_additions INTEGER,
            summary_deletions INTEGER,
            summary_files INTEGER,
            summary_diffs TEXT,
            revert TEXT,
            permission TEXT,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            time_compacting INTEGER,
            time_archived INTEGER,
            workspace_id TEXT,
            path TEXT,
            agent TEXT,
            model TEXT,
            FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
        );
        CREATE TABLE message (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
        );
        CREATE TABLE part (
            id TEXT PRIMARY KEY,
            message_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL,
            FOREIGN KEY (message_id) REFERENCES message(id) ON DELETE CASCADE
        );
        CREATE TABLE todo (
            session_id TEXT NOT NULL,
            content TEXT NOT NULL,
            status TEXT NOT NULL,
            priority TEXT NOT NULL,
            position INTEGER NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            PRIMARY KEY (session_id, position),
            FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
        );
        CREATE TABLE session_share (
            session_id TEXT PRIMARY KEY,
            id TEXT NOT NULL,
            secret TEXT NOT NULL,
            url TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
        );
        CREATE TABLE session_message (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            type TEXT NOT NULL,
            time_created INTEGER NOT NULL,
            time_updated INTEGER NOT NULL,
            data TEXT NOT NULL,
            FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE CASCADE
        );
        ",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO project (id, worktree) VALUES (?1, ?2)",
        ["project-1", "/tmp/project"],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session (
            id, project_id, parent_id, slug, directory, title, version,
            share_url, summary_additions, summary_deletions, summary_files,
            summary_diffs, revert, permission, time_created, time_updated,
            time_compacting, time_archived, workspace_id, path, agent, model
         ) VALUES (
            ?1, 'project-1', NULL, 'before-slug', '/tmp/project', 'Before',
            '1.14.39', 'https://share.test', 3, 4, 5, 'diffs', 'revert',
            'permission', 1700000000000, 1700000000100, NULL, NULL,
            'workspace-1', '/tmp/project', 'build', 'gpt-5.4'
         )",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO message (
            id, session_id, time_created, time_updated, data
         ) VALUES (
            'msg-1', ?1, 1700000000010, 1700000000011, '{\"role\":\"user\"}'
         )",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO part (
            id, message_id, session_id, time_created, time_updated, data
         ) VALUES (
            'part-1', 'msg-1', ?1, 1700000000020, 1700000000021,
            '{\"type\":\"text\",\"text\":\"original\"}'
         )",
        [session_id],
    )
    .unwrap();
    for (position, content) in [(0_i64, "first"), (1_i64, "second")] {
        conn.execute(
            "INSERT INTO todo (
                session_id, content, status, priority, position,
                time_created, time_updated
             ) VALUES (?1, ?2, 'pending', 'high', ?3, 1700000000030, 1700000000031)",
            rusqlite::params![session_id, content, position],
        )
        .unwrap();
    }
    conn.execute(
        "INSERT INTO session_share (
            session_id, id, secret, url, time_created, time_updated
         ) VALUES (?1, 'share-1', 'secret-1', 'https://share.test/1',
                   1700000000040, 1700000000041)",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO session_message (
            id, session_id, type, time_created, time_updated, data
         ) VALUES ('session-message-1', ?1, 'summary',
                   1700000000050, 1700000000051, '{\"summary\":\"exact\"}')",
        [session_id],
    )
    .unwrap();

    let session_dir = opencode_dir
        .join("storage")
        .join("session")
        .join("project-1");
    let message_dir = opencode_dir
        .join("storage")
        .join("message")
        .join(session_id);
    let part_dir = opencode_dir.join("storage").join("part").join("msg-1");
    let orphan_part_dir = opencode_dir.join("storage").join("part").join("msg-orphan");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::create_dir_all(&message_dir).unwrap();
    std::fs::create_dir_all(&part_dir).unwrap();
    std::fs::create_dir_all(&orphan_part_dir).unwrap();

    let original_session_bytes = format!(
        "{{\n  \"id\": \"{session_id}\",\n  \"projectID\": \"project-1\",\n  \"directory\": \"/tmp/project\",\n  \"title\": \"Before\",\n  \"time\": {{\"created\": 1700000000000, \"updated\": 1700000000100}}\n}}\n"
    )
    .into_bytes();
    let session_path = session_dir.join(format!("{session_id}.json"));
    let message_path = message_dir.join("msg-1.json");
    let part_path = part_dir.join("part-1.json");
    let orphan_part_path = orphan_part_dir.join("part-orphan.json");
    std::fs::write(&session_path, &original_session_bytes).unwrap();
    std::fs::write(
        &message_path,
        format!("{{\"id\":\"msg-1\",\"sessionID\":\"{session_id}\",\"role\":\"user\"}}\n"),
    )
    .unwrap();
    std::fs::write(
        &part_path,
        format!(
            "{{\"id\":\"part-1\",\"messageID\":\"msg-1\",\"sessionID\":\"{session_id}\",\"type\":\"text\",\"text\":\"original\"}}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        &orphan_part_path,
        format!(
            "{{\"id\":\"part-orphan\",\"messageID\":\"msg-orphan\",\"sessionID\":\"{session_id}\",\"type\":\"text\",\"text\":\"orphan\"}}\n"
        ),
    )
    .unwrap();

    NativeOpenCodeFixture {
        session_path,
        message_path,
        part_path,
        orphan_part_path,
        original_session_bytes,
    }
}

#[test]
fn scan_sessions_uses_fingerprintable_database_source_locator() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    write_native_opencode_fixture(opencode_dir.path(), "ses_locator");

    let sessions = OpenCodeProvider.scan_sessions().unwrap();
    let session = sessions
        .iter()
        .find(|session| session.session_id == "ses_locator")
        .unwrap();

    assert_eq!(
        session.source_path.as_deref(),
        Some(
            format!(
                "{}#session=ses_locator",
                opencode_dir.path().join("opencode.db").to_string_lossy()
            )
            .as_str()
        )
    );
    assert!(OpenCodeProvider
        .session_source_fingerprint(session.source_path.as_deref().unwrap())
        .unwrap()
        .is_some());
    let imported = OpenCodeProvider
        .import_session(session.source_path.as_deref().unwrap())
        .unwrap();
    assert_eq!(
        imported.session.provenance.primary_source.session_id,
        "ses_locator"
    );
    assert_eq!(
        imported.session.provenance.primary_source.source_path,
        session.source_path
    );
}

#[test]
fn scan_sessions_discovers_filesystem_only_source_plane() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let fixture = write_native_opencode_fixture(opencode_dir.path(), "ses_filesystem_only");
    std::fs::remove_file(opencode_dir.path().join("opencode.db")).unwrap();

    let sessions = OpenCodeProvider.scan_sessions().unwrap();
    let session = sessions
        .iter()
        .find(|session| session.session_id == "ses_filesystem_only")
        .unwrap();

    assert_eq!(
        session.source_path.as_deref(),
        Some(fixture.session_path.to_string_lossy().as_ref())
    );
    let meta = OpenCodeProvider
        .get_session_meta("ses_filesystem_only")
        .unwrap()
        .unwrap();
    assert_eq!(meta.session_id, session.session_id);
    assert_eq!(meta.source_path, session.source_path);
}

#[test]
fn scan_sessions_reports_corrupt_database() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    std::fs::write(opencode_dir.path().join("opencode.db"), b"not sqlite").unwrap();

    let error = OpenCodeProvider.scan_sessions().unwrap_err();

    assert!(error.to_string().contains("file is not a database"));
}

#[test]
fn import_session_reads_the_explicit_source_plane() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let fixture = write_native_opencode_fixture(opencode_dir.path(), "ses_source_plane");
    std::fs::write(
        &fixture.session_path,
        serde_json::json!({
            "id": "ses_source_plane",
            "projectID": "project-1",
            "directory": "/tmp/project",
            "title": "Filesystem title",
            "time": {"created": 1700000000000_i64, "updated": 1700000000100_i64}
        })
        .to_string(),
    )
    .unwrap();

    let from_database = OpenCodeProvider
        .import_session(&opencode_db_session_source_locator("ses_source_plane"))
        .unwrap();
    let from_filesystem = OpenCodeProvider
        .import_session(fixture.session_path.to_string_lossy().as_ref())
        .unwrap();

    assert_eq!(
        from_database.session.identity.source_title.as_deref(),
        Some("Before")
    );
    assert_eq!(
        from_filesystem.session.identity.source_title.as_deref(),
        Some("Filesystem title")
    );
}

#[test]
fn import_session_uses_database_path_from_locator() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    write_native_opencode_fixture(opencode_dir.path(), "ses_database_path");
    let default_database = opencode_dir.path().join("opencode.db");
    let alternate_database = opencode_dir.path().join("alternate.db");
    std::fs::rename(&default_database, &alternate_database).unwrap();
    std::fs::write(&default_database, b"not sqlite").unwrap();
    let locator = format!(
        "{}#session=ses_database_path",
        alternate_database.to_string_lossy()
    );

    let imported = OpenCodeProvider.import_session(&locator).unwrap();

    assert_eq!(
        imported.session.identity.source_title.as_deref(),
        Some("Before")
    );
    assert_eq!(
        imported
            .session
            .provenance
            .primary_source
            .source_path
            .as_deref(),
        Some(locator.as_str())
    );
}

#[test]
fn parse_session_file_keeps_actual_json_source_path() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ses_file.json");
    std::fs::write(
        &path,
        serde_json::json!({
            "id": "ses_file",
            "directory": "/tmp/project",
            "title": "File session",
            "time": {"updated": 1_790_000_000_000_i64}
        })
        .to_string(),
    )
    .unwrap();

    let session = parse_session_file(&path).unwrap();

    assert_eq!(session.session_id, "ses_file");
    assert_eq!(
        session.source_path.as_deref(),
        Some(path.to_string_lossy().as_ref())
    );
}

fn session_owned_row_counts(opencode_dir: &Path, session_id: &str) -> Vec<i64> {
    let conn = Connection::open(opencode_dir.join("opencode.db")).unwrap();
    [
        "session",
        "message",
        "part",
        "todo",
        "session_share",
        "session_message",
    ]
    .into_iter()
    .map(|table| {
        let column = if table == "session" {
            "id"
        } else {
            "session_id"
        };
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE {column} = ?1"),
            [session_id],
            |row| row.get(0),
        )
        .unwrap()
    })
    .collect()
}

#[test]
fn delete_backup_restores_exact_opencode_database_and_filesystem() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-delete-backup";
    let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
    let backup_root = opencode_dir.path().join("backups");
    let binary_payload = vec![0_u8, 1, 2, 127, 128, 255];
    let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
    conn.execute(
        "UPDATE session_message SET data = ?1 WHERE id = 'session-message-1'",
        [binary_payload.as_slice()],
    )
    .unwrap();
    drop(conn);

    let backup = create_opencode_session_backup(
        ProviderSourceMutation::Delete,
        "operation-delete-1",
        session_id,
        &backup_root,
    )
    .unwrap();
    let backup_conn = Connection::open(backup.backup_path.join(OPENCODE_BACKUP_DB_PATH)).unwrap();
    let backed_up_payload: Vec<u8> = backup_conn
        .query_row(
            "SELECT data FROM session_message WHERE id = 'session-message-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(backed_up_payload, binary_payload);
    drop(backup_conn);
    delete_opencode_session(session_id).unwrap();

    assert_eq!(
        session_owned_row_counts(opencode_dir.path(), session_id),
        vec![0, 0, 0, 0, 0, 0]
    );
    assert!(!fixture.session_path.exists());
    assert!(!fixture.message_path.exists());
    assert!(!fixture.part_path.exists());
    assert!(!fixture.orphan_part_path.exists());

    restore_opencode_session_backup(&backup).unwrap();

    assert_eq!(
        session_owned_row_counts(opencode_dir.path(), session_id),
        vec![1, 1, 1, 2, 1, 1]
    );
    assert_eq!(
        std::fs::read(&fixture.session_path).unwrap(),
        fixture.original_session_bytes
    );
    assert!(fixture.message_path.exists());
    assert!(fixture.part_path.exists());
    assert!(fixture.orphan_part_path.exists());
    let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
    let restored_payload: Vec<u8> = conn
        .query_row(
            "SELECT data FROM session_message WHERE id = 'session-message-1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(restored_payload, binary_payload);

    let metadata: OpenCodeSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(backup.backup_path.join("metadata.json")).unwrap())
            .unwrap();
    let tables = metadata
        .sqlite_tables
        .iter()
        .map(|table| table.table.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(
        tables,
        HashSet::from([
            "session",
            "message",
            "part",
            "todo",
            "session_share",
            "session_message",
        ])
    );
    let row_counts = metadata
        .sqlite_tables
        .iter()
        .map(|table| (table.table.as_str(), table.row_count))
        .collect::<HashMap<_, _>>();
    assert_eq!(row_counts["session"], 1);
    assert_eq!(row_counts["message"], 1);
    assert_eq!(row_counts["part"], 1);
    assert_eq!(row_counts["todo"], 2);
    assert_eq!(row_counts["session_share"], 1);
    assert_eq!(row_counts["session_message"], 1);
}

#[test]
fn native_replace_preserves_opencode_identity_and_session_rows() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-native-replace";
    let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
    let source = opencode_db_session_source_locator(session_id);
    let mut session = import_canonical_session_from_source(session_id, &source)
        .unwrap()
        .session;
    session.events.clear();

    OpenCodeProvider
        .replace_session(session_id, &session)
        .unwrap();

    assert_eq!(
        import_canonical_session_from_source(session_id, &source)
            .unwrap()
            .session
            .identity
            .canonical_id,
        session_id
    );
    assert_eq!(
        session_owned_row_counts(opencode_dir.path(), session_id),
        vec![1, 0, 0, 2, 1, 1]
    );
    assert!(fixture.session_path.exists());
    assert!(!fixture.message_path.exists());
    assert!(!fixture.part_path.exists());
    assert!(!fixture.orphan_part_path.exists());
}

#[test]
fn replace_failure_can_restore_exact_opencode_source() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-replace-rollback";
    let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
    let source = opencode_db_session_source_locator(session_id);
    let mut session = import_canonical_session_from_source(session_id, &source)
        .unwrap()
        .session;
    session.events.clear();
    let backup = create_opencode_session_backup(
        ProviderSourceMutation::Replace,
        "operation-replace-rollback",
        session_id,
        &opencode_dir.path().join("backups"),
    )
    .unwrap();

    set_test_opencode_mutation_failure(Some(ProviderSourceMutation::Replace));
    assert!(OpenCodeProvider
        .replace_session(session_id, &session)
        .is_err());
    restore_opencode_session_backup(&backup).unwrap();

    assert_eq!(
        session_owned_row_counts(opencode_dir.path(), session_id),
        vec![1, 1, 1, 2, 1, 1]
    );
    assert_eq!(
        std::fs::read(&fixture.session_path).unwrap(),
        fixture.original_session_bytes
    );
    assert!(fixture.message_path.exists());
    assert!(fixture.part_path.exists());
    assert!(fixture.orphan_part_path.exists());
}

#[test]
fn replace_backup_restores_exact_opencode_source() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-replace-backup";
    let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
    let backup = create_opencode_session_backup(
        ProviderSourceMutation::Replace,
        "operation-replace-1",
        session_id,
        &opencode_dir.path().join("backups"),
    )
    .unwrap();

    delete_opencode_session(session_id).unwrap();
    restore_opencode_session_backup(&backup).unwrap();

    assert_eq!(
        session_owned_row_counts(opencode_dir.path(), session_id),
        vec![1, 1, 1, 2, 1, 1]
    );
    assert_eq!(
        std::fs::read(&fixture.session_path).unwrap(),
        fixture.original_session_bytes
    );
    assert!(fixture.message_path.exists());
    assert!(fixture.part_path.exists());
    assert!(fixture.orphan_part_path.exists());
}

#[test]
fn rename_backup_restores_only_opencode_session_owned_resources() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-rename-backup";
    let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
    let backup_root = opencode_dir.path().join("backups");

    let backup = create_opencode_session_backup(
        ProviderSourceMutation::Rename,
        "operation-rename-1",
        session_id,
        &backup_root,
    )
    .unwrap();
    rename_opencode_session(session_id, "After").unwrap();

    let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
    conn.execute(
        "UPDATE message SET data = '{\"role\":\"user\",\"changed\":true}' WHERE id = 'msg-1'",
        [],
    )
    .unwrap();
    drop(conn);
    std::fs::write(&fixture.message_path, b"changed message state").unwrap();
    std::fs::write(&fixture.part_path, b"changed part state").unwrap();

    restore_opencode_session_backup(&backup).unwrap();

    let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
    let session: (String, i64) = conn
        .query_row(
            "SELECT title, time_updated FROM session WHERE id = ?1",
            [session_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    let message_data: String = conn
        .query_row("SELECT data FROM message WHERE id = 'msg-1'", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(session, ("Before".to_string(), 1_700_000_000_100));
    assert_eq!(message_data, "{\"role\":\"user\",\"changed\":true}");
    assert_eq!(
        std::fs::read(&fixture.session_path).unwrap(),
        fixture.original_session_bytes
    );
    assert_eq!(
        std::fs::read(&fixture.message_path).unwrap(),
        b"changed message state"
    );
    assert_eq!(
        std::fs::read(&fixture.part_path).unwrap(),
        b"changed part state"
    );

    let metadata: OpenCodeSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(backup.backup_path.join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(metadata.sqlite_tables.len(), 1);
    assert_eq!(metadata.sqlite_tables[0].table, "session");
    assert_eq!(metadata.filesystem_entries.len(), 1);
}

#[test]
fn opencode_backup_contract_and_capabilities_are_truthful() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-backup-contract";
    write_native_opencode_fixture(opencode_dir.path(), session_id);
    let backup = create_opencode_session_backup(
        ProviderSourceMutation::Delete,
        "operation-contract-1",
        session_id,
        &opencode_dir.path().join("backups"),
    )
    .unwrap();

    let capabilities = OpenCodeProvider.capabilities();
    assert!(capabilities.backup_support.before_write);
    assert!(capabilities.backup_support.restore);
    assert!(!capabilities.backup_support.sync_only);
    assert_eq!(backup.mutation, ProviderSourceMutation::Delete);
    assert_eq!(backup.operation_id, "operation-contract-1");
    assert_eq!(backup.provider_session_id, session_id);
    assert_eq!(
        backup.source_path,
        opencode_dir.path().canonicalize().unwrap()
    );
    assert_eq!(backup.format, OPENCODE_BACKUP_FORMAT);
    assert_eq!(backup.mime_type, OPENCODE_BACKUP_MIME);
    assert_eq!(
        backup
            .restore_metadata
            .get("restore_mode")
            .and_then(Value::as_str),
        Some("opencode_session_restore")
    );
}

#[test]
fn delete_backup_rejects_non_cascade_session_relationships_before_write() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-non-cascade";
    write_native_opencode_fixture(opencode_dir.path(), session_id);
    let conn = Connection::open(opencode_dir.path().join("opencode.db")).unwrap();
    conn.execute_batch(
        "
        CREATE TABLE retained_session_reference (
            id TEXT PRIMARY KEY,
            session_id TEXT,
            FOREIGN KEY (session_id) REFERENCES session(id) ON DELETE SET NULL
        );
        INSERT INTO retained_session_reference (id, session_id)
        VALUES ('reference-1', 'ses-non-cascade');
        ",
    )
    .unwrap();
    drop(conn);

    let error = create_opencode_session_backup(
        ProviderSourceMutation::Delete,
        "operation-non-cascade",
        session_id,
        &opencode_dir.path().join("backups"),
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains("unsupported ON DELETE SET NULL behavior"));
    assert_eq!(
        session_owned_row_counts(opencode_dir.path(), session_id),
        vec![1, 1, 1, 2, 1, 1]
    );
}

#[test]
fn backup_registration_failure_prevents_opencode_provider_write() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-registration-failure";
    let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
    let backup_root = opencode_dir.path().join("backups");
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
        session_owned_row_counts(opencode_dir.path(), session_id),
        vec![1, 1, 1, 2, 1, 1]
    );
    assert_eq!(
        std::fs::read(&fixture.session_path).unwrap(),
        fixture.original_session_bytes
    );
    assert!(backup_root
        .join(PROVIDER_ID)
        .join("operation-registration-failure")
        .exists());
}

#[test]
fn partial_opencode_delete_failure_restores_registered_backup() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-partial-delete";
    let fixture = write_native_opencode_fixture(opencode_dir.path(), session_id);
    let backup_root = opencode_dir.path().join("backups");
    let mut artifact_conn = Connection::open_in_memory().unwrap();
    local_store::configure_connection(&artifact_conn).unwrap();
    local_store::apply_schema(&mut artifact_conn).unwrap();
    set_test_opencode_mutation_failure(Some(ProviderSourceMutation::Delete));

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
        session_owned_row_counts(opencode_dir.path(), session_id),
        vec![1, 1, 1, 2, 1, 1]
    );
    assert_eq!(
        std::fs::read(&fixture.session_path).unwrap(),
        fixture.original_session_bytes
    );
    assert!(fixture.message_path.exists());
    assert!(fixture.part_path.exists());
    assert!(fixture.orphan_part_path.exists());
}

#[test]
fn database_import_uses_stable_message_and_part_order() {
    let opencode_dir = tempdir().unwrap();
    let _guard = use_test_opencode_dir(opencode_dir.path().to_path_buf());
    let session_id = "ses-stable-order";
    write_native_opencode_fixture(opencode_dir.path(), session_id);
    let db_path = opencode_dir.path().join("opencode.db");
    let conn = Connection::open(&db_path).unwrap();
    conn.execute("DELETE FROM part WHERE session_id = ?1", [session_id])
        .unwrap();
    conn.execute("DELETE FROM message WHERE session_id = ?1", [session_id])
        .unwrap();
    conn.execute(
        "INSERT INTO message (id, session_id, time_created, time_updated, data)
         VALUES ('msg-b', ?1, 1700000000010, 1700000000011,
                 '{\"role\":\"assistant\"}')",
        [session_id],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO message (id, session_id, time_created, time_updated, data)
         VALUES ('msg-a', ?1, 1700000000010, 1700000000011,
                 '{\"role\":\"user\",\"time\":{\"created\":9223372036854775807}}')",
        [session_id],
    )
    .unwrap();
    for (part_id, message_id, text) in [
        ("part-z", "msg-a", "second block"),
        ("part-a", "msg-a", "first block"),
        ("part-b", "msg-b", "assistant block"),
    ] {
        conn.execute(
            "INSERT INTO part (
                id, message_id, session_id, time_created, time_updated, data
             ) VALUES (?1, ?2, ?3, 1700000000020, 1700000000021, ?4)",
            rusqlite::params![
                part_id,
                message_id,
                session_id,
                serde_json::json!({ "type": "text", "text": text }).to_string()
            ],
        )
        .unwrap();
    }
    drop(conn);

    let first = imported_session_from_data(
        session_id,
        load_session_from_db_path(&db_path, session_id).unwrap(),
    )
    .unwrap();
    let second = imported_session_from_data(
        session_id,
        load_session_from_db_path(&db_path, session_id).unwrap(),
    )
    .unwrap();

    assert_eq!(
        serde_json::to_value(&first.session.events).unwrap(),
        serde_json::to_value(&second.session.events).unwrap()
    );
    assert_eq!(
        first
            .session
            .events
            .iter()
            .map(|event| event.id.as_str())
            .collect::<Vec<_>>(),
        vec!["msg-a", "msg-b"]
    );
    assert_eq!(
        first.session.events[0].timestamp.timestamp_millis(),
        1_700_000_000_010
    );
    assert_eq!(
        first.session.events[1].timestamp.timestamp_millis(),
        1_700_000_000_010
    );
    assert!(matches!(
        first.session.events[0].blocks.as_slice(),
        [EventBlock::Text { text: first }, EventBlock::Text { text: second }]
            if first == "first block" && second == "second block"
    ));
}

#[test]
fn opencode_message_data_preserves_model_provider_metadata() {
    let event = SessionEvent {
        id: "source-message".to_string(),
        kind: SessionEventKind::Message,
        role: EventRole::Assistant,
        blocks: vec![EventBlock::Text {
            text: "hello".to_string(),
        }],
        timestamp: Utc
            .timestamp_millis_opt(1_700_000_000_000)
            .single()
            .unwrap(),
        links: EventLinks::default(),
        metadata: EventMetadata {
            source: EventSource {
                provider_id: "codex".to_string(),
                original_id: None,
                original_role: None,
                phase: None,
            },
            model: Some("gpt-5.4".to_string()),
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: BTreeMap::new(),
        },
    };

    let data = build_opencode_message_data_from_event(
        "ses_test",
        &event,
        "msg_test",
        "assistant",
        Some("msg_parent"),
        "/tmp/project",
    );
    let obj = data.as_object().expect("message data should be an object");

    assert_eq!(obj.get("role").and_then(Value::as_str), Some("assistant"));
    assert_eq!(
        obj.get("parentID").and_then(Value::as_str),
        Some("msg_parent")
    );
    assert_eq!(
        obj.get("providerID").and_then(Value::as_str),
        Some("openai")
    );
    assert_eq!(obj.get("modelID").and_then(Value::as_str), Some("gpt-5.4"));
    assert_eq!(
        obj.get("model")
            .and_then(|value| value.get("providerID"))
            .and_then(Value::as_str),
        Some("openai")
    );
    assert_eq!(obj.get("agent").and_then(Value::as_str), Some("build"));
    assert!(obj.get("path").is_some());
    assert!(obj.get("tokens").is_some());
}

#[test]
fn provider_payload_block_is_skipped_in_opencode_part_export() {
    let block = EventBlock::ProviderPayload {
        kind: "internal".to_string(),
        payload: serde_json::json!({"id": "hidden"}),
    };

    assert!(canonical_block_to_opencode_part(
        "ses_test",
        "msg_test",
        "prt_test",
        &block,
        1_700_000_000_001,
    )
    .is_none());
}

#[test]
fn compressed_segment_exports_as_native_opencode_compaction() {
    let event = SessionEvent {
        id: "compressed-source".to_string(),
        kind: SessionEventKind::Message,
        role: EventRole::Assistant,
        blocks: vec![EventBlock::Compressed {
            source_provider_id: "opencode".to_string(),
            summary: "portable summary".to_string(),
            source_event_ids: vec!["old-1".to_string(), "old-2".to_string()],
            source_event_count: Some(2),
            archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
        }],
        timestamp: Utc
            .timestamp_millis_opt(1_700_000_000_000)
            .single()
            .unwrap(),
        links: EventLinks::default(),
        metadata: EventMetadata {
            source: EventSource {
                provider_id: "memorph".to_string(),
                original_id: None,
                original_role: None,
                phase: Some("compression".to_string()),
            },
            model: Some("gpt-5.4".to_string()),
            usage: None,
            fidelity: MappingDisposition::Normalized,
            provider_ext: BTreeMap::new(),
        },
    };
    let mut last_user_msg_id = None;
    let mut messages = Vec::new();
    let mut parts = Vec::new();
    let segment = compression::compressed_segment(&event).expect("canonical compressed segment");

    append_compressed_opencode_segment(
        "ses_test",
        &event,
        segment,
        "/tmp/project",
        &mut last_user_msg_id,
        &mut messages,
        &mut parts,
    );

    assert_eq!(messages.len(), 2);
    assert_eq!(parts.len(), 2);
    assert_eq!(
        messages[0].1,
        Utc.timestamp_millis_opt(1_700_000_000_000)
            .single()
            .unwrap()
            .timestamp_millis()
    );
    assert_eq!(
        messages[0].2.get("role").and_then(Value::as_str),
        Some("user")
    );
    assert_eq!(
        messages[0].2.get("mode").and_then(Value::as_str),
        Some("compaction")
    );
    assert_eq!(
        parts[0].3.get("type").and_then(Value::as_str),
        Some("compaction")
    );
    assert_eq!(
        parts[0]
            .3
            .get("memorph")
            .and_then(|value| value.get("sourceProviderID"))
            .and_then(Value::as_str),
        Some("opencode")
    );
    assert_eq!(
        parts[0]
            .3
            .get("memorph")
            .and_then(|value| value.get("sourceEventCount"))
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        parts[0]
            .3
            .get("memorph")
            .and_then(|value| value.get("archiveRef"))
            .and_then(Value::as_str),
        Some("memorph-archive://s1/archive.json.gz")
    );
    assert_eq!(
        parts[0]
            .3
            .get("memorph")
            .and_then(|value| value.get("retrievalHint"))
            .and_then(Value::as_str),
        Some(
            "Retrieve specific details with: memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"
        )
    );
    assert_eq!(
        messages[1].2.get("summary").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        messages[1].2.get("parentID").and_then(Value::as_str),
        Some(messages[0].0.as_str())
    );
    assert_eq!(parts[1].3.get("type").and_then(Value::as_str), Some("text"));
    assert_eq!(
        parts[1].3.get("text").and_then(Value::as_str),
        Some("portable summary")
    );
}
