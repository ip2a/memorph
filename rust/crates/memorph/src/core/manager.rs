use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    thread,
};

use crate::{
    core, providers,
    storage::{
        activity_store::{
            ActivityActor, ActivityCompletion, ActivityOperationKind, ActivityStore, NewActivity,
        },
        artifact_store::{ArtifactStore, BackupRecord, NewBackupRecord},
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
    pub id: String,
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

impl ManagerItem {
    pub fn action_identity(provider_id: &str, session_id: &str) -> String {
        format!("{}:{provider_id}{session_id}", provider_id.len())
    }
}

impl ManagerPreviewResult {
    fn from_items(mut items: Vec<ManagerItem>, sort: Option<&str>, limit: Option<usize>) -> Self {
        match sort {
            Some("recent") => {
                items.sort_by_key(|item| std::cmp::Reverse(item.last_active_at.unwrap_or(0)));
            }
            Some("title") => items.sort_by(|left, right| {
                left.title
                    .as_deref()
                    .unwrap_or(&left.session_id)
                    .to_lowercase()
                    .cmp(
                        &right
                            .title
                            .as_deref()
                            .unwrap_or(&right.session_id)
                            .to_lowercase(),
                    )
            }),
            _ => items.sort_by_key(|item| std::cmp::Reverse(item.size_bytes)),
        }

        let mut identities = BTreeSet::new();
        items.retain(|item| identities.insert(item.id.clone()));

        let total_count = items.len();
        let total_size_bytes = items.iter().map(|item| item.size_bytes).sum();

        if let Some(limit) = limit {
            items.truncate(limit);
        }

        Self {
            items,
            total_count,
            total_size_bytes,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerWorkspacesResult {
    pub items: Vec<ManagerWorkspaceItem>,
    pub total_count: usize,
    pub total_size_bytes: u64,
}

impl ManagerWorkspacesResult {
    fn from_items(
        mut items: Vec<ManagerWorkspaceItem>,
        sort: Option<&str>,
        limit: Option<usize>,
    ) -> Self {
        match sort {
            Some("size") => {
                items.sort_by_key(|item| std::cmp::Reverse(item.total_size_bytes));
            }
            Some("title") => items.sort_by_key(|item| item.workspace.to_lowercase()),
            Some("sessions") => {
                items.sort_by_key(|item| std::cmp::Reverse(item.session_count));
            }
            _ => {
                items.sort_by_key(|item| std::cmp::Reverse(item.last_active_at.unwrap_or(0)));
            }
        }

        let total_count = items.len();
        let total_size_bytes = items.iter().map(|item| item.total_size_bytes).sum();

        if let Some(limit) = limit {
            items.truncate(limit);
        }

        Self {
            items,
            total_count,
            total_size_bytes,
        }
    }
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
    let items = projected_manager_items(filter)?;

    Ok(ManagerPreviewResult::from_items(
        items,
        filter.sort.as_deref(),
        filter.limit,
    ))
}

fn projected_manager_items(filter: &ManagerFilter) -> Result<Vec<ManagerItem>> {
    let provider_names: BTreeMap<String, String> = manager_provider_ids(filter)
        .into_iter()
        .filter_map(|provider_id| {
            providers::find_provider(&provider_id)
                .map(|provider| (provider_id, provider.name().to_string()))
        })
        .collect();
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
    let conn = local_store::open_database()?;
    let snapshots =
        crate::storage::snapshot_store::SnapshotStore::new(&conn).list_session_snapshots()?;

    Ok(snapshots
        .into_iter()
        .filter_map(|snapshot| {
            let provider_name = provider_names.get(&snapshot.provider_id)?;
            if filter.workspace.as_deref().is_some_and(|workspace| {
                !projected_workspace_matches(
                    &snapshot.provider_id,
                    snapshot.workspace_dir.as_deref(),
                    workspace,
                )
            }) {
                return None;
            }
            if cutoff_ms
                .is_some_and(|cutoff| snapshot.last_active_at_ms.unwrap_or(i64::MAX) > cutoff)
            {
                return None;
            }
            let size_bytes = snapshot.size_bytes.unwrap_or(0);
            if larger_than_bytes.is_some_and(|threshold| size_bytes < threshold)
                || smaller_than_bytes.is_some_and(|threshold| size_bytes > threshold)
            {
                return None;
            }
            let session_id = snapshot
                .provider_session_id
                .unwrap_or(snapshot.canonical_session_id);
            Some(ManagerItem {
                id: ManagerItem::action_identity(&snapshot.provider_id, &session_id),
                provider_id: snapshot.provider_id,
                provider_name: provider_name.clone(),
                session_id,
                source_path: snapshot.source_path,
                title: snapshot.display_title.or(snapshot.title),
                project_dir: snapshot.workspace_dir,
                last_active_at: snapshot.last_active_at_ms,
                size_bytes,
            })
        })
        .collect())
}

fn projected_workspace_group_key(provider_id: &str, workspace: Option<&str>) -> String {
    crate::core::session_management::normalized_workspace_key(provider_id, workspace)
        .unwrap_or_else(|| workspace.unwrap_or("—").to_string())
}

fn projected_workspace_matches(
    provider_id: &str,
    session_workspace: Option<&str>,
    requested_workspace: &str,
) -> bool {
    crate::core::session_management::workspace_matches(
        provider_id,
        session_workspace,
        Some(requested_workspace),
    ) || projected_workspace_group_key(provider_id, session_workspace) == requested_workspace
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
        let mut results =
            core::session_mutation::delete_sessions(provider_id, &session_ids, actor).into_iter();
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
        let mut activity_conn = match local_store::open_database() {
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
        let result: Result<(String, String, String)> = (|| {
            std::fs::create_dir_all(output_dir).with_context(|| {
                format!(
                    "Failed to create output directory: {}",
                    output_dir.display()
                )
            })?;
            let session =
                crate::core::sessions::get_canonical_session(&item.provider_id, &item.session_id)?
                    .session;
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
            let backup =
                register_manager_backup(&mut activity_conn, &activity_id, item, &output_path)?;
            Ok((
                output_path.display().to_string(),
                backup.artifact.id,
                backup.id,
            ))
        })();

        match result {
            Ok((file, artifact_id, backup_id)) => {
                if let Err(error) = ActivityStore::new(&activity_conn).finish(
                    &activity_id,
                    ActivityCompletion::success(
                        "Backed up session",
                        serde_json::json!({
                            "provider_session_id": item.session_id,
                            "file": file,
                            "artifact_id": artifact_id,
                            "backup_id": backup_id,
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

fn register_manager_backup(
    conn: &mut rusqlite::Connection,
    operation_id: &str,
    item: &ManagerItem,
    backup_path: &Path,
) -> Result<BackupRecord> {
    ArtifactStore::new(conn).register_backup(NewBackupRecord {
        operation_id: Some(operation_id.to_string()),
        provider_id: Some(item.provider_id.clone()),
        provider_session_id: Some(item.session_id.clone()),
        session_id: None,
        source_path: item.source_path.as_deref().map(PathBuf::from),
        backup_path: backup_path.to_path_buf(),
        restore_hint: Some(
            "Import this canonical JSON backup through memorph into a selected provider."
                .to_string(),
        ),
        mime_type: Some("application/json".to_string()),
        format: Some("json".to_string()),
        artifact_metadata: serde_json::json!({
            "role": "manager_canonical_backup",
        }),
        backup_metadata: serde_json::json!({
            "restore_mode": "canonical_import",
        }),
    })
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

/// Build an aggregated view of (provider, workspace) groups across the requested providers.
pub fn workspaces(filter: &ManagerFilter) -> Result<ManagerWorkspacesResult> {
    let mut candidates = projected_manager_items(filter)?;
    candidates.sort_by_key(|item| std::cmp::Reverse(item.last_active_at.unwrap_or(0)));
    let mut session_ids_seen = BTreeSet::new();
    candidates.retain(|item| session_ids_seen.insert(item.id.clone()));

    let mut groups: BTreeMap<(String, String), ManagerWorkspaceItem> = BTreeMap::new();
    for item in candidates {
        let workspace =
            projected_workspace_group_key(&item.provider_id, item.project_dir.as_deref());
        let key = (item.provider_id.clone(), workspace.clone());
        let entry = groups.entry(key).or_insert_with(|| ManagerWorkspaceItem {
            provider_id: item.provider_id.clone(),
            provider_name: item.provider_name.clone(),
            workspace,
            session_count: 0,
            total_size_bytes: 0,
            last_active_at: None,
        });
        entry.session_count += 1;
        entry.total_size_bytes += item.size_bytes;
        let last_active = item.last_active_at.unwrap_or(0);
        entry.last_active_at = Some(
            entry
                .last_active_at
                .map_or(last_active, |value| value.max(last_active)),
        );
    }

    let items = groups.into_values().collect();

    Ok(ManagerWorkspacesResult::from_items(
        items,
        filter.sort.as_deref(),
        filter.limit,
    ))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceWithSessionsItem {
    pub path: String,
    pub session_count: usize,
    pub last_active_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct WorkspaceWithSessionsOptions {
    pub search: Option<String>,
    pub page: usize,
    pub page_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceWithSessionsResult {
    pub items: Vec<WorkspaceWithSessionsItem>,
    pub total_count: usize,
    pub page: usize,
    pub page_size: usize,
    pub total_pages: usize,
}

/// List unique workspace paths that have sessions, aggregated across providers.
pub fn workspaces_with_sessions(
    options: &WorkspaceWithSessionsOptions,
) -> Result<WorkspaceWithSessionsResult> {
    let page_size = options.page_size.clamp(1, 50);
    let conn = local_store::open_database()?;
    let result = crate::storage::snapshot_store::SnapshotStore::new(&conn)
        .list_workspaces_with_sessions(options.search.as_deref(), options.page, page_size)?;
    let total_count = result.total_count;
    let total_pages = total_count.div_ceil(page_size).max(1);
    let page = options.page.max(1).min(total_pages);

    Ok(WorkspaceWithSessionsResult {
        items: result
            .items
            .into_iter()
            .map(|item| WorkspaceWithSessionsItem {
                path: item.path,
                session_count: item.session_count,
                last_active_at: item.last_active_at_ms,
            })
            .collect(),
        total_count: result.total_count,
        page,
        page_size,
        total_pages,
    })
}

/// Resolve the concrete ManagerItem rows for a given provider workspace.
fn list_workspace_sessions(provider_id: &str, workspace: &str) -> Result<Vec<ManagerItem>> {
    projected_manager_items(&ManagerFilter {
        providers: vec![provider_id.to_string()],
        older_than_days: None,
        older_than_ms: None,
        larger_than_mb: None,
        larger_than_bytes: None,
        smaller_than_bytes: None,
        workspace: Some(workspace.to_string()),
        sort: Some("recent".to_string()),
        limit: None,
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        activity_store::{ActivityQuery, ActivityStatus},
        artifact_store::{ArtifactManifestKind, NewArtifactManifest},
    };

    struct TestConfigHomeGuard;

    impl TestConfigHomeGuard {
        fn new(path: &Path) -> Self {
            crate::config::set_test_home_dir(path.to_path_buf());
            Self
        }
    }

    impl Drop for TestConfigHomeGuard {
        fn drop(&mut self) {
            crate::config::reset_test_home_dir();
        }
    }

    fn test_connection() -> rusqlite::Connection {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        conn
    }

    fn test_item() -> ManagerItem {
        ManagerItem {
            id: ManagerItem::action_identity("database-provider", "provider-session-1"),
            provider_id: "database-provider".to_string(),
            provider_name: "Database Provider".to_string(),
            session_id: "provider-session-1".to_string(),
            source_path: None,
            title: Some("Session".to_string()),
            project_dir: Some("/workspace".to_string()),
            last_active_at: Some(1),
            size_bytes: 2,
        }
    }

    struct ProjectedManagerSnapshot<'a> {
        conn: &'a rusqlite::Connection,
        session_id: &'a str,
        provider_session_id: &'a str,
        workspace: &'a str,
        source_path: &'a str,
        title: &'a str,
        last_active_at: i64,
        size_bytes: i64,
    }

    fn insert_projected_manager_snapshot(input: ProjectedManagerSnapshot<'_>) {
        let ProjectedManagerSnapshot {
            conn,
            session_id,
            provider_session_id,
            workspace,
            source_path,
            title,
            last_active_at,
            size_bytes,
        } = input;
        conn.execute(
            "INSERT INTO session_sources
             (id, provider_id, provider_session_id, source_path, workspace_dir, file_size_bytes,
              first_seen_at_ms, last_seen_at_ms)
             VALUES (?1, 'claude', ?2, ?3, ?4, ?5, 10, 10)",
            rusqlite::params![
                format!("source-{session_id}"),
                provider_session_id,
                source_path,
                workspace,
                size_bytes,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, primary_source_id, workspace_dir, title,
              status, created_at_ms, last_active_at_ms, event_count, turn_count)
             VALUES (?1, 'claude', ?2, ?3, ?4, ?5, 'completed', 10, ?6, 2, 1)",
            rusqlite::params![
                session_id,
                provider_session_id,
                format!("source-{session_id}"),
                workspace,
                title,
                last_active_at,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_snapshots
             (session_id, provider_id, title, workspace_dir, status, last_active_at_ms,
              event_count, turn_count, flags_json, projection_version, stale, updated_at_ms)
             VALUES (?1, 'claude', ?2, ?3, 'completed', ?4, 2, 1, '{}', 1, 0, ?4)",
            rusqlite::params![session_id, title, workspace, last_active_at],
        )
        .unwrap();
    }

    #[test]
    fn manager_action_identity_is_stable_and_unambiguous() {
        assert_eq!(
            ManagerItem::action_identity("codex", "session-1"),
            ManagerItem::action_identity("codex", "session-1")
        );
        assert_ne!(
            ManagerItem::action_identity("a", "bc"),
            ManagerItem::action_identity("ab", "c")
        );
    }

    #[test]
    fn workspace_actions_resolve_items_from_projected_sessions() {
        let home = tempfile::tempdir().unwrap();
        let _home_guard = TestConfigHomeGuard::new(home.path());
        let conn = local_store::open_database().unwrap();
        insert_projected_manager_snapshot(ProjectedManagerSnapshot {
            conn: &conn,
            session_id: "canonical-workspace",
            provider_session_id: "native-workspace",
            workspace: "/work/project-one",
            source_path: "/missing/provider/source.jsonl",
            title: "Projected workspace session",
            last_active_at: 200,
            size_bytes: 4096,
        });
        drop(conn);

        let items = list_workspace_sessions("claude", "/work/project-one").unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].session_id, "native-workspace");
        assert_eq!(items[0].size_bytes, 4096);
    }

    #[test]
    fn manager_read_models_ignore_provider_history_and_requested_provider_volume() {
        let home = tempfile::tempdir().unwrap();
        let _home_guard = TestConfigHomeGuard::new(home.path());
        let history_dir = home.path().join(".claude/projects/project-history");
        std::fs::create_dir_all(&history_dir).unwrap();
        for index in 0..128 {
            std::fs::write(
                history_dir.join(format!("historical-{index}.jsonl")),
                b"not a provider session",
            )
            .unwrap();
        }

        let conn = local_store::open_database().unwrap();
        insert_projected_manager_snapshot(ProjectedManagerSnapshot {
            conn: &conn,
            session_id: "canonical-1",
            provider_session_id: "native-1",
            workspace: "/work/project-one",
            source_path: "/missing/provider/source.jsonl",
            title: "Projected session",
            last_active_at: 200,
            size_bytes: 4096,
        });
        insert_projected_manager_snapshot(ProjectedManagerSnapshot {
            conn: &conn,
            session_id: "canonical-2",
            provider_session_id: "native-2",
            workspace: "/work/project-two",
            source_path: "/missing/provider/second-source.jsonl",
            title: "Second projected session",
            last_active_at: 100,
            size_bytes: 512,
        });
        drop(conn);

        let mut providers = (0..128)
            .map(|index| format!("unknown-{index}"))
            .collect::<Vec<_>>();
        providers.push("claude".to_string());
        let filter = ManagerFilter {
            providers,
            older_than_days: None,
            older_than_ms: None,
            larger_than_mb: None,
            larger_than_bytes: None,
            smaller_than_bytes: None,
            workspace: None,
            sort: Some("recent".to_string()),
            limit: Some(100),
        };

        let result = preview(&filter).unwrap();
        assert_eq!(result.total_count, 2);
        assert_eq!(result.total_size_bytes, 4608);
        assert_eq!(result.items[0].session_id, "native-1");
        assert_eq!(result.items[0].title.as_deref(), Some("Projected session"));

        let workspaces = workspaces(&filter).unwrap();
        assert_eq!(workspaces.total_count, 2);
        assert_eq!(workspaces.total_size_bytes, 4608);
        assert_eq!(workspaces.items[0].workspace, "/work/project-one");
        assert_eq!(workspaces.items[0].session_count, 1);

        let filtered = preview(&ManagerFilter {
            workspace: Some("/work/project-one".to_string()),
            larger_than_bytes: Some(4096),
            smaller_than_bytes: Some(4096),
            ..filter
        })
        .unwrap();
        assert_eq!(filtered.total_count, 1);
        assert_eq!(filtered.total_size_bytes, 4096);
        assert_eq!(filtered.items[0].session_id, "native-1");
    }

    #[test]
    fn preview_result_deduplicates_before_counting_and_limiting() {
        let mut older_duplicate = test_item();
        older_duplicate.title = Some("Older duplicate".to_string());
        older_duplicate.last_active_at = Some(10);
        older_duplicate.size_bytes = 10;

        let mut newer_duplicate = older_duplicate.clone();
        newer_duplicate.title = Some("Newer duplicate".to_string());
        newer_duplicate.last_active_at = Some(40);

        let mut second = test_item();
        second.id = ManagerItem::action_identity("database-provider", "provider-session-2");
        second.session_id = "provider-session-2".to_string();
        second.last_active_at = Some(30);
        second.size_bytes = 20;

        let mut third = test_item();
        third.id = ManagerItem::action_identity("database-provider", "provider-session-3");
        third.session_id = "provider-session-3".to_string();
        third.last_active_at = Some(20);
        third.size_bytes = 30;

        let result = ManagerPreviewResult::from_items(
            vec![older_duplicate, second, third, newer_duplicate],
            Some("recent"),
            Some(2),
        );

        assert_eq!(result.items.len(), 2);
        assert_eq!(result.total_count, 3);
        assert_eq!(result.total_size_bytes, 60);
        assert_eq!(result.items[0].title.as_deref(), Some("Newer duplicate"));
    }

    #[test]
    fn preview_size_sort_keeps_largest_duplicate() {
        let mut smaller = test_item();
        smaller.title = Some("Smaller".to_string());
        smaller.size_bytes = 10;

        let mut larger = smaller.clone();
        larger.title = Some("Larger".to_string());
        larger.size_bytes = 20;

        let result = ManagerPreviewResult::from_items(vec![smaller, larger], Some("size"), None);

        assert_eq!(result.total_count, 1);
        assert_eq!(result.total_size_bytes, 20);
        assert_eq!(result.items[0].title.as_deref(), Some("Larger"));
    }

    #[test]
    fn workspace_result_counts_before_limiting() {
        let items = [10_u64, 20, 30]
            .into_iter()
            .enumerate()
            .map(|(index, total_size_bytes)| ManagerWorkspaceItem {
                provider_id: "database-provider".to_string(),
                provider_name: "Database Provider".to_string(),
                workspace: format!("/workspace/{index}"),
                session_count: index + 1,
                total_size_bytes,
                last_active_at: Some(index as i64),
            })
            .collect();

        let result = ManagerWorkspacesResult::from_items(items, Some("size"), Some(2));

        assert_eq!(result.items.len(), 2);
        assert_eq!(result.total_count, 3);
        assert_eq!(result.total_size_bytes, 60);
    }

    #[test]
    fn workspace_result_sorts_by_session_count() {
        let items = [2_usize, 8, 4]
            .into_iter()
            .enumerate()
            .map(|(index, session_count)| ManagerWorkspaceItem {
                provider_id: "database-provider".to_string(),
                provider_name: "Database Provider".to_string(),
                workspace: format!("/workspace/{index}"),
                session_count,
                total_size_bytes: 10,
                last_active_at: Some(index as i64),
            })
            .collect();

        let result = ManagerWorkspacesResult::from_items(items, Some("sessions"), None);

        assert_eq!(result.items[0].session_count, 8);
        assert_eq!(result.items[1].session_count, 4);
        assert_eq!(result.items[2].session_count, 2);
    }

    #[test]
    fn registers_manager_backup_with_activity_and_canonical_identity() {
        let dir = tempfile::tempdir().unwrap();
        let backup_path = dir.path().join("session.json");
        std::fs::write(&backup_path, b"{}").unwrap();
        let mut conn = test_connection();
        conn.execute(
            "INSERT INTO sessions
             (id, provider_id, provider_session_id, status, event_count, turn_count,
              projection_version, updated_at_ms)
             VALUES
             ('session-1', 'database-provider', 'provider-session-1', 'active', 0, 0, 1, 1)",
            [],
        )
        .unwrap();
        let item = test_item();
        let activity_id = ActivityStore::new(&conn)
            .start(NewActivity {
                provider_id: Some(item.provider_id.clone()),
                provider_session_id: Some(item.session_id.clone()),
                workspace_dir: item.project_dir.clone(),
                operation_kind: ActivityOperationKind::Backup,
                actor: ActivityActor::System,
                summary: "Backing up session".to_string(),
                details: serde_json::json!({}),
            })
            .unwrap();

        let stored = register_manager_backup(&mut conn, &activity_id, &item, &backup_path).unwrap();

        assert_eq!(stored.operation_id.as_deref(), Some(activity_id.as_str()));
        assert_eq!(
            stored.artifact.operation_id.as_deref(),
            Some(activity_id.as_str())
        );
        assert_eq!(
            stored.artifact.artifact_kind,
            ArtifactManifestKind::SessionBackup
        );
        assert_eq!(stored.source_path, None);
        assert_eq!(stored.session_id.as_deref(), Some("session-1"));
        assert_eq!(stored.artifact.session_id.as_deref(), Some("session-1"));
        assert_eq!(
            stored.artifact.provider_id.as_deref(),
            Some("database-provider")
        );
        assert_eq!(
            stored.artifact.provider_session_id.as_deref(),
            Some("provider-session-1")
        );
        assert_eq!(
            stored.artifact.mime_type.as_deref(),
            Some("application/json")
        );
        assert_eq!(stored.artifact.format.as_deref(), Some("json"));
        assert_eq!(stored.artifact.metadata["role"], "manager_canonical_backup");
        assert_eq!(stored.metadata["restore_mode"], "canonical_import");
        let activities = ActivityStore::new(&conn)
            .query(&ActivityQuery {
                operation_kind: Some(ActivityOperationKind::Backup),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(activities.len(), 1);
        assert_eq!(activities[0].id, activity_id);
        assert_eq!(activities[0].status, ActivityStatus::Running);
    }

    #[test]
    fn registration_failure_keeps_written_manager_backup_file() {
        let dir = tempfile::tempdir().unwrap();
        let backup_path = dir.path().join("session.json");
        std::fs::write(&backup_path, b"{}").unwrap();
        let mut conn = test_connection();
        let item = test_item();
        ArtifactStore::new(&mut conn)
            .register_path(NewArtifactManifest {
                artifact_kind: ArtifactManifestKind::SessionBackup,
                operation_id: Some("operation-1".to_string()),
                provider_id: Some(item.provider_id.clone()),
                provider_session_id: Some(item.session_id.clone()),
                session_id: None,
                projection_report_id: None,
                path: backup_path.clone(),
                mime_type: Some("application/json".to_string()),
                format: Some("json".to_string()),
                metadata: serde_json::json!({"role": "conflicting_backup"}),
            })
            .unwrap();

        let error =
            register_manager_backup(&mut conn, "operation-1", &item, &backup_path).unwrap_err();

        assert!(error
            .to_string()
            .contains("already registered with conflicting context"));
        assert!(backup_path.exists());
        let backup_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM backups", [], |row| row.get(0))
            .unwrap();
        assert_eq!(backup_count, 0);
    }
}
