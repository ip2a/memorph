//! Persistent per-(workspace, provider) scan state.
//!
//! Scan state is the single source of truth for whether a provider has been
//! scanned for a workspace and what the outcome was. It exists so that a
//! provider which legitimately has zero sessions in a workspace can converge:
//! `empty` is a stable, successful result, distinct from "never scanned".
//!
//! The persisted status is one of [`ScanStatus::Unindexed`] /
//! [`Scanning`] / [`Ready`] / [`Empty`] / [`Error`]. "Stale" is *not* a stored
//! status: it is derived from `next_scan_at_ms` — any settled state whose
//! `next_scan_at_ms` has passed is due for rescan. Collapsing stale into a
//! derived view avoids a second transition axis (ponytail: fewer states,
//! `decide_scan` reads `next_scan_at_ms` directly; upgrade to an explicit
//! `Stale` row only if observability needs to distinguish "fresh" from "due"
//! in stored data).

use anyhow::{Context as _, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::providers;

/// Freshness window for a settled scan, by provider scan capability.
///
/// A provider that implements a true workspace-scoped scan is cheap to rerun,
/// so its results are considered fresh for a short window. Providers that can
/// only be scanned provider-wide (then filtered) are expensive, so they get a
/// longer window. `empty` always rechecks eventually — never永久挂起, never
/// hammered. Error uses exponential backoff (see [`error_backoff_secs`]).
const NATIVE_WORKSPACE_SCAN_TTL_SECS: i64 = 30;
const PROVIDER_WIDE_FALLBACK_TTL_SECS: i64 = 300;
const EMPTY_TTL_SECS: i64 = 120;
const ERROR_INITIAL_BACKOFF_SECS: i64 = 15;
const ERROR_MAX_BACKOFF_SECS: i64 = 600;
/// A `scanning` row whose `last_scan_started_at_ms` is older than this is
/// treated as a stuck task (process died mid-scan) and replaced. Replaces the
/// old in-flight-only 300s recovery, which lived in memory and was lost on
/// restart.
pub const SCAN_TIMEOUT_SECS: i64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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
    /// True when this settled state has aged past its freshness window and a
    /// new scan is due. `scanning` rows use the timeout instead.
    fn is_due(&self, now_ms: i64) -> bool {
        match self.next_scan_at_ms {
            Some(deadline) => now_ms >= deadline,
            None => true,
        }
    }

    /// True when a `scanning` row represents a task that has almost certainly
    /// died (its process exited before recording a result).
    fn is_stuck(&self, now_ms: i64) -> bool {
        match self.last_scan_started_at_ms {
            Some(started) => now_ms.saturating_sub(started) >= SCAN_TIMEOUT_SECS * 1000,
            None => true,
        }
    }
}

/// What the feed should do for a provider given its persisted state.
///
/// `UseEmptyResult` is folded into [`ScanDecision::UseCache`]: the caller
/// distinguishes ready vs empty by reading `status`/`discovered_count`, so the
/// decision only needs to say "no scan needed". `JoinScan` is distinct from
/// `StartScan` so the caller knows not to spawn a duplicate task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanDecision {
    /// Serve cached projection; no scan needed this tick.
    UseCache,
    /// A scan should run now (none in flight, or the in-flight one is stuck).
    StartScan,
    /// A scan is already in flight; attach to it instead of spawning.
    JoinScan,
    /// Last attempt failed and the backoff window has not elapsed.
    RetryLater,
}

/// Pure decision over persisted state. Takes `now_ms` so callers (feed read
/// path, discovery scheduler) share one rule and tests can pin time.
pub fn decide_scan(
    state: Option<&WorkspaceProviderScanState>,
    refresh: bool,
    now_ms: i64,
) -> ScanDecision {
    let Some(state) = state else {
        return ScanDecision::StartScan;
    };
    match state.status {
        ScanStatus::Scanning => {
            if state.is_stuck(now_ms) {
                ScanDecision::StartScan
            } else {
                ScanDecision::JoinScan
            }
        }
        ScanStatus::Unindexed => ScanDecision::StartScan,
        ScanStatus::Ready | ScanStatus::Empty => {
            if refresh || state.is_due(now_ms) {
                ScanDecision::StartScan
            } else {
                ScanDecision::UseCache
            }
        }
        ScanStatus::Error => {
            if refresh || state.is_due(now_ms) {
                ScanDecision::StartScan
            } else {
                ScanDecision::RetryLater
            }
        }
    }
}

