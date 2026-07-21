use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use self::transfer::ExportResult;
use crate::canonical::{
    CanonicalSession, ImportedSession, SessionArtifact, SessionEvent, SessionEventKind,
};
use crate::core::active_compression::{
    ActiveCompressionApplyParams, ActiveCompressionParams, ActiveCompressionPolicy,
    ActiveCompressionReport,
};
use crate::provider::{Provider, ProviderSessionSummary};
use crate::storage::session_state;
use crate::storage::snapshot_store::{
    ProjectedSessionIdentityRow, ProjectedSessionSnapshotRow, SnapshotStaleScanReport,
    StaleSnapshotSourceRow,
};
use crate::storage::{
    activity_store::{
        ActivityActor, ActivityCompletion, ActivityOperationKind, ActivityQuery, ActivityRecord,
        ActivityStatus, ActivityStore, NewActivity,
    },
    artifact_store::{
        default_managed_artifact_root, ArtifactCleanupReport, ArtifactInspectionReport,
        ArtifactManifestKind, ArtifactStore, NewArtifactManifest,
    },
    local_store,
};
use crate::{provider, providers, utils};

pub mod active_compression;
pub mod compression;
pub mod compression_application;
pub mod database_management;
pub mod management;
pub mod manager;
pub mod projection;
pub mod query;
pub mod session_management;
pub mod sessions;
pub mod transfer;

