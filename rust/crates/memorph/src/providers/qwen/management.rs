use super::{direct_child_directories, is_session_id, parse_jsonl_session, PROVIDER_ID};
use crate::provider::{ProviderSessionBackup, ProviderSourceMutation};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

const BACKUP_FORMAT: &str = "qwen-code-native-session-backup-v1";
const BACKUP_MIME: &str = "application/vnd.memorph.qwen-code-native-session-backup";
const BACKUP_SOURCE_DIR: &str = "source";
const BACKUP_METADATA_FILE: &str = "metadata.json";
const FILE_HISTORY_DIR: &str = "file-history";
const ORGANIZATION_FILE: &str = "session-organization.v1.json";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum ArtifactRoot {
    Runtime,
    GlobalQwen,
}

impl ArtifactRoot {
    fn directory_name(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::GlobalQwen => "global_qwen",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct SelectedArtifact {
    root: ArtifactRoot,
    relative_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct QwenSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    runtime_root: PathBuf,
    global_qwen_dir: PathBuf,
    project_dir: PathBuf,
    source_path: PathBuf,
    selected_artifacts: Vec<SelectedArtifact>,
    artifact_digest: String,
    entry_count: usize,
    byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ArtifactInspection {
    digest: String,
    entry_count: usize,
    byte_size: u64,
}

#[derive(Debug, Clone)]
struct RootLocations {
    runtime: PathBuf,
    global_qwen: PathBuf,
}

impl RootLocations {
    fn path(&self, artifact: &SelectedArtifact) -> PathBuf {
        match artifact.root {
            ArtifactRoot::Runtime => self.runtime.join(&artifact.relative_path),
            ArtifactRoot::GlobalQwen => self.global_qwen.join(&artifact.relative_path),
        }
    }
}

#[derive(Debug)]
struct SessionSource {
    path: PathBuf,
    parsed: super::ParsedQwenSession,
}

#[derive(Debug)]
struct SessionSources {
    roots: RootLocations,
    project_dir: PathBuf,
    active: Option<SessionSource>,
    archived: Option<SessionSource>,
}

pub(super) fn delete_session(session_id: &str) -> Result<()> {
    let sources = discover_sources(session_id)?;
    let artifacts = selected_artifacts(&sources, ProviderSourceMutation::Delete)?;
    inspect_artifacts(&sources.roots, &artifacts)?;

    for artifact in artifacts.iter().filter(|artifact| {
        !matches!(
            artifact
                .relative_path
                .file_name()
                .and_then(|name| name.to_str()),
            Some(ORGANIZATION_FILE)
        )
    }) {
        remove_entry_if_exists(&sources.roots.path(artifact))?;
    }
    remove_session_from_organization(&sources.project_dir.join(ORGANIZATION_FILE), session_id)?;
    super::fail_qwen_mutation_after_write(ProviderSourceMutation::Delete)
}

pub(super) fn rename_session(session_id: &str, new_title: &str) -> Result<()> {
    let sources = discover_sources(session_id)?;
    let active = sources
        .active
        .as_ref()
        .context("Qwen Code rename only supports an active session source")?;
    let first = active
        .parsed
        .records
        .first()
        .context("Qwen Code session has no records")?;
    let last = active
        .parsed
        .records
        .last()
        .context("Qwen Code session has no records")?;
    let parent_uuid = last.get("uuid").cloned().unwrap_or(Value::Null);
    let cwd = first
        .get("cwd")
        .and_then(Value::as_str)
        .context("Qwen Code session first record has no cwd")?;
    let version = first.get("version").cloned().unwrap_or(Value::Null);

    let title_record = serde_json::json!({
        "uuid": uuid::Uuid::new_v4().to_string(),
        "parentUuid": parent_uuid,
        "sessionId": session_id,
        "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "type": "system",
        "subtype": "custom_title",
        "cwd": cwd,
        "version": version,
        "systemPayload": {
            "customTitle": new_title,
            "titleSource": "manual"
        }
    });

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&active.path)
        .with_context(|| {
            format!(
                "Failed to open Qwen Code session for rename: {}",
                active.path.display()
            )
        })?;
    serde_json::to_writer(&mut file, &title_record)?;
    use std::io::Write;
    file.write_all(b"\n")?;
    file.sync_data()?;
    super::fail_qwen_mutation_after_write(ProviderSourceMutation::Rename)
}

pub(super) fn create_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    validate_safe_component(operation_id, "operation id")?;
    let sources = discover_sources(session_id)?;
    let artifacts = selected_artifacts(&sources, mutation)?;
    let before = inspect_artifacts(&sources.roots, &artifacts)?;
    let backup_path = backup_root
        .join(PROVIDER_ID)
        .join(operation_id)
        .join(session_id);
    if std::fs::symlink_metadata(&backup_path).is_ok() {
        bail!(
            "Qwen Code backup path already exists: {}",
            backup_path.display()
        );
    }

    let backup_source = backup_path.join(BACKUP_SOURCE_DIR);
    let backup_roots = RootLocations {
        runtime: backup_source.join(ArtifactRoot::Runtime.directory_name()),
        global_qwen: backup_source.join(ArtifactRoot::GlobalQwen.directory_name()),
    };
    if let Err(error) = copy_artifacts(&sources.roots, &backup_roots, &artifacts) {
        let _ = std::fs::remove_dir_all(&backup_path);
        return Err(error);
    }
    let after = inspect_artifacts(&sources.roots, &artifacts)?;
    let copied = inspect_artifacts(&backup_roots, &artifacts)?;
    if before != after || before != copied {
        let _ = std::fs::remove_dir_all(&backup_path);
        bail!("Qwen Code source changed while its native backup was being captured");
    }

    let runtime_root = sources.roots.runtime.clone();
    let global_qwen_dir = sources.roots.global_qwen.clone();
    let source_path = sources
        .active
        .as_ref()
        .or(sources.archived.as_ref())
        .map(|source| source.path.clone())
        .context("Qwen Code session has no source path")?;
    let metadata = QwenSessionBackupMetadata {
        version: 1,
        provider_id: PROVIDER_ID.to_string(),
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        runtime_root,
        global_qwen_dir,
        project_dir: sources.project_dir.clone(),
        source_path: source_path.clone(),
        selected_artifacts: artifacts.clone(),
        artifact_digest: before.digest.clone(),
        entry_count: before.entry_count,
        byte_size: before.byte_size,
    };
    std::fs::write(
        backup_path.join(BACKUP_METADATA_FILE),
        serde_json::to_vec_pretty(&metadata)?,
    )?;

    Ok(ProviderSessionBackup {
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        source_path,
        backup_path: backup_path.clone(),
        restore_hint:
            "Restore this exact Qwen Code native session bundle before reopening Qwen Code."
                .to_string(),
        mime_type: BACKUP_MIME.to_string(),
        format: BACKUP_FORMAT.to_string(),
        artifact_metadata: serde_json::json!({
            "role": "qwen_code_native_prewrite_session_backup",
            "complete_source_boundary": true,
            "selected_artifacts": artifacts,
            "runtime_root": metadata.runtime_root,
            "global_qwen_dir": metadata.global_qwen_dir,
            "entry_count": metadata.entry_count,
            "byte_size": metadata.byte_size,
        }),
        restore_metadata: serde_json::json!({
            "restore_mode": "exact_qwen_code_native_source_bundle",
            "provider_id": PROVIDER_ID,
            "mutation": mutation,
            "runtime_root": metadata.runtime_root,
            "global_qwen_dir": metadata.global_qwen_dir,
            "project_dir": metadata.project_dir,
            "source_path": metadata.source_path,
            "selected_artifacts": metadata.selected_artifacts,
            "artifact_digest": metadata.artifact_digest,
        }),
    })
}

pub(super) fn restore_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != BACKUP_FORMAT || backup.mime_type != BACKUP_MIME {
        bail!("Unsupported Qwen Code backup format: {}", backup.format);
    }
    let metadata_path = backup.backup_path.join(BACKUP_METADATA_FILE);
    let metadata: QwenSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read Qwen Code backup: {}",
                metadata_path.display()
            )
        })?)
        .context("Failed to parse Qwen Code backup metadata")?;
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.mutation != backup.mutation
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.source_path != backup.source_path
    {
        bail!("Qwen Code backup metadata does not match the restore request");
    }

    let current_roots = root_locations()?;
    if current_roots.runtime != metadata.runtime_root
        || current_roots.global_qwen != metadata.global_qwen_dir
    {
        bail!("Qwen Code backup roots do not match the current configured roots");
    }
    validate_runtime_source_path(
        &current_roots.runtime,
        &metadata.source_path,
        &metadata.provider_session_id,
    )?;
    validate_project_path(&current_roots.runtime, &metadata.project_dir)?;
    validate_selected_artifacts(&metadata.selected_artifacts)?;
    validate_restore_artifact_boundary(&metadata, &current_roots)?;
    let backup_source = backup.backup_path.join(BACKUP_SOURCE_DIR);
    let backup_roots = RootLocations {
        runtime: backup_source.join(ArtifactRoot::Runtime.directory_name()),
        global_qwen: backup_source.join(ArtifactRoot::GlobalQwen.directory_name()),
    };
    let copied = inspect_artifacts(&backup_roots, &metadata.selected_artifacts)?;
    if copied.digest != metadata.artifact_digest
        || copied.entry_count != metadata.entry_count
        || copied.byte_size != metadata.byte_size
    {
        bail!("Qwen Code backup artifact digest does not match its metadata");
    }

    for artifact in &metadata.selected_artifacts {
        let target = current_roots.path(artifact);
        ensure_safe_parent(&current_roots.root(artifact.root), &artifact.relative_path)?;
        remove_entry_if_exists(&target)?;
        copy_artifact(&backup_roots.path(artifact), &target)?;
    }
    let restored = inspect_artifacts(&current_roots, &metadata.selected_artifacts)?;
    if restored.digest != metadata.artifact_digest
        || restored.entry_count != metadata.entry_count
        || restored.byte_size != metadata.byte_size
    {
        bail!("Qwen Code source restore failed digest verification");
    }
    Ok(())
}

