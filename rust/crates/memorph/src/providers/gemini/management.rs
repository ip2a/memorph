use super::{canonical_source_path, gemini_roots, PROVIDER_ID};
use crate::provider::{ProviderSessionBackup, ProviderSourceMutation};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

const BACKUP_FORMAT: &str = "gemini-current-session-backup-v1";
const BACKUP_MIME: &str = "application/vnd.memorph.gemini-current-session-backup";
const BACKUP_SOURCE_DIR: &str = "source";
const BACKUP_METADATA_FILE: &str = "metadata.json";
const RESERVED_ARTIFACT_DIRS: &[&str] = &["chats", "logs", "tool-outputs"];

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GeminiSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    temp_dir: PathBuf,
    source_path: PathBuf,
    selected_paths: Vec<PathBuf>,
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

pub(super) fn validate_mutation_source(session_id: &str) -> Result<(PathBuf, PathBuf)> {
    validate_session_id(session_id)?;
    let matches = mutation_source_paths(session_id)?;
    let source_path = match matches.as_slice() {
        [] => bail!("Gemini session not found: {session_id}"),
        [source_path] => source_path.clone(),
        _ => bail!("Gemini session ID resolves to multiple current sources: {session_id}"),
    };
    let temp_dir = validate_source_layout(&source_path)?;
    let selected_paths = select_artifact_paths(&temp_dir, &source_path, session_id)?;
    inspect_selected_paths(&temp_dir, &selected_paths)?;
    Ok((source_path, temp_dir))
}

pub(super) fn delete_session(session_id: &str) -> Result<()> {
    let (source_path, temp_dir) = validate_mutation_source(session_id)?;
    let selected_paths = select_artifact_paths(&temp_dir, &source_path, session_id)?;
    for relative in selected_paths.iter().rev() {
        remove_entry(&temp_dir.join(relative))?;
    }
    super::fail_gemini_mutation_after_write(ProviderSourceMutation::Delete)
}

pub(super) fn create_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    validate_safe_component(operation_id, "operation id")?;
    let (source_path, temp_dir) = validate_mutation_source(session_id)?;
    let selected_paths = select_artifact_paths(&temp_dir, &source_path, session_id)?;
    let before = inspect_selected_paths(&temp_dir, &selected_paths)?;
    let backup_path = backup_root
        .join(PROVIDER_ID)
        .join(operation_id)
        .join(session_id);
    if backup_path.exists() {
        bail!(
            "Gemini backup path already exists: {}",
            backup_path.display()
        );
    }

    let backup_source = backup_path.join(BACKUP_SOURCE_DIR);
    copy_selected_paths(&temp_dir, &backup_source, &selected_paths)?;
    let after = inspect_selected_paths(&temp_dir, &selected_paths)?;
    let copied = inspect_selected_paths(&backup_source, &selected_paths)?;
    if before != after || before != copied {
        let _ = std::fs::remove_dir_all(&backup_path);
        bail!("Gemini source changed while its native backup was being captured");
    }

    let metadata = GeminiSessionBackupMetadata {
        version: 1,
        provider_id: PROVIDER_ID.to_string(),
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        temp_dir: temp_dir.clone(),
        source_path: source_path.clone(),
        selected_paths: selected_paths.iter().cloned().collect(),
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
            "Restore this exact Gemini current-session source bundle before reopening Gemini."
                .to_string(),
        mime_type: BACKUP_MIME.to_string(),
        format: BACKUP_FORMAT.to_string(),
        artifact_metadata: serde_json::json!({
            "role": "gemini_current_prewrite_session_backup",
            "mutation": mutation,
            "complete_source_boundary": true,
            "selected_paths": metadata.selected_paths,
            "entry_count": metadata.entry_count,
            "byte_size": metadata.byte_size,
        }),
        restore_metadata: serde_json::json!({
            "restore_mode": "exact_gemini_current_source_bundle",
            "provider_id": PROVIDER_ID,
            "mutation": mutation,
            "temp_dir": metadata.temp_dir,
            "source_path": metadata.source_path,
            "artifact_digest": metadata.artifact_digest,
            "selected_paths": metadata.selected_paths,
        }),
    })
}

