use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::canonical::{
    EventBlock, EventLinks, EventMetadata, EventRole, EventSource, MappingDirection,
    MappingDisposition, SessionEvent, SessionEventKind,
};
use crate::session_projection::{
    ProjectionFidelity, ProjectionItemScope, ProjectionOperationKind, ProjectionStatus,
};
use crate::storage::session_state::ResolvedLocalSessionState;

const DETAIL_REPORT_ITEM_LIMIT: i64 = 20;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SnapshotStaleScanReport {
    pub checked_sources: usize,
    pub fresh_snapshots: usize,
    pub stale_snapshots: usize,
    pub missing_sources: usize,
    pub unknown_sources: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedSessionSnapshotRow {
    pub canonical_session_id: String,
    pub provider_id: String,
    pub provider_session_id: Option<String>,
    pub title: Option<String>,
    pub display_title: Option<String>,
    pub workspace_dir: Option<String>,
    pub last_active_at_ms: Option<i64>,
    pub source_path: Option<String>,
    pub message_count: usize,
    pub event_count: usize,
    pub turn_count: usize,
    pub size_bytes: Option<u64>,
    pub hidden: bool,
    pub pinned: bool,
    pub preferred_targets: Vec<String>,
    pub stale: bool,
}

#[derive(Debug, Clone)]
pub struct ProjectedSessionDetailPage {
    pub canonical_session_id: String,
    pub provider_id: String,
    pub provider_session_id: Option<String>,
    pub title: Option<String>,
    pub display_title: Option<String>,
    pub workspace_dir: Option<String>,
    pub created_at_ms: Option<i64>,
    pub last_active_at_ms: Option<i64>,
    pub source_path: Option<String>,
    pub event_count: usize,
    pub message_count: usize,
    pub turn_count: usize,
    pub local_state: ResolvedLocalSessionState,
    pub projection_report: Option<ProjectedSessionReport>,
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedSessionReport {
    pub id: String,
    pub provider_id: String,
    pub source_id: Option<String>,
    pub operation_kind: ProjectionOperationKind,
    pub projection_version: i64,
    pub status: ProjectionStatus,
    pub created_at_ms: i64,
    pub summary: ProjectedSessionReportSummary,
    pub item_count: usize,
    pub items: Vec<ProjectedSessionReportItem>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectedSessionReportSummary {
    pub canonical_event_count: Option<usize>,
    pub mapping_direction: Option<MappingDirection>,
    pub mapping_overall: Option<MappingDisposition>,
    pub preserved_count: usize,
    pub normalized_count: usize,
    pub dropped_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedSessionReportItem {
    pub item_order: i64,
    pub fidelity: ProjectionFidelity,
    pub scope: ProjectionItemScope,
    pub field_path: Option<String>,
    pub reason: Option<String>,
    pub details: Option<serde_json::Value>,
}

pub struct SnapshotStore<'a> {
    conn: &'a Connection,
}

impl<'a> SnapshotStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn refresh_session_snapshot_staleness(&self) -> Result<SnapshotStaleScanReport> {
        let rows = self.session_snapshot_sources()?;
        let mut report = SnapshotStaleScanReport::default();
        for row in rows {
            let Some(source_path) = row.source_path.as_deref().filter(|value| !value.is_empty())
            else {
                report.unknown_sources += 1;
                report.stale_snapshots += 1;
                self.set_session_snapshot_stale(&row.session_id, true)?;
                continue;
            };
            let Some(stored_fingerprint) = row
                .source_fingerprint
                .as_deref()
                .filter(|value| !value.is_empty())
            else {
                report.unknown_sources += 1;
                report.stale_snapshots += 1;
                self.set_session_snapshot_stale(&row.session_id, true)?;
                continue;
            };

            let Some(current_fingerprint) =
                source_fingerprint_for_path(std::path::Path::new(source_path))?
            else {
                report.missing_sources += 1;
                report.stale_snapshots += 1;
                self.set_session_snapshot_stale(&row.session_id, true)?;
                continue;
            };

            report.checked_sources += 1;
            let stale = current_fingerprint != stored_fingerprint;
            if stale {
                report.stale_snapshots += 1;
            } else {
                report.fresh_snapshots += 1;
            }
            self.set_session_snapshot_stale(&row.session_id, stale)?;
        }
        Ok(report)
    }

    pub fn list_session_snapshots(&self) -> Result<Vec<ProjectedSessionSnapshotRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    ss.session_id,
                    ss.provider_id,
                    COALESCE(s.provider_session_id, src.provider_session_id),
                    ss.title,
                    COALESCE(local.display_title, ss.display_title),
                    ss.workspace_dir,
                    ss.last_active_at_ms,
                    src.source_path,
                    (
                        SELECT COUNT(*)
                        FROM session_events event
                        WHERE event.session_id = ss.session_id
                          AND event.visibility = 'visible'
                          AND event.kind = 'message'
                    ),
                    COALESCE(ss.event_count, s.event_count, 0),
                    COALESCE(ss.turn_count, s.turn_count, 0),
                    src.file_size_bytes,
                    ss.flags_json,
                    COALESCE(workspace.hidden, local.hidden),
                    COALESCE(workspace.pinned, local.pinned),
                    COALESCE(workspace.preferred_targets_json, local.preferred_targets_json),
                    ss.stale
                 FROM session_snapshots ss
                 JOIN sessions s ON s.id = ss.session_id
                 LEFT JOIN session_sources src ON src.id = s.primary_source_id
                 LEFT JOIN session_local_state local ON local.session_id = ss.session_id
                 LEFT JOIN workspace_session_state workspace
                    ON workspace.session_id = ss.session_id
                   AND workspace.workspace_dir = ss.workspace_dir
                 WHERE s.deleted_at_ms IS NULL
                 ORDER BY
                    ss.provider_id ASC,
                    COALESCE(workspace.pinned, local.pinned, 0) DESC,
                    ss.last_active_at_ms DESC,
                    lower(COALESCE(local.display_title, ss.display_title, ss.title, ss.session_id)) ASC",
            )
            .context("Failed to prepare projected session snapshot list")?;

        let rows = stmt
            .query_map([], |row| {
                let flags_json: String = row.get(12)?;
                let local_hidden: Option<i64> = row.get(13)?;
                let local_pinned: Option<i64> = row.get(14)?;
                let preferred_targets_json: Option<String> = row.get(15)?;
                let flags = parse_flags(&flags_json);
                let file_size_bytes: Option<i64> = row.get(11)?;

                Ok(ProjectedSessionSnapshotRow {
                    canonical_session_id: row.get(0)?,
                    provider_id: row.get(1)?,
                    provider_session_id: row.get(2)?,
                    title: row.get(3)?,
                    display_title: row.get(4)?,
                    workspace_dir: row.get(5)?,
                    last_active_at_ms: row.get(6)?,
                    source_path: row.get(7)?,
                    message_count: row.get::<_, i64>(8)?.max(0) as usize,
                    event_count: row.get::<_, i64>(9)?.max(0) as usize,
                    turn_count: row.get::<_, i64>(10)?.max(0) as usize,
                    size_bytes: file_size_bytes.and_then(|value| u64::try_from(value).ok()),
                    hidden: local_hidden.map(sql_bool).unwrap_or(flags.hidden),
                    pinned: local_pinned.map(sql_bool).unwrap_or(flags.pinned),
                    preferred_targets: parse_string_list(preferred_targets_json.as_deref()),
                    stale: sql_bool(row.get::<_, i64>(16)?),
                })
            })
            .context("Failed to query projected session snapshots")?;

        let mut snapshots = Vec::new();
        for row in rows {
            snapshots.push(row.context("Failed to decode projected session snapshot")?);
        }
        Ok(snapshots)
    }

    pub fn get_session_detail_page(
        &self,
        provider_id: &str,
        provider_session_id: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<Option<ProjectedSessionDetailPage>> {
        let Some(header) = self.session_detail_header(provider_id, provider_session_id)? else {
            return Ok(None);
        };
        let events = self.session_detail_events(
            &header.canonical_session_id,
            provider_id,
            event_offset,
            event_limit,
        )?;
        let projection_report = self.session_projection_report(&header.canonical_session_id)?;
        Ok(Some(ProjectedSessionDetailPage {
            projection_report,
            events,
            ..header
        }))
    }

    fn session_detail_header(
        &self,
        provider_id: &str,
        provider_session_id: &str,
    ) -> Result<Option<ProjectedSessionDetailPage>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    s.id,
                    s.provider_id,
                    COALESCE(s.provider_session_id, src.provider_session_id),
                    COALESCE(ss.title, s.title),
                    COALESCE(local.display_title, ss.display_title),
                    COALESCE(ss.workspace_dir, s.workspace_dir, src.workspace_dir),
                    s.created_at_ms,
                    COALESCE(ss.last_active_at_ms, s.last_active_at_ms),
                    src.source_path,
                    COALESCE(ss.event_count, s.event_count, 0),
                    (
                        SELECT COUNT(*)
                        FROM session_events event
                        WHERE event.session_id = s.id
                          AND event.visibility = 'visible'
                          AND event.kind = 'message'
                    ),
                    COALESCE(ss.turn_count, s.turn_count, 0),
                    local.archived,
                    COALESCE(workspace.hidden, local.hidden),
                    COALESCE(workspace.pinned, local.pinned),
                    local.notes,
                    local.tags_json,
                    COALESCE(workspace.preferred_targets_json, local.preferred_targets_json),
                    local.compressed_archive_refs_json,
                    local.display_title
                 FROM sessions s
                 LEFT JOIN session_sources src ON src.id = s.primary_source_id
                 LEFT JOIN session_snapshots ss ON ss.session_id = s.id
                 LEFT JOIN session_local_state local ON local.session_id = s.id
                 LEFT JOIN workspace_session_state workspace
                    ON workspace.session_id = s.id
                   AND workspace.workspace_dir = COALESCE(ss.workspace_dir, s.workspace_dir, src.workspace_dir)
                 WHERE s.deleted_at_ms IS NULL
                   AND s.provider_id = ?1
                   AND (
                        s.provider_session_id = ?2
                        OR s.id = ?2
                        OR EXISTS (
                            SELECT 1
                            FROM session_aliases alias
                            WHERE alias.session_id = s.id
                              AND alias.provider_id = ?1
                              AND alias.alias_kind = 'provider_session_id'
                              AND alias.alias_value = ?2
                        )
                   )
                 ORDER BY COALESCE(ss.last_active_at_ms, s.last_active_at_ms) DESC
                 LIMIT 1",
            )
            .context("Failed to prepare projected session detail header")?;

        let mut rows = stmt
            .query([provider_id, provider_session_id])
            .context("Failed to query projected session detail header")?;
        let Some(row) = rows
            .next()
            .context("Failed to decode projected session detail header")?
        else {
            return Ok(None);
        };

        let workspace_dir: Option<String> = row.get(5)?;
        Ok(Some(ProjectedSessionDetailPage {
            canonical_session_id: row.get(0)?,
            provider_id: row.get(1)?,
            provider_session_id: row.get(2)?,
            title: row.get(3)?,
            display_title: row.get(4)?,
            workspace_dir,
            created_at_ms: row.get(6)?,
            last_active_at_ms: row.get(7)?,
            source_path: row.get(8)?,
            event_count: row.get::<_, i64>(9)?.max(0) as usize,
            message_count: row.get::<_, i64>(10)?.max(0) as usize,
            turn_count: row.get::<_, i64>(11)?.max(0) as usize,
            local_state: ResolvedLocalSessionState {
                display_title: row.get(19)?,
                archived: row
                    .get::<_, Option<i64>>(12)?
                    .map(sql_bool)
                    .unwrap_or(false),
                hidden: row
                    .get::<_, Option<i64>>(13)?
                    .map(sql_bool)
                    .unwrap_or(false),
                pinned: row
                    .get::<_, Option<i64>>(14)?
                    .map(sql_bool)
                    .unwrap_or(false),
                notes: row.get(15)?,
                tags: parse_string_list(row.get::<_, Option<String>>(16)?.as_deref()),
                preferred_targets: parse_string_list(row.get::<_, Option<String>>(17)?.as_deref()),
                compressed_archive_refs: parse_string_list(
                    row.get::<_, Option<String>>(18)?.as_deref(),
                ),
            },
            projection_report: None,
            events: Vec::new(),
        }))
    }

    fn session_projection_report(
        &self,
        session_id: &str,
    ) -> Result<Option<ProjectedSessionReport>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    id,
                    provider_id,
                    source_id,
                    operation_kind,
                    projection_version,
                    status,
                    summary_json,
                    created_at_ms,
                    (
                        SELECT COUNT(*)
                        FROM projection_report_items item
                        WHERE item.report_id = projection_reports.id
                    )
                 FROM projection_reports
                 WHERE session_id = ?1
                 ORDER BY created_at_ms DESC, id DESC
                 LIMIT 1",
            )
            .context("Failed to prepare projected session report")?;

        let mut rows = stmt
            .query([session_id])
            .context("Failed to query projected session report")?;
        let Some(row) = rows
            .next()
            .context("Failed to decode projected session report")?
        else {
            return Ok(None);
        };

        let report_id: String = row.get(0)?;
        let summary_json: String = row.get(6)?;
        let item_count = row.get::<_, i64>(8)?.max(0) as usize;
        Ok(Some(ProjectedSessionReport {
            id: report_id.clone(),
            provider_id: row.get(1)?,
            source_id: row.get(2)?,
            operation_kind: parse_projection_operation(row.get::<_, String>(3)?.as_str())?,
            projection_version: row.get(4)?,
            status: parse_projection_status(row.get::<_, String>(5)?.as_str())?,
            created_at_ms: row.get(7)?,
            summary: parse_projection_report_summary(&summary_json)?,
            item_count,
            items: self.projection_report_items(&report_id)?,
        }))
    }

    fn projection_report_items(&self, report_id: &str) -> Result<Vec<ProjectedSessionReportItem>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT item_order, disposition, scope, field_path, reason, details_json
                 FROM projection_report_items
                 WHERE report_id = ?1
                 ORDER BY item_order
                 LIMIT ?2",
            )
            .context("Failed to prepare projected session report items")?;
        let rows = stmt
            .query_map((report_id, DETAIL_REPORT_ITEM_LIMIT), |row| {
                let details_json: String = row.get(5)?;
                Ok(ProjectedReportItemRow {
                    item_order: row.get(0)?,
                    disposition: row.get(1)?,
                    scope: row.get(2)?,
                    field_path: row.get(3)?,
                    reason: row.get(4)?,
                    details_json,
                })
            })
            .with_context(|| format!("Failed to query projected report items for {report_id}"))?;

        let mut items = Vec::new();
        for row in rows {
            items.push(
                row.with_context(|| {
                    format!("Failed to decode projected report item for {report_id}")
                })?
                .into_item()?,
            );
        }
        Ok(items)
    }

    fn session_detail_events(
        &self,
        session_id: &str,
        provider_id: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<Vec<SessionEvent>> {
        let limit = event_limit
            .and_then(|limit| i64::try_from(limit).ok())
            .unwrap_or(i64::MAX);
        let offset = i64::try_from(event_offset).unwrap_or(i64::MAX);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT
                    event.id,
                    event.role,
                    event.kind,
                    event.timestamp_ms,
                    event.metadata_json,
                    event.turn_id,
                    turn.turn_order
                 FROM session_events event
                 LEFT JOIN session_turns turn ON turn.id = event.turn_id
                 WHERE event.session_id = ?1
                 ORDER BY
                    COALESCE(event.timestamp_ms, 9223372036854775807),
                    event.source_order,
                    event.stable_cursor
                 LIMIT ?2 OFFSET ?3",
            )
            .context("Failed to prepare projected session detail events")?;
        let rows = stmt
            .query_map((session_id, limit, offset), |row| {
                Ok(ProjectedEventRow {
                    id: row.get(0)?,
                    role: row.get(1)?,
                    kind: row.get(2)?,
                    timestamp_ms: row.get(3)?,
                    metadata_json: row.get(4)?,
                    turn_id: row.get(5)?,
                    turn_order: row.get(6)?,
                })
            })
            .context("Failed to query projected session detail events")?;

        let mut event_rows = Vec::new();
        for row in rows {
            event_rows.push(row.context("Failed to decode projected session detail event")?);
        }
        if event_rows.is_empty() {
            return Ok(Vec::new());
        }

        let blocks = self.event_blocks_for_page(&event_rows)?;
        let mut events = Vec::with_capacity(event_rows.len());
        for row in event_rows {
            let event_blocks = blocks.get(&row.id).cloned().unwrap_or_default();
            events.push(row.into_event(provider_id, event_blocks)?);
        }
        Ok(events)
    }

    fn event_blocks_for_page(
        &self,
        event_rows: &[ProjectedEventRow],
    ) -> Result<BTreeMap<String, Vec<EventBlock>>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT content_json
                 FROM session_event_blocks
                 WHERE event_id = ?1
                 ORDER BY block_order",
            )
            .context("Failed to prepare projected session detail event blocks")?;
        let mut blocks_by_event = BTreeMap::new();
        for event in event_rows {
            let rows = stmt
                .query_map([event.id.as_str()], |row| row.get::<_, Option<String>>(0))
                .with_context(|| {
                    format!("Failed to query projected event blocks for {}", event.id)
                })?;
            let mut blocks = Vec::new();
            for row in rows {
                let Some(block_json) = row.with_context(|| {
                    format!("Failed to decode projected event block for {}", event.id)
                })?
                else {
                    continue;
                };
                blocks.push(
                    serde_json::from_str::<EventBlock>(&block_json).with_context(|| {
                        format!("Failed to parse projected event block for {}", event.id)
                    })?,
                );
            }
            blocks_by_event.insert(event.id.clone(), blocks);
        }
        Ok(blocks_by_event)
    }

    fn session_snapshot_sources(&self) -> Result<Vec<SnapshotSourceRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ss.session_id, src.source_path, ss.source_fingerprint
                 FROM session_snapshots ss
                 JOIN sessions s ON s.id = ss.session_id
                 LEFT JOIN session_sources src ON src.id = s.primary_source_id
                 WHERE s.deleted_at_ms IS NULL
                 ORDER BY ss.session_id",
            )
            .context("Failed to prepare projected session source staleness scan")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SnapshotSourceRow {
                    session_id: row.get(0)?,
                    source_path: row.get(1)?,
                    source_fingerprint: row.get(2)?,
                })
            })
            .context("Failed to query projected session source staleness scan")?;

        let mut sources = Vec::new();
        for row in rows {
            sources.push(row.context("Failed to decode projected session source staleness row")?);
        }
        Ok(sources)
    }

    fn set_session_snapshot_stale(&self, session_id: &str, stale: bool) -> Result<()> {
        self.conn
            .execute(
                "UPDATE session_snapshots SET stale = ?1 WHERE session_id = ?2",
                (if stale { 1_i64 } else { 0_i64 }, session_id),
            )
            .with_context(|| {
                format!("Failed to update projected session staleness: {session_id}")
            })?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct SnapshotSourceRow {
    session_id: String,
    source_path: Option<String>,
    source_fingerprint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProjectionReportSummaryJson {
    canonical_event_count: Option<usize>,
    mapping_direction: Option<MappingDirection>,
    mapping_overall: Option<MappingDisposition>,
    #[serde(default)]
    preserved_count: usize,
    #[serde(default)]
    normalized_count: usize,
    #[serde(default)]
    dropped_count: usize,
}

#[derive(Debug, Clone)]
struct ProjectedReportItemRow {
    item_order: i64,
    disposition: String,
    scope: String,
    field_path: Option<String>,
    reason: Option<String>,
    details_json: String,
}

impl ProjectedReportItemRow {
    fn into_item(self) -> Result<ProjectedSessionReportItem> {
        Ok(ProjectedSessionReportItem {
            item_order: self.item_order,
            fidelity: parse_projection_fidelity(&self.disposition)?,
            scope: parse_projection_item_scope(&self.scope)?,
            field_path: self.field_path,
            reason: self.reason,
            details: parse_projection_details(&self.details_json)?,
        })
    }
}

#[derive(Debug, Clone)]
struct ProjectedEventRow {
    id: String,
    role: Option<String>,
    kind: String,
    timestamp_ms: Option<i64>,
    metadata_json: String,
    turn_id: Option<String>,
    turn_order: Option<i64>,
}

impl ProjectedEventRow {
    fn into_event(self, provider_id: &str, blocks: Vec<EventBlock>) -> Result<SessionEvent> {
        let mut metadata =
            parse_event_metadata(provider_id, self.id.as_str(), &self.metadata_json)?;
        metadata
            .provider_ext
            .entry("projected_event_id".to_string())
            .or_insert_with(|| serde_json::Value::String(self.id.clone()));
        if let Some(turn_id) = self.turn_id.as_deref() {
            metadata
                .provider_ext
                .entry("projected_turn_id".to_string())
                .or_insert_with(|| serde_json::Value::String(turn_id.to_string()));
        }

        let links = EventLinks {
            turn_index: self.turn_order.and_then(|value| u32::try_from(value).ok()),
            ..EventLinks::default()
        };

        Ok(SessionEvent {
            id: self.id,
            kind: parse_event_kind(&self.kind)?,
            role: parse_event_role(self.role.as_deref())?,
            timestamp: timestamp_from_ms(self.timestamp_ms),
            links,
            blocks,
            metadata,
        })
    }
}

#[derive(Debug, Default, Deserialize)]
struct SnapshotFlags {
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    pinned: bool,
}

fn parse_flags(value: &str) -> SnapshotFlags {
    serde_json::from_str(value).unwrap_or_default()
}

fn parse_string_list(value: Option<&str>) -> Vec<String> {
    value
        .and_then(|value| serde_json::from_str::<Vec<String>>(value).ok())
        .unwrap_or_default()
}

fn parse_event_kind(value: &str) -> Result<SessionEventKind> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .with_context(|| format!("Unknown projected event kind: {value}"))
}

fn parse_event_role(value: Option<&str>) -> Result<EventRole> {
    let value = value.unwrap_or("unknown");
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .with_context(|| format!("Unknown projected event role: {value}"))
}

fn parse_event_metadata(provider_id: &str, event_id: &str, value: &str) -> Result<EventMetadata> {
    if value.trim().is_empty() || value.trim() == "{}" {
        return Ok(default_event_metadata(provider_id, event_id));
    }
    serde_json::from_str(value)
        .with_context(|| format!("Failed to parse projected event metadata for {event_id}"))
}

fn parse_projection_report_summary(value: &str) -> Result<ProjectedSessionReportSummary> {
    if value.trim().is_empty() || value.trim() == "{}" {
        return Ok(ProjectedSessionReportSummary::default());
    }
    let summary: ProjectionReportSummaryJson =
        serde_json::from_str(value).context("Failed to parse projected session report summary")?;
    Ok(ProjectedSessionReportSummary {
        canonical_event_count: summary.canonical_event_count,
        mapping_direction: summary.mapping_direction,
        mapping_overall: summary.mapping_overall,
        preserved_count: summary.preserved_count,
        normalized_count: summary.normalized_count,
        dropped_count: summary.dropped_count,
    })
}

fn parse_projection_operation(value: &str) -> Result<ProjectionOperationKind> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .with_context(|| format!("Unknown projection operation kind: {value}"))
}