/// Freshness window applied after a scan settles, chosen by how the provider is
/// scanned for this workspace. Native workspace scans are cheap; everything
/// else falls back to a provider-wide scan + filter and is throttled harder.
pub fn scan_ttl_secs(provider_id: &str, status: ScanStatus) -> i64 {
    if status == ScanStatus::Empty {
        return EMPTY_TTL_SECS;
    }
    let native = providers::find_provider(provider_id)
        .map(|provider| provider.supports_workspace_scan())
        .unwrap_or(false);
    if native {
        NATIVE_WORKSPACE_SCAN_TTL_SECS
    } else {
        PROVIDER_WIDE_FALLBACK_TTL_SECS
    }
}

/// Seconds to wait before retrying after a failure. Doubles the prior interval
/// (read from the previous row) up to [`ERROR_MAX_BACKOFF_SECS`]; starts at
/// [`ERROR_INITIAL_BACKOFF_SECS`]. Manual refresh bypasses this entirely.
pub fn error_backoff_secs(prev: Option<&WorkspaceProviderScanState>) -> i64 {
    let Some(prev) = prev else {
        return ERROR_INITIAL_BACKOFF_SECS;
    };
    if prev.status != ScanStatus::Error {
        return ERROR_INITIAL_BACKOFF_SECS;
    }
    let prior_interval = match (prev.last_scan_started_at_ms, prev.next_scan_at_ms) {
        (Some(started), Some(deadline)) => {
            // Both columns are epoch milliseconds; the interval between them is
            // the previous backoff window. Convert to seconds before scaling.
            ((deadline - started).max(0) / 1000).max(ERROR_INITIAL_BACKOFF_SECS)
        }
        _ => ERROR_INITIAL_BACKOFF_SECS,
    };
    (prior_interval * 2).min(ERROR_MAX_BACKOFF_SECS)
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
        let row = self
            .conn
            .query_row(
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
    pub fn mark_scanning(
        &self,
        workspace_key: &str,
        provider_id: &str,
        now_ms: i64,
    ) -> Result<()> {
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

    /// Record a settled scan result (success with N sessions, or zero).
    pub fn settle(
        &self,
        workspace_key: &str,
        provider_id: &str,
        discovered_count: i64,
        now_ms: i64,
    ) -> Result<()> {
        let status = if discovered_count > 0 {
            ScanStatus::Ready
        } else {
            ScanStatus::Empty
        };
        let next = now_ms + scan_ttl_secs(provider_id, status) * 1000;
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
                    next
                ],
            )
            .with_context(|| {
                format!("Failed to settle scan state for {provider_id}@{workspace_key}")
            })?;
        Ok(())
    }

    /// Record a failed scan with exponential backoff.
    pub fn fail(
        &self,
        workspace_key: &str,
        provider_id: &str,
        error: &str,
        prev: Option<&WorkspaceProviderScanState>,
        now_ms: i64,
    ) -> Result<()> {
        let next = now_ms + error_backoff_secs(prev) * 1000;
        self.conn
            .execute(
                "UPDATE workspace_provider_scan_state
                 SET status = 'error', last_scan_completed_at_ms = ?5,
                     next_scan_at_ms = ?6, last_error = ?4, updated_at_ms = ?5
                 WHERE workspace_key = ?1 AND provider_id = ?2",
                params![workspace_key, provider_id, "", error, now_ms, next],
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(status: ScanStatus, next_scan_at_ms: Option<i64>, started_at: Option<i64>) -> WorkspaceProviderScanState {
        WorkspaceProviderScanState {
            workspace_key: "/ws".into(),
            provider_id: "claude".into(),
            status,
            discovered_count: 0,
            last_scan_started_at_ms: started_at,
            last_scan_completed_at_ms: None,
            next_scan_at_ms,
            last_error: None,
        }
    }

    #[test]
    fn unindexed_and_missing_state_request_a_scan() {
        assert_eq!(decide_scan(None, false, 0), ScanDecision::StartScan);
        assert_eq!(
            decide_scan(Some(&state(ScanStatus::Unindexed, None, None)), false, 0),
            ScanDecision::StartScan
        );
    }

    #[test]
    fn scanning_joins_until_it_times_out() {
        let now = 1_000_000;
        let fresh = state(ScanStatus::Scanning, None, Some(now - 5_000));
        assert_eq!(decide_scan(Some(&fresh), false, now), ScanDecision::JoinScan);

        let stuck = state(ScanStatus::Scanning, None, Some(now - (SCAN_TIMEOUT_SECS + 1) * 1000));
        assert_eq!(decide_scan(Some(&stuck), false, now), ScanDecision::StartScan);
    }

    #[test]
    fn ready_and_empty_converge_until_due() {
        let now = 10_000;
        let fresh = state(ScanStatus::Empty, Some(now + 60_000), None);
        assert_eq!(decide_scan(Some(&fresh), false, now), ScanDecision::UseCache);

        let due = state(ScanStatus::Empty, Some(now - 1_000), None);
        assert_eq!(decide_scan(Some(&due), false, now), ScanDecision::StartScan);

        let ready_fresh = state(ScanStatus::Ready, Some(now + 60_000), None);
        assert_eq!(
            decide_scan(Some(&ready_fresh), false, now),
            ScanDecision::UseCache
        );
    }

    #[test]
    fn refresh_ignores_freshness_and_backoff() {
        let now = 5_000;
        let fresh = state(ScanStatus::Ready, Some(now + 60_000), None);
        assert_eq!(decide_scan(Some(&fresh), true, now), ScanDecision::StartScan);

        let mut cooling = state(ScanStatus::Error, Some(now + 60_000), None);
        cooling.last_scan_started_at_ms = Some(now - 1_000);
        assert_eq!(decide_scan(Some(&cooling), true, now), ScanDecision::StartScan);
    }

    #[test]
    fn error_backoff_asks_to_retry_until_due() {
        let now = 5_000;
        let mut err = state(ScanStatus::Error, Some(now + 60_000), None);
        err.last_scan_started_at_ms = Some(now - 1_000);
        assert_eq!(decide_scan(Some(&err), false, now), ScanDecision::RetryLater);

        let due = state(ScanStatus::Error, Some(now - 1_000), Some(now - 1_000));
        assert_eq!(decide_scan(Some(&due), false, now), ScanDecision::StartScan);
    }

    #[test]
    fn backoff_doubles_up_to_max() {
        assert_eq!(error_backoff_secs(None), ERROR_INITIAL_BACKOFF_SECS);

        let ready = state(ScanStatus::Ready, None, None);
        assert_eq!(error_backoff_secs(Some(&ready)), ERROR_INITIAL_BACKOFF_SECS);

        let mut first_error = state(ScanStatus::Error, None, None);
        first_error.last_scan_started_at_ms = Some(0);
        first_error.next_scan_at_ms = Some(ERROR_INITIAL_BACKOFF_SECS * 1000);
        assert_eq!(
            error_backoff_secs(Some(&first_error)),
            ERROR_INITIAL_BACKOFF_SECS * 2
        );

        let mut big = state(ScanStatus::Error, None, None);
        big.last_scan_started_at_ms = Some(0);
        big.next_scan_at_ms = Some((ERROR_MAX_BACKOFF_SECS + 100) * 1000);
        assert_eq!(error_backoff_secs(Some(&big)), ERROR_MAX_BACKOFF_SECS);
    }

    #[test]
    fn store_roundtrips_through_mark_settle_and_fail() {
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

        let store = WorkspaceScanStateStore::new(&conn);
        let now = 7_000_000;

        // Unknown provider/workspace → None → decision starts a scan.
        assert!(store.read("/ws", "claude").unwrap().is_none());

        // mark_scanning inserts the row, preserving a NULL completed time.
        store.mark_scanning("/ws", "claude", now).unwrap();
        let scanning = store.read("/ws", "claude").unwrap().unwrap();
        assert_eq!(scanning.status, ScanStatus::Scanning);
        assert_eq!(scanning.last_scan_started_at_ms, Some(now));
        assert_eq!(scanning.next_scan_at_ms, None);

        // settle(0) → empty, a stable success with a future deadline.
        store.settle("/ws", "claude", 0, now + 1_000).unwrap();
        let empty = store.read("/ws", "claude").unwrap().unwrap();
        assert_eq!(empty.status, ScanStatus::Empty);
        assert_eq!(empty.discovered_count, 0);
        assert!(empty.next_scan_at_ms.unwrap() > now + 1_000);
        // Empty must NOT be re-scanned immediately — convergence.
        assert_eq!(
            decide_scan(Some(&empty), false, now + 2_000),
            ScanDecision::UseCache
        );

        // settle(N) → ready.
        store.settle("/ws", "claude", 3, now + 2_000).unwrap();
        let ready = store.read("/ws", "claude").unwrap().unwrap();
        assert_eq!(ready.status, ScanStatus::Ready);
        assert_eq!(ready.discovered_count, 3);

        // fail → error + backoff; retry asked until due.
        store
            .fail("/ws", "claude", "boom", Some(&ready), now + 3_000)
            .unwrap();
        let errored = store.read("/ws", "claude").unwrap().unwrap();
        assert_eq!(errored.status, ScanStatus::Error);
        assert_eq!(errored.last_error.as_deref(), Some("boom"));
        assert_eq!(
            decide_scan(Some(&errored), false, now + 3_000),
            ScanDecision::RetryLater
        );

        // forget removes the row.
        store.forget("/ws", "claude").unwrap();
        assert!(store.read("/ws", "claude").unwrap().is_none());
    }
}