impl RootLocations {
    fn root(&self, root: ArtifactRoot) -> &Path {
        match root {
            ArtifactRoot::Runtime => &self.runtime,
            ArtifactRoot::GlobalQwen => &self.global_qwen,
        }
    }
}

fn discover_sources(session_id: &str) -> Result<SessionSources> {
    validate_session_id(session_id)?;
    let roots = root_locations()?;
    let projects_dir = roots.runtime.join("projects");
    let mut matches = Vec::new();
    for project_dir in direct_child_directories(&projects_dir)? {
        let project_dir = project_dir.canonicalize()?;
        let chats_dir = project_dir.join("chats");
        let active_path = chats_dir.join(format!("{session_id}.jsonl"));
        let archived_path = chats_dir
            .join("archive")
            .join(format!("{session_id}.jsonl"));
        let active = read_mutation_source(
            &roots.runtime,
            &project_dir,
            &active_path,
            session_id,
            false,
        )?;
        let archived = read_mutation_source(
            &roots.runtime,
            &project_dir,
            &archived_path,
            session_id,
            true,
        )?;
        if active.is_some() || archived.is_some() {
            matches.push((project_dir, active, archived));
        }
    }
    if matches.is_empty() {
        bail!("Qwen Code session not found: {session_id}");
    }
    if matches.len() > 1 {
        bail!("Ambiguous Qwen Code session identity across projects: {session_id}");
    }
    let (project_dir, active, archived) = matches.pop().expect("one Qwen source match");
    Ok(SessionSources {
        roots,
        project_dir,
        active,
        archived,
    })
}

