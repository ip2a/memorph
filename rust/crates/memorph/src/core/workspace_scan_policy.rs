//! Scan scheduling policy: when a (workspace, provider) should be rescanned,
//! and how long a settled result stays fresh.
//!
//! This is the domain layer over [`WorkspaceProviderScanState`]: it reads the
//! persisted row and the provider's scan capability, and answers scheduling
//! questions. Storage ([`crate::storage::workspace_scan_state`]) only persists
//! what it is told; this module decides what that should be, so storage has no
//! dependency on providers or on freshness policy.

use crate::providers;
use crate::storage::workspace_scan_state::{ScanStatus, WorkspaceProviderScanState};

/// Freshness window for a settled scan, by provider scan capability.
///
/// A provider that implements a true workspace-scoped scan is cheap to rerun,
/// so its results stay fresh for a short window. Providers that can only be
/// scanned provider-wide (then filtered) are expensive, so they get a longer
/// window. `empty` always rechecks eventually — never永久挂起, never hammered.
/// Error uses exponential backoff (see [`error_backoff_secs`]).
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

/// True when a `scanning` row represents a task that has almost certainly died
/// (its process exited before recording a result). Uses the policy timeout, so
/// it lives here rather than on the storage row.
pub fn is_stuck(state: &WorkspaceProviderScanState, now_ms: i64) -> bool {
    match state.last_scan_started_at_ms {
        Some(started) => now_ms.saturating_sub(started) >= SCAN_TIMEOUT_SECS * 1000,
        None => true,
    }
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
            if is_stuck(state, now_ms) {
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
/// `ERROR_INITIAL_BACKOFF_SECS`. Manual refresh bypasses this entirely.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::workspace_scan_state::WorkspaceProviderScanState;

    fn state(
        status: ScanStatus,
        next_scan_at_ms: Option<i64>,
        started_at: Option<i64>,
    ) -> WorkspaceProviderScanState {
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
        assert_eq!(
            decide_scan(Some(&fresh), false, now),
            ScanDecision::JoinScan
        );

        let stuck = state(
            ScanStatus::Scanning,
            None,
            Some(now - (SCAN_TIMEOUT_SECS + 1) * 1000),
        );
        assert_eq!(
            decide_scan(Some(&stuck), false, now),
            ScanDecision::StartScan
        );
    }

    #[test]
    fn ready_and_empty_converge_until_due() {
        let now = 10_000;
        let fresh = state(ScanStatus::Empty, Some(now + 60_000), None);
        assert_eq!(
            decide_scan(Some(&fresh), false, now),
            ScanDecision::UseCache
        );

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
        assert_eq!(
            decide_scan(Some(&fresh), true, now),
            ScanDecision::StartScan
        );

        let mut cooling = state(ScanStatus::Error, Some(now + 60_000), None);
        cooling.last_scan_started_at_ms = Some(now - 1_000);
        assert_eq!(
            decide_scan(Some(&cooling), true, now),
            ScanDecision::StartScan
        );
    }

    #[test]
    fn error_backoff_asks_to_retry_until_due() {
        let now = 5_000;
        let mut err = state(ScanStatus::Error, Some(now + 60_000), None);
        err.last_scan_started_at_ms = Some(now - 1_000);
        assert_eq!(
            decide_scan(Some(&err), false, now),
            ScanDecision::RetryLater
        );

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
}
