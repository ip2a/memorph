//! memorph's canonical session layer.
//!
//! The session format itself is defined by the [`oasf`](https://docs.rs/oasf)
//! crate (Open Agent Session Format) and re-exported here. This module adds
//! the memorph-local concerns that are not part of the open format: local
//! session state (archive / pin preferences per workspace) and
//! conversion-fidelity reporting for import and export.

pub use oasf::{
    Artifact, ArtifactKind, Block, Context, Event, EventKind, Fidelity, Identity, Links, Metadata,
    Provenance, ProviderRef, Role, Schema, Session, Source, TurnBoundary, Usage, OASF_SCHEMA_NAME,
    OASF_SCHEMA_VERSION,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Local session state (memorph-local, not part of the open format)
// ---------------------------------------------------------------------------

/// memorph's local view of a session: display preferences, archive/pin
/// state, and per-workspace overrides. Stored under the memorph home,
/// independent of the provider's own session store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSessionState {
    pub locator: SessionLocator,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_overrides: Vec<WorkspaceSessionState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compressed_archive_refs: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLocator {
    pub provider_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSessionState {
    pub workspace_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_targets: Option<Vec<String>>,
}

// ---------------------------------------------------------------------------
// Conversion results
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedSession {
    pub session: Session,
    pub report: MappingReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSession {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    pub report: MappingReport,
}

// ---------------------------------------------------------------------------
// Fidelity reporting (conversion-layer aggregate; per-event fidelity lives in oasf)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingReport {
    pub provider_id: String,
    pub direction: MappingDirection,
    pub overall: Fidelity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<MappingIssue>,
}

impl MappingReport {
    pub fn new(provider_id: impl Into<String>, direction: MappingDirection) -> Self {
        Self {
            provider_id: provider_id.into(),
            direction,
            overall: Fidelity::Preserved,
            issues: Vec::new(),
        }
    }

    pub fn push_issue(&mut self, issue: MappingIssue) {
        self.overall = self.overall.worst(issue.disposition);
        self.issues.push(issue);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingDirection {
    Import,
    Export,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingIssue {
    pub level: MappingIssueLevel,
    pub disposition: Fidelity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingIssueLevel {
    Info,
    Warning,
    Error,
}
