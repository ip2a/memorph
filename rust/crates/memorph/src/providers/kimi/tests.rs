use super::*;
use crate::{
    core::session_management,
    storage::{
        activity_store::{ActivityActor, ActivityOperationKind, ActivityQuery, ActivityStatus},
        artifact_store::{ArtifactVerificationStatus, BackupQuery, BackupRestoreStatus},
        local_store,
    },
};
use std::collections::{BTreeMap, BTreeSet};
use tempfile::tempdir;

static TEST_KIMI_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

struct TestKimiSessionsGuard {
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

impl Drop for TestKimiSessionsGuard {
    fn drop(&mut self) {
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        backup::set_test_backup_failure(false);
        set_test_kimi_mutation_failure(None);
        set_test_kimi_sessions_dir(None);
    }
}

fn use_test_kimi_sessions_dir(path: PathBuf) -> TestKimiSessionsGuard {
    let lock = TEST_KIMI_TEST_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    set_test_kimi_sessions_dir(Some(path));
    crate::cache::global_cache().invalidate(PROVIDER_ID);
    TestKimiSessionsGuard { _lock: lock }
}

fn write_native_kimi_fixture(root: &Path, project: &str, session_id: &str) -> PathBuf {
    let project_dir = format!("/workspace/{project}");
    let project_key = md5_hex(project_dir.as_bytes());
    let metadata_path = root.parent().unwrap().join("kimi.json");
    std::fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
    let mut metadata = if metadata_path.exists() {
        serde_json::from_slice::<Value>(&std::fs::read(&metadata_path).unwrap()).unwrap()
    } else {
        serde_json::json!({ "work_dirs": [] })
    };
    let work_dirs = metadata["work_dirs"].as_array_mut().unwrap();
    if !work_dirs
        .iter()
        .any(|work_dir| work_dir["path"] == project_dir)
    {
        work_dirs.push(serde_json::json!({
            "path": project_dir,
            "kaos": "local",
            "last_session_id": session_id
        }));
        std::fs::write(
            &metadata_path,
            serde_json::to_vec_pretty(&metadata).unwrap(),
        )
        .unwrap();
    }

    let session_dir = root.join(project_key).join(session_id);
    std::fs::create_dir_all(session_dir.join("nested")).unwrap();
    std::fs::write(
        session_dir.join("state.json"),
        b"{\n  \"version\": 1,\n  \"custom_title\": \"Before\",\n  \"archived\": false,\n  \"native\": {\"keep\": true}\n}\n",
    )
    .unwrap();
    std::fs::write(
        session_dir.join("wire.jsonl"),
        b"{\"timestamp\":1710000000.0,\"message\":{\"type\":\"metadata\"}}\n",
    )
    .unwrap();
    std::fs::write(
        session_dir.join("context.jsonl"),
        b"{\"role\":\"user\",\"content\":\"hello\"}\n",
    )
    .unwrap();
    std::fs::write(
        session_dir.join("nested").join("native.bin"),
        [0_u8, 1, 127, 128, 255],
    )
    .unwrap();
    session_dir
}

fn session_tree_bytes(session_dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    WalkDir::new(session_dir)
        .min_depth(1)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_type().is_file())
        .map(|entry| {
            (
                entry
                    .path()
                    .strip_prefix(session_dir)
                    .unwrap()
                    .to_path_buf(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

fn kimi_audit_fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/providers/kimi/fixtures/v1_37_0")
}

fn read_jsonl_values(path: &Path) -> Vec<Result<Value, serde_json::Error>> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

fn provider_payload_kind(event: &Event) -> Option<&str> {
    event.blocks.iter().find_map(|block| match block {
        Block::Other { raw } => raw
            .get("type")
            .and_then(Value::as_str)
            .or_else(|| raw.get("message").and_then(|m| m.get("type")).and_then(Value::as_str)),
        _ => None,
    })
}

fn copy_kimi_audit_fixture(target: &Path) {
    let source = kimi_audit_fixture_root();
    for entry in WalkDir::new(&source).into_iter().map(Result::unwrap) {
        let relative = entry.path().strip_prefix(&source).unwrap();
        let destination = target.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&destination).unwrap();
        } else {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn write_kimi_metadata(root: &Path, work_dirs: Value) {
    std::fs::write(
        root.join("kimi.json"),
        serde_json::to_vec_pretty(&serde_json::json!({ "work_dirs": work_dirs })).unwrap(),
    )
    .unwrap();
}

fn write_context_only_session(root: &Path, work_dir_key: &str, session_id: &str) -> PathBuf {
    let session_dir = root.join("sessions").join(work_dir_key).join(session_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(
        session_dir.join("context.jsonl"),
        b"{\"role\":\"user\",\"content\":\"sanitized\"}\n",
    )
    .unwrap();
    session_dir
}

#[test]
fn scans_known_kimi_work_dirs_from_context_and_uses_directory_locators() {
    let dir = tempdir().unwrap();
    copy_kimi_audit_fixture(dir.path());
    let sessions_root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());

    let sessions = KimiProvider.scan_sessions().unwrap();
    assert_eq!(sessions.len(), 2);

    let normal = sessions
        .iter()
        .find(|session| session.session_id == "11111111-1111-4111-8111-111111111111")
        .unwrap();
    let normal_dir = sessions_root
        .join("2030c6ce97e98c160351b18f097eb584")
        .join(&normal.session_id);
    assert_eq!(normal.title.as_deref(), Some("Sanitized session"));
    assert_eq!(
        normal.project_dir.as_deref(),
        Some("/workspace/sanitized-project")
    );
    assert_eq!(
        normal.source_path.as_deref(),
        Some(normal_dir.to_string_lossy().as_ref())
    );
    assert_eq!(
        normal.last_active_at,
        file_modified_ms(&normal_dir.join("context.jsonl")).unwrap()
    );
    assert!(normal.created_at.is_some());

    let context_only = sessions
        .iter()
        .find(|session| session.session_id == "22222222-2222-4222-8222-222222222222")
        .unwrap();
    assert_eq!(context_only.title, None);
    assert_eq!(
        context_only.source_path.as_deref(),
        Some(
            sessions_root
                .join("0017cc2b0eee031e9194d1384b4bcdd8")
                .join(&context_only.session_id)
                .to_string_lossy()
                .as_ref()
        )
    );

    std::fs::remove_file(normal_dir.join("state.json")).unwrap();
    let fallback = KimiProvider
        .get_session_meta(&normal.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(fallback.title.as_deref(), Some("[sanitized user request]"));
}

#[test]
fn scan_supports_local_remote_and_unmapped_work_dirs() {
    let dir = tempdir().unwrap();
    let sessions_root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let local_path = "/workspace/local";
    let remote_path = "/workspace/remote";
    let local_key = md5_hex(local_path.as_bytes());
    let remote_key = format!("ssh_{}", md5_hex(remote_path.as_bytes()));
    write_kimi_metadata(
        dir.path(),
        serde_json::json!([
            { "path": local_path, "kaos": "local", "last_session_id": "local-session" },
            { "path": remote_path, "kaos": "ssh", "last_session_id": "remote-session" }
        ]),
    );
    write_context_only_session(dir.path(), &local_key, "local-session");
    write_context_only_session(dir.path(), &remote_key, "remote-session");
    write_context_only_session(dir.path(), "orphan-key", "orphan-session");

    let sessions = KimiProvider.scan_sessions().unwrap();
    assert_eq!(sessions.len(), 3);
    assert_eq!(
        sessions
            .iter()
            .filter_map(|session| session
                .project_dir
                .as_deref()
                .map(|project_dir| (session.session_id.as_str(), project_dir)))
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            ("local-session", local_path),
            ("remote-session", remote_path),
        ])
    );
    assert_eq!(
        sessions
            .iter()
            .find(|session| session.session_id == "orphan-session")
            .unwrap()
            .project_dir,
        None
    );
}

#[test]
fn duplicate_kimi_session_ids_are_rejected_by_scan_and_identity_reads() {
    let dir = tempdir().unwrap();
    let sessions_root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root);
    let first_path = "/workspace/first";
    let second_path = "/workspace/second";
    let first_key = md5_hex(first_path.as_bytes());
    let second_key = md5_hex(second_path.as_bytes());
    write_kimi_metadata(
        dir.path(),
        serde_json::json!([
            { "path": first_path, "kaos": "local" },
            { "path": second_path, "kaos": "local" }
        ]),
    );
    write_context_only_session(dir.path(), &first_key, "duplicate-session");
    write_context_only_session(dir.path(), &second_key, "duplicate-session");

    assert!(KimiProvider
        .scan_sessions()
        .unwrap_err()
        .to_string()
        .contains("Ambiguous Kimi session id"));
    assert!(KimiProvider
        .get_session_meta("duplicate-session")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
    assert!(KimiProvider
        .session_size("duplicate-session")
        .unwrap_err()
        .to_string()
        .contains("ambiguous"));
}

#[test]
fn kimi_fingerprint_covers_context_wire_state_and_relevant_mapping() {
    let dir = tempdir().unwrap();
    copy_kimi_audit_fixture(dir.path());
    let sessions_root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let session_dir = sessions_root
        .join("2030c6ce97e98c160351b18f097eb584")
        .join("11111111-1111-4111-8111-111111111111");
    let variants = dir.path().join("variants");
    let fingerprint = || {
        KimiProvider
            .session_source_fingerprint(session_dir.to_str().unwrap())
            .unwrap()
            .unwrap()
            .value
    };

    let before_state = fingerprint();
    std::fs::copy(
        variants.join("state.updated.json"),
        session_dir.join("state.json"),
    )
    .unwrap();
    assert_ne!(fingerprint(), before_state);

    let before_context = fingerprint();
    std::fs::copy(
        variants.join("context.updated.jsonl"),
        session_dir.join("context.jsonl"),
    )
    .unwrap();
    assert_ne!(fingerprint(), before_context);

    let before_wire = fingerprint();
    std::fs::copy(
        variants.join("wire.updated.jsonl"),
        session_dir.join("wire.jsonl"),
    )
    .unwrap();
    assert_ne!(fingerprint(), before_wire);

    let before_mapping = fingerprint();
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("kimi.json")).unwrap()).unwrap();
    metadata["work_dirs"][0]["last_session_id"] = Value::String("changed-session".to_string());
    std::fs::write(
        dir.path().join("kimi.json"),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    assert_ne!(fingerprint(), before_mapping);

    std::fs::copy(
        variants.join("wire.malformed.jsonl"),
        session_dir.join("wire.jsonl"),
    )
    .unwrap();
    assert!(fingerprint().starts_with("kimi-v1:"));
    std::fs::remove_file(session_dir.join("wire.jsonl")).unwrap();
    assert!(fingerprint().contains("wire:absent"));

    assert!(KimiProvider
        .session_source_fingerprint(session_dir.join("state.json").to_str().unwrap())
        .unwrap_err()
        .to_string()
        .contains("must be a directory"));
    std::fs::remove_file(session_dir.join("context.jsonl")).unwrap();
    assert!(KimiProvider
        .session_source_fingerprint(session_dir.to_str().unwrap())
        .unwrap()
        .is_none());
}

#[test]
fn kimi_import_accepts_only_directory_locators_and_context_only_sessions() {
    let dir = tempdir().unwrap();
    copy_kimi_audit_fixture(dir.path());
    let sessions_root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let session_dir = sessions_root
        .join("2030c6ce97e98c160351b18f097eb584")
        .join("11111111-1111-4111-8111-111111111111");

    let imported = KimiProvider
        .import_session(session_dir.to_str().unwrap())
        .unwrap();
    assert_eq!(
        imported.provenance.primary_source.source_path.as_deref(),
        Some(session_dir.to_string_lossy().as_ref())
    );
    assert!(KimiProvider
        .import_session(session_dir.join("wire.jsonl").to_str().unwrap())
        .unwrap_err()
        .to_string()
        .contains("must be a directory"));

    let context_only_dir = sessions_root
        .join("0017cc2b0eee031e9194d1384b4bcdd8")
        .join("22222222-2222-4222-8222-222222222222");
    let context_only = KimiProvider
        .import_session(context_only_dir.to_str().unwrap())
        .unwrap();
    assert_eq!(
        context_only
            .session
            .events
            .iter()
            .filter_map(event_visible_message_text)
            .collect::<Vec<_>>(),
        vec!["[sanitized context-only request]".to_string()]
    );
    assert!(context_only.session.events.iter().any(|event| {
        event.role == Role::System
            && matches!(
                event.blocks.first(),
                Some(Block::Text { text })
                    if text == "[sanitized context-only system prompt]"
            )
    }));
    assert!(!context_only.session.extensions.contains_key("kimi_state"));
    assert!(!context_only
        .session
        .extensions
        .contains_key("kimi_wire_metadata"));
    assert_eq!(context_only.report.overall, Fidelity::Preserved);
    assert!(!context_only_dir.join("wire.jsonl").exists());
}

#[test]
fn sanitized_kimi_fixture_records_real_source_plane() {
    let root = kimi_audit_fixture_root();
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(root.join("fixture.json")).unwrap()).unwrap();
    assert_eq!(manifest["provider"], "kimi");
    assert_eq!(manifest["observed_cli_version"], "1.37.0");
    assert_eq!(manifest["provenance"], "sanitized-local-source");
    assert_eq!(manifest["raw_user_content_committed"], false);

    let metadata: Value =
        serde_json::from_slice(&std::fs::read(root.join("kimi.json")).unwrap()).unwrap();
    let work_dirs = metadata["work_dirs"].as_array().unwrap();
    assert_eq!(work_dirs.len(), 2);
    for work_dir in work_dirs {
        assert_eq!(work_dir["kaos"], "local");
        let path = work_dir["path"].as_str().unwrap();
        let session_id = work_dir["last_session_id"].as_str().unwrap();
        assert!(Uuid::parse_str(session_id).is_ok());
        let session_dir = root
            .join("sessions")
            .join(md5_hex(path.as_bytes()))
            .join(session_id);
        assert!(session_dir.join("context.jsonl").is_file());
    }

    let normal = root
        .join("sessions/2030c6ce97e98c160351b18f097eb584")
        .join("11111111-1111-4111-8111-111111111111");
    assert!(normal.join("wire.jsonl").is_file());
    assert!(normal.join("state.json").is_file());

    let context_only = root
        .join("sessions/0017cc2b0eee031e9194d1384b4bcdd8")
        .join("22222222-2222-4222-8222-222222222222");
    assert!(context_only.join("context.jsonl").is_file());
    assert!(!context_only.join("wire.jsonl").exists());
    assert!(!context_only.join("state.json").exists());
}

#[test]
fn sanitized_kimi_wire_fixture_preserves_observed_v1_37_schema() {
    let path = kimi_audit_fixture_root().join(
        "sessions/2030c6ce97e98c160351b18f097eb584/11111111-1111-4111-8111-111111111111/wire.jsonl",
    );
    let values = read_jsonl_values(&path)
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();

    assert_eq!(values[0]["type"], "metadata");
    assert_eq!(values[0]["protocol_version"], "1.3");
    assert!(values[0].get("timestamp").is_none());
    assert!(values[0].get("message").is_none());

    let message_types: Vec<_> = values[1..]
        .iter()
        .map(|value| value["message"]["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        message_types,
        [
            "TurnBegin",
            "StepBegin",
            "ContentPart",
            "ContentPart",
            "StatusUpdate",
            "TurnEnd",
            "TurnBegin",
            "StepBegin",
            "ContentPart",
            "TurnEnd",
        ]
    );

    let content_part_types: Vec<_> = values[1..]
        .iter()
        .filter(|value| value["message"]["type"] == "ContentPart")
        .map(|value| value["message"]["payload"]["type"].as_str().unwrap())
        .collect();
    assert_eq!(content_part_types, ["think", "text", "text"]);

    let status = values[1..]
        .iter()
        .find(|value| value["message"]["type"] == "StatusUpdate")
        .unwrap();
    let status_keys: BTreeSet<_> = status["message"]["payload"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        status_keys,
        BTreeSet::from([
            "context_tokens",
            "context_usage",
            "max_context_tokens",
            "mcp_status",
            "message_id",
            "plan_mode",
            "token_usage",
        ])
    );

    let timestamps: Vec<_> = values[1..]
        .iter()
        .map(|value| value["timestamp"].as_f64().unwrap())
        .collect();
    assert!(timestamps.windows(2).all(|pair| pair[0] < pair[1]));
}

#[test]
fn sanitized_kimi_fixture_covers_context_state_updates_and_damage() {
    let root = kimi_audit_fixture_root();
    let session =
        root.join("sessions/2030c6ce97e98c160351b18f097eb584/11111111-1111-4111-8111-111111111111");
    let context = read_jsonl_values(&session.join("context.jsonl"))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let roles: Vec<_> = context
        .iter()
        .map(|value| value["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        [
            "_system_prompt",
            "_checkpoint",
            "user",
            "_usage",
            "assistant",
            "_checkpoint",
            "user",
            "_usage",
            "assistant",
        ]
    );
    assert!(context
        .iter()
        .filter(|value| value["role"] == "user")
        .all(|value| value["content"].is_string()));
    assert!(context
        .iter()
        .filter(|value| value["role"] == "assistant")
        .all(|value| value["content"].is_array()));

    let state: Value =
        serde_json::from_slice(&std::fs::read(session.join("state.json")).unwrap()).unwrap();
    let updated_state: Value =
        serde_json::from_slice(&std::fs::read(root.join("variants/state.updated.json")).unwrap())
            .unwrap();
    assert_eq!(
        state.as_object().unwrap().keys().collect::<BTreeSet<_>>(),
        updated_state
            .as_object()
            .unwrap()
            .keys()
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(state["archived"], false);
    assert_eq!(updated_state["archived"], true);
    assert_ne!(state["custom_title"], updated_state["custom_title"]);

    let updated_context = read_jsonl_values(&root.join("variants/context.updated.jsonl"))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(&updated_context[..context.len()], context.as_slice());
    assert_eq!(updated_context.len(), context.len() + 2);

    let wire = read_jsonl_values(&session.join("wire.jsonl"))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    let updated_wire = read_jsonl_values(&root.join("variants/wire.updated.jsonl"))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(&updated_wire[..wire.len()], wire.as_slice());
    assert_eq!(updated_wire.len(), wire.len() + 4);

    let malformed = read_jsonl_values(&root.join("variants/wire.malformed.jsonl"));
    assert_eq!(malformed.iter().filter(|line| line.is_ok()).count(), 3);
    assert_eq!(malformed.iter().filter(|line| line.is_err()).count(), 1);
}

#[test]
fn delete_backup_restores_exact_kimi_directory_and_preserves_unrelated_sessions() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let session_id = "kimi-delete";
    let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
    let unrelated_dir = write_native_kimi_fixture(&root, "project-b", "kimi-other");
    let original = session_tree_bytes(&session_dir);
    let backup = KimiProvider
        .create_session_backup(
            ProviderSourceMutation::Delete,
            "operation-kimi-delete",
            session_id,
            &dir.path().join("backups"),
        )
        .unwrap();

    KimiProvider.delete_session(session_id).unwrap();
    assert!(!session_dir.exists());
    std::fs::write(unrelated_dir.join("wire.jsonl"), b"changed concurrently\n").unwrap();

    KimiProvider.restore_session_backup(&backup).unwrap();
    KimiProvider.restore_session_backup(&backup).unwrap();

    assert_eq!(session_tree_bytes(&session_dir), original);
    assert_eq!(
        std::fs::read(unrelated_dir.join("wire.jsonl")).unwrap(),
        b"changed concurrently\n"
    );
}

#[test]
fn rename_backup_restores_exact_state_only_and_preserves_other_changes() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let session_id = "kimi-rename";
    let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
    let original_state = std::fs::read(session_dir.join("state.json")).unwrap();
    let backup = KimiProvider
        .create_session_backup(
            ProviderSourceMutation::Rename,
            "operation-kimi-rename",
            session_id,
            &dir.path().join("backups"),
        )
        .unwrap();

    KimiProvider.rename_session(session_id, "After").unwrap();
    std::fs::write(
        session_dir.join("wire.jsonl"),
        b"wire changed concurrently\n",
    )
    .unwrap();
    std::fs::write(session_dir.join("concurrent.txt"), b"keep me").unwrap();

    KimiProvider.restore_session_backup(&backup).unwrap();
    KimiProvider.restore_session_backup(&backup).unwrap();

    assert_eq!(
        std::fs::read(session_dir.join("state.json")).unwrap(),
        original_state
    );
    assert_eq!(
        std::fs::read(session_dir.join("wire.jsonl")).unwrap(),
        b"wire changed concurrently\n"
    );
    assert_eq!(
        std::fs::read(session_dir.join("concurrent.txt")).unwrap(),
        b"keep me"
    );
}

#[test]
fn rename_restore_does_not_recreate_concurrently_deleted_state() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let session_id = "kimi-concurrent-delete";
    let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
    let backup = KimiProvider
        .create_session_backup(
            ProviderSourceMutation::Rename,
            "operation-kimi-concurrent-delete",
            session_id,
            &dir.path().join("backups"),
        )
        .unwrap();
    KimiProvider.rename_session(session_id, "After").unwrap();
    std::fs::remove_file(session_dir.join("state.json")).unwrap();

    KimiProvider.restore_session_backup(&backup).unwrap();

    assert!(!session_dir.join("state.json").exists());
}

#[test]
fn kimi_backup_contract_and_capabilities_are_truthful() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let session_id = "kimi-contract";
    let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
    let backup = KimiProvider
        .create_session_backup(
            ProviderSourceMutation::Delete,
            "operation-kimi-contract",
            session_id,
            &dir.path().join("backups"),
        )
        .unwrap();

