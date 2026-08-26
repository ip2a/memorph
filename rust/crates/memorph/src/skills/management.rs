//! Filesystem-backed lifecycle operations for skill bundles: install,
//! uninstall, per-row delete, disable/enable (archive to `.disabled/`),
//! consolidation of scattered copies, and skill-file read/write.
//! The read/analysis side lives in the sibling modules; everything here
//! mutates disk state and is guarded by the same safety checks the HTTP API
//! relied on: every removed path must resolve to exactly
//! `<skills_dir>/<single-normal-component>`.

use anyhow::{anyhow, Context as _, Result};
use std::fs;
use std::io::Read as _;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

use super::groups;
use super::inspection::{
    self, SkillAgent, SkillAsset, SkillInstallation, SkillsOverview, MANAGED_MARKER,
    MAX_PREVIEW_BYTES,
};
use super::{discovery, repository, scanner};
use crate::storage::atomic_write::write_string_atomic;
use crate::storage::local_store::LocalSqliteStore;

/// Locates the skill and the target agent of an install/uninstall request.
/// Mirrors the HTTP mutation payload: `scope_kind` defaults to `global`, and
/// `workspace_dir` is required for project scope.
#[derive(Clone, Debug)]
pub struct MutationTarget {
    pub skill_id: String,
    pub used_by: String,
    pub source_used_by: Option<String>,
    pub scope_kind: Option<String>,
    pub workspace_dir: Option<String>,
}

/// Locates one file inside a skill bundle, relative to its root.
#[derive(Clone, Debug)]
pub struct SkillFileRef {
    pub skill_id: String,
    pub used_by: Option<String>,
    pub rel_path: String,
}

/// Content of one skill file: `text` for UTF-8 sources, `base64` for binary
/// image previews. Asset metadata (category, size, extension) travels on
/// `asset`.
#[derive(Clone, Debug)]
pub struct SkillFileContent {
    pub asset: SkillAsset,
    pub encoding: String,
    pub mime_type: Option<String>,
    pub content: String,
}

/// One archived skill found under `<agent>/skills/.disabled/`.
#[derive(Clone, Debug)]
pub struct DisabledSkill {
    pub used_by: String,
    pub directory: String,
    pub name: String,
    pub description: Option<String>,
    pub archive_path: String,
}

fn agent_id_for_used_by(used_by: &str) -> &str {
    if used_by == "all" {
        "agents-shared"
    } else {
        used_by
    }
}

fn mutation_scope_kind(target: &MutationTarget) -> &str {
    target.scope_kind.as_deref().unwrap_or("global")
}

pub fn validate_workspace_dir(workspace: &str) -> Result<PathBuf> {
    let path = PathBuf::from(workspace);
    if !path.is_absolute() || !path.is_dir() {
        return Err(anyhow!(
            "workspace_dir must be an existing absolute directory"
        ));
    }
    Ok(path)
}

pub fn push_project_agents(agents: &mut Vec<SkillAgent>, workspace: &Path) {
    for (agent_id, name, _, relative) in discovery::SKILL_AGENTS {
        if agent_id == "agents-shared" {
            continue;
        }
        let skills_dir = workspace.join(relative);
        if agents.iter().any(|agent| {
            agent.agent_id == agent_id
                && agent.scope_kind == "project"
                && agent.workspace_dir.as_deref() == Some(workspace)
        }) {
            continue;
        }
        agents.push(SkillAgent {
            agent_id: agent_id.into(),
            name: name.into(),
            skills_dir,
            scope_kind: "project".into(),
            workspace_dir: Some(workspace.to_path_buf()),
        });
    }
}

fn agents_for_mutation(
    base: &[SkillAgent],
    target: &MutationTarget,
) -> Result<Vec<SkillAgent>> {
    let mut agents = base.to_vec();
    if mutation_scope_kind(target) == "project" {
        let workspace = validate_workspace_dir(
            target
                .workspace_dir
                .as_deref()
                .ok_or_else(|| anyhow!("workspace_dir is required for project scope"))?,
        )?;
        push_project_agents(&mut agents, &workspace);
    }
    Ok(agents)
}

fn find_agent_for_mutation<'a>(
    agents: &'a [SkillAgent],
    target: &MutationTarget,
) -> Result<&'a SkillAgent> {
    let agent_id = agent_id_for_used_by(&target.used_by);
    match mutation_scope_kind(target) {
        "global" => agents
            .iter()
            .find(|agent| agent.agent_id == agent_id && agent.scope_kind == "global")
            .ok_or_else(|| anyhow!("Unknown global agent: {}", target.used_by)),
        "project" => {
            let workspace = validate_workspace_dir(
                target
                    .workspace_dir
                    .as_deref()
                    .ok_or_else(|| anyhow!("workspace_dir is required for project scope"))?,
            )?;
            agents
                .iter()
                .find(|agent| {
                    agent.agent_id == agent_id
                        && agent.scope_kind == "project"
                        && agent.workspace_dir.as_deref() == Some(workspace.as_path())
                })
                .ok_or_else(|| {
                    anyhow!(
                        "Unknown project agent {} for workspace {}",
                        target.used_by,
                        workspace.display()
                    )
                })
        }
        other => Err(anyhow!("Unknown scope_kind: {other}")),
    }
}

