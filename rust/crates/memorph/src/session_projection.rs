use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cmp::Ordering;

pub const SESSION_PROJECTION_VERSION: i64 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionIdentity {
    pub canonical_session_id: String,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<SessionAlias>,
}

impl SessionIdentity {
    pub fn from_source(input: SessionIdentityInput<'_>) -> Result<Self> {
        let provider_id = required_text(input.provider_id, "provider_id")?;
        let provider_session_id = optional_text(input.provider_session_id);
        let source_path = optional_text(input.source_path);
        let workspace_dir = optional_text(input.workspace_dir);

        let identity_seed = provider_session_id
            .as_deref()
            .or(source_path.as_deref())
            .ok_or_else(|| anyhow::anyhow!("provider_session_id or source_path is required"))?;
        let canonical_session_id = canonical_session_id(&provider_id, identity_seed);

        let mut aliases = Vec::new();
        if let Some(value) = provider_session_id.clone() {
            aliases.push(SessionAlias {
                kind: SessionAliasKind::ProviderSessionId,
                value,
                provider_id: Some(provider_id.clone()),
            });
        }
        if let Some(value) = source_path.clone() {
            aliases.push(SessionAlias {
                kind: SessionAliasKind::SourcePath,
                value,
                provider_id: Some(provider_id.clone()),
            });
        }

        Ok(Self {
            canonical_session_id,
            provider_id,
            provider_session_id,
            source_path,
            workspace_dir,
            aliases,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionIdentityInput<'a> {
    pub provider_id: &'a str,
    pub provider_session_id: Option<&'a str>,
    pub source_path: Option<&'a str>,
    pub workspace_dir: Option<&'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAlias {
    pub kind: SessionAliasKind,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionAliasKind {
    ProviderSessionId,
    SourcePath,
    SyncHoldingId,
    HookCorrelationId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: String,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    pub status: SessionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at_ms: Option<i64>,
    pub event_count: usize,
    pub turn_count: usize,
    pub flags: SessionSnapshotFlags,
    pub projection_version: i64,
    pub stale: bool,
}

impl SessionSnapshot {
    pub fn visible_title(&self) -> Option<&str> {
        self.display_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                self.title
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Unknown,
    Active,
    Idle,
    Completed,
    Failed,
    Archived,
    Deleted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSnapshotFlags {
    pub archived: bool,
    pub hidden: bool,
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnProjection {
    pub id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_turn_id: Option<String>,
    pub status: TurnStatus,
    pub confidence: TurnConfidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<i64>,
    pub source_range: SourceRange,
    pub turn_order: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Unknown,
    Open,
    Completed,
    Failed,
    Interrupted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnConfidence {
    Exact,
    Inferred,
    Grouped,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedEventKey {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp_ms: Option<i64>,
    pub source_order: i64,
    pub stable_cursor: String,
}

impl ProjectedEventKey {
    pub fn new(
        timestamp_ms: Option<i64>,
        source_order: i64,
        stable_cursor: impl Into<String>,
    ) -> Self {
        Self {
            timestamp_ms,
            source_order,
            stable_cursor: stable_cursor.into(),
        }
    }

    fn timestamp_sort_value(&self) -> i64 {
        self.timestamp_ms.unwrap_or(i64::MAX)
    }
}

impl Ord for ProjectedEventKey {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            self.timestamp_sort_value(),
            self.source_order,
            &self.stable_cursor,
        )
            .cmp(&(
                other.timestamp_sort_value(),
                other.source_order,
                &other.stable_cursor,
            ))
    }
}

impl PartialOrd for ProjectedEventKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventVisibility {
    Visible,
    HiddenInternal,
    Diagnostic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionFidelity {
    Preserved,
    Normalized,
    Dropped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionReport {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    pub operation_kind: ProjectionOperationKind,
    pub projection_version: i64,
    pub status: ProjectionStatus,
    pub summary: ProjectionReportSummary,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ProjectionReportItem>,
}

impl ProjectionReport {
    pub fn new(
        id: impl Into<String>,
        provider_id: impl Into<String>,
        operation_kind: ProjectionOperationKind,
        created_at_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            session_id: None,
            provider_id: provider_id.into(),
            source_id: None,
            operation_kind,
            projection_version: SESSION_PROJECTION_VERSION,
            status: ProjectionStatus::Succeeded,
            summary: ProjectionReportSummary::default(),
            created_at_ms,
            items: Vec::new(),
        }
    }

    pub fn push_item(&mut self, item: ProjectionReportItem) {
        self.summary.record(item.fidelity);
        if item.fidelity == ProjectionFidelity::Dropped {
            self.status = ProjectionStatus::CompletedWithLoss;
        }
        self.items.push(item);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionOperationKind {
    Scan,
    Import,
    Export,
    Refresh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionStatus {
    Succeeded,
    CompletedWithLoss,
    Failed,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionReportSummary {
    pub preserved_count: usize,
    pub normalized_count: usize,
    pub dropped_count: usize,
}

impl ProjectionReportSummary {
    fn record(&mut self, fidelity: ProjectionFidelity) {
        match fidelity {
            ProjectionFidelity::Preserved => self.preserved_count += 1,
            ProjectionFidelity::Normalized => self.normalized_count += 1,
            ProjectionFidelity::Dropped => self.dropped_count += 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectionReportItem {
    pub item_order: i64,
    pub fidelity: ProjectionFidelity,
    pub scope: ProjectionItemScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionItemScope {
    Session,
    Turn,
    Event,
    Block,
    ProviderPayload,
}

fn canonical_session_id(provider_id: &str, identity_seed: &str) -> String {
    let digest = md5::compute(format!("{}\0{}", provider_id, identity_seed).as_bytes());
    format!("session_{}_{:x}", sanitize_id_part(provider_id), digest)
}

fn sanitize_id_part(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn required_text(value: &str, field_name: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{} is required", field_name);
    }
    Ok(value.to_string())
}

fn optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_identity_is_stable_for_same_provider_source() {
        let first = SessionIdentity::from_source(SessionIdentityInput {
            provider_id: "codex",
            provider_session_id: Some("turns-123"),
            source_path: Some("/tmp/a.jsonl"),
            workspace_dir: Some("/work/a"),
        })
        .unwrap();
        let second = SessionIdentity::from_source(SessionIdentityInput {
            provider_id: "codex",
            provider_session_id: Some("turns-123"),
            source_path: Some("/tmp/moved.jsonl"),
            workspace_dir: Some("/work/b"),
        })
        .unwrap();

        assert_eq!(first.canonical_session_id, second.canonical_session_id);
        assert_eq!(first.aliases.len(), 2);
    }

    #[test]
    fn session_identity_uses_source_path_when_provider_session_id_is_missing() {
        let first = SessionIdentity::from_source(SessionIdentityInput {
            provider_id: "claude",
            provider_session_id: None,
            source_path: Some("/sessions/one.jsonl"),
            workspace_dir: None,
        })
        .unwrap();
        let second = SessionIdentity::from_source(SessionIdentityInput {
            provider_id: "claude",
            provider_session_id: None,
            source_path: Some("/sessions/one.jsonl"),
            workspace_dir: Some("/work/a"),
        })
        .unwrap();

        assert_eq!(first.canonical_session_id, second.canonical_session_id);
        assert_eq!(first.aliases[0].kind, SessionAliasKind::SourcePath);
    }

    #[test]
    fn session_identity_requires_provider_and_source_identity() {
        assert!(SessionIdentity::from_source(SessionIdentityInput {
            provider_id: "",
            provider_session_id: Some("s1"),
            source_path: None,
            workspace_dir: None,
        })
        .is_err());
        assert!(SessionIdentity::from_source(SessionIdentityInput {
            provider_id: "opencode",
            provider_session_id: None,
            source_path: None,
            workspace_dir: None,
        })
        .is_err());
    }

    #[test]
    fn projected_event_key_orders_by_timestamp_source_order_and_cursor() {
        let mut keys = vec![
            ProjectedEventKey::new(Some(20), 0, "a"),
            ProjectedEventKey::new(Some(10), 2, "b"),
            ProjectedEventKey::new(None, 0, "missing-time"),
            ProjectedEventKey::new(Some(10), 1, "z"),
            ProjectedEventKey::new(Some(10), 1, "a"),
        ];

        keys.sort();

        assert_eq!(
            keys.iter()
                .map(|key| key.stable_cursor.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "z", "b", "a", "missing-time"]
        );
    }

    #[test]
    fn snapshot_visible_title_prefers_local_display_title() {
        let snapshot = SessionSnapshot {
            session_id: "s1".to_string(),
            provider_id: "codex".to_string(),
            title: Some("native".to_string()),
            display_title: Some("local".to_string()),
            workspace_dir: None,
            status: SessionStatus::Idle,
            last_active_at_ms: None,
            event_count: 2,
            turn_count: 1,
            flags: SessionSnapshotFlags::default(),
            projection_version: SESSION_PROJECTION_VERSION,
            stale: false,
        };

        assert_eq!(snapshot.visible_title(), Some("local"));
    }

    #[test]
    fn projection_report_summarizes_fidelity_and_marks_loss() {
        let mut report =
            ProjectionReport::new("report-1", "codex", ProjectionOperationKind::Import, 1000);
        report.push_item(ProjectionReportItem {
            item_order: 0,
            fidelity: ProjectionFidelity::Preserved,
            scope: ProjectionItemScope::Event,
            field_path: Some("events[0]".to_string()),
            reason: None,
            details: None,
        });
        report.push_item(ProjectionReportItem {
            item_order: 1,
            fidelity: ProjectionFidelity::Normalized,
            scope: ProjectionItemScope::Block,
            field_path: Some("blocks[0].text".to_string()),
            reason: Some("normalized provider text shape".to_string()),
            details: None,
        });
        report.push_item(ProjectionReportItem {
            item_order: 2,
            fidelity: ProjectionFidelity::Dropped,
            scope: ProjectionItemScope::ProviderPayload,
            field_path: Some("payload.debug".to_string()),
            reason: Some("empty debug field".to_string()),
            details: None,
        });

        assert_eq!(report.summary.preserved_count, 1);
        assert_eq!(report.summary.normalized_count, 1);
        assert_eq!(report.summary.dropped_count, 1);
        assert_eq!(report.status, ProjectionStatus::CompletedWithLoss);
    }

    #[test]
    fn turn_confidence_serializes_as_contract_name() {
        let value = serde_json::to_value(TurnConfidence::Inferred).unwrap();
        assert_eq!(value, Value::String("inferred".to_string()));
    }
}
