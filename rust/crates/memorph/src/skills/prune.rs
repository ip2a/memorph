use anyhow::{anyhow, Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MARKER: &str = ".memorph-managed-skill";

#[derive(Clone, Debug, Serialize)]
pub struct PrunePreview {
    pub preview_id: String,
    pub days: u32,
    pub completeness_status: String,
    pub blocked_reason: Option<String>,
    pub items: Vec<PruneItem>,
}
#[derive(Clone, Debug, Serialize)]
pub struct PruneItem {
    pub installation_id: String,
    pub skill_id: String,
    pub name: String,
    pub install_path: String,
    pub install_kind: String,
    pub unused_since_ms: i64,
    pub last_invoked_at_ms: Option<i64>,
    pub installation_bytes: u64,
    pub metadata_tokens: u64,
    pub low_confidence_observations: u64,
    pub action: String,
    pub executable: bool,
    pub blocked_reason: Option<String>,
    pub expected_fingerprint: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct ExecuteRequest {
    pub preview_id: String,
    pub items: Vec<ExecuteItem>,
    pub confirmation: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct ExecuteItem {
    pub installation_id: String,
    pub expected_fingerprint: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct ExecuteResult {
    pub installation_id: String,
    pub status: String,
    pub message: String,
}

pub fn preview(conn: &Connection, days: u32) -> Result<PrunePreview> {
    let days = days.max(1);
    let cutoff = chrono::Utc::now().timestamp_millis() - i64::from(days) * 86_400_000;
    let (status, earliest): (String, Option<i64>) = conn.query_row(
        "SELECT completeness_status, earliest_indexed_at_ms FROM skill_scan_state WHERE state_key = 'skill-sessions:all'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap_or_else(|_| ("unknown".into(), None));
    let history_block = if status != "complete" {
        Some("会话历史索引尚未完整".to_string())
    } else if earliest.is_none_or(|value| value > cutoff) {
        Some("已索引历史未覆盖所选时间窗".to_string())
    } else {
        None
    };
    let mut statement = conn.prepare(
        "SELECT i.id, i.skill_id, c.canonical_name, i.install_path, i.install_kind,
                i.managed_marker_present, i.link_status, i.bundle_content_hash, c.total_bytes,
                c.metadata_json,
                (SELECT MAX(invoked_at_ms) FROM skill_invocations v WHERE v.skill_id = i.skill_id AND v.confidence IN ('high','medium')),
                (SELECT COUNT(*) FROM skill_invocations v WHERE v.skill_id = i.skill_id AND v.confidence = 'low' AND v.invoked_at_ms >= ?1)
         FROM skill_installations i JOIN skill_catalog c ON c.id = i.skill_id
         WHERE i.status = 'active' AND NOT EXISTS (
           SELECT 1 FROM skill_invocations v WHERE v.skill_id = i.skill_id
             AND v.confidence IN ('high','medium') AND v.invoked_at_ms >= ?1)
         ORDER BY c.canonical_name, i.id",
    )?;
    let rows = statement.query_map([cutoff], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, bool>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, String>(9)?,
            row.get::<_, Option<i64>>(10)?,
            row.get::<_, i64>(11)?,
        ))
    })?;
    let mut items = Vec::new();
    for row in rows {
        let (
            id,
            skill_id,
            name,
            path,
            kind,
            marker,
            link_status,
            bundle_hash,
            bytes,
            metadata,
            last,
            low,
        ) = row?;
        let path_ref = Path::new(&path);
        let fingerprint = fingerprint(path_ref, &kind, &bundle_hash)?;
        let item_block = history_block.clone().or_else(|| match kind.as_str() {
            "directory" => Some("真实目录永不由 Prune 删除".into()),
            "managed-copy" if !marker || !path_ref.join(MARKER).is_file() => {
                Some("受管标记缺失".into())
            }
            "symlink" if link_status != "valid" && link_status != "broken" => {
                Some("符号链接状态不安全".into())
            }
            _ if low > 0 => Some("存在低置信调用证据，需人工确认".into()),
            _ => None,
        });
        let metadata_tokens = ((metadata.chars().count() as u64) + 3) / 4;
        items.push(PruneItem {
            installation_id: id,
            skill_id,
            name,
            install_path: path,
            install_kind: kind.clone(),
            unused_since_ms: cutoff,
            last_invoked_at_ms: last,
            installation_bytes: bytes as u64,
            metadata_tokens,
            low_confidence_observations: low as u64,
            action: if kind == "symlink" {
                "unlink".into()
            } else if kind == "managed-copy" {
                "remove-managed-copy".into()
            } else {
                "none".into()
            },
            executable: item_block.is_none(),
            blocked_reason: item_block,
            expected_fingerprint: fingerprint,
        });
    }
    let preview_id = format!(
        "prune-preview-{:x}",
        Sha256::digest(
            format!(
                "{days}:{cutoff}:{}",
                items
                    .iter()
                    .map(|i| i.expected_fingerprint.as_str())
                    .collect::<Vec<_>>()
                    .join(":")
            )
            .as_bytes()
        )
    );
    Ok(PrunePreview {
        preview_id,
        days,
        completeness_status: status,
        blocked_reason: history_block,
        items,
    })
}

pub fn execute(
    conn: &Connection,
    roots: &[PathBuf],
    request: &ExecuteRequest,
) -> Result<Vec<ExecuteResult>> {
    if request.confirmation != "REMOVE_MANAGED_INSTALLATIONS" {
        return Err(anyhow!("Invalid prune confirmation"));
    }
    let days = request
        .preview_id
        .split('-')
        .nth(2)
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| anyhow!("Invalid prune preview"))?;
    let current = preview(conn, days)?;
    if current.preview_id != request.preview_id {
        return Err(anyhow!("Prune candidates changed after preview"));
    }
    let mut results = Vec::new();
    for requested in &request.items {
        let candidate = current
            .items
            .iter()
            .find(|item| {
                item.installation_id == requested.installation_id
                    && item.expected_fingerprint == requested.expected_fingerprint
            })
            .ok_or_else(|| anyhow!("Installation was not present in preview"))?;
        if !candidate.executable {
            return Err(anyhow!(candidate
                .blocked_reason
                .clone()
                .unwrap_or_else(|| "Installation is blocked".into())));
        }
        let record = conn.query_row("SELECT install_path, install_kind, bundle_content_hash, managed_marker_present FROM skill_installations WHERE id = ?1 AND status = 'active'", [&requested.installation_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, bool>(3)?)))?;
        let path = PathBuf::from(&record.0);
        if !roots
            .iter()
            .any(|root| path.parent() == Some(root.as_path()))
        {
            return Err(anyhow!(
                "Installation path changed or is outside an allowed root"
            ));
        }
        if record.1 == "directory" {
            return Err(anyhow!("Refusing to remove a real directory"));
        }
        if fingerprint(&path, &record.1, &record.2)? != requested.expected_fingerprint {
            return Err(anyhow!("Installation changed after preview"));
        }
        if record.1 == "symlink" {
            if !path
                .symlink_metadata()
                .is_ok_and(|m| m.file_type().is_symlink())
            {
                return Err(anyhow!("Installation is no longer a symbolic link"));
            }
            fs::remove_file(&path)
                .with_context(|| format!("Failed to unlink {}", path.display()))?;
        } else if record.1 == "managed-copy" {
            if !record.3 || !path.join(MARKER).is_file() {
                return Err(anyhow!("Managed marker is missing"));
            }
            fs::remove_dir_all(&path)
                .with_context(|| format!("Failed to remove {}", path.display()))?;
        } else {
            return Err(anyhow!("Unsupported installation type"));
        }
        conn.execute("UPDATE skill_installations SET status = 'removed', removed_at_ms = ?2, last_verified_at_ms = ?2 WHERE id = ?1", params![requested.installation_id, chrono::Utc::now().timestamp_millis()])?;
        results.push(ExecuteResult {
            installation_id: requested.installation_id.clone(),
            status: "removed".into(),
            message: "安全安装已移除".into(),
        });
    }
    Ok(results)
}

