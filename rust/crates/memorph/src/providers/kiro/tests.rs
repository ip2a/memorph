use super::*;
use crate::provider::{
    PageStrategy, ProviderActivitySupport, ProviderBackupSupport, ProviderWriteRisk, WriteRiskLevel,
};
use crate::storage::local_store;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::sync::{MutexGuard, OnceLock};

static TEST_KIRO_TEST_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

struct TestKiroSessionsDirGuard {
    _lock: MutexGuard<'static, ()>,
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

impl Drop for TestKiroSessionsDirGuard {
    fn drop(&mut self) {
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        set_test_kiro_mutation_failure(None);
        *TEST_KIRO_SESSIONS_DIR
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

fn use_test_kiro_sessions_dir(path: PathBuf) -> TestKiroSessionsDirGuard {
    let lock = TEST_KIRO_TEST_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *TEST_KIRO_SESSIONS_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(path);
    crate::cache::global_cache().invalidate(PROVIDER_ID);
    TestKiroSessionsDirGuard { _lock: lock }
}

fn kiro_audit_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/providers/kiro/fixtures/v1_0_138")
}

fn read_jsonl_values(path: &Path) -> Vec<Result<Value, serde_json::Error>> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

fn copy_tree(source: &Path, target: &Path) -> Result<()> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry = entry?;
        let relative = entry.path().strip_prefix(source)?;
        let destination = target.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&destination)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(entry.path(), destination)?;
        }
    }
    Ok(())
}

fn copy_fixture_sessions() -> Result<tempfile::TempDir> {
    let temp = tempfile::tempdir()?;
    copy_tree(&kiro_audit_fixture_root().join("sessions"), temp.path())?;
    Ok(temp)
}

