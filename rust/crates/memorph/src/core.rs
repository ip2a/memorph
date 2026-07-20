use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
pub mod database_management;
pub mod manager;
pub mod session_management;

const MEMORPH_ARCHIVE_SCHEME: &str = "memorph-archive://";
const PROJECTED_SESSION_PROVIDER_IDS: &[&str] = &[
    // Tier 1: full L4 providers with independent modules and verified projections.
    "claude",
    "codex",
    "cursor",
    "deepseek",
    "gemini",
    "kimi",
    "kiro",
    "opencode",
    "qwen",
    // Tier 2: emerging providers onboarded via generic_json (minimal visibility).
    // Their sessions are discoverable in the UI, but capability mapping is unverified.
    "antigravity",
    "cline",
    "copilot",
    "windsurf",
    "codebuddy",
    "qoder",
    "trae",
    "droid",
    "stepfun",
    "workbuddy",
    "hermes",
    "pi",
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

pub fn resolve_providers(filter: &[String]) -> Vec<String> {
    if filter.is_empty() {
        providers::all_provider_ids()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        filter.to_vec()
    }
}

pub fn list_sessions(params: &SessionListParams) -> Result<Vec<SessionGroup>> {
    list_projected_session_snapshots(params)
}

pub fn refresh_projected_session_staleness(
    actor: ActivityActor,
) -> Result<SnapshotStaleScanReport> {
    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Scan,
        actor,
        summary: "Scanning projected session source fingerprints".to_string(),
        details: serde_json::json!({"scan_kind": "snapshot_staleness"}),
    })?;
    let result = (|| {
        let conn = local_store::open_database()?;
        crate::storage::snapshot_store::SnapshotStore::new(&conn)
            .refresh_session_snapshot_staleness(|provider_id, source_path| {
                let provider = providers::find_provider(provider_id)
                    .with_context(|| format!("Unknown provider: {provider_id}"))?;
                Ok(provider
                    .session_source_fingerprint(source_path)?
                    .map(|fingerprint| fingerprint.value))
            })
    })();
    match result {
        Ok(report) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Scanned projected session source fingerprints",
                    serde_json::json!({
                        "checked_sources": report.checked_sources,
                        "fresh_snapshots": report.fresh_snapshots,
                        "stale_snapshots": report.stale_snapshots,
                        "missing_sources": report.missing_sources,
                        "unknown_sources": report.unknown_sources,
                    }),
                ),
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to scan projected session source fingerprints",
                    serde_json::json!({"scan_kind": "snapshot_staleness"}),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionReprojectionReport {
    pub candidate_snapshots: usize,
    pub reprojected_snapshots: usize,
    pub missing_sources: usize,
    pub unsupported_providers: usize,
    pub failed_snapshots: usize,
    pub failures: Vec<SessionReprojectionFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionReprojectionFailure {
    pub provider_id: String,
    pub session_id: String,
    pub source_path: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionProjectionBootstrapReport {
    pub scanned_providers: usize,
    pub failed_providers: usize,
    pub discovered_sessions: usize,
    pub projected_sessions: usize,
    pub unchanged_sessions: usize,
    pub missing_sources: usize,
    pub unsupported_providers: usize,
    pub failed_sessions: usize,
    pub failures: Vec<SessionProjectionBootstrapFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionProjectionBootstrapFailure {
    pub provider_id: String,
    pub session_id: Option<String>,
    pub source_path: Option<String>,
    pub reason: String,
}

pub fn bootstrap_session_projections(
    provider_filter: Option<&str>,
    actor: ActivityActor,
) -> Result<SessionProjectionBootstrapReport> {
    let provider_filter = provider_filter.map(providers::canonical_provider_id);
    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: provider_filter.clone(),
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Scan,
        actor,
        summary: "Discovering and projecting provider sessions".to_string(),
        details: serde_json::json!({
            "scan_kind": "projection_bootstrap",
            "provider_filter": provider_filter,
        }),
    })?;
    let result = (|| {
        let mut conn = local_store::open_database()?;
        bootstrap_session_projections_in_connection(&mut conn, provider_filter.as_deref())
    })();
    match result {
        Ok(report) => {
            let has_failures = report.failed_providers > 0
                || report.failed_sessions > 0
                || report.missing_sources > 0
                || report.unsupported_providers > 0;
            let status = if !has_failures {
                ActivityStatus::Success
            } else if report.projected_sessions == 0 && report.unchanged_sessions == 0 {
                ActivityStatus::Failed
            } else {
                ActivityStatus::Partial
            };
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status,
                    provider_id: provider_filter.clone(),
                    provider_session_id: None,
                    workspace_dir: None,
                    summary: "Discovered and projected provider sessions".to_string(),
                    details: serde_json::to_value(&report)?,
                    error: (!report.failures.is_empty()).then(|| {
                        report
                            .failures
                            .iter()
                            .map(|failure| failure.reason.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    }),
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to discover and project provider sessions",
                    serde_json::json!({
                        "scan_kind": "projection_bootstrap",
                        "provider_filter": provider_filter,
                    }),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

fn bootstrap_session_projections_in_connection(
    conn: &mut rusqlite::Connection,
    provider_filter: Option<&str>,
) -> Result<SessionProjectionBootstrapReport> {
    let provider_ids = provider_filter
        .map(|provider_id| vec![providers::canonical_provider_id(provider_id)])
        .unwrap_or_else(|| {
            PROJECTED_SESSION_PROVIDER_IDS
                .iter()
                .map(|provider_id| (*provider_id).to_string())
                .collect()
        });
    let mut report = SessionProjectionBootstrapReport::default();

    for provider_id in provider_ids {
        if !provider_supports_session_projection(&provider_id) {
            report.unsupported_providers += 1;
            report.failures.push(SessionProjectionBootstrapFailure {
                provider_id: provider_id.clone(),
                session_id: None,
                source_path: None,
                reason: format!(
                    "provider does not support projected store bootstrap yet: {provider_id}"
                ),
            });
            continue;
        }
        let Some(provider) = providers::find_provider(&provider_id) else {
            report.unsupported_providers += 1;
            report.failures.push(SessionProjectionBootstrapFailure {
                provider_id: provider_id.clone(),
                session_id: None,
                source_path: None,
                reason: format!("provider is not registered: {provider_id}"),
            });
            continue;
        };
        let sessions = match provider.scan_sessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                report.failed_providers += 1;
                report.failures.push(SessionProjectionBootstrapFailure {
                    provider_id: provider_id.clone(),
                    session_id: None,
                    source_path: None,
                    reason: format!("failed to scan provider sessions: {error:#}"),
                });
                continue;
            }
        };
        report.scanned_providers += 1;
        report.discovered_sessions += sessions.len();

        for session in sessions {
            bootstrap_provider_session(conn, &provider_id, &session, &mut report);
        }
    }

    Ok(report)
}

fn bootstrap_provider_session(
    conn: &mut rusqlite::Connection,
    provider_id: &str,
    session: &ProviderSessionSummary,
    report: &mut SessionProjectionBootstrapReport,
) {
    let Some(source_path) = session
        .source_path
        .as_deref()
        .filter(|source_path| !source_path.is_empty())
    else {
        report.missing_sources += 1;
        report.failures.push(bootstrap_failure(
            provider_id,
            session,
            "provider session has no source path".to_string(),
        ));
        return;
    };
    let Some(provider) = providers::find_provider(provider_id) else {
        report.failed_sessions += 1;
        report.failures.push(bootstrap_failure(
            provider_id,
            session,
            format!("provider is not registered: {provider_id}"),
        ));
        return;
    };
    let fingerprint = match provider.session_source_fingerprint(source_path) {
        Ok(Some(fingerprint)) => fingerprint,
        Ok(None) => {
            report.missing_sources += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("session source not found: {source_path}"),
            ));
            return;
        }
        Err(error) => {
            report.failed_sessions += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("failed to fingerprint provider session source: {error:#}"),
            ));
            return;
        }
    };

    let freshness = crate::storage::snapshot_store::SnapshotStore::new(conn)
        .session_source_is_fresh(
            provider_id,
            &session.session_id,
            source_path,
            &fingerprint.value,
        );
    match freshness {
        Ok(true) => {
            report.unchanged_sessions += 1;
            return;
        }
        Ok(false) => {}
        Err(error) => {
            report.failed_sessions += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("failed to inspect projected source freshness: {error:#}"),
            ));
            return;
        }
    }

    match crate::storage::session_index_store::SessionIndexStore::new(conn).write_session_summary(
        provider_id,
        session,
        provider.capabilities(),
        &fingerprint,
    ) {
        Ok(_) => report.projected_sessions += 1,
        Err(error) => {
            report.failed_sessions += 1;
            report.failures.push(bootstrap_failure(
                provider_id,
                session,
                format!("failed to project provider session: {error:#}"),
            ));
        }
    }
}

fn bootstrap_failure(
    provider_id: &str,
    session: &ProviderSessionSummary,
    reason: String,
) -> SessionProjectionBootstrapFailure {
    SessionProjectionBootstrapFailure {
        provider_id: provider_id.to_string(),
        session_id: Some(session.session_id.clone()),
        source_path: session.source_path.clone(),
        reason,
    }
}

