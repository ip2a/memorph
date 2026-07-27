use super::{
    find_session_dir, kiro_sessions_dir, read_validated_session_metadata, validate_session_id,
    workspace_bucket, KiroSessionMetadata, PROVIDER_ID,
};
use crate::provider::{ProviderSessionBackup, ProviderSourceMutation};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const KIRO_BACKUP_FORMAT: &str = "kiro-current-session-backup-v1";
const KIRO_BACKUP_MIME: &str = "application/vnd.memorph.kiro-current-session-backup";
const KIRO_BACKUP_SESSION_PATH: &str = "session";
const KIRO_BACKUP_METADATA_PATH: &str = "metadata.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    sessions_root: PathBuf,
    workspace_bucket: String,
    source_path: PathBuf,
    session_digest: String,
    entry_count: usize,
    byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionTreeInspection {
    digest: String,
    entry_count: usize,
    byte_size: u64,
}

pub(super) fn validate_mutation_source(session_id: &str) -> Result<PathBuf> {
    validate_session_id(session_id)?;
    let sessions_root = canonical_sessions_root()?;
    let session_path = find_session_dir(session_id)?
        .with_context(|| format!("Kiro session not found: {session_id}"))?;
    let source_metadata = std::fs::symlink_metadata(&session_path)?;
    if !source_metadata.file_type().is_dir() || source_metadata.file_type().is_symlink() {
        anyhow::bail!("Kiro session path is not a regular directory");
    }
    let session_path = session_path.canonicalize().with_context(|| {
        format!(
            "Failed to resolve Kiro session directory: {}",
            session_path.display()
        )
    })?;
    validate_source_identity(&sessions_root, &session_path, session_id)?;
    read_validated_session_metadata(&session_path)?;
    inspect_session_tree(&session_path)?;
    Ok(session_path)
}

pub(super) fn create_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    validate_safe_component(operation_id, "operation id")?;
    let source_path = validate_mutation_source(session_id)?;
    let sessions_root = canonical_sessions_root()?;
    let workspace_bucket = source_path
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .context("Kiro session path has no valid workspace bucket")?
        .to_string();
    let backup_path = backup_root
        .join(PROVIDER_ID)
        .join(operation_id)
        .join(session_id);
    if backup_path.exists() {
        anyhow::bail!("Kiro backup path already exists: {}", backup_path.display());
    }

    let create_result = (|| {
        let backup_session_path = backup_path.join(KIRO_BACKUP_SESSION_PATH);
        let before = inspect_session_tree(&source_path)?;
        copy_regular_tree(&source_path, &backup_session_path)?;
        let after = inspect_session_tree(&source_path)?;
        let copied = inspect_session_tree(&backup_session_path)?;
        if before != after || before != copied {
            anyhow::bail!("Kiro session changed while its native backup was being captured");
        }

        let metadata = KiroSessionBackupMetadata {
            version: 1,
            provider_id: PROVIDER_ID.to_string(),
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            sessions_root,
            workspace_bucket,
            source_path: source_path.clone(),
            session_digest: before.digest.clone(),
            entry_count: before.entry_count,
            byte_size: before.byte_size,
        };
        std::fs::write(
            backup_path.join(KIRO_BACKUP_METADATA_PATH),
            serde_json::to_vec_pretty(&metadata)?,
        )?;

        Ok(ProviderSessionBackup {
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            source_path,
            backup_path: backup_path.clone(),
            restore_hint:
                "Restore this backup with memorph's Kiro current-session restore flow before reopening Kiro."
                    .to_string(),
            mime_type: KIRO_BACKUP_MIME.to_string(),
            format: KIRO_BACKUP_FORMAT.to_string(),
            artifact_metadata: serde_json::json!({
                "role": "kiro_current_prewrite_session_backup",
                "mutation": mutation,
                "complete_session_directory": true,
                "entry_count": metadata.entry_count,
                "byte_size": metadata.byte_size,
            }),
            restore_metadata: serde_json::json!({
                "restore_mode": "kiro_current_session_directory",
                "metadata_file": KIRO_BACKUP_METADATA_PATH,
                "mutation": mutation,
                "session_digest": metadata.session_digest,
            }),
        })
    })();
    if create_result.is_err() {
        let _ = std::fs::remove_dir_all(&backup_path);
    }
    create_result
}