#[test]
fn kiro_v2_audit_fixture_matches_official_session_directory_contract() {
    let root = kiro_audit_fixture_root();
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("fixture.json")).unwrap()).unwrap();
    assert_eq!(manifest["provider"], "kiro");
    assert_eq!(manifest["source_plane"], "kiro-agent-v2");
    assert_eq!(manifest["observed_ide_version"], "1.0.138");
    assert_eq!(manifest["observed_extension_version"], "1.0.231");
    assert_eq!(manifest["observed_schema_version"], "1.0.0");
    assert_eq!(manifest["observed_data_model_version"], 1);
    assert_eq!(manifest["raw_user_content_committed"], false);
    assert_eq!(manifest["storage_root"], "~/.kiro/sessions");
    assert_eq!(
        manifest["official_artifact_sha256"],
        "29c7541056b4ca6849d73c1062ae1d215a80a9f7fc74a8240cb2bf9b8e1fd68b"
    );

    let session_id = manifest["normal_session_id"].as_str().unwrap();
    let workspace_path = "/workspace/sanitized-project";
    assert_eq!(
        workspace_bucket(&[workspace_path.to_string()]).unwrap(),
        "8f3d1d8bb1bd8116"
    );

    let session_dir = root
        .join("sessions")
        .join("8f3d1d8bb1bd8116")
        .join(session_id);
    let metadata: Value =
        serde_json::from_str(&std::fs::read_to_string(session_dir.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["schemaVersion"], "1.0.0");
    assert_eq!(metadata["dataModelVersion"], 1);
    assert_eq!(metadata["id"], session_id);
    assert_eq!(metadata["workspacePaths"], json!([workspace_path]));
    assert_eq!(metadata["title"], "Sanitized Kiro session");
    assert_eq!(metadata["status"], "completed");

    assert!(session_dir.join("messages.jsonl").is_file());
    assert!(session_dir.join("sub-executions/subexec-1.jsonl").is_file());
    assert!(session_dir
        .join("tool-outputs/tool-1-a1b2c3d4.txt")
        .is_file());
    assert!(session_dir
        .join("snapshots/snap0001/src/example.rs")
        .is_file());
    assert!(session_dir.join("snapshots/snap0001/.hash").is_file());

    let messages = read_jsonl_values(&session_dir.join("messages.jsonl"));
    assert_eq!(messages.len(), 10);
    assert!(messages.iter().all(Result::is_ok));
    let payload_types = messages
        .into_iter()
        .map(Result::unwrap)
        .map(|message| message["payload"]["type"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        payload_types,
        [
            "session_start",
            "turn_start",
            "user",
            "assistant",
            "tool_call",
            "tool_result",
            "assistant",
            "usage_summary",
            "turn_end",
            "session_metadata",
        ]
    );

    let global_id = manifest["global_session_id"].as_str().unwrap();
    let global_dir = root.join("sessions").join("_global").join(global_id);
    let global_metadata: Value =
        serde_json::from_str(&std::fs::read_to_string(global_dir.join("session.json")).unwrap())
            .unwrap();
    assert_eq!(global_metadata["id"], global_id);
    assert_eq!(global_metadata["workspacePaths"], json!([]));
    assert_eq!(
        read_jsonl_values(&global_dir.join("messages.jsonl")).len(),
        4
    );
}

#[test]
fn kiro_v2_audit_fixture_covers_projection_changes_and_invalid_records() {
    let root = kiro_audit_fixture_root();
    let variants = root.join("variants");
    let normal_dir = root
        .join("sessions/8f3d1d8bb1bd8116")
        .join("sess_11111111-1111-4111-8111-111111111111");

    let original_metadata: Value =
        serde_json::from_str(&std::fs::read_to_string(normal_dir.join("session.json")).unwrap())
            .unwrap();
    let updated_metadata: Value = serde_json::from_str(
        &std::fs::read_to_string(variants.join("session.updated.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(original_metadata["id"], updated_metadata["id"]);
    assert_ne!(original_metadata["title"], updated_metadata["title"]);
    assert_ne!(
        original_metadata["lastModifiedAt"],
        updated_metadata["lastModifiedAt"]
    );

    assert_eq!(
        read_jsonl_values(&normal_dir.join("messages.jsonl")).len(),
        10
    );
    assert_eq!(
        read_jsonl_values(&variants.join("messages.updated.jsonl")).len(),
        14
    );
    assert_eq!(
        read_jsonl_values(&normal_dir.join("sub-executions/subexec-1.jsonl")).len(),
        2
    );
    assert_eq!(
        read_jsonl_values(&variants.join("sub-execution.updated.jsonl")).len(),
        3
    );

    let malformed = read_jsonl_values(&variants.join("messages.malformed.jsonl"));
    assert_eq!(malformed.len(), 3);
    assert_eq!(malformed.iter().filter(|value| value.is_ok()).count(), 2);
    assert_eq!(malformed.iter().filter(|value| value.is_err()).count(), 1);

    let unknown = read_jsonl_values(&variants.join("messages.unknown.jsonl"));
    assert_eq!(unknown.len(), 1);
    assert_eq!(
        unknown[0].as_ref().unwrap()["payload"]["type"],
        "future_kiro_payload"
    );
    assert_eq!(
        unknown[0].as_ref().unwrap()["payload"]["futureField"]["preserve"],
        true
    );
}

#[test]
fn current_format_scan_uses_directory_locators_and_truthful_capabilities() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());

    let capabilities = KiroProvider.capabilities();
    assert!(capabilities.scan);
    assert!(capabilities.import);
    assert!(!capabilities.export);
    assert!(capabilities.delete);
    assert!(capabilities.rename);
    assert!(!capabilities.resume);
    assert_eq!(capabilities.scan_strategy, ScanStrategy::FullScan);
    assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);
    assert_eq!(capabilities.storage_shape, StorageShape::Directory);
    assert_eq!(capabilities.turn_quality, TurnQuality::Exact);
    assert_eq!(
        capabilities.import_fidelity.tool_call,
        Some(Fidelity::Preserved)
    );
    assert_eq!(
        capabilities.import_fidelity.provider_payload,
        Some(Fidelity::Preserved)
    );
    assert_eq!(
        capabilities.write_risk,
        ProviderWriteRisk {
            level: WriteRiskLevel::Medium,
            multiple_files: true,
            sqlite: false,
            sidecar_files: true,
            index_repair: false,
        }
    );
    assert_eq!(
        capabilities.backup_support,
        ProviderBackupSupport {
            before_write: true,
            restore: true,
            sync_only: false,
        }
    );
    assert_eq!(
        capabilities.activity_support,
        ProviderActivitySupport {
            hook_events: true,
            runtime_endpoint: true,
            session_activity: true,
        }
    );

    let sessions = KiroProvider.scan_sessions()?;
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions[0].session_id,
        "sess_22222222-2222-4222-8222-222222222222"
    );
    assert_eq!(sessions[0].project_dir, None);
    assert_eq!(
        sessions[1].session_id,
        "sess_11111111-1111-4111-8111-111111111111"
    );
    assert_eq!(sessions[1].title.as_deref(), Some("Sanitized Kiro session"));
    assert_eq!(
        sessions[1].project_dir.as_deref(),
        Some("/workspace/sanitized-project")
    );
    let source_path = PathBuf::from(sessions[1].source_path.as_ref().unwrap());
    assert!(source_path.is_dir());
    assert_eq!(
        source_path.file_name().and_then(|name| name.to_str()),
        Some(sessions[1].session_id.as_str())
    );
    assert_eq!(
        KiroProvider
            .get_session_meta(&sessions[1].session_id)?
            .unwrap()
            .source_path,
        sessions[1].source_path
    );
    assert!(KiroProvider.session_size(&sessions[1].session_id)? > 0);
    assert_eq!(
        KiroProvider
            .import_session(source_path.to_str().unwrap())?
            .session
            .identity
            .canonical_id,
        sessions[1].session_id
    );
    assert_eq!(KiroProvider.data_source_paths(), vec![temp.path()]);
    Ok(())
}

#[test]
fn current_format_full_import_page_keeps_total_counts_and_marks_partial_turns_inferred(
) -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_dir = temp
        .path()
        .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
    let full = KiroProvider.import_session_page(session_dir.to_str().unwrap(), 0, None)?;
    assert_eq!(full.event_count, full.imported.session.events.len());
    assert_eq!(full.turn_count, Some(full.turns.len()));
    assert!(full
        .turns
        .iter()
        .all(|turn| { turn.confidence == crate::session_projection::TurnConfidence::Exact }));

    let page = KiroProvider.import_session_page(session_dir.to_str().unwrap(), 3, Some(2))?;
    assert_eq!(page.event_count, full.event_count);
    assert_eq!(page.message_count, full.message_count);
    assert_eq!(page.turn_count, full.turn_count);
    assert_eq!(page.imported.session.events.len(), 2);
    assert_eq!(page.turns.len(), 1);
    assert_eq!(page.turns[0].provider_turn_id.as_deref(), Some("exec-1"));
    assert_eq!(
        page.turns[0].confidence,
        crate::session_projection::TurnConfidence::Inferred
    );
    Ok(())
}

