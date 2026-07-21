
use super::*;
use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, MappingDisposition, ProviderSessionRef, SessionContext, SessionEvent,
    SessionEventKind, SessionIdentity, SessionProvenance,
};
use crate::hooks::model::{RuntimeSession, RuntimeSessionId, RuntimeSessionStatus};
use crate::hooks::protocol::{HookIngestRequest, HookRuntimeEndpoint};
use crate::storage::session_state::ResolvedLocalSessionState;
use axum::{
    body::{to_bytes, Body},
    http::Request,
};
use chrono::Utc;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use tempfile::Builder;
use tower::util::ServiceExt;

static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct ConfigTestHome {
    _guard: std::sync::MutexGuard<'static, ()>,
}

impl ConfigTestHome {
    fn new(path: &Path) -> Self {
        let guard = test_guard();
        crate::config::set_test_home_dir(path.to_path_buf());
        Self { _guard: guard }
    }
}

impl Drop for ConfigTestHome {
    fn drop(&mut self) {
        crate::config::reset_test_home_dir();
    }
}

async fn read_json(app: Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
    (status, value)
}

#[test]
fn directory_listing_returns_only_sorted_directories() {
    let root = Builder::new()
        .prefix("memorph-directory-listing")
        .tempdir()
        .unwrap();
    std::fs::create_dir(root.path().join("zeta")).unwrap();
    std::fs::create_dir(root.path().join("Alpha")).unwrap();
    std::fs::write(root.path().join("session.json"), "{}").unwrap();

    let listing = directory_listing(root.path().to_str()).unwrap();
    let names = listing
        .directories
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["Alpha", "zeta"]);
    assert_eq!(
        listing.path,
        root.path().canonicalize().unwrap().to_string_lossy()
    );
    assert!(listing
        .directories
        .iter()
        .all(|entry| entry.path.starts_with(&listing.path)));
}

#[test]
fn directory_listing_rejects_relative_paths() {
    let error = directory_listing(Some("relative/path")).unwrap_err();
    assert!(error.to_string().contains("must be absolute"));
}

fn runtime_session_for_payload(runtime_id: &str) -> RuntimeSession {
    RuntimeSession {
        runtime_id: RuntimeSessionId::new(runtime_id),
        provider: "claude".to_string(),
        provider_session_id: Some("session-1".to_string()),
        run_id: None,
        cwd: None,
        pid: None,
        parent_pid: None,
        pid_start_time: None,
        tty: None,
        terminal_vars: BTreeMap::new(),
        process_ancestry: Vec::new(),
        correlation: None,
        model: None,
        session_title: None,
        transcript_path: None,
        workspace_roots: Vec::new(),
        last_user_prompt: None,
        last_assistant_message: None,
        last_tool_result: None,
        last_error: None,
        stop_reason: None,
        compact_count: 0,
        tool_call_count: 0,
        failed_tool_count: 0,
        permission_request_count: 0,
        question_count: 0,
        status: RuntimeSessionStatus::Running,
        current_tool: None,
        pending_permission: None,
        pending_question: None,
        recent_activity: Vec::new(),
        subagents: BTreeMap::new(),
        last_event_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn sync_holding(id: &str, provider: &str, session_id: &str) -> session_sync::Holding {
    session_sync::Holding {
        id: id.to_string(),
        provider: provider.to_string(),
        session_id: session_id.to_string(),
        target_dir: None,
        created_at: 1,
        last_active_at: None,
        last_sync_at: None,
        last_sync_from: None,
        last_error: None,
    }
}

#[test]
fn sync_safety_blocks_active_target_runtime() {
    let group = session_sync::SyncGroup {
        id: "group-1".to_string(),
        title: "group".to_string(),
        source_provider: Some("codex".to_string()),
        created_at: 1,
        updated_at: 1,
        holdings: vec![
            sync_holding("source", "codex", "source-session"),
            sync_holding("target", "claude", "session-1"),
        ],
    };
    let snapshot = vec![runtime_session_for_payload("runtime-1")];

    let blocked = blocked_sync_targets_from_snapshot(&group, "source", &snapshot);

    assert_eq!(blocked.len(), 1);
    assert!(blocked[0].contains("claude:session-1"));
}

#[test]
fn sync_safety_allows_active_source_runtime() {
    let group = session_sync::SyncGroup {
        id: "group-1".to_string(),
        title: "group".to_string(),
        source_provider: Some("claude".to_string()),
        created_at: 1,
        updated_at: 1,
        holdings: vec![
            sync_holding("source", "claude", "session-1"),
            sync_holding("target", "codex", "target-session"),
        ],
    };
    let snapshot = vec![runtime_session_for_payload("runtime-1")];

    let blocked = blocked_sync_targets_from_snapshot(&group, "source", &snapshot);

    assert!(blocked.is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn session_refresh_stale_route_returns_scan_report() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/sessions/refresh-stale")
        .body(Body::empty())
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["checked_sources"], 0);
    assert_eq!(value["data"]["fresh_snapshots"], 0);
    assert_eq!(value["data"]["stale_snapshots"], 0);
    assert_eq!(value["data"]["missing_sources"], 0);
    assert_eq!(value["data"]["unknown_sources"], 0);
}

#[tokio::test(flavor = "current_thread")]
async fn session_bootstrap_route_returns_empty_provider_report() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/sessions/bootstrap")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"provider":"claude"}"#))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["scanned_providers"], 1);
    assert_eq!(value["data"]["failed_providers"], 0);
    assert_eq!(value["data"]["discovered_sessions"], 0);
    assert_eq!(value["data"]["projected_sessions"], 0);
    assert_eq!(value["data"]["unchanged_sessions"], 0);
    assert_eq!(value["data"]["failures"].as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn session_reproject_stale_route_returns_report() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/sessions/reproject-stale")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"provider":"claude"}"#))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["candidate_snapshots"], 0);
    assert_eq!(value["data"]["reprojected_snapshots"], 0);
    assert_eq!(value["data"]["missing_sources"], 0);
    assert_eq!(value["data"]["unsupported_providers"], 0);
    assert_eq!(value["data"]["failed_snapshots"], 0);
    assert_eq!(value["data"]["failures"].as_array().unwrap().len(), 0);
}

#[tokio::test(flavor = "current_thread")]
async fn management_activity_route_filters_activity_records() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let conn = crate::storage::local_store::open_database().unwrap();
    let store = crate::storage::activity_store::ActivityStore::new(&conn);
    let activity_id = store
        .start(crate::storage::activity_store::NewActivity {
            provider_id: Some("claude".to_string()),
            provider_session_id: Some("native-activity".to_string()),
            workspace_dir: Some("/tmp/project".to_string()),
            operation_kind: ActivityOperationKind::Export,
            actor: ActivityActor::Cli,
            summary: "Exporting session".to_string(),
            details: serde_json::json!({"format": "json"}),
        })
        .unwrap();
    store
        .finish(
            &activity_id,
            crate::storage::activity_store::ActivityCompletion::success(
                "Exported session",
                serde_json::json!({"files": ["/tmp/session.json"]}),
            ),
        )
        .unwrap();
    drop(conn);

    let request = Request::builder()
        .uri(
            "/api/v1/management/activity?session_id=native-activity&provider=claude\
                 &operation=export&status=success&actor=cli&limit=10",
        )
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    let activities = value["data"].as_array().unwrap();
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0]["id"], activity_id);
    assert_eq!(activities[0]["provider_id"], "claude");
    assert_eq!(activities[0]["provider_session_id"], "native-activity");
    assert_eq!(activities[0]["operation_kind"], "export");
    assert_eq!(activities[0]["status"], "success");
    assert_eq!(activities[0]["actor"], "cli");
}