fn installation_matches_mutation(
    installation: &SkillInstallation,
    target: &MutationTarget,
) -> bool {
    if installation.used_by != target.used_by {
        return false;
    }
    match mutation_scope_kind(target) {
        "global" => installation.scope_kind == "global",
        "project" => {
            installation.scope_kind == "project"
                && target.workspace_dir.as_deref() == installation.workspace_dir.as_deref()
        }
        _ => false,
    }
}

/// A skill directory name is safe to use in a path join only when it is a
/// single normal component — this blocks `..`, absolute paths, and any other
/// traversal before a filesystem path is built from it.
pub fn validate_directory(directory: &str) -> Result<()> {
    let path = Path::new(directory);
    if directory.is_empty()
        || directory == "."
        || directory == ".."
        || path.components().count() != 1
        || !matches!(path.components().next(), Some(Component::Normal(_)))
    {
        return Err(anyhow!("Unsafe skill directory: {directory}"));
    }
    Ok(())
}

fn open_store(database_path: Option<&Path>) -> Result<LocalSqliteStore> {
    match database_path {
        Some(path) => LocalSqliteStore::open(path),
        None => LocalSqliteStore::open_default(),
    }
}

fn persist_installation_change(
    database_path: Option<&Path>,
    overview: &SkillsOverview,
) -> Result<()> {
    match database_path {
        Some(path) => scanner::persist_path(path, overview, scanner::ScanMode::Incremental),
        None => scanner::persist_default(overview, scanner::ScanMode::Incremental),
    }?;
    Ok(())
}

#[cfg(unix)]
fn create_directory_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, destination)
}

#[cfg(windows)]
fn create_directory_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(source, destination)
}

fn copy_skill(source: &Path, destination: &Path) -> Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("Failed to resolve {}", source.display()))?;
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("Skill destination has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("Failed to create {}", parent.display()))?;
    fs::create_dir(destination)
        .with_context(|| format!("Failed to create {}", destination.display()))?;

    let result = (|| {
        for entry in WalkDir::new(&source).min_depth(1).follow_links(false) {
            let entry = entry?;
            if entry.path().is_symlink() {
                return Err(anyhow!(
                    "Skill contains an unsupported symbolic link: {}",
                    entry.path().display()
                ));
            }
            let relative = entry.path().strip_prefix(&source)?;
            let target = destination.join(relative);
            if entry.file_type().is_dir() {
                fs::create_dir(&target)?;
            } else if entry.file_type().is_file() {
                fs::copy(entry.path(), &target)?;
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(destination);
    }
    result
}

fn deploy_skill(source: &Path, destination: &Path) -> Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("Failed to resolve {}", source.display()))?;
    let parent = destination
        .parent()
        .ok_or_else(|| anyhow!("Skill destination has no parent"))?;
    fs::create_dir_all(parent).with_context(|| format!("Failed to create {}", parent.display()))?;

    if create_directory_symlink(&source, destination).is_ok() {
        return Ok(());
    }

    copy_skill(&source, destination)?;
    if let Err(error) = fs::write(
        destination.join(MANAGED_MARKER),
        b"managed by memorph\n",
    ) {
        let _ = fs::remove_dir_all(destination);
        return Err(error).with_context(|| format!("Failed to mark {}", destination.display()));
    }
    Ok(())
}

/// Deploy a discovered skill into one agent's skills directory (symlink when
/// the platform allows it, managed copy otherwise) and persist the catalog.
pub fn install(
    base_agents: &[SkillAgent],
    database_path: Option<&Path>,
    target: &MutationTarget,
) -> Result<SkillsOverview> {
    let agents = agents_for_mutation(base_agents, target)?;
    let overview = discovery::discover(&agents);
    let skill = overview
        .skills
        .iter()
        .find(|skill| skill.id == target.skill_id)
        .ok_or_else(|| anyhow!("Unknown skill: {}", target.skill_id))?;
    validate_directory(&skill.directory)?;
    let source = match target.source_used_by.as_deref() {
        Some(used_by) => skill
            .installations
            .iter()
            .find(|installation| installation.used_by == used_by)
            .ok_or_else(|| anyhow!("Skill is not installed for source used_by: {used_by}"))?,
        None if skill.conflict => {
            return Err(anyhow!(
                "Skill installations contain different content; source_used_by is required"
            ));
        }
        None => skill
            .installations
            .first()
            .ok_or_else(|| anyhow!("Skill has no source installation"))?,
    };
    let agent = find_agent_for_mutation(&agents, target)?;
    let destination = agent.skills_dir.join(&skill.directory);
    if destination.exists() {
        return Err(anyhow!(
            "Skill {} is already installed for {} ({})",
            skill.name,
            agent.name,
            agent.scope_kind
        ));
    }

    deploy_skill(&source.path, &destination)?;
    let overview = discovery::discover(&agents);
    persist_installation_change(database_path, &overview)?;
    Ok(overview)
}

/// Remove one managed installation (symlink or managed copy). Refuses to
/// touch user-owned skill directories.
pub fn uninstall(
    base_agents: &[SkillAgent],
    database_path: Option<&Path>,
    target: &MutationTarget,
) -> Result<SkillsOverview> {
    let agents = agents_for_mutation(base_agents, target)?;
    let overview = discovery::discover(&agents);
    let skill = overview
        .skills
        .iter()
        .find(|skill| skill.id == target.skill_id)
        .ok_or_else(|| anyhow!("Unknown skill: {}", target.skill_id))?;
    let agent = find_agent_for_mutation(&agents, target)?;
    let installation = skill
        .installations
        .iter()
        .find(|installation| installation_matches_mutation(installation, target))
        .ok_or_else(|| {
            anyhow!(
                "Skill is not installed for {} ({})",
                agent.name,
                agent.scope_kind
            )
        })?;
    let directory = installation
        .path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("Skill installation has no directory name"))?;
    validate_directory(directory)?;
    let expected = agent.skills_dir.join(directory);
    if installation.path != expected || !installation.managed {
        return Err(anyhow!("Refusing to remove a user-owned skill"));
    }
    if expected
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        fs::remove_file(&expected)
            .with_context(|| format!("Failed to remove symbolic link {}", expected.display()))?;
    } else if expected.join(MANAGED_MARKER).is_file() {
        fs::remove_dir_all(&expected)
            .with_context(|| format!("Failed to remove {}", expected.display()))?;
    } else {
        return Err(anyhow!("Refusing to remove a user-owned skill"));
    }
    let overview = discovery::discover(&agents);
    persist_installation_change(database_path, &overview)?;
    Ok(overview)
}

