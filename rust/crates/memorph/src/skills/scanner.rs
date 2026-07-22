use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;

use super::{
    repository,
    server::{SkillAgent, SkillEntry, SkillsOverview},
};
use crate::storage::local_store::LocalSqliteStore;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    Incremental,
    Full,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
pub struct ScanSummary {
    pub roots_scanned: usize,
    pub skills_seen: usize,
    pub installations_seen: usize,
    pub session_sources_seen: usize,
}

pub fn persist_default(overview: &SkillsOverview, mode: ScanMode) -> Result<ScanSummary> {
    let mut store = LocalSqliteStore::open_default()?;
    persist(store.connection_mut(), overview, mode)
}

pub fn persist(
    conn: &mut rusqlite::Connection,
    overview: &SkillsOverview,
    mode: ScanMode,
) -> Result<ScanSummary> {
    let now_ms = Utc::now().timestamp_millis();
    let mut summary = ScanSummary {
        skills_seen: overview.skills.len(),
        ..ScanSummary::default()
    };
    for agent in &overview.agents {
        let entries = entries_for_agent(overview, agent);
        let (catalog, installations) = records(&entries, &agent.provider_id)?;
        let fingerprint = root_fingerprint(agent, &installations);
        repository::persist_root(
            conn,
            &agent.provider_id,
            &agent.skills_dir.to_string_lossy(),
            &fingerprint,
            &catalog,
            &installations,
            mode == ScanMode::Full,
            now_ms,
        )?;
        summary.roots_scanned += 1;
        summary.installations_seen += installations.len();
    }
    summary.session_sources_seen = persist_session_source_states(conn, now_ms)?;
    Ok(summary)
}

fn entries_for_agent<'a>(overview: &'a SkillsOverview, agent: &SkillAgent) -> Vec<&'a SkillEntry> {
    overview
        .skills
        .iter()
        .filter(|skill| {
            skill
                .installations
                .iter()
                .any(|item| item.provider_id == agent.provider_id)
        })
        .collect()
}

fn records(
    entries: &[&SkillEntry],
    provider_id: &str,
) -> Result<(
    Vec<repository::CatalogRecord>,
    Vec<repository::InstallationRecord>,
)> {
    let mut catalog = BTreeMap::new();
    let mut installations = Vec::new();
    for skill in entries {
        for item in skill.installations.iter().filter(|item| {
            item.provider_id == provider_id
                && entries.iter().any(|entry| entry.id == skill.id)
                && item.path.join("SKILL.md").is_file()
        }) {
            let entry = fs::read(item.path.join("SKILL.md"))
                .with_context(|| format!("Failed to read {}", item.path.display()))?;
            let entry_hash = hash(&entry);
            let id = format!(
                "skill:{}",
                hash(format!("{entry_hash}:{}", item.fingerprint).as_bytes())
            );
            catalog
                .entry(id.clone())
                .or_insert_with(|| repository::CatalogRecord {
                    id: id.clone(),
                    name: skill.name.clone(),
                    normalized_name: skill.id.clone(),
                    description: skill.description.clone(),
                    entry_hash,
                    bundle_hash: item.fingerprint.clone(),
                    file_count: skill.statistics.files as u64,
                    total_bytes: skill.statistics.bytes,
                });
            let metadata = item.path.symlink_metadata()?;
            let is_link = metadata.file_type().is_symlink();
            let canonical = item
                .path
                .canonicalize()
                .unwrap_or_else(|_| item.path.clone());
            let install_id = format!(
                "installation:{}",
                hash(format!("{}:{}", item.provider_id, canonical.display()).as_bytes())
            );
            installations.push(repository::InstallationRecord {
                id: install_id,
                skill_id: id,
                provider_id: item.provider_id.clone(),
                install_path: item.path.to_string_lossy().into_owned(),
                canonical_path: canonical.to_string_lossy().into_owned(),
                install_kind: if is_link {
                    "symlink"
                } else if item.managed {
                    "managed-copy"
                } else {
                    "directory"
                }
                .into(),
                symlink_target: is_link
                    .then(|| fs::read_link(&item.path).ok())
                    .flatten()
                    .map(|path| path.to_string_lossy().into_owned()),
                managed_marker_present: !is_link && item.managed,
                link_status: if is_link && !item.link_valid {
                    "broken"
                } else if is_link {
                    "valid"
                } else {
                    "not-applicable"
                }
                .into(),
                bundle_hash: item.fingerprint.clone(),
            });
        }
    }
    Ok((catalog.into_values().collect(), installations))
}