#[test]
fn current_format_index_and_detail_are_idempotent_source_backed_and_bodyless() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let home = temp.path().join("home");
    fs::create_dir_all(&home)?;
    let _home_guard = TestConfigHomeGuard::new(&home);
    let _kiro_guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_id = "sess_11111111-1111-4111-8111-111111111111";
    let session_dir = temp.path().join("8f3d1d8bb1bd8116").join(session_id);
    let summary = KiroProvider
        .scan_sessions()?
        .into_iter()
        .find(|summary| summary.session_id == session_id)
        .unwrap();
    assert_eq!(
        summary.source_path.as_deref(),
        Some(session_dir.to_string_lossy().as_ref())
    );
    let fingerprint = KiroProvider
        .session_source_fingerprint(summary.source_path.as_deref().unwrap())?
        .unwrap();
    assert!(fingerprint.value.starts_with("kiro-v2:"));
    let full =
        KiroProvider.import_session_page(summary.source_path.as_deref().unwrap(), 0, None)?;
    let expected_turn_count = full.turn_count.unwrap();

    let mut conn = local_store::open_database()?;
    let first = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
        .write_session_summary(
            PROVIDER_ID,
            &summary,
            KiroProvider.capabilities(),
            &fingerprint,
        )?;
    let counts_after_first: (i64, i64, i64, i64) = conn.query_row(
        "SELECT
            (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'kiro'),
            (SELECT COUNT(*) FROM sessions WHERE provider_id = 'kiro'),
            (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'kiro'),
            (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'kiro')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let second = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
        .write_session_summary(
            PROVIDER_ID,
            &summary,
            KiroProvider.capabilities(),
            &fingerprint,
        )?;
    let counts_after_second: (i64, i64, i64, i64) = conn.query_row(
        "SELECT
            (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'kiro'),
            (SELECT COUNT(*) FROM sessions WHERE provider_id = 'kiro'),
            (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'kiro'),
            (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'kiro')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    assert_eq!(first, second);
    assert_eq!(counts_after_first, counts_after_second);
    assert_eq!(counts_after_second.0, 1);
    assert_eq!(counts_after_second.1, 1);
    assert_eq!(counts_after_second.2, 1);

    let (source_path, storage_shape, source_cursor): (String, String, String) = conn.query_row(
        "SELECT source_path, storage_shape, source_cursor
             FROM session_sources WHERE id = ?1",
        [&first.source_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_eq!(source_path, session_dir.to_string_lossy());
    assert_eq!(storage_shape, "directory");
    assert_eq!(source_cursor, fingerprint.value);
    let snapshot_json: String = conn.query_row(
        "SELECT snapshot_json FROM session_snapshots WHERE session_id = ?1",
        [&first.canonical_session_id],
        |row| row.get(0),
    )?;
    let snapshot_json: Value = serde_json::from_str(&snapshot_json)?;
    let snapshot_keys = snapshot_json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        snapshot_keys,
        BTreeSet::from(["index_version", "source_fingerprint"])
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
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
    assert!(detail.events.is_empty());
    assert!(detail.turns.is_empty());
    assert_eq!(detail.event_count, full.event_count);
    assert_eq!(detail.message_count, full.message_count);
    assert!(!detail.stale);
    assert_eq!(
        detail.source_path.as_deref(),
        Some(session_dir.to_string_lossy().as_ref())
    );
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
    assert_eq!(cached_counts.2, expected_turn_count as i64);
    assert_eq!(cached_counts.3, 1);
    drop(conn);

    fs::OpenOptions::new()
        .append(true)
        .open(session_dir.join("messages.jsonl"))?
        .write_all(b"\n")?;
    let stale_detail =
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
    assert!(stale_detail.stale);

    fs::remove_dir_all(&session_dir)?;
    let groups = crate::core::projection::list_sessions(&crate::core::SessionListParams {
        all: true,
        providers: vec![PROVIDER_ID.to_string()],
        cwd: None,
        include_message_counts: true,
        limit: None,
        offset: None,
        sort: crate::core::SessionListSort::Recent,
    })?;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].sessions.len(), 1);
    assert_eq!(groups[0].sessions[0].session_id, session_id);
    assert_eq!(
        groups[0].sessions[0].message_count,
        Some(full.message_count)
    );
    let error =
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(1))
            .unwrap_err();
    assert!(format!("{error:#}").contains("Session source is missing"));
    Ok(())
}