#[tokio::test(flavor = "current_thread")]
async fn backup_routes_query_empty_store_and_reject_unknown_restore() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());

    let request = Request::builder()
        .uri("/api/v1/backups?provider=claude&restore_status=success&limit=10")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"].as_array().unwrap().len(), 0);

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/backups/missing/restore")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Unknown backup: missing"));
}

#[tokio::test(flavor = "current_thread")]
async fn backup_query_rejects_unknown_restore_status() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let request = Request::builder()
        .uri("/api/v1/backups?restore_status=not-a-status")
        .body(Body::empty())
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Unknown backup restore status"));
}

#[tokio::test(flavor = "current_thread")]
async fn database_backup_routes_create_and_verify_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let output_dir = dir.path().join("database-backups");
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/database/backups")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"output_dir": output_dir}).to_string(),
        ))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["backup"]["quick_check"], "ok");
    assert_eq!(
        value["data"]["artifact"]["artifact_kind"],
        "database_backup"
    );
    let bundle = value["data"]["backup"]["bundle_path"].as_str().unwrap();
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/database/backups/verify")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"bundle": bundle}).to_string(),
        ))
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["quick_check"], "ok");
    assert_eq!(value["data"]["foreign_key_violations"], 0);
}

#[tokio::test(flavor = "current_thread")]
async fn database_backup_verify_rejects_invalid_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/database/backups/verify")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::json!({"bundle": dir.path().join("missing")}).to_string(),
        ))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Failed to inspect"));
}

#[tokio::test(flavor = "current_thread")]
async fn artifact_routes_inspect_empty_store_and_default_cleanup_to_dry_run() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());

    let request = Request::builder()
        .uri("/api/v1/artifacts/inspection")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["registered"].as_array().unwrap().len(), 0);
    assert_eq!(value["data"]["orphan_files"].as_array().unwrap().len(), 0);

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/artifacts/cleanup")
        .header("content-type", "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, value) = read_json(router(), request).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["applied"], false);
    assert_eq!(
        value["data"]["candidate_orphan_paths"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test(flavor = "current_thread")]
async fn artifact_cleanup_route_rejects_zero_retention() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/artifacts/cleanup")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"retention_hours":0}"#))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("retention must be at least one hour"));
}

#[tokio::test(flavor = "current_thread")]
async fn applied_artifact_cleanup_records_terminal_activity() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/artifacts/cleanup")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"retention_hours":1,"apply":true}"#))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"]["applied"], true);
    let activities = core::list_management_activity(&ActivityQuery {
        operation_kind: Some(ActivityOperationKind::ArtifactCleanup),
        status: Some(ActivityStatus::Success),
        actor: Some(ActivityActor::Api),
        ..ActivityQuery::default()
    })
    .unwrap();
    assert_eq!(activities.len(), 1);
    assert!(activities[0].finished_at_ms.is_some());
    assert_eq!(activities[0].details["applied"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn management_activity_route_rejects_invalid_filters() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let request = Request::builder()
        .uri("/api/v1/management/activity?operation=unknown")
        .body(Body::empty())
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Invalid operation"));
}

#[test]
fn failed_sync_and_backup_operations_remain_queryable() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());

    assert!(
        session_sync::push_sync("missing-group", "missing-holding", ActivityActor::Api).is_err()
    );
    let backup = crate::core::manager::backup(
        &[crate::core::manager::ManagerItem {
            id: crate::core::manager::ManagerItem::action_identity(
                "missing-provider",
                "missing-session",
            ),
            provider_id: "missing-provider".to_string(),
            provider_name: "Missing".to_string(),
            session_id: "missing-session".to_string(),
            source_path: None,
            title: Some("Missing session".to_string()),
            project_dir: Some("/tmp/project".to_string()),
            last_active_at: None,
            size_bytes: 0,
        }],
        &dir.path().join("backups"),
        ActivityActor::Api,
    );
    assert_eq!(backup.failed, 1);

    let sync_activities = core::list_management_activity(&ActivityQuery {
        operation_kind: Some(ActivityOperationKind::Sync),
        status: Some(ActivityStatus::Failed),
        actor: Some(ActivityActor::Api),
        ..ActivityQuery::default()
    })
    .unwrap();
    assert_eq!(sync_activities.len(), 1);
    assert!(sync_activities[0]
        .error
        .as_deref()
        .unwrap()
        .contains("Sync group not found"));

    let backup_activities = core::list_management_activity(&ActivityQuery {
        session_id: Some("missing-session".to_string()),
        operation_kind: Some(ActivityOperationKind::Backup),
        status: Some(ActivityStatus::Failed),
        actor: Some(ActivityActor::Api),
        ..ActivityQuery::default()
    })
    .unwrap();
    assert_eq!(backup_activities.len(), 1);
    assert_eq!(
        backup_activities[0].workspace_dir.as_deref(),
        Some("/tmp/project")
    );
    assert!(backup_activities[0].error.is_some());
}

#[test]
fn management_operations_record_terminal_activity() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());
    let missing_provider = "missing-provider";
    let missing_session = "missing-session";

    core::refresh_projected_session_staleness(ActivityActor::Api).unwrap();
    assert!(core::export_session(
        &core::ExportParams {
            provider: missing_provider.to_string(),
            session_id: missing_session.to_string(),
            output_prefix: None,
            output_dir: None,
            format: "json".to_string(),
        },
        ActivityActor::Api,
    )
    .is_err());
    assert!(core::import_session(
        &core::ImportParams {
            provider: missing_provider.to_string(),
            file_or_id: "missing.json".to_string(),
            to_dir: None,
        },
        ActivityActor::Api,
    )
    .is_err());
    assert!(core::delete_session(missing_provider, missing_session, ActivityActor::Api).is_err());
    assert!(core::rename_session(
        missing_provider,
        missing_session,
        "Renamed",
        ActivityActor::Api,
    )
    .is_err());
    assert!(core::update_session_local_state(
        missing_provider,
        missing_session,
        &crate::storage::session_state::SessionLocalStateUpdate {
            hidden: Some(true),
            ..Default::default()
        },
        ActivityActor::Api,
    )
    .is_err());
    assert!(core::update_session_local_state(
        missing_provider,
        missing_session,
        &crate::storage::session_state::SessionLocalStateUpdate {
            pinned: Some(true),
            ..Default::default()
        },
        ActivityActor::Api,
    )
    .is_err());
    assert!(core::update_session_local_state(
        missing_provider,
        missing_session,
        &crate::storage::session_state::SessionLocalStateUpdate {
            notes: Some(Some("note".to_string())),
            ..Default::default()
        },
        ActivityActor::Api,
    )
    .is_err());
    assert!(core::active_compression_apply(
        &core::ActiveCompressionApplyCommandParams {
            source_provider_id: missing_provider.to_string(),
            target_provider_id: "codex".to_string(),
            session_id: Some(missing_session.to_string()),
            file: None,
            policy: Default::default(),
            candidate_ids: Vec::new(),
            output_prefix: None,
            format: "json".to_string(),
        },
        ActivityActor::Api,
    )
    .is_err());

    let expected = [
        (ActivityOperationKind::Scan, ActivityStatus::Success),
        (ActivityOperationKind::Export, ActivityStatus::Failed),
        (ActivityOperationKind::Import, ActivityStatus::Failed),
        (ActivityOperationKind::Delete, ActivityStatus::Failed),
        (ActivityOperationKind::Rename, ActivityStatus::Failed),
        (ActivityOperationKind::Hide, ActivityStatus::Failed),
        (ActivityOperationKind::Pin, ActivityStatus::Failed),
        (
            ActivityOperationKind::LocalStateUpdate,
            ActivityStatus::Failed,
        ),
        (ActivityOperationKind::Compress, ActivityStatus::Failed),
    ];
    for (operation_kind, status) in expected {
        let activities = core::list_management_activity(&ActivityQuery {
            operation_kind: Some(operation_kind),
            status: Some(status),
            actor: Some(ActivityActor::Api),
            ..ActivityQuery::default()
        })
        .unwrap();
        assert_eq!(
            activities.len(),
            1,
            "missing terminal activity for {operation_kind}"
        );
        assert!(activities[0].finished_at_ms.is_some());
    }
}

