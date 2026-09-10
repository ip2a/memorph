use super::{SessionGroup, SessionItem, SessionListFields, SessionListSort};
use crate::core::workspace_scan_policy::{self, decide_scan, ScanDecision};
use crate::core::{projection, session_management};
use crate::providers;
use crate::storage::{
    activity_store::ActivityActor,
    local_store,
    snapshot_store::SnapshotStore,
    workspace_scan_state::{ScanStatus, WorkspaceProviderScanState, WorkspaceScanStateStore},
};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Parameters for the unified workspace session feed.
///
/// The feed is designed for the home/refresh path: return whatever is already
/// projected for the workspace immediately, then trigger (or join) a
/// background per-provider scan for providers that are unindexed, stale, or
/// explicitly being refreshed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSessionFeedParams {
    pub workspace_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub providers: Vec<String>,
    #[serde(default = "default_per_provider_limit")]
    pub per_provider_limit: usize,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub fields: SessionListFields,
    #[serde(default)]
    pub sort: SessionListSort,
}

impl Default for WorkspaceSessionFeedParams {
    fn default() -> Self {
        Self {
            workspace_dir: PathBuf::new(),
            providers: Vec::new(),
            per_provider_limit: default_per_provider_limit(),
            refresh: false,
            fields: SessionListFields::WithStats,
            sort: SessionListSort::Recent,
        }
    }
}

fn default_per_provider_limit() -> usize {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FeedStateKind {
    /// All requested providers returned cached data and no refresh was requested.
    Fresh,
    /// Returned cached data and at least one provider is being rescanned in the
    /// background.
    Warming,
    /// No cached data existed for at least one provider; a background scan has
    /// been started (or is already running).
    ColdScanning,
    /// All providers returned cached data and any background refresh has
    /// finished since this response was assembled.
    Complete,
    /// One or more providers reported an error during the scan.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedState {
    pub kind: FeedStateKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Per-provider feed state, derived from persisted scan state.
///
/// `scanned` and `error` are produced by real persistent scan results (a
/// completed scan with data, and a failed scan respectively). `empty` is the
/// stable converged result for a provider that has zero sessions in the
/// workspace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFeedStateKind {
    /// A scan for this provider is currently in flight.
    Scanning,
    /// A scan completed and produced projected sessions.
    Scanned,
    /// A scan completed and discovered zero sessions (stable).
    Empty,
    /// The scan failed and is backing off.
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderFeedState {
    pub provider_id: String,
    pub kind: ProviderFeedStateKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub discovered_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSessionFeed {
    pub workspace: String,
    pub groups: Vec<SessionGroup>,
    pub feed_state: FeedState,
    pub provider_states: Vec<ProviderFeedState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refreshed_at: Option<i64>,
    pub has_more: bool,
}

/// In-process record of scans this process has spawned. Persisted scan state
/// (`workspace_scan_state`) is the source of truth; this map only dedupes
/// spawns within one process so two concurrent feed reads do not launch two
/// scans for the same (workspace, provider). On restart the persisted row —
/// not this map — drives recovery via `decide_scan`'s stuck-scanning check.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct InFlightScan {
    started_at: Instant,
    workspace_key: String,
    provider_id: String,
}

static IN_FLIGHT_SCANS: OnceLock<Mutex<HashMap<String, InFlightScan>>> = OnceLock::new();

fn in_flight_scans() -> &'static Mutex<HashMap<String, InFlightScan>> {
    IN_FLIGHT_SCANS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn in_flight_key(workspace_key: &str, provider_id: &str) -> String {
    format!("{provider_id}@{workspace_key}")
}

fn is_scanning(workspace_key: &str, provider_id: &str) -> bool {
    prune_stale_in_flight();
    in_flight_scans()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(&in_flight_key(workspace_key, provider_id))
}

fn mark_scanning_inflight(workspace_key: String, provider_id: String) {
    prune_stale_in_flight();
    in_flight_scans()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            in_flight_key(&workspace_key, &provider_id),
            InFlightScan {
                started_at: Instant::now(),
                workspace_key,
                provider_id,
            },
        );
}

fn mark_scan_complete(workspace_key: &str, provider_id: &str) {
    prune_stale_in_flight();
    in_flight_scans()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&in_flight_key(workspace_key, provider_id));
}

/// Defensive prune: a scan thread that panicked or was killed without running
/// its completion path would otherwise live here until process exit. Persisted
/// scan state has its own (longer) timeout; this only keeps this map from
/// growing without bound in a long-running process.
fn prune_stale_in_flight() {
    let Ok(mut map) = in_flight_scans().try_lock() else {
        return;
    };
    let now = Instant::now();
    map.retain(|_, scan| {
        now.duration_since(scan.started_at).as_secs()
            < workspace_scan_policy::SCAN_TIMEOUT_SECS as u64
    });
}

