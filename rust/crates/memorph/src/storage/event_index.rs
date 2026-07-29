use crate::session::TurnOutcome;
use anyhow::{Context as _, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFileFingerprint {
    pub modified_ms: i64,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedSessionState {
    pub provider_id: String,
    pub session_id: String,
    pub source_path: String,
    pub file_fingerprint: SourceFileFingerprint,
    pub workspace_dir: Option<String>,
    pub created_at_ms: Option<i64>,
    pub last_active_at_ms: Option<i64>,
    pub source_title: Option<String>,
    pub event_count: usize,
    pub message_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedEventLocation {
    pub event_index: usize,
    pub byte_offset: u64,
    pub byte_length: u64,
    pub line_no: usize,
    pub provider_turn_id: Option<String>,
    pub turn_index: Option<u32>,
    pub turn_boundary: Option<TurnOutcome>,
}

pub fn database_path() -> Result<PathBuf> {
    crate::storage::local_store::database_path()
}

pub fn open_database() -> Result<Connection> {
    crate::storage::local_store::open_database()
}

pub fn source_file_fingerprint(path: &Path) -> Result<SourceFileFingerprint> {
    let metadata = std::fs::metadata(path)
        .with_context(|| format!("Failed to read source metadata: {}", path.display()))?;
    let modified_ms = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0);

    Ok(SourceFileFingerprint {
        modified_ms,
        size_bytes: metadata.len(),
    })
}

pub fn load_fresh_session_state(
    conn: &Connection,
    provider_id: &str,
    source_path: &str,
    fingerprint: SourceFileFingerprint,
) -> Result<Option<IndexedSessionState>> {
    conn.query_row(
        "SELECT provider_id, session_id, source_path, file_mtime_ms, file_size_bytes,
                workspace_dir, created_at_ms, last_active_at_ms, source_title, event_count, message_count
         FROM session_index_state
         WHERE provider_id = ?1
           AND source_path = ?2
           AND file_mtime_ms = ?3
           AND file_size_bytes = ?4",
        params![
            provider_id,
            source_path,
            fingerprint.modified_ms,
            fingerprint.size_bytes as i64
        ],
        |row| {
            Ok(IndexedSessionState {
                provider_id: row.get(0)?,
                session_id: row.get(1)?,
                source_path: row.get(2)?,
                file_fingerprint: SourceFileFingerprint {
                    modified_ms: row.get(3)?,
                    size_bytes: row.get::<_, i64>(4)? as u64,
                },
                workspace_dir: row.get(5)?,
                created_at_ms: row.get(6)?,
                last_active_at_ms: row.get(7)?,
                source_title: row.get(8)?,
                event_count: row.get::<_, i64>(9)? as usize,
                message_count: row.get::<_, i64>(10)? as usize,
            })
        },
    )
    .optional()
    .context("Failed to load session index state")
}

pub fn replace_session_index(
    conn: &mut Connection,
    state: &IndexedSessionState,
    events: &[IndexedEventLocation],
) -> Result<()> {
    let tx = conn.transaction().context("Failed to start index update")?;
    tx.execute(
        "DELETE FROM session_event_index WHERE provider_id = ?1 AND source_path = ?2",
        params![state.provider_id, state.source_path],
    )
    .context("Failed to clear stale event index")?;
    tx.execute(
        "DELETE FROM session_index_state WHERE provider_id = ?1 AND source_path = ?2",
        params![state.provider_id, state.source_path],
    )
    .context("Failed to clear stale session index state")?;
    tx.execute(
        "INSERT INTO session_index_state
         (provider_id, session_id, source_path, file_mtime_ms, file_size_bytes,
          workspace_dir, created_at_ms, last_active_at_ms, source_title, event_count, message_count, indexed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, strftime('%s','now') * 1000)",
        params![
            state.provider_id,
            state.session_id,
            state.source_path,
            state.file_fingerprint.modified_ms,
            state.file_fingerprint.size_bytes as i64,
            state.workspace_dir.as_deref(),
            state.created_at_ms,
            state.last_active_at_ms,
            state.source_title.as_deref(),
            state.event_count as i64,
            state.message_count as i64,
        ],
    )
    .context("Failed to write session index state")?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO session_event_index
                 (provider_id, session_id, source_path, file_mtime_ms, file_size_bytes, event_index,
                  byte_offset, byte_length, line_no, provider_turn_id, turn_index, turn_boundary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            )
            .context("Failed to prepare event index insert")?;
        for event in events {
            stmt.execute(params![
                state.provider_id,
                state.session_id,
                state.source_path,
                state.file_fingerprint.modified_ms,
                state.file_fingerprint.size_bytes as i64,
                event.event_index as i64,
                event.byte_offset as i64,
                event.byte_length as i64,
                event.line_no as i64,
                event.provider_turn_id.as_deref(),
                event.turn_index.map(i64::from),
                event.turn_boundary.map(turn_boundary_name),
            ])
            .context("Failed to write event index row")?;
        }
    }

    tx.commit().context("Failed to commit session index")?;
    Ok(())
}

pub fn load_event_locations(
    conn: &Connection,
    provider_id: &str,
    source_path: &str,
    fingerprint: SourceFileFingerprint,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<Vec<IndexedEventLocation>> {
    let base = "SELECT event_index, byte_offset, byte_length, line_no,
                       provider_turn_id, turn_index, turn_boundary
         FROM session_event_index
         WHERE provider_id = ?1
           AND source_path = ?2
           AND file_mtime_ms = ?3
           AND file_size_bytes = ?4
           AND event_index >= ?5
         ORDER BY event_index";
    let sql = if event_limit.is_some() {
        format!("{base} LIMIT ?6")
    } else {
        base.to_string()
    };
    let mut stmt = conn
        .prepare(&sql)
        .context("Failed to prepare event index lookup")?;
    let mut rows = if let Some(limit) = event_limit {
        stmt.query(params![
            provider_id,
            source_path,
            fingerprint.modified_ms,
            fingerprint.size_bytes as i64,
            event_offset as i64,
            limit as i64
        ])?
    } else {
        stmt.query(params![
            provider_id,
            source_path,
            fingerprint.modified_ms,
            fingerprint.size_bytes as i64,
            event_offset as i64
        ])?
    };

    let mut locations = Vec::new();
    while let Some(row) = rows.next()? {
        locations.push(IndexedEventLocation {
            event_index: row.get::<_, i64>(0)? as usize,
            byte_offset: row.get::<_, i64>(1)? as u64,
            byte_length: row.get::<_, i64>(2)? as u64,
            line_no: row.get::<_, i64>(3)? as usize,
            provider_turn_id: row.get(4)?,
            turn_index: row
                .get::<_, Option<i64>>(5)?
                .and_then(|value| u32::try_from(value).ok()),
            turn_boundary: row
                .get::<_, Option<String>>(6)?
                .as_deref()
                .map(parse_turn_boundary)
                .transpose()?,
        });
    }
    Ok(locations)
}

fn turn_boundary_name(boundary: TurnOutcome) -> &'static str {
    match boundary {
        TurnOutcome::Completed => "completed",
        TurnOutcome::Failed => "failed",
        TurnOutcome::Interrupted => "interrupted",
        TurnOutcome::Incomplete => "incomplete",
    }
}

fn parse_turn_boundary(value: &str) -> rusqlite::Result<TurnOutcome> {
    match value {
        "started" => Ok(TurnOutcome::Incomplete),
        "completed" => Ok(TurnOutcome::Completed),
        "failed" => Ok(TurnOutcome::Failed),
        "interrupted" => Ok(TurnOutcome::Interrupted),
        "incomplete" => Ok(TurnOutcome::Incomplete),
        _ => Err(rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Invalid turn boundary: {value}"),
            )
            .into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_and_load_event_index_page() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::storage::local_store::apply_schema(&mut conn).unwrap();
        let fingerprint = SourceFileFingerprint {
            modified_ms: 42,
            size_bytes: 99,
        };
        let state = IndexedSessionState {
            provider_id: "codex".to_string(),
            session_id: "session-1".to_string(),
            source_path: "/tmp/session.jsonl".to_string(),
            file_fingerprint: fingerprint,
            workspace_dir: Some("/tmp/project".to_string()),
            created_at_ms: Some(10),
            last_active_at_ms: Some(20),
            source_title: Some("Title".to_string()),
            event_count: 3,
            message_count: 2,
        };
        let events = vec![
            IndexedEventLocation {
                event_index: 0,
                byte_offset: 0,
                byte_length: 10,
                line_no: 1,
                provider_turn_id: Some("turn-1".to_string()),
                turn_index: Some(0),
                turn_boundary: Some(TurnOutcome::Incomplete),
            },
            IndexedEventLocation {
                event_index: 1,
                byte_offset: 10,
                byte_length: 12,
                line_no: 2,
                provider_turn_id: Some("turn-1".to_string()),
                turn_index: Some(0),
                turn_boundary: None,
            },
            IndexedEventLocation {
                event_index: 2,
                byte_offset: 22,
                byte_length: 8,
                line_no: 3,
                provider_turn_id: Some("turn-1".to_string()),
                turn_index: Some(0),
                turn_boundary: Some(TurnOutcome::Completed),
            },
        ];

        replace_session_index(&mut conn, &state, &events).unwrap();

        let loaded_state =
            load_fresh_session_state(&conn, "codex", "/tmp/session.jsonl", fingerprint).unwrap();
        assert_eq!(loaded_state.unwrap().event_count, 3);
        let page = load_event_locations(
            &conn,
            "codex",
            "/tmp/session.jsonl",
            fingerprint,
            1,
            Some(1),
        )
        .unwrap();
        assert_eq!(page, vec![events[1].clone()]);
    }
}
