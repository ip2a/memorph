use super::{SessionGroup, SessionItem, SessionListFields, SessionListSort};
use crate::core::{projection, session_management};
use crate::providers;
use crate::storage::{
    activity_store::ActivityActor, local_store, snapshot_store::SnapshotStore,
};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Parameters for the unified workspace session feed.
///
/// The feed is designed for the home/refresh path: return whatever is already
/// projected for the workspace immediately, then trigger (or join) a
/// background per-provider scan for providers that are missing or explicitly
/// being refreshed.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderFeedStateKind {
    /// Data came from the local SQLite projection without a scan.
    Cached,
    /// A scan for this provider is currently in flight.
    Scanning,
    /// A scan completed and produced a projected result.
    Scanned,
    /// The scan failed.
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

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct InFlightScan {
    started_at: Instant,
    workspace: String,
    provider_id: String,
}

static IN_FLIGHT_SCANS: OnceLock<Mutex<HashMap<String, InFlightScan>>> = OnceLock::new();
const IN_FLIGHT_MAX_AGE_SECS: u64 = 300;

fn in_flight_scans() -> &'static Mutex<HashMap<String, InFlightScan>> {
    IN_FLIGHT_SCANS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn in_flight_key(workspace: &str, provider_id: &str) -> String {
    format!("{provider_id}@{workspace}")
}

fn is_scanning(workspace: &str, provider_id: &str) -> bool {
    prune_stale_in_flight();
    in_flight_scans()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains_key(&in_flight_key(workspace, provider_id))
}

fn mark_scanning(workspace: String, provider_id: String) {
    prune_stale_in_flight();
    in_flight_scans()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            in_flight_key(&workspace, &provider_id),
            InFlightScan {
                started_at: Instant::now(),
                workspace,
                provider_id,
            },
        );
}

fn mark_scan_complete(workspace: &str, provider_id: &str) {
    prune_stale_in_flight();
    in_flight_scans()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&in_flight_key(workspace, provider_id));
}

fn prune_stale_in_flight() {
    let Ok(mut map) = in_flight_scans().try_lock() else {
        return;
    };
    let now = Instant::now();
    map.retain(|_, scan| now.duration_since(scan.started_at).as_secs() < IN_FLIGHT_MAX_AGE_SECS);
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Return a fast workspace-scoped session feed.
///
/// Behavior:
/// * Resolve the workspace directory and the relevant provider set.
/// * For each provider, read already-projected sessions for the workspace.
/// * If `refresh` is true, or no cached sessions exist for a provider, start
///   (or join) a background per-provider workspace scan. The scan uses the
///   provider's native workspace scan when available and falls back to a full
///   provider scan filtered by workspace.
/// * Return the cached/limited sessions immediately along with `feed_state`
///   so callers know whether to poll.
pub fn workspace_session_feed(params: &WorkspaceSessionFeedParams) -> Result<WorkspaceSessionFeed> {
    let workspace_dir = params
        .workspace_dir
        .canonicalize()
        .with_context(|| format!("Failed to resolve workspace: {}", params.workspace_dir.display()))?;
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

    let per_provider_limit = params.per_provider_limit.max(1);
    let mut groups = Vec::with_capacity(provider_ids.len());
    let mut provider_states = Vec::with_capacity(provider_ids.len());
    let mut any_scanning = false;
    let mut any_missing = false;
    let mut any_error = false;
    let mut all_cached = true;
    let mut refreshed_at: Option<i64> = None;

    for provider_id in &provider_ids {
        let provider_name = providers::find_provider(provider_id)
            .map(|provider| provider.name().to_string())
            .unwrap_or_else(|| provider_id.clone());

        let workspace_key =
            session_management::normalized_workspace_key(provider_id, Some(&workspace));
        let workspace_scopes = workspace_key
            .as_deref()
            .map(|key| vec![(provider_id.clone(), key.to_string())]);

        let snapshots = store
            .list_session_snapshots_filtered(
                Some(&[provider_id.clone()]),
                workspace_scopes.as_deref(),
                params.fields.include_stats(),
            )
            .with_context(|| {
                format!("Failed to list projected snapshots for provider {provider_id}")
            })?;

        let has_cache = !snapshots.is_empty();
        all_cached = all_cached && has_cache;

        let mut sessions: Vec<SessionItem> = snapshots
            .into_iter()
            .map(|snapshot| projection::projected_snapshot_item(&snapshot))
            .collect();
        projection::sort_session_items(&mut sessions, &params.sort);
        sessions.truncate(per_provider_limit);

        let need_scan = params.refresh || !has_cache;
        let state = if need_scan {
            if is_scanning(&workspace, provider_id) {
                any_scanning = true;
                ProviderFeedState {
                    provider_id: provider_id.clone(),
                    kind: ProviderFeedStateKind::Scanning,
                    message: Some("Scanning provider sessions for workspace".to_string()),
                    discovered_count: sessions.len(),
                }
            } else {
                any_scanning = true;
                if !has_cache {
                    any_missing = true;
                }
                spawn_provider_workspace_scan(
                    workspace_dir.clone(),
                    provider_id.clone(),
                    ActivityActor::Api,
                );
                ProviderFeedState {
                    provider_id: provider_id.clone(),
                    kind: ProviderFeedStateKind::Scanning,
                    message: Some("Started scanning provider sessions for workspace".to_string()),
                    discovered_count: sessions.len(),
                }
            }
        } else {
            ProviderFeedState {
                provider_id: provider_id.clone(),
                kind: ProviderFeedStateKind::Cached,
                message: None,
                discovered_count: sessions.len(),
            }
        };

        if state.kind == ProviderFeedStateKind::Error {
            any_error = true;
        }
        provider_states.push(state);

        if !sessions.is_empty() {
            groups.push(SessionGroup {
                provider_id: provider_id.clone(),
                provider_name,
                sessions,
            });
        }
    }

    let feed_state = if any_error {
        FeedState {
            kind: FeedStateKind::Error,
            message: Some("One or more providers failed to scan".to_string()),
        }
    } else if any_scanning && any_missing {
        FeedState {
            kind: FeedStateKind::ColdScanning,
            message: Some("Scanning providers for workspace sessions".to_string()),
        }
    } else if any_scanning {
        FeedState {
            kind: FeedStateKind::Warming,
            message: Some("Refreshing provider sessions in background".to_string()),
        }
    } else if !all_cached {
        FeedState {
            kind: FeedStateKind::ColdScanning,
            message: Some("No sessions indexed for this workspace yet".to_string()),
        }
    } else if params.refresh {
        // Refresh requested but no scans are in flight: they finished before we
        // assembled the response.
        refreshed_at = Some(now_ms());
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

fn spawn_provider_workspace_scan(workspace_dir: PathBuf, provider_id: String, actor: ActivityActor) {
    let workspace_text = workspace_dir.to_string_lossy().to_string();
    mark_scanning(workspace_text.clone(), provider_id.clone());

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
            mark_scan_complete(&workspace_text, &provider_id);
        })
        .ok();
}