    let capabilities = KimiProvider.capabilities();
    assert_eq!(capabilities.scan_strategy, ScanStrategy::Hybrid);
    assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);
    assert_eq!(capabilities.storage_shape, StorageShape::Directory);
    assert_eq!(capabilities.turn_quality, TurnQuality::Inferred);
    assert_eq!(
        capabilities.import_fidelity,
        ProviderContentFidelity {
            text: Some(Fidelity::Preserved),
            thinking: Some(Fidelity::Preserved),
            tool_call: Some(Fidelity::Downgraded),
            tool_result: Some(Fidelity::Downgraded),
            patch: Some(Fidelity::Unsupported),
            image: Some(Fidelity::Normalized),
            file: Some(Fidelity::Downgraded),
            compressed: Some(Fidelity::Unsupported),
            provider_payload: Some(Fidelity::Preserved),
        }
    );
    assert_eq!(
        capabilities.export_fidelity,
        ProviderContentFidelity {
            text: Some(Fidelity::Preserved),
            thinking: Some(Fidelity::Preserved),
            tool_call: Some(Fidelity::Downgraded),
            tool_result: Some(Fidelity::Downgraded),
            patch: Some(Fidelity::Downgraded),
            image: Some(Fidelity::Downgraded),
            file: Some(Fidelity::Downgraded),
            compressed: Some(Fidelity::Downgraded),
            provider_payload: Some(Fidelity::Dropped),
        }
    );
    assert_eq!(capabilities.resume_quality, ResumeQuality::Native);
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
    assert!(capabilities.backup_support.before_write);
    assert!(capabilities.backup_support.restore);
    assert!(!capabilities.backup_support.sync_only);
    assert_eq!(
        capabilities.activity_support,
        ProviderActivitySupport {
            hook_events: true,
            runtime_endpoint: true,
            session_activity: true,
        }
    );
    assert_eq!(backup.source_path, session_dir.canonicalize().unwrap());
    assert_eq!(backup.format, "kimi-session-backup-v1");
    assert_eq!(
        backup.mime_type,
        "application/vnd.memorph.kimi-session-backup"
    );
    assert!(backup.backup_path.join("metadata.json").is_file());
    assert!(backup.backup_path.join("session/state.json").is_file());
    assert!(backup
        .backup_path
        .join("session/nested/native.bin")
        .is_file());
}

