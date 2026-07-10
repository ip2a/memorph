use crate::canonical::{
    EventBlock, EventRole, ImportedSession, MappingDisposition, SessionEvent, SessionEventKind,
};
use crate::provider::{
    canonical_block_text, canonical_event_is_visible_message, canonical_event_visible_message_role,
    canonical_session_title, ProviderCapabilities, StorageShape, TurnQuality,
};
use crate::session_projection::{
    EventVisibility, ProjectedEventKey, ProjectionFidelity, ProjectionOperationKind,
    ProjectionStatus, SessionIdentity, SessionIdentityInput, TurnConfidence, TurnStatus,
    SESSION_PROJECTION_VERSION,
};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredProjection {
    pub session_id: String,
    pub source_id: String,
    pub report_id: String,
    pub event_count: usize,
    pub block_count: usize,
    pub turn_count: usize,
}

pub struct ProjectionStore<'a> {
    conn: &'a mut Connection,
}

impl<'a> ProjectionStore<'a> {
    pub fn new(conn: &'a mut Connection) -> Self {
        Self { conn }
    }

    pub fn write_imported_session(
        &mut self,
        source_path: &Path,
        imported: &ImportedSession,
        capabilities: ProviderCapabilities,
    ) -> Result<StoredProjection> {
        let provider_id = imported
            .session
            .provenance
            .primary_source
            .provider_id
            .as_str();
        let provider_session_id = imported
            .session
            .provenance
            .primary_source
            .session_id
            .as_str();
        let source_path_text = source_path.to_string_lossy().to_string();
        let now_ms = Utc::now().timestamp_millis();
        let source_file =
            SourceFileProjection::read(provider_id, Some(provider_session_id), source_path)?;
        let identity = SessionIdentity::from_source(SessionIdentityInput {
            provider_id,
            provider_session_id: Some(provider_session_id),
            source_path: Some(&source_path_text),
            workspace_dir: imported.session.context.workspace_dir.as_deref(),
        })?;
        let session_id = identity.canonical_session_id.clone();
        let ordered_events = ordered_events(&imported.session.events);
        let turns = infer_turns(
            &session_id,
            &ordered_events,
            turn_confidence(capabilities.turn_quality),
        );
        let report_id = Uuid::new_v4().to_string();
        let title = canonical_session_title(&imported.session);
        let created_at_ms = imported
            .session
            .context
            .created_at
            .map(|value| value.timestamp_millis());
        let last_active_at_ms = imported
            .session
            .context
            .last_active_at
            .map(|value| value.timestamp_millis())
            .or_else(|| {
                ordered_events
                    .last()
                    .map(|(_, event)| event.timestamp.timestamp_millis())
            });

        let tx = self
            .conn
            .transaction()
            .context("Failed to start session projection transaction")?;

        upsert_source(
            &tx,
            &source_file,
            imported,
            capabilities.storage_shape,
            now_ms,
        )?;
        upsert_session(
            &tx,
            &session_id,
            &source_file.source_id,
            provider_id,
            Some(provider_session_id),
            imported.session.context.workspace_dir.as_deref(),
            &title,
            created_at_ms,
            last_active_at_ms,
            ordered_events.len(),
            turns.len(),
            now_ms,
        )?;
        upsert_aliases(&tx, &identity, &source_file.source_id, now_ms)?;
        upsert_snapshot(
            &tx,
            &session_id,
            provider_id,
            &title,
            imported.session.context.workspace_dir.as_deref(),
            last_active_at_ms,
            ordered_events.len(),
            turns.len(),
            &source_file.source_cursor,
            now_ms,
        )?;

        tx.execute(
            "DELETE FROM session_events WHERE session_id = ?1",
            params![session_id],
        )
        .context("Failed to clear projected session events")?;
        tx.execute(
            "DELETE FROM session_turns WHERE session_id = ?1",
            params![session_id],
        )
        .context("Failed to clear projected session turns")?;

        write_turns(&tx, &turns)?;
        let block_count =
            write_events_and_blocks(&tx, &session_id, &source_file.source_id, &ordered_events)?;
        write_projection_report(
            &tx,
            &report_id,
            Some(&session_id),
            provider_id,
            Some(&source_file.source_id),
            imported,
            now_ms,
        )?;

        tx.commit()
            .context("Failed to commit session projection transaction")?;

        Ok(StoredProjection {
            session_id,
            source_id: source_file.source_id,
            report_id,
            event_count: ordered_events.len(),
            block_count,
            turn_count: turns.len(),
        })
    }
}