#[test]
fn current_format_bootstrap_stale_and_system_sync_are_incremental_and_bodyless() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let home = tempfile::tempdir()?;
    let _home_guard = TestConfigHomeGuard::new(home.path());
    let _kiro_guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_id = "sess_11111111-1111-4111-8111-111111111111";
    let session_dir = temp.path().join("8f3d1d8bb1bd8116").join(session_id);

    let first = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::Cli,
    )?;
    assert_eq!(first.scanned_providers, 1);
    assert_eq!(first.discovered_sessions, 2);
    assert_eq!(first.projected_sessions, 2);
    assert_eq!(first.unchanged_sessions, 0);
    assert!(first.failures.is_empty());

    let detail =
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
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
         WHERE ss.provider_id = 'kiro' AND s.provider_session_id = ?1",
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
        "SELECT src.source_cursor
         FROM session_sources src
         WHERE src.provider_id = 'kiro' AND src.provider_session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    assert_eq!(initial.0, "Sanitized Kiro session");
    assert_eq!(initial.1, "/workspace/sanitized-project");
    assert_eq!(initial.2, 1);
    assert_eq!(initial.3, 0);
    assert_eq!(initial.4, 1);
    assert_eq!(initial.5, 0);
    drop(conn);

    let unchanged = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )?;
    assert_eq!(unchanged.scanned_providers, 1);
    assert_eq!(unchanged.discovered_sessions, 2);
    assert_eq!(unchanged.projected_sessions, 0);
    assert_eq!(unchanged.unchanged_sessions, 2);
    assert!(unchanged.failures.is_empty());

    let conn = local_store::open_database()?;
    let unchanged_state: (i64, i64) = conn.query_row(
        "SELECT src.scan_generation, ss.counts_complete
         FROM session_snapshots ss
         JOIN sessions s ON s.id = ss.session_id
         JOIN session_sources src ON src.id = s.primary_source_id
         WHERE ss.provider_id = 'kiro' AND s.provider_session_id = ?1",
        [session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(unchanged_state, (1, 1));
    drop(conn);

    fs::OpenOptions::new()
        .append(true)
        .open(session_dir.join("messages.jsonl"))?
        .write_all(b"\n")?;
    let stale = crate::core::projection::refresh_projected_session_staleness(
        crate::storage::activity_store::ActivityActor::System,
    )?;
    assert_eq!(stale.checked_sources, 2);
    assert_eq!(stale.fresh_snapshots, 1);
    assert_eq!(stale.stale_snapshots, 1);
    assert_eq!(stale.missing_sources, 0);

    let refreshed = crate::core::projection::reproject_stale_sessions(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )?;
    assert_eq!(refreshed.candidate_snapshots, 1);
    assert_eq!(refreshed.reprojected_snapshots, 1);
    assert_eq!(refreshed.missing_sources, 0);
    assert!(refreshed.failures.is_empty());

    let conn = local_store::open_database()?;
    let after_messages: (String, i64, i64) = conn.query_row(
        "SELECT src.source_cursor, ss.stale, ss.counts_complete
         FROM session_snapshots ss
         JOIN sessions s ON s.id = ss.session_id
         JOIN session_sources src ON src.id = s.primary_source_id
         WHERE ss.provider_id = 'kiro' AND s.provider_session_id = ?1",
        [session_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    assert_ne!(after_messages.0, initial_fingerprint);
    assert_eq!(after_messages.1, 0);
    assert_eq!(after_messages.2, 0);
    drop(conn);

    let detail =
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))?;
    assert!(!detail.stale);
    let session_path = session_dir.join("session.json");
    fs::copy(
        kiro_audit_fixture_root().join("variants/session.updated.json"),
        &session_path,
    )?;
    let metadata_sync = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )?;
    assert_eq!(metadata_sync.projected_sessions, 1);
    assert_eq!(metadata_sync.unchanged_sessions, 1);

    let conn = local_store::open_database()?;
    let after_metadata: (String, String) = conn.query_row(
        "SELECT ss.title, src.source_cursor
         FROM session_snapshots ss
         JOIN sessions s ON s.id = ss.session_id
         JOIN session_sources src ON src.id = s.primary_source_id
         WHERE ss.provider_id = 'kiro' AND s.provider_session_id = ?1",
        [session_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(after_metadata.0, "Sanitized Kiro session (updated)");
    assert_ne!(after_metadata.1, after_messages.0);
    drop(conn);

    fs::OpenOptions::new()
        .append(true)
        .open(session_dir.join("sub-executions/subexec-1.jsonl"))?
        .write_all(b"\n")?;
    let sub_execution_sync = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )?;
    assert_eq!(sub_execution_sync.projected_sessions, 1);
    assert_eq!(sub_execution_sync.unchanged_sessions, 1);

    let conn = local_store::open_database()?;
    let after_sub_execution: String = conn.query_row(
        "SELECT src.source_cursor
         FROM session_sources src
         WHERE src.provider_id = 'kiro' AND src.provider_session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    assert_ne!(after_sub_execution, after_metadata.1);
    drop(conn);

    fs::remove_dir_all(&session_dir)?;
    let missing = crate::core::projection::refresh_projected_session_staleness(
        crate::storage::activity_store::ActivityActor::System,
    )?;
    assert_eq!(missing.checked_sources, 1);
    assert_eq!(missing.fresh_snapshots, 1);
    assert_eq!(missing.missing_sources, 1);
    assert_eq!(missing.stale_snapshots, 1);

    let missing_reprojection = crate::core::projection::reproject_stale_sessions(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )?;
    assert_eq!(missing_reprojection.candidate_snapshots, 1);
    assert_eq!(missing_reprojection.reprojected_snapshots, 0);
    assert_eq!(missing_reprojection.missing_sources, 1);

    let groups = crate::core::projection::list_sessions(&crate::core::SessionListParams {
        all: true,
        providers: vec![PROVIDER_ID.to_string()],
        cwd: None,
        include_message_counts: true,
        limit: None,
        offset: None,
        sort: crate::core::SessionListSort::Recent,
    })?;
    let session = groups
        .iter()
        .flat_map(|group| &group.sessions)
        .find(|session| session.session_id == session_id)
        .unwrap();
    assert!(session.stale);

    let error =
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(1))
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
fn current_format_native_management_is_backed_up_restorable_and_activity_complete() -> Result<()> {
    use crate::storage::activity_store::ActivityActor;
    use crate::storage::artifact_store::{
        ArtifactVerificationStatus, BackupQuery, BackupRestoreStatus,
    };

    let temp = copy_fixture_sessions()?;
    let home = tempfile::tempdir()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let _home_guard = TestConfigHomeGuard::new(home.path());
    let session_id = "sess_11111111-1111-4111-8111-111111111111";
    let session_dir = temp.path().join("8f3d1d8bb1bd8116").join(session_id);
    let original_metadata = fs::read(session_dir.join("session.json"))?;
    let original_messages = fs::read(session_dir.join("messages.jsonl"))?;
    let original_tool_output = fs::read(session_dir.join("tool-outputs/tool-1-a1b2c3d4.txt"))?;
    let original_snapshot = fs::read(session_dir.join("snapshots/snap0001/src/example.rs"))?;

    let bootstrap = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        ActivityActor::System,
    )?;
    assert_eq!(bootstrap.projected_sessions, 2);
    let timeline =
        crate::core::sessions::compute_session_activity_timeline(PROVIDER_ID, session_id)?;
    assert_eq!(timeline.provider_id, PROVIDER_ID);
    assert!(timeline.total_events > 0);
    assert!(timeline.total_messages > 0);
    assert!(crate::providers::hook_registry::find_provider_hook(PROVIDER_ID).is_some());
    assert!(crate::providers::hook_registry::find_hook_adapter(PROVIDER_ID).is_some());

    let renamed = crate::core::session_mutation::rename_session(
        PROVIDER_ID,
        session_id,
        "Current Kiro title",
        ActivityActor::Cli,
    )?;
    assert!(renamed.native_updated);
    assert_eq!(renamed.warning, None);
    assert_eq!(
        read_validated_session_metadata(&session_dir)?
            .title
            .as_deref(),
        Some("Current Kiro title")
    );
    assert_eq!(
        fs::read(session_dir.join("messages.jsonl"))?,
        original_messages
    );
    assert_eq!(
        fs::read(session_dir.join("tool-outputs/tool-1-a1b2c3d4.txt"))?,
        original_tool_output
    );
    assert_eq!(
        fs::read(session_dir.join("snapshots/snap0001/src/example.rs"))?,
        original_snapshot
    );

    let rename_backups = crate::core::session_management::list_registered_backups(BackupQuery {
        provider_id: Some(PROVIDER_ID.to_string()),
        provider_session_id: Some(session_id.to_string()),
        ..BackupQuery::default()
    })?;
    assert_eq!(rename_backups.len(), 1);
    let rename_backup = &rename_backups[0];
    assert_eq!(
        rename_backup.verification.status,
        ArtifactVerificationStatus::Verified
    );
    assert!(rename_backup
        .entry
        .backup
        .artifact
        .path
        .join("session/tool-outputs/tool-1-a1b2c3d4.txt")
        .is_file());
    assert!(rename_backup
        .entry
        .backup
        .artifact
        .path
        .join("session/snapshots/snap0001/src/example.rs")
        .is_file());
    let restore = crate::core::session_management::restore_registered_backup(
        &rename_backup.entry.backup.id,
        ActivityActor::Cli,
    )?;
    assert_eq!(restore.status, BackupRestoreStatus::Success);
    assert_eq!(
        fs::read(session_dir.join("session.json"))?,
        original_metadata
    );
    let repeat_restore = crate::core::session_management::restore_registered_backup(
        &rename_backup.entry.backup.id,
        ActivityActor::Cli,
    )?;
    assert_eq!(repeat_restore.status, BackupRestoreStatus::Success);

    set_test_kiro_mutation_failure(Some(ProviderSourceMutation::Rename));
    let rename_error = crate::core::session_mutation::rename_session(
        PROVIDER_ID,
        session_id,
        "Must roll back",
        ActivityActor::Cli,
    )
    .unwrap_err();
    assert!(format!("{rename_error:#}").contains("restored from registered backup"));
    assert_eq!(
        fs::read(session_dir.join("session.json"))?,
        original_metadata
    );

    set_test_kiro_mutation_failure(Some(ProviderSourceMutation::Delete));
    let delete_error =
        crate::core::session_mutation::delete_session(PROVIDER_ID, session_id, ActivityActor::Cli)
            .unwrap_err();
    assert!(format!("{delete_error:#}").contains("restored from registered backup"));
    assert!(session_dir.is_dir());
    assert_eq!(
        fs::read(session_dir.join("session.json"))?,
        original_metadata
    );
    assert_eq!(
        fs::read(session_dir.join("messages.jsonl"))?,
        original_messages
    );
    assert_eq!(
        fs::read(session_dir.join("tool-outputs/tool-1-a1b2c3d4.txt"))?,
        original_tool_output
    );
    assert_eq!(
        fs::read(session_dir.join("snapshots/snap0001/src/example.rs"))?,
        original_snapshot
    );

    let before_delete = crate::core::session_management::list_registered_backups(BackupQuery {
        provider_id: Some(PROVIDER_ID.to_string()),
        provider_session_id: Some(session_id.to_string()),
        ..BackupQuery::default()
    })?
    .into_iter()
    .map(|view| view.entry.backup.id)
    .collect::<BTreeSet<_>>();
    crate::core::session_mutation::delete_session(PROVIDER_ID, session_id, ActivityActor::Cli)?;
    assert!(!session_dir.exists());
    let after_delete = crate::core::session_management::list_registered_backups(BackupQuery {
        provider_id: Some(PROVIDER_ID.to_string()),
        provider_session_id: Some(session_id.to_string()),
        ..BackupQuery::default()
    })?;
    let delete_backup = after_delete
        .iter()
        .find(|view| !before_delete.contains(&view.entry.backup.id))
        .context("successful Kiro delete did not register a new backup")?;
    assert_eq!(
        delete_backup.entry.backup.artifact.metadata["mutation"],
        "delete"
    );
    let delete_restore = crate::core::session_management::restore_registered_backup(
        &delete_backup.entry.backup.id,
        ActivityActor::Cli,
    )?;
    assert_eq!(delete_restore.status, BackupRestoreStatus::Success);
    assert_eq!(
        fs::read(session_dir.join("session.json"))?,
        original_metadata
    );
    assert_eq!(
        fs::read(session_dir.join("messages.jsonl"))?,
        original_messages
    );
    assert_eq!(
        fs::read(session_dir.join("tool-outputs/tool-1-a1b2c3d4.txt"))?,
        original_tool_output
    );
    assert_eq!(
        fs::read(session_dir.join("snapshots/snap0001/src/example.rs"))?,
        original_snapshot
    );
    let repeat_delete_restore = crate::core::session_management::restore_registered_backup(
        &delete_backup.entry.backup.id,
        ActivityActor::Cli,
    )?;
    assert_eq!(repeat_delete_restore.status, BackupRestoreStatus::Success);

    let conn = local_store::open_database()?;
    for (operation, status) in [
        ("rename", "success"),
        ("rename", "failed"),
        ("delete", "success"),
        ("delete", "failed"),
    ] {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM session_activity
             WHERE provider_id = 'kiro' AND provider_session_id = ?1
               AND operation_kind = ?2 AND status = ?3 AND finished_at_ms IS NOT NULL",
            rusqlite::params![session_id, operation, status],
            |row| row.get(0),
        )?;
        assert!(count >= 1, "missing terminal {operation}/{status} activity");
    }
    let running: i64 = conn.query_row(
        "SELECT COUNT(*) FROM session_activity
         WHERE provider_id = 'kiro' AND provider_session_id = ?1 AND status = 'running'",
        [session_id],
        |row| row.get(0),
    )?;
    assert_eq!(running, 0);
    let successful_restores: i64 = conn.query_row(
        "SELECT COUNT(*) FROM backup_restores WHERE status = 'success'",
        [],
        |row| row.get(0),
    )?;
    assert!(successful_restores >= 4);
    let body_table_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
         WHERE type = 'table'
           AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(body_table_count, 0);
    assert!(after_delete.iter().all(|view| {
        view.entry
            .latest_restore
            .as_ref()
            .map(|restore| restore.status)
            != Some(BackupRestoreStatus::Failed)
    }));
    Ok(())
}

