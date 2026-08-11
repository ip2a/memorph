use anyhow::{Context as _, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;

use super::{
    inspection::{
        inspect_bundle, read_frontmatter, BundleInspection, SkillAgent, SkillEntry, SkillsOverview,
    },
    repository,
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
}

pub fn persist_default(overview: &SkillsOverview, mode: ScanMode) -> Result<ScanSummary> {
    let mut store = LocalSqliteStore::open_default()?;
    persist(store.connection_mut(), overview, mode)
}

pub fn persist_path(
    path: &std::path::Path,
    overview: &SkillsOverview,
    mode: ScanMode,
) -> Result<ScanSummary> {
    let mut store = LocalSqliteStore::open(path)?;
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
        let (catalog, installations) = records(&entries, agent, overview.catalog_only)?;
        let fingerprint = root_fingerprint(agent, &catalog, &installations);
        repository::persist_root(
            conn,
            &agent.agent_id,
            &agent.scope_kind,
            agent
                .workspace_dir
                .as_deref()
                .map(|path| path.to_string_lossy())
                .as_deref(),
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
    Ok(summary)
}

fn installation_used_by(agent_id: &str) -> &str {
    if agent_id == "agents-shared" {
        "all"
    } else {
        agent_id
    }
}

fn entries_for_agent<'a>(overview: &'a SkillsOverview, agent: &SkillAgent) -> Vec<&'a SkillEntry> {
    overview
        .skills
        .iter()
        .filter(|skill| {
            skill
                .installations
                .iter()
                .any(|item| item.used_by == installation_used_by(&agent.agent_id))
        })
        .collect()
}

fn records(
    entries: &[&SkillEntry],
    agent: &SkillAgent,
    catalog_only: bool,
) -> Result<(
    Vec<repository::CatalogRecord>,
    Vec<repository::InstallationRecord>,
)> {
    let mut catalog = BTreeMap::new();
    let mut installations = Vec::new();
    for skill in entries {
        for item in skill.installations.iter().filter(|item| {
            item.used_by == installation_used_by(&agent.agent_id)
                && item.path.join("SKILL.md").is_file()
        }) {
            let entry_path = item.path.join("SKILL.md");
            let entry = fs::read(&entry_path)
                .with_context(|| format!("Failed to read {}", item.path.display()))?;
            let entry_hash = hash(&entry);
            let inspection = if catalog_only {
                BundleInspection {
                    fingerprint: item.fingerprint.clone(),
                    statistics: skill.statistics.clone(),
                    assets: Vec::new(),
                    issues: skill.issues.clone(),
                }
            } else {
                inspect_bundle(&item.path)
            };
            let frontmatter = read_frontmatter(&entry_path);
            // Catalog identity is content-independent: keyed by the skill's stable
            // normalized name and its canonical install location. Editing a skill's
            // body must update the same row in place via ON CONFLICT, not fork a new
            // id and orphan the previous row as a "missing" duplicate — which a
            // content-hash id would do on every save.
            let canonical = item
                .path
                .canonicalize()
                .unwrap_or_else(|_| item.path.clone());
            let id = format!(
                "skill:{}",
                hash(format!("{}:{}", skill.id, canonical.display()).as_bytes())
            );
            catalog
                .entry(id.clone())
                .or_insert_with(|| repository::CatalogRecord {
                    id: id.clone(),
                    name: skill.name.clone(),
                    normalized_name: skill.id.clone(),
                    description: skill.description.clone(),
                    version: frontmatter.get("version").cloned(),
                    author: frontmatter.get("author").cloned(),
                    entry_hash,
                    bundle_hash: inspection.fingerprint.clone(),
                    metadata_json: serde_json::to_string(&frontmatter)
                        .expect("frontmatter map is serializable"),
                    trigger_terms_json: serde_json::to_string(&trigger_terms(skill, &frontmatter))
                        .expect("trigger terms are serializable"),
                    section_index_json: serde_json::to_string(&sections(&String::from_utf8_lossy(
                        &entry,
                    )))
                    .expect("section index is serializable"),
                    file_manifest_json: serde_json::to_string(&inspection.assets)
                        .expect("bundle assets are serializable"),
                    file_count: inspection.statistics.files as u64,
                    total_bytes: inspection.statistics.bytes,
                    tags: tags(agent, &inspection),
                });
            let metadata = item.path.symlink_metadata()?;
            let is_link = metadata.file_type().is_symlink();
            let install_id = format!(
                "installation:{}",
                hash(format!("{}:{}", item.used_by, item.path.display()).as_bytes())
            );
            installations.push(repository::InstallationRecord {
                id: install_id,
                skill_id: id,
                used_by: item.used_by.clone(),
                scope_kind: agent.scope_kind.clone(),
                workspace_dir: agent
                    .workspace_dir
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned()),
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
                bundle_hash: inspection.fingerprint,
            });
        }
    }
    let mut catalog = catalog.into_values().collect::<Vec<_>>();
    let mut names = std::collections::BTreeMap::<String, usize>::new();
    for record in &catalog {
        *names.entry(record.normalized_name.clone()).or_default() += 1;
    }
    for record in &mut catalog {
        if names
            .get(&record.normalized_name)
            .copied()
            .unwrap_or_default()
            > 1
        {
            record.tags.push("conflict".into());
        }
        record.tags.sort();
        record.tags.dedup();
    }
    Ok((catalog, installations))
}