pub fn installation_to_catalog_record(
    installation: &SkillInstallation,
) -> repository::CatalogInstallation {
    repository::CatalogInstallation {
        used_by: installation.used_by.clone(),
        scope_kind: installation.scope_kind.clone(),
        workspace_dir: installation.workspace_dir.clone(),
        install_path: installation.path.to_string_lossy().into_owned(),
        install_kind: match installation.deployment_mode.as_str() {
            "symlink" => "symlink",
            "copy" => "managed-copy",
            _ => "directory",
        }
        .into(),
        symlink_target: installation.symlink_target.clone(),
        link_status: installation.link_status.clone(),
        status: "active".into(),
    }
}

/// Remove a specific set of installations from disk — the installations
/// belonging to one catalog row. Symlinks and the real source directory are
/// both removed; unlike [`uninstall`], user-owned directories (the real source)
/// go too, which is what "delete this copy" has to mean. Safety: each path is
/// checked to be exactly `<skills_dir>/<validated-directory>` before removal, so
/// we never follow a stray path elsewhere. Removal is driven by the actual
/// filesystem type (`symlink_metadata`), so the stored `install_kind` is only
/// informational.
pub fn remove_catalog_installations(
    agents: &[SkillAgent],
    installations: &[repository::CatalogInstallation],
) -> Result<()> {
    for installation in installations {
        let path = Path::new(&installation.install_path);
        let agent = agents
            .iter()
            .find(|agent| agent.agent_id == agent_id_for_used_by(&installation.used_by))
            .ok_or_else(|| anyhow!("Unknown agent for installation: {}", installation.used_by))?;
        let directory = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow!("Skill installation has no directory name"))?;
        validate_directory(directory)?;
        let expected = agent.skills_dir.join(directory);
        if path != expected {
            return Err(anyhow!(
                "Refusing to delete {}: expected it under {}",
                path.display(),
                agent.skills_dir.display()
            ));
        }
        let is_link = path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink());
        if is_link {
            // remove_file on a symlink removes the link itself, not its target.
            fs::remove_file(path)
                .with_context(|| format!("Failed to remove symlink {}", path.display()))?;
        } else if path.is_dir() {
            fs::remove_dir_all(path)
                .with_context(|| format!("Failed to remove {}", path.display()))?;
        }
    }
    Ok(())
}

/// Remove one installation from disk. Deleting a real directory also removes
/// every symlink in the same logical skill that resolves to it. Returns the
/// normalized skill name so callers can purge superseded catalog rows.
pub fn delete_installation_at_path(agents: &[SkillAgent], install_path: &str) -> Result<String> {
    let overview = discovery::discover(agents);
    let target = PathBuf::from(install_path);
    let skill = overview
        .skills
        .iter()
        .find(|skill| skill.installations.iter().any(|item| item.path == target))
        .ok_or_else(|| anyhow!("Unknown installation path: {install_path}"))?;
    let is_link = target
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink());
    let mut to_remove = Vec::new();
    if is_link {
        let installation = skill
            .installations
            .iter()
            .find(|item| item.path == target)
            .ok_or_else(|| anyhow!("Unknown installation path: {install_path}"))?;
        to_remove.push(installation.clone());
    } else {
        let canonical = target
            .canonicalize()
            .with_context(|| format!("Failed to resolve {install_path}"))?;
        for installation in &skill.installations {
            if installation.path == target {
                to_remove.push(installation.clone());
                continue;
            }
            let other_is_link = installation
                .path
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink());
            if other_is_link
                && installation
                    .path
                    .canonicalize()
                    .is_ok_and(|resolved| resolved == canonical)
            {
                to_remove.push(installation.clone());
            }
        }
    }
    let records = to_remove
        .iter()
        .map(installation_to_catalog_record)
        .collect::<Vec<_>>();
    remove_catalog_installations(agents, &records)?;
    Ok(skill.id.clone())
}

/// Delete one installation (and the symlinks pointing at it), persist the
/// catalog, and drop catalog rows left without an active installation.
pub fn delete_installation(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    install_path: &str,
) -> Result<SkillsOverview> {
    let normalized_name = delete_installation_at_path(agents, install_path)?;
    let overview = discovery::discover(agents);
    persist_installation_change(database_path, &overview)?;
    if let Ok(mut store) = open_store(database_path) {
        let _ = repository::delete_inactive_catalog_rows_for_name(
            store.connection_mut(),
            &normalized_name,
        );
    }
    Ok(overview)
}