struct ArchiveFixture {
    archive_ref: String,
    group_dir: std::path::PathBuf,
    _home: ConfigTestHome,
    _root: tempfile::TempDir,
}

impl Drop for ArchiveFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.group_dir);
    }
}

fn write_api_retrieve_archive_fixture() -> ArchiveFixture {
    let root = tempfile::tempdir().unwrap();
    let home = ConfigTestHome::new(root.path());
    let now = Utc::now();
    let group = format!("api-retrieve-{}", uuid::Uuid::new_v4());
    let archive_dir = config::memorph_dir()
        .unwrap()
        .join("compression_archives")
        .join(&group);
    std::fs::create_dir_all(&archive_dir).unwrap();

    let source = EventSource {
        provider_id: "claude".to_string(),
        original_id: None,
        original_role: Some("user".to_string()),
        phase: None,
    };
    let metadata = EventMetadata {
        source,
        model: None,
        usage: None,
        fidelity: MappingDisposition::Preserved,
        provider_ext: BTreeMap::new(),
    };
    let archive = core::compression::CompressionArchive {
        version: 1,
        created_at: now,
        canonical_id: group.clone(),
        source_provider_id: "claude".to_string(),
        target_provider_id: "codex".to_string(),
        workspace_dir: None,
        summary_event_id: "summary-event".to_string(),
        source_event_ids: vec!["needle-event".to_string(), "other-event".to_string()],
        events: vec![
            SessionEvent {
                id: "needle-event".to_string(),
                kind: SessionEventKind::Message,
                role: EventRole::User,
                timestamp: now,
                links: EventLinks::default(),
                blocks: vec![EventBlock::Text {
                    text: "needle detail from archived original event".to_string(),
                }],
                metadata: metadata.clone(),
            },
            SessionEvent {
                id: "other-event".to_string(),
                kind: SessionEventKind::Message,
                role: EventRole::Assistant,
                timestamp: now,
                links: EventLinks::default(),
                blocks: vec![EventBlock::Text {
                    text: "unrelated archived original event".to_string(),
                }],
                metadata,
            },
        ],
    };
    let file = std::fs::File::create(archive_dir.join("archive.json.gz")).unwrap();
    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    encoder
        .write_all(&serde_json::to_vec(&archive).unwrap())
        .unwrap();
    encoder.finish().unwrap();

    ArchiveFixture {
        archive_ref: format!("memorph-archive://{}/archive.json.gz", group),
        group_dir: archive_dir,
        _home: home,
        _root: root,
    }
}