#[test]
fn current_format_import_maps_main_and_sub_execution_events_without_fake_artifacts() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_dir = temp
        .path()
        .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");

    let imported = KiroProvider.import_session(session_dir.to_str().unwrap())?;
    assert_eq!(
        imported.session.identity.canonical_id,
        "sess_11111111-1111-4111-8111-111111111111"
    );
    assert_eq!(
        imported.session.identity.source_title.as_deref(),
        Some("Sanitized Kiro session")
    );
    assert_eq!(
        imported.session.context.workspace_dir.as_deref(),
        Some("/workspace/sanitized-project")
    );
    assert_eq!(imported.session.events.len(), 12);
    assert!(imported.session.artifacts.is_empty());
    assert_eq!(imported.report.overall, Fidelity::Preserved);
    assert!(imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "sub_execution_parent_unresolved"));
    assert_eq!(
        imported.session.extensions["kiro_session_metadata"]["modelId"],
        "sanitized-model"
    );

    let event = |id: &str| {
        imported
            .session
            .events
            .iter()
            .find(|event| event.metadata.source.original_id.as_deref() == Some(id))
            .unwrap()
    };
    assert_eq!(
        event("msg-user-1").links.provider_turn_id.as_deref(),
        Some("exec-1")
    );
    assert!(matches!(
        event("msg-reasoning-1").blocks.as_slice(),
        [Block::Thinking { text, .. }] if text == "[sanitized reasoning]"
    ));
    assert!(matches!(
        event("msg-assistant-1").blocks.as_slice(),
        [Block::Text { text }] if text == "[sanitized assistant response]"
    ));
    assert!(matches!(
        event("msg-tool-call-1").blocks.as_slice(),
        [Block::ToolCall { tool_call_id, name, input: Some(input) }]
            if tool_call_id == "tool-1"
                && name == "read_file"
                && input["path"] == "src/example.rs"
    ));
    assert_eq!(
        event("msg-tool-result-1").links.parent_event_id.as_deref(),
        Some("msg-tool-call-1")
    );
    assert!(matches!(
        event("msg-tool-result-1").blocks.as_slice(),
        [Block::ToolResult { tool_call_id, content, is_error }]
            if tool_call_id == "tool-1"
                && content == "[sanitized tool output]"
                && !is_error
    ));
    assert_eq!(
        event("exec-1-turn-start").links.turn_boundary,
        Some(TurnBoundary::Started)
    );
    assert_eq!(
        event("exec-1-turn-end").links.turn_boundary,
        Some(TurnBoundary::Completed)
    );
    assert_eq!(
        event("sub-msg-user-1").links.provider_turn_id.as_deref(),
        Some("subexec-1")
    );
    assert_eq!(
        event("sub-msg-assistant-1").metadata.provider_ext["kiro_source"]["file"],
        "sub-executions/subexec-1.jsonl"
    );

    let ordered_ids = imported
        .session
        .events
        .iter()
        .map(|event| event.metadata.source.original_id.as_deref().unwrap())
        .collect::<Vec<_>>();
    let tool_call_index = ordered_ids
        .iter()
        .position(|id| *id == "msg-tool-call-1")
        .unwrap();
    let sub_user_index = ordered_ids
        .iter()
        .position(|id| *id == "sub-msg-user-1")
        .unwrap();
    let tool_result_index = ordered_ids
        .iter()
        .position(|id| *id == "msg-tool-result-1")
        .unwrap();
    assert!(tool_call_index < sub_user_index && sub_user_index < tool_result_index);
    assert!(!serde_json::to_string(&imported.session.events)?
        .contains("sanitized external tool output"));
    Ok(())
}