#[test]
fn native_export_is_discoverable_resumable_and_round_trips_declared_fidelity() {
    let dir = tempdir().unwrap();
    let sessions_root = dir.path().join("provider-home/sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let source_dir = dir.path().join("source/source-session");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(
        source_dir.join("context.jsonl"),
        concat!(
            "{\"role\":\"user\",\"content\":\"hello\"}\n",
            "{\"role\":\"assistant\",\"content\":[{\"type\":\"think\",\"think\":\"reasoning\",\"encrypted\":null},{\"type\":\"text\",\"text\":\"answer\"}]}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        source_dir.join("state.json"),
        br#"{"version":1,"custom_title":"Exported Kimi"}"#,
    )
    .unwrap();
    let canonical = KimiProvider
        .import_session(source_dir.to_str().unwrap())
        .unwrap()
        .session;
    let target_dir = dir.path().join("workspace");
    std::fs::create_dir_all(&target_dir).unwrap();
    let metadata_path = sessions_root.parent().unwrap().join("kimi.json");
    std::fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
    std::fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "future_top_level": {"keep": true},
            "work_dirs": [{
                "path": "/sanitized/existing",
                "kaos": "local",
                "last_session_id": "existing-session",
                "future_entry": 7
            }]
        }))
        .unwrap(),
    )
    .unwrap();

    let exported = KimiProvider
        .export_session(&canonical, &target_dir)
        .unwrap();
    assert_eq!(
        exported.resume_command.as_deref(),
        Some(format!("kimi --resume {}", exported.session_id).as_str())
    );
    let project_hash = md5_hex(target_dir.to_string_lossy().as_bytes());
    let session_dir = sessions_root.join(project_hash).join(&exported.session_id);
    assert!(session_dir.join("wire.jsonl").is_file());
    assert!(session_dir.join("context.jsonl").is_file());
    assert!(session_dir.join("state.json").is_file());

    let metadata: Value = serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    assert_eq!(metadata["future_top_level"]["keep"], true);
    assert_eq!(metadata["work_dirs"][0]["future_entry"], 7);
    let target_entries = metadata["work_dirs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["path"] == target_dir.to_string_lossy().as_ref())
        .collect::<Vec<_>>();
    assert_eq!(target_entries.len(), 1);
    assert_eq!(target_entries[0]["kaos"], "local");
    assert_eq!(target_entries[0]["last_session_id"], exported.session_id);

    let scanned = KimiProvider.scan_sessions().unwrap();
    let summary = scanned
        .iter()
        .find(|summary| summary.session_id == exported.session_id)
        .expect("exported Kimi session should be discoverable");
    assert_eq!(
        summary.source_path.as_deref(),
        Some(session_dir.to_string_lossy().as_ref())
    );
    let imported = KimiProvider
        .import_session(summary.source_path.as_deref().unwrap())
        .unwrap();
    let assistant = imported
        .session
        .events
        .iter()
        .find(|event| event.role == Role::Assistant)
        .unwrap();
    assert!(assistant
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Thinking { text, .. } if text == "reasoning")));
    assert!(assistant
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Text { text } if text == "answer")));

    let second = KimiProvider
        .export_session(&canonical, &target_dir)
        .unwrap();
    let metadata: Value = serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    let target_entries = metadata["work_dirs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["path"] == target_dir.to_string_lossy().as_ref())
        .collect::<Vec<_>>();
    assert_eq!(target_entries.len(), 1);
    assert_eq!(target_entries[0]["last_session_id"], second.session_id);
    assert_eq!(target_entries[0]["kaos"], "local");
}