pub(super) fn restore_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != BACKUP_FORMAT || backup.mime_type != BACKUP_MIME {
        bail!("Unsupported Gemini backup format: {}", backup.format);
    }
    let metadata_path = backup.backup_path.join(BACKUP_METADATA_FILE);
    let metadata: GeminiSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!("Failed to read Gemini backup: {}", metadata_path.display())
        })?)
        .context("Failed to parse Gemini backup metadata")?;
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.mutation != backup.mutation
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.source_path != backup.source_path
    {
        bail!("Gemini backup metadata does not match the restore request");
    }
    validate_source_layout(&metadata.source_path).or_else(|_| {
        let temp_dir = metadata.temp_dir.canonicalize().with_context(|| {
            format!(
                "Gemini temp directory is unavailable: {}",
                metadata.temp_dir.display()
            )
        })?;
        validate_temp_dir(&temp_dir).map(|_| temp_dir)
    })?;
    validate_selected_paths(&metadata.selected_paths)?;

    let backup_source = backup.backup_path.join(BACKUP_SOURCE_DIR);
    let selected_paths = metadata
        .selected_paths
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let copied = inspect_selected_paths(&backup_source, &selected_paths)?;
    if copied.digest != metadata.artifact_digest
        || copied.entry_count != metadata.entry_count
        || copied.byte_size != metadata.byte_size
    {
        bail!("Gemini backup artifact digest does not match its metadata");
    }

    let temp_dir = metadata.temp_dir;
    std::fs::create_dir_all(&temp_dir)?;
    validate_temp_dir(&temp_dir)?;
    for relative in selected_paths.iter().rev() {
        let target = temp_dir.join(relative);
        if target.exists() {
            remove_entry(&target)?;
        }
    }
    copy_selected_paths(&backup_source, &temp_dir, &selected_paths)?;
    let restored = inspect_selected_paths(&temp_dir, &selected_paths)?;
    if restored.digest != metadata.artifact_digest
        || restored.entry_count != metadata.entry_count
        || restored.byte_size != metadata.byte_size
    {
        bail!("Gemini source restore failed digest verification");
    }
    Ok(())
}

fn mutation_source_paths(session_id: &str) -> Result<Vec<PathBuf>> {
    let mut matches = Vec::new();
    for root in gemini_roots() {
        let mut project_dirs = super::read_child_directories(&root)?;
        project_dirs.sort();
        for project_dir in project_dirs {
            let chats_dir = project_dir.join("chats");
            let mut files = super::read_session_files(&chats_dir)?;
            files.sort();
            for path in files {
                let path = match canonical_source_path(&path) {
                    Ok(path) => path,
                    Err(_) => continue,
                };
                let parsed = match super::parse_jsonl_session(&path) {
                    Ok(parsed) => parsed,
                    Err(_) => continue,
                };
                if super::metadata_string(&parsed.metadata, "sessionId").as_deref()
                    == Some(session_id)
                {
                    matches.push(path);
                }
            }
        }
    }
    Ok(matches)
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty() || session_id == "." || session_id == ".." {
        bail!("Invalid Gemini session ID: {session_id}");
    }
    validate_safe_component(session_id, "session id")?;
    let safe = sanitize_filename_part(session_id);
    if safe.is_empty() || RESERVED_ARTIFACT_DIRS.contains(&safe.to_ascii_lowercase().as_str()) {
        bail!("Invalid or reserved Gemini session ID: {session_id}");
    }
    Ok(())
}

