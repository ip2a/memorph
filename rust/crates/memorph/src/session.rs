//! memorph's session layer.
//!
//! The session format itself is the external [`oasf`] crate (Open Agent
//! Session Format), re-exported here. oasf v1 is a pure specification —
//! identity, context, events, blocks, model, usage — and intentionally
//! carries nothing about *how* a session was produced. This module owns the
//! memorph-local concerns that sit beside the pure format:
//!
//! - conversion provenance ([`Provenance`]) and per-event conversion
//!   metadata ([`EventMeta`]: origin, fidelity, provider extras),
//! - the conversion-fidelity report ([`MappingReport`]),
//! - memorph's local display/archive state ([`LocalSessionState`]).
//!
//! These never live on `oasf::Session`; they ride alongside it on
//! [`ImportedSession`] and in memorph's own projection store.

pub use oasf::{
    Block, Context, Event, EventKind, ExecutionOutcome, Identity, Links, Metadata, Role, Schema,
    Session, SessionRelation, TurnOutcome, Usage, OASF_SCHEMA_NAME, OASF_SCHEMA_VERSION,
};

use chrono::{DateTime, Utc};

pub fn execution_outcome(is_error: bool) -> ExecutionOutcome {
    if is_error {
        ExecutionOutcome::Failed
    } else {
        ExecutionOutcome::Succeeded
    }
}

pub fn execution_outcome_is_error(outcome: ExecutionOutcome) -> bool {
    matches!(
        outcome,
        ExecutionOutcome::Failed
            | ExecutionOutcome::Cancelled
            | ExecutionOutcome::Declined
            | ExecutionOutcome::TimedOut
    )
}
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Conversion provenance (memorph-local)
// ---------------------------------------------------------------------------

/// Where, when, and by whom a session entered memorph. A session may alias
/// several provider sources; [`Provenance::primary_source`] is authoritative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provenance {
    pub imported_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_by: Option<String>,
    pub primary_source: ProviderRef,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<ProviderRef>,
}

/// A reference to a session as it lives in a specific provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderRef {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

// ---------------------------------------------------------------------------
// Per-event conversion metadata (memorph-local; oasf::Event carries none)
// ---------------------------------------------------------------------------

/// The provider an event originated from, with its native identity preserved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSource {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

/// How faithfully an event captures its source, ordered by severity:
/// [`Fidelity::Preserved`] is best, [`Fidelity::Unsupported`] is worst.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Fidelity {
    Preserved,
    Normalized,
    Downgraded,
    Dropped,
    Unsupported,
}

impl Fidelity {
    /// The more severe of two fidelities.
    pub fn worst(self, other: Self) -> Self {
        self.max(other)
    }
}

/// Per-event conversion metadata that oasf does not carry. Paired by index
/// with [`Session::events`] on [`ImportedSession`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMeta {
    pub source: EventSource,
    pub fidelity: Fidelity,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_ext: BTreeMap<String, Value>,
}

impl EventMeta {
    /// Build an event-meta entry for an event fully preserved from `provider_id`.
    pub fn preserved(provider_id: impl Into<String>) -> Self {
        Self {
            source: EventSource {
                provider_id: provider_id.into(),
                original_id: None,
                original_role: None,
                phase: None,
            },
            fidelity: Fidelity::Preserved,
            provider_ext: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Conversion results
// ---------------------------------------------------------------------------

/// Result of importing a provider session: the pure oasf session, the
/// per-event conversion metadata (parallel to [`ImportedSession::session`]'s
/// events), the session provenance, and the conversion-fidelity report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedSession {
    pub session: Session,
    pub provenance: Provenance,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub event_meta: Vec<EventMeta>,
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
// Fidelity reporting (conversion-layer aggregate)
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

// ---------------------------------------------------------------------------
// Local session state (memorph-local)
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