#[test]
fn failed_metadata_registration_removes_new_kimi_session_directory() {
    let dir = tempdir().unwrap();
    let sessions_root = dir.path().join("provider-home/sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let source_dir = dir.path().join("source/source-session");
    std::fs::create_dir_all(&source_dir).unwrap();
    std::fs::write(
        source_dir.join("context.jsonl"),
        b"{\"role\":\"user\",\"content\":\"hello\"}\n",
    )
    .unwrap();
    let canonical = KimiProvider
        .import_session(source_dir.to_str().unwrap())
        .unwrap()
        .session;
    let target_dir = dir.path().join("workspace");
    std::fs::create_dir_all(&target_dir).unwrap();
    let metadata_path = sessions_root.parent().unwrap().join("kimi.json");
    std::fs::create_dir_all(metadata_path.parent().unwrap()).unwrap();
    std::fs::write(&metadata_path, br#"{"work_dirs":{}}"#).unwrap();

    let error = KimiProvider
        .export_session(&canonical, &target_dir)
        .unwrap_err();
    assert!(error.to_string().contains("work_dirs must be an array"));
    let project_dir = sessions_root.join(md5_hex(target_dir.to_string_lossy().as_bytes()));
    assert!(
        !project_dir.exists() || std::fs::read_dir(project_dir).unwrap().next().is_none(),
        "failed export must not leave an orphan Kimi session directory"
    );
}

#[test]
fn backup_registration_failure_prevents_kimi_provider_write() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let session_id = "kimi-registration-failure";
    let session_dir = write_native_kimi_fixture(&root, "project-a", session_id);
    let mut artifact_conn = rusqlite::Connection::open_in_memory().unwrap();

    let results = session_management::delete_sessions(
        PROVIDER_ID,
        &[session_id],
        &["operation-kimi-registration".to_string()],
        &dir.path().join("backups"),
        &mut artifact_conn,
    );

    assert!(results[0]
        .as_ref()
        .unwrap_err()
        .to_string()
        .contains("Delete cancelled before provider write"));
    assert!(session_dir.exists());
    assert!(dir
        .path()
        .join("backups/kimi/operation-kimi-registration")
        .exists());
}

#[test]
fn partial_kimi_delete_and_rename_failures_restore_registered_backups() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let delete_id = "kimi-partial-delete";
    let rename_id = "kimi-partial-rename";
    let delete_dir = write_native_kimi_fixture(&root, "project-a", delete_id);
    let rename_dir = write_native_kimi_fixture(&root, "project-a", rename_id);
    let delete_original = session_tree_bytes(&delete_dir);
    let rename_original = std::fs::read(rename_dir.join("state.json")).unwrap();
    let mut artifact_conn = rusqlite::Connection::open_in_memory().unwrap();
    local_store::configure_connection(&artifact_conn).unwrap();
    local_store::apply_schema(&mut artifact_conn).unwrap();

    set_test_kimi_mutation_failure(Some(ProviderSourceMutation::Delete));
    let delete_results = session_management::delete_sessions(
        PROVIDER_ID,
        &[delete_id],
        &["operation-kimi-partial-delete".to_string()],
        &dir.path().join("backups"),
        &mut artifact_conn,
    );
    assert!(delete_results[0]
        .as_ref()
        .unwrap_err()
        .to_string()
        .contains("Provider source was restored from registered backup"));
    assert_eq!(session_tree_bytes(&delete_dir), delete_original);

    set_test_kimi_mutation_failure(Some(ProviderSourceMutation::Rename));
    let rename_error = session_management::rename_session(
        PROVIDER_ID,
        rename_id,
        "After",
        "operation-kimi-partial-rename",
        &dir.path().join("backups"),
        &mut artifact_conn,
    )
    .unwrap_err();
    assert!(rename_error
        .to_string()
        .contains("Provider source was restored from registered backup"));
    assert_eq!(
        std::fs::read(rename_dir.join("state.json")).unwrap(),
        rename_original
    );
}