pub fn projection_source_file_path(path: &Path) -> PathBuf {
    let path_text = path.to_string_lossy();
    let file_path = match path_text.split_once('#') {
        Some((file_path, fragment)) if fragment.starts_with("session=") => file_path,
        _ => path_text.as_ref(),
    };
    PathBuf::from(file_path)
}

#[derive(Debug, Clone)]
struct SourceFileProjection {
    source_id: String,
    provider_id: String,
    provider_session_id: Option<String>,
    source_path: String,
    file_mtime_ms: i64,
    file_size_bytes: i64,
    content_hash: String,
    source_cursor: String,
}

impl SourceFileProjection {
    fn read(provider_id: &str, provider_session_id: Option<&str>, path: &Path) -> Result<Self> {
        let file_path = projection_source_file_path(path);
        let metadata = std::fs::metadata(&file_path).with_context(|| {
            format!(
                "Failed to read session source metadata: {}",
                file_path.display()
            )
        })?;
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
            .unwrap_or(0);
        let bytes = std::fs::read(&file_path).with_context(|| {
            format!(
                "Failed to read session source content: {}",
                file_path.display()
            )
        })?;
        let content_hash = format!("{:x}", md5::compute(&bytes));
        let source_path = path.to_string_lossy().to_string();
        let source_id = stable_row_id("source", provider_id, &source_path);
        let source_cursor = format!("{}:{}:{}", modified_ms, metadata.len(), content_hash);

        Ok(Self {
            source_id,
            provider_id: provider_id.to_string(),
            provider_session_id: provider_session_id.map(str::to_string),
            source_path,
            file_mtime_ms: modified_ms,
            file_size_bytes: metadata.len().min(i64::MAX as u64) as i64,
            content_hash,
            source_cursor,
        })
    }
}

#[derive(Debug, Clone)]
struct ProjectedTurnRow {
    id: String,
    session_id: String,
    status: TurnStatus,
    confidence: TurnConfidence,
    started_at_ms: Option<i64>,
    ended_at_ms: Option<i64>,
    start_cursor: Option<String>,
    end_cursor: Option<String>,
    turn_order: i64,
}

fn ordered_events(events: &[SessionEvent]) -> Vec<(ProjectedEventKey, &SessionEvent)> {
    let mut ordered = events
        .iter()
        .enumerate()
        .map(|(idx, event)| {
            let stable_cursor = event
                .metadata
                .source
                .original_id
                .as_deref()
                .unwrap_or(event.id.as_str());
            (
                ProjectedEventKey::new(
                    Some(event.timestamp.timestamp_millis()),
                    idx as i64,
                    stable_cursor,
                ),
                event,
            )
        })
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    ordered
}

fn infer_turns(
    session_id: &str,
    ordered_events: &[(ProjectedEventKey, &SessionEvent)],
    confidence: TurnConfidence,
) -> Vec<ProjectedTurnRow> {
    let mut turns = Vec::new();
    let mut current: Option<ProjectedTurnRow> = None;

    for (key, event) in ordered_events {
        let starts_turn = matches!(
            canonical_event_visible_message_role(event),
            Some(EventRole::User)
        );
        if starts_turn {
            if let Some(turn) = current.take() {
                turns.push(turn);
            }
            let turn_order = turns.len() as i64;
            current = Some(ProjectedTurnRow {
                id: stable_row_id("turn", session_id, &turn_order.to_string()),
                session_id: session_id.to_string(),
                status: TurnStatus::Completed,
                confidence,
                started_at_ms: key.timestamp_ms,
                ended_at_ms: key.timestamp_ms,
                start_cursor: Some(key.stable_cursor.clone()),
                end_cursor: Some(key.stable_cursor.clone()),
                turn_order,
            });
        } else if let Some(turn) = current.as_mut() {
            turn.ended_at_ms = key.timestamp_ms.or(turn.ended_at_ms);
            turn.end_cursor = Some(key.stable_cursor.clone());
        }
    }

    if let Some(turn) = current {
        turns.push(turn);
    }
    if turns.is_empty() && !ordered_events.is_empty() {
        let first = &ordered_events[0].0;
        let last = &ordered_events[ordered_events.len() - 1].0;
        turns.push(ProjectedTurnRow {
            id: stable_row_id("turn", session_id, "0"),
            session_id: session_id.to_string(),
            status: TurnStatus::Completed,
            confidence,
            started_at_ms: first.timestamp_ms,
            ended_at_ms: last.timestamp_ms,
            start_cursor: Some(first.stable_cursor.clone()),
            end_cursor: Some(last.stable_cursor.clone()),
            turn_order: 0,
        });
    }
    turns
}