fn read_mutation_source(
    runtime_root: &Path,
    project_dir: &Path,
    path: &Path,
    session_id: &str,
    archived: bool,
) -> Result<Option<SessionSource>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        bail!(
            "Qwen Code session source must not be a symlink: {}",
            path.display()
        );
    }
    if !metadata.file_type().is_file() {
        bail!(
            "Qwen Code session source is not a regular file: {}",
            path.display()
        );
    }
    validate_runtime_layout_path(runtime_root, path, session_id, archived)?;
    let parsed = parse_jsonl_session(path).with_context(|| {
        format!(
            "Failed to validate Qwen Code mutation source: {}",
            path.display()
        )
    })?;
    validate_project_ownership(project_dir, path, session_id, &parsed)?;
    Ok(Some(SessionSource {
        path: path.to_path_buf(),
        parsed,
    }))
}

fn validate_project_ownership(
    project_dir: &Path,
    source_path: &Path,
    session_id: &str,
    parsed: &super::ParsedQwenSession,
) -> Result<()> {
    let project_name = project_dir
        .file_name()
        .and_then(|name| name.to_str())
        .context("Qwen Code project directory has no valid name")?;
    let first_cwd = parsed
        .records
        .first()
        .and_then(|record| record.get("cwd"))
        .and_then(Value::as_str)
        .filter(|cwd| !cwd.trim().is_empty());
    let owns_by_cwd = first_cwd
        .map(sanitize_cwd)
        .is_some_and(|sanitized| sanitized == project_name);
    let owns_by_runtime_status = if owns_by_cwd {
        false
    } else {
        runtime_status_owns_session(project_dir, session_id, project_name)
    };
    if !owns_by_cwd && !owns_by_runtime_status {
        bail!(
            "Qwen Code session source does not belong to project {}: {}",
            project_name,
            source_path.display()
        );
    }
    Ok(())
}