#[test]
fn current_format_import_keeps_exact_multi_turn_ids_and_explicit_sub_parent() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_dir = temp
        .path()
        .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
    let messages_path = session_dir.join("messages.jsonl");
    let mut messages =
        fs::read_to_string(kiro_audit_fixture_root().join("variants/messages.updated.jsonl"))?;
    messages.push_str(
        "{\"id\":\"sub-parent\",\"timestamp\":\"2026-07-16T00:00:04.050Z\",\"payload\":{\"type\":\"sub_agent_start\",\"executionId\":\"exec-1\",\"subExecutionId\":\"subexec-1\"}}\n",
    );
    fs::write(&messages_path, messages)?;

    let imported = KiroProvider.import_session(session_dir.to_str().unwrap())?;
    let event = |id: &str| {
        imported
            .session
            .events
            .iter()
            .find(|event| event.metadata.source.original_id.as_deref() == Some(id))
            .unwrap()
    };
    assert_eq!(
        event("msg-user-2").links.provider_turn_id.as_deref(),
        Some("exec-2")
    );
    assert_eq!(
        event("exec-2-turn-start").links.turn_boundary,
        Some(TurnBoundary::Started)
    );
    assert_eq!(
        event("exec-2-turn-end").links.turn_boundary,
        Some(TurnBoundary::Completed)
    );
    assert_eq!(
        event("sub-parent").links.provider_turn_id.as_deref(),
        Some("exec-1")
    );
    assert_eq!(
        event("sub-msg-user-1").links.parent_event_id.as_deref(),
        Some("sub-parent")
    );
    assert_eq!(
        event("sub-msg-assistant-1")
            .links
            .parent_event_id
            .as_deref(),
        Some("sub-parent")
    );
    assert!(!imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "sub_execution_parent_unresolved"));
    Ok(())
}