/// Delete one catalog row and every installation it records, then return the
/// post-deletion overview. The catalog row is removed by id directly, so no
/// scan persist is needed.
pub fn delete_skill(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    skill_id: &str,
) -> Result<SkillsOverview> {
    // Per-row delete: `skill_id` is the catalog row's hash id (one list entry).
    // Load that row's installations, remove only those from disk, then drop the
    // catalog row + children by id — sibling copies under the same name survive.
    let result: Result<()> = (|| {
        let mut store = open_store(database_path)?;
        let item = repository::get_catalog_item(store.connection(), skill_id)?
            .ok_or_else(|| anyhow!("Unknown skill: {skill_id}"))?;
        remove_catalog_installations(agents, &item.installations)?;
        repository::delete_skill(store.connection_mut(), skill_id)?;
        Ok(())
    })();
    result?;
    Ok(discovery::discover(agents))
}

/// Archive a skill's canonical directory into `<agent>/skills/.disabled/` and
/// remove every other installation. The real directory is renamed in place
/// (same filesystem → O(1)), so a large skill is not copied. The moved source
/// is gone from its old path, so [`remove_catalog_installations`] silently
/// skips it while clearing the remaining symlinks and managed copies.
fn archive_skill(agents: &[SkillAgent], item: &repository::CatalogItem) -> Result<()> {
    let real = item
        .installations
        .iter()
        .find(|installation| {
            installation.status == "active"
                && matches!(
                    installation.install_kind.as_str(),
                    "directory" | "managed-copy"
                )
        })
        .ok_or_else(|| anyhow!("Cannot disable: no managed source directory to archive"))?;
    let real_path = Path::new(&real.install_path);
    let directory = real_path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| anyhow!("Skill installation has no directory name"))?;
    validate_directory(directory)?;
    let agent = agents
        .iter()
        .find(|agent| agent.agent_id == agent_id_for_used_by(&real.used_by))
        .ok_or_else(|| anyhow!("Unknown agent: {}", real.used_by))?;
    let archive_root = agent.skills_dir.join(".disabled");
    fs::create_dir_all(&archive_root)
        .with_context(|| format!("Failed to create {}", archive_root.display()))?;
    let archive_target = archive_root.join(directory);
    if archive_target.exists() {
        return Err(anyhow!(
            "Skill is already archived at {}",
            archive_target.display()
        ));
    }
    fs::rename(real_path, &archive_target)
        .with_context(|| format!("Failed to archive {}", real_path.display()))?;
    remove_catalog_installations(agents, &item.installations)?;
    Ok(())
}

/// Disable = delete from the catalog, but preserve the files in `.disabled/`.
/// The skill vanishes from the list (no misleading "missing" badge); the
/// archive folder is the durable record and survives a catalog rebuild.
/// The trailing persist syncs the scan fingerprint to the post-archive disk
/// state, so a later `enable` is detected as a change instead of skipped.
pub fn disable_skill(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    skill_id: &str,
) -> Result<SkillsOverview> {
    let result: Result<()> = (|| {
        let mut store = open_store(database_path)?;
        let item = repository::get_catalog_item(store.connection(), skill_id)?
            .ok_or_else(|| anyhow!("Unknown skill: {skill_id}"))?;
        archive_skill(agents, &item)?;
        repository::delete_skill(store.connection_mut(), skill_id)?;
        Ok(())
    })();
    let overview = discovery::discover(agents);
    result?;
    let _ = persist_installation_change(database_path, &overview);
    Ok(overview)
}

/// Restore an archived skill: move it back out of `.disabled/` into its
/// agent's skills dir, then rescan so the catalog row is recreated.
pub fn enable_skill(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    used_by: &str,
    directory: &str,
) -> Result<SkillsOverview> {
    let result: Result<()> = (|| {
        let agent = agents
            .iter()
            .find(|agent| agent.agent_id == agent_id_for_used_by(used_by))
            .ok_or_else(|| anyhow!("Unknown agent: {used_by}"))?;
        validate_directory(directory)?;
        let archive_path = agent.skills_dir.join(".disabled").join(directory);
        if !archive_path.join("SKILL.md").is_file() {
            return Err(anyhow!("No archived skill at {}", archive_path.display()));
        }
        let target = agent.skills_dir.join(directory);
        if target.exists() {
            return Err(anyhow!(
                "A skill named '{}' is already active in {}",
                directory,
                agent.skills_dir.display()
            ));
        }
        fs::rename(&archive_path, &target)
            .with_context(|| format!("Failed to restore {}", archive_path.display()))?;
        Ok(())
    })();
    result?;
    let overview = discovery::discover(agents);
    let _ = persist_installation_change(database_path, &overview);
    Ok(overview)
}

/// Scan each global agent's `.disabled/` folder one level deep. The archive
/// folder is the single source of truth for disabled skills: anything parked
/// here is disabled, independent of the catalog, so the list survives a full
/// rebuild.
pub fn list_disabled(agents: &[SkillAgent]) -> Vec<DisabledSkill> {
    let mut items = Vec::new();
    for agent in agents.iter() {
        if agent.scope_kind != "global" {
            continue;
        }
        let archive_root = agent.skills_dir.join(".disabled");
        let Ok(children) = fs::read_dir(&archive_root) else {
            continue;
        };
        for child in children.flatten() {
            let path = child.path();
            let is_hidden = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('.'));
            if is_hidden || !path.is_dir() || !path.join("SKILL.md").is_file() {
                continue;
            }
            let Some(directory) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let frontmatter = inspection::read_frontmatter(&path.join("SKILL.md"));
            let name = frontmatter
                .get("name")
                .cloned()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| directory.to_string());
            let description = frontmatter
                .get("description")
                .cloned()
                .filter(|value| !value.is_empty());
            items.push(DisabledSkill {
                used_by: if agent.agent_id == "agents-shared" {
                    "all".into()
                } else {
                    agent.agent_id.clone()
                },
                directory: directory.to_string(),
                name,
                description,
                archive_path: path.to_string_lossy().into_owned(),
            });
        }
    }
    items.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.directory.cmp(&b.directory))
    });
    items
}

