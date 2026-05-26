use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

use crate::{core, provider::ProviderSessionSummary, providers};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagerFilter {
    pub providers: Vec<String>,
    pub older_than_days: Option<u32>,
    pub older_than_ms: Option<i64>,
    pub larger_than_mb: Option<u32>,
    pub larger_than_bytes: Option<u64>,
    pub smaller_than_bytes: Option<u64>,
    pub workspace: Option<String>,
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
    let provider_ids = if filter.providers.is_empty() {
        providers::all_provider_ids()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        filter.providers.clone()
    };

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

    let mut items = Vec::new();
    let mut total_size_bytes: u64 = 0;

    for pid in &provider_ids {
        let provider = match providers::find_provider(pid) {
            Some(p) => p,
            None => continue,
        };

        let sessions = match provider.scan_sessions() {
            Ok(s) => s,
            Err(_) => continue,
        };

        let candidates: Vec<ProviderSessionSummary> = sessions
            .into_iter()
            .filter(|meta| {
                if let Some(ref ws) = filter.workspace {
                    let matches = provider.workspace_matches(meta.project_dir.as_deref(), Some(ws));
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

        for meta in candidates {
            // Filter by workspace if specified
            let size_bytes = sizes.get(&meta.session_id).copied().unwrap_or(0);

            // Filter by size
            if let Some(threshold) = larger_than_bytes {
                if size_bytes < threshold {
                    continue;
                }
            }
            if let Some(threshold) = smaller_than_bytes {
                if size_bytes > threshold {
                    continue;
                }
            }

            total_size_bytes += size_bytes;
            items.push(ManagerItem {
                provider_id: pid.clone(),
                provider_name: provider.name().to_string(),
                session_id: meta.session_id.clone(),
                source_path: meta.source_path.clone(),
                title: meta.title.clone(),
                project_dir: meta.project_dir.clone(),
                last_active_at: meta.last_active_at,
                size_bytes,
            });
        }
    }

    // Sort by size descending (largest first)
    items.sort_by_key(|item| std::cmp::Reverse(item.size_bytes));

    Ok(ManagerPreviewResult {
        total_count: items.len(),
        total_size_bytes,
        items,
    })
}

/// Clean (delete) the specified sessions.
pub fn clean(items: &[ManagerItem]) -> ManagerCleanResult {
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
        let mut results = core::delete_sessions(provider_id, &session_ids).into_iter();
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
    }

    ManagerCleanResult {
        success,
        failed,
        freed_bytes,
        errors,
    }
}

/// Backup (export) the specified sessions to a directory.
pub fn backup(items: &[ManagerItem], output_dir: &Path) -> ManagerBackupResult {
    let mut success = 0usize;
    let mut failed = 0usize;
    let mut files = Vec::new();
    let mut errors = Vec::new();

    if !output_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(output_dir) {
            return ManagerBackupResult {
                success: 0,
                failed: items.len(),
                files: Vec::new(),
                errors: vec![format!("Failed to create output directory: {}", e)],
            };
        }
    }

    for item in items {
        let session = match crate::core::get_canonical_session(&item.provider_id, &item.session_id)
        {
            Ok(imported) => imported.session,
            Err(e) => {
                failed += 1;
                errors.push(format!(
                    "Failed to load {} ({}): {}",
                    item.session_id,
                    item.title.as_deref().unwrap_or("untitled"),
                    e
                ));
                continue;
            }
        };

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

        match export_session_to_json(&session, &output_path) {
            Ok(()) => {
                success += 1;
                files.push(output_path.display().to_string());
            }
            Err(e) => {
                failed += 1;
                errors.push(format!(
                    "Failed to export {} ({}): {}",
                    item.session_id,
                    item.title.as_deref().unwrap_or("untitled"),
                    e
                ));
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
