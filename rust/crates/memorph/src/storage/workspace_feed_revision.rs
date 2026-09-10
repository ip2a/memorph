//! Lightweight per-workspace feed revision.
//!
//! A monotonic counter bumped whenever the workspace's visible session list may
//! have changed (sessions added/removed/renamed, display or sort fields
//! updated, or a provider's scan status converged). The web home view polls
//! [`read`] cheaply and only refetches the full session list when the revision
//! moves, so background updates arrive without a full-list refetch on every
//! tick and without ever blocking on a provider scan.
//!
//! Bump sites live in `core` (scan settlement, projection, session mutation);
//! this module is pure storage.

use anyhow::{Context as _, Result};
use rusqlite::{params, Connection};

/// Bump the revision for a workspace by one. The first bump for a workspace
/// creates the row at revision 1. Returns the new revision.
pub fn bump(conn: &Connection, workspace_key: &str, now_ms: i64) -> Result<i64> {
    conn.execute(
        "INSERT INTO workspace_feed_revision (workspace_key, revision, updated_at_ms)
         VALUES (?1, 1, ?2)
         ON CONFLICT(workspace_key) DO UPDATE SET
            revision = revision + 1,
            updated_at_ms = excluded.updated_at_ms",
        params![workspace_key, now_ms],
    )
    .with_context(|| format!("Failed to bump feed revision for {workspace_key}"))?;
    read(conn, workspace_key).map(|entry| entry.revision)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceFeedRevision {
    pub revision: i64,
    pub updated_at_ms: i64,
}

/// Read the current revision for a workspace. A workspace that has never been
/// bumped reports revision 0 (nothing to refetch yet).
pub fn read(conn: &Connection, workspace_key: &str) -> Result<WorkspaceFeedRevision> {
    let row = conn.query_row(
        "SELECT revision, updated_at_ms FROM workspace_feed_revision
         WHERE workspace_key = ?1",
        params![workspace_key],
        |row| {
            Ok(WorkspaceFeedRevision {
                revision: row.get(0)?,
                updated_at_ms: row.get(1)?,
            })
        },
    );
    match row {
        Ok(entry) => Ok(entry),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(WorkspaceFeedRevision {
            revision: 0,
            updated_at_ms: 0,
        }),
        Err(error) => {
            Err(error).with_context(|| format!("Failed to read feed revision for {workspace_key}"))
        }
    }
}

/// Drop a workspace's revision row when the workspace itself is forgotten.
pub fn forget(conn: &Connection, workspace_key: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM workspace_feed_revision WHERE workspace_key = ?1",
        params![workspace_key],
    )
    .with_context(|| format!("Failed to forget feed revision for {workspace_key}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::storage::local_store::configure_connection(&mut conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE workspace_feed_revision (
                workspace_key TEXT PRIMARY KEY,
                revision INTEGER NOT NULL DEFAULT 0,
                updated_at_ms INTEGER NOT NULL
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn read_returns_zero_for_unknown_workspace() {
        let conn = fresh_conn();
        let entry = read(&conn, "/unknown").unwrap();
        assert_eq!(entry.revision, 0);
        assert_eq!(entry.updated_at_ms, 0);
    }

    #[test]
    fn bump_is_monotonic_and_records_time() {
        let conn = fresh_conn();
        let r1 = bump(&conn, "/ws", 1_000).unwrap();
        let r2 = bump(&conn, "/ws", 2_000).unwrap();
        let r3 = bump(&conn, "/ws", 3_000).unwrap();
        assert_eq!((r1, r2, r3), (1, 2, 3));
        let entry = read(&conn, "/ws").unwrap();
        assert_eq!(entry.revision, 3);
        assert_eq!(entry.updated_at_ms, 3_000);
    }

    #[test]
    fn bump_is_per_workspace() {
        let conn = fresh_conn();
        bump(&conn, "/a", 1).unwrap();
        bump(&conn, "/a", 1).unwrap();
        bump(&conn, "/b", 1).unwrap();
        assert_eq!(read(&conn, "/a").unwrap().revision, 2);
        assert_eq!(read(&conn, "/b").unwrap().revision, 1);
    }

    #[test]
    fn forget_drops_the_row() {
        let conn = fresh_conn();
        bump(&conn, "/ws", 1).unwrap();
        forget(&conn, "/ws").unwrap();
        assert_eq!(read(&conn, "/ws").unwrap().revision, 0);
    }
}
