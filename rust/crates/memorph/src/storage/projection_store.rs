use crate::canonical::{
    EventBlock, EventRole, ImportedSession, MappingDisposition, SessionEvent, SessionEventKind,
    TurnBoundary,
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
use crate::storage::artifact_store::{
    default_event_payload_root, event_payload_content_hash, persist_event_payload_at,
    register_path_in_transaction, ArtifactManifestKind, NewArtifactManifest, EVENT_PAYLOAD_FORMAT,
    EVENT_PAYLOAD_INLINE_LIMIT_BYTES, EVENT_PAYLOAD_MIME_TYPE,
};
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, Connection, Transaction};
use serde_json::{json, Value};
use std::collections::HashMap;
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
    event_payload_root: Option<PathBuf>,
}

impl<'a> ProjectionStore<'a> {
    pub fn new(conn: &'a mut Connection) -> Self {
        Self {
            conn,
            event_payload_root: None,
        }
    }

    #[cfg(test)]
    fn with_event_payload_root(conn: &'a mut Connection, event_payload_root: PathBuf) -> Self {
        Self {
            conn,
            event_payload_root: Some(event_payload_root),
        }
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
        let turn_plan = project_turns(
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

        let event_payload_root = self
            .event_payload_root
            .clone()
            .map(Ok)
            .unwrap_or_else(default_event_payload_root)?;
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
            turn_plan.turns.len(),
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
            turn_plan.turns.len(),
            &source_file.source_cursor,
            now_ms,
        )?;

        tx.execute(
            "UPDATE artifact_manifests
             SET block_id = NULL
             WHERE artifact_kind = 'event_payload'
               AND session_id = ?1
               AND block_id IS NOT NULL",
            params![session_id],
        )
        .context("Failed to detach prior projected event payload manifests")?;
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

        write_turns(&tx, &turn_plan.turns)?;
        write_projection_report(
            &tx,
            &report_id,
            Some(&session_id),
            provider_id,
            Some(&source_file.source_id),
            imported,
            now_ms,
        )?;
        let block_count = write_events_and_blocks(
            &tx,
            &event_payload_root,
            &session_id,
            &source_file.source_id,
            provider_id,
            provider_session_id,
            &report_id,
            &ordered_events,
            &turn_plan.event_turn_ids,
        )?;

        tx.commit()
            .context("Failed to commit session projection transaction")?;

        Ok(StoredProjection {
            session_id,
            source_id: source_file.source_id,
            report_id,
            event_count: ordered_events.len(),
            block_count,
            turn_count: turn_plan.turns.len(),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectionSourceFingerprint {
    pub file_mtime_ms: i64,
    pub file_size_bytes: i64,
    pub content_hash: String,
    pub source_cursor: String,
}

pub(crate) fn projection_source_fingerprint(
    path: &Path,
) -> Result<Option<ProjectionSourceFingerprint>> {
    let file_path = projection_source_file_path(path);
    let metadata = match std::fs::metadata(&file_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read session source metadata: {}",
                    file_path.display()
                )
            })
        }
    };
    let mut bytes = match std::fs::read(&file_path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read session source content: {}",
                    file_path.display()
                )
            })
        }
    };
    let mut file_mtime_ms = metadata_modified_ms(&metadata);
    let mut file_size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);

    if has_session_source_fragment(path) {
        let mut wal_path = file_path.as_os_str().to_os_string();
        wal_path.push("-wal");
        let wal_path = PathBuf::from(wal_path);
        match std::fs::metadata(&wal_path) {
            Ok(wal_metadata) => {
                let wal_bytes = std::fs::read(&wal_path).with_context(|| {
                    format!(
                        "Failed to read session source WAL content: {}",
                        wal_path.display()
                    )
                })?;
                bytes.extend_from_slice(b"\0memorph-sqlite-wal\0");
                bytes.extend_from_slice(&(wal_bytes.len() as u64).to_le_bytes());
                bytes.extend_from_slice(&wal_bytes);
                file_mtime_ms = file_mtime_ms.max(metadata_modified_ms(&wal_metadata));
                file_size_bytes = file_size_bytes
                    .saturating_add(i64::try_from(wal_metadata.len()).unwrap_or(i64::MAX));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "Failed to read session source WAL metadata: {}",
                        wal_path.display()
                    )
                })
            }
        }
    }

    let content_hash = format!("{:x}", md5::compute(&bytes));
    let source_cursor = format!("{file_mtime_ms}:{file_size_bytes}:{content_hash}");
    Ok(Some(ProjectionSourceFingerprint {
        file_mtime_ms,
        file_size_bytes,
        content_hash,
        source_cursor,
    }))
}