#[test]
fn kimi_backup_rejects_ambiguous_and_unsafe_sources_before_mutation() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let session_id = "kimi-ambiguous";
    let first = write_native_kimi_fixture(&root, "project-a", session_id);
    let second = write_native_kimi_fixture(&root, "project-b", session_id);

    let error = KimiProvider
        .create_session_backup(
            ProviderSourceMutation::Delete,
            "operation-kimi-ambiguous",
            session_id,
            &dir.path().join("backups"),
        )
        .unwrap_err();
    assert!(error.to_string().contains("not found or ambiguous"));
    assert!(first.exists());

    std::fs::remove_dir_all(second).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(first.join("wire.jsonl"), first.join("unsafe-wire-link"))
            .unwrap();
        let error = KimiProvider
            .create_session_backup(
                ProviderSourceMutation::Delete,
                "operation-kimi-unsafe",
                session_id,
                &dir.path().join("backups"),
            )
            .unwrap_err();
        assert!(error.to_string().contains("unsupported filesystem entry"));
    }
}

#[test]
fn kimi_restore_rejects_metadata_and_content_tampering() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let session_id = "kimi-tamper";
    write_native_kimi_fixture(&root, "project-a", session_id);
    let backup_root = dir.path().join("backups");

    let content_backup = KimiProvider
        .create_session_backup(
            ProviderSourceMutation::Delete,
            "operation-kimi-content-tamper",
            session_id,
            &backup_root,
        )
        .unwrap();
    std::fs::write(
        content_backup.backup_path.join("session/state.json"),
        b"tampered",
    )
    .unwrap();
    assert!(KimiProvider
        .restore_session_backup(&content_backup)
        .unwrap_err()
        .to_string()
        .contains("does not match its manifest"));

    let metadata_backup = KimiProvider
        .create_session_backup(
            ProviderSourceMutation::Rename,
            "operation-kimi-metadata-tamper",
            session_id,
            &backup_root,
        )
        .unwrap();
    let metadata_path = metadata_backup.backup_path.join("metadata.json");
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    metadata["provider_session_id"] = Value::String("other-session".to_string());
    std::fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    assert!(KimiProvider
        .restore_session_backup(&metadata_backup)
        .unwrap_err()
        .to_string()
        .contains("does not match the registered restore context"));
}

#[test]
fn failed_kimi_backup_creation_removes_operation_directory() {
    let dir = tempdir().unwrap();
    let root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(root.clone());
    let session_id = "kimi-backup-failure";
    write_native_kimi_fixture(&root, "project-a", session_id);
    backup::set_test_backup_failure(true);

    let error = KimiProvider
        .create_session_backup(
            ProviderSourceMutation::Delete,
            "operation-kimi-backup-failure",
            session_id,
            &dir.path().join("backups"),
        )
        .unwrap_err();

    assert!(error.to_string().contains("injected Kimi backup failure"));
    assert!(!dir
        .path()
        .join("backups/kimi/operation-kimi-backup-failure")
        .exists());
}

#[test]
fn kimi_full_import_pages_keep_total_counts_and_project_only_page_turns() {
    let dir = tempdir().unwrap();
    copy_kimi_audit_fixture(dir.path());
    let sessions_root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let session_dir = sessions_root
        .join("2030c6ce97e98c160351b18f097eb584")
        .join("11111111-1111-4111-8111-111111111111");
    let source_path = session_dir.to_str().unwrap();

    let capabilities = KimiProvider.capabilities();
    assert_eq!(capabilities.page_strategy, PageStrategy::FullImport);
    assert_eq!(capabilities.storage_shape, StorageShape::Directory);
    assert_eq!(capabilities.turn_quality, TurnQuality::Inferred);

    let full = KimiProvider
        .import_session_page(source_path, 0, None)
        .unwrap();
    assert_eq!(full.imported.session.events.len(), full.event_count);
    assert_eq!(
        full.message_count,
        full.imported
            .session
            .events
            .iter()
            .filter(|event| event_is_visible_message(event))
            .count()
    );
    assert_eq!(full.turns.len(), 2);
    assert_eq!(full.turn_count, Some(full.turns.len()));

    let assistant_index = full
        .imported
        .session
        .events
        .iter()
        .position(|event| event.role == Role::Assistant && event.links.turn_id.is_some())
        .unwrap();
    let expected_turn_id = full.imported.session.events[assistant_index]
        .links
        .turn_id
        .clone()
        .unwrap();
    let page = KimiProvider
        .import_session_page(source_path, assistant_index, Some(1))
        .unwrap();

    assert_eq!(page.imported.session.events.len(), 1);
    assert_eq!(page.event_count, full.event_count);
    assert_eq!(page.message_count, full.message_count);
    assert_eq!(page.turn_count, full.turn_count);
    assert_eq!(page.turns.len(), 1);
    assert_eq!(
        page.turns[0].provider_turn_id.as_deref(),
        Some(expected_turn_id.as_str())
    );
    assert_eq!(
        page.turns[0].confidence,
        crate::session_projection::TurnConfidence::Exact
    );

    let empty = KimiProvider
        .import_session_page(source_path, full.event_count, Some(0))
        .unwrap();
    assert!(empty.imported.session.events.is_empty());
    assert!(empty.turns.is_empty());
    assert_eq!(empty.event_count, full.event_count);
    assert_eq!(empty.message_count, full.message_count);
    assert_eq!(empty.turn_count, full.turn_count);

    let context_only_dir = sessions_root
        .join("0017cc2b0eee031e9194d1384b4bcdd8")
        .join("22222222-2222-4222-8222-222222222222");
    let context_only = KimiProvider
        .import_session_page(context_only_dir.to_str().unwrap(), 0, None)
        .unwrap();
    assert_eq!(context_only.turns.len(), 1);
    assert_eq!(context_only.turns[0].provider_turn_id, None);
    assert_eq!(
        context_only.turns[0].confidence,
        crate::session_projection::TurnConfidence::Inferred
    );
}