fn sanitize_filename_part(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn validate_source_layout(source_path: &Path) -> Result<PathBuf> {
    let source_path = canonical_source_path(source_path)?;
    let chats_dir = source_path
        .parent()
        .context("Gemini source has no chats directory")?;
    if chats_dir.file_name().and_then(|name| name.to_str()) != Some("chats") {
        bail!("Gemini source is not under a direct chats directory");
    }
    let temp_dir = chats_dir
        .parent()
        .context("Gemini chats directory has no project temp directory")?;
    let temp_dir = temp_dir.canonicalize()?;
    validate_temp_dir(&temp_dir)?;
    Ok(temp_dir)
}

fn validate_temp_dir(temp_dir: &Path) -> Result<()> {
    let temp_dir = temp_dir.canonicalize()?;
    let tmp_root = temp_dir
        .parent()
        .context("Gemini temp directory has no tmp root")?;
    let matches_root = gemini_roots().into_iter().any(|root| {
        root.canonicalize()
            .map(|canonical| canonical == tmp_root)
            .unwrap_or(false)
    });
    if !matches_root {
        bail!(
            "Gemini source is outside the configured tmp root: {}",
            temp_dir.display()
        );
    }
    Ok(())
}

fn select_artifact_paths(
    temp_dir: &Path,
    source_path: &Path,
    session_id: &str,
) -> Result<BTreeSet<PathBuf>> {
    let safe_session_id = sanitize_filename_part(session_id);
    let mut selected = BTreeSet::new();
    selected.insert(relative_path(temp_dir, source_path)?);
    add_if_exists(
        temp_dir,
        &mut selected,
        PathBuf::from("logs").join(format!("session-{safe_session_id}.jsonl")),
    )?;
    add_if_exists(
        temp_dir,
        &mut selected,
        PathBuf::from("tool-outputs").join(format!("session-{safe_session_id}")),
    )?;
    add_if_exists(temp_dir, &mut selected, PathBuf::from(&safe_session_id))?;

    let subagent_dir = temp_dir.join("chats").join(&safe_session_id);
    if path_exists(&subagent_dir)? {
        add_if_exists(
            temp_dir,
            &mut selected,
            PathBuf::from("chats").join(&safe_session_id),
        )?;
        for entry in std::fs::read_dir(&subagent_dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file()
                && matches!(
                    entry.path().extension().and_then(|value| value.to_str()),
                    Some("json") | Some("jsonl")
                )
            {
                let agent_path = entry.path();
                let agent_id = agent_path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .context("Gemini subagent file has no valid basename")?;
                let safe_agent_id = sanitize_filename_part(agent_id);
                add_if_exists(
                    temp_dir,
                    &mut selected,
                    PathBuf::from("logs").join(format!("session-{safe_agent_id}.jsonl")),
                )?;
                add_if_exists(
                    temp_dir,
                    &mut selected,
                    PathBuf::from("tool-outputs").join(format!("session-{safe_agent_id}")),
                )?;
                add_if_exists(temp_dir, &mut selected, PathBuf::from(&safe_agent_id))?;
            }
        }
    }
    validate_selected_paths(&selected.iter().cloned().collect::<Vec<_>>())?;
    Ok(selected)
}

fn validate_selected_paths(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        if path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            bail!(
                "Gemini backup path is not a safe relative path: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn relative_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let relative = path
        .strip_prefix(root)
        .with_context(|| format!("Gemini source escapes temp directory: {}", path.display()))?
        .to_path_buf();
    validate_selected_paths(std::slice::from_ref(&relative))?;
    Ok(relative)
}

fn path_exists(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn add_if_exists(
    temp_dir: &Path,
    selected: &mut BTreeSet<PathBuf>,
    relative: PathBuf,
) -> Result<()> {
    validate_selected_paths(std::slice::from_ref(&relative))?;
    if path_exists(&temp_dir.join(&relative))? {
        selected.insert(relative);
    }
    Ok(())
}

fn inspect_selected_paths(root: &Path, selected: &BTreeSet<PathBuf>) -> Result<ArtifactInspection> {
    let mut hasher = Sha256::new();
    let mut entry_count = 0_usize;
    let mut byte_size = 0_u64;
    for relative in selected {
        inspect_entry(
            &root.join(relative),
            relative,
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
    relative: &Path,
    hasher: &mut Sha256,
    entry_count: &mut usize,
    byte_size: &mut u64,
) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("Gemini source artifact is missing: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Gemini source artifact may not be a symlink: {}",
            path.display()
        );
    }
    if metadata.file_type().is_dir() {
        hash_entry(hasher, b"D", relative, &[]);
        *entry_count = entry_count.saturating_add(1);
        for entry in sorted_directory_entries(path)? {
            let child_relative = relative.join(entry.file_name());
            inspect_entry(
                &entry.path(),
                &child_relative,
                hasher,
                entry_count,
                byte_size,
            )?;
        }
    } else if metadata.file_type().is_file() {
        let bytes = std::fs::read(path)?;
        hash_entry(hasher, b"F", relative, &bytes);
        *entry_count = entry_count.saturating_add(1);
        *byte_size = byte_size.saturating_add(bytes.len() as u64);
    } else {
        bail!("Gemini source artifact is unsupported: {}", path.display());
    }
    Ok(())
}

fn hash_entry(hasher: &mut Sha256, kind: &[u8], relative: &Path, bytes: &[u8]) {
    hasher.update(kind);
    hasher.update(b"\0");
    hasher.update(relative.to_string_lossy().as_bytes());
    hasher.update(b"\0");
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn copy_selected_paths(
    root: &Path,
    target_root: &Path,
    selected: &BTreeSet<PathBuf>,
) -> Result<()> {
    for relative in selected {
        copy_entry(&root.join(relative), &target_root.join(relative))?;
    }
    Ok(())
}

fn copy_entry(source: &Path, target: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Gemini source artifact may not be a symlink: {}",
            source.display()
        );
    }
    if metadata.file_type().is_dir() {
        std::fs::create_dir_all(target)?;
        for entry in sorted_directory_entries(source)? {
            copy_entry(&entry.path(), &target.join(entry.file_name()))?;
        }
    } else if metadata.file_type().is_file() {
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(source, target)?;
    } else {
        bail!(
            "Gemini source artifact is unsupported: {}",
            source.display()
        );
    }
    Ok(())
}

fn sorted_directory_entries(path: &Path) -> Result<Vec<std::fs::DirEntry>> {
    let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn remove_entry(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        bail!(
            "Gemini source artifact may not be a symlink: {}",
            path.display()
        );
    }
    if metadata.file_type().is_dir() {
        std::fs::remove_dir_all(path)?;
    } else if metadata.file_type().is_file() {
        std::fs::remove_file(path)?;
    } else {
        bail!("Gemini source artifact is unsupported: {}", path.display());
    }
    Ok(())
}

fn validate_safe_component(value: &str, label: &str) -> Result<()> {
    let mut components = Path::new(value).components();
    if value.is_empty()
        || !matches!(
            (components.next(), components.next()),
            (Some(Component::Normal(_)), None)
        )
    {
        bail!("Invalid Gemini {label}: {value}");
    }
    Ok(())
}