#[test]
fn current_format_import_reports_malformed_and_preserves_unknown_payloads() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_dir = temp
        .path()
        .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
    fs::remove_dir_all(session_dir.join("sub-executions"))?;
    let variants = kiro_audit_fixture_root().join("variants");
    let messages_path = session_dir.join("messages.jsonl");

    fs::copy(variants.join("messages.malformed.jsonl"), &messages_path)?;
    let malformed = KiroProvider.import_session(session_dir.to_str().unwrap())?;
    assert_eq!(malformed.session.events.len(), 2);
    assert_eq!(malformed.report.overall, Fidelity::Dropped);
    let issue = malformed
        .report
        .issues
        .iter()
        .find(|issue| issue.code == "invalid_jsonl_line")
        .unwrap();
    assert_eq!(issue.path.as_deref(), Some("messages:line:2"));
    assert!(matches!(issue.raw, Some(Value::String(_))));

    fs::copy(variants.join("messages.unknown.jsonl"), &messages_path)?;
    let unknown = KiroProvider.import_session(session_dir.to_str().unwrap())?;
    assert_eq!(unknown.session.events.len(), 1);
    assert_eq!(unknown.report.overall, Fidelity::Preserved);
    assert!(unknown
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "unknown_payload_preserved"));
    assert_eq!(unknown.session.events[0].kind, EventKind::Unknown);
    assert_eq!(unknown.session.events[0].role, Role::Unknown);
    assert!(matches!(
        unknown.session.events[0].blocks.as_slice(),
        [Block::ProviderPayload { kind, payload }]
            if kind == "future_kiro_payload"
                && payload["futureField"]["preserve"] == true
    ));
    Ok(())
}

#[test]
fn current_format_import_reports_missing_tool_identifiers() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_dir = temp
        .path()
        .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
    fs::remove_dir_all(session_dir.join("sub-executions"))?;
    fs::write(
        session_dir.join("messages.jsonl"),
        concat!(
            "{\"id\":\"tool-call-missing-id\",\"timestamp\":\"2026-07-16T00:00:01.000Z\",\"payload\":{\"type\":\"tool_call\",\"args\":{\"path\":\"src/example.rs\"}}}\n",
            "{\"id\":\"tool-result-missing-id\",\"timestamp\":\"2026-07-16T00:00:02.000Z\",\"payload\":{\"type\":\"tool_result\",\"content\":\"[sanitized result]\"}}\n",
        ),
    )?;

    let imported = KiroProvider.import_session(session_dir.to_str().unwrap())?;
    assert_eq!(imported.report.overall, Fidelity::Normalized);
    assert!(imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "missing_tool_call_id"));
    assert!(imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "missing_tool_name"));
    assert!(imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "missing_tool_result_call_id"));
    assert!(matches!(
        imported.session.events[0].blocks.as_slice(),
        [Block::ToolCall { tool_call_id, name, .. }]
            if tool_call_id == "tool-call-missing-id" && name == "unknown"
    ));
    assert!(matches!(
        imported.session.events[1].blocks.as_slice(),
        [Block::ToolResult { tool_call_id, .. }]
            if tool_call_id == "unknown"
    ));
    Ok(())
}

#[test]
fn current_format_import_classifies_known_payload_matrix() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_dir = temp
        .path()
        .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
    fs::remove_dir_all(session_dir.join("sub-executions"))?;
    fs::copy(
        kiro_audit_fixture_root().join("variants/messages.payload-matrix.jsonl"),
        session_dir.join("messages.jsonl"),
    )?;

    let imported = KiroProvider.import_session(session_dir.to_str().unwrap())?;
    assert_eq!(imported.session.events.len(), 11);
    assert_eq!(imported.report.overall, Fidelity::Preserved);
    assert_eq!(imported.session.events[0].kind, EventKind::Message);
    assert_eq!(imported.session.events[0].role, Role::System);
    assert_eq!(imported.session.events[1].kind, EventKind::Message);
    assert_eq!(imported.session.events[1].role, Role::Assistant);
    assert!(imported.session.events[2..]
        .iter()
        .all(|event| event.kind == EventKind::Lifecycle));
    assert!(imported.session.events[2..]
        .iter()
        .all(|event| { matches!(event.blocks.as_slice(), [Block::ProviderPayload { .. }]) }));
    Ok(())
}