fn has_session_source_fragment(path: &Path) -> bool {
    path.to_string_lossy()
        .split_once('#')
        .is_some_and(|(_, fragment)| fragment.starts_with("session="))
}

fn metadata_modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
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
        let fingerprint = projection_source_fingerprint(path)?.with_context(|| {
            format!(
                "Session source does not exist: {}",
                projection_source_file_path(path).display()
            )
        })?;
        let source_path = path.to_string_lossy().to_string();
        let source_id = stable_row_id("source", provider_id, &source_path);

        Ok(Self {
            source_id,
            provider_id: provider_id.to_string(),
            provider_session_id: provider_session_id.map(str::to_string),
            source_path,
            file_mtime_ms: fingerprint.file_mtime_ms,
            file_size_bytes: fingerprint.file_size_bytes,
            content_hash: fingerprint.content_hash,
            source_cursor: fingerprint.source_cursor,
        })
    }
}

#[derive(Debug, Clone)]
struct ProjectedTurnRow {
    id: String,
    session_id: String,
    provider_turn_id: Option<String>,
    status: TurnStatus,
    confidence: TurnConfidence,
    started_at_ms: Option<i64>,
    ended_at_ms: Option<i64>,
    start_cursor: Option<String>,
    end_cursor: Option<String>,
    turn_order: i64,
    last_event_at_ms: Option<i64>,
}