fn runtime_status_owns_session(project_dir: &Path, session_id: &str, project_name: &str) -> bool {
    let path = project_dir
        .join("chats")
        .join(format!("{session_id}.runtime.json"));
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return false;
    };
    value.get("session_id").and_then(Value::as_str) == Some(session_id)
        && value
            .get("work_dir")
            .and_then(Value::as_str)
            .map(sanitize_cwd)
            == Some(project_name.to_string())
}

fn root_locations() -> Result<RootLocations> {
    let runtime = super::qwen_runtime_base()
        .context("Qwen Code runtime root is not configured")?
        .canonicalize()
        .context("Qwen Code runtime root does not exist")?;
    if !runtime.is_dir() {
        bail!(
            "Qwen Code runtime root is not a directory: {}",
            runtime.display()
        );
    }
    let global_qwen =
        super::qwen_global_dir().context("Qwen Code global directory is not configured")?;
    let global_qwen = absolute_path(&global_qwen)?;
    Ok(RootLocations {
        runtime,
        global_qwen,
    })
}

fn absolute_path(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    if absolute.exists() {
        Ok(absolute.canonicalize()?)
    } else {
        Ok(absolute)
    }
}

fn selected_artifacts(
    sources: &SessionSources,
    mutation: ProviderSourceMutation,
) -> Result<Vec<SelectedArtifact>> {
    let mut artifacts = Vec::new();
    match mutation {
        ProviderSourceMutation::Rename => {
            let active = sources
                .active
                .as_ref()
                .context("Qwen Code rename only supports an active session source")?;
            artifacts.push(runtime_artifact(&sources.roots.runtime, &active.path)?);
        }
        ProviderSourceMutation::Delete => {
            let session_id = sources
                .active
                .as_ref()
                .or(sources.archived.as_ref())
                .map(|source| source.parsed.session_id.as_str())
                .context("Qwen Code session has no parsed source")?;
            if let Some(active) = &sources.active {
                artifacts.push(runtime_artifact(&sources.roots.runtime, &active.path)?);
            }
            if let Some(archived) = &sources.archived {
                artifacts.push(runtime_artifact(&sources.roots.runtime, &archived.path)?);
            }
            add_optional_runtime_artifact(
                &sources.roots,
                sources
                    .project_dir
                    .join("chats")
                    .join(format!("{session_id}.worktree.json")),
                &mut artifacts,
            )?;
            add_optional_runtime_artifact(
                &sources.roots,
                sources
                    .project_dir
                    .join("chats/archive")
                    .join(format!("{session_id}.worktree.json")),
                &mut artifacts,
            )?;
            add_optional_global_artifact(
                &sources.roots,
                sources
                    .roots
                    .global_qwen
                    .join(FILE_HISTORY_DIR)
                    .join(session_id),
                &mut artifacts,
            )?;
            add_optional_runtime_artifact(
                &sources.roots,
                sources.project_dir.join(ORGANIZATION_FILE),
                &mut artifacts,
            )?;
        }
    }
    validate_selected_artifacts(&artifacts)?;
    Ok(artifacts)
}

fn runtime_artifact(root: &Path, path: &Path) -> Result<SelectedArtifact> {
    Ok(SelectedArtifact {
        root: ArtifactRoot::Runtime,
        relative_path: relative_path(root, path)?,
    })
}