fn upsert_source(
    conn: &Connection,
    source: &SourceFileProjection,
    imported: &ImportedSession,
    storage_shape: StorageShape,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO session_sources
         (id, provider_id, provider_session_id, source_path, workspace_dir, storage_shape,
          file_mtime_ms, file_size_bytes, content_hash, source_cursor, scan_generation,
          provider_schema_version, first_seen_at_ms, last_seen_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 1, ?11, ?12, ?12)
         ON CONFLICT(provider_id, source_path) DO UPDATE SET
          provider_session_id = excluded.provider_session_id,
          workspace_dir = excluded.workspace_dir,
          storage_shape = excluded.storage_shape,
          file_mtime_ms = excluded.file_mtime_ms,
          file_size_bytes = excluded.file_size_bytes,
          content_hash = excluded.content_hash,
          source_cursor = excluded.source_cursor,
          scan_generation = session_sources.scan_generation + 1,
          provider_schema_version = excluded.provider_schema_version,
          last_seen_at_ms = excluded.last_seen_at_ms",
        params![
            source.source_id,
            source.provider_id,
            source.provider_session_id.as_deref(),
            source.source_path,
            imported.session.context.workspace_dir.as_deref(),
            enum_name(storage_shape),
            source.file_mtime_ms,
            source.file_size_bytes,
            source.content_hash,
            source.source_cursor,
            imported.session.schema.version.to_string(),
            now_ms,
        ],
    )
    .context("Failed to upsert projected session source")?;
    Ok(())
}

fn turn_confidence(quality: TurnQuality) -> TurnConfidence {
    match quality {
        TurnQuality::Exact => TurnConfidence::Exact,
        TurnQuality::Inferred => TurnConfidence::Inferred,
        TurnQuality::Grouped => TurnConfidence::Grouped,
        TurnQuality::Unknown => TurnConfidence::Unknown,
    }
}

fn upsert_session(
    conn: &Connection,
    session_id: &str,
    source_id: &str,
    provider_id: &str,
    provider_session_id: Option<&str>,
    workspace_dir: Option<&str>,
    title: &str,
    created_at_ms: Option<i64>,
    last_active_at_ms: Option<i64>,
    event_count: usize,
    turn_count: usize,
    now_ms: i64,
) -> Result<()> {
    conn.execute(
        "INSERT INTO sessions
         (id, provider_id, provider_session_id, primary_source_id, workspace_dir, title, status,
          created_at_ms, updated_at_ms, last_active_at_ms, event_count, turn_count,
          projection_version, deleted_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'completed', ?7, ?8, ?9, ?10, ?11, ?12, NULL)
         ON CONFLICT(id) DO UPDATE SET
          provider_id = excluded.provider_id,
          provider_session_id = excluded.provider_session_id,
          primary_source_id = excluded.primary_source_id,
          workspace_dir = excluded.workspace_dir,
          title = excluded.title,
          status = excluded.status,
          created_at_ms = COALESCE(sessions.created_at_ms, excluded.created_at_ms),
          updated_at_ms = excluded.updated_at_ms,
          last_active_at_ms = excluded.last_active_at_ms,
          event_count = excluded.event_count,
          turn_count = excluded.turn_count,
          projection_version = excluded.projection_version,
          deleted_at_ms = NULL",
        params![
            session_id,
            provider_id,
            provider_session_id,
            source_id,
            workspace_dir,
            title,
            created_at_ms,
            now_ms,
            last_active_at_ms,
            event_count as i64,
            turn_count as i64,
            SESSION_PROJECTION_VERSION,
        ],
    )
    .context("Failed to upsert projected session")?;
    Ok(())
}