/// Repoint every installation of a logical skill to one canonical directory.
/// Discovery merges installations by normalized name, so this reaches every
/// scattered copy — independent directories across agents collapse into
/// symlinks at the chosen canonical path, and the catalog rows merge on the
/// next scan. The picked installation may itself be a symlink; it is resolved
/// to its target, which becomes the canonical real directory.
fn consolidate_to_canonical(
    overview: &SkillsOverview,
    canonical_install_path: &str,
) -> Result<String> {
    let agents = &overview.agents;
    let canonical_path = Path::new(canonical_install_path);
    let canonical_real = canonical_path
        .canonicalize()
        .with_context(|| format!("Failed to resolve canonical path {canonical_install_path}"))?;
    if !canonical_real.join("SKILL.md").is_file() {
        return Err(anyhow!(
            "Canonical path is not a skill directory: {}",
            canonical_real.display()
        ));
    }
    let skill = overview
        .skills
        .iter()
        .find(|skill| {
            skill.installations.iter().any(|installation| {
                installation.path == canonical_path || installation.path == canonical_real
            })
        })
        .ok_or_else(|| anyhow!("Canonical path is not a known skill installation"))?;
    for installation in &skill.installations {
        let path = &installation.path;
        // Already resolves to the canonical real directory — leave it. This
        // covers the picked installation, its target, and any symlink already
        // pointing there.
        let resolved = path.canonicalize().unwrap_or_else(|_| path.clone());
        if resolved == canonical_real {
            continue;
        }
        let agent = agents
            .iter()
            .find(|agent| agent.agent_id == agent_id_for_used_by(&installation.used_by))
            .ok_or_else(|| anyhow!("Unknown agent: {}", installation.used_by))?;
        let directory = path
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| anyhow!("Skill installation has no directory name"))?;
        validate_directory(directory)?;
        if path != &agent.skills_dir.join(directory) {
            return Err(anyhow!(
                "Refusing to consolidate {}: expected it under {}",
                path.display(),
                agent.skills_dir.display()
            ));
        }
        if path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            fs::remove_file(path)
                .with_context(|| format!("Failed to remove symlink {}", path.display()))?;
        } else if path.is_dir() {
            fs::remove_dir_all(path)
                .with_context(|| format!("Failed to remove {}", path.display()))?;
        }
        create_directory_symlink(&canonical_real, path).with_context(|| {
            format!(
                "Failed to link {} -> {}",
                path.display(),
                canonical_real.display()
            )
        })?;
    }
    Ok(skill.id.clone())
}

/// Consolidate scattered copies of one skill onto a canonical directory,
/// persist the merged state, carry group assignments over to the surviving
/// row, and purge the superseded catalog rows.
pub fn consolidate(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    canonical_install_path: &str,
) -> Result<SkillsOverview> {
    let before = discovery::discover(agents);
    let normalized_name = consolidate_to_canonical(&before, canonical_install_path)?;
    let overview = discovery::discover(agents);
    let _ = persist_installation_change(database_path, &overview);
    // Scattered copies merged into one canonical row leave the superseded
    // rows without an active installation; purge them so they don't linger
    // as "missing" entries next to the merged skill.
    if let Ok(mut store) = open_store(database_path) {
        let conn = store.connection_mut();
        // Before the merged-away rows are deleted, move their group
        // assignments onto the surviving skill so consolidation does
        // not silently drop a user's grouping.
        let inactive_ids =
            repository::inactive_catalog_ids_for_name(conn, &normalized_name)
                .unwrap_or_default();
        if let Ok(Some(survivor)) =
            repository::active_catalog_id_for_name(conn, &normalized_name)
        {
            let _ = groups::migrate_members(conn, &inactive_ids, &survivor);
        }
        let _ = repository::delete_inactive_catalog_rows_for_name(conn, &normalized_name);
    }
    Ok(overview)
}

/// Strip every symlink installation of one catalog row's skill; real
/// directories and managed copies stay in place.
pub fn remove_symlinks(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    skill_id: &str,
) -> Result<SkillsOverview> {
    let result: Result<()> = (|| {
        let store = open_store(database_path)?;
        let item = repository::get_catalog_item(store.connection(), skill_id)?
            .ok_or_else(|| anyhow!("Unknown skill: {skill_id}"))?;
        let symlinks: Vec<repository::CatalogInstallation> = item
            .installations
            .iter()
            .filter(|installation| installation.install_kind == "symlink")
            .cloned()
            .collect();
        remove_catalog_installations(agents, &symlinks)?;
        Ok(())
    })();
    let overview = discovery::discover(agents);
    result?;
    let _ = persist_installation_change(database_path, &overview);
    Ok(overview)
}