pub(super) fn restore_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != KIRO_BACKUP_FORMAT {
        anyhow::bail!("Unsupported Kiro session backup format: {}", backup.format);
    }
    if backup.mime_type != KIRO_BACKUP_MIME {
        anyhow::bail!(
            "Unsupported Kiro session backup MIME type: {}",
            backup.mime_type
        );
    }
    validate_session_id(&backup.provider_session_id)?;
    validate_safe_component(&backup.operation_id, "operation id")?;

    let metadata_path = backup.backup_path.join(KIRO_BACKUP_METADATA_PATH);
    require_regular_file(&metadata_path, "Kiro backup metadata")?;
    let metadata: KiroSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read Kiro backup metadata: {}",
                metadata_path.display()
            )
        })?)?;
    validate_restore_context(backup, &metadata)?;

    let backup_session_path = backup.backup_path.join(KIRO_BACKUP_SESSION_PATH);
    require_regular_directory(&backup_session_path, "Kiro backup session payload")?;
    let inspection = inspect_session_tree(&backup_session_path)?;
    if inspection.digest != metadata.session_digest
        || inspection.entry_count != metadata.entry_count
        || inspection.byte_size != metadata.byte_size
    {
        anyhow::bail!("Kiro backup session payload does not match its metadata");
    }
    let restore_digest = backup
        .restore_metadata
        .get("session_digest")
        .and_then(serde_json::Value::as_str)
        .context("Kiro backup restore metadata is missing the session digest")?;
    if restore_digest != metadata.session_digest {
        anyhow::bail!("Kiro backup restore metadata has a mismatched session digest");
    }

    let current_root = canonical_sessions_root()?;
    if current_root != metadata.sessions_root {
        anyhow::bail!("Kiro backup belongs to a different configured sessions root");
    }
    let target_path = current_root
        .join(&metadata.workspace_bucket)
        .join(&metadata.provider_session_id);
    if target_path != metadata.source_path || target_path != backup.source_path {
        anyhow::bail!("Kiro backup source path does not match its current-format identity");
    }
    restore_session_tree(&backup_session_path, &target_path, &metadata)?;
    Ok(())
}

fn validate_restore_context(
    backup: &ProviderSessionBackup,
    metadata: &KiroSessionBackupMetadata,
) -> Result<()> {
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.mutation != backup.mutation
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
    {
        anyhow::bail!("Kiro backup identity does not match its metadata");
    }
    validate_safe_component(&metadata.workspace_bucket, "workspace bucket")?;
    let session_metadata = read_session_metadata_from_backup(
        &backup
            .backup_path
            .join(KIRO_BACKUP_SESSION_PATH)
            .join("session.json"),
    )?;
    if session_metadata.id != metadata.provider_session_id
        || workspace_bucket(&session_metadata.workspace_paths)? != metadata.workspace_bucket
    {
        anyhow::bail!("Kiro backup payload identity does not match its metadata");
    }
    Ok(())
}

fn read_session_metadata_from_backup(path: &Path) -> Result<KiroSessionMetadata> {
    require_regular_file(path, "Kiro backup session metadata")?;
    serde_json::from_slice(&std::fs::read(path)?)
        .with_context(|| format!("Failed to parse Kiro backup metadata: {}", path.display()))
}

fn restore_session_tree(
    backup_session_path: &Path,
    target_path: &Path,
    metadata: &KiroSessionBackupMetadata,
) -> Result<()> {
    let parent = target_path
        .parent()
        .context("Kiro restore target has no workspace bucket")?;
    ensure_restore_parent(&metadata.sessions_root, parent, &metadata.workspace_bucket)?;

    if let Ok(target_metadata) = std::fs::symlink_metadata(target_path) {
        if !target_metadata.file_type().is_dir() || target_metadata.file_type().is_symlink() {
            anyhow::bail!(
                "Kiro restore target is not a regular directory: {}",
                target_path.display()
            );
        }
        read_validated_session_metadata(target_path)?;
    }

    let nonce = uuid::Uuid::new_v4();
    let staging = parent.join(format!(".memorph-kiro-restore-{nonce}"));
    let previous = parent.join(format!(".memorph-kiro-previous-{nonce}"));
    let restore_result = (|| {
        copy_regular_tree(backup_session_path, &staging)?;
        if inspect_session_tree(&staging)?.digest != metadata.session_digest {
            anyhow::bail!("Kiro restore staging directory failed digest verification");
        }

        let had_previous = target_path.exists();
        if had_previous {
            std::fs::rename(target_path, &previous).with_context(|| {
                format!(
                    "Failed to preserve current Kiro session before restore: {}",
                    target_path.display()
                )
            })?;
        }
        if let Err(error) = std::fs::rename(&staging, target_path) {
            if had_previous {
                let _ = std::fs::rename(&previous, target_path);
            }
            return Err(error).with_context(|| {
                format!(
                    "Failed to install restored Kiro session: {}",
                    target_path.display()
                )
            });
        }
        if let Err(error) = read_validated_session_metadata(target_path) {
            let _ = std::fs::remove_dir_all(target_path);
            if had_previous {
                let _ = std::fs::rename(&previous, target_path);
            }
            return Err(error).context("Restored Kiro session failed current-format validation");
        }
        if had_previous {
            std::fs::remove_dir_all(&previous).with_context(|| {
                format!(
                    "Failed to remove superseded Kiro session after restore: {}",
                    previous.display()
                )
            })?;
        }
        Ok(())
    })();
    if restore_result.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    restore_result
}