#[derive(Serialize)]
struct SectionIndex {
    id: String,
    title: String,
    level: usize,
    path: Vec<String>,
}

fn sections(entry: &str) -> Vec<SectionIndex> {
    let mut parents = Vec::<String>::new();
    entry
        .lines()
        .filter_map(|line| {
            let hashes = line.chars().take_while(|ch| *ch == '#').count();
            if hashes == 0 || hashes > 6 || !line[hashes..].starts_with(' ') {
                return None;
            }
            let title = line[hashes..].trim().to_string();
            parents.truncate(hashes.saturating_sub(1));
            parents.push(title.clone());
            let normalized = parents
                .iter()
                .map(|value| value.to_lowercase())
                .collect::<Vec<_>>()
                .join("/");
            Some(SectionIndex {
                id: hash(format!("{normalized}:{hashes}:{}", title.to_lowercase()).as_bytes()),
                title,
                level: hashes,
                path: parents.clone(),
            })
        })
        .collect()
}

fn tags(agent: &SkillAgent, inspection: &super::inspection::BundleInspection) -> Vec<String> {
    let mut tags = vec![format!("scope:{}", agent.scope_kind)];
    if agent.agent_id == "agents-shared" {
        tags.extend(["shared".into(), "used-by:all".into()]);
    } else {
        tags.push(format!("used-by:{}", agent.agent_id));
    }
    if inspection.statistics.scripts > 0 {
        tags.push("has:scripts".into());
    }
    if inspection.statistics.references > 0 {
        tags.push("has:references".into());
    }
    if inspection.statistics.assets > 0 {
        tags.push("has:assets".into());
    }
    if inspection
        .issues
        .iter()
        .any(|issue| issue.message.starts_with("Risk signal:"))
    {
        tags.push("risk".into());
    }
    if inspection
        .issues
        .iter()
        .any(|issue| issue.message == "File is unreadable")
    {
        tags.push("missing".into());
    }
    tags.sort();
    tags.dedup();
    tags
}

fn trigger_terms(skill: &SkillEntry, metadata: &BTreeMap<String, String>) -> Vec<String> {
    let mut terms = vec![skill.id.clone(), skill.name.clone()];
    for key in ["trigger", "triggers", "command"] {
        if let Some(value) = metadata.get(key) {
            terms.extend(
                value
                    .split([',', '|'])
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string),
            );
        }
    }
    terms.sort();
    terms.dedup();
    terms
}