/// Canonical workspace key shared by the feed, persisted scan state, and feed
/// revision. Resolved identically everywhere so the lightweight revision
/// endpoint agrees with the scan path. All current providers use the default
/// normalization, so the key depends only on the resolved workspace path.
pub fn workspace_feed_key(workspace_dir: &std::path::Path) -> Option<String> {
    let resolved = workspace_dir
        .canonicalize()
        .unwrap_or_else(|_| workspace_dir.to_path_buf());
    crate::provider::default_normalized_workspace_key(Some(&resolved.to_string_lossy()))
}

/// Return a fast workspace-scoped session feed.
///
/// Behavior:
/// * Resolve the workspace directory and the relevant provider set.
/// * For each provider, read its persisted scan state and decide (via
///   [`decide_scan`]) whether to serve cached projection, start/join a scan,
///   or wait out a backoff. The decision never blocks on the provider scan;
///   cached sessions are returned immediately.
/// * Settled scans converge: a provider with zero sessions reaches `empty`
///   and is no longer rescanned every request.
pub fn workspace_session_feed(params: &WorkspaceSessionFeedParams) -> Result<WorkspaceSessionFeed> {
    let workspace_dir = params.workspace_dir.canonicalize().with_context(|| {
        format!(
            "Failed to resolve workspace: {}",
            params.workspace_dir.display()
        )
    })?;
    let workspace = workspace_dir.to_string_lossy().to_string();

    let provider_ids = if params.providers.is_empty() {
        projection::readiness_workspace_provider_ids()
            .into_iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    } else {
        params.providers.clone()
    };

    let conn = local_store::open_database()?;
    let store = SnapshotStore::new(&conn);
    let scan_state_store = WorkspaceScanStateStore::new(&conn);

    let now = crate::utils::now_ms();
    let per_provider_limit = params.per_provider_limit.max(1);
    let mut groups = Vec::with_capacity(provider_ids.len());
    let mut provider_states = Vec::with_capacity(provider_ids.len());
    let mut any_scanning = false;
    let mut any_unindexed = false;
    let mut any_error = false;
    let mut refreshed_at: Option<i64> = None;

    for provider_id in &provider_ids {
        let provider_name = providers::find_provider(provider_id)
            .map(|provider| provider.name().to_string())
            .unwrap_or_else(|| provider_id.clone());

        let workspace_key =
            session_management::normalized_workspace_key(provider_id, Some(&workspace))
                .unwrap_or_else(|| workspace.clone());
        let workspace_scopes = vec![(provider_id.clone(), workspace_key.clone())];

        let snapshots = store
            .list_session_snapshots_filtered(
                Some(&[provider_id.clone()]),
                Some(workspace_scopes.as_slice()),
                params.fields.include_stats(),
            )
            .with_context(|| {
                format!("Failed to list projected snapshots for provider {provider_id}")
            })?;

        let mut sessions: Vec<SessionItem> = snapshots
            .into_iter()
            .map(|snapshot| projection::projected_snapshot_item(&snapshot))
            .collect();
        projection::sort_session_items(&mut sessions, &params.sort);
        sessions.truncate(per_provider_limit);

        let state = scan_state_store
            .read(&workspace_key, provider_id)
            .with_context(|| format!("Failed to read scan state for provider {provider_id}"))?;
        let decision = decide_scan(state.as_ref(), params.refresh, now);

        if matches!(
            state.as_ref().map(|s| s.status),
            None | Some(ScanStatus::Unindexed)
        ) {
            any_unindexed = true;
        }

        let (kind, needs_scan) = provider_kind_for_decision(&decision, state.as_ref());

        if needs_scan {
            ensure_provider_scan(
                workspace_dir.clone(),
                workspace_key.clone(),
                provider_id.clone(),
                ActivityActor::Api,
            );
        }

        if kind == ProviderFeedStateKind::Scanning {
            any_scanning = true;
        } else if kind == ProviderFeedStateKind::Error {
            any_error = true;
        }

        provider_states.push(ProviderFeedState {
            provider_id: provider_id.clone(),
            kind,
            message: provider_message(state.as_ref(), decision),
            discovered_count: sessions.len(),
        });

        if !sessions.is_empty() {
            groups.push(SessionGroup {
                provider_id: provider_id.clone(),
                provider_name,
                sessions,
            });
        }
    }

    let feed_state = if any_scanning && any_unindexed {
        FeedState {
            kind: FeedStateKind::ColdScanning,
            message: Some("Scanning providers for workspace sessions".to_string()),
        }
    } else if any_scanning {
        FeedState {
            kind: FeedStateKind::Warming,
            message: Some("Refreshing provider sessions in background".to_string()),
        }
    } else if any_error {
        FeedState {
            kind: FeedStateKind::Error,
            message: Some("One or more providers failed to scan".to_string()),
        }
    } else if params.refresh {
        refreshed_at = Some(now);
        FeedState {
            kind: FeedStateKind::Complete,
            message: Some("Workspace session feed is up to date".to_string()),
        }
    } else {
        FeedState {
            kind: FeedStateKind::Fresh,
            message: None,
        }
    };

    let has_more = groups
        .iter()
        .any(|group| group.sessions.len() >= per_provider_limit);

    Ok(WorkspaceSessionFeed {
        workspace,
        groups,
        feed_state,
        provider_states,
        refreshed_at,
        has_more,
    })
}