fn root_fingerprint(
    agent: &SkillAgent,
    installations: &[repository::InstallationRecord],
) -> String {
    let mut value = format!("{}:{}", agent.provider_id, agent.skills_dir.display());
    for item in installations {
        value.push_str(&format!("\n{}:{}", item.canonical_path, item.skill_id));
    }
    hash(value.as_bytes())
}

fn persist_session_source_states(conn: &rusqlite::Connection, now_ms: i64) -> Result<usize> {
    let sources = repository::session_sources(conn)?;
    for source in &sources {
        let key = format!("session-source:{}", source.id);
        repository::begin_scan(
            conn,
            &key,
            "session-source",
            Some(&source.provider_id),
            Some(&source.source_path),
            now_ms,
        )?;
        repository::complete_scan(
            conn,
            &key,
            Some(&source.fingerprint),
            source.source_cursor.as_deref(),
            0,
            false,
            "partial",
            source.earliest_at_ms,
            source.latest_at_ms,
            now_ms,
        )?;
    }
    repository::begin_scan(conn, "aggregate:sessions", "aggregate", None, None, now_ms)?;
    let earliest = sources.iter().filter_map(|item| item.earliest_at_ms).min();
    let latest = sources.iter().filter_map(|item| item.latest_at_ms).max();
    repository::complete_scan(
        conn,
        "aggregate:sessions",
        None,
        None,
        sources.len(),
        false,
        if sources.is_empty() {
            "unknown"
        } else {
            "partial"
        },
        earliest,
        latest,
        now_ms,
    )?;
    Ok(sources.len())
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::server::{SkillInstallation, SkillStatistics};

    #[test]
    fn persists_all_session_sources_without_a_200_item_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        for index in 0..250 {
            store.connection().execute(
                "INSERT INTO session_sources (id, provider_id, provider_session_id, source_path, first_seen_at_ms, last_seen_at_ms)
                 VALUES (?1, 'codex', ?1, ?2, 1, 1)",
                rusqlite::params![format!("source-{index}"), format!("/tmp/{index}.jsonl")],
            ).unwrap();
        }
        let count = persist_session_source_states(store.connection_mut(), 10).unwrap();
        let states: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM skill_scan_state WHERE state_kind = 'session-source'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 250);
        assert_eq!(states, 250);
    }

    #[test]
    fn incremental_scan_persists_catalog_and_installation() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        let path = root.join("demo");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), "---\nname: Demo\n---\n# Demo").unwrap();
        let overview = SkillsOverview {
            agents: vec![SkillAgent {
                provider_id: "codex".into(),
                name: "Codex".into(),
                skills_dir: root,
            }],
            skills: vec![SkillEntry {
                id: "demo".into(),
                name: "Demo".into(),
                description: None,
                directory: "demo".into(),
                fingerprint: "sha256:bundle".into(),
                conflict: false,
                statistics: SkillStatistics {
                    files: 1,
                    bytes: 30,
                    scripts: 0,
                    references: 0,
                    assets: 0,
                    previewable: 1,
                },
                issues: vec![],
                installations: vec![SkillInstallation {
                    provider_id: "codex".into(),
                    path,
                    managed: false,
                    deployment_mode: "external".into(),
                    link_valid: true,
                    fingerprint: "sha256:bundle".into(),
                    drifted: false,
                }],
            }],
        };
        let mut store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        let summary = persist(store.connection_mut(), &overview, ScanMode::Incremental).unwrap();
        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM skill_catalog", [], |row| row.get(0))
            .unwrap();
        assert_eq!(summary.installations_seen, 1);
        assert_eq!(count, 1);
    }
}