/// Locate a skill's bundle directory: from the live filesystem overview
/// first, then from the catalog (via `database_path`) for archived rows.
pub fn resolve_skill_bundle_path(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    skill_id: &str,
    used_by: Option<&str>,
) -> Result<PathBuf> {
    let overview = discovery::discover(agents);
    if let Some(skill) = overview.skills.iter().find(|skill| skill.id == skill_id) {
        let installation = match used_by {
            Some(agent) => skill
                .installations
                .iter()
                .find(|item| item.used_by == agent)
                .ok_or_else(|| anyhow!("Skill is not installed for {agent}"))?,
            None => skill
                .installations
                .first()
                .ok_or_else(|| anyhow!("Skill has no installation"))?,
        };
        return Ok(installation.path.clone());
    }

    let item = match database_path {
        Some(path) => repository::get_catalog_item_path(path, skill_id)?,
        None => repository::get_catalog_item_default(skill_id)?,
    }
    .ok_or_else(|| anyhow!("Unknown skill: {skill_id}"))?;
    let installation = match used_by {
        Some(agent) => item
            .installations
            .iter()
            .find(|item| item.status == "active" && item.used_by == agent)
            .or_else(|| item.installations.iter().find(|item| item.used_by == agent))
            .ok_or_else(|| anyhow!("Skill is not installed for {agent}"))?,
        None => item
            .installations
            .iter()
            .find(|item| item.status == "active")
            .or_else(|| item.installations.first())
            .ok_or_else(|| anyhow!("Skill has no installation"))?,
    };
    Ok(PathBuf::from(&installation.install_path))
}

fn resolve_skill_file(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    skill_id: &str,
    rel_path: &str,
    used_by: Option<&str>,
) -> Result<(SkillAsset, PathBuf)> {
    let bundle_path = resolve_skill_bundle_path(agents, database_path, skill_id, used_by)?;
    let relative = Path::new(rel_path);
    if relative.is_absolute()
        || relative.components().count() == 0
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(anyhow!("Unsafe skill file path"));
    }
    let inspection = inspection::inspect_bundle(&bundle_path);
    let asset = inspection
        .assets
        .iter()
        .find(|asset| asset.path == rel_path)
        .ok_or_else(|| anyhow!("Unknown skill asset: {rel_path}"))?
        .clone();
    if !asset.previewable {
        return Err(anyhow!("Skill asset is not previewable"));
    }
    let root = bundle_path.canonicalize()?;
    let target = root.join(relative).canonicalize()?;
    if !target.starts_with(&root) {
        return Err(anyhow!("Skill file escapes its bundle"));
    }
    Ok((asset, target))
}

