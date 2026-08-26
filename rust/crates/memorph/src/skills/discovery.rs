use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::inspection::{
    inspect_bundle, read_frontmatter, SkillAgent, SkillEntry, SkillInstallation, SkillStatistics,
    SkillsOverview, MANAGED_MARKER,
};

pub const SKILL_AGENTS: [(&str, &str, &str, &str); 6] = [
    ("claude", "Claude Code", ".claude/skills", ".claude/skills"),
    ("codex", "Codex", ".codex/skills", ".codex/skills"),
    ("gemini", "Gemini CLI", ".gemini/skills", ".gemini/skills"),
    (
        "opencode",
        "OpenCode",
        ".config/opencode/skills",
        ".opencode/skills",
    ),
    ("hermes", "Hermes", ".hermes/skills", ".hermes/skills"),
    ("agents-shared", "Shared Agents", ".agents/skills", ""),
];

pub fn agents(home: &Path, workspace: Option<&Path>) -> Vec<SkillAgent> {
    let mut agents = SKILL_AGENTS
        .iter()
        .map(|(agent_id, name, global_root, _)| SkillAgent {
            agent_id: (*agent_id).into(),
            name: (*name).into(),
            skills_dir: home.join(global_root),
            scope_kind: "global".into(),
            workspace_dir: None,
        })
        .collect::<Vec<_>>();
    if let Some(workspace) = workspace {
        agents.extend(
            SKILL_AGENTS
                .iter()
                .filter(|(agent_id, _, _, _)| *agent_id != "agents-shared")
                .map(|(agent_id, name, _, relative)| SkillAgent {
                    agent_id: (*agent_id).into(),
                    name: (*name).into(),
                    skills_dir: workspace.join(relative),
                    scope_kind: "project".into(),
                    workspace_dir: Some(PathBuf::from(workspace)),
                }),
        );
    }
    agents
}

/// Dot-prefixed entries are never skills. `.disabled/` parks an archived skill
/// inside its agent's own skills dir (same filesystem → O(1) rename) while
/// staying invisible to discovery; this guard makes that contract explicit so a
/// future recursive scan cannot resurface archived skills.
fn is_hidden_entry(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

pub fn discover(agents: &[SkillAgent]) -> SkillsOverview {
    let mut skills = BTreeMap::<String, SkillEntry>::new();
    for agent in agents {
        let Ok(children) = std::fs::read_dir(&agent.skills_dir) else {
            continue;
        };
        let mut children = children.filter_map(Result::ok).collect::<Vec<_>>();
        children.sort_by_key(|entry| entry.file_name());
        for child in children {
            let path = child.path();
            if is_hidden_entry(&path) || !path.is_dir() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let directory = child.file_name().to_string_lossy().into_owned();
            let (name, description) = read_metadata(&path.join("SKILL.md"), &directory);
            let id = {
                let normalized = skill_id(&name);
                if normalized.is_empty() {
                    skill_id(&directory)
                } else {
                    normalized
                }
            };
            let bundle = inspect_bundle(&path);
            let installation = build_installation(agent, path, bundle.fingerprint.clone());
            let skill = skills.entry(id.clone()).or_insert_with(|| SkillEntry {
                id,
                name,
                description: description.clone(),
                directory,
                fingerprint: bundle.fingerprint.clone(),
                conflict: false,
                statistics: bundle.statistics.clone(),
                issues: bundle.issues.clone(),
                installations: Vec::new(),
            });
            if skill.description.is_none() {
                skill.description = description;
            }
            skill.conflict |= skill.fingerprint != bundle.fingerprint;
            skill.installations.push(installation);
        }
    }
    let skills = skills
        .into_values()
        .map(|mut skill| {
            for installation in &mut skill.installations {
                installation.drifted = installation.fingerprint != skill.fingerprint;
            }
            skill
        })
        .collect();
    SkillsOverview {
        agents: agents.to_vec(),
        skills,
        catalog_only: false,
    }
}

/// Discover the catalog without opening any files below a skill's entry file.
pub fn discover_catalog(agents: &[SkillAgent]) -> SkillsOverview {
    let mut skills = BTreeMap::<String, SkillEntry>::new();
    for agent in agents {
        let Ok(children) = std::fs::read_dir(&agent.skills_dir) else {
            continue;
        };
        for child in children.filter_map(Result::ok) {
            let path = child.path();
            let entry_path = path.join("SKILL.md");
            if is_hidden_entry(&path) || !path.is_dir() || !entry_path.is_file() {
                continue;
            }
            let directory = child.file_name().to_string_lossy().into_owned();
            let (name, description) = read_metadata(&entry_path, &directory);
            let id = skill_id(&name);
            let id = if id.is_empty() {
                skill_id(&directory)
            } else {
                id
            };
            let fingerprint = std::fs::read(&entry_path)
                .map(|body| format!("sha256:{:x}", Sha256::digest(body)))
                .unwrap_or_else(|_| "sha256:unreadable".into());
            let installation = build_installation(agent, path, fingerprint.clone());
            let skill = skills.entry(id.clone()).or_insert_with(|| SkillEntry {
                id,
                name,
                description: description.clone(),
                directory,
                fingerprint: fingerprint.clone(),
                conflict: false,
                statistics: SkillStatistics::default(),
                issues: Vec::new(),
                installations: Vec::new(),
            });
            skill.conflict |= skill.fingerprint != fingerprint;
            skill.installations.push(installation);
        }
    }
    SkillsOverview {
        agents: agents.to_vec(),
        skills: skills.into_values().collect(),
        catalog_only: true,
    }
}

fn installation_used_by(agent_id: &str) -> &str {
    if agent_id == "agents-shared" {
        "all"
    } else {
        agent_id
    }
}

fn build_installation(agent: &SkillAgent, path: PathBuf, fingerprint: String) -> SkillInstallation {
    let is_link = path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink());
    let has_marker = path.join(MANAGED_MARKER).is_file();
    let link_valid = !is_link || path.canonicalize().is_ok();
    let symlink_target = if is_link {
        std::fs::read_link(&path)
            .ok()
            .map(|target| target.to_string_lossy().into_owned())
    } else {
        None
    };
    let link_status = if is_link {
        if link_valid {
            "valid"
        } else {
            "broken"
        }
    } else {
        "not-applicable"
    };
    SkillInstallation {
        used_by: installation_used_by(&agent.agent_id).into(),
        fingerprint,
        drifted: false,
        managed: is_link || has_marker,
        deployment_mode: if is_link {
            "symlink"
        } else if has_marker {
            "copy"
        } else {
            "external"
        }
        .into(),
        link_valid,
        path,
        symlink_target,
        scope_kind: agent.scope_kind.clone(),
        workspace_dir: agent
            .workspace_dir
            .as_ref()
            .map(|dir| dir.to_string_lossy().into_owned()),
        link_status: link_status.into(),
    }
}