#[derive(Debug, Clone)]
struct TurnProjectionPlan {
    turns: Vec<ProjectedTurnRow>,
    event_turn_ids: Vec<Option<String>>,
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

fn project_turns(
    session_id: &str,
    ordered_events: &[(ProjectedEventKey, &SessionEvent)],
    inferred_confidence: TurnConfidence,
) -> TurnProjectionPlan {
    let mut turns: Vec<ProjectedTurnRow> = Vec::new();
    let mut exact_turns = HashMap::<String, usize>::new();
    let mut current_inferred: Option<usize> = None;
    let mut event_turn_ids = vec![None; ordered_events.len()];
    let mut next_turn_order = 0i64;

    for (event_index, (key, event)) in ordered_events.iter().enumerate() {
        if let Some(provider_turn_id) = event.links.provider_turn_id.as_deref() {
            let turn_index = if let Some(turn_index) = exact_turns.get(provider_turn_id) {
                *turn_index
            } else {
                let requested_order = event.links.turn_index.map(i64::from);
                let turn_order = requested_order
                    .filter(|order| !turns.iter().any(|turn| turn.turn_order == *order))
                    .unwrap_or(next_turn_order);
                next_turn_order = next_turn_order.max(turn_order.saturating_add(1));
                let turn_index = turns.len();
                turns.push(ProjectedTurnRow {
                    id: stable_row_id("turn", session_id, &format!("provider:{provider_turn_id}")),
                    session_id: session_id.to_string(),
                    provider_turn_id: Some(provider_turn_id.to_string()),
                    status: TurnStatus::Unknown,
                    confidence: TurnConfidence::Exact,
                    started_at_ms: None,
                    ended_at_ms: None,
                    start_cursor: None,
                    end_cursor: None,
                    turn_order,
                    last_event_at_ms: None,
                });
                exact_turns.insert(provider_turn_id.to_string(), turn_index);
                turn_index
            };
            let turn = &mut turns[turn_index];
            include_turn_event(turn, key);
            apply_turn_boundary(turn, event.links.turn_boundary, key.timestamp_ms);
            event_turn_ids[event_index] = Some(turn.id.clone());
            continue;
        }

        let starts_turn = matches!(
            canonical_event_visible_message_role(event),
            Some(EventRole::User)
        );
        if starts_turn {
            if let Some(turn_index) = current_inferred.take() {
                let turn = &mut turns[turn_index];
                if matches!(turn.status, TurnStatus::Open | TurnStatus::Unknown) {
                    turn.status = TurnStatus::Completed;
                    turn.ended_at_ms = turn.last_event_at_ms;
                }
            }
            let turn_order = next_turn_order;
            next_turn_order = next_turn_order.saturating_add(1);
            let turn_index = turns.len();
            turns.push(ProjectedTurnRow {
                id: stable_row_id("turn", session_id, &turn_order.to_string()),
                session_id: session_id.to_string(),
                provider_turn_id: None,
                status: TurnStatus::Open,
                confidence: inferred_confidence,
                started_at_ms: key.timestamp_ms,
                ended_at_ms: None,
                start_cursor: None,
                end_cursor: None,
                turn_order,
                last_event_at_ms: None,
            });
            current_inferred = Some(turn_index);
        }

        if let Some(turn_index) = current_inferred {
            let turn = &mut turns[turn_index];
            include_turn_event(turn, key);
            apply_turn_boundary(turn, event.links.turn_boundary, key.timestamp_ms);
            event_turn_ids[event_index] = Some(turn.id.clone());
            if matches!(
                event.links.turn_boundary,
                Some(TurnBoundary::Completed | TurnBoundary::Failed | TurnBoundary::Interrupted)
            ) {
                current_inferred = None;
            }
        }
    }

    turns.sort_by_key(|turn| turn.turn_order);
    TurnProjectionPlan {
        turns,
        event_turn_ids,
    }
}

fn include_turn_event(turn: &mut ProjectedTurnRow, key: &ProjectedEventKey) {
    if turn.start_cursor.is_none() {
        turn.start_cursor = Some(key.stable_cursor.clone());
    }
    turn.end_cursor = Some(key.stable_cursor.clone());
    turn.last_event_at_ms = key.timestamp_ms.or(turn.last_event_at_ms);
}

fn apply_turn_boundary(
    turn: &mut ProjectedTurnRow,
    boundary: Option<TurnBoundary>,
    timestamp_ms: Option<i64>,
) {
    match boundary {
        Some(TurnBoundary::Started) => {
            turn.status = TurnStatus::Open;
            turn.started_at_ms = turn.started_at_ms.or(timestamp_ms);
        }
        Some(TurnBoundary::Completed) => {
            turn.status = TurnStatus::Completed;
            turn.ended_at_ms = timestamp_ms.or(turn.last_event_at_ms);
        }
        Some(TurnBoundary::Failed) => {
            turn.status = TurnStatus::Failed;
            turn.ended_at_ms = timestamp_ms.or(turn.last_event_at_ms);
        }
        Some(TurnBoundary::Interrupted) => {
            turn.status = TurnStatus::Interrupted;
            turn.ended_at_ms = timestamp_ms.or(turn.last_event_at_ms);
        }
        None => {}
    }
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
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            turn.provider_turn_id.as_deref(),
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
    conn: &Transaction<'_>,
    event_payload_root: &Path,
    session_id: &str,
    source_id: &str,
    provider_id: &str,
    provider_session_id: &str,
    report_id: &str,
    ordered_events: &[(ProjectedEventKey, &SessionEvent)],
    event_turn_ids: &[Option<String>],
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
    for (event_index, (key, event)) in ordered_events.iter().enumerate() {
        let event_id = stable_row_id("event", session_id, &event.id);
        let turn_id = event_turn_ids
            .get(event_index)
            .and_then(|turn_id| turn_id.as_deref());
        let metadata_json = serde_json::to_string(&event.metadata)
            .context("Failed to encode projected event metadata")?;
        event_stmt
            .execute(params![
                event_id,
                session_id,
                turn_id,
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
            let block_bytes = block_json.as_bytes();
            let content_text = block_text_for_store(block);
            let preview = block_preview(block, content_text.as_deref(), &block_json);
            let block_id = stable_row_id("block", &event_id, &block_order.to_string());
            let content_hash = event_payload_content_hash(block_bytes);
            let store_as_artifact = block_uses_event_payload_artifact(block, block_bytes.len());
            block_stmt
                .execute(params![
                    block_id,
                    event_id,
                    block_order as i64,
                    block_kind(block),
                    enum_name(block_fidelity(block)),
                    if store_as_artifact {
                        None
                    } else {
                        content_text.as_deref()
                    },
                    if store_as_artifact {
                        None
                    } else {
                        Some(block_json.as_str())
                    },
                    preview,
                    block_bytes.len() as i64,
                    content_hash,
                    provider_path(block),
                ])
                .context("Failed to insert projected event block")?;
            if store_as_artifact {
                let persisted =
                    persist_event_payload_at(event_payload_root, &block_id, block_bytes)?;
                let stored = register_path_in_transaction(
                    conn,
                    NewArtifactManifest {
                        artifact_kind: ArtifactManifestKind::EventPayload,
                        operation_id: Some(report_id.to_string()),
                        provider_id: Some(provider_id.to_string()),
                        provider_session_id: Some(provider_session_id.to_string()),
                        session_id: Some(session_id.to_string()),
                        projection_report_id: Some(report_id.to_string()),
                        event_id: Some(event_id.clone()),
                        block_id: Some(block_id.clone()),
                        path: persisted.path,
                        mime_type: Some(EVENT_PAYLOAD_MIME_TYPE.to_string()),
                        format: Some(EVENT_PAYLOAD_FORMAT.to_string()),
                        metadata: json!({
                            "ownership": "memorph",
                            "payload_schema": "canonical_event_block",
                            "payload_version": 1,
                            "block_kind": block_kind(block),
                            "inline_limit_bytes": EVENT_PAYLOAD_INLINE_LIMIT_BYTES,
                        }),
                    },
                )?;
                if stored.content_hash != persisted.content_hash
                    || stored.byte_size != persisted.byte_size
                {
                    anyhow::bail!(
                        "Registered event payload does not match persisted block: {block_id}"
                    );
                }
            }
            block_count += 1;
        }
    }

    Ok(block_count)
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
    let mut mapping_overall = imported.report.overall;
    // Fidelity totals cover materialized events and separate mapping findings.
    for event in &imported.session.events {
        mapping_overall = mapping_overall.worst(event.metadata.fidelity);
        match projection_fidelity(event.metadata.fidelity) {
            ProjectionFidelity::Preserved => preserved_count += 1,
            ProjectionFidelity::Normalized => normalized_count += 1,
            ProjectionFidelity::Dropped => dropped_count += 1,
        }
    }
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
        "mapping_overall": enum_name(mapping_overall),
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

fn block_preview(block: &EventBlock, content_text: Option<&str>, content_json: &str) -> String {
    let text = match block {
        EventBlock::Image {
            mime_type, path, ..
        } => {
            return path
                .as_deref()
                .map(|path| format!("{mime_type}: {path}"))
                .unwrap_or_else(|| mime_type.clone());
        }
        _ => content_text.unwrap_or(content_json),
    };
    text.chars().take(240).collect()
}

fn block_uses_event_payload_artifact(block: &EventBlock, byte_size: usize) -> bool {
    byte_size > EVENT_PAYLOAD_INLINE_LIMIT_BYTES
        && matches!(
            block,
            EventBlock::ToolResult { .. }
                | EventBlock::Patch { .. }
                | EventBlock::CommandResult { .. }
                | EventBlock::File { .. }
                | EventBlock::Image { .. }
                | EventBlock::ProviderPayload { .. }
                | EventBlock::Unknown { .. }
        )
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
        MappingDirection, MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef,
        SessionContext, SessionIdentity as CanonicalIdentity, SessionProvenance,
    };
    use crate::storage::local_store;
    use crate::storage::snapshot_store::SnapshotStore;
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
    fn persists_exact_provider_turn_lifecycle_and_snapshot_fields() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut imported = imported_session();
        imported.session.identity.canonical_id = "codex-session-1".to_string();
        imported.session.provenance.primary_source.provider_id = "codex".to_string();
        imported.session.provenance.primary_source.session_id = "codex-session-1".to_string();
        imported.session.events = vec![
            event("context", EventRole::System, "context", base),
            event(
                "started",
                EventRole::System,
                "started",
                base + chrono::Duration::seconds(1),
            ),
            event(
                "user-1",
                EventRole::User,
                "Build it",
                base + chrono::Duration::seconds(2),
            ),
            event(
                "complete",
                EventRole::System,
                "complete",
                base + chrono::Duration::seconds(3),
            ),
        ];
        for event in &mut imported.session.events {
            event.metadata.source.provider_id = "codex".to_string();
            event.links.provider_turn_id = Some("turn-1".to_string());
            event.links.turn_index = Some(0);
        }
        imported.session.events[1].links.turn_boundary = Some(TurnBoundary::Started);
        imported.session.events[3].links.turn_boundary = Some(TurnBoundary::Completed);

        ProjectionStore::new(&mut conn)
            .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
            .unwrap();

        let stored: (String, String, String, i64, i64, String, String) = conn
            .query_row(
                "SELECT provider_turn_id, status, confidence, started_at_ms, ended_at_ms,
                        source_start_cursor, source_end_cursor
                 FROM session_turns",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .unwrap();
        let distinct_event_turns: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT turn_id) FROM session_events WHERE turn_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let page = SnapshotStore::new(&conn)
            .get_session_detail_page("codex", "codex-session-1", 0, None)
            .unwrap()
            .unwrap();

        assert_eq!(stored.0, "turn-1");
        assert_eq!(stored.1, "completed");
        assert_eq!(stored.2, "exact");
        assert_eq!(
            stored.3,
            (base + chrono::Duration::seconds(1)).timestamp_millis()
        );
        assert_eq!(
            stored.4,
            (base + chrono::Duration::seconds(3)).timestamp_millis()
        );
        assert_eq!(stored.5, "context");
        assert_eq!(stored.6, "complete");
        assert_eq!(distinct_event_turns, 1);
        assert_eq!(page.turns.len(), 1);
        assert_eq!(page.turns[0].provider_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(page.turns[0].status, TurnStatus::Completed);
        assert_eq!(page.turns[0].confidence, TurnConfidence::Exact);
        assert_eq!(
            page.turns[0].source_range.start_cursor.as_deref(),
            Some("context")
        );
        assert_eq!(
            page.turns[0].source_range.end_cursor.as_deref(),
            Some("complete")
        );
    }

    #[test]
    fn keeps_final_inferred_turn_open_and_closes_only_preceding_turn() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let base = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let mut imported = imported_session();
        imported.session.events = vec![
            event("user-1", EventRole::User, "First", base),
            event(
                "assistant-1",
                EventRole::Assistant,
                "First answer",
                base + chrono::Duration::seconds(1),
            ),
            event(
                "user-2",
                EventRole::User,
                "Second",
                base + chrono::Duration::seconds(2),
            ),
            event(
                "assistant-2",
                EventRole::Assistant,
                "Still running",
                base + chrono::Duration::seconds(3),
            ),
        ];

        ProjectionStore::new(&mut conn)
            .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
            .unwrap();

        let statuses = conn
            .prepare("SELECT status, ended_at_ms FROM session_turns ORDER BY turn_order")
            .unwrap()
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Option<i64>>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(statuses[0].0, "completed");
        assert_eq!(
            statuses[0].1,
            Some((base + chrono::Duration::seconds(1)).timestamp_millis())
        );
        assert_eq!(statuses[1], ("open".to_string(), None));
    }

    #[test]
    fn applies_explicit_terminal_boundary_to_inferred_turn() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let mut imported = imported_session();
        imported.session.events[1].links.turn_boundary = Some(TurnBoundary::Failed);

        ProjectionStore::new(&mut conn)
            .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
            .unwrap();

        let status: String = conn
            .query_row("SELECT status FROM session_turns", [], |row| row.get(0))
            .unwrap();
        assert_eq!(status, "failed");
    }