fn image_mime_type(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// Read one previewable file from a skill bundle. Text files come back as
/// UTF-8, images as base64.
pub fn read_skill_file(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    file: &SkillFileRef,
) -> Result<SkillFileContent> {
    let (asset, target) = resolve_skill_file(
        agents,
        database_path,
        &file.skill_id,
        &file.rel_path,
        file.used_by.as_deref(),
    )?;
    let file_handle = fs::File::open(&target)?;
    let mut bytes = Vec::new();
    file_handle
        .take(MAX_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_PREVIEW_BYTES {
        return Err(anyhow!("Skill asset exceeds preview limit"));
    }

    let extension = asset.extension.as_deref().unwrap_or("");
    let (encoding, mime_type, content) = if inspection::is_image_extension(extension) {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        (
            "base64".to_string(),
            Some(image_mime_type(extension).to_string()),
            STANDARD.encode(&bytes),
        )
    } else {
        let text = String::from_utf8(bytes).map_err(|_| anyhow!("Skill asset is not UTF-8 text"))?;
        ("text".to_string(), None, text)
    };

    Ok(SkillFileContent {
        asset,
        encoding,
        mime_type,
        content,
    })
}

/// Overwrite one text file inside a skill bundle atomically and return the
/// freshly read content.
pub fn write_skill_file(
    agents: &[SkillAgent],
    database_path: Option<&Path>,
    file: &SkillFileRef,
    content: &str,
) -> Result<SkillFileContent> {
    let (asset, target) = resolve_skill_file(
        agents,
        database_path,
        &file.skill_id,
        &file.rel_path,
        file.used_by.as_deref(),
    )?;
    let extension = asset.extension.as_deref().unwrap_or("");
    if inspection::is_image_extension(extension) {
        return Err(anyhow!("Skill asset is not editable"));
    }
    if content.len() as u64 > MAX_PREVIEW_BYTES {
        return Err(anyhow!("Skill asset exceeds preview limit"));
    }
    if content.contains('\0') {
        return Err(anyhow!("Skill asset is not UTF-8 text"));
    }
    write_string_atomic(&target, content)?;
    read_skill_file(agents, database_path, file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use repository::CatalogQuery;

    fn agents(root: &Path) -> Vec<SkillAgent> {
        [
            ("claude", "Claude Code"),
            ("codex", "Codex"),
            ("gemini", "Gemini CLI"),
        ]
        .into_iter()
        .map(|(agent_id, name)| SkillAgent {
            agent_id: agent_id.into(),
            name: name.into(),
            skills_dir: root.join(agent_id).join("skills"),
            scope_kind: "global".into(),
            workspace_dir: None,
        })
        .collect()
    }

    fn create_skill(root: &Path, provider: &str, directory: &str, contents: &str) -> PathBuf {
        let path = root.join(provider).join("skills").join(directory);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("SKILL.md"), contents).unwrap();
        path
    }

    fn catalog_id(database_path: &Path) -> String {
        let page = repository::list_catalog_path(
            database_path,
            &CatalogQuery {
                page: 1,
                page_size: 50,
                ..CatalogQuery::default()
            },
        )
        .unwrap();
        page.items[0].id.clone()
    }

    #[test]
    fn install_deploys_skill_and_rejects_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let source = create_skill(root.path(), "claude", "writer", "---\nname: Writer\n---\n");
        fs::write(source.join("example.txt"), "example").unwrap();
        let agents = agents(root.path());
        let database_path = root.path().join("skills-test.db");
        let target = MutationTarget {
            skill_id: "writer".into(),
            used_by: "codex".into(),
            source_used_by: None,
            scope_kind: None,
            workspace_dir: None,
        };

        let overview = install(&agents, Some(&database_path), &target).unwrap();
        let installed = root.path().join("codex/skills/writer");
        assert_eq!(
            fs::read_to_string(installed.join("example.txt")).unwrap(),
            "example"
        );
        let installation = overview.skills[0]
            .installations
            .iter()
            .find(|item| item.used_by == "codex")
            .unwrap();
        assert!(installation.managed);
        assert!(matches!(
            installation.deployment_mode.as_str(),
            "symlink" | "copy"
        ));
        assert!(installation.link_valid);
        assert_eq!(overview.skills[0].installations.len(), 2);
        assert!(install(&agents, Some(&database_path), &target).is_err());
        // The catalog was persisted alongside the deployment.
        let page = repository::list_catalog_path(
            &database_path,
            &CatalogQuery {
                page: 1,
                page_size: 50,
                ..CatalogQuery::default()
            },
        )
        .unwrap();
        assert_eq!(page.total, 1);
    }

    #[test]
    fn copy_skill_rejects_symbolic_links_and_copies_recursively() {
        let root = tempfile::tempdir().unwrap();
        let source = create_skill(root.path(), "claude", "writer", "# Writer");
        fs::create_dir_all(source.join("scripts")).unwrap();
        fs::write(source.join("scripts/run.sh"), "echo ok").unwrap();
        let destination = root.path().join("codex/skills/writer");

        copy_skill(&source, &destination).unwrap();
        assert_eq!(
            fs::read_to_string(destination.join("scripts/run.sh")).unwrap(),
            "echo ok"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let nested = create_skill(root.path(), "gemini", "linked", "# Linked");
            symlink(&nested, nested.join("outside")).unwrap();
            let refused = root.path().join("codex/skills/linked");
            assert!(copy_skill(&nested, &refused).is_err());
            assert!(!refused.exists(), "failed copy must clean up after itself");
        }
    }

    #[test]
    fn uninstall_removes_managed_copy_but_refuses_user_owned_skill() {
        let root = tempfile::tempdir().unwrap();
        create_skill(root.path(), "claude", "writer", "# Writer");
        let agents = agents(root.path());
        let database_path = root.path().join("skills-test.db");
        let managed = MutationTarget {
            skill_id: "writer".into(),
            used_by: "codex".into(),
            source_used_by: None,
            scope_kind: None,
            workspace_dir: None,
        };
        install(&agents, Some(&database_path), &managed).unwrap();
        uninstall(&agents, Some(&database_path), &managed).unwrap();
        assert!(!root.path().join("codex/skills/writer").exists());

        let user_owned = MutationTarget {
            used_by: "claude".into(),
            ..managed
        };
        assert!(uninstall(&agents, Some(&database_path), &user_owned).is_err());
        assert!(root.path().join("claude/skills/writer").exists());
    }

    #[test]
    fn delete_installation_removes_real_directory_and_its_symlinks() {
        let root = tempfile::tempdir().unwrap();
        // Real, user-owned source under claude — the case uninstall refuses.
        create_skill(root.path(), "claude", "writer", "# Writer");
        let agents = agents(root.path());
        let database_path = root.path().join("skills-test.db");
        let managed = MutationTarget {
            skill_id: "writer".into(),
            used_by: "gemini".into(),
            source_used_by: None,
            scope_kind: None,
            workspace_dir: None,
        };
        install(&agents, Some(&database_path), &managed).unwrap();
        assert!(root.path().join("claude/skills/writer").exists());
        assert!(root.path().join("gemini/skills/writer").exists());

        // remove_catalog_installations takes the row's installation records and
        // removes exactly those paths: the gemini deployment AND the
        // user-owned claude source directory. Removal follows the real
        // filesystem type, so install_kind here is informational.
        let installations = vec![
            repository::CatalogInstallation {
                used_by: "claude".into(),
                scope_kind: "global".into(),
                workspace_dir: None,
                install_path: root
                    .path()
                    .join("claude/skills/writer")
                    .to_string_lossy()
                    .into_owned(),
                install_kind: "directory".into(),
                symlink_target: None,
                link_status: "not-applicable".into(),
                status: "active".into(),
            },
            repository::CatalogInstallation {
                used_by: "gemini".into(),
                scope_kind: "global".into(),
                workspace_dir: None,
                install_path: root
                    .path()
                    .join("gemini/skills/writer")
                    .to_string_lossy()
                    .into_owned(),
                install_kind: "directory".into(),
                symlink_target: None,
                link_status: "not-applicable".into(),
                status: "active".into(),
            },
        ];
        remove_catalog_installations(&agents, &installations).unwrap();
        assert!(
            !root.path().join("gemini/skills/writer").exists(),
            "symlink/copy installation should be removed"
        );
        assert!(
            !root.path().join("claude/skills/writer").exists(),
            "user-owned source directory should be removed"
        );
    }

    #[test]
    fn disable_archives_to_disabled_folder_and_enable_restores() {
        let root = tempfile::tempdir().unwrap();
        create_skill(
            root.path(),
            "claude",
            "writer",
            "---\nname: Writer\ndescription: Writes docs\n---\n# Writer",
        );
        let agents = agents(root.path());
        let database_path = root.path().join("skills-test.db");
        // Deploy a symlink to gemini so the skill has a real directory
        // (claude) plus a linked installation, and persist the catalog.
        install(
            &agents,
            Some(&database_path),
            &MutationTarget {
                skill_id: "writer".into(),
                used_by: "gemini".into(),
                source_used_by: Some("claude".into()),
                scope_kind: None,
                workspace_dir: None,
            },
        )
        .unwrap();
        let skill_id = catalog_id(&database_path);

        let real_dir = root.path().join("claude/skills/writer");
        let gemini_link = root.path().join("gemini/skills/writer");
        let archive = root
            .path()
            .join("claude/skills/.disabled/writer");

        // Disable: real dir moves into .disabled/, symlink removed, catalog row deleted.
        disable_skill(&agents, Some(&database_path), &skill_id).unwrap();
        assert!(archive.join("SKILL.md").is_file(), "skill must be archived");
        assert!(!real_dir.exists(), "real dir must have moved");
        assert!(
            gemini_link.symlink_metadata().is_err(),
            "gemini installation must be removed"
        );

        let disabled = list_disabled(&agents);
        assert_eq!(disabled.len(), 1);
        assert_eq!(disabled[0].used_by, "claude");
        assert_eq!(disabled[0].directory, "writer");
        assert_eq!(disabled[0].name, "Writer");
        assert_eq!(disabled[0].description.as_deref(), Some("Writes docs"));

        // Enable: move back to the original location.
        enable_skill(&agents, Some(&database_path), "claude", "writer").unwrap();
        assert!(
            real_dir.join("SKILL.md").is_file(),
            "skill restored in place"
        );
        assert!(!archive.exists(), "archive folder emptied");
        assert!(list_disabled(&agents).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn consolidate_merges_independent_copies_into_symlinks() {
        let root = tempfile::tempdir().unwrap();
        create_skill(
            root.path(),
            "claude",
            "writer",
            "---\nname: Writer\n---\n# Claude copy",
        );
        create_skill(
            root.path(),
            "codex",
            "writer",
            "---\nname: Writer\n---\n# Codex copy",
        );
        let agents = agents(root.path());
        let database_path = root.path().join("skills-test.db");
        // Two independent copies surface as two catalog rows.
        let overview = discovery::discover(&agents);
        scanner::persist_path(&database_path, &overview, scanner::ScanMode::Full).unwrap();

        let claude_path = root.path().join("claude/skills/writer");
        let codex_path = root.path().join("codex/skills/writer");

        // Pick claude's copy as canonical; codex must collapse into a symlink.
        consolidate(
            &agents,
            Some(&database_path),
            &claude_path.to_string_lossy(),
        )
        .unwrap();
        assert!(codex_path
            .symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink()));
        assert_eq!(
            fs::read_link(&codex_path).unwrap(),
            claude_path.canonicalize().unwrap()
        );
        assert!(
            claude_path.join("SKILL.md").is_file(),
            "canonical copy kept"
        );

        // Symlinked copies merge into one catalog row; superseded rows purged.
        let page = repository::list_catalog_path(
            &database_path,
            &CatalogQuery {
                page: 1,
                page_size: 50,
                ..CatalogQuery::default()
            },
        )
        .unwrap();
        assert_eq!(page.total, 1, "merged copies share one catalog row");
    }

    #[test]
    fn read_write_skill_file_rejects_traversal_and_handles_images() {
        let root = tempfile::tempdir().unwrap();
        let skill = create_skill(root.path(), "claude", "writer", "# Writer");
        fs::write(skill.join("image.bin"), [0_u8, 159, 146, 150]).unwrap();
        fs::create_dir_all(skill.join("assets")).unwrap();
        // Minimal 1x1 PNG
        let png = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xFE,
            0x02, 0xFE, 0xDC, 0xCC, 0x59, 0xE7, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
            0xAE, 0x42, 0x60, 0x82,
        ];
        fs::write(skill.join("assets/icon.png"), png).unwrap();
        let agents = agents(root.path());
        let file = |path: &str| SkillFileRef {
            skill_id: "writer".into(),
            used_by: None,
            rel_path: path.into(),
        };

        assert!(read_skill_file(&agents, None, &file("../secret")).is_err());
        assert!(read_skill_file(&agents, None, &file("image.bin")).is_err());
        let preview = read_skill_file(&agents, None, &file("SKILL.md")).unwrap();
        assert_eq!(preview.encoding, "text");
        assert_eq!(preview.content, "# Writer");

        let image = read_skill_file(&agents, None, &file("assets/icon.png")).unwrap();
        assert_eq!(image.encoding, "base64");
        assert_eq!(image.mime_type.as_deref(), Some("image/png"));
        assert!(!image.content.is_empty());

        let updated = write_skill_file(&agents, None, &file("SKILL.md"), "# Writer Updated")
            .unwrap();
        assert_eq!(updated.content, "# Writer Updated");
        assert_eq!(
            fs::read_to_string(skill.join("SKILL.md")).unwrap(),
            "# Writer Updated"
        );
        assert!(write_skill_file(&agents, None, &file("assets/icon.png"), "x").is_err());
    }
}