fn upsert_aliases(
    conn: &Connection,
    identity: &SessionIdentity,
    source_id: &str,
    now_ms: i64,
) -> Result<()> {
    for alias in &identity.aliases {
        conn.execute(
            "INSERT INTO session_aliases
             (alias_kind, alias_value, session_id, provider_id, source_id, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(alias_kind, alias_value) DO UPDATE SET
              session_id = excluded.session_id,
              provider_id = excluded.provider_id,
              source_id = excluded.source_id",
            params![
                enum_name(alias.kind),
                alias.value,
                identity.canonical_session_id,
                alias.provider_id.as_deref(),
                source_id,
                now_ms,
            ],
        )
        .context("Failed to upsert projected session alias")?;
    }
    Ok(())
}

fn upsert_snapshot(
    conn: &Connection,
    session_id: &str,
    provider_id: &str,
    title: &str,
    workspace_dir: Option<&str>,
    last_active_at_ms: Option<i64>,
    event_count: usize,
    turn_count: usize,
    source_fingerprint: &str,
    now_ms: i64,
) -> Result<()> {
    let snapshot_json = json!({
        "projection_version": SESSION_PROJECTION_VERSION,
        "source_fingerprint": source_fingerprint,
    });
    conn.execute(
        "INSERT INTO session_snapshots
         (session_id, provider_id, title, display_title, workspace_dir, status, last_active_at_ms,
          event_count, turn_count, flags_json, snapshot_json, projection_version,
          source_fingerprint, stale, updated_at_ms)
         VALUES (?1, ?2, ?3, NULL, ?4, 'completed', ?5, ?6, ?7, '{}', ?8, ?9, ?10, 0, ?11)
         ON CONFLICT(session_id) DO UPDATE SET
          provider_id = excluded.provider_id,
          title = excluded.title,
          workspace_dir = excluded.workspace_dir,
          status = excluded.status,
          last_active_at_ms = excluded.last_active_at_ms,
          event_count = excluded.event_count,
          turn_count = excluded.turn_count,
          snapshot_json = excluded.snapshot_json,
          projection_version = excluded.projection_version,
          source_fingerprint = excluded.source_fingerprint,
          stale = 0,
          updated_at_ms = excluded.updated_at_ms",
        params![
            session_id,
            provider_id,
            title,
            workspace_dir,
            last_active_at_ms,
            event_count as i64,
            turn_count as i64,
            snapshot_json.to_string(),
            SESSION_PROJECTION_VERSION,
            source_fingerprint,
            now_ms,
        ],
    )
    .context("Failed to upsert projected session snapshot")?;
    Ok(())
}

fn write_turns(conn: &Connection, turns: &[ProjectedTurnRow]) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "INSERT INTO session_turns
             (id, session_id, provider_turn_id, status, confidence, started_at_ms, ended_at_ms,
              source_start_cursor, source_end_cursor, source_range_json, turn_order)
             VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )
        .context("Failed to prepare projected turn insert")?;
    for turn in turns {
        let source_range = json!({
            "start_cursor": turn.start_cursor,
            "end_cursor": turn.end_cursor,
        });
        stmt.execute(params![
            turn.id,
            turn.session_id,
            enum_name(turn.status),
            enum_name(turn.confidence),
            turn.started_at_ms,
            turn.ended_at_ms,
            turn.start_cursor.as_deref(),
            turn.end_cursor.as_deref(),
            source_range.to_string(),
            turn.turn_order,
        ])
        .context("Failed to insert projected turn")?;
    }
    Ok(())
}

