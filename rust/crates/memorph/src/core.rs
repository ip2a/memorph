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
pub mod session_event_search;
pub mod session_management;
pub mod session_mutation;
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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionListSort {
    #[default]
    Recent,
    Title,
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

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
