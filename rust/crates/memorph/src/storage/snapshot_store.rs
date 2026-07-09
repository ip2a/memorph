use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Deserialize;

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

pub struct SnapshotStore<'a> {
    conn: &'a Connection,
}

impl<'a> SnapshotStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
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
                    ss.event_count,
                    ss.turn_count,
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

fn sql_bool(value: i64) -> bool {
    value != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::local_store;
    use rusqlite::params;

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

    fn insert_projected_snapshot(
        conn: &Connection,
        session_id: &str,
        provider_id: &str,
        provider_session_id: &str,
        workspace_dir: &str,
        file_size_bytes: i64,
    ) {
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, workspace_dir, file_size_bytes,
              first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, ?2, ?3, '/tmp/source.jsonl', ?4, ?5, 10, 10)",
            params![
                format!("source-{session_id}"),
                provider_id,
                provider_session_id,
                workspace_dir,
                file_size_bytes,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, workspace_dir, title,
              status, last_active_at_ms, event_count, turn_count)
             VALUES (?1, ?2, ?3, ?4, ?5, 'Native title', 'completed', 20, 2, 1)",
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
}