#[tokio::test]
async fn compression_plan_route_returns_candidates_from_file() {
    let now = Utc::now();
    let session = CanonicalSession {
        schema: CanonicalSchema::default(),
        identity: SessionIdentity {
            canonical_id: "api-dry-run-file".to_string(),
            source_title: Some("API Dry Run File".to_string()),
        },
        provenance: SessionProvenance {
            imported_at: now,
            imported_by: None,
            primary_source: ProviderSessionRef {
                provider_id: "claude".to_string(),
                session_id: "api-dry-run-file".to_string(),
                source_path: None,
            },
            aliases: Vec::new(),
        },
        context: SessionContext::default(),
        events: vec![
            SessionEvent {
                id: "old-user".to_string(),
                kind: SessionEventKind::Message,
                role: EventRole::User,
                timestamp: now,
                links: EventLinks::default(),
                blocks: vec![EventBlock::Text {
                    text: "historical context ".repeat(80),
                }],
                metadata: EventMetadata {
                    source: EventSource {
                        provider_id: "claude".to_string(),
                        original_id: Some("old-user".to_string()),
                        original_role: Some("user".to_string()),
                        phase: None,
                    },
                    model: None,
                    usage: None,
                    fidelity: MappingDisposition::Preserved,
                    provider_ext: BTreeMap::new(),
                },
            },
            SessionEvent {
                id: "recent-user".to_string(),
                kind: SessionEventKind::Message,
                role: EventRole::User,
                timestamp: now,
                links: EventLinks::default(),
                blocks: vec![EventBlock::Text {
                    text: "latest active request".to_string(),
                }],
                metadata: EventMetadata {
                    source: EventSource {
                        provider_id: "claude".to_string(),
                        original_id: Some("recent-user".to_string()),
                        original_role: Some("user".to_string()),
                        phase: None,
                    },
                    model: None,
                    usage: None,
                    fidelity: MappingDisposition::Preserved,
                    provider_ext: BTreeMap::new(),
                },
            },
        ],
        artifacts: Vec::new(),
        extensions: BTreeMap::new(),
    };
    let mut file = Builder::new().suffix(".json").tempfile().unwrap();
    write!(file, "{}", serde_json::to_string(&session).unwrap()).unwrap();

    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/compression/plan")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "source_provider_id": "claude",
                "target_provider_id": "codex",
                "file": file.path().to_string_lossy(),
                "policy": {
                    "protect_recent_message_events": 1,
                    "min_candidate_bytes": 16,
                    "min_savings_ratio_percent": 20,
                    "mode": "plan_only"
                }
            }))
            .unwrap(),
        ))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["dry_run"], true);
    assert_eq!(value["data"]["candidates"][0]["event_ids"][0], "old-user");
    assert_eq!(
        value["data"]["candidates"][0]["reason"],
        "historical_context"
    );
    assert!(
        value["data"]["candidates"][0]["estimated_bytes_saved"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(value["data"]["candidates"][0]["risk"], "medium");
    assert!(value["data"]["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .any(|skipped| skipped["event_id"] == "recent-user"
            && skipped["reason"] == "protected_recent_message"));
}

#[test]
fn session_detail_payload_serializes_hook_runtime_sessions() {
    let payload = SessionDetailPayload {
        view: core::SessionDetailView {
            provider_id: "claude".to_string(),
            provider_name: "claude".to_string(),
            session_id: "session-1".to_string(),
            canonical_id: "canonical-1".to_string(),
            title: Some("Session".to_string()),
            native_title: None,
            display_title: None,
            workspace_dir: Some("/tmp/project".to_string()),
            created_at: None,
            last_active_at: None,
            source_path: None,
            resume_command: None,
            compressed_archive_refs: Vec::new(),
            local_state: ResolvedLocalSessionState::default(),
            event_count: 0,
            message_count: 0,
            artifact_count: 0,
            length_metrics: core::SessionLengthMetrics {
                provider_source_bytes_measured: 0,
                model_visible_bytes_measured: 0,
                estimated_tokens: 0,
                event_count: 0,
                message_count: 0,
                turn_count: 0,
                compressed_segment_count: 0,
                archive_count: 0,
            },
            stale: true,
            hook_runtime_summary: Some(hooks::augmentation::HookRuntimeSummary {
                linked_sessions: 1,
                waiting_sessions: 0,
                status: hooks::model::RuntimeSessionStatus::Running,
                current_tool_name: Some("Bash".to_string()),
                has_pending_permission: false,
                has_pending_question: false,
                last_event_at: None,
                matched_by: Some("provider_session_id".to_string()),
                confidence: Some(hooks::augmentation::HookLinkConfidence::High),
            }),
            hook_diagnosis: Some(hooks::augmentation::SessionHookDiagnosis {
                kind: hooks::augmentation::SessionHookDiagnosisKind::Linked,
                provider_status: hooks::model::HookHealthStatus::InstalledOk,
                linked_runtime_sessions: 1,
                provider_runtime_sessions: 1,
                matched_by: Some("provider_session_id".to_string()),
                confidence: Some(hooks::augmentation::HookLinkConfidence::High),
                last_event_at: None,
                message: "Hook runtime is linked directly to this session.".to_string(),
                actions: Vec::new(),
            }),
            hook_runtime_sessions: vec![runtime_session_for_payload("claude:session:session-1")],
            projection_report: None,
            turns: vec![crate::session_projection::TurnProjection {
                id: "turn-1".to_string(),
                session_id: "canonical-1".to_string(),
                provider_turn_id: None,
                status: crate::session_projection::TurnStatus::Completed,
                confidence: crate::session_projection::TurnConfidence::Inferred,
                started_at_ms: None,
                ended_at_ms: None,
                source_range: crate::session_projection::SourceRange::default(),
                turn_order: 0,
            }],
            events: Vec::new(),
            artifacts: Vec::new(),
        },
        events_offset: 0,
        events_limit: Some(50),
        returned_event_count: 0,
        has_more_events: false,
        hook_runtime_sessions: vec![runtime_session_for_payload("claude:session:session-1")],
    };

    let value = serde_json::to_value(payload).unwrap();
    assert_eq!(value["hook_runtime_sessions"].as_array().unwrap().len(), 1);
    assert_eq!(
        value["view"]["hook_runtime_summary"]["matched_by"],
        "provider_session_id"
    );
    assert_eq!(value["view"]["hook_runtime_summary"]["confidence"], "high");
    assert_eq!(value["view"]["hook_diagnosis"]["kind"], "linked");
    assert_eq!(value["view"]["stale"], true);
    assert_eq!(value["view"]["turns"][0]["confidence"], "inferred");
    assert_eq!(
        value["hook_runtime_sessions"][0]["provider_session_id"],
        "session-1"
    );
}

#[test]
fn sync_holding_payload_serializes_hook_runtime_sessions() {
    let _guard = crate::hooks::test_support::test_runtime_guard();
    let dir = tempfile::tempdir().unwrap();
    crate::hooks::store::set_test_store_root(dir.path().to_path_buf());
    crate::hooks::server::reset_for_tests();
    let endpoint = HookRuntimeEndpoint {
        endpoint: "http://127.0.0.1:3737".to_string(),
        token: "test-token".to_string(),
        pid: 1,
        started_at: Utc::now(),
    };
    crate::hooks::server::set_runtime_endpoint_for_tests(endpoint.clone());

    let request = HookIngestRequest::new(
        "generic",
        "tool_started",
        serde_json::json!({
            "session_id": "session-1",
            "cwd": "/tmp/project",
            "tool": {"name": "Bash", "input": {"command": "cargo check"}}
        }),
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let (status, value) = read_json(
            router(),
            Request::builder()
                .method("POST")
                .uri("/api/v1/hooks/ingest")
                .header("content-type", "application/json")
                .header("x-memorph-hook-token", endpoint.token)
                .body(Body::from(serde_json::to_vec(&request).unwrap()))
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["ok"], true);
    });

    let payload = sync_holding_payload(session_sync::Holding {
        id: "holding-1".to_string(),
        provider: "generic".to_string(),
        session_id: "session-1".to_string(),
        target_dir: Some("/tmp/project".to_string()),
        created_at: 1,
        last_active_at: None,
        last_sync_at: None,
        last_sync_from: None,
        last_error: None,
    });

    let value = serde_json::to_value(payload).unwrap();
    assert_eq!(value["provider"], "generic");
    assert_eq!(value["session_id"], "session-1");
    assert_eq!(value["hook_runtime_summary"]["current_tool_name"], "Bash");
    assert_eq!(
        value["hook_runtime_summary"]["matched_by"],
        "provider_session_id"
    );
    assert_eq!(value["hook_runtime_summary"]["confidence"], "high");
    assert_eq!(value["hook_diagnosis"]["kind"], "linked");
    assert_eq!(value["hook_runtime_sessions"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn compression_apply_route_rejects_ambiguous_source_without_writing_archive() {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/compression/apply")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "source_provider_id": "claude",
                "target_provider_id": "codex",
                "session_id": "s1",
                "file": "session.json",
                "format": "json"
            }))
            .unwrap(),
        ))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(value["ok"], false);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Native compression requires session_id and does not accept file"));
}

#[tokio::test]
async fn compression_retrieve_route_rejects_invalid_archive_ref() {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/compression/retrieve")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "archive_ref": "not-an-archive-ref"
            }))
            .unwrap(),
        ))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(value["ok"], false);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Unsupported compression archive ref"));
}