#[test]
fn session_index_and_detail_dispatch_are_idempotent_source_backed_and_bodyless() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let _home_guard = TestConfigHomeGuard::new(&home);
    let sessions_root = dir.path().join("sessions");
    let _kimi_guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let session_id = "33333333-3333-4333-8333-333333333333";
    let session_dir = write_native_kimi_fixture(&sessions_root, "project-index", session_id);
    let summary = KimiProvider
        .scan_sessions()
        .unwrap()
        .into_iter()
        .find(|summary| summary.session_id == session_id)
        .unwrap();
    assert_eq!(
        summary.source_path.as_deref(),
        Some(session_dir.to_string_lossy().as_ref())
    );
    let fingerprint = KimiProvider
        .session_source_fingerprint(summary.source_path.as_deref().unwrap())
        .unwrap()
        .unwrap();
    let full = KimiProvider
        .import_session_page(summary.source_path.as_deref().unwrap(), 0, None)
        .unwrap();
    let expected_turn_count = full.turn_count.unwrap();

    let mut conn = local_store::open_database().unwrap();
    let first = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
        .write_session_summary(
            PROVIDER_ID,
            &summary,
            KimiProvider.capabilities(),
            &fingerprint,
        )
        .unwrap();
    let counts_after_first: (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'kimi'),
                (SELECT COUNT(*) FROM sessions WHERE provider_id = 'kimi'),
                (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'kimi'),
                (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'kimi')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    let second = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
        .write_session_summary(
            PROVIDER_ID,
            &summary,
            KimiProvider.capabilities(),
            &fingerprint,
        )
        .unwrap();
    let counts_after_second: (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT
                (SELECT COUNT(*) FROM session_sources WHERE provider_id = 'kimi'),
                (SELECT COUNT(*) FROM sessions WHERE provider_id = 'kimi'),
                (SELECT COUNT(*) FROM session_snapshots WHERE provider_id = 'kimi'),
                (SELECT COUNT(*) FROM session_aliases WHERE provider_id = 'kimi')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(counts_after_first, counts_after_second);
    assert_eq!(counts_after_second.0, 1);
    assert_eq!(counts_after_second.1, 1);
    assert_eq!(counts_after_second.2, 1);

    let (source_path, storage_shape, source_cursor): (String, String, String) = conn
        .query_row(
            "SELECT source_path, storage_shape, source_cursor
             FROM session_sources WHERE id = ?1",
            [&first.source_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(source_path, session_dir.to_string_lossy());
    assert_eq!(storage_shape, "directory");
    assert_eq!(source_cursor, fingerprint.value);
    let snapshot_json: String = conn
        .query_row(
            "SELECT snapshot_json FROM session_snapshots WHERE session_id = ?1",
            [&first.canonical_session_id],
            |row| row.get(0),
        )
        .unwrap();
    let snapshot_json: Value = serde_json::from_str(&snapshot_json).unwrap();
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
    let body_table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(body_table_count, 0);
    drop(conn);

    let detail =
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))
            .unwrap();
    assert!(detail.events.is_empty());
    assert!(detail.turns.is_empty());
    assert_eq!(detail.event_count, full.event_count);
    assert_eq!(detail.message_count, full.message_count);
    assert_eq!(
        detail.source_path.as_deref(),
        Some(session_dir.to_string_lossy().as_ref())
    );
    assert_eq!(
        detail.projection_report.as_ref().unwrap().id,
        format!("source-read:{PROVIDER_ID}:{session_id}")
    );

    let conn = local_store::open_database().unwrap();
    let cached_counts: (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT event_count, message_count, turn_count, counts_complete
             FROM session_snapshots WHERE session_id = ?1",
            [&first.canonical_session_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(cached_counts.0, full.event_count as i64);
    assert_eq!(cached_counts.1, full.message_count as i64);
    assert_eq!(cached_counts.2, expected_turn_count as i64);
    assert_eq!(cached_counts.3, 1);
    drop(conn);

    std::fs::remove_dir_all(&session_dir).unwrap();
    let groups = crate::core::projection::list_sessions(&crate::core::SessionListParams {
        all: true,
        providers: vec![PROVIDER_ID.to_string()],
        cwd: None,
        fields: crate::core::SessionListFields::WithStats,
        limit: None,
        offset: None,
        sort: crate::core::SessionListSort::Recent,
    })
    .unwrap();
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
}

#[test]
fn bootstrap_stale_and_system_sync_are_incremental_source_backed_and_bodyless() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let _home_guard = TestConfigHomeGuard::new(&home);
    let sessions_root = dir.path().join("sessions");
    let _kimi_guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let session_id = "44444444-4444-4444-8444-444444444444";
    let project = "project-bootstrap";
    let project_dir = format!("/workspace/{project}");
    let session_dir = write_native_kimi_fixture(&sessions_root, project, session_id);

    let first = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::Cli,
    )
    .unwrap();
    assert_eq!(first.scanned_providers, 1);
    assert_eq!(first.discovered_sessions, 1);
    assert_eq!(first.projected_sessions, 1);
    assert_eq!(first.unchanged_sessions, 0);
    assert!(first.failures.is_empty());

    let detail =
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))
            .unwrap();
    assert!(detail.events.is_empty());
    assert!(detail.turns.is_empty());

    let conn = local_store::open_database().unwrap();
    let initial: (String, String, i64, i64, i64, i64) = conn
        .query_row(
            "SELECT ss.title, ss.workspace_dir, ss.counts_complete, ss.stale,
                    src.scan_generation,
                    (SELECT COUNT(*) FROM sqlite_master
                     WHERE type = 'table'
                       AND name IN ('session_turns', 'session_events', 'session_event_blocks'))
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kimi'",
            [],
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
    let initial_fingerprint: String = conn
        .query_row(
            "SELECT source_cursor FROM session_sources WHERE provider_id = 'kimi'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(initial.0, "Before");
    assert_eq!(initial.1, project_dir);
    assert_eq!(initial.2, 1);
    assert_eq!(initial.3, 0);
    assert_eq!(initial.4, 1);
    assert_eq!(initial.5, 0);
    drop(conn);

    let unchanged = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )
    .unwrap();
    assert_eq!(unchanged.scanned_providers, 1);
    assert_eq!(unchanged.discovered_sessions, 1);
    assert_eq!(unchanged.projected_sessions, 0);
    assert_eq!(unchanged.unchanged_sessions, 1);
    assert!(unchanged.failures.is_empty());

    let conn = local_store::open_database().unwrap();
    let unchanged_state: (i64, i64) = conn
        .query_row(
            "SELECT src.scan_generation, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kimi'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(unchanged_state, (1, 1));
    drop(conn);

    let mut context = std::fs::OpenOptions::new()
        .append(true)
        .open(session_dir.join("context.jsonl"))
        .unwrap();
    writeln!(
        context,
        "{}",
        serde_json::json!({"role": "assistant", "content": "background refresh"})
    )
    .unwrap();
    drop(context);

    let stale = crate::core::projection::refresh_projected_session_staleness(
        crate::storage::activity_store::ActivityActor::System,
    )
    .unwrap();
    assert_eq!(stale.checked_sources, 1);
    assert_eq!(stale.fresh_snapshots, 0);
    assert_eq!(stale.stale_snapshots, 1);
    assert_eq!(stale.missing_sources, 0);

    let conn = local_store::open_database().unwrap();
    let stale_flag: i64 = conn
        .query_row(
            "SELECT stale FROM session_snapshots WHERE provider_id = 'kimi'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stale_flag, 1);
    drop(conn);

    let reprojected = crate::core::projection::reproject_stale_sessions(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )
    .unwrap();
    assert_eq!(reprojected.candidate_snapshots, 1);
    assert_eq!(reprojected.reprojected_snapshots, 1);
    assert_eq!(reprojected.missing_sources, 0);
    assert!(reprojected.failures.is_empty());

    let conn = local_store::open_database().unwrap();
    let after_context: (String, i64, i64) = conn
        .query_row(
            "SELECT src.source_cursor, ss.stale, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kimi'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_ne!(after_context.0, initial_fingerprint);
    assert_eq!(after_context.1, 0);
    assert_eq!(after_context.2, 0);
    drop(conn);

    crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(0))
        .unwrap();
    let conn = local_store::open_database().unwrap();
    let counts_complete: i64 = conn
        .query_row(
            "SELECT counts_complete FROM session_snapshots WHERE provider_id = 'kimi'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(counts_complete, 1);
    drop(conn);

    std::fs::write(
        session_dir.join("state.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "version": 1,
            "custom_title": "After background refresh",
            "archived": false,
            "native": {"keep": true}
        }))
        .unwrap(),
    )
    .unwrap();
    let state_sync = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )
    .unwrap();
    assert_eq!(state_sync.projected_sessions, 1);
    assert_eq!(state_sync.unchanged_sessions, 0);

    let conn = local_store::open_database().unwrap();
    let after_state: (String, String, i64) = conn
        .query_row(
            "SELECT ss.title, src.source_cursor, ss.counts_complete
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kimi'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(after_state.0, "After background refresh");
    assert_ne!(after_state.1, after_context.0);
    assert_eq!(after_state.2, 0);
    drop(conn);

    let mut wire = std::fs::OpenOptions::new()
        .append(true)
        .open(session_dir.join("wire.jsonl"))
        .unwrap();
    writeln!(
        wire,
        "{}",
        serde_json::json!({
            "timestamp": 1710000001.0,
            "message": {"type": "StatusUpdate", "payload": {"status": "synced"}}
        })
    )
    .unwrap();
    drop(wire);
    let wire_sync = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )
    .unwrap();
    assert_eq!(wire_sync.projected_sessions, 1);

    let conn = local_store::open_database().unwrap();
    let after_wire: String = conn
        .query_row(
            "SELECT source_cursor FROM session_sources WHERE provider_id = 'kimi'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_ne!(after_wire, after_state.1);
    drop(conn);

    let metadata_path = sessions_root.parent().unwrap().join("kimi.json");
    let mut metadata: Value =
        serde_json::from_slice(&std::fs::read(&metadata_path).unwrap()).unwrap();
    metadata["work_dirs"][0]["sync_marker"] = Value::String("changed".to_string());
    std::fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
    let mapping_sync = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )
    .unwrap();
    assert_eq!(mapping_sync.projected_sessions, 1);

    let conn = local_store::open_database().unwrap();
    let after_mapping: (String, String) = conn
        .query_row(
            "SELECT src.source_cursor, ss.workspace_dir
             FROM session_snapshots ss
             JOIN sessions s ON s.id = ss.session_id
             JOIN session_sources src ON src.id = s.primary_source_id
             WHERE ss.provider_id = 'kimi'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_ne!(after_mapping.0, after_wire);
    assert_eq!(after_mapping.1, project_dir);
    drop(conn);

    std::fs::remove_dir_all(&session_dir).unwrap();
    let missing = crate::core::projection::refresh_projected_session_staleness(
        crate::storage::activity_store::ActivityActor::System,
    )
    .unwrap();
    assert_eq!(missing.checked_sources, 0);
    assert_eq!(missing.missing_sources, 1);
    assert_eq!(missing.stale_snapshots, 1);

    let missing_reprojection = crate::core::projection::reproject_stale_sessions(
        Some(PROVIDER_ID),
        crate::storage::activity_store::ActivityActor::System,
    )
    .unwrap();
    assert_eq!(missing_reprojection.candidate_snapshots, 1);
    assert_eq!(missing_reprojection.reprojected_snapshots, 0);
    assert_eq!(missing_reprojection.missing_sources, 1);

    let groups = crate::core::projection::list_sessions(&crate::core::SessionListParams {
        all: true,
        providers: vec![PROVIDER_ID.to_string()],
        cwd: None,
        fields: crate::core::SessionListFields::WithStats,
        limit: None,
        offset: None,
        sort: crate::core::SessionListSort::Recent,
    })
    .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].sessions.len(), 1);
    assert_eq!(groups[0].sessions[0].session_id, session_id);
    assert!(groups[0].sessions[0].stale);

    let error =
        crate::core::sessions::get_session_detail_view_page(PROVIDER_ID, session_id, 0, Some(1))
            .unwrap_err();
    assert!(format!("{error:#}").contains("Session source is missing"));

    let conn = local_store::open_database().unwrap();
    let system_scan_activities: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_activity
             WHERE actor = 'system' AND operation_kind = 'scan' AND status != 'running'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(system_scan_activities >= 8);
    let body_table_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table'
               AND name IN ('session_turns', 'session_events', 'session_event_blocks')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(body_table_count, 0);
}