/// Map a scan decision (plus the persisted state it was made from) to the
/// provider-visible feed kind. Returns whether a scan needs to be running.
fn provider_kind_for_decision(
    decision: &ScanDecision,
    state: Option<&WorkspaceProviderScanState>,
) -> (ProviderFeedStateKind, bool) {
    match decision {
        ScanDecision::StartScan | ScanDecision::JoinScan => (ProviderFeedStateKind::Scanning, true),
        ScanDecision::UseCache => {
            let kind = match state.map(|s| s.status) {
                Some(ScanStatus::Empty) => ProviderFeedStateKind::Empty,
                // Ready, or anything unexpected: a prior scan produced data.
                _ => ProviderFeedStateKind::Scanned,
            };
            (kind, false)
        }
        ScanDecision::RetryLater => (ProviderFeedStateKind::Error, false),
    }
}

fn provider_message(
    state: Option<&WorkspaceProviderScanState>,
    decision: ScanDecision,
) -> Option<String> {
    match decision {
        ScanDecision::RetryLater => state
            .and_then(|s| s.last_error.clone())
            .or_else(|| Some("Provider scan failed and is backing off".to_string())),
        ScanDecision::StartScan => {
            Some("Started scanning provider sessions for workspace".to_string())
        }
        ScanDecision::JoinScan => Some("Scanning provider sessions for workspace".to_string()),
        ScanDecision::UseCache => None,
    }
}

/// Start a background scan for (workspace, provider) unless one is already
/// running in this process. Persists the scanning state so a crash still
/// leaves a recoverable row, records the in-flight task for in-process dedup,
/// and launches the scan thread that settles the state and bumps the feed
/// revision on completion. Shared by the feed read path and the discovery
/// scheduler so both converge through one settlement point.
pub(crate) fn ensure_provider_scan(
    workspace_dir: PathBuf,
    workspace_key: String,
    provider_id: String,
    actor: ActivityActor,
) {
    if is_scanning(&workspace_key, &provider_id) {
        return;
    }
    if let Ok(conn) = local_store::open_database() {
        let store = WorkspaceScanStateStore::new(&conn);
        let _ = store.mark_scanning(&workspace_key, &provider_id, crate::utils::now_ms());
    }
    mark_scanning_inflight(workspace_key.clone(), provider_id.clone());
    spawn_provider_workspace_scan(workspace_dir, workspace_key, provider_id, actor);
}

/// Spawn a background per-provider workspace scan. On completion, persist the
/// settled result (ready/empty/error) through `complete_workspace_provider_scan`
/// so the next feed read converges instead of rescanning forever.
fn spawn_provider_workspace_scan(
    workspace_dir: PathBuf,
    workspace_key: String,
    provider_id: String,
    actor: ActivityActor,
) {
    let workspace_text = workspace_dir.to_string_lossy().to_string();
    std::thread::Builder::new()
        .name(format!(
            "memorph-feed-{}-{}",
            workspace_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("workspace"),
            provider_id
        ))
        .spawn(move || {
            let result = projection::index_workspace_sessions(&provider_id, &workspace_dir, actor);
            match &result {
                Ok(report) => {
                    crate::logging::info(
                        "workspace_session_feed",
                        format!(
                            "Provider {provider_id} workspace {} scan completed: {} discovered, {} projected",
                            workspace_dir.display(),
                            report.discovered_sessions,
                            report.projected_sessions
                        ),
                    );
                }
                Err(error) => {
                    crate::logging::error(
                        "workspace_session_feed",
                        format!(
                            "Provider {provider_id} workspace {} scan failed: {error:#}",
                            workspace_dir.display()
                        ),
                    );
                }
            }
            complete_workspace_provider_scan(&workspace_key, &provider_id, &result);
            mark_scan_complete(&workspace_key, &provider_id);
            // Suppress unused-warning while the canonicalized path stays useful
            // for diagnostics above.
            let _ = &workspace_text;
        })
        .ok();
}