fn root_fingerprint(
    agent: &SkillAgent,
    catalog: &[repository::CatalogRecord],
    installations: &[repository::InstallationRecord],
) -> String {
    // Content-aware fingerprint: includes SKILL.md entry hash and bundle hash
    // so editing a skill's content invalidates the cached scan state. Without
    // this, incremental mode silently skips content-only changes because the
    // installation paths and skill ids stay the same.
    let entry_by_id: std::collections::HashMap<&str, &repository::CatalogRecord> = catalog
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect();
    let mut value = format!("{}:{}", agent.agent_id, agent.skills_dir.display());
    for item in installations {
        let entry_hash = entry_by_id
            .get(item.skill_id.as_str())
            .map(|skill| skill.entry_hash.as_str())
            .unwrap_or("");
        value.push_str(&format!(
            "\n{}:{}:{}:{}",
            item.canonical_path, item.skill_id, entry_hash, item.bundle_hash
        ));
    }
    hash(value.as_bytes())
}

fn hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::inspection::{SkillInstallation, SkillStatistics};
    use crate::skills::invocation;

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
        let summary = invocation::index(store.connection_mut(), false).unwrap();
        let states: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM skill_scan_state WHERE state_kind = 'session-source'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(summary.sources_scanned, 250);
        assert_eq!(states, 250);
    }

    #[test]
    fn incremental_and_full_scans_persist_and_rebuild_catalog_state() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        let path = root.join("demo");
        fs::create_dir_all(&path).unwrap();
        fs::write(
            path.join("SKILL.md"),
            "---\nname: Demo\nversion: 1.2.3\nauthor: Ada\n---\n# Demo",
        )
        .unwrap();
        fs::write(path.join("notes.txt"), "actual bundle file").unwrap();
        let mut overview = SkillsOverview {
            agents: vec![SkillAgent {
                agent_id: "codex".into(),
                name: "Codex".into(),
                skills_dir: root,
                scope_kind: "global".into(),
                workspace_dir: None,
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
                    used_by: "codex".into(),
                    path,
                    managed: false,
                    deployment_mode: "external".into(),
                    link_valid: true,
                    fingerprint: "sha256:bundle".into(),
                    drifted: false,
                }],
            }],
            catalog_only: false,
        };
        let mut store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        store.connection().execute(
            "INSERT INTO session_sources (id, provider_id, provider_session_id, source_path, first_seen_at_ms, last_seen_at_ms)
             VALUES ('source-1', 'codex', 'session-1', '/tmp/session.jsonl', 1, 1)",
            [],
        ).unwrap();
        let summary = persist(store.connection_mut(), &overview, ScanMode::Incremental).unwrap();
        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM skill_catalog", [], |row| row.get(0))
            .unwrap();
        let metadata: (Option<String>, Option<String>, i64, String) = store
            .connection()
            .query_row(
                "SELECT version, author, file_count, file_manifest_json FROM skill_catalog",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(summary.installations_seen, 1);
        let session_scan_states: i64 = store
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM skill_scan_state WHERE state_kind IN ('session-source', 'aggregate')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let invocation_rows: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM skill_invocations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(session_scan_states, 0);
        assert_eq!(invocation_rows, 0);
        assert_eq!(count, 1);
        assert_eq!(metadata.0.as_deref(), Some("1.2.3"));
        assert_eq!(metadata.1.as_deref(), Some("Ada"));
        assert_eq!(metadata.2, 2);
        assert!(metadata.3.contains("notes.txt"));
        let catalog_id: String = store
            .connection()
            .query_row("SELECT id FROM skill_catalog LIMIT 1", [], |row| row.get(0))
            .unwrap();
        store.connection().execute(
            "INSERT INTO skill_usage_daily (usage_date, skill_id, provider_id, workspace_key, invocation_count, session_count, updated_at_ms) VALUES ('2026-07-23', ?1, 'codex', '', 1, 1, 1)",
            [&catalog_id],
        ).unwrap();

        persist(store.connection_mut(), &overview, ScanMode::Incremental).unwrap();
        let usage_rows: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM skill_usage_daily", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(usage_rows, 1);
        let incremental_state: (i64, Option<i64>) = store
            .connection()
            .query_row(
                "SELECT scan_generation, last_full_scan_at_ms FROM skill_scan_state
                 WHERE state_kind = 'skill-root'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(incremental_state, (2, None));

        overview.skills.clear();
        persist(store.connection_mut(), &overview, ScanMode::Full).unwrap();
        let rebuilt_state: (i64, Option<i64>, String) = store
            .connection()
            .query_row(
                "SELECT scan_generation, last_full_scan_at_ms, completeness_status
                 FROM skill_scan_state WHERE state_kind = 'skill-root'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let installation_status: String = store
            .connection()
            .query_row("SELECT status FROM skill_installations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(rebuilt_state.0, 3);
        assert!(rebuilt_state.1.is_some());
        assert_eq!(rebuilt_state.2, "complete");
        assert_eq!(installation_status, "missing");
    }

    #[test]
    fn content_edit_updates_catalog_row_in_place_without_orphan() {
        // Editing a skill's body must not fork a new catalog id and strand the
        // previous row as a "missing" duplicate. The id is derived from the
        // stable name + canonical path, so a content edit upserts the same row.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        let path = root.join("demo");
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), "---\nname: demo\n---\n# Demo v1").unwrap();

        let overview = SkillsOverview {
            agents: vec![SkillAgent {
                agent_id: "codex".into(),
                name: "Codex".into(),
                skills_dir: root,
                scope_kind: "global".into(),
                workspace_dir: None,
            }],
            skills: vec![SkillEntry {
                id: "demo".into(),
                name: "demo".into(),
                description: None,
                directory: "demo".into(),
                fingerprint: "sha256:bundle-v1".into(),
                conflict: false,
                statistics: SkillStatistics {
                    files: 1,
                    bytes: 10,
                    scripts: 0,
                    references: 0,
                    assets: 0,
                    previewable: 1,
                },
                issues: vec![],
                installations: vec![SkillInstallation {
                    used_by: "codex".into(),
                    path: path.clone(),
                    managed: false,
                    deployment_mode: "external".into(),
                    link_valid: true,
                    fingerprint: "sha256:bundle-v1".into(),
                    drifted: false,
                }],
            }],
            catalog_only: false,
        };
        let mut store = LocalSqliteStore::open(dir.path().join("memorph.db")).unwrap();
        persist(store.connection_mut(), &overview, ScanMode::Incremental).unwrap();

        let (id_before, hash_before): (String, String) = store
            .connection()
            .query_row(
                "SELECT id, entry_content_hash FROM skill_catalog",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM skill_catalog", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);

        // Simulate a content edit: same skill name, same path, new body + new bundle hash.
        fs::write(
            path.join("SKILL.md"),
            "---\nname: demo\n---\n# Demo v2 entirely new body",
        )
        .unwrap();
        let mut edited = overview.clone();
        edited.skills[0].fingerprint = "sha256:bundle-v2".into();
        edited.skills[0].installations[0].fingerprint = "sha256:bundle-v2".into();
        persist(store.connection_mut(), &edited, ScanMode::Incremental).unwrap();

        let (id_after, hash_after, missing_since): (String, String, Option<i64>) = store
            .connection()
            .query_row(
                "SELECT id, entry_content_hash, missing_since_ms FROM skill_catalog",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let count: i64 = store
            .connection()
            .query_row("SELECT COUNT(*) FROM skill_catalog", [], |row| row.get(0))
            .unwrap();

        assert_eq!(count, 1, "content edit must not spawn a second catalog row");
        assert_eq!(
            id_after, id_before,
            "catalog id must be stable across content edits"
        );
        assert_ne!(
            hash_after, hash_before,
            "entry hash must reflect the new content"
        );
        assert!(
            missing_since.is_none(),
            "row must not be marked missing after an edit"
        );
    }
}