#[tokio::test(flavor = "current_thread")]
async fn compression_retrieve_route_returns_query_matches_from_archive() {
    let fixture = write_api_retrieve_archive_fixture();
    eprintln!("fixture created: {}", fixture.archive_ref);
    let direct = core::retrieve_compression_archive(&core::RetrieveCompressionArchiveParams {
        archive_ref: fixture.archive_ref.clone(),
        query: Some("needle".to_string()),
        max_results: Some(5),
    });
    eprintln!("direct retrieve ok={}", direct.is_ok());
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/compression/retrieve")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "archive_ref": fixture.archive_ref.clone(),
                "query": "needle",
                "max_results": 5
            }))
            .unwrap(),
        ))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["archive_ref"], fixture.archive_ref);
    assert_eq!(value["data"]["retrieval_mode"], "query_matches");
    assert!(value["data"]["recommended_next_action"]
        .as_str()
        .unwrap()
        .contains("partial retrieval"));
    assert_eq!(value["data"]["source_event_count"], 2);
    assert_eq!(
        value["data"]["returned_event_ids"],
        serde_json::json!(["needle-event"])
    );
    assert_eq!(value["data"]["returned_event_count"], 1);
    assert_eq!(value["data"]["omitted_event_count"], 1);
    assert_eq!(value["data"]["events"][0]["id"], "needle-event");
    assert_eq!(value["data"]["matches"][0]["event_id"], "needle-event");
    assert!(value["data"]["matches"][0]["snippets"][0]
        .as_str()
        .unwrap()
        .contains("needle detail"));
}

#[tokio::test]
async fn compression_tool_spec_route_returns_retrieval_contract() {
    let request = Request::builder()
        .uri("/api/v1/compression/tool-spec")
        .body(Body::empty())
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);
    assert_eq!(
        value["data"]["name"],
        "memorph_retrieve_compression_archive"
    );
    assert_eq!(value["data"]["api"]["path"], "/api/v1/compression/retrieve");
    assert_eq!(
        value["data"]["input_schema"]["required"],
        serde_json::json!(["archive_ref"])
    );
    assert!(value["data"]["usage_rules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rule| rule.as_str().unwrap().contains("Prefer query retrieval")));
    assert!(value["data"]["usage_rules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rule| rule.as_str().unwrap().contains("exact phrase matches")));
}

#[tokio::test]
async fn compression_instructions_route_returns_archive_specific_examples() {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/compression/instructions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "archive_ref": "memorph-archive://group/archive.json.gz"
            }))
            .unwrap(),
        ))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);
    assert_eq!(
        value["data"]["archive_ref"],
        "memorph-archive://group/archive.json.gz"
    );
    assert!(value["data"]["query_first_cli"]
        .as_str()
        .unwrap()
        .contains("--query <terms> --max-results 5"));
    assert_eq!(
        value["data"]["api_query_body"]["archive_ref"],
        "memorph-archive://group/archive.json.gz"
    );
    assert!(value["data"]["suggested_steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step.as_str().unwrap().contains("broader term coverage")));
}

#[tokio::test]
async fn compression_instructions_route_rejects_invalid_archive_ref() {
    let request = Request::builder()
        .method("POST")
        .uri("/api/v1/compression/instructions")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "archive_ref": "not-an-archive-ref"
            }))
            .unwrap(),
        ))
        .unwrap();

    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(value["ok"], false);
    assert!(value["error"]
        .as_str()
        .unwrap()
        .contains("Unsupported compression archive ref"));
}

#[tokio::test]
async fn settings_route_lists_codex_repair_setting() {
    let request = Request::builder()
        .uri("/api/v1/providers/codex/settings")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);

    let settings = value["data"]["settings"].as_array().unwrap();
    assert!(settings
        .iter()
        .any(|setting| setting["id"] == "repair_workspace_sessions"));
}
#[tokio::test]
async fn settings_route_lists_codeisland_gap_provider_hook_actions() {
    let request = Request::builder()
        .uri("/api/v1/providers/qoder/settings")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);

    let settings = value["data"]["settings"].as_array().unwrap();
    for action in [
        "install_hook",
        "verify_hook",
        "repair_hook",
        "uninstall_hook",
    ] {
        assert!(
            settings.iter().any(|setting| setting["id"] == action),
            "missing {action}"
        );
    }
}

#[tokio::test]
async fn provider_settings_route_accepts_provider_aliases() {
    let canonical_request = Request::builder()
        .uri("/api/v1/providers/droid/settings")
        .body(Body::empty())
        .unwrap();
    let alias_request = Request::builder()
        .uri("/api/v1/providers/factory/settings")
        .body(Body::empty())
        .unwrap();

    let (canonical_status, canonical_value) = read_json(router(), canonical_request).await;
    let (alias_status, alias_value) = read_json(router(), alias_request).await;

    assert_eq!(canonical_status, StatusCode::OK);
    assert_eq!(alias_status, StatusCode::OK);
    assert_eq!(
        canonical_value["data"]["settings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|setting| setting["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        alias_value["data"]["settings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|setting| setting["id"].as_str().unwrap())
            .collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn provider_setting_update_updates_settings_payload() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());

    let update_request = Request::builder()
        .method("PUT")
        .uri("/api/v1/providers/opencode/settings/show_subagents")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({ "value": true })).unwrap(),
        ))
        .unwrap();
    let (update_status, update_value) = read_json(router(), update_request).await;

    assert_eq!(update_status, StatusCode::OK);
    assert_eq!(update_value["data"]["value"], true);

    let settings_request = Request::builder()
        .uri("/api/v1/settings")
        .body(Body::empty())
        .unwrap();
    let (settings_status, settings_value) = read_json(router(), settings_request).await;

    assert_eq!(settings_status, StatusCode::OK);
    assert_eq!(settings_value["data"]["show_opencode_subagents"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn settings_update_updates_provider_setting_payload() {
    let dir = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(dir.path());

    let update_request = Request::builder()
        .method("PUT")
        .uri("/api/v1/settings")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "sessions_per_provider": 12,
                "language": "en",
                "show_opencode_subagents": true,
                "sort_providers_by_session_count": true,
                "default_backup_dir": "./backups",
                "logging": {
                    "max_size_bytes": 5 * 1024 * 1024,
                    "retention_days": null
                },
                "home_buttons": {
                    "switch": true,
                    "view": true,
                    "compress": true,
                    "export": true,
                    "sync": false,
                    "delete": false
                },
                "agent_order": [],
                "primary_agents": []
            }))
            .unwrap(),
        ))
        .unwrap();
    let (update_status, update_value) = read_json(router(), update_request).await;

    assert_eq!(update_status, StatusCode::OK);
    assert_eq!(update_value["data"]["show_opencode_subagents"], true);

    let setting_request = Request::builder()
        .uri("/api/v1/providers/opencode/settings/show_subagents")
        .body(Body::empty())
        .unwrap();
    let (setting_status, setting_value) = read_json(router(), setting_request).await;

    assert_eq!(setting_status, StatusCode::OK);
    assert_eq!(setting_value["data"]["id"], "show_subagents");
    assert_eq!(setting_value["data"]["value"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn meta_route_exposes_resolved_backup_and_log_paths() {
    let home_dir = tempfile::tempdir().unwrap();
    let workspace_root = tempfile::tempdir().unwrap();
    let _home = ConfigTestHome::new(home_dir.path());
    let workspace_dir = workspace_root.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();
    crate::config::remember_workspace(&workspace_dir).unwrap();

    let update_request = Request::builder()
        .method("PUT")
        .uri("/api/v1/settings")
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({
                "sessions_per_provider": 12,
                "language": "en",
                "show_opencode_subagents": false,
                "sort_providers_by_session_count": true,
                "default_backup_dir": "./backups",
                "logging": {
                    "max_size_bytes": 5 * 1024 * 1024,
                    "retention_days": null
                },
                "home_buttons": {
                    "switch": true,
                    "view": true,
                    "compress": true,
                    "export": true,
                    "sync": false,
                    "delete": false
                },
                "agent_order": [],
                "primary_agents": []
            }))
            .unwrap(),
        ))
        .unwrap();
    let (update_status, _) = read_json(router(), update_request).await;
    assert_eq!(update_status, StatusCode::OK);

    let meta_request = Request::builder()
        .uri("/api/v1/meta")
        .body(Body::empty())
        .unwrap();
    let (meta_status, meta_value) = read_json(router(), meta_request).await;

    assert_eq!(meta_status, StatusCode::OK);
    assert_eq!(
        meta_value["data"]["settings_paths"]["backup_dir_input"],
        "./backups"
    );
    let expected_base = workspace_dir.canonicalize().unwrap();
    let expected_resolved = expected_base.join("./backups");
    assert_eq!(
        meta_value["data"]["settings_paths"]["backup_dir_base"],
        expected_base.display().to_string()
    );
    assert_eq!(
        meta_value["data"]["settings_paths"]["backup_dir_resolved"],
        expected_resolved.display().to_string()
    );
    assert_eq!(
        meta_value["data"]["settings_paths"]["log_dir"],
        "~/.memorph/logs"
    );
    assert_eq!(
        meta_value["data"]["settings_paths"]["log_file_name"],
        "memorph.log"
    );
    assert_eq!(
        meta_value["data"]["settings_paths"]["log_file_path"],
        "~/.memorph/logs/memorph.log"
    );
}