fn add_optional_runtime_artifact(
    roots: &RootLocations,
    path: PathBuf,
    artifacts: &mut Vec<SelectedArtifact>,
) -> Result<()> {
    if std::fs::symlink_metadata(&path).is_ok() {
        artifacts.push(runtime_artifact(&roots.runtime, &path)?);
    }
    Ok(())
}

fn add_optional_global_artifact(
    roots: &RootLocations,
    path: PathBuf,
    artifacts: &mut Vec<SelectedArtifact>,
) -> Result<()> {
    if std::fs::symlink_metadata(&path).is_ok() {
        artifacts.push(SelectedArtifact {
            root: ArtifactRoot::GlobalQwen,
            relative_path: relative_path(&roots.global_qwen, &path)?,
        });
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let root = absolute_path(root)?;
    let path = absolute_path(path)?;
    let relative = path
        .strip_prefix(&root)
        .with_context(|| format!("Path escapes configured root: {}", path.display()))?;
    validate_relative_path(relative)?;
    Ok(relative.to_path_buf())
}

fn validate_selected_artifacts(artifacts: &[SelectedArtifact]) -> Result<()> {
    let mut seen = BTreeSet::new();
    for artifact in artifacts {
        validate_relative_path(&artifact.relative_path)?;
        let key = format!("{:?}:{}", artifact.root, artifact.relative_path.display());
        if !seen.insert(key) {
            bail!("Qwen Code backup contains duplicate artifact path");
        }
    }
    Ok(())
}

fn validate_restore_artifact_boundary(
    metadata: &QwenSessionBackupMetadata,
    roots: &RootLocations,
) -> Result<()> {
    validate_session_id(&metadata.provider_session_id)?;
    let project_relative = metadata
        .project_dir
        .strip_prefix(&roots.runtime)
        .context("Qwen Code backup project is outside the configured runtime root")?;
    let source_relative = metadata
        .source_path
        .strip_prefix(&roots.runtime)
        .context("Qwen Code backup source is outside the configured runtime root")?;
    let session_id = &metadata.provider_session_id;
    let active_source = project_relative
        .join("chats")
        .join(format!("{session_id}.jsonl"));
    let archived_source = project_relative
        .join("chats/archive")
        .join(format!("{session_id}.jsonl"));

    let allowed = match metadata.mutation {
        ProviderSourceMutation::Rename => vec![SelectedArtifact {
            root: ArtifactRoot::Runtime,
            relative_path: active_source.clone(),
        }],
        ProviderSourceMutation::Delete => vec![
            SelectedArtifact {
                root: ArtifactRoot::Runtime,
                relative_path: active_source.clone(),
            },
            SelectedArtifact {
                root: ArtifactRoot::Runtime,
                relative_path: archived_source.clone(),
            },
            SelectedArtifact {
                root: ArtifactRoot::Runtime,
                relative_path: project_relative
                    .join("chats")
                    .join(format!("{session_id}.worktree.json")),
            },
            SelectedArtifact {
                root: ArtifactRoot::Runtime,
                relative_path: project_relative
                    .join("chats/archive")
                    .join(format!("{session_id}.worktree.json")),
            },
            SelectedArtifact {
                root: ArtifactRoot::GlobalQwen,
                relative_path: Path::new(FILE_HISTORY_DIR).join(session_id),
            },
            SelectedArtifact {
                root: ArtifactRoot::Runtime,
                relative_path: project_relative.join(ORGANIZATION_FILE),
            },
        ],
    };

    if metadata.mutation == ProviderSourceMutation::Rename
        && (source_relative != active_source
            || metadata.selected_artifacts.as_slice() != allowed.as_slice())
    {
        bail!("Qwen Code rename backup is outside its official source boundary");
    }
    if metadata.mutation == ProviderSourceMutation::Delete {
        if source_relative != active_source && source_relative != archived_source {
            bail!("Qwen Code delete backup source is outside its official source boundary");
        }
        let has_source = metadata.selected_artifacts.iter().any(|artifact| {
            artifact.root == ArtifactRoot::Runtime
                && (artifact.relative_path == active_source
                    || artifact.relative_path == archived_source)
        });
        if !has_source
            || !metadata
                .selected_artifacts
                .iter()
                .all(|artifact| allowed.contains(artifact))
        {
            bail!("Qwen Code delete backup is outside its official source boundary");
        }
    }
    Ok(())
}

fn inspect_artifacts(
    roots: &RootLocations,
    artifacts: &[SelectedArtifact],
) -> Result<ArtifactInspection> {
    validate_selected_artifacts(artifacts)?;
    let mut hasher = Sha256::new();
    let mut entry_count = 0_usize;
    let mut byte_size = 0_u64;
    for artifact in artifacts {
        let path = roots.path(artifact);
        ensure_safe_parent(roots.root(artifact.root), &artifact.relative_path)?;
        inspect_entry(
            &path,
            &format!(
                "{}/{}",
                artifact.root.directory_name(),
                artifact.relative_path.display()
            ),
            &mut hasher,
            &mut entry_count,
            &mut byte_size,
        )?;
    }
    Ok(ArtifactInspection {
        digest: format!("sha256:{:x}", hasher.finalize()),
        entry_count,
        byte_size,
    })
}

fn inspect_entry(
    path: &Path,
    logical_path: &str,
    hasher: &mut Sha256,
    entry_count: &mut usize,
    byte_size: &mut u64,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("Qwen Code source artifact is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Qwen Code source artifact may not be a symlink: {}",
            path.display()
        );
    }
    if metadata.file_type().is_dir() {
        hash_entry(hasher, b"D", logical_path, &[]);
        *entry_count = entry_count.saturating_add(1);
        let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            inspect_entry(
                &entry.path(),
                &format!("{logical_path}/{}", entry.file_name().to_string_lossy()),
                hasher,
                entry_count,
                byte_size,
            )?;
        }
    } else if metadata.file_type().is_file() {
        let bytes = std::fs::read(path)?;
        hash_entry(hasher, b"F", logical_path, &bytes);
        *entry_count = entry_count.saturating_add(1);
        *byte_size = byte_size.saturating_add(bytes.len() as u64);
    } else {
        bail!(
            "Qwen Code source artifact is unsupported: {}",
            path.display()
        );
    }
    Ok(())
}