fn ensure_restore_parent(root: &Path, parent: &Path, workspace_bucket: &str) -> Result<()> {
    if parent != root.join(workspace_bucket) {
        anyhow::bail!("Kiro restore target escapes the configured sessions root");
    }
    match std::fs::symlink_metadata(parent) {
        Ok(metadata) => {
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "Kiro restore workspace bucket is not a regular directory: {}",
                    parent.display()
                );
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(parent).with_context(|| {
                format!(
                    "Failed to recreate Kiro restore workspace bucket: {}",
                    parent.display()
                )
            })?;
        }
        Err(error) => return Err(error.into()),
    }
    if !parent.canonicalize()?.starts_with(root) {
        anyhow::bail!("Kiro restore workspace bucket escapes the configured sessions root");
    }
    Ok(())
}

fn canonical_sessions_root() -> Result<PathBuf> {
    let root = kiro_sessions_dir()?;
    require_regular_directory(&root, "Kiro sessions root")?;
    root.canonicalize()
        .with_context(|| format!("Failed to resolve Kiro sessions root: {}", root.display()))
}

fn validate_source_identity(root: &Path, session_path: &Path, session_id: &str) -> Result<()> {
    let relative = session_path.strip_prefix(root).with_context(|| {
        format!(
            "Kiro session path escapes the configured sessions root: {}",
            session_path.display()
        )
    })?;
    let components = relative.components().collect::<Vec<_>>();
    if !matches!(components.as_slice(), [Component::Normal(_), Component::Normal(id)] if *id == std::ffi::OsStr::new(session_id))
    {
        anyhow::bail!("Kiro session path does not match the current directory layout");
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
        anyhow::bail!("Invalid Kiro {label}: {value}");
    }
    Ok(())
}

fn require_regular_file(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("{label} does not exist: {}", path.display()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!("{label} is not a regular file: {}", path.display());
    }
    Ok(())
}

fn require_regular_directory(path: &Path, label: &str) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("{label} does not exist: {}", path.display()))?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!("{label} is not a regular directory: {}", path.display());
    }
    Ok(())
}

fn copy_regular_tree(source: &Path, target: &Path) -> Result<()> {
    require_regular_directory(source, "Kiro session tree")?;
    std::fs::create_dir_all(target).with_context(|| {
        format!(
            "Failed to create Kiro session tree copy: {}",
            target.display()
        )
    })?;
    for entry in WalkDir::new(source)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .skip(1)
    {
        let entry = entry
            .with_context(|| format!("Failed to walk Kiro session tree: {}", source.display()))?;
        let relative = entry.path().strip_prefix(source)?;
        let destination = target.join(relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir(&destination).with_context(|| {
                format!(
                    "Failed to copy Kiro session directory: {}",
                    destination.display()
                )
            })?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(entry.path(), &destination).with_context(|| {
                format!(
                    "Failed to copy Kiro session file {} to {}",
                    entry.path().display(),
                    destination.display()
                )
            })?;
        } else {
            anyhow::bail!(
                "Kiro session tree contains a symlink or unsupported entry: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn inspect_session_tree(path: &Path) -> Result<SessionTreeInspection> {
    require_regular_directory(path, "Kiro session tree")?;
    let mut hasher = Sha256::new();
    hasher.update(b"memorph-kiro-current-session-tree-v1\0");
    let mut entry_count = 0_usize;
    let mut byte_size = 0_u64;

    for entry in WalkDir::new(path)
        .follow_links(false)
        .sort_by_file_name()
        .into_iter()
        .skip(1)
    {
        let entry = entry
            .with_context(|| format!("Failed to inspect Kiro session tree: {}", path.display()))?;
        let relative = entry.path().strip_prefix(path)?;
        let relative = relative.to_string_lossy();
        entry_count = entry_count.saturating_add(1);
        if entry.file_type().is_dir() {
            hasher.update(b"D\0");
            hasher.update(relative.as_bytes());
            hasher.update(b"\0");
        } else if entry.file_type().is_file() {
            let bytes = std::fs::read(entry.path()).with_context(|| {
                format!(
                    "Failed to read Kiro session file: {}",
                    entry.path().display()
                )
            })?;
            byte_size = byte_size.saturating_add(bytes.len() as u64);
            hasher.update(b"F\0");
            hasher.update(relative.as_bytes());
            hasher.update(b"\0");
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        } else {
            anyhow::bail!(
                "Kiro session tree contains a symlink or unsupported entry: {}",
                entry.path().display()
            );
        }
    }

    Ok(SessionTreeInspection {
        digest: format!("sha256:{:x}", hasher.finalize()),
        entry_count,
        byte_size,
    })
}