/// Single settlement point for a workspace/provider scan. Writes the session
/// snapshots (done by `index_workspace_sessions`), then records the scan
/// outcome and its freshness deadline. Keeping this in one place means the
/// success, empty, and failure paths can't diverge on which bookkeeping they
/// skip — the bug that made empty providers scan forever.
fn complete_workspace_provider_scan(
    workspace_key: &str,
    provider_id: &str,
    result: &Result<projection::SessionProjectionBootstrapReport>,
) {
    let Ok(conn) = local_store::open_database() else {
        crate::logging::error(
            "workspace_session_feed",
            format!("Failed to open DB to settle scan for {provider_id}@{workspace_key}"),
        );
        return;
    };
    let store = WorkspaceScanStateStore::new(&conn);
    let now = crate::utils::now_ms();
    let prev = store.read(workspace_key, provider_id).ok().flatten();
    let discovered = result
        .as_ref()
        .map(|report| report.discovered_sessions as i64)
        .unwrap_or(0);
    let new_status = match result {
        Ok(_) => {
            if discovered > 0 {
                ScanStatus::Ready
            } else {
                ScanStatus::Empty
            }
        }
        Err(_) => ScanStatus::Error,
    };
    // Storage writes exactly the status + deadline the policy picks, so this
    // settlement point carries no freshness logic itself.
    let settle_result = match result {
        Ok(_) => {
            let next = now + workspace_scan_policy::scan_ttl_secs(provider_id, new_status) * 1000;
            store.settle(
                workspace_key,
                provider_id,
                new_status,
                discovered,
                next,
                now,
            )
        }
        Err(error) => {
            let next = now + workspace_scan_policy::error_backoff_secs(prev.as_ref()) * 1000;
            store.fail(workspace_key, provider_id, &format!("{error:#}"), next, now)
        }
    };
    // Bump the feed revision when the provider's status transitions, so a
    // client polling revision sees the convergence (e.g. first scan that finds
    // zero sessions: unindexed -> empty). A data change also bumps, but that is
    // done in `index_workspace_sessions` where the workspace key is already
    // known; bumping on status here covers the zero-data convergence that the
    // data path would miss.
    let status_changed = prev.as_ref().map_or(true, |p| p.status != new_status);
    if status_changed {
        if let Err(error) = crate::storage::workspace_feed_revision::bump(&conn, workspace_key, now)
        {
            crate::logging::error(
                "workspace_session_feed",
                format!("Failed to bump feed revision for {workspace_key}: {error:#}"),
            );
        }
    }
    if let Err(error) = settle_result {
        crate::logging::error(
            "workspace_session_feed",
            format!("Failed to persist scan state for {provider_id}@{workspace_key}: {error:#}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(status: ScanStatus) -> WorkspaceProviderScanState {
        WorkspaceProviderScanState {
            workspace_key: "/ws".into(),
            provider_id: "claude".into(),
            status,
            discovered_count: 0,
            last_scan_started_at_ms: None,
            last_scan_completed_at_ms: Some(1),
            next_scan_at_ms: Some(i64::MAX),
            last_error: None,
        }
    }

    #[test]
    fn ready_and_empty_serve_from_cache_without_scanning() {
        let (kind, needs) =
            provider_kind_for_decision(&ScanDecision::UseCache, Some(&settled(ScanStatus::Ready)));
        assert_eq!(kind, ProviderFeedStateKind::Scanned);
        assert!(!needs);

        let (kind, needs) =
            provider_kind_for_decision(&ScanDecision::UseCache, Some(&settled(ScanStatus::Empty)));
        assert_eq!(kind, ProviderFeedStateKind::Empty);
        assert!(!needs);
    }

    #[test]
    fn start_and_join_both_need_a_scan_running() {
        for decision in [ScanDecision::StartScan, ScanDecision::JoinScan] {
            let (kind, needs) = provider_kind_for_decision(&decision, None);
            assert_eq!(kind, ProviderFeedStateKind::Scanning);
            assert!(needs);
        }
    }

    #[test]
    fn retry_later_reports_error_without_scanning() {
        let (kind, needs) = provider_kind_for_decision(&ScanDecision::RetryLater, None);
        assert_eq!(kind, ProviderFeedStateKind::Error);
        assert!(!needs);
    }
}