fn read_metadata(path: &Path, directory: &str) -> (String, Option<String>) {
    let frontmatter = read_frontmatter(path);
    let name = frontmatter
        .get("name")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| directory.to_string());
    let description = frontmatter
        .get("description")
        .filter(|value| !value.is_empty())
        .cloned();
    (name, description)
}

fn skill_id(name: &str) -> String {
    let mut id = String::new();
    let mut separator = false;
    for ch in name.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_alphanumeric() || ch == '_' || ch == '-' {
            if separator && !id.is_empty() {
                id.push('-');
            }
            separator = false;
            id.push(ch);
        } else {
            separator = true;
        }
    }
    id.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_discovery_reads_multiline_description() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        let skill = root.join("explore");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: explore\ndescription: |\n  first line\n  second line\n---\n",
        )
        .unwrap();
        let agents = vec![SkillAgent {
            agent_id: "codex".into(),
            name: "Codex".into(),
            skills_dir: root,
            scope_kind: "global".into(),
            workspace_dir: None,
        }];

        let overview = discover_catalog(&agents);

        assert_eq!(
            overview.skills[0].description.as_deref(),
            Some("first line\nsecond line")
        );
    }

    #[test]
    fn catalog_discovery_ignores_non_entry_file_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        let skill = root.join("demo");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: Demo\ndescription: Test\nversion: 1\n---\n",
        )
        .unwrap();
        std::fs::write(skill.join("large.bin"), vec![1_u8; 1024]).unwrap();
        let agents = vec![SkillAgent {
            agent_id: "codex".into(),
            name: "Codex".into(),
            skills_dir: root,
            scope_kind: "global".into(),
            workspace_dir: None,
        }];

        let first = discover_catalog(&agents);
        std::fs::write(skill.join("large.bin"), vec![2_u8; 2048]).unwrap();
        let second = discover_catalog(&agents);

        assert_eq!(first.skills[0].fingerprint, second.skills[0].fingerprint);
        assert_eq!(first.skills[0].statistics.files, 0);
        assert!(first.skills[0].issues.is_empty());
    }

    #[test]
    fn discover_skips_dot_prefixed_entries() {
        // A dot-prefixed folder with a SKILL.md must be ignored. This is the
        // explicit contract that keeps `.disabled/` archives out of the catalog
        // even if discovery ever learns to recurse.
        let dir = tempfile::tempdir().unwrap();
        let skills_dir = dir.path().join(".codex").join("skills");
        let visible = skills_dir.join("demo");
        std::fs::create_dir_all(&visible).unwrap();
        std::fs::write(visible.join("SKILL.md"), "---\nname: demo\n---\n").unwrap();
        let hidden = skills_dir.join(".hidden-skill");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::write(hidden.join("SKILL.md"), "---\nname: hidden\n---\n").unwrap();

        let agents = vec![SkillAgent {
            agent_id: "codex".into(),
            name: "Codex".into(),
            skills_dir,
            scope_kind: "global".into(),
            workspace_dir: None,
        }];
        let overview = discover(&agents);

        assert_eq!(overview.skills.len(), 1);
        assert_eq!(overview.skills[0].name, "demo");
    }
}