#[test]
fn import_canonical_session_reconciles_context_with_wire_lifecycle() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let session_dir = temp.path().join("project-hash").join("kimi-session-1");
    std::fs::create_dir_all(&session_dir)?;
    let wire_path = session_dir.join("wire.jsonl");
    let context_path = session_dir.join("context.jsonl");
    let state_path = session_dir.join("state.json");
    std::fs::write(
        &context_path,
        concat!(
            "{\"role\":\"_system_prompt\",\"content\":\"system\"}\n",
            "{\"role\":\"_checkpoint\",\"id\":0}\n",
            "{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hello\"},{\"type\":\"image_url\",\"image_url\":{\"url\":\"data:image/png;base64,abc\"}}]}\n",
            "{\"role\":\"_usage\",\"token_count\":42}\n",
            "{\"role\":\"assistant\",\"content\":[{\"type\":\"think\",\"think\":\"reasoning\",\"encrypted\":null},{\"type\":\"text\",\"text\":\"answer\"},{\"type\":\"custom\",\"payload\":{\"kept\":true}}]}\n"
        ),
    )?;
    std::fs::write(
        &state_path,
        serde_json::to_string_pretty(&serde_json::json!({
            "custom_title": "Kimi Title",
            "archived": false,
            "todos": [{"content": "keep raw state"}]
        }))?,
    )?;
    let mut wire_file = File::create(&wire_path)?;
    for value in [
        serde_json::json!({"type": "metadata", "protocol_version": "1.9"}),
        serde_json::json!({
            "timestamp": 1710000001.0,
            "message": {
                "type": "TurnBegin",
                "payload": {"user_input": [
                    {"type": "text", "text": "hello"},
                    {"type": "image_url", "image_url": {"url": "data:image/png;base64,abc"}}
                ]}
            }
        }),
        serde_json::json!({
            "timestamp": 1710000002.0,
            "message": {"type": "StepBegin", "payload": {"n": 1}}
        }),
        serde_json::json!({
            "timestamp": 1710000003.0,
            "message": {"type": "ContentPart", "payload": {"type": "think", "think": "reasoning"}}
        }),
        serde_json::json!({
            "timestamp": 1710000004.0,
            "message": {"type": "ContentPart", "payload": {"type": "text", "text": "answer"}}
        }),
        serde_json::json!({
            "timestamp": 1710000004.5,
            "message": {"type": "ContentPart", "payload": {"type": "custom", "payload": {"kept": true}}}
        }),
        serde_json::json!({
            "timestamp": 1710000005.0,
            "message": {"type": "StatusUpdate", "payload": {"context_tokens": 42}}
        }),
        serde_json::json!({
            "timestamp": 1710000006.0,
            "message": {"type": "TurnEnd", "payload": {}}
        }),
        serde_json::json!({
            "timestamp": 1710000007.0,
            "message": {"type": "FutureRecord", "payload": {"kept": true}}
        }),
    ] {
        writeln!(wire_file, "{value}")?;
    }

    let imported = import_canonical_session_from_dir(&session_dir)?;

    assert_eq!(imported.session.identity.id, "kimi-session-1");
    assert_eq!(
        imported.session.identity.title.as_deref(),
        Some("Kimi Title")
    );
    assert!(imported.session.extensions.contains_key("kimi_state"));
    assert_eq!(
        imported.session.extensions["kimi_wire_metadata"][0]["protocol_version"],
        "1.9"
    );
    assert!(!imported.session.events.iter().any(|event| {
        event
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Other { raw } if raw["type"] == "metadata"))
    }));

    let visible = imported
        .session
        .events
        .iter()
        .filter_map(|event| {
            event_visible_message_text(event).map(|text| (event.role, text))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec![
            (
                Role::User,
                "hello\n[Image: image/png]\ndata:image/png;base64,abc".to_string()
            ),
            (Role::Assistant, "reasoning\nanswer".to_string()),
        ]
    );
    let user = imported
        .session
        .events
        .iter()
        .find(|event| event.role == Role::User)
        .unwrap();
    assert!(user.blocks.iter().any(
        |block| matches!(block, Block::Image { data: Some(data), .. } if data == "data:image/png;base64,abc")
    ));
    let assistant = imported
        .session
        .events
        .iter()
        .find(|event| event.role == Role::Assistant && event.kind == EventKind::Message)
        .unwrap();
    assert!(assistant
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Thinking { text, .. } if text == "reasoning")));
    assert!(assistant
        .blocks
        .iter()
        .any(|block| matches!(block, Block::Other { raw } if raw["type"] == "custom")));

    let turn_begin = imported
        .session
        .events
        .iter()
        .find(|event| provider_payload_kind(event) == Some("TurnBegin"))
        .unwrap();
    let turn_end = imported
        .session
        .events
        .iter()
        .find(|event| provider_payload_kind(event) == Some("TurnEnd"))
        .unwrap();
    assert_eq!(turn_begin.links.turn_outcome, None);
    assert_eq!(turn_end.links.turn_outcome, Some(TurnOutcome::Completed));
    assert_eq!(turn_begin.links.turn_id, turn_end.links.turn_id);
    assert_eq!(user.links.turn_id, turn_begin.links.turn_id);
    assert_eq!(assistant.links.turn_id, turn_begin.links.turn_id);
    for kind in ["StepBegin", "StatusUpdate", "custom", "FutureRecord"] {
        assert!(imported
            .session
            .events
            .iter()
            .any(|event| provider_payload_kind(event) == Some(kind)));
    }
    assert_eq!(imported.report.overall, Fidelity::Preserved);
    assert!(imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "provider_block_preserved"));
    assert!(imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "provider_wire_message_preserved"));
    Ok(())
}