pub fn reproject_stale_sessions(
    provider_filter: Option<&str>,
    actor: ActivityActor,
) -> Result<SessionReprojectionReport> {
    let provider_filter = provider_filter.map(providers::canonical_provider_id);
    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: provider_filter.clone(),
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Scan,
        actor,
        summary: "Reprojecting stale session snapshots".to_string(),
        details: serde_json::json!({
            "scan_kind": "stale_reprojection",
            "provider_filter": provider_filter,
        }),
    })?;
    let result = (|| {
        let mut conn = local_store::open_database()?;
        let sources = crate::storage::snapshot_store::SnapshotStore::new(&conn)
            .list_stale_snapshot_sources(provider_filter.as_deref())?;
        reproject_stale_snapshot_sources(&mut conn, sources)
    })();
    match result {
        Ok(report) => {
            let status = if report.failed_snapshots == 0
                && report.missing_sources == 0
                && report.unsupported_providers == 0
            {
                ActivityStatus::Success
            } else if report.reprojected_snapshots == 0 {
                ActivityStatus::Failed
            } else {
                ActivityStatus::Partial
            };
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status,
                    provider_id: provider_filter.clone(),
                    provider_session_id: None,
                    workspace_dir: None,
                    summary: "Reprojected stale session snapshots".to_string(),
                    details: serde_json::to_value(&report)?,
                    error: (!report.failures.is_empty()).then(|| {
                        report
                            .failures
                            .iter()
                            .map(|failure| failure.reason.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    }),
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to reproject stale session snapshots",
                    serde_json::json!({
                        "scan_kind": "stale_reprojection",
                        "provider_filter": provider_filter,
                    }),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

fn reproject_stale_snapshot_sources(
    conn: &mut rusqlite::Connection,
    sources: Vec<StaleSnapshotSourceRow>,
) -> Result<SessionReprojectionReport> {
    let mut report = SessionReprojectionReport {
        candidate_snapshots: sources.len(),
        ..SessionReprojectionReport::default()
    };
    for source in sources {
        if !provider_supports_session_projection(&source.provider_id) {
            report.unsupported_providers += 1;
            report.failures.push(reprojection_failure(
                &source,
                format!(
                    "provider does not support projected store reprojection yet: {}",
                    source.provider_id
                ),
            ));
            continue;
        }
        let Some(provider_session_id) = source
            .provider_session_id
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            report.failed_snapshots += 1;
            report.failures.push(reprojection_failure(
                &source,
                "indexed session has no provider session id".to_string(),
            ));
            continue;
        };
        let Some(provider) = providers::find_provider(&source.provider_id) else {
            report.unsupported_providers += 1;
            report.failures.push(reprojection_failure(
                &source,
                format!("provider is not registered: {}", source.provider_id),
            ));
            continue;
        };
        let summary = match provider.get_session_meta(provider_session_id) {
            Ok(Some(summary)) => summary,
            Ok(None) => {
                report.missing_sources += 1;
                report.failures.push(reprojection_failure(
                    &source,
                    "provider no longer reports the indexed session".to_string(),
                ));
                continue;
            }
            Err(error) => {
                report.failed_snapshots += 1;
                report.failures.push(reprojection_failure(
                    &source,
                    format!("failed to refresh provider session summary: {error:#}"),
                ));
                continue;
            }
        };
        let Some(summary_source_path) = summary
            .source_path
            .as_deref()
            .filter(|value| !value.is_empty())
        else {
            report.missing_sources += 1;
            report.failures.push(reprojection_failure(
                &source,
                "provider session summary has no source locator".to_string(),
            ));
            continue;
        };
        let fingerprint = match provider.session_source_fingerprint(summary_source_path) {
            Ok(Some(fingerprint)) => fingerprint,
            Ok(None) => {
                report.missing_sources += 1;
                report.failures.push(reprojection_failure(
                    &source,
                    format!("session source not found: {summary_source_path}"),
                ));
                continue;
            }
            Err(error) => {
                report.failed_snapshots += 1;
                report.failures.push(reprojection_failure(
                    &source,
                    format!("failed to fingerprint provider session source: {error:#}"),
                ));
                continue;
            }
        };
        match crate::storage::session_index_store::SessionIndexStore::new(conn)
            .write_session_summary(
                &source.provider_id,
                &summary,
                provider.capabilities(),
                &fingerprint,
            ) {
            Ok(_) => report.reprojected_snapshots += 1,
            Err(error) => {
                report.failed_snapshots += 1;
                report
                    .failures
                    .push(reprojection_failure(&source, format!("{error:#}")));
            }
        }
    }
    Ok(report)
}

fn provider_supports_session_projection(provider_id: &str) -> bool {
    PROJECTED_SESSION_PROVIDER_IDS.contains(&provider_id)
}

fn reprojection_failure(
    source: &StaleSnapshotSourceRow,
    reason: String,
) -> SessionReprojectionFailure {
    SessionReprojectionFailure {
        provider_id: source.provider_id.clone(),
        session_id: source
            .provider_session_id
            .clone()
            .unwrap_or_else(|| source.canonical_session_id.clone()),
        source_path: source.source_path.clone(),
        reason,
    }
}

fn list_projected_session_snapshots(params: &SessionListParams) -> Result<Vec<SessionGroup>> {
    let provider_ids = resolve_providers(&params.providers);
    let workspace_scopes = if params.all {
        None
    } else {
        let scopes: Vec<(String, String)> = provider_ids
            .iter()
            .filter_map(|provider_id| {
                session_management::normalized_workspace_key(provider_id, params.cwd.as_deref())
                    .map(|workspace| (provider_id.clone(), workspace))
            })
            .collect();
        Some(scopes)
    };

    let conn = crate::storage::local_store::open_database()?;
    let snapshots = crate::storage::snapshot_store::SnapshotStore::new(&conn)
        .list_session_snapshots_filtered(
            Some(&provider_ids),
            workspace_scopes.as_deref(),
            params.include_message_counts,
        )?;
    let hook_runtime_snapshot = crate::hooks::server::runtime_sessions_snapshot();
    let providers_with_snapshots: Vec<String> = provider_ids
        .iter()
        .filter(|provider_id| {
            snapshots
                .iter()
                .any(|snapshot| snapshot.provider_id == **provider_id)
        })
        .cloned()
        .collect();
    let last_event_at_cache =
        crate::hooks::store::last_event_observed_at_ms_for_providers(&providers_with_snapshots)
            .unwrap_or_default();
    let hook_statuses = providers_with_snapshots
        .iter()
        .map(|provider_id| {
            let hook_status = crate::hooks::operations::status_with_cached_last_event_at(
                provider_id,
                &last_event_at_cache,
            )
            .unwrap_or(crate::hooks::model::HookInstallStatus {
                provider: provider_id.clone(),
                status: crate::hooks::model::HookHealthStatus::InstalledBrokenConfig,
                config_path: None,
                installed_version: None,
                current_version: None,
                message: Some(
                    "Failed to inspect hook status while building session list.".to_string(),
                ),
                last_event_at: None,
            });
            (provider_id.clone(), hook_status)
        })
        .collect();
    Ok(projected_snapshot_groups(
        snapshots,
        params,
        &hook_runtime_snapshot,
        &hook_statuses,
    ))
}

fn projected_snapshot_groups(
    snapshots: Vec<ProjectedSessionSnapshotRow>,
    params: &SessionListParams,
    hook_runtime_snapshot: &[crate::hooks::model::RuntimeSession],
    hook_statuses: &HashMap<String, crate::hooks::model::HookInstallStatus>,
) -> Vec<SessionGroup> {
    let provider_ids = resolve_providers(&params.providers);
    let requested_workspace = if params.all {
        None
    } else {
        params.cwd.as_deref()
    };
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(usize::MAX);

    provider_ids
        .into_iter()
        .filter_map(|provider_id| {
            let provider_name = providers::find_provider(&provider_id)
                .map(|provider| provider.name().to_string())
                .unwrap_or_else(|| provider_id.clone());
            let requested_workspace_key = requested_workspace.and_then(|workspace| {
                session_management::normalized_workspace_key(&provider_id, Some(workspace))
            });
            let hook_status = hook_statuses.get(&provider_id);
            let mut sessions: Vec<SessionItem> = snapshots
                .iter()
                .filter(|snapshot| snapshot.provider_id == provider_id)
                .filter(|snapshot| {
                    let Some(requested_workspace_key) = requested_workspace_key.as_deref() else {
                        return true;
                    };
                    let session_workspace_key = session_management::normalized_workspace_key(
                        &provider_id,
                        snapshot.workspace_dir.as_deref(),
                    );
                    session_workspace_key.as_deref() == Some(requested_workspace_key)
                })
                .map(|snapshot| {
                    let mut item = projected_snapshot_item(snapshot);
                    if let Some(hook_status) = hook_status {
                        let hook_augmentation =
                            crate::hooks::augmentation::augment_session_from_snapshot_with_status(
                                hook_runtime_snapshot,
                                hook_status.clone(),
                                &provider_id,
                                &item.session_id,
                                snapshot.workspace_dir.as_deref(),
                            );
                        item.hook_runtime_summary = hook_augmentation.runtime_summary;
                        item.hook_diagnosis = hook_augmentation.diagnosis;
                    }
                    item
                })
                .collect();

            sessions.retain(|item| session_matches_hook_filter(item, &params.hook_filter));
            sort_session_items(&mut sessions, &params.sort);
            let sessions: Vec<SessionItem> =
                sessions.into_iter().skip(offset).take(limit).collect();
            if sessions.is_empty() {
                None
            } else {
                Some(SessionGroup {
                    provider_id,
                    provider_name,
                    sessions,
                })
            }
        })
        .collect()
}

fn projected_snapshot_item(snapshot: &ProjectedSessionSnapshotRow) -> SessionItem {
    let provider_session_id = snapshot
        .provider_session_id
        .as_deref()
        .unwrap_or(&snapshot.canonical_session_id);
    SessionItem {
        session_id: provider_session_id.to_string(),
        title: snapshot.title.clone(),
        native_title: snapshot.title.clone(),
        display_title: snapshot.display_title.clone(),
        hidden: snapshot.hidden,
        pinned: snapshot.pinned,
        stale: snapshot.stale,
        preferred_targets: snapshot.preferred_targets.clone(),
        project_dir: snapshot
            .workspace_dir
            .as_deref()
            .map(utils::user_visible_path),
        last_active_at: snapshot.last_active_at_ms,
        source_path: snapshot
            .source_path
            .as_deref()
            .map(utils::user_visible_path),
        provider_id: snapshot.provider_id.clone(),
        message_count: snapshot.message_count,
        size_bytes: snapshot.size_bytes,
        hook_runtime_summary: None,
        hook_diagnosis: None,
    }
}

fn session_matches_hook_filter(item: &SessionItem, hook_filter: &SessionHookFilter) -> bool {
    use crate::hooks::augmentation::SessionHookDiagnosisKind;

    match hook_filter {
        SessionHookFilter::All => true,
        SessionHookFilter::Attention => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| {
                matches!(
                    diagnosis.kind,
                    SessionHookDiagnosisKind::HookNotInstalled
                        | SessionHookDiagnosisKind::HookNeedsAttention
                        | SessionHookDiagnosisKind::NoEventsYet
                        | SessionHookDiagnosisKind::NoActiveRuntime
                        | SessionHookDiagnosisKind::NoSessionMatch
                )
            })
            .unwrap_or(false),
        SessionHookFilter::Weak => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| diagnosis.kind == SessionHookDiagnosisKind::WeaklyLinked)
            .unwrap_or(false),
        SessionHookFilter::Runtime => item.hook_runtime_summary.is_some(),
        SessionHookFilter::NoHook => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| {
                matches!(
                    diagnosis.kind,
                    SessionHookDiagnosisKind::HookNotInstalled
                        | SessionHookDiagnosisKind::HookUnsupported
                )
            })
            .unwrap_or(false),
        SessionHookFilter::NoMatch => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| diagnosis.kind == SessionHookDiagnosisKind::NoSessionMatch)
            .unwrap_or(false),
        SessionHookFilter::Linked => item
            .hook_diagnosis
            .as_ref()
            .map(|diagnosis| diagnosis.kind == SessionHookDiagnosisKind::Linked)
            .unwrap_or(false),
    }
}

fn sort_session_items(items: &mut [SessionItem], sort: &SessionListSort) {
    items.sort_by(|left, right| compare_session_items(left, right, sort));
}

fn compare_session_items(
    left: &SessionItem,
    right: &SessionItem,
    sort: &SessionListSort,
) -> std::cmp::Ordering {
    let pin_order = right.pinned.cmp(&left.pinned);
    if pin_order != std::cmp::Ordering::Equal {
        return pin_order;
    }

    match sort {
        SessionListSort::Recent => compare_recent_then_title(left, right),
        SessionListSort::Title => compare_title_then_recent(left, right),
        SessionListSort::HookAttention => {
            let severity_order = hook_attention_priority(left).cmp(&hook_attention_priority(right));
            if severity_order != std::cmp::Ordering::Equal {
                return severity_order;
            }
            compare_recent_then_title(left, right)
        }
    }
}

fn compare_recent_then_title(left: &SessionItem, right: &SessionItem) -> std::cmp::Ordering {
    right
        .last_active_at
        .cmp(&left.last_active_at)
        .then_with(|| session_display_key(left).cmp(&session_display_key(right)))
}

fn compare_title_then_recent(left: &SessionItem, right: &SessionItem) -> std::cmp::Ordering {
    session_display_key(left)
        .cmp(&session_display_key(right))
        .then_with(|| right.last_active_at.cmp(&left.last_active_at))
}

fn session_display_key(item: &SessionItem) -> String {
    item.display_title
        .as_deref()
        .or(item.title.as_deref())
        .unwrap_or(&item.session_id)
        .to_ascii_lowercase()
}

fn hook_attention_priority(item: &SessionItem) -> usize {
    use crate::hooks::augmentation::SessionHookDiagnosisKind;

    match item
        .hook_diagnosis
        .as_ref()
        .map(|diagnosis| &diagnosis.kind)
    {
        Some(SessionHookDiagnosisKind::HookNeedsAttention) => 0,
        Some(SessionHookDiagnosisKind::NoSessionMatch) => 1,
        Some(SessionHookDiagnosisKind::HookNotInstalled) => 2,
        Some(SessionHookDiagnosisKind::NoActiveRuntime) => 3,
        Some(SessionHookDiagnosisKind::NoEventsYet) => 4,
        Some(SessionHookDiagnosisKind::WeaklyLinked) => 5,
        Some(SessionHookDiagnosisKind::Linked) => 6,
        Some(SessionHookDiagnosisKind::HookUnsupported) => 7,
        None => 8,
    }
}

#[cfg(test)]
mod session_list_hook_tests {
    use super::*;
    use crate::hooks::augmentation::{SessionHookDiagnosis, SessionHookDiagnosisKind};
    use crate::hooks::model::HookHealthStatus;

