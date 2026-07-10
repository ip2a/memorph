use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    thread,
};

use crate::{
    core,
    provider::{Provider, ProviderSessionSummary},
    providers,
    storage::{
        activity_store::{
            ActivityActor, ActivityCompletion, ActivityOperationKind, ActivityStore, NewActivity,
        },
        local_store,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerFilter {
    pub providers: Vec<String>,
    pub older_than_days: Option<u32>,
    pub older_than_ms: Option<i64>,
    pub larger_than_mb: Option<u32>,
    pub larger_than_bytes: Option<u64>,
    pub smaller_than_bytes: Option<u64>,
    pub workspace: Option<String>,
    pub sort: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerItem {
    pub provider_id: String,
    pub provider_name: String,
    pub session_id: String,
    pub source_path: Option<String>,
    pub title: Option<String>,
    pub project_dir: Option<String>,
    pub last_active_at: Option<i64>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerPreviewResult {
    pub items: Vec<ManagerItem>,
    pub total_count: usize,
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerWorkspacesResult {
    pub items: Vec<ManagerWorkspaceItem>,
    pub total_count: usize,
    pub total_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerStatsResult {
    pub selected_agent_count: usize,
    pub current_workspace_session_count: usize,
    pub current_workspace_size_bytes: u64,
    pub all_workspace_count: usize,
    pub all_workspace_session_count: usize,
    pub all_workspace_size_bytes: u64,
}

pub fn invalidate_stats_cache() {
    crate::cache::manager_stats_cache().invalidate_all();
}

fn manager_provider_ids(filter: &ManagerFilter) -> Vec<String> {
    if filter.providers.is_empty() {
        providers::all_provider_ids()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        filter.providers.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerCleanResult {
    pub success: usize,
    pub failed: usize,
    pub freed_bytes: u64,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerBackupResult {
    pub success: usize,
    pub failed: usize,
    pub files: Vec<String>,
    pub errors: Vec<String>,
}

/// Preview sessions matching the filter criteria.
pub fn preview(filter: &ManagerFilter) -> Result<ManagerPreviewResult> {
    let provider_ids = manager_provider_ids(filter);

    let cutoff_ms = filter.older_than_ms.or_else(|| {
        filter.older_than_days.map(|days| {
            let duration = chrono::Duration::days(days as i64);
            (Utc::now() - duration).timestamp_millis()
        })
    });

    let larger_than_bytes = filter
        .larger_than_bytes
        .or_else(|| filter.larger_than_mb.map(|mb| mb as u64 * 1024 * 1024));
    let smaller_than_bytes = filter.smaller_than_bytes;

    let items_by_provider = thread::scope(|scope| {
        let handles: Vec<_> = provider_ids
            .iter()
            .map(|pid| {
                scope.spawn(move || {
                    let provider = match providers::find_provider(pid) {
                        Some(p) => p,
                        None => return Vec::new(),
                    };

                    let cache = crate::cache::global_cache();
                    let sessions = match cache.get_or_refresh(pid, || provider.scan_sessions()) {
                        Ok(s) => s,
                        Err(_) => return Vec::new(),
                    };

                    let candidates: Vec<ProviderSessionSummary> = sessions
                        .into_iter()
                        .filter(|meta| {
                            if let Some(ref ws) = filter.workspace {
                                let matches = workspace_session_matches(&*provider, meta, ws);
                                if !matches {
                                    return false;
                                }
                            }
                            if let Some(cutoff) = cutoff_ms {
                                let last_active = meta.last_active_at.unwrap_or(i64::MAX);
                                if last_active > cutoff {
                                    return false;
                                }
                            }
                            true
                        })
                        .collect();
                    let session_ids: Vec<&str> = candidates
                        .iter()
                        .map(|meta| meta.session_id.as_str())
                        .collect();
                    let sizes = provider.session_sizes(&session_ids);

                    candidates
                        .into_iter()
                        .filter_map(|meta| {
                            let size_bytes = sizes.get(&meta.session_id).copied().unwrap_or(0);
                            if larger_than_bytes.is_some_and(|threshold| size_bytes < threshold) {
                                return None;
                            }
                            if smaller_than_bytes.is_some_and(|threshold| size_bytes > threshold) {
                                return None;
                            }
                            Some(ManagerItem {
                                provider_id: pid.clone(),
                                provider_name: provider.name().to_string(),
                                session_id: meta.session_id.clone(),
                                source_path: meta.source_path.clone(),
                                title: meta.title.clone(),
                                project_dir: meta.project_dir.clone(),
                                last_active_at: meta.last_active_at,
                                size_bytes,
                            })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .collect::<Vec<_>>()
    });
    let mut items: Vec<ManagerItem> = items_by_provider.into_iter().flatten().collect();

    if filter.sort.as_deref() == Some("recent") {
        items.sort_by_key(|item| std::cmp::Reverse(item.last_active_at.unwrap_or(0)));
    } else {
        items.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));
    }

    if let Some(limit) = filter.limit {
        items.truncate(limit);
    }

    let total_count = items.len();
    let total_size_bytes = items.iter().map(|item| item.size_bytes).sum();

    Ok(ManagerPreviewResult {
        total_count,
        total_size_bytes,
        items,
    })
}

pub fn stats(filter: &ManagerFilter) -> Result<ManagerStatsResult> {
    if let Some(result) = crate::cache::manager_stats_cache().get(filter) {
        return Ok(result);
    }

    let selected_agent_count = if filter.providers.is_empty() {
        providers::all_provider_ids().len()
    } else {
        filter.providers.iter().collect::<BTreeSet<_>>().len()
    };

    let mut current_filter = filter.clone();
    current_filter.limit = None;

    let mut all_filter = filter.clone();
    all_filter.workspace = None;
    all_filter.limit = None;
    let (current_workspace, all_workspaces) = thread::scope(|scope| {
        let current_handle = scope.spawn(|| preview(&current_filter));
        let all_handle = scope.spawn(|| workspaces(&all_filter));
        let current_workspace = current_handle
            .join()
            .map_err(|_| anyhow::anyhow!("manager stats preview task failed"))??;
        let all_workspaces = all_handle
            .join()
            .map_err(|_| anyhow::anyhow!("manager stats workspaces task failed"))??;
        Ok::<_, anyhow::Error>((current_workspace, all_workspaces))
    })?;

    let all_workspace_count = all_workspaces
        .items
        .iter()
        .map(|item| item.workspace.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let all_workspace_session_count = all_workspaces
        .items
        .iter()
        .map(|item| item.session_count)
        .sum();
    let all_workspace_size_bytes = all_workspaces
        .items
        .iter()
        .map(|item| item.total_size_bytes)
        .sum();

    let result = ManagerStatsResult {
        selected_agent_count,
        current_workspace_session_count: current_workspace.total_count,
        current_workspace_size_bytes: current_workspace.total_size_bytes,
        all_workspace_count,
        all_workspace_session_count,
        all_workspace_size_bytes,
    };

    crate::cache::manager_stats_cache().set(filter, result.clone());

    Ok(result)
}

/// Clean (delete) the specified sessions.
pub fn clean(items: &[ManagerItem], actor: ActivityActor) -> ManagerCleanResult {
    let mut success = 0usize;
    let mut failed = 0usize;
    let mut freed_bytes: u64 = 0;
    let mut errors = Vec::new();

    let mut by_provider: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (idx, item) in items.iter().enumerate() {
        by_provider
            .entry(item.provider_id.as_str())
            .or_default()
            .push(idx);
    }

    for (provider_id, indices) in by_provider {
        let session_ids: Vec<&str> = indices
            .iter()
            .map(|idx| items[*idx].session_id.as_str())
            .collect();
        let mut results = core::delete_sessions(provider_id, &session_ids, actor).into_iter();
        let mut provider_deleted = false;
        for idx in indices {
            let item = &items[idx];
            let result = results.next().unwrap_or_else(|| {
                Err(anyhow::anyhow!(
                    "Provider returned no delete result for session {}",
                    item.session_id
                ))
            });
            match result {
                Ok(()) => {
                    success += 1;
                    freed_bytes += item.size_bytes;
                    provider_deleted = true;
                }
                Err(e) => {
                    failed += 1;
                    errors.push(format!(
                        "Failed to delete {} ({}): {}",
                        item.session_id,
                        item.title.as_deref().unwrap_or("untitled"),
                        e
                    ));
                }
            }
        }
        if provider_deleted {
            crate::cache::global_cache().invalidate(provider_id);
        }
    }

    invalidate_stats_cache();
    ManagerCleanResult {
        success,
        failed,
        freed_bytes,
        errors,
    }
}

/// Backup (export) the specified sessions to a directory.
pub fn backup(
    items: &[ManagerItem],
    output_dir: &Path,
    actor: ActivityActor,
) -> ManagerBackupResult {
    let mut success = 0usize;
    let mut failed = 0usize;
    let mut files = Vec::new();
    let mut errors = Vec::new();

    for item in items {
        let input_details = serde_json::json!({
            "provider_session_id": item.session_id,
            "output_dir": output_dir,
        });
        let activity_conn = match local_store::open_database() {
            Ok(conn) => conn,
            Err(error) => {
                failed += 1;
                errors.push(format!(
                    "Failed to start backup for {} ({}): {:#}",
                    item.session_id,
                    item.title.as_deref().unwrap_or("untitled"),
                    error
                ));
                continue;
            }
        };
        let activity_id = match ActivityStore::new(&activity_conn).start(NewActivity {
            provider_id: Some(item.provider_id.clone()),
            provider_session_id: Some(item.session_id.clone()),
            workspace_dir: item.project_dir.clone(),
            operation_kind: ActivityOperationKind::Backup,
            actor,
            summary: "Backing up session".to_string(),
            details: input_details.clone(),
        }) {
            Ok(activity_id) => activity_id,
            Err(error) => {
                failed += 1;
                errors.push(format!(
                    "Failed to start backup for {} ({}): {:#}",
                    item.session_id,
                    item.title.as_deref().unwrap_or("untitled"),
                    error
                ));
                continue;
            }
        };
        let result: Result<String> = (|| {
            std::fs::create_dir_all(output_dir).with_context(|| {
                format!(
                    "Failed to create output directory: {}",
                    output_dir.display()
                )
            })?;
            let session =
                crate::core::get_canonical_session(&item.provider_id, &item.session_id)?.session;
            let safe_title = item
                .title
                .as_deref()
                .unwrap_or("untitled")
                .replace(|c: char| !c.is_alphanumeric() && c != '-' && c != '_', "_")
                .replace("__", "_");
            let filename = format!(
                "{}_{}_{}.json",
                item.provider_id,
                safe_title,
                &item.session_id[..8.min(item.session_id.len())]
            );
            let output_path = output_dir.join(&filename);
            export_session_to_json(&session, &output_path)?;
            Ok(output_path.display().to_string())
        })();

        match result {
            Ok(file) => {
                if let Err(error) = ActivityStore::new(&activity_conn).finish(
                    &activity_id,
                    ActivityCompletion::success(
                        "Backed up session",
                        serde_json::json!({
                            "provider_session_id": item.session_id,
                            "file": file,
                        }),
                    ),
                ) {
                    failed += 1;
                    errors.push(format!(
                        "Backed up {} but failed to finish its activity record: {:#}",
                        item.session_id, error
                    ));
                    continue;
                }
                success += 1;
                files.push(file);
            }
            Err(error) => {
                let message = format!("{error:#}");
                let audit_error = ActivityStore::new(&activity_conn)
                    .finish(
                        &activity_id,
                        ActivityCompletion::failed(
                            "Failed to back up session",
                            input_details,
                            &message,
                        ),
                    )
                    .err();
                failed += 1;
                let mut error_message = format!(
                    "Failed to export {} ({}): {}",
                    item.session_id,
                    item.title.as_deref().unwrap_or("untitled"),
                    message
                );
                if let Some(audit_error) = audit_error {
                    error_message.push_str(&format!(
                        "; failed to finish activity record: {audit_error:#}"
                    ));
                }
                errors.push(error_message);
            }
        }
    }

    ManagerBackupResult {
        success,
        failed,
        files,
        errors,
    }
}

fn export_session_to_json(session: &crate::canonical::CanonicalSession, path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(session)?;
    std::fs::write(path, json)
        .with_context(|| format!("Failed to write export file: {}", path.display()))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerWorkspaceItem {
    pub provider_id: String,
    pub provider_name: String,
    pub workspace: String,
    pub session_count: usize,
    pub total_size_bytes: u64,
    pub last_active_at: Option<i64>,
}

fn workspace_group_key(provider: &dyn Provider, meta: &ProviderSessionSummary) -> String {
    provider
        .normalized_workspace_key(meta.project_dir.as_deref())
        .unwrap_or_else(|| meta.project_dir.clone().unwrap_or_else(|| "—".to_string()))
}

fn workspace_session_matches(
    provider: &dyn Provider,
    meta: &ProviderSessionSummary,
    workspace: &str,
) -> bool {
    provider.workspace_matches(meta.project_dir.as_deref(), Some(workspace))
        || workspace_group_key(provider, meta) == workspace
}

/// Build an aggregated view of (provider, workspace) groups across the requested providers.
pub fn workspaces(filter: &ManagerFilter) -> Result<ManagerWorkspacesResult> {
    let provider_ids = manager_provider_ids(filter);

    let cutoff_ms = filter.older_than_ms.or_else(|| {
        filter.older_than_days.map(|days| {
            let duration = chrono::Duration::days(days as i64);
            (Utc::now() - duration).timestamp_millis()
        })
    });

    let larger_than_bytes = filter
        .larger_than_bytes
        .or_else(|| filter.larger_than_mb.map(|mb| mb as u64 * 1024 * 1024));
    let smaller_than_bytes = filter.smaller_than_bytes;

    let groups_by_provider = thread::scope(|scope| {
        let handles: Vec<_> = provider_ids
            .iter()
            .map(|pid| {
                scope.spawn(move || {
                    let provider = match providers::find_provider(pid) {
                        Some(p) => p,
                        None => return BTreeMap::new(),
                    };

                    let cache = crate::cache::global_cache();
                    let sessions = match cache.get_or_refresh(pid, || provider.scan_sessions()) {
                        Ok(s) => s,
                        Err(_) => return BTreeMap::new(),
                    };

                    let candidates: Vec<&ProviderSessionSummary> = sessions
                        .iter()
                        .filter(|meta| {
                            if let Some(cutoff) = cutoff_ms {
                                let last_active = meta.last_active_at.unwrap_or(i64::MAX);
                                if last_active > cutoff {
                                    return false;
                                }
                            }
                            true
                        })
                        .collect();

                    let session_ids: Vec<&str> = candidates
                        .iter()
                        .map(|meta| meta.session_id.as_str())
                        .collect();
                    let sizes = provider.session_sizes(&session_ids);
                    let mut groups: BTreeMap<(String, String), ManagerWorkspaceItem> =
                        BTreeMap::new();

                    for meta in candidates {
                        let size_bytes = sizes.get(&meta.session_id).copied().unwrap_or(0);

                        if larger_than_bytes.is_some_and(|threshold| size_bytes < threshold) {
                            continue;
                        }
                        if smaller_than_bytes.is_some_and(|threshold| size_bytes > threshold) {
                            continue;
                        }

                        let workspace = workspace_group_key(&*provider, meta);

                        let key = (pid.clone(), workspace.clone());
                        let entry = groups.entry(key).or_insert_with(|| ManagerWorkspaceItem {
                            provider_id: pid.clone(),
                            provider_name: provider.name().to_string(),
                            workspace,
                            session_count: 0,
                            total_size_bytes: 0,
                            last_active_at: None,
                        });
                        entry.session_count += 1;
                        entry.total_size_bytes += size_bytes;
                        let last_active = meta.last_active_at.unwrap_or(0);
                        entry.last_active_at = Some(
                            entry
                                .last_active_at
                                .map_or(last_active, |v| v.max(last_active)),
                        );
                    }

                    groups
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| handle.join().ok())
            .collect::<Vec<_>>()
    });
    let mut groups: BTreeMap<(String, String), ManagerWorkspaceItem> = BTreeMap::new();
    for provider_groups in groups_by_provider {
        groups.extend(provider_groups);
    }

    let mut items: Vec<ManagerWorkspaceItem> = groups.into_values().collect();

    if filter.sort.as_deref() == Some("size") {
        items.sort_by_key(|item| std::cmp::Reverse(item.total_size_bytes));
    } else {
        items.sort_by_key(|item| std::cmp::Reverse(item.last_active_at.unwrap_or(0)));
    }

    if let Some(limit) = filter.limit {
        items.truncate(limit);
    }

    let total_count = items.len();
    let total_size_bytes = items.iter().map(|i| i.total_size_bytes).sum();
    Ok(ManagerWorkspacesResult {
        items,
        total_count,
        total_size_bytes,
    })
}

/// Resolve the concrete ManagerItem rows for a given provider workspace.
fn list_workspace_sessions(provider_id: &str, workspace: &str) -> Result<Vec<ManagerItem>> {
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;

    let cache = crate::cache::global_cache();
    let sessions = cache.get_or_refresh(provider_id, || provider.scan_sessions())?;

    let candidates: Vec<ProviderSessionSummary> = sessions
        .into_iter()
        .filter(|meta| workspace_session_matches(&*provider, meta, workspace))
        .collect();
    let session_ids: Vec<&str> = candidates
        .iter()
        .map(|meta| meta.session_id.as_str())
        .collect();
    let sizes = provider.session_sizes(&session_ids);

    let items: Vec<ManagerItem> = candidates
        .into_iter()
        .map(|meta| ManagerItem {
            provider_id: provider_id.to_string(),
            provider_name: provider.name().to_string(),
            session_id: meta.session_id.clone(),
            source_path: meta.source_path.clone(),
            title: meta.title.clone(),
            project_dir: meta.project_dir.clone(),
            last_active_at: meta.last_active_at,
            size_bytes: sizes.get(&meta.session_id).copied().unwrap_or(0),
        })
        .collect();

    Ok(items)
}

/// Delete all sessions in a provider workspace.
pub fn clean_workspace(
    provider_id: &str,
    workspace: &str,
    actor: ActivityActor,
) -> ManagerCleanResult {
    let items = match list_workspace_sessions(provider_id, workspace) {
        Ok(items) => items,
        Err(e) => {
            return ManagerCleanResult {
                success: 0,
                failed: 0,
                freed_bytes: 0,
                errors: vec![e.to_string()],
            };
        }
    };
    clean(&items, actor)
}

/// Backup all sessions in a provider workspace.
pub fn backup_workspace(
    provider_id: &str,
    workspace: &str,
    output_dir: &Path,
    actor: ActivityActor,
) -> ManagerBackupResult {
    let items = match list_workspace_sessions(provider_id, workspace) {
        Ok(items) => items,
        Err(e) => {
            return ManagerBackupResult {
                success: 0,
                failed: 0,
                files: Vec::new(),
                errors: vec![e.to_string()],
            };
        }
    };
    backup(&items, output_dir, actor)
}