#[test]
fn sanitized_kimi_fixture_imports_context_authoritatively_with_native_turns() {
    let dir = tempdir().unwrap();
    copy_kimi_audit_fixture(dir.path());
    let sessions_root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let session_dir = sessions_root
        .join("2030c6ce97e98c160351b18f097eb584")
        .join("11111111-1111-4111-8111-111111111111");

    let imported = KimiProvider
        .import_session(session_dir.to_str().unwrap())
        .unwrap();
    let visible = imported
        .session
        .events
        .iter()
        .filter_map(|event| {
            event_visible_message_text(event).map(|text| (event.role, text))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        visible,
        vec![
            (Role::User, "[sanitized user request]".to_string()),
            (
                Role::Assistant,
                "[sanitized reasoning]\n[sanitized assistant response]".to_string()
            ),
            (Role::User, "[sanitized follow-up]".to_string()),
            (
                Role::Assistant,
                "[sanitized follow-up response]".to_string()
            ),
        ]
    );
    assert_eq!(
        imported.session.extensions["kimi_wire_metadata"][0]["protocol_version"],
        "1.3"
    );
    assert!(imported
        .session
        .events
        .iter()
        .any(|event| provider_payload_kind(event) == Some("StepBegin")));
    assert!(imported
        .session
        .events
        .iter()
        .any(|event| provider_payload_kind(event) == Some("StatusUpdate")));
    let turn_ids = imported
        .session
        .events
        .iter()
        .filter(|event| matches!(event.role, Role::User | Role::Assistant))
        .filter_map(|event| event.links.turn_id.as_deref())
        .collect::<Vec<_>>();
    assert_eq!(turn_ids.len(), 4);
    assert_eq!(turn_ids[0], turn_ids[1]);
    assert_eq!(turn_ids[2], turn_ids[3]);
    assert_ne!(turn_ids[0], turn_ids[2]);
    assert_eq!(imported.report.overall, Fidelity::Preserved);
    assert!(imported.report.issues.is_empty());
}

#[test]
fn malformed_wire_line_is_reported_without_losing_context_messages() {
    let dir = tempdir().unwrap();
    copy_kimi_audit_fixture(dir.path());
    let sessions_root = dir.path().join("sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let session_dir = sessions_root
        .join("2030c6ce97e98c160351b18f097eb584")
        .join("11111111-1111-4111-8111-111111111111");
    std::fs::copy(
        dir.path().join("variants/wire.malformed.jsonl"),
        session_dir.join("wire.jsonl"),
    )
    .unwrap();

    let imported = KimiProvider
        .import_session(session_dir.to_str().unwrap())
        .unwrap();
    assert_eq!(
        imported
            .session
            .events
            .iter()
            .filter_map(event_visible_message_text)
            .collect::<Vec<_>>(),
        vec![
            "[sanitized user request]".to_string(),
            "[sanitized reasoning]\n[sanitized assistant response]".to_string(),
            "[sanitized follow-up]".to_string(),
            "[sanitized follow-up response]".to_string(),
        ]
    );
    assert!(imported
        .report
        .issues
        .iter()
        .any(|issue| issue.code == "invalid_wire_jsonl_line"
            && issue.disposition == Fidelity::Dropped));
    assert!(imported
        .session
        .events
        .iter()
        .any(|event| provider_payload_kind(event) == Some("TurnBegin")));
    assert!(imported
        .session
        .events
        .iter()
        .any(|event| provider_payload_kind(event) == Some("TurnEnd")));
    assert_eq!(imported.report.overall, Fidelity::Dropped);
}

#[test]
fn core_kimi_mutations_register_backups_restore_failures_and_finish_activity() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    let sessions_root = dir.path().join("provider-home/sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let _home_guard = TestConfigHomeGuard::new(&home);
    let session_id = "kimi-core-management";
    let session_dir = write_native_kimi_fixture(&sessions_root, "core-management", session_id);

    let renamed = crate::core::session_mutation::rename_session(
        PROVIDER_ID,
        session_id,
        "Renamed through core",
        ActivityActor::Cli,
    )
    .unwrap();
    assert!(renamed.native_updated);
    let state: Value =
        serde_json::from_slice(&std::fs::read(session_dir.join("state.json")).unwrap()).unwrap();
    assert_eq!(state["custom_title"], "Renamed through core");

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

    let before_failed_delete = session_tree_bytes(&session_dir);
    set_test_kimi_mutation_failure(Some(ProviderSourceMutation::Delete));
    let error =
        crate::core::session_mutation::delete_session(PROVIDER_ID, session_id, ActivityActor::Cli)
            .unwrap_err();
    assert!(format!("{error:#}").contains("Provider source was restored from registered backup"));
    assert_eq!(session_tree_bytes(&session_dir), before_failed_delete);

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
    assert_eq!(session_tree_bytes(&session_dir), before_failed_delete);

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

#[test]
fn kimi_session_activity_is_computed_from_the_live_source() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    let sessions_root = dir.path().join("provider-home/sessions");
    let _guard = use_test_kimi_sessions_dir(sessions_root.clone());
    let _home_guard = TestConfigHomeGuard::new(&home);
    let session_id = "kimi-source-activity";
    let session_dir = write_native_kimi_fixture(&sessions_root, "source-activity", session_id);

    let bootstrap = crate::core::projection::bootstrap_session_projections(
        Some(PROVIDER_ID),
        ActivityActor::System,
    )
    .unwrap();
    assert_eq!(bootstrap.projected_sessions, 1);
    let timeline =
        crate::core::sessions::compute_session_activity_timeline(PROVIDER_ID, session_id).unwrap();
    assert_eq!(timeline.provider_id, PROVIDER_ID);
    assert_eq!(timeline.session_id, session_id);
    assert!(timeline.total_events > 0);
    assert!(timeline.total_messages > 0);
    assert_eq!(
        timeline.total_events,
        timeline
            .buckets
            .iter()
            .map(|bucket| bucket.event_count)
            .sum::<usize>()
    );

    std::fs::remove_dir_all(&session_dir).unwrap();
    let error = crate::core::sessions::compute_session_activity_timeline(PROVIDER_ID, session_id)
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(
        message.contains("Session source is missing")
            || message.contains("Failed to read Kimi session directory"),
        "unexpected missing-source error: {message}"
    );
}

#[test]
fn compressed_segment_exports_as_portable_kimi_text_part() {
    let block = Block::Compressed {
        raw: serde_json::json!({
            "format": "memorph.compressed.v1",
            "source_provider_id": "opencode",
            "summary": "compressed summary",
            "source_event_ids": ["old-event-1", "old-event-2", "old-event-3"],
            "source_event_count": 3,
            "archive_ref": "memorph-archive://s1/archive.json.gz",
        }),
    };

    let part = block_to_kimi_content_part(&block).expect("kimi text part");
    let text = part
        .get("text")
        .and_then(Value::as_str)
        .expect("portable compressed text");

    assert_eq!(part.get("type").and_then(Value::as_str), Some("text"));
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
fn provider_payload_block_is_skipped_in_kimi_text_part_export() {
    let block = Block::Other {
        raw: serde_json::json!({"type": "custom", "kept": true}),
    };

    assert!(block_to_kimi_content_part(&block).is_none());
}
