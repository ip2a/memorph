//! Background discovery of new or changed external sessions.
//!
//! The feed read path scans on demand when a workspace is opened. This module
//! is the other half: a periodic pass that re-scans providers whose freshness
//! window has expired, so sessions an external agent creates after the home
//! view is already open appear without the user re-entering or refreshing.
//!
//! Discovery shares the feed's scan + settlement path
//! ([`ensure_provider_scan`]), so a discovered session flows through the same
//! projection → scan-state → feed-revision pipeline and reaches the client via
//! the revision poll. Only workspaces the feed has already touched (those with
//! a `workspace_provider_scan_state` row) are scheduled — a workspace that was
//! never opened is never polled.

use crate::core::workspace_session_feed::ensure_provider_scan;
use crate::providers;
use crate::storage::{
    activity_store::ActivityActor,
    local_store,
    workspace_scan_state::{decide_scan, ScanDecision, WorkspaceScanStateStore},
};
use anyhow::{Context as _, Result};

/// Per-tick cap on fallback (provider-wide) scans launched by one discovery
/// pass. Native workspace scans are cheap and exempt; fallback scans enumerate
/// every provider session and are throttled.
// ponytail: per-tick cap rather than a global ongoing-concurrency semaphore;
// upgrade if scan latency under load causes memory pressure.
const MAX_FALLBACK_SPAWNS_PER_TICK: usize = 3;

/// One discovery tick. For every workspace the feed has touched, kick a
/// background scan for each provider whose persisted scan state is due (its
/// freshness window elapsed, per `decide_scan`). Returns the number of scans
/// started. Only relevant (installed or configured) providers are considered.
pub fn run_discovery_pass() -> Result<usize> {
    let conn = local_store::open_database()?;
    let store = WorkspaceScanStateStore::new(&conn);
    let now = now_ms();

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT workspace_key, provider_id FROM workspace_provider_scan_state",
        )
        .context("Failed to read scan state rows for discovery")?;
    let rows: rusqlite::Result<Vec<(String, String)>> = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .context("Failed to decode scan state rows for discovery")?
        .collect();
    let active = rows?;

    let mut started = 0usize;
    let mut fallback_started = 0usize;
    for (workspace_key, provider_id) in active {
        if !provider_is_relevant(&provider_id) {
            // Uninstalled provider: do not spawn a scan thread. It will be
            // re-checked cheaply on later ticks and scanned if it returns.
            continue;
        }
        let state = match store.read(&workspace_key, &provider_id) {
            Ok(state) => state,
            Err(error) => {
                crate::logging::error(
                    "discovery",
                    format!(
                        "Failed to read scan state for {provider_id}@{workspace_key}: {error:#}"
                    ),
                );
                continue;
            }
        };
        // decide_scan folds in TTL expiry, so a provider is due only when its
        // freshness window (native short / fallback long / empty / error
        // backoff) has elapsed.
        if !matches!(
            decide_scan(state.as_ref(), false, now),
            ScanDecision::StartScan
        ) {
            continue;
        }
        let native = providers::find_provider(&provider_id)
            .map(|provider| provider.supports_workspace_scan())
            .unwrap_or(false);
        if !native && fallback_started >= MAX_FALLBACK_SPAWNS_PER_TICK {
            continue;
        }
        ensure_provider_scan(
            std::path::PathBuf::from(&workspace_key),
            workspace_key.clone(),
            provider_id,
            ActivityActor::System,
        );
        started += 1;
        if !native {
            fallback_started += 1;
        }
    }
    Ok(started)
}

/// A provider is relevant if it is installed or has a config file present —
/// the same readiness test the feed uses to pick provider candidates.
fn provider_is_relevant(provider_id: &str) -> bool {
    let environment = crate::agent_environment::detect_provider_environment_fast(provider_id);
    environment.installed || crate::agent_environment::provider_config_path(provider_id).exists()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct TestHome {
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TestHome {
        fn new(path: &std::path::Path) -> Self {
            let guard = lock();
            crate::config::set_test_home_dir(path.to_path_buf());
            Self { _guard: guard }
        }
    }

    impl Drop for TestHome {
        fn drop(&mut self) {
            crate::config::reset_test_home_dir();
        }
    }

    #[test]
    fn run_discovery_pass_is_noop_without_scan_state() {
        let dir = tempfile::tempdir().unwrap();
        let _home = TestHome::new(dir.path());

        // The home initializes the schema; no scan_state rows exist yet.
        assert_eq!(run_discovery_pass().unwrap(), 0);
    }

    #[test]
    fn run_discovery_pass_skips_due_but_uninstalled_providers() {
        let dir = tempfile::tempdir().unwrap();
        let _home = TestHome::new(dir.path());

        // Seed a provider that settled Empty, then force its freshness window
        // into the past so decide_scan would request a rescan. The provider is
        // not installed, so discovery must skip it rather than spawn a scan.
        {
            let conn = local_store::open_database().unwrap();
            let store = WorkspaceScanStateStore::new(&conn);
            store
                .mark_scanning("/fake/workspace", "nonexistent-provider", 1_000)
                .unwrap();
            store
                .settle("/fake/workspace", "nonexistent-provider", 0, 2_000)
                .unwrap();
            conn.execute(
                "UPDATE workspace_provider_scan_state
                 SET next_scan_at_ms = 1
                 WHERE workspace_key = '/fake/workspace'
                   AND provider_id = 'nonexistent-provider'",
                [],
            )
            .unwrap();
        }

        // Due (StartScan) but irrelevant → skipped, nothing spawned.
        assert_eq!(run_discovery_pass().unwrap(), 0);
    }
}
