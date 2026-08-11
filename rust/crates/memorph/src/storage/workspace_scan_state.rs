//! Persistent per-(workspace, provider) scan state — storage only.
//!
//! This module owns the row type and CRUD. It persists exactly what callers
//! pass: `mark_scanning` records a start, `settle`/`fail` record a resolved
//! status and a caller-chosen deadline. The *policy* — which status to write,
//! how long freshness lasts, when to retry — lives in
//! [`crate::core::workspace_scan_policy`]. Keeping the dependency one-way (core
//! depends on storage, never the reverse) means storage has no tie to
//! providers or to freshness rules.

use anyhow::{Context as _, Result};
use rusqlite::{params, Connection};

/// Persisted scan lifecycle status for a (workspace, provider). The source of
/// truth for whether a provider has been scanned and what the outcome was, so
/// that a provider with zero sessions can converge: `Empty` is a stable,
/// successful result, distinct from "never scanned".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStatus {
    /// No scan has ever completed for this (workspace, provider).
    Unindexed,
    /// A scan is in flight.
    Scanning,
    /// Scan completed and discovered at least one session.
    Ready,
    /// Scan completed and discovered zero sessions. Stable success.
    Empty,
    /// Scan failed.
    Error,
}

impl ScanStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ScanStatus::Unindexed => "unindexed",
            ScanStatus::Scanning => "scanning",
            ScanStatus::Ready => "ready",
            ScanStatus::Empty => "empty",
            ScanStatus::Error => "error",
        }
    }

    fn from_str(value: &str) -> Self {
        match value {
            "scanning" => ScanStatus::Scanning,
            "ready" => ScanStatus::Ready,
            "empty" => ScanStatus::Empty,
            "error" => ScanStatus::Error,
            _ => ScanStatus::Unindexed,
        }
    }
}

/// One persisted scan-state row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderScanState {
    pub workspace_key: String,
    pub provider_id: String,
    pub status: ScanStatus,
    pub discovered_count: i64,
    pub last_scan_started_at_ms: Option<i64>,
    pub last_scan_completed_at_ms: Option<i64>,
    pub next_scan_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

impl WorkspaceProviderScanState {
    /// True when this settled state has aged past its stored deadline. A row
    /// with no deadline (e.g. `scanning`) reads as due. Whether "due" should
    /// trigger a rescan is policy, not storage.
    pub fn is_due(&self, now_ms: i64) -> bool {
        match self.next_scan_at_ms {
            Some(deadline) => now_ms >= deadline,
            None => true,
        }
    }
}

/// Typed read/write access to `workspace_provider_scan_state`.
pub struct WorkspaceScanStateStore<'a> {
    conn: &'a Connection,
}