#[test]
fn resolve_backup_output_dir_uses_workspace_for_relative_paths() {
    let path = resolve_backup_output_dir("./backups", Some("/tmp/current-workspace"));
    assert_eq!(
        path,
        std::path::PathBuf::from("/tmp/current-workspace").join("./backups")
    );

    let absolute = resolve_backup_output_dir("/tmp/exports", Some("/tmp/current-workspace"));
    assert_eq!(absolute, std::path::PathBuf::from("/tmp/exports"));
}

#[tokio::test]
async fn catalog_route_preserves_native_provider_capabilities() {
    let request = Request::builder()
        .uri("/api/v1/providers/catalog")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    let providers = value["data"]["providers"].as_array().unwrap();
    for provider_id in crate::providers::ProviderRegistry::ids() {
        let provider = crate::providers::find_provider(provider_id).unwrap();
        let entry = providers
            .iter()
            .find(|entry| entry["provider_id"] == *provider_id)
            .unwrap_or_else(|| panic!("missing catalog entry for {provider_id}"));
        let expected = serde_json::to_value(provider.capabilities()).unwrap();
        assert_eq!(
            entry["capability_set"], expected,
            "capability drift: {provider_id}"
        );
    }
}

#[tokio::test]
async fn catalog_route_returns_classified_providers() {
    let request = Request::builder()
        .uri("/api/v1/providers/catalog")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);

    let providers = value["data"]["providers"].as_array().unwrap();
    assert!(!providers.is_empty());

    let claude = providers
        .iter()
        .find(|provider| provider["provider_id"] == "claude")
        .expect("missing claude catalog entry");
    assert_eq!(claude["display_name"], "Claude");
    assert!(claude["capability_set"].is_object());
    assert_eq!(claude["capability_set"]["scan_strategy"], "full_scan");
    assert_eq!(claude["capability_set"]["page_strategy"], "indexed_page");
    assert_eq!(claude["capability_set"]["storage_shape"], "jsonl");
    assert_eq!(claude["capability_set"]["turn_quality"], "inferred");
    assert_eq!(
        claude["capability_set"]["export_fidelity"]["patch"],
        "downgraded"
    );
    assert_eq!(claude["capability_set"]["resume_quality"], "native");
    assert_eq!(claude["capability_set"]["write_risk"]["level"], "medium");
    assert!(claude["capability_set"]["backup_support"].is_object());
    assert_eq!(
        claude["capability_set"]["activity_support"]["hook_events"],
        true
    );
    assert!(claude["install_state"].is_object());
    assert!(claude["hidden_state"].is_object());
    assert!(claude["sort_order"].is_object());
    assert!(claude["active_time"].is_object());
    assert!(claude["filter_tags"].is_array());

    let codex = providers
        .iter()
        .find(|provider| provider["provider_id"] == "codex")
        .expect("missing codex catalog entry");
    assert_eq!(codex["capability_set"]["page_strategy"], "indexed_page");

    let kimi = providers
        .iter()
        .find(|provider| provider["provider_id"] == "kimi")
        .expect("missing kimi catalog entry");
    let capabilities = &kimi["capability_set"];
    assert_eq!(capabilities["scan"], true);
    assert_eq!(capabilities["import"], true);
    assert_eq!(capabilities["export"], true);
    assert_eq!(capabilities["delete"], true);
    assert_eq!(capabilities["rename"], true);
    assert_eq!(capabilities["resume"], true);
    assert_eq!(capabilities["scan_strategy"], "hybrid");
    assert_eq!(capabilities["page_strategy"], "full_import");
    assert_eq!(capabilities["storage_shape"], "directory");
    assert_eq!(capabilities["turn_quality"], "inferred");
    assert_eq!(capabilities["import_fidelity"]["text"], "preserved");
    assert_eq!(capabilities["import_fidelity"]["thinking"], "preserved");
    assert_eq!(capabilities["import_fidelity"]["tool_call"], "downgraded");
    assert_eq!(capabilities["import_fidelity"]["tool_result"], "downgraded");
    assert_eq!(capabilities["import_fidelity"]["patch"], "unsupported");
    assert_eq!(capabilities["import_fidelity"]["image"], "normalized");
    assert_eq!(capabilities["import_fidelity"]["file"], "downgraded");
    assert_eq!(capabilities["import_fidelity"]["compressed"], "unsupported");
    assert_eq!(
        capabilities["import_fidelity"]["provider_payload"],
        "preserved"
    );
    assert_eq!(capabilities["export_fidelity"]["text"], "preserved");
    assert_eq!(capabilities["export_fidelity"]["thinking"], "preserved");
    assert_eq!(capabilities["export_fidelity"]["tool_call"], "downgraded");
    assert_eq!(capabilities["export_fidelity"]["tool_result"], "downgraded");
    assert_eq!(capabilities["export_fidelity"]["patch"], "downgraded");
    assert_eq!(capabilities["export_fidelity"]["image"], "downgraded");
    assert_eq!(capabilities["export_fidelity"]["file"], "downgraded");
    assert_eq!(capabilities["export_fidelity"]["compressed"], "downgraded");
    assert_eq!(
        capabilities["export_fidelity"]["provider_payload"],
        "dropped"
    );
    assert_eq!(capabilities["resume_quality"], "native");
    assert_eq!(capabilities["write_risk"]["level"], "medium");
    assert_eq!(capabilities["write_risk"]["multiple_files"], true);
    assert_eq!(capabilities["write_risk"]["sqlite"], false);
    assert_eq!(capabilities["write_risk"]["sidecar_files"], true);
    assert_eq!(capabilities["write_risk"]["index_repair"], false);
    assert_eq!(capabilities["backup_support"]["before_write"], true);
    assert_eq!(capabilities["backup_support"]["restore"], true);
    assert_eq!(capabilities["backup_support"]["sync_only"], false);
    assert_eq!(capabilities["activity_support"]["hook_events"], true);
    assert_eq!(capabilities["activity_support"]["runtime_endpoint"], true);
    assert_eq!(capabilities["activity_support"]["session_activity"], true);

    let kiro = providers
        .iter()
        .find(|provider| provider["provider_id"] == "kiro")
        .expect("missing kiro catalog entry");
    let capabilities = &kiro["capability_set"];
    assert_eq!(kiro["display_name"], "Kiro");
    assert_eq!(capabilities["scan"], true);
    assert_eq!(capabilities["import"], true);
    assert_eq!(capabilities["export"], false);
    assert_eq!(capabilities["delete"], true);
    assert_eq!(capabilities["rename"], true);
    assert_eq!(capabilities["resume"], false);
    assert_eq!(capabilities["scan_strategy"], "full_scan");
    assert_eq!(capabilities["page_strategy"], "full_import");
    assert_eq!(capabilities["storage_shape"], "directory");
    assert_eq!(capabilities["turn_quality"], "exact");
    assert_eq!(capabilities["import_fidelity"]["text"], "preserved");
    assert_eq!(capabilities["import_fidelity"]["thinking"], "preserved");
    assert_eq!(capabilities["import_fidelity"]["tool_call"], "preserved");
    assert_eq!(capabilities["import_fidelity"]["tool_result"], "preserved");
    assert_eq!(capabilities["import_fidelity"]["patch"], "unsupported");
    assert_eq!(capabilities["import_fidelity"]["image"], "unsupported");
    assert_eq!(capabilities["import_fidelity"]["file"], "unsupported");
    assert_eq!(capabilities["import_fidelity"]["compressed"], "unsupported");
    assert_eq!(
        capabilities["import_fidelity"]["provider_payload"],
        "preserved"
    );
    assert_eq!(capabilities["resume_quality"], "none");
    assert_eq!(capabilities["write_risk"]["level"], "medium");
    assert_eq!(capabilities["write_risk"]["multiple_files"], true);
    assert_eq!(capabilities["write_risk"]["sqlite"], false);
    assert_eq!(capabilities["write_risk"]["sidecar_files"], true);
    assert_eq!(capabilities["write_risk"]["index_repair"], false);
    assert_eq!(capabilities["backup_support"]["before_write"], true);
    assert_eq!(capabilities["backup_support"]["restore"], true);
    assert_eq!(capabilities["backup_support"]["sync_only"], false);
    assert_eq!(capabilities["activity_support"]["hook_events"], true);
    assert_eq!(capabilities["activity_support"]["runtime_endpoint"], true);
    assert_eq!(capabilities["activity_support"]["session_activity"], true);

    let opencode = providers
        .iter()
        .find(|provider| provider["provider_id"] == "opencode")
        .expect("missing opencode catalog entry");
    assert_eq!(opencode["capability_set"]["page_strategy"], "native_page");

    let openclaw = providers
        .iter()
        .find(|provider| provider["provider_id"] == "openclaw")
        .expect("missing openclaw catalog entry");
    assert_eq!(openclaw["display_name"], "OpenClaw");
    assert_eq!(openclaw["capability_set"]["scan"], true);
    assert_eq!(openclaw["capability_set"]["import"], true);
    assert_eq!(openclaw["capability_set"]["storage_shape"], "sqlite");
    assert_eq!(openclaw["capability_set"]["resume"], false);
    assert_eq!(openclaw["capability_set"]["delete"], false);

    let gemini = providers
        .iter()
        .find(|provider| provider["provider_id"] == "gemini")
        .expect("missing gemini catalog entry");
    let gemini_capabilities = &gemini["capability_set"];
    assert_eq!(gemini["display_name"], "Gemini");
    assert_eq!(gemini_capabilities["scan"], true);
    assert_eq!(gemini_capabilities["import"], true);
    assert_eq!(gemini_capabilities["export"], false);
    assert_eq!(gemini_capabilities["delete"], true);
    assert_eq!(gemini_capabilities["rename"], false);
    assert_eq!(gemini_capabilities["resume"], true);
    assert_eq!(gemini_capabilities["scan_strategy"], "full_scan");
    assert_eq!(gemini_capabilities["page_strategy"], "full_import");
    assert_eq!(gemini_capabilities["storage_shape"], "jsonl");
    assert_eq!(gemini_capabilities["turn_quality"], "inferred");
    assert_eq!(gemini_capabilities["resume_quality"], "native");
    assert_eq!(gemini_capabilities["import_fidelity"]["text"], "preserved");
    assert_eq!(
        gemini_capabilities["import_fidelity"]["provider_payload"],
        "preserved"
    );
    assert_eq!(
        gemini_capabilities["export_fidelity"]["text"],
        "unsupported"
    );
    assert_eq!(gemini_capabilities["write_risk"]["level"], "medium");
    assert_eq!(gemini_capabilities["write_risk"]["multiple_files"], true);
    assert_eq!(gemini_capabilities["write_risk"]["sqlite"], false);
    assert_eq!(gemini_capabilities["write_risk"]["sidecar_files"], true);
    assert_eq!(gemini_capabilities["backup_support"]["before_write"], true);
    assert_eq!(gemini_capabilities["backup_support"]["restore"], true);
    assert_eq!(gemini_capabilities["activity_support"]["hook_events"], true);
    assert_eq!(
        gemini_capabilities["activity_support"]["runtime_endpoint"],
        false
    );
    assert_eq!(
        gemini_capabilities["activity_support"]["session_activity"],
        false
    );

    let deepseek = providers
        .iter()
        .find(|provider| provider["provider_id"] == "deepseek")
        .expect("missing deepseek catalog entry");
    let deepseek_capabilities = &deepseek["capability_set"];
    assert_eq!(deepseek_capabilities["scan"], true);
    assert_eq!(deepseek_capabilities["import"], true);
    assert_eq!(deepseek_capabilities["export"], true);
    assert_eq!(deepseek_capabilities["delete"], true);
    assert_eq!(deepseek_capabilities["rename"], true);
    assert_eq!(deepseek_capabilities["resume"], true);
    assert_eq!(deepseek_capabilities["scan_strategy"], "full_scan");
    assert_eq!(deepseek_capabilities["page_strategy"], "full_import");
    assert_eq!(deepseek_capabilities["storage_shape"], "sqlite");
    assert_eq!(deepseek_capabilities["turn_quality"], "inferred");
    assert_eq!(deepseek_capabilities["resume_quality"], "native");
    assert_eq!(deepseek_capabilities["write_risk"]["level"], "high");
    assert_eq!(deepseek_capabilities["write_risk"]["sqlite"], true);
    assert_eq!(deepseek_capabilities["write_risk"]["sidecar_files"], true);
    assert_eq!(deepseek_capabilities["write_risk"]["index_repair"], true);
    assert_eq!(
        deepseek_capabilities["backup_support"]["before_write"],
        true
    );
    assert_eq!(deepseek_capabilities["backup_support"]["restore"], true);
    assert_eq!(
        deepseek_capabilities["activity_support"]["hook_events"],
        false
    );
    assert_eq!(
        deepseek_capabilities["activity_support"]["runtime_endpoint"],
        false
    );
    assert_eq!(
        deepseek_capabilities["activity_support"]["session_activity"],
        false
    );
    assert_eq!(
        deepseek_capabilities["export_fidelity"]["tool_call"],
        "downgraded"
    );
    assert_eq!(
        deepseek_capabilities["export_fidelity"]["provider_payload"],
        "dropped"
    );
}