fn write_events_and_blocks(
    conn: &Connection,
    session_id: &str,
    source_id: &str,
    ordered_events: &[(ProjectedEventKey, &SessionEvent)],
) -> Result<usize> {
    let mut event_stmt = conn
        .prepare(
            "INSERT INTO session_events
             (id, session_id, turn_id, provider_event_id, role, kind, visibility, timestamp_ms,
              source_order, stable_cursor, source_id, source_cursor, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        )
        .context("Failed to prepare projected event insert")?;
    let mut block_stmt = conn
        .prepare(
            "INSERT INTO session_event_blocks
             (id, event_id, block_order, block_kind, fidelity, content_text, content_json,
              artifact_id, preview, byte_size, content_hash, provider_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, ?11)",
        )
        .context("Failed to prepare projected event block insert")?;

    let mut block_count = 0;
    for (key, event) in ordered_events {
        let event_id = stable_row_id("event", session_id, &event.id);
        let turn_id = turn_order_for_event(ordered_events, key)
            .map(|turn_order| stable_row_id("turn", session_id, &turn_order.to_string()));
        let metadata_json = serde_json::to_string(&event.metadata)
            .context("Failed to encode projected event metadata")?;
        event_stmt
            .execute(params![
                event_id,
                session_id,
                turn_id.as_deref(),
                event
                    .metadata
                    .source
                    .original_id
                    .as_deref()
                    .unwrap_or(event.id.as_str()),
                enum_name(event.role),
                enum_name(event.kind),
                enum_name(event_visibility(event)),
                key.timestamp_ms,
                key.source_order,
                key.stable_cursor,
                source_id,
                key.stable_cursor,
                metadata_json,
            ])
            .context("Failed to insert projected event")?;

        for (block_order, block) in event.blocks.iter().enumerate() {
            let block_json =
                serde_json::to_string(block).context("Failed to encode projected event block")?;
            let content_text = block_text_for_store(block);
            let preview = block_preview(content_text.as_deref(), &block_json);
            block_stmt
                .execute(params![
                    stable_row_id("block", &event_id, &block_order.to_string()),
                    event_id,
                    block_order as i64,
                    block_kind(block),
                    enum_name(block_fidelity(block)),
                    content_text.as_deref(),
                    block_json,
                    preview,
                    block_json.len() as i64,
                    format!("{:x}", md5::compute(block_json.as_bytes())),
                    provider_path(block),
                ])
                .context("Failed to insert projected event block")?;
            block_count += 1;
        }
    }

    Ok(block_count)
}

fn turn_order_for_event(
    ordered_events: &[(ProjectedEventKey, &SessionEvent)],
    target_key: &ProjectedEventKey,
) -> Option<i64> {
    let mut order = -1;
    for (key, event) in ordered_events {
        if matches!(
            canonical_event_visible_message_role(event),
            Some(EventRole::User)
        ) {
            order += 1;
        }
        if key == target_key {
            return (order >= 0).then_some(order);
        }
    }
    None
}

fn write_projection_report(
    conn: &Connection,
    report_id: &str,
    session_id: Option<&str>,
    provider_id: &str,
    source_id: Option<&str>,
    imported: &ImportedSession,
    now_ms: i64,
) -> Result<()> {
    let mut preserved_count = 0;
    let mut normalized_count = 0;
    let mut dropped_count = 0;
    for issue in &imported.report.issues {
        match projection_fidelity(issue.disposition) {
            ProjectionFidelity::Preserved => preserved_count += 1,
            ProjectionFidelity::Normalized => normalized_count += 1,
            ProjectionFidelity::Dropped => dropped_count += 1,
        }
    }
    let status = if dropped_count > 0 {
        ProjectionStatus::CompletedWithLoss
    } else {
        ProjectionStatus::Succeeded
    };
    let summary = json!({
        "canonical_event_count": imported.session.events.len(),
        "mapping_direction": enum_name(imported.report.direction),
        "mapping_overall": enum_name(imported.report.overall),
        "preserved_count": preserved_count,
        "normalized_count": normalized_count,
        "dropped_count": dropped_count,
    });

    conn.execute(
        "INSERT INTO projection_reports
         (id, session_id, provider_id, source_id, operation_kind, projection_version,
          status, summary_json, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            report_id,
            session_id,
            provider_id,
            source_id,
            enum_name(ProjectionOperationKind::Import),
            SESSION_PROJECTION_VERSION,
            enum_name(status),
            summary.to_string(),
            now_ms,
        ],
    )
    .context("Failed to insert projection report")?;

    let mut stmt = conn
        .prepare(
            "INSERT INTO projection_report_items
             (id, report_id, item_order, disposition, scope, field_path, reason, details_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .context("Failed to prepare projection report item insert")?;
    for (idx, issue) in imported.report.issues.iter().enumerate() {
        let details = json!({
            "level": enum_name(issue.level),
            "code": issue.code,
            "raw": issue.raw,
        });
        stmt.execute(params![
            stable_row_id("report_item", report_id, &idx.to_string()),
            report_id,
            idx as i64,
            enum_name(projection_fidelity(issue.disposition)),
            "provider_payload",
            issue.path.as_deref(),
            issue.message,
            details.to_string(),
        ])
        .context("Failed to insert projection report item")?;
    }
    Ok(())
}

fn event_visibility(event: &SessionEvent) -> EventVisibility {
    if canonical_event_is_visible_message(event) {
        EventVisibility::Visible
    } else if matches!(event.kind, SessionEventKind::Unknown) {
        EventVisibility::Diagnostic
    } else {
        EventVisibility::HiddenInternal
    }
}

fn block_fidelity(block: &EventBlock) -> ProjectionFidelity {
    match block {
        EventBlock::Unknown { .. } => ProjectionFidelity::Normalized,
        _ => ProjectionFidelity::Preserved,
    }
}

fn projection_fidelity(disposition: MappingDisposition) -> ProjectionFidelity {
    match disposition {
        MappingDisposition::Preserved => ProjectionFidelity::Preserved,
        MappingDisposition::Dropped | MappingDisposition::Unsupported => {
            ProjectionFidelity::Dropped
        }
        MappingDisposition::Normalized | MappingDisposition::Downgraded => {
            ProjectionFidelity::Normalized
        }
    }
}

fn block_kind(block: &EventBlock) -> &'static str {
    match block {
        EventBlock::Text { .. } => "text",
        EventBlock::Thinking { .. } => "thinking",
        EventBlock::ToolCall { .. } => "tool_call",
        EventBlock::ToolResult { .. } => "tool_result",
        EventBlock::Patch { .. } => "patch",
        EventBlock::Command { .. } => "command",
        EventBlock::CommandResult { .. } => "command_result",
        EventBlock::File { .. } => "file",
        EventBlock::Image { .. } => "image",
        EventBlock::ProviderPayload { .. } => "provider_payload",
        EventBlock::Compressed { .. } => "compressed",
        EventBlock::Unknown { .. } => "unknown",
    }
}

fn block_text_for_store(block: &EventBlock) -> Option<String> {
    match block {
        EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. } => None,
        _ => {
            let text = canonical_block_text(block);
            (!text.trim().is_empty()).then_some(text)
        }
    }
}

fn block_preview(content_text: Option<&str>, content_json: &str) -> String {
    let text = content_text.unwrap_or(content_json);
    text.chars().take(240).collect()
}

fn provider_path(block: &EventBlock) -> Option<&str> {
    match block {
        EventBlock::File { path, .. } => Some(path.as_str()),
        EventBlock::Image { path, .. } => path.as_deref(),
        _ => None,
    }
}

fn stable_row_id(kind: &str, left: &str, right: &str) -> String {
    format!(
        "{}_{}",
        kind,
        format!(
            "{:x}",
            md5::compute(format!("{}\0{}", left, right).as_bytes())
        )
    )
}

fn enum_name<T>(value: T) -> String
where
    T: serde::Serialize,
{
    match serde_json::to_value(value).unwrap_or(Value::Null) {
        Value::String(value) => value,
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canonical::{
        CanonicalSchema, CanonicalSession, EventLinks, EventMetadata, EventSource,
        MappingDirection, MappingReport, ProviderSessionRef, SessionContext,
        SessionIdentity as CanonicalIdentity, SessionProvenance,
    };
    use crate::storage::local_store;
    use chrono::TimeZone;
    use std::collections::BTreeMap;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn writes_projection_rows_idempotently_for_same_source() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        let imported = imported_session();

        let first = ProjectionStore::new(&mut conn)
            .write_imported_session(file.path(), &imported, ProviderCapabilities::default())
            .unwrap();
        let second = ProjectionStore::new(&mut conn)
            .write_imported_session(file.path(), &imported, ProviderCapabilities::default())
            .unwrap();

        assert_eq!(first.session_id, second.session_id);
        assert_eq!(first.event_count, 2);
        assert_eq!(second.event_count, 2);
        assert_eq!(count_rows(&conn, "sessions"), 1);
        assert_eq!(count_rows(&conn, "session_sources"), 1);
        assert_eq!(count_rows(&conn, "session_snapshots"), 1);
        assert_eq!(count_rows(&conn, "session_events"), 2);
        assert_eq!(count_rows(&conn, "session_event_blocks"), 2);
        assert_eq!(count_rows(&conn, "session_turns"), 1);
        assert_eq!(count_rows(&conn, "projection_reports"), 2);
    }

    #[test]
    fn stores_visibility_and_block_payload_shape() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();

        ProjectionStore::new(&mut conn)
            .write_imported_session(
                file.path(),
                &imported_session(),
                ProviderCapabilities::default(),
            )
            .unwrap();

        let visibility: String = conn
            .query_row(
                "SELECT visibility FROM session_events WHERE role = 'user'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let block_kind: String = conn
            .query_row(
                "SELECT block_kind FROM session_event_blocks ORDER BY block_order LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(visibility, "visible");
        assert_eq!(block_kind, "text");
    }

    #[test]
    fn stores_cataloged_storage_shape_and_turn_quality() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        let capabilities = ProviderCapabilities {
            storage_shape: StorageShape::Sqlite,
            turn_quality: TurnQuality::Exact,
            ..ProviderCapabilities::default()
        };

        ProjectionStore::new(&mut conn)
            .write_imported_session(file.path(), &imported_session(), capabilities)
            .unwrap();

        let storage_shape: String = conn
            .query_row("SELECT storage_shape FROM session_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        let confidence: String = conn
            .query_row("SELECT confidence FROM session_turns", [], |row| row.get(0))
            .unwrap();

        assert_eq!(storage_shape, "sqlite");
        assert_eq!(confidence, "exact");
    }

    #[test]
    fn projection_source_file_path_only_strips_session_fragments() {
        assert_eq!(
            projection_source_file_path(Path::new("/tmp/opencode.db#session=ses_1")),
            PathBuf::from("/tmp/opencode.db")
        );
        assert_eq!(
            projection_source_file_path(Path::new("/tmp/session#1.jsonl")),
            PathBuf::from("/tmp/session#1.jsonl")
        );
    }

    fn count_rows(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    fn imported_session() -> ImportedSession {
        let timestamp = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        ImportedSession {
            session: CanonicalSession {
                schema: CanonicalSchema::default(),
                identity: CanonicalIdentity {
                    canonical_id: "claude-session-1".to_string(),
                    source_title: Some("Build it".to_string()),
                },
                provenance: SessionProvenance {
                    imported_at: timestamp,
                    imported_by: Some("test".to_string()),
                    primary_source: ProviderSessionRef {
                        provider_id: "claude".to_string(),
                        session_id: "claude-session-1".to_string(),
                        source_path: None,
                    },
                    aliases: Vec::new(),
                },
                context: SessionContext {
                    workspace_dir: Some("/tmp/project".to_string()),
                    created_at: Some(timestamp),
                    last_active_at: Some(timestamp),
                    tags: Vec::new(),
                },
                events: vec![
                    event("user-1", EventRole::User, "Build it", timestamp),
                    event("assistant-1", EventRole::Assistant, "Done", timestamp),
                ],
                artifacts: Vec::new(),
                extensions: BTreeMap::new(),
            },
            report: MappingReport::new("claude", MappingDirection::Import),
        }
    }

    fn event(
        id: &str,
        role: EventRole,
        text: &str,
        timestamp: chrono::DateTime<Utc>,
    ) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind: SessionEventKind::Message,
            role,
            timestamp,
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
}