impl<'a> WorkspaceScanStateStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    pub fn read(
        &self,
        workspace_key: &str,
        provider_id: &str,
    ) -> Result<Option<WorkspaceProviderScanState>> {
        let row = self.conn.query_row(
            "SELECT workspace_key, provider_id, status, discovered_count,
                    last_scan_started_at_ms, last_scan_completed_at_ms,
                    next_scan_at_ms, last_error
             FROM workspace_provider_scan_state
             WHERE workspace_key = ?1 AND provider_id = ?2",
            params![workspace_key, provider_id],
            |row| {
                Ok(WorkspaceProviderScanState {
                    workspace_key: row.get(0)?,
                    provider_id: row.get(1)?,
                    status: ScanStatus::from_str(row.get::<_, String>(2)?.as_str()),
                    discovered_count: row.get(3)?,
                    last_scan_started_at_ms: row.get(4)?,
                    last_scan_completed_at_ms: row.get(5)?,
                    next_scan_at_ms: row.get(6)?,
                    last_error: row.get(7)?,
                })
            },
        );
        match row {
            Ok(state) => Ok(Some(state)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(error) => Err(error).with_context(|| {
                format!("Failed to read scan state for {provider_id}@{workspace_key}")
            }),
        }
    }

    /// Record that a scan has started. Overwrites any prior row (including a
    /// stuck `scanning` entry) so the start time is accurate for timeout
    /// recovery.
    pub fn mark_scanning(&self, workspace_key: &str, provider_id: &str, now_ms: i64) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO workspace_provider_scan_state
                    (workspace_key, provider_id, status, discovered_count,
                     last_scan_started_at_ms, last_scan_completed_at_ms,
                     next_scan_at_ms, last_error, updated_at_ms)
                 VALUES
                    (?1, ?2, 'scanning', 0, ?3,
                     (SELECT last_scan_completed_at_ms
                        FROM workspace_provider_scan_state
                        WHERE workspace_key = ?1 AND provider_id = ?2),
                     NULL, NULL, ?3)
                 ON CONFLICT(workspace_key, provider_id) DO UPDATE SET
                    status = 'scanning',
                    last_scan_started_at_ms = excluded.last_scan_started_at_ms,
                    last_scan_completed_at_ms = excluded.last_scan_completed_at_ms,
                    next_scan_at_ms = NULL,
                    last_error = NULL,
                    updated_at_ms = excluded.updated_at_ms",
                params![workspace_key, provider_id, now_ms],
            )
            .with_context(|| {
                format!("Failed to record scanning state for {provider_id}@{workspace_key}")
            })?;
        Ok(())
    }

    /// Record a settled scan result. The caller chooses `status` (Ready or
    /// Empty) and the `next_scan_at_ms` deadline; storage writes them verbatim.
    pub fn settle(
        &self,
        workspace_key: &str,
        provider_id: &str,
        status: ScanStatus,
        discovered_count: i64,
        next_scan_at_ms: i64,
        now_ms: i64,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE workspace_provider_scan_state
                 SET status = ?3, discovered_count = ?4,
                     last_scan_completed_at_ms = ?5, next_scan_at_ms = ?6,
                     last_error = NULL, updated_at_ms = ?5
                 WHERE workspace_key = ?1 AND provider_id = ?2",
                params![
                    workspace_key,
                    provider_id,
                    status.as_str(),
                    discovered_count,
                    now_ms,
                    next_scan_at_ms
                ],
            )
            .with_context(|| {
                format!("Failed to settle scan state for {provider_id}@{workspace_key}")
            })?;
        Ok(())
    }

    /// Record a failed scan. The caller chooses the `next_scan_at_ms` backoff
    /// deadline; storage writes it verbatim.
    pub fn fail(
        &self,
        workspace_key: &str,
        provider_id: &str,
        error: &str,
        next_scan_at_ms: i64,
        now_ms: i64,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE workspace_provider_scan_state
                 SET status = 'error', last_scan_completed_at_ms = ?5,
                     next_scan_at_ms = ?6, last_error = ?4, updated_at_ms = ?5
                 WHERE workspace_key = ?1 AND provider_id = ?2",
                params![
                    workspace_key,
                    provider_id,
                    "",
                    error,
                    now_ms,
                    next_scan_at_ms
                ],
            )
            .with_context(|| {
                format!("Failed to record scan error for {provider_id}@{workspace_key}")
            })?;
        Ok(())
    }

    /// Drop a row (used when a workspace is forgotten). Best-effort: a missing
    /// row just means there was nothing to forget.
    pub fn forget(&self, workspace_key: &str, provider_id: &str) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM workspace_provider_scan_state
                 WHERE workspace_key = ?1 AND provider_id = ?2",
                params![workspace_key, provider_id],
            )
            .with_context(|| {
                format!("Failed to forget scan state for {provider_id}@{workspace_key}")
            })?;
        Ok(())
    }

    /// True when any provider for this workspace is mid-scan. Used by the feed
    /// revision endpoint so the client can poll faster while work is in flight
    /// and relax once everything settles.
    pub fn is_workspace_busy(&self, workspace_key: &str) -> Result<bool> {
        let busy: i64 = self.conn.query_row(
            "SELECT EXISTS(SELECT 1
                 FROM workspace_provider_scan_state
                 WHERE workspace_key = ?1 AND status = 'scanning')",
            params![workspace_key],
            |row| row.get(0),
        )?;
        Ok(busy != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::storage::local_store::configure_connection(&mut conn).unwrap();
        conn.execute_batch(
            "CREATE TABLE workspace_provider_scan_state (
                workspace_key TEXT NOT NULL,
                provider_id TEXT NOT NULL,
                status TEXT NOT NULL,
                discovered_count INTEGER NOT NULL DEFAULT 0,
                last_scan_started_at_ms INTEGER,
                last_scan_completed_at_ms INTEGER,
                next_scan_at_ms INTEGER,
                last_error TEXT,
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY (workspace_key, provider_id)
            );",
        )
        .unwrap();
        conn
    }

    #[test]
    fn read_returns_none_for_unknown_row() {
        let conn = fresh_conn();
        let store = WorkspaceScanStateStore::new(&conn);
        assert!(store.read("/ws", "claude").unwrap().is_none());
    }

    #[test]
    fn store_roundtrips_through_mark_settle_and_fail() {
        let conn = fresh_conn();
        let store = WorkspaceScanStateStore::new(&conn);
        let now = 7_000_000;

        // mark_scanning inserts a scanning row with no deadline.
        store.mark_scanning("/ws", "claude", now).unwrap();
        let scanning = store.read("/ws", "claude").unwrap().unwrap();
        assert_eq!(scanning.status, ScanStatus::Scanning);
        assert_eq!(scanning.last_scan_started_at_ms, Some(now));
        assert_eq!(scanning.next_scan_at_ms, None);

        // settle writes the caller-chosen status + deadline verbatim.
        store
            .settle(
                "/ws",
                "claude",
                ScanStatus::Empty,
                0,
                now + 120_000,
                now + 1_000,
            )
            .unwrap();
        let empty = store.read("/ws", "claude").unwrap().unwrap();
        assert_eq!(empty.status, ScanStatus::Empty);
        assert_eq!(empty.discovered_count, 0);
        assert_eq!(empty.next_scan_at_ms, Some(now + 120_000));

        store
            .settle(
                "/ws",
                "claude",
                ScanStatus::Ready,
                3,
                now + 30_000,
                now + 2_000,
            )
            .unwrap();
        let ready = store.read("/ws", "claude").unwrap().unwrap();
        assert_eq!(ready.status, ScanStatus::Ready);
        assert_eq!(ready.discovered_count, 3);

        // fail writes the caller-chosen backoff deadline verbatim.
        store
            .fail("/ws", "claude", "boom", now + 15_000, now + 3_000)
            .unwrap();
        let errored = store.read("/ws", "claude").unwrap().unwrap();
        assert_eq!(errored.status, ScanStatus::Error);
        assert_eq!(errored.last_error.as_deref(), Some("boom"));
        assert_eq!(errored.next_scan_at_ms, Some(now + 15_000));

        store.forget("/ws", "claude").unwrap();
        assert!(store.read("/ws", "claude").unwrap().is_none());
    }

    #[test]
    fn is_workspace_busy_reflects_scanning_rows() {
        let conn = fresh_conn();
        let store = WorkspaceScanStateStore::new(&conn);
        assert!(!store.is_workspace_busy("/ws").unwrap());
        store.mark_scanning("/ws", "claude", 1).unwrap();
        assert!(store.is_workspace_busy("/ws").unwrap());
        store
            .settle("/ws", "claude", ScanStatus::Ready, 1, 100, 2)
            .unwrap();
        assert!(!store.is_workspace_busy("/ws").unwrap());
    }
}