const MEMORPH_ARCHIVE_SCHEME: &str = "memorph-archive://";
const PROJECTED_SESSION_PROVIDER_IDS: &[&str] = &[
    // Providers that passed the source-backed projection gate.
    "claude",
    "codex",
    "cursor",
    "deepseek",
    "gemini",
    "kimi",
    "kiro",
    "opencode",
    "openclaw",
    "cline",
    "copilot",
    "droid",
    "hermes",
    "codebuddy",
    "qoder",
    "workbuddy",
    "pi",
    "antigravity",
    "windsurf",
    "trae",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListParams {
    pub all: bool,
    pub providers: Vec<String>,
    pub cwd: Option<String>,
    pub include_message_counts: bool,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    #[serde(default)]
    pub sort: SessionListSort,
    #[serde(default)]
    pub hook_filter: SessionHookFilter,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionListSort {
    #[default]
    Recent,
    Title,
    HookAttention,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionHookFilter {
    #[default]
    All,
    Attention,
    Weak,
    Runtime,
    NoHook,
    NoMatch,
    Linked,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionGroup {
    pub provider_id: String,
    pub provider_name: String,
    pub sessions: Vec<SessionItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionItem {
    pub session_id: String,
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_targets: Vec<String>,
    pub project_dir: Option<String>,
    pub last_active_at: Option<i64>,
    pub source_path: Option<String>,
    pub provider_id: String,
    pub message_count: Option<usize>,
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_runtime_summary: Option<crate::hooks::augmentation::HookRuntimeSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_diagnosis: Option<crate::hooks::augmentation::SessionHookDiagnosis>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLengthMetrics {
    pub provider_source_bytes_measured: u64,
    pub model_visible_bytes_measured: u64,
    pub estimated_tokens: u64,
    pub event_count: usize,
    pub message_count: usize,
    pub turn_count: usize,
    pub compressed_segment_count: usize,
    pub archive_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetailView {
    pub provider_id: String,
    pub provider_name: String,
    pub session_id: String,
    pub canonical_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    pub local_state: session_state::ResolvedLocalSessionState,
    pub event_count: usize,
    pub message_count: usize,
    pub artifact_count: usize,
    pub length_metrics: SessionLengthMetrics,
    pub stale: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_runtime_summary: Option<crate::hooks::augmentation::HookRuntimeSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_diagnosis: Option<crate::hooks::augmentation::SessionHookDiagnosis>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hook_runtime_sessions: Vec<crate::hooks::model::RuntimeSession>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection_report: Option<SessionProjectionReportView>,
    pub turns: Vec<crate::session_projection::TurnProjection>,
    pub events: Vec<SessionEvent>,
    pub artifacts: Vec<SessionArtifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compressed_archive_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionProjectionReportView {
    pub id: String,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub operation_kind: crate::session_projection::ProjectionOperationKind,
    pub projection_version: i64,
    pub status: crate::session_projection::ProjectionStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub created_at_ms: i64,
    pub summary: SessionProjectionReportSummaryView,
    pub item_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<SessionProjectionReportItemView>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionProjectionReportSummaryView {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_event_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping_direction: Option<crate::canonical::MappingDirection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mapping_overall: Option<crate::canonical::MappingDisposition>,
    pub preserved_count: usize,
    pub normalized_count: usize,
    pub dropped_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionProjectionReportItemView {
    pub item_order: i64,
    pub fidelity: crate::session_projection::ProjectionFidelity,
    pub scope: crate::session_projection::ProjectionItemScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

pub(super) fn register_session_export_artifacts(
    conn: &mut rusqlite::Connection,
    operation_id: &str,
    provider_id: &str,
    provider_session_id: &str,
    requested_format: &str,
    export: &ExportResult,
) -> Result<Vec<crate::storage::artifact_store::ArtifactManifest>> {
    let manifests = export
        .files
        .iter()
        .map(|file| {
            let path = std::path::PathBuf::from(file);
            let extension = path
                .extension()
                .and_then(|value| value.to_str())
                .with_context(|| format!("Export file has no supported extension: {file}"))?;
            let (format, mime_type) = match extension {
                "morph" => ("morph", "application/x-ndjson"),
                "json" => ("json", "application/json"),
                "md" => ("md", "text/markdown"),
                "html" => ("html", "text/html"),
                _ => anyhow::bail!("Export file has unsupported extension: {file}"),
            };
            Ok(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::SessionExport,
                operation_id: Some(operation_id.to_string()),
                provider_id: Some(provider_id.to_string()),
                provider_session_id: Some(provider_session_id.to_string()),
                session_id: None,
                projection_report_id: None,
                path,
                mime_type: Some(mime_type.to_string()),
                format: Some(format.to_string()),
                metadata: serde_json::json!({
                    "role": "canonical_session_export",
                    "requested_format": requested_format,
                }),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ArtifactStore::new(conn).register_paths(manifests)
}

pub fn delete_session(provider_id: &str, session_id: &str, actor: ActivityActor) -> Result<()> {
    delete_sessions(provider_id, &[session_id], actor)
        .into_iter()
        .next()
        .unwrap_or_else(|| Err(anyhow::anyhow!("No delete result for session {session_id}")))
}

pub fn delete_sessions(
    provider_id: &str,
    session_ids: &[&str],
    actor: ActivityActor,
) -> Vec<Result<()>> {
    let mut activity_conn = match local_store::open_database() {
        Ok(conn) => conn,
        Err(error) => {
            let message = format!("Failed to open activity store before delete: {error:#}");
            return session_ids
                .iter()
                .map(|_| Err(anyhow::anyhow!(message.clone())))
                .collect();
        }
    };
    let backup_root = match crate::config::memorph_dir() {
        Ok(path) => path.join("artifacts").join("backups"),
        Err(error) => {
            let message =
                format!("Failed to resolve provider backup root before delete: {error:#}");
            return session_ids
                .iter()
                .map(|_| Err(anyhow::anyhow!(message.clone())))
                .collect();
        }
    };
    let mut activities = Vec::with_capacity(session_ids.len());
    for session_id in session_ids {
        match ActivityStore::new(&activity_conn).start(NewActivity {
            provider_id: Some(provider_id.to_string()),
            provider_session_id: Some((*session_id).to_string()),
            workspace_dir: None,
            operation_kind: ActivityOperationKind::Delete,
            actor,
            summary: "Deleting session".to_string(),
            details: serde_json::json!({"provider_session_id": session_id}),
        }) {
            Ok(activity_id) => activities.push(activity_id),
            Err(error) => {
                let message = format!("Failed to start delete activity: {error:#}");
                for (started_session_id, activity_id) in session_ids.iter().zip(activities.iter()) {
                    let _ = ActivityStore::new(&activity_conn).finish(
                        activity_id,
                        ActivityCompletion::failed(
                            "Delete cancelled before provider write",
                            serde_json::json!({
                                "provider_session_id": started_session_id,
                            }),
                            &message,
                        ),
                    );
                }
                return session_ids
                    .iter()
                    .map(|_| Err(anyhow::anyhow!(message.clone())))
                    .collect();
            }
        }
    }

    let results = session_management::delete_sessions(
        provider_id,
        session_ids,
        &activities,
        &backup_root,
        &mut activity_conn,
    );
    results
        .into_iter()
        .zip(activities)
        .zip(session_ids)
        .map(|((result, activity_id), session_id)| match result {
            Ok(()) => {
                ActivityStore::new(&activity_conn).finish(
                    &activity_id,
                    ActivityCompletion::success(
                        "Deleted session",
                        serde_json::json!({"provider_session_id": session_id}),
                    ),
                )?;
                Ok(())
            }
            Err(error) => {
                let message = format!("{error:#}");
                ActivityStore::new(&activity_conn).finish(
                    &activity_id,
                    ActivityCompletion::failed(
                        "Failed to delete session",
                        serde_json::json!({"provider_session_id": session_id}),
                        &message,
                    ),
                )?;
                Err(error)
            }
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameResult {
    pub provider_name: String,
    pub session_id: String,
    pub display_title: String,
    pub native_updated: bool,
    pub warning: Option<String>,
}

pub fn rename_session(
    provider_id: &str,
    session_id: &str,
    new_title: &str,
    actor: ActivityActor,
) -> Result<RenameResult> {
    let mut activity_conn = local_store::open_database()?;
    let backup_root = crate::config::memorph_dir()?
        .join("artifacts")
        .join("backups");
    let details = serde_json::json!({
        "provider_session_id": session_id,
        "new_title": new_title,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(provider_id.to_string()),
        provider_session_id: Some(session_id.to_string()),
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Rename,
        actor,
        summary: "Renaming session".to_string(),
        details: details.clone(),
    })?;
    match session_management::rename_session(
        provider_id,
        session_id,
        new_title,
        &activity_id,
        &backup_root,
        &mut activity_conn,
    ) {
        Ok(renamed) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Renamed session",
                    serde_json::json!({
                        "provider_session_id": session_id,
                        "display_title": renamed.display_title,
                        "native_updated": renamed.native_updated,
                        "warning": renamed.warning,
                    }),
                ),
            )?;
            Ok(renamed)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed("Failed to rename session", details, &message),
            )?;
            Err(error)
        }
    }
}

pub fn update_session_local_state(
    provider_id: &str,
    session_id: &str,
    update: &session_state::SessionLocalStateUpdate,
    actor: ActivityActor,
) -> Result<session_state::ResolvedLocalSessionState> {
    let operation_kind = local_state_activity_kind(update);
    let activity_conn = local_store::open_database()?;
    let input_details = serde_json::to_value(update)?;
    let workspace_dir = update
        .workspace_override
        .as_ref()
        .map(|workspace| workspace.workspace_dir.clone());
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(provider_id.to_string()),
        provider_session_id: Some(session_id.to_string()),
        workspace_dir,
        operation_kind,
        actor,
        summary: "Updating session local state".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let prov = providers::find_provider(provider_id)
            .with_context(|| format!("Unknown provider: {}", provider_id))?;
        let projected_identity = crate::storage::snapshot_store::SnapshotStore::new(&activity_conn)
            .find_session_identity(provider_id, session_id)?;
        if projected_identity.is_none() {
            anyhow::bail!("Projected session not found: {}", session_id);
        }

        let mut normalized_update = update.clone();
        if let Some(workspace_override) = normalized_update.workspace_override.as_mut() {
            let workspace = workspace_override.workspace_dir.trim();
            if workspace.is_empty() {
                anyhow::bail!("Workspace path cannot be empty");
            }
            workspace_override.workspace_dir = prov
                .normalized_workspace_key(Some(workspace))
                .with_context(|| format!("Failed to normalize workspace: {}", workspace))?;
        }

        session_state::update_session_state(provider_id, session_id, &normalized_update)
    })();
    match result {
        Ok(state) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Updated session local state",
                    serde_json::to_value(&state)?,
                ),
            )?;
            Ok(state)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to update session local state",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

fn local_state_activity_kind(
    update: &session_state::SessionLocalStateUpdate,
) -> ActivityOperationKind {
    let only_hidden = update.hidden.is_some()
        && update.pinned.is_none()
        && update.display_title.is_none()
        && update.notes.is_none()
        && update.tags.is_none()
        && update.preferred_targets.is_none()
        && update.compressed_archive_refs.is_none()
        && update.workspace_override.is_none();
    if only_hidden {
        return ActivityOperationKind::Hide;
    }
    let only_pinned = update.pinned.is_some()
        && update.hidden.is_none()
        && update.display_title.is_none()
        && update.notes.is_none()
        && update.tags.is_none()
        && update.preferred_targets.is_none()
        && update.compressed_archive_refs.is_none()
        && update.workspace_override.is_none();
    if only_pinned {
        return ActivityOperationKind::Pin;
    }
    let only_workspace_override = update.workspace_override.is_some()
        && update.hidden.is_none()
        && update.pinned.is_none()
        && update.display_title.is_none()
        && update.notes.is_none()
        && update.tags.is_none()
        && update.preferred_targets.is_none()
        && update.compressed_archive_refs.is_none();
    if only_workspace_override {
        let workspace = update.workspace_override.as_ref().unwrap();
        let only_workspace_hidden = workspace.hidden.is_some()
            && workspace.pinned.is_none()
            && workspace.preferred_targets.is_none();
        if only_workspace_hidden {
            return ActivityOperationKind::Hide;
        }
        let only_workspace_pinned = workspace.pinned.is_some()
            && workspace.hidden.is_none()
            && workspace.preferred_targets.is_none();
        if only_workspace_pinned {
            return ActivityOperationKind::Pin;
        }
    }
    ActivityOperationKind::LocalStateUpdate
}

#[cfg(test)]
mod tests {
    use super::compression_application::*;
    use super::projection::*;
    use super::sessions::*;
    use super::*;
    use crate::canonical::{
        CanonicalSchema, EventBlock, EventLinks, EventMetadata, EventRole, EventSource,
        MappingDirection, MappingDisposition, MappingReport, ProviderSessionRef, SessionContext,
        SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance,
    };
    use crate::hooks::model::{
        HookToolCall, PermissionRequest, QuestionRequest, RuntimeSession, RuntimeSessionId,
        RuntimeSessionStatus,
    };
    use crate::provider::Provider;
    use crate::storage::{local_store, snapshot_store::StaleSnapshotSourceRow};
    use chrono::Utc;
    use std::collections::BTreeMap;
    use std::io::Write;
    use std::path::Path;
    use tempfile::Builder;

    fn required_provider_source_fingerprint(
        provider: &dyn Provider,
        source_path: &Path,
    ) -> crate::provider::ProviderSourceFingerprint {
        provider
            .session_source_fingerprint(source_path.to_str().unwrap())
            .unwrap()
            .expect("provider source fingerprint")
    }

    #[test]
    fn registers_canonical_export_files_with_operation_identity() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("session.json");
        let markdown_path = dir.path().join("session.md");
        std::fs::write(&json_path, b"{}").unwrap();
        std::fs::write(&markdown_path, b"# Session").unwrap();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        register_session_export_artifacts(
            &mut conn,
            "operation-1",
            "claude",
            "provider-session-1",
            "both",
            &ExportResult {
                files: vec![
                    json_path.display().to_string(),
                    markdown_path.display().to_string(),
                ],
            },
        )
        .unwrap();

        let rows = crate::storage::artifact_store::ArtifactStore::new(&mut conn)
            .query(crate::storage::artifact_store::ArtifactQuery {
                artifact_kind: Some(ArtifactManifestKind::SessionExport),
                operation_id: Some("operation-1".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|artifact| {
            artifact.provider_id.as_deref() == Some("claude")
                && artifact.provider_session_id.as_deref() == Some("provider-session-1")
                && artifact.metadata["role"] == "canonical_session_export"
                && artifact.metadata["requested_format"] == "both"
        }));
        assert!(rows
            .iter()
            .any(|artifact| artifact.mime_type.as_deref() == Some("application/json")));
        assert!(rows
            .iter()
            .any(|artifact| artifact.mime_type.as_deref() == Some("text/markdown")));
    }

    #[test]
    fn compression_archive_exports_register_complete_artifact_matrix() {
        let root = tempfile::tempdir().unwrap();
        let _home = TestConfigHomeGuard::new(root.path());
        let source = active_compression_source_session();
        let source_file = write_active_compression_source_file(&source);

        let expanded = expand_compression_session(
            &ExpandCompressionSessionParams {
                file: source_file.path().display().to_string(),
                output_prefix: Some(root.path().join("expanded").display().to_string()),
                format: "json".to_string(),
            },
            ActivityActor::System,
        )
        .unwrap();
        let applied = apply_active_compression_to_session(
            &ActiveCompressionApplyCommandParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                session_id: None,
                file: Some(source_file.path().display().to_string()),
                policy: active_compression::ActiveCompressionPolicy {
                    protect_recent_message_events: 1,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: active_compression::ActiveCompressionMode::Auto,
                },
                candidate_ids: vec!["candidate-0001".to_string()],
                output_prefix: None,
                format: "json".to_string(),
            },
            &source,
            compression::archive_base_dir().unwrap().as_path(),
        )
        .unwrap();
        let restored = restore_compression_archive(
            &RestoreCompressionArchiveParams {
                archive_ref: applied.report.archive_refs[0].clone(),
                output_prefix: Some(root.path().join("restored").display().to_string()),
                format: "json".to_string(),
            },
            ActivityActor::System,
        )
        .unwrap();

        assert!(expanded.files.iter().all(|file| Path::new(file).exists()));
        assert!(restored.files.iter().all(|file| Path::new(file).exists()));

        let mut conn = local_store::open_database().unwrap();
        let activities = ActivityStore::new(&conn)
            .query(&ActivityQuery {
                operation_kind: Some(ActivityOperationKind::Compress),
                status: Some(ActivityStatus::Success),
                actor: Some(ActivityActor::System),
                limit: Some(10),
                ..Default::default()
            })
            .unwrap();
        let cases = [
            ("Expanded compressed session", 1, 0),
            ("Restored compression archive", 1, 0),
        ];
        for (summary, expected_exports, expected_archives) in cases {
            let activity = activities
                .iter()
                .find(|activity| activity.summary == summary)
                .unwrap_or_else(|| panic!("missing activity: {summary}"));
            let artifacts = ArtifactStore::new(&mut conn)
                .query(crate::storage::artifact_store::ArtifactQuery {
                    operation_id: Some(activity.id.clone()),
                    limit: Some(10),
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(
                artifacts
                    .iter()
                    .filter(|artifact| {
                        artifact.artifact_kind == ArtifactManifestKind::SessionExport
                    })
                    .count(),
                expected_exports
            );
            assert_eq!(
                artifacts
                    .iter()
                    .filter(|artifact| {
                        artifact.artifact_kind == ArtifactManifestKind::CompressionArchive
                    })
                    .count(),
                expected_archives
            );
            for artifact in &artifacts {
                assert_eq!(
                    artifact.operation_id.as_deref(),
                    Some(activity.id.as_str()),
                    "{summary}: {artifact:?}"
                );
                assert_eq!(
                    artifact.provider_id.as_deref(),
                    Some("claude"),
                    "{summary}: {artifact:?}"
                );
                assert_eq!(
                    artifact.provider_session_id.as_deref(),
                    Some("dry-run-file"),
                    "{summary}: {artifact:?}"
                );
                assert!(artifact.path.exists(), "{summary}: {artifact:?}");
            }
            let detail_ids = activity.details["artifact_ids"].as_array().unwrap();
            assert_eq!(detail_ids.len(), artifacts.len());
            assert!(artifacts.iter().all(|artifact| {
                detail_ids
                    .iter()
                    .any(|value| value.as_str() == Some(artifact.id.as_str()))
            }));
        }
    }

    #[test]
    fn session_length_metrics_distinguish_measured_bytes_and_estimated_tokens() {
        let session = active_compression_source_session();
        let metrics = session_length_metrics(12_345, &session, 4, 3, 2).unwrap();
        assert_eq!(metrics.provider_source_bytes_measured, 12_345);
        assert_eq!(metrics.event_count, 4);
        assert_eq!(metrics.message_count, 3);
        assert_eq!(metrics.turn_count, 2);
        assert_eq!(
            metrics.estimated_tokens,
            metrics.model_visible_bytes_measured.div_ceil(4)
        );
        assert_eq!(metrics.compressed_segment_count, 0);
        assert_eq!(metrics.archive_count, 0);
    }

    #[test]
    fn hook_runtime_summary_prefers_pending_states_and_latest_status() {
        let mut latest =
            runtime_session_fixture("runtime-1", RuntimeSessionStatus::WaitingPermission);
        latest.current_tool = Some(HookToolCall {
            id: None,
            name: "Bash".to_string(),
            input: serde_json::json!({"command": "cargo check"}),
        });
        latest.pending_permission = Some(PermissionRequest {
            request_id: Some("perm-1".to_string()),
            tool: None,
            prompt: Some("Allow Bash?".to_string()),
        });

        let mut older = runtime_session_fixture("runtime-2", RuntimeSessionStatus::WaitingUser);
        older.pending_question = Some(QuestionRequest {
            request_id: Some("question-1".to_string()),
            prompt: "Continue?".to_string(),
        });
        older.last_event_at = Utc::now() - chrono::TimeDelta::seconds(30);

        let summary = crate::hooks::augmentation::summarize_runtime_sessions(
            &[latest.clone(), older.clone()],
            "claude",
            "session-1",
            Some("/tmp/project"),
        )
        .unwrap();

        assert_eq!(summary.linked_sessions, 2);
        assert_eq!(summary.waiting_sessions, 2);
        assert_eq!(summary.status, RuntimeSessionStatus::WaitingPermission);
        assert_eq!(summary.current_tool_name.as_deref(), Some("Bash"));
        assert!(summary.has_pending_permission);
        assert!(summary.has_pending_question);
        assert_eq!(summary.last_event_at, Some(latest.last_event_at));
        assert_eq!(summary.matched_by.as_deref(), Some("provider_session_id"));
        assert_eq!(
            summary.confidence,
            Some(crate::hooks::augmentation::HookLinkConfidence::High)
        );
    }

    fn runtime_session_fixture(id: &str, status: RuntimeSessionStatus) -> RuntimeSession {
        let now = Utc::now();
        RuntimeSession {
            runtime_id: RuntimeSessionId::new(id),
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
            status,
            current_tool: None,
            pending_permission: None,
            pending_question: None,
            recent_activity: Vec::new(),
            subagents: BTreeMap::new(),
            last_event_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn imported_session_title_prefers_display_title_before_native_and_meta() {
        let meta = ProviderSessionSummary {
            session_id: "session-1".to_string(),
            title: Some("Meta".to_string()),
            project_dir: Some("/tmp/project".to_string()),
            last_active_at: None,
            source_path: Some("/tmp/session.jsonl".to_string()),
        };
        let mut imported = ImportedSession {
            session: CanonicalSession {
                schema: CanonicalSchema::default(),
                identity: SessionIdentity {
                    canonical_id: "canonical-1".to_string(),
                    source_title: Some("Native".to_string()),
                },
                provenance: SessionProvenance {
                    imported_at: Utc::now(),
                    imported_by: None,
                    primary_source: ProviderSessionRef {
                        provider_id: "codex".to_string(),
                        session_id: "session-1".to_string(),
                        source_path: None,
                    },
                    aliases: Vec::new(),
                },
                context: SessionContext {
                    workspace_dir: None,
                    created_at: None,
                    last_active_at: None,
                    tags: Vec::new(),
                },
                events: Vec::new(),
                artifacts: Vec::new(),
                extensions: BTreeMap::new(),
            },
            report: MappingReport::new("codex", MappingDirection::Import),
        };

        apply_imported_session_title(&mut imported, &meta, Some("Display".to_string()));

        assert_eq!(
            imported.session.identity.source_title.as_deref(),
            Some("Display")
        );
    }

    #[test]
    fn session_from_compression_archive_restores_source_events() {
        let now = Utc::now();
        let archive = compression::CompressionArchive {
            version: 1,
            created_at: now,
            canonical_id: "canonical-archive".to_string(),
            source_provider_id: "opencode".to_string(),
            target_provider_id: "codex".to_string(),
            workspace_dir: None,
            summary_event_id: "summary-event".to_string(),
            source_event_ids: vec!["old-event".to_string()],
            events: vec![SessionEvent {
                id: "old-event".to_string(),
                kind: SessionEventKind::Message,
                role: EventRole::User,
                timestamp: now,
                links: EventLinks::default(),
                blocks: vec![EventBlock::Text {
                    text: "restored source context".to_string(),
                }],
                metadata: EventMetadata {
                    source: EventSource {
                        provider_id: "opencode".to_string(),
                        original_id: Some("old-event".to_string()),
                        original_role: None,
                        phase: None,
                    },
                    model: None,
                    usage: None,
                    fidelity: MappingDisposition::Preserved,
                    provider_ext: BTreeMap::new(),
                },
            }],
        };

        let session = session_management::session_from_compression_archive(
            "memorph-archive://test/archive.json",
            archive,
        )
        .unwrap();

        assert_eq!(session.identity.canonical_id, "canonical-archive");
        assert_eq!(session.events.len(), 1);
        assert_eq!(session.provenance.primary_source.provider_id, "memorph");
        assert_eq!(
            session.provenance.primary_source.source_path.as_deref(),
            Some("memorph-archive://test/archive.json")
        );
        assert_eq!(session.context.tags, vec!["compression-archive"]);
        assert!(session.extensions.contains_key("compression_archive"));
    }

    #[test]
    fn list_compression_provider_support_marks_native_and_portable_providers() {
        let support = list_compression_provider_support();
        let opencode = support
            .iter()
            .find(|item| item.provider_id == "opencode")
            .expect("opencode support profile");
        assert_eq!(
            opencode.default_projection,
            crate::provider::CompressionProjection::Native
        );
        assert!(opencode.detects_native_source);
        assert!(opencode.native_target_projection);

        let codex = support
            .iter()
            .find(|item| item.provider_id == "codex")
            .expect("codex support profile");
        assert_eq!(
            codex.default_projection,
            crate::provider::CompressionProjection::Native
        );
        assert!(codex.detects_native_source);
        assert!(codex.native_target_projection);
    }

    #[test]
    fn compression_retrieval_tool_spec_is_machine_readable_and_query_first() {
        let spec = compression_retrieval_tool_spec();

        assert_eq!(spec.name, "memorph_retrieve_compression_archive");
        assert_eq!(spec.archive_ref_scheme, "memorph-archive://");
        assert_eq!(spec.api.method, "POST");
        assert_eq!(spec.api.path, "/api/v1/compression/retrieve");
        assert_eq!(
            spec.input_schema["required"],
            serde_json::json!(["archive_ref"])
        );
        assert!(spec.input_schema["properties"].get("query").is_some());
        assert!(spec
            .usage_rules
            .iter()
            .any(|rule| rule.contains("Do not expand")));
        assert!(spec
            .usage_rules
            .iter()
            .any(|rule| rule.contains("Prefer query retrieval")));
        assert!(spec
            .usage_rules
            .iter()
            .any(|rule| rule.contains("exact phrase matches")));
    }

    #[test]
    fn compression_retrieval_instructions_are_archive_specific_and_query_first() {
        let instructions =
            compression_retrieval_instructions("memorph-archive://session/archive.json.gz")
                .unwrap();

        assert_eq!(
            instructions.archive_ref,
            "memorph-archive://session/archive.json.gz"
        );
        assert!(instructions
            .query_first_cli
            .contains("--query <terms> --max-results 5"));
        assert_eq!(
            instructions.full_cli,
            "memorph compression retrieve memorph-archive://session/archive.json.gz"
        );
        assert_eq!(
            instructions.api_query_body["archive_ref"],
            "memorph-archive://session/archive.json.gz"
        );
        assert_eq!(instructions.api_query_body["max_results"], 5);
        assert!(instructions
            .suggested_steps
            .iter()
            .any(|step| step.contains("full retrieval only")));
        assert!(instructions
            .suggested_steps
            .iter()
            .any(|step| step.contains("broader term coverage")));
    }

    #[test]
    fn compression_retrieval_instructions_reject_invalid_refs() {
        let error = compression_retrieval_instructions("not-an-archive-ref").unwrap_err();
        assert!(error
            .to_string()
            .contains("Unsupported compression archive ref"));
    }

    #[test]
    fn active_compression_dry_run_from_file_returns_candidates_and_skips() {
        let file = write_active_compression_source_file(&active_compression_source_session());

        let report = active_compression_dry_run(&ActiveCompressionDryRunParams {
            source_provider_id: "claude".to_string(),
            target_provider_id: "codex".to_string(),
            session_id: None,
            file: Some(file.path().to_string_lossy().to_string()),
            policy: active_compression::ActiveCompressionPolicy {
                protect_recent_message_events: 1,
                min_candidate_bytes: 16,
                min_savings_ratio_percent: 20,
                mode: active_compression::ActiveCompressionMode::PlanOnly,
            },
        })
        .unwrap();

        assert!(report.dry_run);
        assert_eq!(report.source_provider_id, "claude");
        assert_eq!(report.target_provider_id, "codex");
        assert_eq!(report.candidates.len(), 1);
        assert_eq!(report.candidates[0].event_ids, vec!["old-user"]);
        assert!(report.candidates[0].estimated_bytes_saved > 0);
        assert!(matches!(
            report.candidates[0].reason,
            active_compression::CompressionSelectionReason::HistoricalContext
        ));
        assert!(matches!(
            report.candidates[0].risk,
            active_compression::CompressionRisk::Medium
        ));
        assert!(report.skipped.iter().any(|skipped| {
            skipped.event_id == "recent-user"
                && matches!(
                    skipped.reason,
                    active_compression::CompressionSkipReason::ProtectedRecentMessage
                )
        }));
    }

    #[test]
    fn active_compression_archive_is_expandable_and_retrievable() {
        let archive_dir = tempfile::tempdir().unwrap();
        let source = active_compression_source_session();
        let params = ActiveCompressionApplyCommandParams {
            source_provider_id: "claude".to_string(),
            target_provider_id: "codex".to_string(),
            session_id: Some("provider-session-1".to_string()),
            file: None,
            policy: active_compression::ActiveCompressionPolicy {
                protect_recent_message_events: 1,
                min_candidate_bytes: 16,
                min_savings_ratio_percent: 20,
                mode: active_compression::ActiveCompressionMode::Auto,
            },
            candidate_ids: vec!["candidate-0001".to_string()],
            output_prefix: None,
            format: "json".to_string(),
        };
        let applied =
            apply_active_compression_to_session(&params, &source, archive_dir.path()).unwrap();
        let compressed = applied.session;
        let archive_refs = applied.report.archive_refs;
        assert_eq!(archive_refs.len(), 1);
        assert!(compressed.events.iter().any(|event| {
            event.blocks.iter().any(|block| {
                matches!(
                    block,
                    EventBlock::Compressed {
                        archive_ref: Some(archive_ref),
                        ..
                    } if archive_ref == &archive_refs[0]
                )
            })
        }));
        assert!(!compressed.events.iter().any(|event| {
            event.blocks.iter().any(|block| {
                matches!(
                    block,
                    EventBlock::Text { text }
                        if text.contains("historical context historical context historical context")
                )
            })
        }));

        let (expanded, expand_report) = compression::expand_compressed_segments_in_dir(
            &compressed,
            "claude",
            "codex",
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(expand_report.expanded_segments, 1);
        assert_eq!(expand_report.restored_events, 1);
        assert!(expanded.events.iter().any(|event| event.id == "old-user"));

        let retrieved = retrieve_compression_archive_in_dir(
            &RetrieveCompressionArchiveParams {
                archive_ref: archive_refs[0].clone(),
                query: None,
                max_results: None,
            },
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(retrieved.source_provider_id, "claude");
        assert_eq!(retrieved.target_provider_id, "codex");
        assert_eq!(retrieved.source_event_ids, vec!["old-user"]);
        assert_eq!(retrieved.source_event_count, 1);
        assert_eq!(retrieved.returned_event_ids, vec!["old-user"]);
        assert_eq!(retrieved.returned_event_count, 1);
        assert_eq!(retrieved.omitted_event_count, 0);
        assert_eq!(
            retrieved.retrieval_mode,
            CompressionRetrievalMode::FullArchive
        );
        assert!(retrieved
            .recommended_next_action
            .contains("complete archived segment"));
        assert!(retrieved.events.iter().any(|event| event.id == "old-user"));

        let searched = retrieve_compression_archive_in_dir(
            &RetrieveCompressionArchiveParams {
                archive_ref: archive_refs[0].clone(),
                query: Some("historical context".to_string()),
                max_results: Some(5),
            },
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(searched.query.as_deref(), Some("historical context"));
        assert_eq!(searched.source_event_count, 1);
        assert_eq!(searched.returned_event_ids, vec!["old-user"]);
        assert_eq!(searched.returned_event_count, 1);
        assert_eq!(searched.omitted_event_count, 0);
        assert_eq!(
            searched.retrieval_mode,
            CompressionRetrievalMode::QueryMatches
        );
        assert!(searched
            .recommended_next_action
            .contains("query-filtered partial retrieval"));
        assert_eq!(searched.events[0].id, "old-user");
        assert_eq!(searched.matches.len(), 1);
        assert_eq!(searched.matches[0].event_id, "old-user");
        assert!(searched.matches[0]
            .snippets
            .iter()
            .any(|snippet| snippet.contains("historical context")));

        let no_match = retrieve_compression_archive_in_dir(
            &RetrieveCompressionArchiveParams {
                archive_ref: archive_refs[0].clone(),
                query: Some("not present".to_string()),
                max_results: Some(5),
            },
            archive_dir.path(),
        )
        .unwrap();
        assert_eq!(no_match.source_event_count, 1);
        assert!(no_match.returned_event_ids.is_empty());
        assert_eq!(no_match.returned_event_count, 0);
        assert_eq!(no_match.omitted_event_count, 1);
        assert_eq!(
            no_match.retrieval_mode,
            CompressionRetrievalMode::QueryNoMatches
        );
        assert!(no_match
            .recommended_next_action
            .contains("Try a broader query"));
        assert!(no_match.events.is_empty());
        assert!(no_match.matches.is_empty());
    }

    #[test]
    fn registers_active_compression_archives_with_operation_identity() {
        let archive_dir = tempfile::tempdir().unwrap();
        let session = active_compression_source_session();
        let params = ActiveCompressionApplyCommandParams {
            source_provider_id: "claude".to_string(),
            target_provider_id: "codex".to_string(),
            session_id: Some("provider-session-1".to_string()),
            file: None,
            policy: active_compression::ActiveCompressionPolicy {
                protect_recent_message_events: 1,
                min_candidate_bytes: 16,
                min_savings_ratio_percent: 20,
                mode: active_compression::ActiveCompressionMode::Auto,
            },
            candidate_ids: vec!["candidate-0001".to_string()],
            output_prefix: None,
            format: "json".to_string(),
        };
        let applied =
            apply_active_compression_to_session(&params, &session, archive_dir.path()).unwrap();
        assert_eq!(applied.report.archive_refs.len(), 1);

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, status, event_count, turn_count,
              projection_version, updated_at_ms)
             VALUES
             ('session-1', 'claude', 'provider-session-1', 'active', 0, 0, 1, 1)",
            [],
        )
        .unwrap();
        let activity_id = ActivityStore::new(&conn)
            .start(NewActivity {
                provider_id: Some("claude".to_string()),
                provider_session_id: Some("provider-session-1".to_string()),
                workspace_dir: None,
                operation_kind: ActivityOperationKind::Compress,
                actor: ActivityActor::System,
                summary: "Applying active session compression".to_string(),
                details: serde_json::json!({}),
            })
            .unwrap();

        let artifacts = register_active_compression_archive_artifacts(
            &mut conn,
            &activity_id,
            &params,
            &session,
            archive_dir.path(),
            &applied.report.archive_refs,
        )
        .unwrap();

        assert_eq!(artifacts.len(), 1);
        let artifact = &artifacts[0];
        assert_eq!(
            artifact.artifact_kind,
            ArtifactManifestKind::CompressionArchive
        );
        assert_eq!(artifact.operation_id.as_deref(), Some(activity_id.as_str()));
        assert_eq!(artifact.provider_id.as_deref(), Some("claude"));
        assert_eq!(
            artifact.provider_session_id.as_deref(),
            Some("provider-session-1")
        );
        assert_eq!(artifact.session_id.as_deref(), Some("session-1"));
        assert_eq!(artifact.mime_type.as_deref(), Some("application/gzip"));
        assert_eq!(artifact.format.as_deref(), Some("json.gz"));
        assert_eq!(
            artifact.metadata["role"],
            "active_compression_recovery_archive"
        );
        assert_eq!(
            artifact.metadata["archive_ref"],
            applied.report.archive_refs[0]
        );
        assert_eq!(artifact.metadata["canonical_id"], "dry-run-file");
        assert!(artifact.path.exists());
        assert!(artifact.content_hash.starts_with("sha256:"));
        assert!(artifact.byte_size > 0);
    }

    #[test]
    fn registration_failure_keeps_written_active_compression_archive() {
        let archive_dir = tempfile::tempdir().unwrap();
        let session = active_compression_source_session();
        let params = ActiveCompressionApplyCommandParams {
            source_provider_id: "claude".to_string(),
            target_provider_id: "codex".to_string(),
            session_id: Some("provider-session-1".to_string()),
            file: None,
            policy: active_compression::ActiveCompressionPolicy {
                protect_recent_message_events: 1,
                min_candidate_bytes: 16,
                min_savings_ratio_percent: 20,
                mode: active_compression::ActiveCompressionMode::Auto,
            },
            candidate_ids: vec!["candidate-0001".to_string()],
            output_prefix: None,
            format: "json".to_string(),
        };
        let applied =
            apply_active_compression_to_session(&params, &session, archive_dir.path()).unwrap();
        let archive_ref = &applied.report.archive_refs[0];
        let archive_path =
            compression::archive_path_from_ref_in_dir(archive_dir.path(), archive_ref).unwrap();

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, status, event_count, turn_count,
              projection_version, updated_at_ms)
             VALUES
             ('session-1', 'claude', 'provider-session-1', 'active', 0, 0, 1, 1)",
            [],
        )
        .unwrap();
        let activity_id = ActivityStore::new(&conn)
            .start(NewActivity {
                provider_id: Some("claude".to_string()),
                provider_session_id: Some("provider-session-1".to_string()),
                workspace_dir: None,
                operation_kind: ActivityOperationKind::Compress,
                actor: ActivityActor::System,
                summary: "Applying active session compression".to_string(),
                details: serde_json::json!({}),
            })
            .unwrap();
        ArtifactStore::new(&mut conn)
            .register_path(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::CompressionArchive,
                operation_id: Some(activity_id.clone()),
                provider_id: Some("claude".to_string()),
                provider_session_id: Some("provider-session-1".to_string()),
                session_id: None,
                projection_report_id: None,
                path: archive_path.clone(),
                mime_type: Some("application/gzip".to_string()),
                format: Some("json.gz".to_string()),
                metadata: serde_json::json!({"role": "conflicting_archive"}),
            })
            .unwrap();

        let error = register_active_compression_archive_artifacts(
            &mut conn,
            &activity_id,
            &params,
            &session,
            archive_dir.path(),
            &applied.report.archive_refs,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("already registered with conflicting context"));
        assert!(archive_path.exists());
        let artifact_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artifact_manifests WHERE operation_id = ?1",
                [&activity_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(artifact_count, 1);
    }

    #[test]
    fn local_state_activity_kind_only_uses_specific_hide_or_pin_operations() {
        assert_eq!(
            local_state_activity_kind(&session_state::SessionLocalStateUpdate {
                hidden: Some(true),
                ..Default::default()
            }),
            ActivityOperationKind::Hide
        );
        assert_eq!(
            local_state_activity_kind(&session_state::SessionLocalStateUpdate {
                workspace_override: Some(session_state::WorkspaceLocalStateUpdate {
                    workspace_dir: "/tmp/project".to_string(),
                    pinned: Some(Some(true)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ActivityOperationKind::Pin
        );
        assert_eq!(
            local_state_activity_kind(&session_state::SessionLocalStateUpdate {
                notes: Some(Some("note".to_string())),
                workspace_override: Some(session_state::WorkspaceLocalStateUpdate {
                    workspace_dir: "/tmp/project".to_string(),
                    hidden: Some(Some(true)),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ActivityOperationKind::LocalStateUpdate
        );
    }

    #[test]
    fn archive_query_search_ranks_phrase_before_repeated_scattered_terms() {
        let events = vec![
            archive_search_event(
                "earlier-single",
                "needle appears once in an earlier event",
                EventRole::User,
            ),
            archive_search_event(
                "phrase-match",
                "the exact needle detail phrase should outrank scattered terms",
                EventRole::Assistant,
            ),
            archive_search_event(
                "term-frequency",
                "needle appears repeatedly but detail is only connected loosely to needle",
                EventRole::User,
            ),
        ];

        let (events, matches) = search_archive_events(&events, "needle detail", 2);

        assert_eq!(
            events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["phrase-match", "term-frequency"]
        );
        assert_eq!(
            matches
                .iter()
                .map(|search_match| search_match.event_id.as_str())
                .collect::<Vec<_>>(),
            vec!["phrase-match", "term-frequency"]
        );
        assert!(matches[0].score > matches[1].score);
        assert!(matches[1].score > 0);
        assert!(matches[0]
            .snippets
            .iter()
            .any(|snippet| snippet.contains("needle detail")));
    }

    #[test]
    fn archive_query_search_ranks_term_coverage_before_single_term_repetition() {
        let events = vec![
            archive_search_event(
                "single-term-repeated",
                "needle needle needle needle needle",
                EventRole::User,
            ),
            archive_search_event(
                "covered-terms",
                "needle appears with the missing detail",
                EventRole::Assistant,
            ),
        ];

        let (events, matches) = search_archive_events(&events, "needle detail", 10);

        assert_eq!(
            events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["covered-terms", "single-term-repeated"]
        );
        assert!(matches[0].score > matches[1].score);
    }

    #[test]
    fn archive_query_search_keeps_source_order_for_equal_scores() {
        let events = vec![
            archive_search_event("first", "same needle evidence", EventRole::User),
            archive_search_event("second", "same needle evidence", EventRole::Assistant),
        ];

        let (events, matches) = search_archive_events(&events, "needle", 10);

        assert_eq!(
            events
                .iter()
                .map(|event| event.id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert_eq!(matches[0].event_index, 0);
        assert_eq!(matches[1].event_index, 1);
        assert_eq!(matches[0].score, matches[1].score);
    }

    fn archive_search_event(id: &str, text: &str, role: EventRole) -> SessionEvent {
        let now = Utc::now();
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Message,
            role,
            timestamp: now,
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: text.to_string(),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "claude".to_string(),
                    original_id: Some(id.to_string()),
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: BTreeMap::new(),
            },
        }
    }

    fn active_compression_source_session() -> CanonicalSession {
        let now = Utc::now();
        CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "dry-run-file".to_string(),
                source_title: Some("Dry Run File".to_string()),
            },
            provenance: SessionProvenance {
                imported_at: now,
                imported_by: None,
                primary_source: ProviderSessionRef {
                    provider_id: "claude".to_string(),
                    session_id: "dry-run-file".to_string(),
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
        }
    }

    fn write_active_compression_source_file(session: &CanonicalSession) -> tempfile::NamedTempFile {
        let mut file = Builder::new().suffix(".json").tempfile().unwrap();
        write!(file, "{}", serde_json::to_string(session).unwrap()).unwrap();
        file
    }

    #[test]
    fn activity_bucket_selection_follows_span_thresholds() {
        assert_eq!(
            choose_activity_bucket(chrono::TimeDelta::minutes(20)),
            (SessionActivityBucketUnit::Minute, 60)
        );
        assert_eq!(
            choose_activity_bucket(chrono::TimeDelta::hours(6)),
            (SessionActivityBucketUnit::Hour, 60 * 60)
        );
        assert_eq!(
            choose_activity_bucket(chrono::TimeDelta::days(3)),
            (SessionActivityBucketUnit::TwelveHour, 12 * 60 * 60)
        );

        let (_, mut bucket_seconds) = choose_activity_bucket(chrono::TimeDelta::days(100));
        assert!(activity_bucket_count(chrono::TimeDelta::days(100), &mut bucket_seconds) <= 96);
        assert_eq!(
            activity_bucket_unit(bucket_seconds),
            SessionActivityBucketUnit::Adaptive
        );
    }

    #[test]
    fn bootstrap_projects_hermes_native_sqlite_source() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, model TEXT, started_at REAL NOT NULL, ended_at REAL, message_count INTEGER, tool_call_count INTEGER, archived INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT, tool_call_id TEXT, tool_calls TEXT, tool_name TEXT, effect_disposition TEXT, timestamp REAL NOT NULL, reasoning TEXT, reasoning_content TEXT, reasoning_details TEXT, compacted INTEGER NOT NULL DEFAULT 0, active INTEGER NOT NULL DEFAULT 1, api_content TEXT);
             INSERT INTO sessions VALUES ('hermes-1','Hermes fixture','/tmp/hermes-project','model-x',1000,NULL,1,0,0);
             INSERT INTO messages (id,session_id,role,content,tool_call_id,tool_calls,tool_name,effect_disposition,timestamp,reasoning,reasoning_content,reasoning_details,compacted,active,api_content) VALUES (1,'hermes-1','user','hello',NULL,NULL,NULL,NULL,1000,NULL,NULL,NULL,0,1,NULL);"
        ).unwrap();
        drop(conn);

        let provider = providers::find_provider("hermes").unwrap();
        assert!(provider.capabilities().scan);
        assert!(provider.capabilities().import);

        let session = ProviderSessionSummary {
            session_id: "hermes-1".into(),
            title: Some("Hermes fixture".into()),
            project_dir: Some("/tmp/hermes-project".into()),
            last_active_at: Some(1_000_000),
            source_path: Some(format!("{}#session=hermes-1", db.display())),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "hermes", &session, &mut report);

        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'hermes' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Hermes fixture");
    }

    #[test]
    fn hermes_projection_refreshes_after_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT, cwd TEXT, model TEXT, started_at REAL NOT NULL, ended_at REAL, message_count INTEGER, tool_call_count INTEGER, archived INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT, tool_call_id TEXT, tool_calls TEXT, tool_name TEXT, effect_disposition TEXT, timestamp REAL NOT NULL, reasoning TEXT, reasoning_content TEXT, reasoning_details TEXT, compacted INTEGER NOT NULL DEFAULT 0, active INTEGER NOT NULL DEFAULT 1, api_content TEXT);
             INSERT INTO sessions VALUES ('hermes-1','Before','/tmp/hermes-project','model-x',1000,NULL,1,0,0);
             INSERT INTO messages (id,session_id,role,content,tool_call_id,tool_calls,tool_name,effect_disposition,timestamp,reasoning,reasoning_content,reasoning_details,compacted,active,api_content) VALUES (1,'hermes-1','user','before',NULL,NULL,NULL,NULL,1000,NULL,NULL,NULL,0,1,NULL);"
        ).unwrap();
        drop(conn);

        let session = ProviderSessionSummary {
            session_id: "hermes-1".into(),
            title: Some("Before".into()),
            project_dir: Some("/tmp/hermes-project".into()),
            last_active_at: Some(1_000_000),
            source_path: Some(format!("{}#session=hermes-1", db.display())),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "hermes", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);

        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE sessions SET title = 'After' WHERE id = 'hermes-1'",
            [],
        )
        .unwrap();
        conn.execute("UPDATE messages SET content = 'after' WHERE id = 1", [])
            .unwrap();
        drop(conn);

        let session = ProviderSessionSummary {
            title: Some("After".into()),
            ..session
        };
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "hermes", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'hermes' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "After");
    }

    #[test]
    fn bootstrap_projects_cline_task_source() {
        let dir = tempfile::tempdir().unwrap();
        let task = dir.path().join("task-cline-1");
        std::fs::create_dir_all(&task).unwrap();
        let history = task.join("api_conversation_history.json");
        std::fs::write(
            &history,
            r#"[{"role":"user","content":"hello"},{"role":"assistant","content":[{"type":"thinking","thinking":"reason"},{"type":"text","text":"done"}]}]"#,
        )
        .unwrap();

        let session = ProviderSessionSummary {
            session_id: "task-cline-1".into(),
            title: Some("hello".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(history.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "cline", &session, &mut report);

        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'cline' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "hello");
    }

    #[test]
    fn cline_projection_refreshes_after_task_history_change() {
        let dir = tempfile::tempdir().unwrap();
        let task = dir.path().join("task-cline-1");
        std::fs::create_dir_all(&task).unwrap();
        let history = task.join("api_conversation_history.json");
        std::fs::write(&history, r#"[{"role":"user","content":"before"}]"#).unwrap();

        let mut session = ProviderSessionSummary {
            session_id: "task-cline-1".into(),
            title: Some("before".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(history.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "cline", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);

        std::fs::write(&history, r#"[{"role":"user","content":"after"}]"#).unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "cline", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'cline' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "after");
    }

    #[test]
    fn bootstrap_projects_copilot_event_source() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("copilot-session-1");
        std::fs::create_dir_all(&session_dir).unwrap();
        let events = session_dir.join("events.jsonl");
        std::fs::write(
            &events,
            r#"{"type":"session.start","data":{"context":{"cwd":"/tmp/copilot"}}}
{"type":"user.message","data":{"content":"before"}}
"#,
        )
        .unwrap();
        let session = ProviderSessionSummary {
            session_id: "copilot-session-1".into(),
            title: Some("before".into()),
            project_dir: Some("/tmp/copilot".into()),
            last_active_at: None,
            source_path: Some(events.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "copilot", &session, &mut report);
        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'copilot' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "before");
    }

    #[test]
    fn copilot_projection_refreshes_after_event_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let session_dir = dir.path().join("copilot-session-1");
        std::fs::create_dir_all(&session_dir).unwrap();
        let events = session_dir.join("events.jsonl");
        std::fs::write(
            &events,
            r#"{"type":"session.start","data":{"context":{"cwd":"/tmp/copilot"}}}
{"type":"user.message","data":{"content":"before"}}
"#,
        )
        .unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "copilot-session-1".into(),
            title: Some("before".into()),
            project_dir: Some("/tmp/copilot".into()),
            last_active_at: None,
            source_path: Some(events.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "copilot", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(
            &events,
            r#"{"type":"session.start","data":{"context":{"cwd":"/tmp/copilot"}}}
{"type":"user.message","data":{"content":"after"}}
"#,
        )
        .unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "copilot", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'copilot' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "after");
    }

    #[test]
    fn bootstrap_projects_droid_session_source() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("encoded-cwd");
        std::fs::create_dir_all(&sessions).unwrap();
        let source = sessions.join("droid-session-1.jsonl");
        std::fs::write(
            &source,
            r#"{"role":"user","content":"before","cwd":"/tmp/droid"}
{"role":"assistant","content":"done"}
"#,
        )
        .unwrap();
        let session = ProviderSessionSummary {
            session_id: "droid-session-1".into(),
            title: Some("before".into()),
            project_dir: Some("/tmp/droid".into()),
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "droid", &session, &mut report);
        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'droid' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "before");
    }

    #[test]
    fn droid_projection_refreshes_after_session_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let sessions = dir.path().join("encoded-cwd");
        std::fs::create_dir_all(&sessions).unwrap();
        let source = sessions.join("droid-session-1.jsonl");
        std::fs::write(
            &source,
            r#"{"role":"user","content":"before"}
"#,
        )
        .unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "droid-session-1".into(),
            title: Some("before".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "droid", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(
            &source,
            r#"{"role":"user","content":"after"}
"#,
        )
        .unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "droid", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'droid' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "after");
    }

    #[test]
    fn bootstrap_projects_codebuddy_session_source() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("project");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("codebuddy-session-1.jsonl");
        std::fs::write(
            &source,
            r#"{"role":"user","content":"before","cwd":"/tmp/codebuddy"}
"#,
        )
        .unwrap();
        let session = ProviderSessionSummary {
            session_id: "codebuddy-session-1".into(),
            title: Some("before".into()),
            project_dir: Some("/tmp/codebuddy".into()),
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "codebuddy", &session, &mut report);
        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
    }

    #[test]
    fn codebuddy_projection_refreshes_after_session_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("project");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("codebuddy-session-1.jsonl");
        std::fs::write(
            &source,
            r#"{"role":"user","content":"before"}
"#,
        )
        .unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "codebuddy-session-1".into(),
            title: Some("before".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "codebuddy", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(
            &source,
            r#"{"role":"user","content":"after"}
"#,
        )
        .unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "codebuddy", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
    }

    #[test]
    fn bootstrap_projects_qoder_session_source() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("encoded-cwd");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("qoder-session-1.jsonl");
        std::fs::write(
            &source,
            r#"{"role":"user","content":"before","cwd":"/tmp/qoder"}
"#,
        )
        .unwrap();
        let session = ProviderSessionSummary {
            session_id: "qoder-session-1".into(),
            title: Some("before".into()),
            project_dir: Some("/tmp/qoder".into()),
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "qoder", &session, &mut report);
        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
    }

    #[test]
    fn qoder_projection_refreshes_after_session_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("encoded-cwd");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("qoder-session-1.jsonl");
        std::fs::write(
            &source,
            r#"{"role":"user","content":"before"}
"#,
        )
        .unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "qoder-session-1".into(),
            title: Some("before".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "qoder", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(
            &source,
            r#"{"role":"user","content":"after"}
"#,
        )
        .unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "qoder", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
    }

    #[test]
    fn bootstrap_projects_workbuddy_trace_source() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("traces");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("trace-workbuddy-1.json");
        std::fs::write(
            &source,
            r#"{"trace":{"traceId":"workbuddy-1","title":"before"},"spans":[]}"#,
        )
        .unwrap();
        let session = ProviderSessionSummary {
            session_id: "workbuddy-1".into(),
            title: Some("before".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "workbuddy", &session, &mut report);
        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
    }

    #[test]
    fn workbuddy_projection_refreshes_after_trace_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("traces");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("trace-workbuddy-1.json");
        std::fs::write(
            &source,
            r#"{"trace":{"traceId":"workbuddy-1","title":"before"},"spans":[]}"#,
        )
        .unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "workbuddy-1".into(),
            title: Some("before".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "workbuddy", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(
            &source,
            r#"{"trace":{"traceId":"workbuddy-1","title":"after"},"spans":[]}"#,
        )
        .unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "workbuddy", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
    }

    #[test]
    fn bootstrap_projects_pi_session_source() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("--tmp-pi--");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("20260720_pi-1.jsonl");
        std::fs::write(
            &source,
            r#"{"type":"session","id":"pi-1","cwd":"/tmp/pi"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"before"}]}}
"#,
        )
        .unwrap();
        let session = ProviderSessionSummary {
            session_id: "pi-1".into(),
            title: Some("before".into()),
            project_dir: Some("/tmp/pi".into()),
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "pi", &session, &mut report);
        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
    }

    #[test]
    fn pi_projection_refreshes_after_session_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let source_dir = dir.path().join("--tmp-pi--");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("20260720_pi-1.jsonl");
        std::fs::write(
            &source,
            r#"{"type":"session","id":"pi-1","cwd":"/tmp/pi"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"before"}]}}
"#,
        )
        .unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "pi-1".into(),
            title: Some("before".into()),
            project_dir: Some("/tmp/pi".into()),
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "pi", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(
            &source,
            r#"{"type":"session","id":"pi-1","cwd":"/tmp/pi"}
{"type":"message","message":{"role":"user","content":[{"type":"text","text":"after"}]}}
"#,
        )
        .unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "pi", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
    }

    #[test]
    fn bootstrap_projects_antigravity_session_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("session.json");
        std::fs::write(&source, r#"{"sessionId":"ag-1","directories":["/tmp/ag"],"messages":[{"type":"user","content":[{"text":"before"}]}]}"#).unwrap();
        let session = ProviderSessionSummary {
            session_id: "ag-1".into(),
            title: Some("before".into()),
            project_dir: Some("/tmp/ag".into()),
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "antigravity", &session, &mut report);
        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
    }

    #[test]
    fn antigravity_projection_refreshes_after_session_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("session.json");
        std::fs::write(
            &source,
            r#"{"sessionId":"ag-1","messages":[{"type":"user","content":[{"text":"before"}]}]}"#,
        )
        .unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "ag-1".into(),
            title: Some("before".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "antigravity", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(
            &source,
            r#"{"sessionId":"ag-1","messages":[{"type":"user","content":[{"text":"after"}]}]}"#,
        )
        .unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "antigravity", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
    }

    #[test]
    fn bootstrap_projects_windsurf_active_trajectory_source() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable(key,value) VALUES (?1,?2)",
            rusqlite::params![
                "windsurf.state.cachedActiveTrajectory:workspace-1",
                "CgZ3aW5kLTE="
            ],
        )
        .unwrap();
        drop(conn);
        let source = format!("{}#workspace=workspace-1", db.display());
        let session = ProviderSessionSummary {
            session_id: "wind-1".into(),
            title: None,
            project_dir: None,
            last_active_at: None,
            source_path: Some(source),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "windsurf", &session, &mut report);
        assert_eq!(report.projected_sessions, 1);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 0);
    }

    #[test]
    fn windsurf_projection_refreshes_after_active_trajectory_change() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.vscdb");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "CREATE TABLE ItemTable (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable(key,value) VALUES (?1,?2)",
            rusqlite::params![
                "windsurf.state.cachedActiveTrajectory:workspace-1",
                "CgZ3aW5kLTE="
            ],
        )
        .unwrap();
        drop(conn);
        let source = format!("{}#workspace=workspace-1", db.display());
        let session = ProviderSessionSummary {
            session_id: "wind-1".into(),
            title: None,
            project_dir: None,
            last_active_at: None,
            source_path: Some(source),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "windsurf", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE ItemTable SET value = ?1 WHERE key = ?2",
            rusqlite::params![
                "CgZ3aW5kLTEQYQ==",
                "windsurf.state.cachedActiveTrajectory:workspace-1"
            ],
        )
        .unwrap();
        drop(conn);
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "windsurf", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
    }

    #[test]
    fn windsurf_legacy_chat_projection_refreshes_after_source_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.pbtxt");
        std::fs::write(
            &path,
            r#"message: {source: CHAT_MESSAGE_SOURCE_USER conversation_id: "legacy-1" intent: {text: "before"}}
"#,
        )
        .unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "legacy-1".into(),
            title: Some("before".into()),
            project_dir: None,
            last_active_at: None,
            source_path: Some(format!("{}#conversation=legacy-1", path.display())),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap();
        local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "windsurf", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(
            &path,
            r#"message: {source: CHAT_MESSAGE_SOURCE_USER conversation_id: "legacy-1" intent: {text: "after"}}
"#,
        )
        .unwrap();
        session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "windsurf", &session, &mut second);
        assert_eq!(second.projected_sessions, 1);
        assert_eq!(second.unchanged_sessions, 0);
        let title: String = projection
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'windsurf' LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "after");
    }

    #[test]
    fn bootstrap_discovers_projects_skips_unchanged_and_refreshes_changed_indexes() {
        let opencode_dir = tempfile::tempdir().unwrap();
        let _guard = TestOpenCodeDirGuard::new(opencode_dir.path().to_path_buf());
        write_opencode_projection_sample(
            opencode_dir.path(),
            "ses_bootstrap",
            "Initial OpenCode title",
        );
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let first =
            bootstrap_session_projections_in_connection(&mut conn, Some("opencode")).unwrap();
        assert_eq!(first.scanned_providers, 1);
        assert_eq!(first.discovered_sessions, 1);
        assert_eq!(first.projected_sessions, 1);
        assert_eq!(first.unchanged_sessions, 0);
        assert!(first.failures.is_empty());

        let second =
            bootstrap_session_projections_in_connection(&mut conn, Some("opencode")).unwrap();
        assert_eq!(second.discovered_sessions, 1);
        assert_eq!(second.projected_sessions, 0);
        assert_eq!(second.unchanged_sessions, 1);
        let report_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM projection_reports", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(report_count, 0);

        write_opencode_projection_sample(
            opencode_dir.path(),
            "ses_bootstrap",
            "Changed OpenCode title",
        );
        let third =
            bootstrap_session_projections_in_connection(&mut conn, Some("opencode")).unwrap();
        assert_eq!(third.projected_sessions, 1);
        assert_eq!(third.unchanged_sessions, 0);
        let snapshot_title: String = conn
            .query_row(
                "SELECT title FROM session_snapshots WHERE provider_id = 'opencode'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(snapshot_title, "Changed OpenCode title");
        let report_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM projection_reports", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(report_count, 0);
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
    fn bootstrap_continues_after_missing_sources_without_parsing_body_content() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut valid = tempfile::NamedTempFile::new().unwrap();
        write_claude_projection_sample(&mut valid, "Valid title");
        let invalid = tempfile::tempdir().unwrap();
        let sessions = [
            ProviderSessionSummary {
                session_id: "valid-session".to_string(),
                title: Some("Valid title".to_string()),
                project_dir: Some("/tmp/project".to_string()),
                last_active_at: None,
                source_path: Some(valid.path().to_string_lossy().to_string()),
            },
            ProviderSessionSummary {
                session_id: "invalid-session".to_string(),
                title: None,
                project_dir: None,
                last_active_at: None,
                source_path: Some(invalid.path().to_string_lossy().to_string()),
            },
            ProviderSessionSummary {
                session_id: "missing-session".to_string(),
                title: None,
                project_dir: None,
                last_active_at: None,
                source_path: Some("/tmp/memorph-bootstrap-missing.jsonl".to_string()),
            },
        ];
        let mut report = SessionProjectionBootstrapReport::default();

        for session in &sessions {
            bootstrap_provider_session(&mut conn, "claude", session, &mut report);
        }

        assert_eq!(report.projected_sessions, 2);
        assert_eq!(report.failed_sessions, 0);
        assert_eq!(report.missing_sources, 1);
        assert_eq!(report.failures.len(), 1);
        assert!(report
            .failures
            .iter()
            .any(|failure| failure.session_id.as_deref() == Some("missing-session")));
    }

    #[test]
    fn bootstrap_reports_provider_scan_failure() {
        let opencode_dir = tempfile::tempdir().unwrap();
        let _guard = TestOpenCodeDirGuard::new(opencode_dir.path().to_path_buf());
        std::fs::write(opencode_dir.path().join("opencode.db"), b"not sqlite").unwrap();
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let report =
            bootstrap_session_projections_in_connection(&mut conn, Some("opencode")).unwrap();

        assert_eq!(report.scanned_providers, 0);
        assert_eq!(report.failed_providers, 1);
        assert_eq!(report.discovered_sessions, 0);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].provider_id, "opencode");
        assert_eq!(report.failures[0].session_id, None);
        assert!(report.failures[0]
            .reason
            .contains("failed to scan provider sessions"));
    }

    #[test]
    fn projected_provider_whitelist_matches_native_registry_contract() {
        for provider_id in PROJECTED_SESSION_PROVIDER_IDS {
            let provider = crate::providers::find_provider(provider_id)
                .unwrap_or_else(|| panic!("missing provider registry entry: {provider_id}"));
            let capabilities = provider.capabilities();
            assert!(
                capabilities.scan,
                "{provider_id} must scan before projection"
            );
            assert!(
                capabilities.import,
                "{provider_id} must import before projection"
            );
        }
    }

    #[test]
    fn projected_provider_whitelist_has_no_duplicates() {
        let unique: std::collections::HashSet<_> =
            PROJECTED_SESSION_PROVIDER_IDS.iter().copied().collect();
        assert_eq!(unique.len(), PROJECTED_SESSION_PROVIDER_IDS.len());
    }

    #[test]
    fn gemini_is_enabled_for_default_projection_bootstrap() {
        assert!(PROJECTED_SESSION_PROVIDER_IDS.contains(&"gemini"));
        assert!(provider_supports_session_projection("gemini"));
    }

    #[test]
    fn kimi_is_enabled_for_default_projection_bootstrap() {
        assert!(PROJECTED_SESSION_PROVIDER_IDS.contains(&"kimi"));
        assert!(provider_supports_session_projection("kimi"));
    }

    #[test]
    fn cursor_is_enabled_for_default_projection_bootstrap() {
        assert!(PROJECTED_SESSION_PROVIDER_IDS.contains(&"cursor"));
        assert!(provider_supports_session_projection("cursor"));
    }

    #[test]
    fn kiro_is_enabled_for_default_projection_bootstrap() {
        assert!(PROJECTED_SESSION_PROVIDER_IDS.contains(&"kiro"));
        assert!(provider_supports_session_projection("kiro"));
    }

    #[test]
    fn bootstrap_scans_deepseek_without_source() {
        let home = tempfile::tempdir().unwrap();
        let _home_guard = TestConfigHomeGuard::new(home.path());
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let report =
            bootstrap_session_projections_in_connection(&mut conn, Some("deepseek")).unwrap();

        assert_eq!(report.scanned_providers, 1);
        assert_eq!(report.unsupported_providers, 0);
        assert_eq!(report.discovered_sessions, 0);
        assert!(report.failures.is_empty());
    }

    #[test]
    fn non_default_session_list_reads_index_without_provider_source() {
        let home = tempfile::tempdir().unwrap();
        let _home_guard = TestConfigHomeGuard::new(home.path());
        let mut source = tempfile::NamedTempFile::new().unwrap();
        write_claude_projection_sample(&mut source, "Projected title");
        let mut conn = local_store::open_database().unwrap();

        let provider = providers::find_provider("claude").unwrap();
        let stored = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                "claude",
                &ProviderSessionSummary {
                    session_id: "session-projection-1".to_string(),
                    title: Some("Projected title".to_string()),
                    project_dir: Some("/tmp/project".to_string()),
                    last_active_at: None,
                    source_path: Some(source.path().to_string_lossy().to_string()),
                },
                provider.capabilities(),
                &required_provider_source_fingerprint(provider.as_ref(), source.path()),
            )
            .unwrap();
        crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .record_complete_counts(
                &stored.canonical_session_id,
                &stored.source_fingerprint,
                3,
                2,
                1,
            )
            .unwrap();
        drop(conn);
        drop(source);

        let groups = list_sessions(&SessionListParams {
            all: true,
            providers: vec!["claude".to_string()],
            cwd: None,
            include_message_counts: true,
            limit: None,
            offset: None,
            sort: SessionListSort::Title,
            hook_filter: SessionHookFilter::All,
        })
        .unwrap();

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].sessions.len(), 1);
        assert_eq!(groups[0].sessions[0].session_id, "session-projection-1");
        assert_eq!(
            groups[0].sessions[0].title.as_deref(),
            Some("Projected title")
        );
        assert_eq!(groups[0].sessions[0].message_count, Some(2));

        let found = query::find_sessions(&query::FindParams {
            dir: Some("/tmp/project".to_string()),
            session: Some("Projected title".to_string()),
            providers: vec!["claude".to_string()],
        })
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].sessions.len(), 1);
        assert_eq!(found[0].sessions[0].session_id, "session-projection-1");

        let error = compute_session_stats("claude", "session-projection-1").unwrap_err();
        assert!(format!("{error:#}").contains("source"));

        let local_state = update_session_local_state(
            "claude",
            "session-projection-1",
            &session_state::SessionLocalStateUpdate {
                pinned: Some(true),
                ..Default::default()
            },
            ActivityActor::System,
        )
        .unwrap();
        assert!(local_state.pinned);
    }

    #[test]
    fn activity_timelines_read_provider_source_and_fail_when_it_is_missing() {
        let mut source = tempfile::NamedTempFile::new().unwrap();
        write_claude_projection_sample(&mut source, "Projected activity");
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                "claude",
                &ProviderSessionSummary {
                    session_id: "session-projection-1".to_string(),
                    title: Some("Projected activity".to_string()),
                    project_dir: Some("/tmp/project".to_string()),
                    last_active_at: Some(1_767_225_602_000),
                    source_path: Some(source.path().to_string_lossy().to_string()),
                },
                crate::providers::claude::ClaudeProvider.capabilities(),
                &required_provider_source_fingerprint(
                    &crate::providers::claude::ClaudeProvider,
                    source.path(),
                ),
            )
            .unwrap();

        let session = compute_session_activity_timeline_in_connection(
            &conn,
            "claude",
            "session-projection-1",
        )
        .unwrap();
        assert_eq!(session.total_events, 3);
        assert_eq!(session.total_messages, 2);
        assert_eq!(session.total_activity, 6.25);

        let provider = compute_provider_activity_timeline_in_connection(
            &conn,
            "claude",
            Some("/tmp/project"),
            sessions::PROVIDER_ACTIVITY_DEFAULT_HOURS,
            false,
            true,
        )
        .unwrap();
        assert_eq!(provider.projected_sessions, 1);
        assert_eq!(provider.sessions_with_activity, 1);
        assert_eq!(provider.total_activity, 6.25);
        assert_eq!(
            provider
                .buckets
                .iter()
                .map(|bucket| bucket.message_count)
                .sum::<usize>(),
            2
        );

        let source_path = source.path().to_path_buf();
        drop(source);
        let error = compute_session_activity_timeline_in_connection(
            &conn,
            "claude",
            "session-projection-1",
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains(&source_path.to_string_lossy().to_string()));
    }

    #[test]
    fn provider_activity_applies_index_scope_source_range_and_deletion_rules() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let sources = tempfile::tempdir().unwrap();
        let now = Utc::now();

        project_claude_activity_sample(
            &mut conn,
            sources.path(),
            "activity-recent-a",
            "/tmp/project-a",
            now - chrono::TimeDelta::hours(2),
        );
        project_claude_activity_sample(
            &mut conn,
            sources.path(),
            "activity-recent-b",
            "/tmp/project-b",
            now - chrono::TimeDelta::hours(3),
        );
        project_claude_activity_sample(
            &mut conn,
            sources.path(),
            "activity-old-a",
            "/tmp/project-a",
            now - chrono::TimeDelta::days(40),
        );
        project_claude_activity_sample(
            &mut conn,
            sources.path(),
            "activity-future-a",
            "/tmp/project-a",
            now + chrono::TimeDelta::hours(2),
        );

        let workspace = compute_provider_activity_timeline_in_connection(
            &conn,
            "claude",
            Some("/tmp/project-a"),
            24 * 30,
            false,
            false,
        )
        .unwrap();
        assert_eq!(workspace.hours, 720);
        assert_eq!(workspace.bucket_seconds, 12 * 60 * 60);
        assert_eq!(workspace.buckets.len(), 60);
        assert_eq!(workspace.projected_sessions, 3);
        assert_eq!(workspace.sessions_with_activity, 1);
        assert_eq!(workspace.total_activity, 6.25);
        assert_eq!(activity_event_count(&workspace), 3);

        let all_workspaces = compute_provider_activity_timeline_in_connection(
            &conn,
            "claude",
            None,
            24 * 30,
            true,
            false,
        )
        .unwrap();
        assert_eq!(all_workspaces.projected_sessions, 4);
        assert_eq!(all_workspaces.sessions_with_activity, 2);
        assert_eq!(all_workspaces.total_activity, 12.5);
        assert_eq!(activity_event_count(&all_workspaces), 6);

        let all_time_workspace = compute_provider_activity_timeline_in_connection(
            &conn,
            "claude",
            Some("/tmp/project-a"),
            sessions::PROVIDER_ACTIVITY_DEFAULT_HOURS,
            false,
            true,
        )
        .unwrap();
        assert_eq!(all_time_workspace.projected_sessions, 3);
        assert_eq!(all_time_workspace.sessions_with_activity, 2);
        assert_eq!(all_time_workspace.total_activity, 12.5);
        assert_eq!(activity_event_count(&all_time_workspace), 6);
        assert!(all_time_workspace.range_start <= now - chrono::TimeDelta::days(40));

        conn.execute(
            "UPDATE sessions SET deleted_at_ms = ?1 WHERE provider_session_id = ?2",
            rusqlite::params![Utc::now().timestamp_millis(), "activity-recent-a"],
        )
        .unwrap();
        let after_delete = compute_provider_activity_timeline_in_connection(
            &conn,
            "claude",
            None,
            24 * 30,
            true,
            false,
        )
        .unwrap();
        assert_eq!(after_delete.projected_sessions, 3);
        assert_eq!(after_delete.sessions_with_activity, 1);
        assert_eq!(after_delete.total_activity, 6.25);
        assert_eq!(activity_event_count(&after_delete), 3);
    }

    #[test]
    fn reproject_stale_snapshot_sources_refreshes_claude_index() {
        let home = tempfile::tempdir().unwrap();
        let _home_guard = TestConfigHomeGuard::new(home.path());
        let project_dir = home.path().join(".claude/projects/project-1");
        std::fs::create_dir_all(&project_dir).unwrap();
        let source_path = project_dir.join("session-projection-1.jsonl");
        let mut file = std::fs::File::create(&source_path).unwrap();
        write_claude_projection_sample(&mut file, "Old title");

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let provider = providers::find_provider("claude").unwrap();
        let summary = provider
            .get_session_meta("session-projection-1")
            .unwrap()
            .unwrap();
        let stored = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                "claude",
                &summary,
                provider.capabilities(),
                &required_provider_source_fingerprint(
                    provider.as_ref(),
                    Path::new(summary.source_path.as_deref().unwrap()),
                ),
            )
            .unwrap();
        conn.execute(
            "UPDATE session_snapshots SET stale = 1 WHERE session_id = ?1",
            [stored.canonical_session_id.as_str()],
        )
        .unwrap();
        let mut file = std::fs::File::create(&source_path).unwrap();
        write_claude_projection_sample(&mut file, "New title");

        let report = reproject_stale_snapshot_sources(
            &mut conn,
            vec![StaleSnapshotSourceRow {
                canonical_session_id: stored.canonical_session_id.clone(),
                provider_id: "claude".to_string(),
                provider_session_id: Some("session-projection-1".to_string()),
                source_path: Some(source_path.to_string_lossy().to_string()),
            }],
        )
        .unwrap();

        assert_eq!(report.candidate_snapshots, 1);
        assert_eq!(report.reprojected_snapshots, 1);
        assert_eq!(report.missing_sources, 0);
        assert_eq!(report.failed_snapshots, 0);
        assert!(report.failures.is_empty());

        let snapshot: (String, i64) = conn
            .query_row(
                "SELECT title, stale FROM session_snapshots WHERE session_id = ?1",
                [stored.canonical_session_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(snapshot.0, "New title");
        assert_eq!(snapshot.1, 0);
    }

    #[test]
    fn reproject_stale_snapshot_sources_refreshes_codex_index() {
        let codex_dir = tempfile::tempdir().unwrap();
        let _guard = TestCodexDirGuard::new(codex_dir.path().to_path_buf());
        let sessions_dir = codex_dir.path().join("sessions/2026/05/21");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let source_path = sessions_dir.join("rollout-2026-05-21T10-00-00-codex-projection-1.jsonl");
        let mut file = std::fs::File::create(&source_path).unwrap();
        write_codex_projection_sample(&mut file, "Old Codex title");
        write_codex_index_sample(codex_dir.path(), "Old Codex title");

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let provider = providers::find_provider("codex").unwrap();
        let summary = provider
            .get_session_meta("codex-projection-1")
            .unwrap()
            .unwrap();
        let stored = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                "codex",
                &summary,
                provider.capabilities(),
                &required_provider_source_fingerprint(
                    provider.as_ref(),
                    Path::new(summary.source_path.as_deref().unwrap()),
                ),
            )
            .unwrap();
        conn.execute(
            "UPDATE session_snapshots SET stale = 1 WHERE session_id = ?1",
            [stored.canonical_session_id.as_str()],
        )
        .unwrap();
        let mut file = std::fs::File::create(&source_path).unwrap();
        write_codex_projection_sample(&mut file, "New Codex title");
        write_codex_index_sample(codex_dir.path(), "New Codex title");

        let report = reproject_stale_snapshot_sources(
            &mut conn,
            vec![StaleSnapshotSourceRow {
                canonical_session_id: stored.canonical_session_id.clone(),
                provider_id: "codex".to_string(),
                provider_session_id: Some("codex-projection-1".to_string()),
                source_path: Some(source_path.to_string_lossy().to_string()),
            }],
        )
        .unwrap();

        assert_eq!(report.candidate_snapshots, 1);
        assert_eq!(report.reprojected_snapshots, 1);
        assert_eq!(report.missing_sources, 0);
        assert_eq!(report.failed_snapshots, 0);
        assert!(report.failures.is_empty());

        let snapshot: (String, i64) = conn
            .query_row(
                "SELECT title, stale FROM session_snapshots WHERE session_id = ?1",
                [stored.canonical_session_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(snapshot.0, "New Codex title");
        assert_eq!(snapshot.1, 0);
    }

    #[test]
    fn reproject_stale_snapshot_sources_refreshes_opencode_index() {
        let opencode_dir = tempfile::tempdir().unwrap();
        let _guard = TestOpenCodeDirGuard::new(opencode_dir.path().to_path_buf());
        let source_path = write_opencode_projection_sample(
            opencode_dir.path(),
            "ses_projection",
            "Old OpenCode title",
        );
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let provider = providers::find_provider("opencode").unwrap();
        let summary = provider
            .get_session_meta("ses_projection")
            .unwrap()
            .unwrap();
        let stored = crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .write_session_summary(
                "opencode",
                &summary,
                provider.capabilities(),
                &required_provider_source_fingerprint(
                    provider.as_ref(),
                    Path::new(summary.source_path.as_deref().unwrap()),
                ),
            )
            .unwrap();
        conn.execute(
            "UPDATE session_snapshots SET stale = 1 WHERE session_id = ?1",
            [stored.canonical_session_id.as_str()],
        )
        .unwrap();
        write_opencode_projection_sample(
            opencode_dir.path(),
            "ses_projection",
            "New OpenCode title",
        );

        let report = reproject_stale_snapshot_sources(
            &mut conn,
            vec![StaleSnapshotSourceRow {
                canonical_session_id: stored.canonical_session_id.clone(),
                provider_id: "opencode".to_string(),
                provider_session_id: Some("ses_projection".to_string()),
                source_path: Some(source_path.to_string_lossy().to_string()),
            }],
        )
        .unwrap();

        assert_eq!(report.candidate_snapshots, 1);
        assert_eq!(report.reprojected_snapshots, 1);
        assert_eq!(report.missing_sources, 0);
        assert_eq!(report.failed_snapshots, 0);
        assert!(report.failures.is_empty());

        let snapshot: (String, i64) = conn
            .query_row(
                "SELECT title, stale FROM session_snapshots WHERE session_id = ?1",
                [stored.canonical_session_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(snapshot.0, "New OpenCode title");
        assert_eq!(snapshot.1, 0);
    }

    #[test]
    fn reproject_stale_snapshot_sources_reports_missing_source() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let report = reproject_stale_snapshot_sources(
            &mut conn,
            vec![StaleSnapshotSourceRow {
                canonical_session_id: "canonical-1".to_string(),
                provider_id: "claude".to_string(),
                provider_session_id: Some("native-1".to_string()),
                source_path: Some("/tmp/memorph-missing-source.jsonl".to_string()),
            }],
        )
        .unwrap();

        assert_eq!(report.candidate_snapshots, 1);
        assert_eq!(report.reprojected_snapshots, 0);
        assert_eq!(report.missing_sources, 1);
        assert_eq!(report.failed_snapshots, 0);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(report.failures[0].session_id, "native-1");
    }

    fn write_claude_projection_sample(file: &mut impl Write, title: &str) {
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "custom-title",
                "customTitle": title,
                "sessionId": "session-projection-1",
                "timestamp": "2026-01-01T00:00:00Z"
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "user",
                "uuid": "user-1",
                "sessionId": "session-projection-1",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:01Z",
                "message": {
                    "role": "user",
                    "content": "Build this"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "assistant",
                "uuid": "assistant-1",
                "parentUuid": "user-1",
                "sessionId": "session-projection-1",
                "cwd": "/tmp/project",
                "timestamp": "2026-01-01T00:00:02Z",
                "message": {
                    "role": "assistant",
                    "content": "Done"
                }
            })
        )
        .unwrap();
        file.flush().unwrap();
    }

    fn project_claude_activity_sample(
        conn: &mut rusqlite::Connection,
        source_dir: &Path,
        session_id: &str,
        workspace: &str,
        timestamp: chrono::DateTime<Utc>,
    ) {
        let source_path = source_dir.join(format!("{session_id}.jsonl"));
        let mut file = std::fs::File::create(&source_path).unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "custom-title",
                "customTitle": session_id,
                "sessionId": session_id,
                "timestamp": timestamp.to_rfc3339()
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "user",
                "uuid": format!("{session_id}-user"),
                "sessionId": session_id,
                "cwd": workspace,
                "timestamp": (timestamp + chrono::TimeDelta::seconds(1)).to_rfc3339(),
                "message": {
                    "role": "user",
                    "content": "Build this"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "type": "assistant",
                "uuid": format!("{session_id}-assistant"),
                "parentUuid": format!("{session_id}-user"),
                "sessionId": session_id,
                "cwd": workspace,
                "timestamp": (timestamp + chrono::TimeDelta::seconds(2)).to_rfc3339(),
                "message": {
                    "role": "assistant",
                    "content": "Done"
                }
            })
        )
        .unwrap();
        file.flush().unwrap();
        crate::storage::session_index_store::SessionIndexStore::new(conn)
            .write_session_summary(
                "claude",
                &ProviderSessionSummary {
                    session_id: session_id.to_string(),
                    title: Some(session_id.to_string()),
                    project_dir: Some(workspace.to_string()),
                    last_active_at: Some(
                        (timestamp + chrono::TimeDelta::seconds(2)).timestamp_millis(),
                    ),
                    source_path: Some(source_path.to_string_lossy().to_string()),
                },
                crate::providers::claude::ClaudeProvider.capabilities(),
                &required_provider_source_fingerprint(
                    &crate::providers::claude::ClaudeProvider,
                    &source_path,
                ),
            )
            .unwrap();
    }

    fn activity_event_count(timeline: &ProviderActivityTimeline) -> usize {
        timeline
            .buckets
            .iter()
            .map(|bucket| bucket.event_count)
            .sum()
    }

    fn write_codex_projection_sample(file: &mut impl Write, title: &str) {
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "codex-projection-1",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project",
                    "title": title,
                    "model": "gpt-5.3-codex"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            serde_json::json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        { "type": "input_text", "text": "Build this" }
                    ]
                }
            })
        )
        .unwrap();
        file.flush().unwrap();
    }

    fn write_codex_index_sample(codex_dir: &Path, title: &str) {
        std::fs::write(
            codex_dir.join("session_index.jsonl"),
            format!(
                "{{\"id\":\"codex-projection-1\",\"thread_name\":{},\"updated_at\":\"2026-05-21T10:00:00Z\"}}\n",
                serde_json::to_string(title).unwrap()
            ),
        )
        .unwrap();
    }

    struct TestConfigHomeGuard;

    impl TestConfigHomeGuard {
        fn new(path: &std::path::Path) -> Self {
            crate::config::set_test_home_dir(path.to_path_buf());
            Self
        }
    }

    impl Drop for TestConfigHomeGuard {
        fn drop(&mut self) {
            crate::config::reset_test_home_dir();
        }
    }

    struct TestCodexDirGuard;

    impl TestCodexDirGuard {
        fn new(path: std::path::PathBuf) -> Self {
            crate::providers::codex::set_test_codex_dir(Some(path));
            Self
        }
    }

    impl Drop for TestCodexDirGuard {
        fn drop(&mut self) {
            crate::providers::codex::set_test_codex_dir(None);
        }
    }

    struct TestOpenCodeDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl TestOpenCodeDirGuard {
        fn new(path: std::path::PathBuf) -> Self {
            let lock = crate::providers::opencode::lock_test_opencode_state();
            crate::providers::opencode::set_test_opencode_dir(Some(path));
            Self { _lock: lock }
        }
    }

    impl Drop for TestOpenCodeDirGuard {
        fn drop(&mut self) {
            crate::providers::opencode::set_test_opencode_dir(None);
        }
    }

    fn write_opencode_projection_sample(
        opencode_dir: &std::path::Path,
        session_id: &str,
        title: &str,
    ) -> std::path::PathBuf {
        let storage_dir = opencode_dir.join("storage");
        let session_dir = storage_dir.join("session").join("project-1");
        let message_dir = storage_dir.join("message").join(session_id);
        let part_dir = storage_dir.join("part").join("msg_projection");
        std::fs::create_dir_all(&session_dir).unwrap();
        std::fs::create_dir_all(&message_dir).unwrap();
        std::fs::create_dir_all(&part_dir).unwrap();

        let session_path = session_dir.join(format!("{session_id}.json"));
        std::fs::write(
            &session_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "id": session_id,
                "projectID": "project-1",
                "directory": "/tmp/project",
                "title": title,
                "time": {
                    "created": 1_790_000_000_000_i64,
                    "updated": 1_790_000_000_001_i64
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            message_dir.join("msg_projection.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "msg_projection",
                "sessionID": session_id,
                "role": "user",
                "time": {
                    "created": 1_790_000_000_000_i64,
                    "updated": 1_790_000_000_000_i64
                }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            part_dir.join("prt_projection.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "id": "prt_projection",
                "messageID": "msg_projection",
                "sessionID": session_id,
                "type": "text",
                "text": "Build this"
            }))
            .unwrap(),
        )
        .unwrap();
        session_path
    }
}