    fn session_item(
        session_id: &str,
        last_active_at: Option<i64>,
        diagnosis: Option<SessionHookDiagnosisKind>,
        has_runtime: bool,
    ) -> SessionItem {
        SessionItem {
            session_id: session_id.to_string(),
            title: Some(session_id.to_string()),
            native_title: Some(session_id.to_string()),
            display_title: None,
            hidden: false,
            pinned: false,
            stale: false,
            preferred_targets: Vec::new(),
            project_dir: Some("/tmp/project".to_string()),
            last_active_at,
            source_path: None,
            provider_id: "claude".to_string(),
            message_count: None,
            size_bytes: None,
            hook_runtime_summary: has_runtime.then(|| {
                crate::hooks::augmentation::HookRuntimeSummary {
                    linked_sessions: 1,
                    waiting_sessions: 0,
                    status: crate::hooks::model::RuntimeSessionStatus::Running,
                    current_tool_name: None,
                    has_pending_permission: false,
                    has_pending_question: false,
                    last_event_at: None,
                    matched_by: None,
                    confidence: None,
                }
            }),
            hook_diagnosis: diagnosis.map(|kind| SessionHookDiagnosis {
                kind,
                provider_status: HookHealthStatus::InstalledOk,
                linked_runtime_sessions: usize::from(has_runtime),
                provider_runtime_sessions: usize::from(has_runtime),
                matched_by: None,
                confidence: None,
                last_event_at: None,
                message: String::new(),
                actions: Vec::new(),
            }),
        }
    }

    #[test]
    fn session_matches_attention_hook_filter() {
        let attention = session_item(
            "session-attention",
            Some(20),
            Some(SessionHookDiagnosisKind::HookNeedsAttention),
            false,
        );
        let linked = session_item(
            "session-linked",
            Some(10),
            Some(SessionHookDiagnosisKind::Linked),
            true,
        );

        assert!(session_matches_hook_filter(
            &attention,
            &SessionHookFilter::Attention
        ));
        assert!(!session_matches_hook_filter(
            &linked,
            &SessionHookFilter::Attention
        ));
    }

    #[test]
    fn session_matches_runtime_hook_filter() {
        let runtime = session_item("session-runtime", Some(20), None, true);
        let offline = session_item("session-offline", Some(10), None, false);

        assert!(session_matches_hook_filter(
            &runtime,
            &SessionHookFilter::Runtime
        ));
        assert!(!session_matches_hook_filter(
            &offline,
            &SessionHookFilter::Runtime
        ));
    }

    #[test]
    fn hook_attention_sort_prioritizes_diagnosis_before_recency() {
        let mut items = vec![
            session_item(
                "linked-newer",
                Some(300),
                Some(SessionHookDiagnosisKind::Linked),
                true,
            ),
            session_item(
                "attention-older",
                Some(100),
                Some(SessionHookDiagnosisKind::HookNeedsAttention),
                false,
            ),
            session_item(
                "weak-middle",
                Some(200),
                Some(SessionHookDiagnosisKind::WeaklyLinked),
                true,
            ),
        ];

        sort_session_items(&mut items, &SessionListSort::HookAttention);

        assert_eq!(items[0].session_id, "attention-older");
        assert_eq!(items[1].session_id, "weak-middle");
        assert_eq!(items[2].session_id, "linked-newer");
    }

    #[test]
    fn projected_snapshot_groups_filter_workspace_and_apply_limit() {
        let params = SessionListParams {
            all: false,
            providers: vec!["claude".to_string()],
            cwd: Some("/tmp/project".to_string()),
            include_message_counts: true,
            limit: Some(1),
            offset: None,
            sort: SessionListSort::Recent,
            hook_filter: SessionHookFilter::All,
        };
        let groups = projected_snapshot_groups(
            vec![
                projected_row("canonical-new", "native-new", "/tmp/project", 30),
                projected_row("canonical-old", "native-old", "/tmp/project", 20),
                projected_row("canonical-other", "native-other", "/tmp/other", 40),
            ],
            &params,
            &[],
            &HashMap::new(),
        );

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].provider_id, "claude");
        assert_eq!(groups[0].sessions.len(), 1);
        assert_eq!(groups[0].sessions[0].session_id, "native-new");
        assert_eq!(groups[0].sessions[0].message_count, Some(3));
        assert_eq!(
            groups[0].sessions[0].project_dir.as_deref(),
            Some("/tmp/project")
        );
    }

    #[test]
    fn projected_snapshot_item_exposes_stale_snapshot_state() {
        let mut row = projected_row("canonical-1", "native-1", "/tmp/project", 30);
        row.stale = true;

        let item = projected_snapshot_item(&row);

        assert!(item.stale);
    }

    fn projected_row(
        canonical_session_id: &str,
        provider_session_id: &str,
        workspace_dir: &str,
        last_active_at_ms: i64,
    ) -> ProjectedSessionSnapshotRow {
        ProjectedSessionSnapshotRow {
            canonical_session_id: canonical_session_id.to_string(),
            provider_id: "claude".to_string(),
            provider_session_id: Some(provider_session_id.to_string()),
            title: Some(provider_session_id.to_string()),
            display_title: None,
            workspace_dir: Some(workspace_dir.to_string()),
            last_active_at_ms: Some(last_active_at_ms),
            source_path: Some(format!("/tmp/{provider_session_id}.jsonl")),
            message_count: Some(3),
            event_count: 5,
            turn_count: 2,
            size_bytes: Some(128),
            hidden: false,
            pinned: false,
            preferred_targets: Vec::new(),
            stale: false,
        }
    }
}

pub fn get_canonical_session(provider_id: &str, session_id: &str) -> Result<ImportedSession> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let capabilities = prov.capabilities();
    if !capabilities.scan || !capabilities.import {
        anyhow::bail!(
            "Provider does not support loading sessions: {}",
            provider_id
        );
    }

    let meta = prov
        .get_session_meta(session_id)?
        .with_context(|| format!("Session not found: {}", session_id))?;

    load_canonical_session_from_meta(prov.as_ref(), provider_id, meta)
}

pub fn get_session_detail_view(provider_id: &str, session_id: &str) -> Result<SessionDetailView> {
    get_session_detail_view_page(provider_id, session_id, 0, None)
}