fn fingerprint(path: &Path, kind: &str, bundle_hash: &str) -> Result<String> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("Installation path is unavailable: {}", path.display()))?;
    let target = if metadata.file_type().is_symlink() {
        fs::read_link(path)?.to_string_lossy().into_owned()
    } else {
        String::new()
    };
    let marker = path.join(MARKER).is_file();
    Ok(format!(
        "sha256:{:x}",
        Sha256::digest(
            format!("{}:{kind}:{bundle_hash}:{target}:{marker}", path.display()).as_bytes()
        )
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::local_store::LocalSqliteStore;

    fn insert(conn: &Connection, id: &str, path: &Path, kind: &str, marker: bool) {
        conn.execute("INSERT OR IGNORE INTO skill_catalog (id, canonical_name, normalized_name, entry_content_hash, bundle_content_hash, metadata_json, file_count, total_bytes, first_seen_at_ms, last_scanned_at_ms, created_at_ms, updated_at_ms) VALUES (?1, ?1, ?1, 'entry', 'bundle', '{}', 1, 10, 1, 1, 1, 1)", [id]).unwrap();
        conn.execute("INSERT INTO skill_installations (id, skill_id, provider_id, scope_kind, install_path, canonical_install_path, install_kind, managed_marker_present, bundle_content_hash, discovered_at_ms, last_verified_at_ms) VALUES (?1, ?2, 'codex', 'global', ?3, ?3, ?4, ?5, 'bundle', 1, 1)", params![format!("install-{id}"), id, path.to_string_lossy(), kind, marker]).unwrap();
    }

    #[test]
    fn never_removes_real_directories_and_rechecks_managed_copy() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalSqliteStore::open(root.path().join("db.sqlite")).unwrap();
        store.connection().execute("INSERT OR REPLACE INTO skill_scan_state (state_key, state_kind, completeness_status, earliest_indexed_at_ms, items_seen, updated_at_ms) VALUES ('skill-sessions:all', 'aggregate', 'complete', 1, 0, 1)", []).unwrap();
        let real = root.path().join("real");
        fs::create_dir(&real).unwrap();
        insert(store.connection(), "real", &real, "directory", false);
        let previewed = preview(store.connection(), 30).unwrap();
        let item = previewed
            .items
            .iter()
            .find(|item| item.skill_id == "real")
            .unwrap();
        assert!(!item.executable);
        assert!(execute(
            store.connection(),
            &[root.path().to_path_buf()],
            &ExecuteRequest {
                preview_id: previewed.preview_id,
                items: vec![ExecuteItem {
                    installation_id: item.installation_id.clone(),
                    expected_fingerprint: item.expected_fingerprint.clone()
                }],
                confirmation: "REMOVE_MANAGED_INSTALLATIONS".into()
            }
        )
        .is_err());
        assert!(real.is_dir());

        let managed = root.path().join("managed");
        fs::create_dir(&managed).unwrap();
        fs::write(managed.join(MARKER), "managed").unwrap();
        insert(
            store.connection(),
            "managed",
            &managed,
            "managed-copy",
            true,
        );
        let previewed = preview(store.connection(), 30).unwrap();
        let item = previewed
            .items
            .iter()
            .find(|item| item.skill_id == "managed")
            .unwrap()
            .clone();
        fs::remove_file(managed.join(MARKER)).unwrap();
        assert!(execute(
            store.connection(),
            &[root.path().to_path_buf()],
            &ExecuteRequest {
                preview_id: previewed.preview_id,
                items: vec![ExecuteItem {
                    installation_id: item.installation_id,
                    expected_fingerprint: item.expected_fingerprint
                }],
                confirmation: "REMOVE_MANAGED_INSTALLATIONS".into()
            }
        )
        .is_err());
        assert!(managed.is_dir());
    }
}