fn parse_projection_status(value: &str) -> Result<ProjectionStatus> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .with_context(|| format!("Unknown projection status: {value}"))
}

fn parse_projection_fidelity(value: &str) -> Result<ProjectionFidelity> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .with_context(|| format!("Unknown projection report item fidelity: {value}"))
}

fn parse_projection_item_scope(value: &str) -> Result<ProjectionItemScope> {
    serde_json::from_value(serde_json::Value::String(value.to_string()))
        .with_context(|| format!("Unknown projection report item scope: {value}"))
}

fn parse_projection_details(value: &str) -> Result<Option<serde_json::Value>> {
    if value.trim().is_empty() || value.trim() == "{}" {
        return Ok(None);
    }
    serde_json::from_str(value)
        .map(Some)
        .context("Failed to parse projected session report item details")
}

fn default_event_metadata(provider_id: &str, event_id: &str) -> EventMetadata {
    EventMetadata {
        source: EventSource {
            provider_id: provider_id.to_string(),
            original_id: Some(event_id.to_string()),
            original_role: None,
            phase: None,
        },
        model: None,
        usage: None,
        fidelity: MappingDisposition::Preserved,
        provider_ext: BTreeMap::new(),
    }
}

fn timestamp_from_ms(value: Option<i64>) -> DateTime<Utc> {
    value
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
}