#[test]
fn active_catalog_uses_projected_sessions_for_workspace_activity() {
    let ordered_ids = vec!["codex".to_string(), "claude".to_string()];
    let snapshots = vec![
        crate::storage::snapshot_store::ProjectedSessionSnapshotRow {
            canonical_session_id: "codex:one".to_string(),
            provider_id: "codex".to_string(),
            provider_session_id: Some("one".to_string()),
            title: None,
            display_title: None,
            workspace_dir: Some("/tmp/current".to_string()),
            last_active_at_ms: Some(30),
            source_path: Some("/missing/codex.jsonl".to_string()),
            message_count: Some(0),
            event_count: 0,
            turn_count: 0,
            size_bytes: None,
            hidden: false,
            pinned: false,
            preferred_targets: Vec::new(),
            stale: false,
        },
        crate::storage::snapshot_store::ProjectedSessionSnapshotRow {
            canonical_session_id: "codex:two".to_string(),
            provider_id: "codex".to_string(),
            provider_session_id: Some("two".to_string()),
            title: None,
            display_title: None,
            workspace_dir: Some("/tmp/other".to_string()),
            last_active_at_ms: Some(50),
            source_path: Some("/missing/codex-other.jsonl".to_string()),
            message_count: Some(0),
            event_count: 0,
            turn_count: 0,
            size_bytes: None,
            hidden: false,
            pinned: false,
            preferred_targets: Vec::new(),
            stale: false,
        },
    ];

    let catalog =
        provider_active_catalog_from_snapshots(&ordered_ids, Some("/tmp/current"), &snapshots);

    assert_eq!(catalog.providers.len(), 2);
    assert_eq!(catalog.providers[0].provider_id, "codex");
    assert!(catalog.providers[0].has_sessions);
    assert_eq!(catalog.providers[0].active_time.global, 50);
    assert_eq!(catalog.providers[0].active_time.workspace, 30);
    assert_eq!(catalog.providers[1].provider_id, "claude");
    assert!(!catalog.providers[1].has_sessions);
    assert_eq!(catalog.providers[1].active_time.global, 0);
    assert_eq!(catalog.providers[1].active_time.workspace, 0);
}