#[test]
fn current_format_fingerprint_covers_metadata_messages_and_sub_executions() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_dir = temp
        .path()
        .join("8f3d1d8bb1bd8116/sess_11111111-1111-4111-8111-111111111111");
    let variants = kiro_audit_fixture_root().join("variants");
    let fingerprint = || {
        KiroProvider
            .session_source_fingerprint(session_dir.to_str().unwrap())
            .unwrap()
            .unwrap()
            .value
    };

    let baseline = fingerprint();
    assert!(baseline.starts_with("kiro-v2:"));
    assert!(baseline.contains(":sub-executions:1:"));

    let session_path = session_dir.join("session.json");
    let original_session = fs::read(&session_path)?;
    fs::copy(variants.join("session.updated.json"), &session_path)?;
    assert_ne!(fingerprint(), baseline);
    fs::write(&session_path, original_session)?;

    let messages_path = session_dir.join("messages.jsonl");
    let original_messages = fs::read(&messages_path)?;
    let restored_session_fingerprint = fingerprint();
    fs::copy(variants.join("messages.updated.jsonl"), &messages_path)?;
    assert_ne!(fingerprint(), restored_session_fingerprint);
    fs::write(&messages_path, original_messages)?;

    let sub_execution_path = session_dir.join("sub-executions/subexec-1.jsonl");
    let original_sub_execution = fs::read(&sub_execution_path)?;
    let restored_messages_fingerprint = fingerprint();
    fs::copy(
        variants.join("sub-execution.updated.jsonl"),
        &sub_execution_path,
    )?;
    assert_ne!(fingerprint(), restored_messages_fingerprint);
    fs::write(&sub_execution_path, original_sub_execution)?;

    let source_fingerprint = fingerprint();
    fs::write(
        session_dir.join("tool-outputs/tool-1-a1b2c3d4.txt"),
        "[changed artifact outside C2 canonical source scope]",
    )?;
    assert_eq!(fingerprint(), source_fingerprint);

    assert!(KiroProvider
        .session_source_fingerprint(session_path.to_str().unwrap())
        .unwrap_err()
        .to_string()
        .contains("outside the configured sessions root"));
    fs::remove_file(&messages_path)?;
    assert!(KiroProvider
        .session_source_fingerprint(session_dir.to_str().unwrap())?
        .is_none());
    assert!(KiroProvider
        .session_source_fingerprint(
            temp.path()
                .join("missing/session")
                .to_string_lossy()
                .as_ref()
        )?
        .is_none());
    Ok(())
}

#[test]
fn current_format_rejects_duplicate_ids_and_invalid_identity_buckets() -> Result<()> {
    let temp = copy_fixture_sessions()?;
    let _guard = use_test_kiro_sessions_dir(temp.path().to_path_buf());
    let session_id = "sess_11111111-1111-4111-8111-111111111111";
    let source_dir = temp.path().join("8f3d1d8bb1bd8116").join(session_id);
    let duplicate_workspace = "/workspace/duplicate".to_string();
    let duplicate_bucket = workspace_bucket(std::slice::from_ref(&duplicate_workspace))?;
    let duplicate_dir = temp.path().join(duplicate_bucket).join(session_id);
    copy_tree(&source_dir, &duplicate_dir)?;
    let metadata_path = duplicate_dir.join("session.json");
    let mut metadata: Value = serde_json::from_slice(&fs::read(&metadata_path)?)?;
    metadata["workspacePaths"] = json!([duplicate_workspace]);
    fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)?;

    assert!(KiroProvider
        .scan_sessions()
        .unwrap_err()
        .to_string()
        .contains("Ambiguous Kiro session id"));
    assert!(KiroProvider
        .get_session_meta(session_id)
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(KiroProvider
        .session_size(session_id)
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(KiroProvider
        .delete_session(session_id)
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(KiroProvider
        .rename_session(session_id, "not allowed")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(KiroProvider
        .get_session_meta("../outside")
        .unwrap_err()
        .to_string()
        .contains("Invalid Kiro session id"));

    fs::remove_dir_all(duplicate_dir)?;
    let original_metadata = fs::read(source_dir.join("session.json"))?;
    let mut invalid_id: Value = serde_json::from_slice(&original_metadata)?;
    invalid_id["id"] = Value::String("different-session-id".to_string());
    fs::write(
        source_dir.join("session.json"),
        serde_json::to_vec_pretty(&invalid_id)?,
    )?;
    assert!(KiroProvider
        .scan_sessions()
        .unwrap_err()
        .to_string()
        .contains("does not match session directory"));

    fs::write(source_dir.join("session.json"), &original_metadata)?;
    let mut invalid_bucket: Value = serde_json::from_slice(&original_metadata)?;
    invalid_bucket["workspacePaths"] = json!(["/workspace/different"]);
    fs::write(
        source_dir.join("session.json"),
        serde_json::to_vec_pretty(&invalid_bucket)?,
    )?;
    assert!(KiroProvider
        .scan_sessions()
        .unwrap_err()
        .to_string()
        .contains("does not match metadata workspacePaths"));
    Ok(())
}