fn hash_entry(hasher: &mut Sha256, kind: &[u8], logical_path: &str, bytes: &[u8]) {
    hasher.update(kind);
    hasher.update(b"\0");
    hasher.update(logical_path.as_bytes());
    hasher.update(b"\0");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn copy_artifacts(
    source: &RootLocations,
    target: &RootLocations,
    artifacts: &[SelectedArtifact],
) -> Result<()> {
    for artifact in artifacts {
        let source_path = source.path(artifact);
        let target_path = target.path(artifact);
        ensure_safe_parent(target.root(artifact.root), &artifact.relative_path)?;
        copy_artifact(&source_path, &target_path)?;
    }
    Ok(())
}

fn copy_artifact(source: &Path, target: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Qwen Code source artifact may not be a symlink: {}",
            source.display()
        );
    }
    if metadata.file_type().is_dir() {
        std::fs::create_dir_all(target)?;
        let mut entries = std::fs::read_dir(source)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            copy_artifact(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else if metadata.file_type().is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, target)?;
    } else {
        bail!(
            "Qwen Code source artifact is unsupported: {}",
            source.display()
        );
    }
    Ok(())
}

fn remove_entry_if_exists(path: &Path) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        bail!(
            "Qwen Code source artifact may not be a symlink: {}",
            path.display()
        );
    }
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
    } else if metadata.file_type().is_file() {
        std::fs::remove_file(path)?;
    } else {
        bail!(
            "Qwen Code source artifact is unsupported: {}",
            path.display()
        );
    }
    Ok(())
}

fn remove_session_from_organization(path: &Path, session_id: &str) -> Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        bail!(
            "Qwen Code session organization store may not be a symlink: {}",
            path.display()
        );
    }
    if !metadata.file_type().is_file() {
        bail!(
            "Qwen Code session organization store is not a regular file: {}",
            path.display()
        );
    }
    let raw = std::fs::read_to_string(path)?;
    let mut value: Value =
        serde_json::from_str(&raw).context("Malformed Qwen Code session organization store")?;
    let Some(root) = value.as_object_mut() else {
        return Ok(());
    };
    let Some(sessions) = root.get_mut("sessions").and_then(Value::as_object_mut) else {
        return Ok(());
    };
    if sessions.remove(session_id).is_none() {
        return Ok(());
    }
    let temporary = path.with_file_name(format!(
        ".{ORGANIZATION_FILE}.memorph-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&temporary, serde_json::to_vec_pretty(&value)?)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.into());
    }
    Ok(())
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if !is_session_id(session_id) {
        bail!("Invalid Qwen Code session ID: {session_id}");
    }
    Ok(())
}