pub fn get_session_detail_view_page(
    provider_id: &str,
    session_id: &str,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<SessionDetailView> {
    let mut conn = crate::storage::local_store::open_database()?;
    let identity = crate::storage::snapshot_store::SnapshotStore::new(&conn)
        .find_session_identity(provider_id, session_id)?
        .with_context(|| format!("Session is not indexed: {provider_id}/{session_id}"))?;
    let source_path = identity
        .source_path
        .as_deref()
        .filter(|value| !value.is_empty())
        .with_context(|| format!("Session has no source locator: {provider_id}/{session_id}"))?;
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    let source_fingerprint = provider
        .session_source_fingerprint(source_path)?
        .with_context(|| format!("Session source is missing: {source_path}"))?;
    if !provider.capabilities().import {
        anyhow::bail!("Provider does not support session detail reads: {provider_id}");
    }
    let provider_session_id = identity
        .provider_session_id
        .as_deref()
        .unwrap_or(session_id)
        .to_string();
    let mut page = provider.import_session_page(source_path, event_offset, event_limit)?;
    let meta = ProviderSessionSummary {
        session_id: provider_session_id.clone(),
        title: identity.title.clone(),
        project_dir: identity.workspace_dir.clone(),
        last_active_at: identity.last_active_at_ms,
        source_path: Some(source_path.to_string()),
    };
    enrich_imported_session_from_meta(&mut page.imported, provider_id, &meta);
    for turn in &mut page.turns {
        turn.session_id = identity.canonical_session_id.clone();
        turn.id = format!(
            "turn_{:x}",
            md5::compute(
                format!(
                    "{}\0{}",
                    identity.canonical_session_id,
                    turn.provider_turn_id
                        .as_deref()
                        .map(|value| format!("provider:{value}"))
                        .unwrap_or_else(|| turn.turn_order.to_string())
                )
                .as_bytes()
            )
        );
    }
    let local_state_store = session_state::load_state_store()?;
    let local_state = session_state::resolve_session_state(
        &local_state_store,
        provider_id,
        &provider_session_id,
        identity.workspace_dir.as_deref(),
    );
    let stale =
        identity.stale || identity.source_fingerprint.as_deref() != Some(&source_fingerprint.value);
    if let Some(turn_count) = page.turn_count {
        crate::storage::session_index_store::SessionIndexStore::new(&mut conn)
            .record_complete_counts(
                &identity.canonical_session_id,
                &source_fingerprint.value,
                page.event_count,
                page.message_count,
                turn_count,
            )?;
    }
    let display_title = local_state
        .display_title
        .clone()
        .or_else(|| identity.display_title.clone());
    let title = display_title.clone().or_else(|| identity.title.clone());
    let last_active_at = page.imported.session.context.last_active_at.or_else(|| {
        identity
            .last_active_at_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
    });
    let created_at = page.imported.session.context.created_at;
    let compressed_archive_refs = compression::compressed_archive_refs(&page.imported.session);
    let length_metrics = session_length_metrics(
        provider.session_size(&provider_session_id)?,
        &page.imported.session,
        page.event_count,
        page.message_count,
        page.turn_count.unwrap_or(page.turns.len()),
    )?;

    Ok(SessionDetailView {
        provider_id: provider_id.to_string(),
        provider_name: provider.name().to_string(),
        session_id: provider_session_id,
        canonical_id: identity.canonical_session_id.clone(),
        title,
        native_title: identity.title,
        display_title,
        workspace_dir: page
            .imported
            .session
            .context
            .workspace_dir
            .as_deref()
            .map(utils::user_visible_path),
        created_at,
        last_active_at,
        source_path: Some(utils::user_visible_path(source_path)),
        resume_command: provider.resume_command(
            identity
                .provider_session_id
                .as_deref()
                .unwrap_or(session_id),
        ),
        local_state: local_state.clone(),
        event_count: page.event_count,
        message_count: page.message_count,
        artifact_count: page.imported.session.artifacts.len(),
        length_metrics,
        stale,
        hook_runtime_summary: None,
        hook_diagnosis: None,
        hook_runtime_sessions: Vec::new(),
        projection_report: Some(source_mapping_report_view(
            provider_id,
            identity.source_id.as_deref(),
            &page.imported,
            page.event_count,
        )),
        turns: page.turns,
        events: page.imported.session.events,
        artifacts: page.imported.session.artifacts,
        compressed_archive_refs,
    })
}

fn session_length_metrics(
    provider_source_bytes: u64,
    session: &CanonicalSession,
    event_count: usize,
    message_count: usize,
    turn_count: usize,
) -> Result<SessionLengthMetrics> {
    let model_visible_bytes = serde_json::to_vec(&session.events)?.len() as u64;
    let archive_count = compression::compressed_archive_refs(session).len();
    Ok(SessionLengthMetrics {
        provider_source_bytes_measured: provider_source_bytes,
        model_visible_bytes_measured: model_visible_bytes,
        estimated_tokens: model_visible_bytes.div_ceil(4),
        event_count,
        message_count,
        turn_count,
        compressed_segment_count: archive_count,
        archive_count,
    })
}

fn source_mapping_report_view(
    provider_id: &str,
    source_id: Option<&str>,
    imported: &ImportedSession,
    event_count: usize,
) -> SessionProjectionReportView {
    let mut preserved_count = 0;
    let mut normalized_count = 0;
    let mut dropped_count = 0;
    for disposition in imported
        .session
        .events
        .iter()
        .map(|event| event.metadata.fidelity)
        .chain(imported.report.issues.iter().map(|issue| issue.disposition))
    {
        match disposition {
            crate::canonical::MappingDisposition::Preserved => preserved_count += 1,
            crate::canonical::MappingDisposition::Normalized
            | crate::canonical::MappingDisposition::Downgraded => normalized_count += 1,
            crate::canonical::MappingDisposition::Dropped
            | crate::canonical::MappingDisposition::Unsupported => dropped_count += 1,
        }
    }
    SessionProjectionReportView {
        id: format!(
            "source-read:{provider_id}:{}",
            imported.session.identity.canonical_id
        ),
        provider_id: provider_id.to_string(),
        source_id: source_id.map(str::to_string),
        operation_kind: crate::session_projection::ProjectionOperationKind::Import,
        projection_version: crate::session_projection::SESSION_PROJECTION_VERSION,
        status: if dropped_count > 0 {
            crate::session_projection::ProjectionStatus::CompletedWithLoss
        } else {
            crate::session_projection::ProjectionStatus::Succeeded
        },
        created_at: chrono::Utc::now(),
        created_at_ms: chrono::Utc::now().timestamp_millis(),
        summary: SessionProjectionReportSummaryView {
            canonical_event_count: Some(event_count),
            mapping_direction: Some(imported.report.direction),
            mapping_overall: Some(imported.report.overall),
            preserved_count,
            normalized_count,
            dropped_count,
        },
        item_count: imported.report.issues.len(),
        items: imported
            .report
            .issues
            .iter()
            .enumerate()
            .map(|(index, issue)| SessionProjectionReportItemView {
                item_order: index as i64,
                fidelity: match issue.disposition {
                    crate::canonical::MappingDisposition::Preserved => {
                        crate::session_projection::ProjectionFidelity::Preserved
                    }
                    crate::canonical::MappingDisposition::Normalized
                    | crate::canonical::MappingDisposition::Downgraded => {
                        crate::session_projection::ProjectionFidelity::Normalized
                    }
                    crate::canonical::MappingDisposition::Dropped
                    | crate::canonical::MappingDisposition::Unsupported => {
                        crate::session_projection::ProjectionFidelity::Dropped
                    }
                },
                scope: crate::session_projection::ProjectionItemScope::ProviderPayload,
                field_path: issue.path.clone(),
                reason: Some(issue.message.clone()),
                details: issue.raw.clone(),
            })
            .collect(),
    }
}

pub fn get_resolved_local_session_state(
    provider_id: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> session_state::ResolvedLocalSessionState {
    let session_states = session_state::load_state_store().unwrap_or_default();
    let workspace_dir = session_management::normalized_workspace_key(provider_id, workspace_dir);
    session_state::resolve_session_state(
        &session_states,
        provider_id,
        session_id,
        workspace_dir.as_deref(),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStats {
    pub event_id: String,
    pub char_count: usize,
    pub byte_size: usize,
    pub visible_char_count: usize,
    pub visible_byte_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    pub provider_id: String,
    pub session_id: String,
    pub events: Vec<EventStats>,
    pub total_char_count: usize,
    pub total_byte_size: usize,
    pub total_visible_char_count: usize,
    pub total_visible_byte_size: usize,
}

pub fn compute_session_stats(provider_id: &str, session_id: &str) -> Result<SessionStats> {
    let detail = get_session_detail_view(provider_id, session_id)?;
    let mut events = Vec::with_capacity(detail.events.len());
    let mut total_char_count = 0usize;
    let mut total_byte_size = 0usize;
    let mut total_visible_char_count = 0usize;
    let mut total_visible_byte_size = 0usize;

    for event in &detail.events {
        let full_text = provider::canonical_event_text(event);
        let visible_text = provider::canonical_event_visible_text(event);
        let char_count = full_text.chars().count();
        let byte_size = full_text.len();
        let visible_char_count = visible_text.chars().count();
        let visible_byte_size = visible_text.len();

        total_char_count = total_char_count.saturating_add(char_count);
        total_byte_size = total_byte_size.saturating_add(byte_size);
        total_visible_char_count = total_visible_char_count.saturating_add(visible_char_count);
        total_visible_byte_size = total_visible_byte_size.saturating_add(visible_byte_size);

        events.push(EventStats {
            event_id: event.id.clone(),
            char_count,
            byte_size,
            visible_char_count,
            visible_byte_size,
        });
    }

    Ok(SessionStats {
        provider_id: provider_id.to_string(),
        session_id: session_id.to_string(),
        events,
        total_char_count,
        total_byte_size,
        total_visible_char_count,
        total_visible_byte_size,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionActivityBucketUnit {
    Minute,
    Hour,
    TwelveHour,
    Adaptive,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionActivityBucket {
    pub start: chrono::DateTime<chrono::Utc>,
    pub end: chrono::DateTime<chrono::Utc>,
    pub event_count: usize,
    pub message_count: usize,
    pub activity_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionActivityTimeline {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    pub bucket_unit: SessionActivityBucketUnit,
    pub bucket_seconds: i64,
    pub buckets: Vec<SessionActivityBucket>,
    pub total_events: usize,
    pub total_messages: usize,
    pub total_activity: f64,
}

pub fn compute_session_activity_timeline(
    provider_id: &str,
    session_id: &str,
) -> Result<SessionActivityTimeline> {
    let conn = local_store::open_database()?;
    compute_session_activity_timeline_in_connection(&conn, provider_id, session_id)
}

fn compute_session_activity_timeline_in_connection(
    conn: &rusqlite::Connection,
    provider_id: &str,
    session_id: &str,
) -> Result<SessionActivityTimeline> {
    use chrono::TimeDelta;

    let identity = crate::storage::snapshot_store::SnapshotStore::new(conn)
        .find_session_identity(provider_id, session_id)?
        .with_context(|| format!("Session is not indexed: {provider_id}/{session_id}"))?;
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    let activity = import_session_activity(provider_id, provider.as_ref(), &identity)?;
    let event_timestamps = activity
        .events
        .iter()
        .map(|event| event.timestamp)
        .collect::<Vec<_>>();
    let first_event_at = event_timestamps.iter().copied().min();
    let last_event_at = event_timestamps.iter().copied().max();
    let created_at = match (activity.created_at, first_event_at) {
        (Some(source), Some(event)) => Some(source.min(event)),
        (source, event) => source.or(event),
    };
    let last_active_at = match (activity.last_active_at, last_event_at) {
        (Some(source), Some(event)) => Some(source.max(event)),
        (source, event) => source.or(event),
    };

    let range_start = created_at
        .or_else(|| event_timestamps.first().copied())
        .unwrap_or_else(chrono::Utc::now);
    let range_end = last_active_at
        .or_else(|| event_timestamps.last().copied())
        .unwrap_or(range_start);
    let range_end = if range_end < range_start {
        range_start
    } else {
        range_end
    };

    let span = range_end.signed_duration_since(range_start);
    let (_, mut bucket_seconds) = choose_activity_bucket(span);
    let bucket_count = activity_bucket_count(span, &mut bucket_seconds);
    let bucket_unit = activity_bucket_unit(bucket_seconds);
    let mut buckets = Vec::with_capacity(bucket_count);
    for index in 0..bucket_count {
        let start = range_start + TimeDelta::seconds(index as i64 * bucket_seconds);
        let end = if index + 1 == bucket_count {
            range_end
        } else {
            range_start + TimeDelta::seconds((index as i64 + 1) * bucket_seconds)
        };
        buckets.push(SessionActivityBucket {
            start,
            end,
            event_count: 0,
            message_count: 0,
            activity_score: 0.0,
        });
    }

    let mut total_activity = 0.0;
    let mut total_messages = 0usize;
    for event in &activity.events {
        let weight = event_activity_weight(&event.kind, event.visible_message);
        total_activity += weight;
        if event.visible_message {
            total_messages += 1;
        }
        if let Some(bucket) = bucket_for_timestamp(
            event.timestamp,
            range_start,
            range_end,
            bucket_seconds,
            &mut buckets,
        ) {
            bucket.event_count += 1;
            bucket.activity_score += weight;
            if event.visible_message {
                bucket.message_count += 1;
            }
        }
    }

    Ok(SessionActivityTimeline {
        provider_id: provider_id.to_string(),
        session_id: session_id.to_string(),
        created_at,
        last_active_at,
        bucket_unit,
        bucket_seconds,
        buckets,
        total_events: activity.events.len(),
        total_messages,
        total_activity,
    })
}

#[derive(Debug)]
struct SourceActivityEvent {
    kind: SessionEventKind,
    timestamp: chrono::DateTime<chrono::Utc>,
    visible_message: bool,
}

#[derive(Debug)]
struct SourceSessionActivity {
    canonical_session_id: String,
    created_at: Option<chrono::DateTime<chrono::Utc>>,
    last_active_at: Option<chrono::DateTime<chrono::Utc>>,
    events: Vec<SourceActivityEvent>,
}

fn import_session_activity(
    provider_id: &str,
    provider: &dyn Provider,
    identity: &ProjectedSessionIdentityRow,
) -> Result<SourceSessionActivity> {
    let source_path = identity
        .source_path
        .as_deref()
        .filter(|value| !value.is_empty())
        .with_context(|| {
            format!(
                "Session has no source locator: {provider_id}/{}",
                identity.canonical_session_id
            )
        })?;
    if !provider.capabilities().import {
        anyhow::bail!("Provider does not support session activity reads: {provider_id}");
    }
    let imported = provider.import_session(source_path).with_context(|| {
        format!(
            "Failed to read session activity from provider source: {provider_id}/{}",
            identity
                .provider_session_id
                .as_deref()
                .unwrap_or(&identity.canonical_session_id)
        )
    })?;
    let events = imported
        .session
        .events
        .iter()
        .map(|event| SourceActivityEvent {
            kind: event.kind.clone(),
            timestamp: event.timestamp,
            visible_message: provider::canonical_event_is_visible_message(event),
        })
        .collect();
    let last_active_at = imported.session.context.last_active_at.or_else(|| {
        identity
            .last_active_at_ms
            .and_then(chrono::DateTime::from_timestamp_millis)
    });

    Ok(SourceSessionActivity {
        canonical_session_id: identity.canonical_session_id.clone(),
        created_at: imported.session.context.created_at,
        last_active_at,
        events,
    })
}

pub const PROVIDER_ACTIVITY_DEFAULT_HOURS: i64 = 72;
const PROVIDER_ACTIVITY_MAX_HOURS: i64 = 24 * 30;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderActivityTimeline {
    pub provider_id: String,
    pub hours: i64,
    pub bucket_seconds: i64,
    pub range_start: chrono::DateTime<chrono::Utc>,
    pub range_end: chrono::DateTime<chrono::Utc>,
    pub buckets: Vec<SessionActivityBucket>,
    pub total_activity: f64,
    pub projected_sessions: usize,
    pub sessions_with_activity: usize,
}

pub fn compute_provider_activity_timeline(
    provider_id: &str,
    workspace: Option<&str>,
    hours: i64,
    all_workspaces: bool,
    all_time: bool,
) -> Result<ProviderActivityTimeline> {
    let conn = local_store::open_database()?;
    compute_provider_activity_timeline_in_connection(
        &conn,
        provider_id,
        workspace,
        hours,
        all_workspaces,
        all_time,
    )
}

fn compute_provider_activity_timeline_in_connection(
    conn: &rusqlite::Connection,
    provider_id: &str,
    workspace: Option<&str>,
    hours: i64,
    all_workspaces: bool,
    all_time: bool,
) -> Result<ProviderActivityTimeline> {
    use chrono::TimeDelta;

    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    let hours = hours.clamp(1, PROVIDER_ACTIVITY_MAX_HOURS);
    let range_end = chrono::Utc::now();
    let requested_range_start = (!all_time).then(|| range_end - TimeDelta::hours(hours));
    let sessions = crate::storage::snapshot_store::SnapshotStore::new(conn)
        .list_provider_session_identities(provider_id)?
        .into_iter()
        .filter(|session| {
            if all_workspaces {
                return true;
            }
            prov.normalized_workspace_key(workspace).as_deref()
                == prov
                    .normalized_workspace_key(session.workspace_dir.as_deref())
                    .as_deref()
        })
        .collect::<Vec<_>>();
    let projected_sessions = sessions.len();
    let mut activities = Vec::with_capacity(sessions.len());
    for session in &sessions {
        activities.push(import_session_activity(
            provider_id,
            prov.as_ref(),
            session,
        )?);
    }
    let events = activities
        .iter()
        .flat_map(|activity| {
            activity.events.iter().filter_map(|event| {
                if requested_range_start.is_none_or(|start| event.timestamp >= start)
                    && event.timestamp <= range_end
                {
                    Some((&activity.canonical_session_id, event))
                } else {
                    None
                }
            })
        })
        .collect::<Vec<_>>();
    let range_start = requested_range_start.unwrap_or_else(|| {
        events
            .iter()
            .map(|(_, event)| event.timestamp)
            .min()
            .unwrap_or(range_end)
    });
    let span = range_end.signed_duration_since(range_start);
    let mut bucket_seconds = if all_time {
        choose_activity_bucket(span).1
    } else if hours <= 7 * 24 {
        60 * 60
    } else {
        12 * 60 * 60
    };
    let bucket_count = if all_time {
        activity_bucket_count(span, &mut bucket_seconds)
    } else {
        ((span.num_seconds().max(0) + bucket_seconds - 1) / bucket_seconds).max(1) as usize
    };

    let mut buckets = Vec::with_capacity(bucket_count);
    for index in 0..bucket_count {
        let start = range_start + TimeDelta::seconds(index as i64 * bucket_seconds);
        let end = if index + 1 == bucket_count {
            range_end
        } else {
            range_start + TimeDelta::seconds((index as i64 + 1) * bucket_seconds)
        };
        buckets.push(SessionActivityBucket {
            start,
            end,
            event_count: 0,
            message_count: 0,
            activity_score: 0.0,
        });
    }
    let mut sessions_with_events = HashSet::new();
    let mut total_activity = 0.0f64;
    for (canonical_session_id, event) in &events {
        let timestamp = event.timestamp;
        sessions_with_events.insert(canonical_session_id.as_str());
        let weight = event_activity_weight(&event.kind, event.visible_message);
        total_activity += weight;
        if let Some(bucket) = bucket_for_timestamp(
            timestamp,
            range_start,
            range_end,
            bucket_seconds,
            &mut buckets,
        ) {
            bucket.event_count += 1;
            bucket.activity_score += weight;
            if event.visible_message {
                bucket.message_count += 1;
            }
        }
    }
    let actual_hours = ((span.num_seconds().max(0) + 3599) / 3600).max(1);

    Ok(ProviderActivityTimeline {
        provider_id: provider_id.to_string(),
        hours: if all_time { actual_hours } else { hours },
        bucket_seconds,
        range_start,
        range_end,
        buckets,
        total_activity,
        projected_sessions,
        sessions_with_activity: sessions_with_events.len(),
    })
}

fn event_activity_weight(kind: &SessionEventKind, visible_message: bool) -> f64 {
    match kind {
        SessionEventKind::Lifecycle => 0.25,
        SessionEventKind::Message if visible_message => 3.0,
        SessionEventKind::Message => 1.5,
        SessionEventKind::ToolCall | SessionEventKind::ToolResult => 2.0,
        SessionEventKind::Command | SessionEventKind::CommandResult => 1.75,
        SessionEventKind::Patch | SessionEventKind::Artifact => 1.25,
        SessionEventKind::Unknown => 0.5,
    }
}

fn choose_activity_bucket(span: chrono::TimeDelta) -> (SessionActivityBucketUnit, i64) {
    if span < chrono::TimeDelta::hours(1) {
        (SessionActivityBucketUnit::Minute, 60)
    } else if span < chrono::TimeDelta::days(1) {
        (SessionActivityBucketUnit::Hour, 60 * 60)
    } else {
        (SessionActivityBucketUnit::TwelveHour, 12 * 60 * 60)
    }
}

fn activity_bucket_unit(bucket_seconds: i64) -> SessionActivityBucketUnit {
    match bucket_seconds {
        60 => SessionActivityBucketUnit::Minute,
        3_600 => SessionActivityBucketUnit::Hour,
        43_200 => SessionActivityBucketUnit::TwelveHour,
        _ => SessionActivityBucketUnit::Adaptive,
    }
}

fn activity_bucket_count(span: chrono::TimeDelta, bucket_seconds: &mut i64) -> usize {
    const MAX_BUCKETS: i64 = 96;
    let span_seconds = span.num_seconds().max(0);
    if span_seconds == 0 {
        return 1;
    }
    let mut count = (span_seconds + *bucket_seconds - 1) / *bucket_seconds;
    while count > MAX_BUCKETS {
        *bucket_seconds *= 2;
        count = (span_seconds + *bucket_seconds - 1) / *bucket_seconds;
    }
    count.max(1) as usize
}

fn bucket_for_timestamp(
    timestamp: chrono::DateTime<chrono::Utc>,
    range_start: chrono::DateTime<chrono::Utc>,
    range_end: chrono::DateTime<chrono::Utc>,
    bucket_seconds: i64,
    buckets: &mut [SessionActivityBucket],
) -> Option<&mut SessionActivityBucket> {
    if timestamp < range_start || timestamp > range_end {
        return None;
    }
    let offset = timestamp
        .signed_duration_since(range_start)
        .num_seconds()
        .max(0);
    let index = (offset / bucket_seconds).min(buckets.len().saturating_sub(1) as i64) as usize;
    buckets.get_mut(index)
}

fn load_canonical_session_from_meta(
    provider: &dyn provider::Provider,
    provider_id: &str,
    meta: ProviderSessionSummary,
) -> Result<ImportedSession> {
    let source_path = meta
        .source_path
        .as_deref()
        .context("Session has no source path")?;
    let mut imported = provider.import_session(source_path)?;
    enrich_imported_session_from_meta(&mut imported, provider_id, &meta);
    Ok(imported)
}

fn enrich_imported_session_from_meta(
    imported: &mut ImportedSession,
    provider_id: &str,
    meta: &ProviderSessionSummary,
) {
    let display_title = resolved_display_title(provider_id, meta);
    apply_imported_session_title(imported, meta, display_title);
    if imported.session.context.workspace_dir.is_none() {
        imported.session.context.workspace_dir = meta.project_dir.clone();
    }
    if imported.session.context.last_active_at.is_none() {
        imported.session.context.last_active_at = meta
            .last_active_at
            .and_then(chrono::DateTime::from_timestamp_millis);
    }
    if imported
        .session
        .provenance
        .aliases
        .iter()
        .all(|alias| alias.provider_id != provider_id || alias.session_id != meta.session_id)
    {
        imported
            .session
            .provenance
            .aliases
            .push(crate::canonical::ProviderSessionRef {
                provider_id: provider_id.to_string(),
                session_id: meta.session_id.clone(),
                source_path: meta.source_path.clone(),
            });
    }
}

fn apply_imported_session_title(
    imported: &mut ImportedSession,
    meta: &ProviderSessionSummary,
    display_title: Option<String>,
) {
    imported.session.identity.source_title = display_title.or_else(|| {
        imported
            .session
            .identity
            .source_title
            .clone()
            .or(meta.title.clone())
    });
}

fn resolved_display_title(provider_id: &str, meta: &ProviderSessionSummary) -> Option<String> {
    let session_states = session_state::load_state_store().unwrap_or_default();
    let workspace_dir =
        session_management::normalized_workspace_key(provider_id, meta.project_dir.as_deref());
    session_state::resolve_session_state(
        &session_states,
        provider_id,
        &meta.session_id,
        workspace_dir.as_deref(),
    )
    .display_title
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportParams {
    pub provider: String,
    pub session_id: String,
    pub output_prefix: Option<String>,
    pub format: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportResult {
    pub files: Vec<String>,
}

pub fn export_session(params: &ExportParams, actor: ActivityActor) -> Result<ExportResult> {
    let mut activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "provider_session_id": params.session_id,
        "format": params.format,
        "output_prefix": params.output_prefix,
        "output_dir": params.output_dir,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(params.provider.clone()),
        provider_session_id: Some(params.session_id.clone()),
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Export,
        actor,
        summary: "Exporting session".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let imported = get_canonical_session(&params.provider, &params.session_id)?;
        let prefix = params
            .output_prefix
            .as_deref()
            .unwrap_or(&params.session_id);
        let output_dir = params.output_dir.as_deref().map(std::path::Path::new);
        let export = session_management::write_session_export_files(
            &imported.session,
            prefix,
            &params.format,
            output_dir,
        )?;
        let artifacts = register_session_export_artifacts(
            &mut activity_conn,
            &activity_id,
            &params.provider,
            &params.session_id,
            &params.format,
            &export,
        )?;
        Ok((export, artifacts))
    })();
    match result {
        Ok((export, artifacts)) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Exported session",
                    serde_json::json!({
                        "provider_session_id": params.session_id,
                        "format": params.format,
                        "files": export.files,
                        "artifact_ids": artifacts.iter().map(|artifact| artifact.id.clone()).collect::<Vec<_>>(),
                    }),
                ),
            )?;
            Ok(export)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed("Failed to export session", input_details, &message),
            )?;
            Err(error)
        }
    }
}

fn register_session_export_artifacts(
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpandCompressionSessionParams {
    pub file: String,
    pub output_prefix: Option<String>,
    pub format: String,
}

pub fn expand_compression_session(
    params: &ExpandCompressionSessionParams,
    actor: ActivityActor,
) -> Result<ExportResult> {
    let mut conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "source_file": params.file,
        "format": params.format,
        "output_prefix": params.output_prefix,
    });
    let activity_id = ActivityStore::new(&conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Compress,
        actor,
        summary: "Expanding compressed session".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let session = session_management::read_session_export_file(&params.file)?;
        let provider_id = session.provenance.primary_source.provider_id.trim();
        let provider_id = if provider_id.is_empty() {
            "memorph".to_string()
        } else {
            provider_id.to_string()
        };
        let provider_session_id = session.provenance.primary_source.session_id.trim();
        let provider_session_id = if provider_session_id.is_empty() {
            session.identity.canonical_id.clone()
        } else {
            provider_session_id.to_string()
        };
        let export = session_management::expand_compression_session(params, &session)?;
        let artifacts = register_session_export_artifacts(
            &mut conn,
            &activity_id,
            &provider_id,
            &provider_session_id,
            &params.format,
            &export,
        )?;
        Ok((export, artifacts, provider_id, provider_session_id))
    })();
    match result {
        Ok((export, artifacts, provider_id, provider_session_id)) => {
            let mut completion = ActivityCompletion::success(
                "Expanded compressed session",
                serde_json::json!({
                    "source_file": params.file,
                    "format": params.format,
                    "files": export.files,
                    "artifact_ids": artifacts.iter().map(|artifact| artifact.id.clone()).collect::<Vec<_>>(),
                }),
            );
            completion.provider_id = Some(provider_id);
            completion.provider_session_id = Some(provider_session_id);
            ActivityStore::new(&conn).finish(&activity_id, completion)?;
            Ok(export)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to expand compressed session",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreNativeCompressionParams {
    pub provider_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreNativeCompressionResult {
    pub restored_segments: usize,
    pub restored_events: usize,
    pub remaining_archive_refs: Vec<String>,
    pub source_bytes_before: u64,
    pub source_bytes_after: u64,
}

pub fn restore_native_compression(
    params: &RestoreNativeCompressionParams,
    actor: ActivityActor,
) -> Result<RestoreNativeCompressionResult> {
    let mut conn = local_store::open_database()?;
    let details = serde_json::to_value(params)?;
    let activity_id = ActivityStore::new(&conn).start(NewActivity {
        provider_id: Some(params.provider_id.clone()),
        provider_session_id: Some(params.session_id.clone()),
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Compress,
        actor,
        summary: "Restoring compressed segments in native session".to_string(),
        details: details.clone(),
    })?;
    let result = (|| {
        let session = get_canonical_session(&params.provider_id, &params.session_id)?.session;
        let (restored, report) = compression::restore_compressed_segments_in_place(
            &session,
            params.archive_ref.as_deref(),
        )?;
        if report.expanded_segments == 0 {
            anyhow::bail!("Session has no restorable compressed segments");
        }
        let remaining_archive_refs = compression::compressed_archive_refs(&restored);
        let backup_root = crate::config::memorph_dir()?
            .join("artifacts")
            .join("backups");
        let replaced = session_management::replace_native_session(
            &params.provider_id,
            &params.session_id,
            &restored,
            &remaining_archive_refs,
            &activity_id,
            &backup_root,
            &mut conn,
        )?;
        session_state::update_session_state(
            &params.provider_id,
            &params.session_id,
            &session_state::SessionLocalStateUpdate {
                compressed_archive_refs: Some(remaining_archive_refs.clone()),
                ..Default::default()
            },
        )?;
        refresh_target_provider_sessions(&params.provider_id)?;
        Ok(RestoreNativeCompressionResult {
            restored_segments: report.expanded_segments,
            restored_events: report.restored_events,
            remaining_archive_refs,
            source_bytes_before: replaced.source_bytes_before,
            source_bytes_after: replaced.source_bytes_after,
        })
    })();
    match result {
        Ok(restored) => {
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Restored compressed segments in native session",
                    serde_json::to_value(&restored)?,
                ),
            )?;
            Ok(restored)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to restore compressed segments in native session",
                    details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreCompressionArchiveParams {
    pub archive_ref: String,
    pub output_prefix: Option<String>,
    pub format: String,
}

pub fn restore_compression_archive(
    params: &RestoreCompressionArchiveParams,
    actor: ActivityActor,
) -> Result<ExportResult> {
    let mut conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "archive_ref": params.archive_ref,
        "format": params.format,
        "output_prefix": params.output_prefix,
    });
    let activity_id = ActivityStore::new(&conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Compress,
        actor,
        summary: "Restoring compression archive".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let archive = compression::load_archive(&params.archive_ref)?;
        let session =
            session_management::session_from_compression_archive(&params.archive_ref, archive)?;
        let source = session.provenance.aliases.first();
        let provider_id = source
            .map(|reference| reference.provider_id.clone())
            .unwrap_or_else(|| "memorph".to_string());
        let provider_session_id = source
            .map(|reference| reference.session_id.clone())
            .unwrap_or_else(|| session.identity.canonical_id.clone());
        let export = session_management::restore_compression_archive(params, &session)?;
        let artifacts = register_session_export_artifacts(
            &mut conn,
            &activity_id,
            &provider_id,
            &provider_session_id,
            &params.format,
            &export,
        )?;
        Ok((export, artifacts, provider_id, provider_session_id))
    })();
    match result {
        Ok((export, artifacts, provider_id, provider_session_id)) => {
            let mut completion = ActivityCompletion::success(
                "Restored compression archive",
                serde_json::json!({
                    "archive_ref": params.archive_ref,
                    "format": params.format,
                    "files": export.files,
                    "artifact_ids": artifacts.iter().map(|artifact| artifact.id.clone()).collect::<Vec<_>>(),
                }),
            );
            completion.provider_id = Some(provider_id);
            completion.provider_session_id = Some(provider_session_id);
            ActivityStore::new(&conn).finish(&activity_id, completion)?;
            Ok(export)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to restore compression archive",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrieveCompressionArchiveParams {
    pub archive_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedCompressionArchive {
    pub archive_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<usize>,
    pub retrieval_mode: CompressionRetrievalMode,
    pub recommended_next_action: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub canonical_id: String,
    pub source_provider_id: String,
    pub target_provider_id: String,
    pub summary_event_id: String,
    pub source_event_ids: Vec<String>,
    pub source_event_count: usize,
    pub returned_event_ids: Vec<String>,
    pub returned_event_count: usize,
    pub omitted_event_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<RetrievedCompressionArchiveMatch>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionRetrievalMode {
    FullArchive,
    QueryMatches,
    QueryNoMatches,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievedCompressionArchiveMatch {
    pub event_id: String,
    pub event_index: usize,
    pub score: usize,
    pub snippets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalToolSpec {
    pub name: String,
    pub description: String,
    pub archive_ref_scheme: String,
    pub api: CompressionRetrievalToolApiSpec,
    pub cli: CompressionRetrievalToolCliSpec,
    pub input_schema: serde_json::Value,
    pub output_contract: CompressionRetrievalToolOutputContract,
    pub usage_rules: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalToolApiSpec {
    pub method: String,
    pub path: String,
    pub body_example: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalToolCliSpec {
    pub command: String,
    pub query_command: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalToolOutputContract {
    pub full_retrieval: Vec<String>,
    pub query_retrieval: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionRetrievalInstructions {
    pub archive_ref: String,
    pub summary: String,
    pub query_first_cli: String,
    pub full_cli: String,
    pub api_query_body: serde_json::Value,
    pub api_full_body: serde_json::Value,
    pub suggested_steps: Vec<String>,
}

pub fn compression_retrieval_tool_spec() -> CompressionRetrievalToolSpec {
    CompressionRetrievalToolSpec {
        name: "memorph_retrieve_compression_archive".to_string(),
        description: "Retrieve original events from a durable memorph compression archive. Use query retrieval first when only specific details are needed.".to_string(),
        archive_ref_scheme: MEMORPH_ARCHIVE_SCHEME.to_string(),
        api: CompressionRetrievalToolApiSpec {
            method: "POST".to_string(),
            path: "/api/v1/compression/retrieve".to_string(),
            body_example: serde_json::json!({
                "archive_ref": "memorph-archive://canonical-id/archive.json.gz",
                "query": "optional search terms",
                "max_results": 5
            }),
        },
        cli: CompressionRetrievalToolCliSpec {
            command: "memorph compression retrieve <ARCHIVE_REF>".to_string(),
            query_command:
                "memorph compression retrieve <ARCHIVE_REF> --query <QUERY> --max-results 5"
                    .to_string(),
        },
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "archive_ref": {
                    "type": "string",
                    "description": "Durable archive reference from a compressed session block. Must start with memorph-archive://."
                },
                "query": {
                    "type": "string",
                    "description": "Optional search query. When provided, retrieval returns only matching archived events and snippets."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Maximum matching events returned in query mode. Omit to use the default."
                }
            },
            "required": ["archive_ref"]
        }),
        output_contract: CompressionRetrievalToolOutputContract {
            full_retrieval: vec![
                "retrieval_mode is full_archive".to_string(),
                "events contains every original archived SessionEvent".to_string(),
                "source_event_count equals the archive's full original event count".to_string(),
                "returned_event_ids contains every returned event id".to_string(),
                "returned_event_count equals events.length".to_string(),
                "omitted_event_count is 0".to_string(),
            ],
            query_retrieval: vec![
                "retrieval_mode is query_matches or query_no_matches".to_string(),
                "events contains only matching archived SessionEvent values".to_string(),
                "returned_event_ids contains only matching event ids".to_string(),
                "omitted_event_count reports archived events not returned by the query".to_string(),
                "matches contains event_id, event_index, score, and snippets for each returned event".to_string(),
                "source_event_count still reports the archive's full original event count".to_string(),
            ],
        },
        usage_rules: vec![
            "Do not expand a compressed archive unconditionally when switching or continuing a session.".to_string(),
            "Prefer query retrieval before full retrieval to avoid putting large archived history back into context.".to_string(),
            "Query scores prioritize exact phrase matches, then coverage of distinct query terms, with repeated single-term hits treated as weak evidence.".to_string(),
            "Use full retrieval only when the task explicitly requires the complete original segment.".to_string(),
            "Archive retrieval is lossless; summaries are model-visible hints, not the source of truth.".to_string(),
        ],
    }
}

pub fn compression_retrieval_instructions(
    archive_ref: &str,
) -> Result<CompressionRetrievalInstructions> {
    let archive_ref = archive_ref.trim();
    if !archive_ref.starts_with(MEMORPH_ARCHIVE_SCHEME) {
        anyhow::bail!("Unsupported compression archive ref: {}", archive_ref);
    }

    Ok(CompressionRetrievalInstructions {
        archive_ref: archive_ref.to_string(),
        summary: "Use query retrieval first. Full retrieval should be reserved for tasks that explicitly need the entire original compressed segment.".to_string(),
        query_first_cli: format!(
            "memorph compression retrieve {} --query <terms> --max-results 5",
            archive_ref
        ),
        full_cli: format!("memorph compression retrieve {}", archive_ref),
        api_query_body: serde_json::json!({
            "archive_ref": archive_ref,
            "query": "<terms>",
            "max_results": 5
        }),
        api_full_body: serde_json::json!({
            "archive_ref": archive_ref
        }),
        suggested_steps: vec![
            "Extract the memorph-archive://... value from the compressed block.".to_string(),
            "Choose a narrow query from the current user question or missing detail.".to_string(),
            "Run query retrieval and use only the returned matching events/snippets.".to_string(),
            "When multiple matches are returned, prefer higher scores; scoring favors exact phrases and broader term coverage over repeated single-term noise.".to_string(),
            "Use full retrieval only if query retrieval is insufficient and complete original context is required.".to_string(),
        ],
    })
}

pub fn retrieve_compression_archive(
    params: &RetrieveCompressionArchiveParams,
) -> Result<RetrievedCompressionArchive> {
    let archive = compression::load_archive(&params.archive_ref)?;
    Ok(retrieved_compression_archive(params, archive))
}

#[cfg(test)]
fn retrieve_compression_archive_in_dir(
    params: &RetrieveCompressionArchiveParams,
    archive_dir: &std::path::Path,
) -> Result<RetrievedCompressionArchive> {
    let archive = compression::load_archive_from_dir(archive_dir, &params.archive_ref)?;
    Ok(retrieved_compression_archive(params, archive))
}

fn retrieved_compression_archive(
    params: &RetrieveCompressionArchiveParams,
    archive: compression::CompressionArchive,
) -> RetrievedCompressionArchive {
    let source_event_count = archive.events.len();
    let query = params
        .query
        .as_deref()
        .map(str::trim)
        .filter(|q| !q.is_empty());
    let max_results = params.max_results;
    let (events, matches) = if let Some(query) = query {
        search_archive_events(&archive.events, query, max_results.unwrap_or(20))
    } else {
        (archive.events, Vec::new())
    };
    let returned_event_count = events.len();
    let returned_event_ids = events
        .iter()
        .map(|event| event.id.clone())
        .collect::<Vec<_>>();
    let omitted_event_count = source_event_count.saturating_sub(returned_event_count);
    let retrieval_mode = match (query, returned_event_count) {
        (Some(_), 0) => CompressionRetrievalMode::QueryNoMatches,
        (Some(_), _) => CompressionRetrievalMode::QueryMatches,
        (None, _) => CompressionRetrievalMode::FullArchive,
    };
    let recommended_next_action = retrieval_next_action(retrieval_mode);
    RetrievedCompressionArchive {
        archive_ref: params.archive_ref.clone(),
        query: query.map(str::to_string),
        max_results,
        retrieval_mode,
        recommended_next_action,
        created_at: archive.created_at,
        canonical_id: archive.canonical_id,
        source_provider_id: archive.source_provider_id,
        target_provider_id: archive.target_provider_id,
        summary_event_id: archive.summary_event_id,
        source_event_ids: archive.source_event_ids,
        source_event_count,
        returned_event_ids,
        returned_event_count,
        omitted_event_count,
        matches,
        events,
    }
}

fn retrieval_next_action(mode: CompressionRetrievalMode) -> String {
    match mode {
        CompressionRetrievalMode::FullArchive => {
            "This is the complete archived segment. Use only the needed parts in the active context."
        }
        CompressionRetrievalMode::QueryMatches => {
            "This is a query-filtered partial retrieval. Treat it as relevant snippets/events, not the complete archived history."
        }
        CompressionRetrievalMode::QueryNoMatches => {
            "No archived events matched this query. Try a broader query or use full retrieval only if the complete original segment is required."
        }
    }
    .to_string()
}

fn search_archive_events(
    events: &[SessionEvent],
    query: &str,
    max_results: usize,
) -> (Vec<SessionEvent>, Vec<RetrievedCompressionArchiveMatch>) {
    if max_results == 0 {
        return (Vec::new(), Vec::new());
    }
    let query_lower = query.to_ascii_lowercase();
    let mut terms = Vec::new();
    for term in query_lower
        .split_whitespace()
        .filter(|term| !term.is_empty())
    {
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    if terms.is_empty() {
        return (events.to_vec(), Vec::new());
    }

    let mut ranked = events
        .iter()
        .enumerate()
        .filter_map(|(event_index, event)| {
            let text = provider::canonical_event_text(event);
            let text_lower = text.to_ascii_lowercase();
            let score = archive_query_score(&text_lower, &query_lower, &terms);
            if score == 0 {
                return None;
            }
            let snippets = archive_search_snippets(&text, &query_lower, &terms);
            Some((
                event_index,
                score,
                event.clone(),
                RetrievedCompressionArchiveMatch {
                    event_id: event.id.clone(),
                    event_index,
                    score,
                    snippets,
                },
            ))
        })
        .collect::<Vec<_>>();

    ranked.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    ranked.truncate(max_results);

    let events = ranked
        .iter()
        .map(|(_, _, event, _)| event.clone())
        .collect::<Vec<_>>();
    let matches = ranked
        .into_iter()
        .map(|(_, _, _, search_match)| search_match)
        .collect::<Vec<_>>();
    (events, matches)
}

fn archive_query_score(text_lower: &str, query_lower: &str, terms: &[&str]) -> usize {
    let mut matched_terms = 0;
    let mut capped_occurrences = 0;
    for term in terms {
        let count = text_lower.matches(term).count();
        if count > 0 {
            matched_terms += 1;
            capped_occurrences += count.min(3);
        }
    }
    if matched_terms == 0 {
        return 0;
    }

    let mut score = matched_terms * 20 + capped_occurrences;
    if matched_terms == terms.len() {
        score += 50;
    }
    if text_lower.contains(query_lower) {
        score += 100;
    }
    score
}

fn archive_search_snippets(text: &str, query_lower: &str, terms: &[&str]) -> Vec<String> {
    let mut snippets = text
        .lines()
        .filter_map(|line| {
            let line_lower = line.to_ascii_lowercase();
            if line_lower.contains(query_lower)
                || terms.iter().any(|term| line_lower.contains(term))
            {
                Some(truncate_search_snippet(line.trim(), 240))
            } else {
                None
            }
        })
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>();
    if snippets.is_empty() {
        snippets.push(truncate_search_snippet(text.trim(), 240));
    }
    snippets
}

fn truncate_search_snippet(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

pub fn list_compression_archives(
    workspace: Option<&str>,
) -> Result<Vec<compression::CompressionArchiveSummary>> {
    session_management::list_compression_archives(workspace)
}

pub fn get_compression_archive(archive_ref: &str) -> Result<compression::CompressionArchive> {
    compression::load_archive(archive_ref)
}

pub fn list_compression_provider_support() -> Vec<crate::provider::ProviderCompressionSupport> {
    session_management::list_compression_provider_support()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCompressionDryRunParams {
    pub source_provider_id: String,
    pub target_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default)]
    pub policy: ActiveCompressionPolicy,
}

pub fn active_compression_dry_run(
    params: &ActiveCompressionDryRunParams,
) -> Result<ActiveCompressionReport> {
    let session = load_active_compression_source_session(
        &params.source_provider_id,
        params.session_id.as_deref(),
        params.file.as_deref(),
    )?;

    Ok(active_compression::build_dry_run_report(
        &session,
        ActiveCompressionParams {
            source_provider_id: params.source_provider_id.clone(),
            target_provider_id: params.target_provider_id.clone(),
            policy: params.policy.clone(),
            dry_run: true,
        },
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCompressionApplyCommandParams {
    pub source_provider_id: String,
    pub target_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default)]
    pub policy: ActiveCompressionPolicy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidate_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_prefix: Option<String>,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveCompressionApplyCommandResult {
    pub files: Vec<String>,
    pub archive_refs: Vec<String>,
    pub report: ActiveCompressionReport,
    pub source_bytes_before: u64,
    pub source_bytes_after: u64,
}

pub fn active_compression_apply(
    params: &ActiveCompressionApplyCommandParams,
    actor: ActivityActor,
) -> Result<ActiveCompressionApplyCommandResult> {
    let mut activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "provider_session_id": params.session_id,
        "source_file": params.file,
        "source_provider_id": params.source_provider_id,
        "target_provider_id": params.target_provider_id,
        "candidate_ids": params.candidate_ids,
        "format": params.format,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(params.source_provider_id.clone()),
        provider_session_id: params.session_id.clone(),
        workspace_dir: None,
        operation_kind: ActivityOperationKind::Compress,
        actor,
        summary: "Applying active session compression".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        if params.session_id.is_none() || params.file.is_some() {
            anyhow::bail!("Native compression requires session_id and does not accept file");
        }
        if params.source_provider_id != params.target_provider_id {
            anyhow::bail!("Native compression target must match the source provider");
        }
        let session = load_active_compression_source_session(
            &params.source_provider_id,
            params.session_id.as_deref(),
            params.file.as_deref(),
        )?;
        let archive_dir = compression::archive_base_dir()?;
        let applied = apply_active_compression_to_session(params, &session, archive_dir.as_path())?;
        let artifacts = register_active_compression_archive_artifacts(
            &mut activity_conn,
            &activity_id,
            params,
            &session,
            archive_dir.as_path(),
            &applied.report.archive_refs,
        )?;
        let result = write_active_compression_application(
            params,
            applied,
            &activity_id,
            &mut activity_conn,
        )?;
        Ok((
            result,
            artifacts
                .into_iter()
                .map(|artifact| artifact.id)
                .collect::<Vec<_>>(),
        ))
    })();
    match result {
        Ok((applied, artifact_ids)) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::success(
                    "Applied active session compression",
                    serde_json::json!({
                        "provider_session_id": params.session_id,
                        "source_provider_id": params.source_provider_id,
                        "target_provider_id": params.target_provider_id,
                        "files": applied.files,
                        "archive_refs": applied.archive_refs,
                        "artifact_ids": artifact_ids,
                        "candidate_count": applied.report.candidates.len(),
                        "source_bytes_before": applied.source_bytes_before,
                        "source_bytes_after": applied.source_bytes_after,
                    }),
                ),
            )?;
            Ok(applied)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to apply active session compression",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

fn apply_active_compression_to_session(
    params: &ActiveCompressionApplyCommandParams,
    session: &CanonicalSession,
    archive_dir: &std::path::Path,
) -> Result<active_compression::ActiveCompressionApplyResult> {
    let apply_params = ActiveCompressionApplyParams {
        source_provider_id: params.source_provider_id.clone(),
        target_provider_id: params.target_provider_id.clone(),
        policy: params.policy.clone(),
        candidate_ids: params.candidate_ids.clone(),
    };
    active_compression::apply_active_compression_with_archive_dir(
        session,
        apply_params,
        archive_dir,
    )
}

fn write_active_compression_application(
    params: &ActiveCompressionApplyCommandParams,
    applied: active_compression::ActiveCompressionApplyResult,
    operation_id: &str,
    artifact_conn: &mut rusqlite::Connection,
) -> Result<ActiveCompressionApplyCommandResult> {
    let session_id = params
        .session_id
        .as_deref()
        .context("Native compression requires session_id")?;
    let backup_root = crate::config::memorph_dir()?
        .join("artifacts")
        .join("backups");
    let replaced = session_management::replace_native_session(
        &params.source_provider_id,
        session_id,
        &applied.session,
        &applied.report.archive_refs,
        operation_id,
        &backup_root,
        artifact_conn,
    )?;
    session_state::update_session_state(
        &params.source_provider_id,
        session_id,
        &session_state::SessionLocalStateUpdate {
            compressed_archive_refs: Some(applied.report.archive_refs.clone()),
            ..Default::default()
        },
    )?;
    refresh_target_provider_sessions(&params.source_provider_id)?;

    Ok(ActiveCompressionApplyCommandResult {
        files: Vec::new(),
        archive_refs: applied.report.archive_refs.clone(),
        report: applied.report,
        source_bytes_before: replaced.source_bytes_before,
        source_bytes_after: replaced.source_bytes_after,
    })
}

fn register_active_compression_archive_artifacts(
    conn: &mut rusqlite::Connection,
    operation_id: &str,
    params: &ActiveCompressionApplyCommandParams,
    session: &CanonicalSession,
    archive_dir: &std::path::Path,
    archive_refs: &[String],
) -> Result<Vec<crate::storage::artifact_store::ArtifactManifest>> {
    let manifests = archive_refs
        .iter()
        .map(|archive_ref| {
            Ok(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::CompressionArchive,
                operation_id: Some(operation_id.to_string()),
                provider_id: Some(params.source_provider_id.clone()),
                provider_session_id: Some(
                    params
                        .session_id
                        .clone()
                        .unwrap_or_else(|| session.identity.canonical_id.clone()),
                ),
                session_id: None,
                projection_report_id: None,
                path: compression::archive_path_from_ref_in_dir(archive_dir, archive_ref)?,
                mime_type: Some("application/gzip".to_string()),
                format: Some("json.gz".to_string()),
                metadata: serde_json::json!({
                    "role": "active_compression_recovery_archive",
                    "archive_ref": archive_ref,
                    "canonical_id": session.identity.canonical_id,
                    "source_provider_id": params.source_provider_id,
                    "target_provider_id": params.target_provider_id,
                }),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    ArtifactStore::new(conn).register_paths(manifests)
}

fn load_active_compression_source_session(
    source_provider_id: &str,
    session_id: Option<&str>,
    file: Option<&str>,
) -> Result<CanonicalSession> {
    match (session_id, file) {
        (Some(_), Some(_)) => anyhow::bail!("Use either session_id or file, not both"),
        (Some(session_id), None) => {
            Ok(get_canonical_session(source_provider_id, session_id)?.session)
        }
        (None, Some(file)) => session_management::read_session_export_file(file),
        (None, None) => anyhow::bail!("Either session_id or file is required"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportParams {
    pub provider: String,
    pub file_or_id: String,
    pub to_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub provider_name: String,
    pub new_session_id: String,
    pub resume_command: Option<String>,
}

pub fn import_session(params: &ImportParams, actor: ActivityActor) -> Result<ImportResult> {
    let activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "source_ref": params.file_or_id,
        "target_provider_id": params.provider,
        "target_dir": params.to_dir,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(params.provider.clone()),
        provider_session_id: None,
        workspace_dir: params.to_dir.clone(),
        operation_kind: ActivityOperationKind::Import,
        actor,
        summary: "Importing session".to_string(),
        details: input_details.clone(),
    })?;
    let result = (|| {
        let session = if params.file_or_id.ends_with(".morph")
            || params.file_or_id.ends_with(".json")
            || params.file_or_id.ends_with(".md")
            || params.file_or_id.ends_with(".html")
        {
            session_management::read_session_export_file(&params.file_or_id)?
        } else {
            get_canonical_session(&params.provider, &params.file_or_id)?.session
        };

        let target_prov = providers::find_provider(&params.provider)
            .with_context(|| format!("Target provider not available: {}", params.provider))?;
        let target_capabilities = target_prov.capabilities();
        if !target_capabilities.export {
            anyhow::bail!(
                "Provider does not support writing sessions: {}",
                params.provider
            );
        }
        let target_dir = target_prov.resolve_workspace_dir(params.to_dir.as_deref())?;
        let (session, _) =
            session_management::prepare_session_for_target_provider(&session, &params.provider)?;
        let exported = target_prov.export_session(&session, &target_dir)?;

        Ok((
            ImportResult {
                provider_name: target_prov.name().to_string(),
                new_session_id: exported.session_id,
                resume_command: exported.resume_command,
            },
            target_dir,
        ))
    })();
    match result {
        Ok((imported, target_dir)) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status: ActivityStatus::Success,
                    provider_id: Some(params.provider.clone()),
                    provider_session_id: Some(imported.new_session_id.clone()),
                    workspace_dir: Some(target_dir.to_string_lossy().to_string()),
                    summary: "Imported session".to_string(),
                    details: serde_json::json!({
                        "source_ref": params.file_or_id,
                        "target_provider_id": params.provider,
                        "new_session_id": imported.new_session_id,
                        "target_dir": target_dir,
                        "resume_command": imported.resume_command,
                    }),
                    error: None,
                },
            )?;
            // Same index-refresh rationale as switch_session: the UI opens the
            // imported session immediately, so project it now rather than wait
            // for the background sync.
            index_target_provider_sessions(&params.provider);
            Ok(imported)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed("Failed to import session", input_details, &message),
            )?;
            Err(error)
        }
    }
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

pub fn list_management_activity(query: &ActivityQuery) -> Result<Vec<ActivityRecord>> {
    let conn = local_store::open_database()?;
    ActivityStore::new(&conn).query(query)
}

pub fn inspect_artifacts() -> Result<ArtifactInspectionReport> {
    let mut conn = local_store::open_database()?;
    let root = default_managed_artifact_root()?;
    ArtifactStore::new(&mut conn).inspect(&root)
}

pub fn cleanup_artifacts(
    retention_hours: u64,
    apply: bool,
    actor: ActivityActor,
) -> Result<ArtifactCleanupReport> {
    if retention_hours == 0 {
        anyhow::bail!("Artifact retention must be at least one hour");
    }
    let retention_ms = i64::try_from(retention_hours)
        .ok()
        .and_then(|hours| hours.checked_mul(60 * 60 * 1000))
        .context("Artifact retention exceeds supported range")?;
    let cutoff_ms = chrono::Utc::now()
        .timestamp_millis()
        .checked_sub(retention_ms)
        .context("Artifact retention cutoff is out of range")?;
    let activity_conn = local_store::open_database()?;
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: None,
        provider_session_id: None,
        workspace_dir: None,
        operation_kind: ActivityOperationKind::ArtifactCleanup,
        actor,
        summary: if apply {
            "Cleaning orphan artifact files"
        } else {
            "Planning orphan artifact cleanup"
        }
        .to_string(),
        details: serde_json::json!({
            "apply": apply,
            "retention_hours": retention_hours,
            "cutoff_ms": cutoff_ms,
        }),
    })?;
    let result = (|| {
        let mut conn = local_store::open_database()?;
        let root = default_managed_artifact_root()?;
        ArtifactStore::new(&mut conn).cleanup_orphan_files(&root, cutoff_ms, apply)
    })();
    match result {
        Ok(report) => {
            let status = if report.failures.is_empty() {
                ActivityStatus::Success
            } else {
                ActivityStatus::Partial
            };
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    status,
                    provider_id: None,
                    provider_session_id: None,
                    workspace_dir: None,
                    summary: if apply {
                        "Cleaned orphan artifact files"
                    } else {
                        "Planned orphan artifact cleanup"
                    }
                    .to_string(),
                    details: serde_json::to_value(&report)?,
                    error: (!report.failures.is_empty()).then(|| {
                        report
                            .failures
                            .iter()
                            .map(|failure| failure.reason.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    }),
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to manage orphan artifact files",
                    serde_json::json!({
                        "apply": apply,
                        "retention_hours": retention_hours,
                        "cutoff_ms": cutoff_ms,
                    }),
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchParams {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_title: Option<String>,
    #[serde(default)]
    pub move_original: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwitchResult {
    pub from_name: String,
    pub to_name: String,
    pub source_session_id: String,
    pub target_session_id: String,
    pub resume_command: Option<String>,
    #[serde(default)]
    pub removed_original: bool,
}

/// Immediately project the target provider into the SQLite session index after
/// `switch_session` or `import_session` writes its file.
///
/// Both the session list (`list_sessions`) and the detail view read session
/// identities straight from the SQLite index, which the 60s background sync
/// loop (`spawn_background_sync_loop`) only fills in after a delay. Without
/// this synchronous pass the UI opens the freshly written session and hits
/// "Session is not indexed" for up to a minute. Best-effort: a failure only
/// logs, because the export already succeeded and the background sync will
/// still catch up.
fn refresh_target_provider_sessions(provider_id: &str) -> Result<()> {
    let provider_id = providers::canonical_provider_id(provider_id);
    let mut conn = local_store::open_database()?;
    bootstrap_session_projections_in_connection(&mut conn, Some(provider_id.as_str())).map(|_| ())
}

fn index_target_provider_sessions(provider_id: &str) {
    let provider_id = providers::canonical_provider_id(provider_id);
    if let Err(error) = refresh_target_provider_sessions(&provider_id) {
        crate::logging::error(
            "target_provider_index_refresh",
            &format!(
                "Failed to project {provider_id} sessions into the index after writing a session: {error:#}"
            ),
        );
    }
}

pub fn switch_session(params: &SwitchParams) -> Result<SwitchResult> {
    let cwd = std::env::current_dir()?;

    let source_prov = providers::find_provider(&params.from)
        .with_context(|| format!("Unknown source provider: {}", params.from))?;
    let source_capabilities = source_prov.capabilities();
    if !source_capabilities.scan || !source_capabilities.import {
        anyhow::bail!(
            "Source provider does not support reading sessions: {}",
            params.from
        );
    }
    let cwd_str = cwd.to_string_lossy().to_string();

    let session_meta = if let Some(id) = &params.session_id {
        source_prov
            .get_session_meta(id)?
            .with_context(|| format!("Session not found: {}", id))?
    } else {
        let cache = crate::cache::global_cache();
        let sessions = cache.get_or_refresh(&params.from, || source_prov.scan_sessions())?;
        let mut candidates: Vec<_> = sessions
            .into_iter()
            .filter(|s| source_prov.workspace_matches(s.project_dir.as_deref(), Some(&cwd_str)))
            .collect();
        candidates.sort_by_key(|s| std::cmp::Reverse(s.last_active_at));
        candidates.into_iter().next().with_context(|| {
            format!(
                "No {} session found in current workspace: {}\nUse --session-id to specify one, or run from the project directory.",
                source_prov.name(),
                cwd_str
            )
        })?
    };

    let source_session_id = session_meta.session_id.clone();
    let imported =
        load_canonical_session_from_meta(source_prov.as_ref(), &params.from, session_meta)?;

    let target_prov = providers::find_provider(&params.to)
        .with_context(|| format!("Unknown target provider: {}", params.to))?;
    let target_capabilities = target_prov.capabilities();
    if !target_capabilities.export {
        anyhow::bail!(
            "Target provider does not support writing sessions: {}",
            params.to
        );
    }
    let target_dir = target_prov.resolve_workspace_dir(params.to_dir.as_deref())?;
    let (mut session, _) = session_management::prepare_session_for_export(
        &imported.session,
        &params.from,
        &params.to,
    )?;
    if let Some(raw_title) = params.target_title.as_ref() {
        let trimmed = raw_title.trim();
        if !trimmed.is_empty() {
            session.identity.source_title = Some(trimmed.to_string());
        }
    }
    let exported = target_prov.export_session(&session, &target_dir)?;

    let mut removed_original = false;
    if params.move_original {
        if !source_capabilities.delete {
            anyhow::bail!(
                "Source provider does not support deleting sessions: {}",
                params.from
            );
        }
        source_prov.delete_session(&source_session_id)?;
        removed_original = true;
    }

    // Make the freshly exported session visible to list/detail views without
    // waiting on the 60s background sync, which otherwise leaves a
    // "Session is not indexed" window for the UI.
    index_target_provider_sessions(&params.to);

    Ok(SwitchResult {
        from_name: source_prov.name().to_string(),
        to_name: target_prov.name().to_string(),
        source_session_id,
        target_session_id: exported.session_id,
        resume_command: exported.resume_command,
        removed_original,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindParams {
    pub dir: Option<String>,
    pub session: Option<String>,
    pub providers: Vec<String>,
}

pub fn find_sessions(params: &FindParams) -> Result<Vec<SessionGroup>> {
    let groups = list_sessions(&SessionListParams {
        all: true,
        providers: params.providers.clone(),
        cwd: None,
        include_message_counts: true,
        limit: None,
        offset: None,
        sort: SessionListSort::Recent,
        hook_filter: SessionHookFilter::All,
    })?;

    Ok(groups
        .into_iter()
        .filter_map(|mut group| {
            group.sessions.retain(|session| {
                let dir_match = params.dir.as_ref().map_or(true, |directory| {
                    session
                        .project_dir
                        .as_ref()
                        .is_some_and(|project_dir| project_dir.contains(directory))
                });
                let session_match = params.session.as_ref().map_or(true, |pattern| {
                    session.session_id.contains(pattern)
                        || session
                            .title
                            .as_ref()
                            .is_some_and(|title| title.contains(pattern))
                        || session
                            .native_title
                            .as_ref()
                            .is_some_and(|title| title.contains(pattern))
                });
                dir_match && session_match
            });
            (!group.sessions.is_empty()).then_some(group)
        })
        .collect())
}

#[cfg(test)]
mod tests {
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
             CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT, tool_call_id TEXT, tool_calls TEXT, tool_name TEXT, timestamp REAL NOT NULL, reasoning TEXT, reasoning_content TEXT, reasoning_details TEXT, active INTEGER NOT NULL DEFAULT 1);
             INSERT INTO sessions VALUES ('hermes-1','Hermes fixture','/tmp/hermes-project','model-x',1000,NULL,1,0,0);
             INSERT INTO messages VALUES (1,'hermes-1','user','hello',NULL,NULL,NULL,1000,NULL,NULL,NULL,1);"
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
             CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, role TEXT NOT NULL, content TEXT, tool_call_id TEXT, tool_calls TEXT, tool_name TEXT, timestamp REAL NOT NULL, reasoning TEXT, reasoning_content TEXT, reasoning_details TEXT, active INTEGER NOT NULL DEFAULT 1);
             INSERT INTO sessions VALUES ('hermes-1','Before','/tmp/hermes-project','model-x',1000,NULL,1,0,0);
             INSERT INTO messages VALUES (1,'hermes-1','user','before',NULL,NULL,NULL,1000,NULL,NULL,NULL,1);"
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
        conn.execute("UPDATE sessions SET title = 'After' WHERE id = 'hermes-1'", [])
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
        std::fs::write(&source, r#"{"role":"user","content":"before"}
"#).unwrap();
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
        std::fs::write(&source, r#"{"role":"user","content":"after"}
"#).unwrap();
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
        std::fs::write(&source, r#"{"role":"user","content":"before","cwd":"/tmp/codebuddy"}
"#).unwrap();
        let session = ProviderSessionSummary {
            session_id: "codebuddy-session-1".into(), title: Some("before".into()),
            project_dir: Some("/tmp/codebuddy".into()), last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap(); local_store::apply_schema(&mut projection).unwrap();
        let mut report = SessionProjectionBootstrapReport::default();
        bootstrap_provider_session(&mut projection, "codebuddy", &session, &mut report);
        assert_eq!(report.projected_sessions, 1); assert_eq!(report.failed_sessions, 0); assert_eq!(report.missing_sources, 0);
    }

    #[test]
    fn codebuddy_projection_refreshes_after_session_source_change() {
        let dir = tempfile::tempdir().unwrap(); let source_dir = dir.path().join("project");
        std::fs::create_dir_all(&source_dir).unwrap(); let source = source_dir.join("codebuddy-session-1.jsonl");
        std::fs::write(&source, r#"{"role":"user","content":"before"}
"#).unwrap();
        let mut session = ProviderSessionSummary {
            session_id: "codebuddy-session-1".into(), title: Some("before".into()), project_dir: None, last_active_at: None,
            source_path: Some(source.to_string_lossy().into_owned()),
        };
        let mut projection = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&projection).unwrap(); local_store::apply_schema(&mut projection).unwrap();
        let mut first = SessionProjectionBootstrapReport::default(); bootstrap_provider_session(&mut projection, "codebuddy", &session, &mut first);
        assert_eq!(first.projected_sessions, 1);
        std::fs::write(&source, r#"{"role":"user","content":"after"}
"#).unwrap(); session.title = Some("after".into());
        let mut second = SessionProjectionBootstrapReport::default(); bootstrap_provider_session(&mut projection, "codebuddy", &session, &mut second);
        assert_eq!(second.projected_sessions, 1); assert_eq!(second.unchanged_sessions, 0);
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
    fn gemini_is_enabled_for_default_projection_bootstrap() {
        assert!(PROJECTED_SESSION_PROVIDER_IDS.contains(&"gemini"));
        assert!(provider_supports_session_projection("gemini"));
    }

    #[test]
    fn qwen_is_enabled_for_default_projection_bootstrap() {
        assert!(PROJECTED_SESSION_PROVIDER_IDS.contains(&"qwen"));
        assert!(provider_supports_session_projection("qwen"));
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
    fn emerging_providers_are_onboarded_into_projection_whitelist() {
        // Route A: all 12 generic providers must be on the projection whitelist so their
        // sessions are visible in the UI even before per-provider capability verification.
        for id in [
            "antigravity",
            "cline",
            "copilot",
            "windsurf",
            "codebuddy",
            "qoder",
            "trae",
            "droid",
            "stepfun",
            "workbuddy",
            "hermes",
            "pi",
        ] {
            assert!(
                PROJECTED_SESSION_PROVIDER_IDS.contains(&id),
                "emerging provider {id} must be whitelisted for projection",
            );
            assert!(provider_supports_session_projection(id));
        }
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

        let found = find_sessions(&FindParams {
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
            PROVIDER_ACTIVITY_DEFAULT_HOURS,
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
            PROVIDER_ACTIVITY_DEFAULT_HOURS,
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