#[tokio::test]
async fn agents_route_exposes_settings_field() {
    let request = Request::builder()
        .uri("/api/v1/agents")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);

    let providers = value["data"]["providers"].as_array().unwrap();
    let codex = providers
        .iter()
        .find(|provider| provider["provider_id"] == "codex")
        .expect("missing codex agent entry");
    assert!(codex.get("settings").is_some());
    assert!(codex.get("environment").is_some());
    assert!(codex.get("features").is_none());
}

#[tokio::test]
async fn agents_summary_route_omits_expensive_session_diagnosis() {
    let request = Request::builder()
        .uri("/api/v1/agents/summary")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);

    let providers = value["data"]["providers"].as_array().unwrap();
    let codex = providers
        .iter()
        .find(|provider| provider["provider_id"] == "codex")
        .expect("missing codex agent summary");
    assert!(codex.get("settings").is_some());
    assert!(codex.get("environment").is_some());
    assert!(codex.get("hook").is_some());
    assert!(codex.get("hook_diagnosis").is_none());
}

#[tokio::test]
async fn agents_route_exposes_all_hook_profile_providers() {
    let request = Request::builder()
        .uri("/api/v1/agents")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    let providers = value["data"]["providers"].as_array().unwrap();
    for descriptor in crate::hooks::registry::all() {
        let entry = providers
            .iter()
            .find(|provider| provider["provider_id"] == descriptor.provider())
            .unwrap_or_else(|| panic!("missing agent entry for {}", descriptor.provider()));
        assert_eq!(entry["hook"]["provider"], descriptor.provider());
        assert_eq!(entry["hook_profile"]["provider"], descriptor.provider());
        assert_eq!(
            entry["hook_required_events"].as_array().unwrap().len(),
            descriptor.required_events.len()
        );
        assert!(entry["hook_profile"]["events"].as_array().unwrap().len() > 0);
        assert!(entry["settings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|setting| setting["id"] == "install_hook"));
    }
}

#[tokio::test]
async fn agents_route_keeps_environment_block_and_flat_fields_in_sync() {
    let request = Request::builder()
        .uri("/api/v1/agents")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    let providers = value["data"]["providers"].as_array().unwrap();
    let codex = providers
        .iter()
        .find(|provider| provider["provider_id"] == "codex")
        .expect("missing codex agent entry");

    assert_eq!(codex["environment"]["installed"], codex["installed"]);
    assert_eq!(codex["environment"]["config_path"], codex["config_path"]);
    assert_eq!(
        codex["environment"]["install_method"],
        codex["install_method"]
    );
}

#[tokio::test]
async fn agent_detail_route_returns_single_provider_entry() {
    let request = Request::builder()
        .uri("/api/v1/agents/codex")
        .body(Body::empty())
        .unwrap();
    let (status, value) = read_json(router(), request).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["ok"], true);
    assert_eq!(value["data"]["provider_id"], "codex");
    assert!(value["data"]["settings"].is_array());
    assert!(value["data"]["environment"].is_object());
    assert_eq!(
        value["data"]["environment"]["config_path"],
        value["data"]["config_path"]
    );
}

#[tokio::test]
async fn agent_detect_route_matches_detail_route_for_provider_entry() {
    let detail_request = Request::builder()
        .uri("/api/v1/agents/codex")
        .body(Body::empty())
        .unwrap();
    let detect_request = Request::builder()
        .method("POST")
        .uri("/api/v1/agents/codex/detect")
        .body(Body::empty())
        .unwrap();

    let (detail_status, detail_value) = read_json(router(), detail_request).await;
    let (detect_status, detect_value) = read_json(router(), detect_request).await;

    assert_eq!(detail_status, StatusCode::OK);
    assert_eq!(detect_status, StatusCode::OK);
    assert_eq!(
        detail_value["data"]["provider_id"],
        detect_value["data"]["provider_id"]
    );
    assert_eq!(
        detail_value["data"]["environment"],
        detect_value["data"]["environment"]
    );
}

#[tokio::test]
async fn provider_feature_and_control_routes_are_not_registered() {
    for path in [
        "/api/v1/providers/codex/features",
        "/api/v1/providers/codex/controls",
    ] {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        let response = router().oneshot(request).await.unwrap();

        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "unexpected route: {path}"
        );
    }
}