fn validate_runtime_layout_path(
    root: &Path,
    path: &Path,
    session_id: &str,
    archived: bool,
) -> Result<()> {
    let canonical = path.canonicalize()?;
    let relative = canonical.strip_prefix(root).with_context(|| {
        format!(
            "Qwen Code source is outside the configured runtime root: {}",
            path.display()
        )
    })?;
    validate_runtime_relative_layout(relative, session_id, archived)?;
    ensure_safe_parent(root, relative)?;
    Ok(())
}

fn validate_runtime_relative_layout(
    relative: &Path,
    session_id: &str,
    archived: bool,
) -> Result<()> {
    let components = relative.components().collect::<Vec<_>>();
    let expected = if archived { 5 } else { 4 };
    let expected_filename = format!("{session_id}.jsonl");
    if components.len() != expected
        || !matches!(components.first(), Some(Component::Normal(value)) if *value == std::ffi::OsStr::new("projects"))
        || !matches!(components.get(2), Some(Component::Normal(value)) if *value == std::ffi::OsStr::new("chats"))
        || !matches!(components.last(), Some(Component::Normal(value)) if *value == std::ffi::OsStr::new(&expected_filename))
        || (archived
            && !matches!(components.get(3), Some(Component::Normal(value)) if *value == std::ffi::OsStr::new("archive")))
    {
        bail!(
            "Not a Qwen Code native session source: {}",
            relative.display()
        );
    }
    Ok(())
}

fn validate_runtime_source_path(root: &Path, path: &Path, session_id: &str) -> Result<()> {
    let absolute = absolute_path(path)?;
    let relative = absolute.strip_prefix(root).with_context(|| {
        format!(
            "Qwen Code backup source escapes the configured runtime root: {}",
            path.display()
        )
    })?;
    let archived = relative.components().count() == 5;
    validate_runtime_relative_layout(relative, session_id, archived)?;
    ensure_safe_parent(root, relative)
}

fn validate_project_path(root: &Path, project_dir: &Path) -> Result<()> {
    let canonical = absolute_path(project_dir)?;
    let relative = canonical.strip_prefix(root)?;
    let components = relative.components().collect::<Vec<_>>();
    if components.len() != 2
        || !matches!(components.first(), Some(Component::Normal(value)) if *value == std::ffi::OsStr::new("projects"))
    {
        bail!(
            "Qwen Code project path escapes the configured runtime root: {}",
            project_dir.display()
        );
    }
    Ok(())
}

fn ensure_safe_parent(root: &Path, relative: &Path) -> Result<()> {
    validate_relative_path(relative)?;
    let mut current = root.to_path_buf();
    for component in relative.parent().into_iter().flat_map(Path::components) {
        let Component::Normal(value) = component else {
            bail!("Qwen Code artifact path contains an unsafe parent");
        };
        current.push(value);
        if let Ok(metadata) = std::fs::symlink_metadata(&current) {
            if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
                bail!(
                    "Qwen Code artifact parent is not a regular directory: {}",
                    current.display()
                );
            }
        }
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        bail!("Qwen Code artifact path must be relative");
    }
    for component in path.components() {
        if !matches!(component, Component::Normal(_)) {
            bail!(
                "Qwen Code artifact path contains an unsafe component: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn validate_safe_component(value: &str, label: &str) -> Result<()> {
    let path = Path::new(value);
    if value.is_empty()
        || !matches!(
            path.components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        )
    {
        bail!("Invalid Qwen Code {label}: {value}");
    }
    Ok(())
}

fn sanitize_cwd(cwd: &str) -> String {
    let normalized = if cfg!(windows) {
        cwd.to_ascii_lowercase()
    } else {
        cwd.to_string()
    };
    normalized
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}