    #[test]
    fn persists_native_aborted_turn_as_interrupted() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let mut imported = imported_session();
        for event in &mut imported.session.events {
            event.links.provider_turn_id = Some("turn-aborted".to_string());
            event.links.turn_index = Some(0);
        }
        imported.session.events[0].links.turn_boundary = Some(TurnBoundary::Started);
        imported.session.events[1].links.turn_boundary = Some(TurnBoundary::Interrupted);

        ProjectionStore::new(&mut conn)
            .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
            .unwrap();

        let stored: (String, String, String) = conn
            .query_row(
                "SELECT provider_turn_id, status, confidence FROM session_turns",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(stored.0, "turn-aborted");
        assert_eq!(stored.1, "interrupted");
        assert_eq!(stored.2, "exact");
    }

    #[test]
    fn does_not_invent_turn_for_unrelated_lifecycle_events() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let mut imported = imported_session();
        imported.session.events = vec![event(
            "session-meta",
            EventRole::System,
            "metadata",
            Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap(),
        )];
        imported.session.events[0].kind = SessionEventKind::Lifecycle;

        ProjectionStore::new(&mut conn)
            .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
            .unwrap();

        assert_eq!(count_rows(&conn, "session_turns"), 0);
        let linked_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session_events WHERE turn_id IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(linked_events, 0);
    }

    #[test]
    fn projection_report_counts_lossless_canonical_events_as_preserved() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();

        let stored = ProjectionStore::new(&mut conn)
            .write_imported_session(
                file.path(),
                &imported_session(),
                ProviderCapabilities::default(),
            )
            .unwrap();
        let (status, summary_json): (String, String) = conn
            .query_row(
                "SELECT status, summary_json FROM projection_reports WHERE id = ?1",
                [stored.report_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let summary: Value = serde_json::from_str(&summary_json).unwrap();

        assert_eq!(status, "succeeded");
        assert_eq!(summary["canonical_event_count"], 2);
        assert_eq!(summary["mapping_overall"], "preserved");
        assert_eq!(summary["preserved_count"], 2);
        assert_eq!(summary["normalized_count"], 0);
        assert_eq!(summary["dropped_count"], 0);
    }

    #[test]
    fn projection_report_combines_event_fidelity_and_mapping_findings() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut file = NamedTempFile::new().unwrap();
        writeln!(file, "one").unwrap();
        let mut imported = imported_session();
        imported.session.events[1].metadata.fidelity = MappingDisposition::Downgraded;
        imported.report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: MappingDisposition::Normalized,
            code: "normalized_field".to_string(),
            message: "Normalized one provider field".to_string(),
            path: Some("events[1].field".to_string()),
            raw: None,
        });
        imported.report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: MappingDisposition::Unsupported,
            code: "dropped_record".to_string(),
            message: "Dropped one unsupported provider record".to_string(),
            path: Some("line:3".to_string()),
            raw: None,
        });

        let stored = ProjectionStore::new(&mut conn)
            .write_imported_session(file.path(), &imported, ProviderCapabilities::default())
            .unwrap();
        let (status, summary_json): (String, String) = conn
            .query_row(
                "SELECT status, summary_json FROM projection_reports WHERE id = ?1",
                [stored.report_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let summary: Value = serde_json::from_str(&summary_json).unwrap();

        assert_eq!(status, "completed_with_loss");
        assert_eq!(summary["mapping_overall"], "unsupported");
        assert_eq!(summary["preserved_count"], 1);
        assert_eq!(summary["normalized_count"], 2);
        assert_eq!(summary["dropped_count"], 1);
        assert_eq!(count_rows(&conn, "projection_report_items"), 2);
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
    fn stores_large_payload_blocks_as_verified_artifacts_and_reads_them_back() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let large = "x".repeat(EVENT_PAYLOAD_INLINE_LIMIT_BYTES + 1024);
        let blocks = vec![
            EventBlock::ToolResult {
                tool_call_id: "tool-1".to_string(),
                content: large.clone(),
                is_error: false,
            },
            EventBlock::Patch {
                summary: Some("large patch".to_string()),
                diff_text: Some(large.clone()),
                files: vec!["src/main.rs".to_string()],
                hash: None,
            },
            EventBlock::CommandResult {
                command: Some("build".to_string()),
                exit_code: Some(0),
                stdout: Some(large.clone()),
                stderr: None,
            },
            EventBlock::File {
                path: "attachment.txt".to_string(),
                content: Some(large.clone()),
                mime_type: Some("text/plain".to_string()),
            },
            EventBlock::Image {
                mime_type: "image/png".to_string(),
                data: Some(large.clone()),
                path: Some("image.png".to_string()),
            },
            EventBlock::ProviderPayload {
                kind: "native".to_string(),
                payload: json!({"raw": large}),
            },
        ];
        let expected = blocks
            .iter()
            .map(|block| serde_json::to_value(block).unwrap())
            .collect::<Vec<_>>();
        let mut imported = imported_session();
        imported.session.events.truncate(1);
        imported.session.events[0].blocks = blocks;

        let stored =
            ProjectionStore::with_event_payload_root(&mut conn, artifact_dir.path().to_path_buf())
                .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
                .unwrap();

        let artifact_block_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM session_event_blocks
                 WHERE artifact_id IS NOT NULL
                   AND content_json IS NULL
                   AND content_text IS NULL",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let manifest_count: i64 = conn
            .query_row(
                "SELECT COUNT(*)
                 FROM artifact_manifests
                 WHERE artifact_kind = 'event_payload'
                   AND projection_report_id = ?1",
                [stored.report_id.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        let page = SnapshotStore::new(&conn)
            .get_session_detail_page("claude", "claude-session-1", 0, None)
            .unwrap()
            .unwrap();
        let actual = page.events[0]
            .blocks
            .iter()
            .map(|block| serde_json::to_value(block).unwrap())
            .collect::<Vec<_>>();

        assert_eq!(artifact_block_count, expected.len() as i64);
        assert_eq!(manifest_count, expected.len() as i64);
        assert_eq!(actual, expected);
        let paths = conn
            .prepare(
                "SELECT path FROM artifact_manifests
                 WHERE artifact_kind = 'event_payload'",
            )
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        let artifact_root = std::fs::canonicalize(artifact_dir.path()).unwrap();
        assert!(paths.iter().all(|path| {
            let path = Path::new(path);
            path.starts_with(&artifact_root) && path.is_file()
        }));
    }

    #[test]
    fn keeps_small_payload_blocks_inline() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let mut imported = imported_session();
        imported.session.events.truncate(1);
        imported.session.events[0].blocks = vec![EventBlock::ToolResult {
            tool_call_id: "tool-1".to_string(),
            content: "small output".to_string(),
            is_error: false,
        }];

        ProjectionStore::with_event_payload_root(&mut conn, artifact_dir.path().to_path_buf())
            .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
            .unwrap();

        let (content_json, artifact_id): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT content_json, artifact_id FROM session_event_blocks",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(content_json.is_some());
        assert!(artifact_id.is_none());
        assert_eq!(count_rows(&conn, "artifact_manifests"), 0);
    }

    #[test]
    fn rejects_missing_or_changed_event_payload_without_fallback() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let mut imported = imported_session();
        imported.session.events.truncate(1);
        imported.session.events[0].blocks = vec![EventBlock::Image {
            mime_type: "image/png".to_string(),
            data: Some("x".repeat(EVENT_PAYLOAD_INLINE_LIMIT_BYTES + 1024)),
            path: None,
        }];

        ProjectionStore::with_event_payload_root(&mut conn, artifact_dir.path().to_path_buf())
            .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
            .unwrap();
        let path = PathBuf::from(
            conn.query_row(
                "SELECT path FROM artifact_manifests WHERE artifact_kind = 'event_payload'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap(),
        );

        std::fs::write(&path, b"changed").unwrap();
        let changed_error = SnapshotStore::new(&conn)
            .get_session_detail_page("claude", "claude-session-1", 0, None)
            .unwrap_err();
        assert!(format!("{changed_error:#}").contains("Event payload artifact content changed"));

        std::fs::remove_file(&path).unwrap();
        let missing_error = SnapshotStore::new(&conn)
            .get_session_detail_page("claude", "claude-session-1", 0, None)
            .unwrap_err();
        assert!(format!("{missing_error:#}").contains("Failed to read event payload artifact"));
    }

    #[test]
    fn retains_prior_payload_manifest_by_projection_report_on_reprojection() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let mut imported = imported_session();
        imported.session.events.truncate(1);
        imported.session.events[0].blocks = vec![EventBlock::ToolResult {
            tool_call_id: "tool-1".to_string(),
            content: "a".repeat(EVENT_PAYLOAD_INLINE_LIMIT_BYTES + 1024),
            is_error: false,
        }];

        let first =
            ProjectionStore::with_event_payload_root(&mut conn, artifact_dir.path().to_path_buf())
                .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
                .unwrap();
        imported.session.events[0].blocks = vec![EventBlock::ToolResult {
            tool_call_id: "tool-1".to_string(),
            content: "b".repeat(EVENT_PAYLOAD_INLINE_LIMIT_BYTES + 1024),
            is_error: false,
        }];
        let second =
            ProjectionStore::with_event_payload_root(&mut conn, artifact_dir.path().to_path_buf())
                .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
                .unwrap();

        let rows = conn
            .prepare(
                "SELECT projection_report_id, block_id, path
                 FROM artifact_manifests
                 WHERE artifact_kind = 'event_payload'
                 ORDER BY created_at_ms, id",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();

        assert_eq!(rows.len(), 2);
        let first_row = rows.iter().find(|row| row.0 == first.report_id).unwrap();
        let second_row = rows.iter().find(|row| row.0 == second.report_id).unwrap();
        assert!(first_row.1.is_none());
        assert!(second_row.1.is_some());
        assert!(Path::new(&first_row.2).is_file());
        assert!(Path::new(&second_row.2).is_file());
        assert_ne!(first_row.2, second_row.2);
    }

    #[test]
    fn retains_unregistered_blob_when_projection_transaction_fails() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER fail_event_payload_registration
             BEFORE INSERT ON artifact_manifests
             WHEN NEW.artifact_kind = 'event_payload'
             BEGIN
                 SELECT RAISE(ABORT, 'forced event payload registration failure');
             END;",
        )
        .unwrap();
        let mut source = NamedTempFile::new().unwrap();
        writeln!(source, "one").unwrap();
        let artifact_dir = tempfile::tempdir().unwrap();
        let mut imported = imported_session();
        imported.session.events.truncate(1);
        imported.session.events[0].blocks = vec![EventBlock::ProviderPayload {
            kind: "native".to_string(),
            payload: json!({
                "raw": "x".repeat(EVENT_PAYLOAD_INLINE_LIMIT_BYTES + 1024),
            }),
        }];

        let error =
            ProjectionStore::with_event_payload_root(&mut conn, artifact_dir.path().to_path_buf())
                .write_imported_session(source.path(), &imported, ProviderCapabilities::default())
                .unwrap_err();

        assert!(format!("{error:#}").contains("forced event payload registration failure"));
        assert_eq!(count_rows(&conn, "sessions"), 0);
        assert_eq!(count_rows(&conn, "artifact_manifests"), 0);
        let files = walkdir::WalkDir::new(artifact_dir.path())
            .into_iter()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_type().is_file())
            .collect::<Vec<_>>();
        assert_eq!(files.len(), 1);
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

    #[test]
    fn database_session_fingerprint_includes_wal_content() {
        let dir = tempfile::tempdir().unwrap();
        let database_path = dir.path().join("opencode.db");
        std::fs::write(&database_path, b"database").unwrap();
        let locator = PathBuf::from(format!("{}#session=ses_1", database_path.to_string_lossy()));
        let without_wal = projection_source_fingerprint(&locator).unwrap().unwrap();

        let wal_path = PathBuf::from(format!("{}-wal", database_path.to_string_lossy()));
        std::fs::write(&wal_path, b"first wal state").unwrap();
        let first_wal = projection_source_fingerprint(&locator).unwrap().unwrap();
        assert_ne!(without_wal.source_cursor, first_wal.source_cursor);
        assert_eq!(
            first_wal.file_size_bytes,
            b"database".len() as i64 + b"first wal state".len() as i64
        );

        std::fs::write(&wal_path, b"second wal state").unwrap();
        let second_wal = projection_source_fingerprint(&locator).unwrap().unwrap();
        assert_ne!(first_wal.content_hash, second_wal.content_hash);
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