fn source_fingerprint_for_path(path: &std::path::Path) -> Result<Option<String>> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("Failed to read session source metadata: {}", path.display())
            })
        }
    };
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(err).with_context(|| {
                format!("Failed to read session source content: {}", path.display())
            })
        }
    };
    Ok(Some(format!(
        "{}:{}:{:x}",
        modified_ms,
        metadata.len(),
        md5::compute(&bytes)
    )))
}

fn sql_bool(value: i64) -> bool {
    value != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::local_store;
    use rusqlite::params;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn reads_projected_session_snapshots() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        insert_projected_snapshot(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/project",
            20,
        );
        conn.execute(
            "INSERT INTO session_events
             (id, session_id, role, kind, visibility, source_order, stable_cursor, metadata_json)
             VALUES
             ('event-1', 'canonical-1', 'user', 'message', 'visible', 0, '0', '{}'),
             ('event-2', 'canonical-1', 'assistant', 'message', 'hidden_internal', 1, '1', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_local_state
             (session_id, display_title, hidden, pinned, preferred_targets_json, updated_at_ms)
             VALUES ('canonical-1', 'Local title', 1, 1, '[\"codex\"]', 30)",
            [],
        )
        .unwrap();

        let rows = SnapshotStore::new(&conn).list_session_snapshots().unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].canonical_session_id, "canonical-1");
        assert_eq!(rows[0].provider_session_id.as_deref(), Some("native-1"));
        assert_eq!(rows[0].display_title.as_deref(), Some("Local title"));
        assert_eq!(rows[0].workspace_dir.as_deref(), Some("/tmp/project"));
        assert_eq!(rows[0].source_path.as_deref(), Some("/tmp/source.jsonl"));
        assert_eq!(rows[0].message_count, 1);
        assert_eq!(rows[0].event_count, 2);
        assert_eq!(rows[0].turn_count, 1);
        assert_eq!(rows[0].size_bytes, Some(20));
        assert!(rows[0].hidden);
        assert!(rows[0].pinned);
        assert_eq!(rows[0].preferred_targets, vec!["codex"]);
    }

    #[test]
    fn list_session_snapshots_does_not_read_provider_source_file() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let file = NamedTempFile::new().unwrap();
        let source_path = file.path().to_string_lossy().to_string();
        insert_projected_snapshot_with_source_path(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/project",
            &source_path,
            20,
        );
        drop(file);

        let rows = SnapshotStore::new(&conn).list_session_snapshots().unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source_path.as_deref(), Some(source_path.as_str()));
        assert_eq!(rows[0].provider_session_id.as_deref(), Some("native-1"));
    }

    #[test]
    fn reads_projected_session_detail_page() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        insert_projected_snapshot(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/project",
            20,
        );
        conn.execute(
            "INSERT INTO session_turns
             (id, session_id, status, confidence, turn_order)
             VALUES ('turn-1', 'canonical-1', 'completed', 'exact', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_local_state
             (session_id, display_title, archived, hidden, pinned, notes, tags_json,
              preferred_targets_json, compressed_archive_refs_json, updated_at_ms)
             VALUES
             ('canonical-1', 'Local title', 1, 0, 1, 'note', '[\"tag-a\"]',
              '[\"codex\"]', '[\"archive-1\"]', 30)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO workspace_session_state
             (session_id, workspace_dir, hidden, pinned, preferred_targets_json, updated_at_ms)
             VALUES ('canonical-1', '/tmp/project', 1, 0, '[\"kiro\"]', 31)",
            [],
        )
        .unwrap();
        insert_event(&conn, "event-1", "turn-1", "user", 1000, 0, "First");
        insert_event(&conn, "event-2", "turn-1", "assistant", 2000, 1, "Second");
        insert_event(&conn, "event-3", "turn-1", "assistant", 3000, 2, "Third");
        insert_projection_report(&conn, "report-old", "canonical-1", 40, 1, 1, 0);
        insert_projection_report(&conn, "report-new", "canonical-1", 50, 2, 3, 1);

        let page = SnapshotStore::new(&conn)
            .get_session_detail_page("claude", "native-1", 1, Some(1))
            .unwrap()
            .unwrap();

        assert_eq!(page.canonical_session_id, "canonical-1");
        assert_eq!(page.provider_session_id.as_deref(), Some("native-1"));
        assert_eq!(page.title.as_deref(), Some("Native title"));
        assert_eq!(page.display_title.as_deref(), Some("Local title"));
        assert_eq!(page.workspace_dir.as_deref(), Some("/tmp/project"));
        assert_eq!(page.created_at_ms, Some(10));
        assert_eq!(page.last_active_at_ms, Some(20));
        assert_eq!(page.source_path.as_deref(), Some("/tmp/source.jsonl"));
        assert_eq!(page.event_count, 2);
        assert_eq!(page.message_count, 3);
        assert_eq!(page.turn_count, 1);
        assert!(page.local_state.archived);
        assert!(page.local_state.hidden);
        assert!(!page.local_state.pinned);
        assert_eq!(page.local_state.notes.as_deref(), Some("note"));
        assert_eq!(page.local_state.tags, vec!["tag-a"]);
        assert_eq!(page.local_state.preferred_targets, vec!["kiro"]);
        assert_eq!(page.local_state.compressed_archive_refs, vec!["archive-1"]);
        let report = page.projection_report.as_ref().unwrap();
        assert_eq!(report.id, "report-new");
        assert_eq!(report.operation_kind, ProjectionOperationKind::Import);
        assert_eq!(report.status, ProjectionStatus::CompletedWithLoss);
        assert_eq!(report.created_at_ms, 50);
        assert_eq!(report.summary.canonical_event_count, Some(3));
        assert_eq!(
            report.summary.mapping_direction,
            Some(MappingDirection::Import)
        );
        assert_eq!(
            report.summary.mapping_overall,
            Some(MappingDisposition::Dropped)
        );
        assert_eq!(report.summary.preserved_count, 2);
        assert_eq!(report.summary.normalized_count, 3);
        assert_eq!(report.summary.dropped_count, 1);
        assert_eq!(report.item_count, 2);
        assert_eq!(report.items.len(), 2);
        assert_eq!(report.items[0].fidelity, ProjectionFidelity::Normalized);
        assert_eq!(report.items[0].scope, ProjectionItemScope::ProviderPayload);
        assert_eq!(
            report.items[0].field_path.as_deref(),
            Some("events[0].content")
        );
        assert_eq!(
            report.items[0].reason.as_deref(),
            Some("normalized text block")
        );
        assert_eq!(
            report.items[0].details.as_ref().unwrap()["code"],
            serde_json::Value::String("normalized_text".to_string())
        );
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].id, "event-2");
        assert_eq!(page.events[0].role, EventRole::Assistant);
        assert_eq!(page.events[0].kind, SessionEventKind::Message);
        assert_eq!(page.events[0].links.turn_index, Some(0));
        assert_eq!(
            page.events[0].metadata.provider_ext["projected_turn_id"],
            serde_json::Value::String("turn-1".to_string())
        );
        match &page.events[0].blocks[0] {
            EventBlock::Text { text } => assert_eq!(text, "Second"),
            block => panic!("unexpected block: {block:?}"),
        }
    }

    #[test]
    fn get_session_detail_page_does_not_read_provider_source_file() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let file = NamedTempFile::new().unwrap();
        let source_path = file.path().to_string_lossy().to_string();
        insert_projected_snapshot_with_source_path(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            "/tmp/project",
            &source_path,
            20,
        );
        conn.execute(
            "INSERT INTO session_turns
             (id, session_id, status, confidence, turn_order)
             VALUES ('turn-1', 'canonical-1', 'completed', 'exact', 0)",
            [],
        )
        .unwrap();
        insert_event(&conn, "event-1", "turn-1", "user", 1000, 0, "Cached only");
        drop(file);

        let page = SnapshotStore::new(&conn)
            .get_session_detail_page("claude", "native-1", 0, None)
            .unwrap()
            .unwrap();

        assert_eq!(page.source_path.as_deref(), Some(source_path.as_str()));
        assert_eq!(page.events.len(), 1);
        match &page.events[0].blocks[0] {
            EventBlock::Text { text } => assert_eq!(text, "Cached only"),
            block => panic!("unexpected block: {block:?}"),
        }
    }

    #[test]
    fn refresh_session_snapshot_staleness_marks_fresh_source_not_stale() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        file.flush().unwrap();
        let fingerprint = source_fingerprint_for_path(file.path()).unwrap().unwrap();
        insert_projected_snapshot_source(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            file.path().to_str().unwrap(),
            &fingerprint,
            true,
        );

        let report = SnapshotStore::new(&conn)
            .refresh_session_snapshot_staleness()
            .unwrap();

        assert_eq!(
            report,
            SnapshotStaleScanReport {
                checked_sources: 1,
                fresh_snapshots: 1,
                stale_snapshots: 0,
                missing_sources: 0,
                unknown_sources: 0,
            }
        );
        assert!(!snapshot_stale(&conn, "canonical-1"));
    }

    #[test]
    fn refresh_session_snapshot_staleness_marks_modified_source_stale() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        file.flush().unwrap();
        let fingerprint = source_fingerprint_for_path(file.path()).unwrap().unwrap();
        insert_projected_snapshot_source(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            file.path().to_str().unwrap(),
            &fingerprint,
            false,
        );
        writeln!(file, "two").unwrap();
        file.flush().unwrap();

        let report = SnapshotStore::new(&conn)
            .refresh_session_snapshot_staleness()
            .unwrap();

        assert_eq!(report.checked_sources, 1);
        assert_eq!(report.fresh_snapshots, 0);
        assert_eq!(report.stale_snapshots, 1);
        assert_eq!(report.missing_sources, 0);
        assert!(snapshot_stale(&conn, "canonical-1"));
        let stored_fingerprint: String = conn
            .query_row(
                "SELECT source_fingerprint FROM session_snapshots WHERE session_id = 'canonical-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stored_fingerprint, fingerprint);
    }

    #[test]
    fn refresh_session_snapshot_staleness_marks_missing_source_stale() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        file.flush().unwrap();
        let source_path = file.path().to_string_lossy().to_string();
        let fingerprint = source_fingerprint_for_path(file.path()).unwrap().unwrap();
        insert_projected_snapshot_source(
            &conn,
            "canonical-1",
            "claude",
            "native-1",
            &source_path,
            &fingerprint,
            false,
        );
        drop(file);

        let report = SnapshotStore::new(&conn)
            .refresh_session_snapshot_staleness()
            .unwrap();

        assert_eq!(report.checked_sources, 0);
        assert_eq!(report.fresh_snapshots, 0);
        assert_eq!(report.stale_snapshots, 1);
        assert_eq!(report.missing_sources, 1);
        assert_eq!(report.unknown_sources, 0);
        assert!(snapshot_stale(&conn, "canonical-1"));
    }

    fn insert_projected_snapshot(
        conn: &Connection,
        session_id: &str,
        provider_id: &str,
        provider_session_id: &str,
        workspace_dir: &str,
        file_size_bytes: i64,
    ) {
        insert_projected_snapshot_with_source_path(
            conn,
            session_id,
            provider_id,
            provider_session_id,
            workspace_dir,
            "/tmp/source.jsonl",
            file_size_bytes,
        );
    }

    fn insert_projected_snapshot_with_source_path(
        conn: &Connection,
        session_id: &str,
        provider_id: &str,
        provider_session_id: &str,
        workspace_dir: &str,
        source_path: &str,
        file_size_bytes: i64,
    ) {
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, workspace_dir, file_size_bytes,
              first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 10, 10)",
            params![
                format!("source-{session_id}"),
                provider_id,
                provider_session_id,
                source_path,
                workspace_dir,
                file_size_bytes,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, workspace_dir, title,
              status, created_at_ms, last_active_at_ms, event_count, turn_count)
             VALUES (?1, ?2, ?3, ?4, ?5, 'Native title', 'completed', 10, 20, 2, 1)",
            params![
                session_id,
                provider_id,
                provider_session_id,
                format!("source-{session_id}"),
                workspace_dir,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_snapshots
             (session_id, provider_id, title, workspace_dir, status, last_active_at_ms,
              event_count, turn_count, flags_json, projection_version, stale, updated_at_ms)
             VALUES (?1, ?2, 'Native title', ?3, 'completed', 20, 2, 1,
              '{\"hidden\":false,\"pinned\":false}', 1, 0, 20)",
            params![session_id, provider_id, workspace_dir],
        )
        .unwrap();
    }

    fn insert_projected_snapshot_source(
        conn: &Connection,
        session_id: &str,
        provider_id: &str,
        provider_session_id: &str,
        source_path: &str,
        source_fingerprint: &str,
        stale: bool,
    ) {
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, ?4, 10, 10)",
            params![
                format!("source-{session_id}"),
                provider_id,
                provider_session_id,
                source_path,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, title,
              status, created_at_ms, last_active_at_ms, event_count, turn_count)
             VALUES (?1, ?2, ?3, ?4, 'Native title', 'completed', 10, 20, 2, 1)",
            params![
                session_id,
                provider_id,
                provider_session_id,
                format!("source-{session_id}"),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_snapshots
             (session_id, provider_id, title, status, last_active_at_ms, event_count, turn_count,
              flags_json, projection_version, source_fingerprint, stale, updated_at_ms)
             VALUES (?1, ?2, 'Native title', 'completed', 20, 2, 1, '{}', 1, ?3, ?4, 20)",
            params![
                session_id,
                provider_id,
                source_fingerprint,
                if stale { 1 } else { 0 }
            ],
        )
        .unwrap();
    }

    fn snapshot_stale(conn: &Connection, session_id: &str) -> bool {
        let stale: i64 = conn
            .query_row(
                "SELECT stale FROM session_snapshots WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        stale != 0
    }

    fn insert_projection_report(
        conn: &Connection,
        report_id: &str,
        session_id: &str,
        created_at_ms: i64,
        preserved_count: usize,
        normalized_count: usize,
        dropped_count: usize,
    ) {
        conn.execute(
            "INSERT INTO projection_reports
             (id, session_id, provider_id, source_id, operation_kind, projection_version,
              status, summary_json, created_at_ms)
             VALUES (?1, ?2, 'claude', ?3, 'import', 1, ?4, ?5, ?6)",
            params![
                report_id,
                session_id,
                format!("source-{session_id}"),
                if dropped_count > 0 {
                    "completed_with_loss"
                } else {
                    "succeeded"
                },
                format!(
                    "{{\"canonical_event_count\":3,\"mapping_direction\":\"import\",\"mapping_overall\":\"dropped\",\"preserved_count\":{preserved_count},\"normalized_count\":{normalized_count},\"dropped_count\":{dropped_count}}}"
                ),
                created_at_ms,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO projection_report_items
             (id, report_id, item_order, disposition, scope, field_path, reason, details_json)
             VALUES
             (?1, ?2, 0, 'normalized', 'provider_payload', 'events[0].content',
              'normalized text block', '{\"level\":\"warning\",\"code\":\"normalized_text\"}'),
             (?3, ?2, 1, 'dropped', 'provider_payload', 'events[1].meta',
              'dropped unsupported field', '{}')",
            params![
                format!("{report_id}-item-1"),
                report_id,
                format!("{report_id}-item-2"),
            ],
        )
        .unwrap();
    }

    fn insert_event(
        conn: &Connection,
        event_id: &str,
        turn_id: &str,
        role: &str,
        timestamp_ms: i64,
        source_order: i64,
        text: &str,
    ) {
        conn.execute(
            "INSERT INTO session_events
             (id, session_id, turn_id, provider_event_id, role, kind, visibility, timestamp_ms,
              source_order, stable_cursor, metadata_json)
             VALUES (?1, 'canonical-1', ?2, ?1, ?3, 'message', 'visible', ?4, ?5, ?6, ?7)",
            params![
                event_id,
                turn_id,
                role,
                timestamp_ms,
                source_order,
                source_order.to_string(),
                format!(
                    "{{\"source\":{{\"provider_id\":\"claude\",\"original_id\":\"{event_id}\"}},\"fidelity\":\"preserved\",\"provider_ext\":{{}}}}"
                ),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_event_blocks
             (id, event_id, block_order, block_kind, fidelity, content_text, content_json)
             VALUES (?1, ?2, 0, 'text', 'preserved', ?3, ?4)",
            params![
                format!("block-{event_id}"),
                event_id,
                text,
                format!("{{\"type\":\"text\",\"text\":\"{text}\"}}"),
            ],
        )
        .unwrap();
    }
}
