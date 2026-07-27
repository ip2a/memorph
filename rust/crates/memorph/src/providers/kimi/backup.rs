use super::{get_kimi_sessions_dir, PROVIDER_ID};
use crate::provider::{ProviderSessionBackup, ProviderSourceMutation};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const KIMI_BACKUP_FORMAT: &str = "kimi-session-backup-v1";
const KIMI_BACKUP_MIME: &str = "application/vnd.memorph.kimi-session-backup";
const KIMI_SESSION_BACKUP_PATH: &str = "session";
const KIMI_STATE_BACKUP_PATH: &str = "files/state.json";

#[cfg(test)]
static TEST_KIMI_BACKUP_FAILURE: std::sync::OnceLock<std::sync::Mutex<bool>> =
    std::sync::OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KimiSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    sessions_root: PathBuf,
    session_path: PathBuf,
    state_path: PathBuf,
    entries: Vec<KimiBackupEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct KimiBackupEntry {
    relative_path: PathBuf,
    kind: KimiBackupEntryKind,
    byte_len: u64,
    sha256: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum KimiBackupEntryKind {
    Directory,
    File,
}

pub(super) fn create_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    let session_path = validate_mutation_source(mutation, session_id)?;
    let sessions_root = canonical_sessions_root()?;
    let state_path = session_path.join("state.json");

    let provider_backup_root = backup_root.join(PROVIDER_ID);
    std::fs::create_dir_all(&provider_backup_root).with_context(|| {
        format!(
            "Failed to create Kimi backup root: {}",
            provider_backup_root.display()
        )
    })?;
    let backup_path = provider_backup_root.join(operation_id);
    std::fs::create_dir(&backup_path).with_context(|| {
        format!(
            "Failed to create Kimi session backup: {}",
            backup_path.display()
        )
    })?;

    let create_result = (|| -> Result<ProviderSessionBackup> {
        let entries = match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
                let destination = backup_path.join(KIMI_SESSION_BACKUP_PATH);
                copy_validated_session_tree(&session_path, &destination)?
            }
            ProviderSourceMutation::Rename => {
                std::fs::create_dir(backup_path.join("files"))?;
                let bytes = std::fs::read(&state_path)?;
                std::fs::write(backup_path.join(KIMI_STATE_BACKUP_PATH), &bytes)?;
                vec![KimiBackupEntry {
                    relative_path: PathBuf::from("state.json"),
                    kind: KimiBackupEntryKind::File,
                    byte_len: u64::try_from(bytes.len()).context("Kimi state.json is too large")?,
                    sha256: Some(sha256_bytes(&bytes)),
                }]
            }
        };
        fail_kimi_backup_after_capture()?;

        let metadata = KimiSessionBackupMetadata {
            version: 1,
            provider_id: PROVIDER_ID.to_string(),
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            sessions_root,
            session_path: session_path.clone(),
            state_path,
            entries,
        };
        std::fs::write(
            backup_path.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata)?,
        )?;

        Ok(ProviderSessionBackup {
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            source_path: session_path,
            backup_path: backup_path.clone(),
            restore_hint:
                "Restore this backup with memorph's Kimi native session restore flow before reopening Kimi."
                    .to_string(),
            mime_type: KIMI_BACKUP_MIME.to_string(),
            format: KIMI_BACKUP_FORMAT.to_string(),
            artifact_metadata: serde_json::json!({
                "role": "kimi_prewrite_session_backup",
                "mutation": mutation,
                "entry_count": metadata.entries.len(),
                "complete_session_directory": matches!(
                    mutation,
                    ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
                ),
            }),
            restore_metadata: serde_json::json!({
                "restore_mode": "kimi_session_restore",
                "metadata_file": "metadata.json",
                "mutation": mutation,
            }),
        })
    })();
    if create_result.is_err() {
        let _ = std::fs::remove_dir_all(&backup_path);
    }
    create_result
}

pub(super) fn restore_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != KIMI_BACKUP_FORMAT {
        anyhow::bail!("Unsupported Kimi session backup format: {}", backup.format);
    }
    if backup.mime_type != KIMI_BACKUP_MIME {
        anyhow::bail!(
            "Unsupported Kimi session backup MIME type: {}",
            backup.mime_type
        );
    }

    let metadata_path = backup.backup_path.join("metadata.json");
    let metadata: KimiSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read Kimi backup metadata: {}",
                metadata_path.display()
            )
        })?)?;
    validate_restore_context(backup, &metadata)?;

    match metadata.mutation {
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
            let source = backup.backup_path.join(KIMI_SESSION_BACKUP_PATH);
            validate_session_tree(&source, &metadata.entries)?;
            if metadata.session_path.exists() {
                let source_type = std::fs::symlink_metadata(&metadata.session_path)?;
                if !source_type.file_type().is_dir() || source_type.file_type().is_symlink() {
                    anyhow::bail!(
                        "Kimi restore target is not a regular directory: {}",
                        metadata.session_path.display()
                    );
                }
            }
            restore_deleted_session(&source, &metadata.session_path)?;
        }
        ProviderSourceMutation::Rename => {
            validate_rename_manifest(&backup.backup_path, &metadata)?;
            if !metadata.session_path.exists() || !metadata.state_path.exists() {
                return Ok(());
            }
            let current_session = std::fs::symlink_metadata(&metadata.session_path)?;
            let current_state = std::fs::symlink_metadata(&metadata.state_path)?;
            if !current_session.file_type().is_dir()
                || current_session.file_type().is_symlink()
                || !current_state.file_type().is_file()
                || current_state.file_type().is_symlink()
            {
                anyhow::bail!("Kimi rename restore target has an unsafe source shape");
            }
            let bytes = std::fs::read(backup.backup_path.join(KIMI_STATE_BACKUP_PATH))?;
            super::write_kimi_state_atomically(&metadata.state_path, &bytes)?;
        }
    }
    Ok(())
}

pub(super) fn validate_mutation_source(
    mutation: ProviderSourceMutation,
    session_id: &str,
) -> Result<PathBuf> {
    if session_id.is_empty() {
        anyhow::bail!("Kimi session id cannot be empty");
    }
    let root = canonical_sessions_root()?;
    let matches = super::find_session_dirs(session_id)?;
    if matches.len() != 1 {
        anyhow::bail!("Kimi session not found or ambiguous: {session_id}");
    }
    let session_path = matches[0].canonicalize().with_context(|| {
        format!(
            "Failed to resolve Kimi session directory: {}",
            matches[0].display()
        )
    })?;
    if !session_path.starts_with(&root)
        || session_path.file_name().and_then(|name| name.to_str()) != Some(session_id)
    {
        anyhow::bail!("Kimi session path escapes the configured sessions root");
    }
    let metadata = std::fs::symlink_metadata(&matches[0])?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!("Kimi session path is not a regular directory");
    }
    validate_session_tree_shape(&session_path)?;
    if mutation == ProviderSourceMutation::Rename {
        let state_path = session_path.join("state.json");
        let state_metadata = std::fs::symlink_metadata(&state_path)
            .with_context(|| format!("Kimi state.json not found for session: {session_id}"))?;
        if !state_metadata.file_type().is_file() || state_metadata.file_type().is_symlink() {
            anyhow::bail!("Kimi state.json is not a regular file");
        }
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&state_path)?)
            .with_context(|| format!("Failed to parse state.json: {}", state_path.display()))?;
        if !value.is_object() {
            anyhow::bail!("Kimi state.json must contain a JSON object");
        }
    }
    Ok(session_path)
}

fn canonical_sessions_root() -> Result<PathBuf> {
    let root = get_kimi_sessions_dir();
    if !root.exists() {
        anyhow::bail!("Kimi sessions root does not exist: {}", root.display());
    }
    let metadata = std::fs::symlink_metadata(&root)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!("Kimi sessions root is not a regular directory");
    }
    root.canonicalize()
        .with_context(|| format!("Failed to resolve Kimi sessions root: {}", root.display()))
}

fn validate_session_tree_shape(source: &Path) -> Result<()> {
    for entry in WalkDir::new(source).follow_links(false) {
        let entry =
            entry.with_context(|| format!("Failed to walk Kimi session: {}", source.display()))?;
        let file_type = entry.file_type();
        if file_type.is_symlink() || (!file_type.is_dir() && !file_type.is_file()) {
            anyhow::bail!(
                "Kimi session contains unsupported filesystem entry: {}",
                entry.path().display()
            );
        }
    }
    Ok(())
}

fn copy_validated_session_tree(source: &Path, destination: &Path) -> Result<Vec<KimiBackupEntry>> {
    std::fs::create_dir(destination)?;
    let mut entries = Vec::new();
    for entry in WalkDir::new(source).follow_links(false).min_depth(1) {
        let entry =
            entry.with_context(|| format!("Failed to walk Kimi session: {}", source.display()))?;
        let relative = entry.path().strip_prefix(source)?.to_path_buf();
        validate_relative_path(&relative)?;
        let target = destination.join(&relative);
        if entry.file_type().is_dir() {
            std::fs::create_dir(&target)?;
            entries.push(KimiBackupEntry {
                relative_path: relative,
                kind: KimiBackupEntryKind::Directory,
                byte_len: 0,
                sha256: None,
            });
        } else if entry.file_type().is_file() {
            let bytes = std::fs::read(entry.path())?;
            std::fs::write(&target, &bytes)?;
            entries.push(KimiBackupEntry {
                relative_path: relative,
                kind: KimiBackupEntryKind::File,
                byte_len: u64::try_from(bytes.len()).context("Kimi session file is too large")?,
                sha256: Some(sha256_bytes(&bytes)),
            });
        } else {
            anyhow::bail!(
                "Kimi session contains unsupported filesystem entry: {}",
                entry.path().display()
            );
        }
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(entries)
}

fn validate_session_tree(source: &Path, expected: &[KimiBackupEntry]) -> Result<()> {
    let actual = inspect_session_tree(source)?;
    if actual != expected {
        anyhow::bail!("Kimi session backup tree does not match its manifest");
    }
    Ok(())
}

fn inspect_session_tree(source: &Path) -> Result<Vec<KimiBackupEntry>> {
    let metadata = std::fs::symlink_metadata(source)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!("Kimi session backup root is not a regular directory");
    }
    let mut entries = Vec::new();
    for entry in WalkDir::new(source).follow_links(false).min_depth(1) {
        let entry = entry.with_context(|| {
            format!(
                "Failed to inspect Kimi session backup: {}",
                source.display()
            )
        })?;
        let relative = entry.path().strip_prefix(source)?.to_path_buf();
        validate_relative_path(&relative)?;
        if entry.file_type().is_dir() {
            entries.push(KimiBackupEntry {
                relative_path: relative,
                kind: KimiBackupEntryKind::Directory,
                byte_len: 0,
                sha256: None,
            });
        } else if entry.file_type().is_file() {
            let bytes = std::fs::read(entry.path())?;
            entries.push(KimiBackupEntry {
                relative_path: relative,
                kind: KimiBackupEntryKind::File,
                byte_len: u64::try_from(bytes.len()).context("Kimi backup file is too large")?,
                sha256: Some(sha256_bytes(&bytes)),
            });
        } else {
            anyhow::bail!(
                "Kimi session backup contains unsupported filesystem entry: {}",
                entry.path().display()
            );
        }
    }
    entries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(entries)
}

fn restore_deleted_session(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("Kimi session restore path has no parent directory")?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(
        ".{}.memorph-restore-{}",
        destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("kimi-session"),
        uuid::Uuid::new_v4()
    ));
    let restore_result = (|| -> Result<()> {
        copy_validated_session_tree(source, &temporary)?;
        if destination.exists() {
            std::fs::remove_dir_all(destination)?;
        }
        std::fs::rename(&temporary, destination)?;
        Ok(())
    })();
    if restore_result.is_err() {
        let _ = std::fs::remove_dir_all(&temporary);
    }
    restore_result
}

fn validate_restore_context(
    backup: &ProviderSessionBackup,
    metadata: &KimiSessionBackupMetadata,
) -> Result<()> {
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.mutation != backup.mutation
        || metadata.session_path != backup.source_path
    {
        anyhow::bail!(
            "Kimi backup metadata does not match the registered restore context: {}",
            backup.backup_path.display()
        );
    }
    let current_root = canonical_sessions_root()?;
    if metadata.sessions_root != current_root
        || !metadata.session_path.starts_with(&current_root)
        || metadata
            .session_path
            .file_name()
            .and_then(|name| name.to_str())
            != Some(metadata.provider_session_id.as_str())
        || metadata.state_path != metadata.session_path.join("state.json")
    {
        anyhow::bail!("Kimi backup source paths are invalid");
    }
    let mut seen = HashSet::new();
    for entry in &metadata.entries {
        validate_relative_path(&entry.relative_path)?;
        if !seen.insert(entry.relative_path.clone()) {
            anyhow::bail!("Kimi backup manifest contains duplicate paths");
        }
        match entry.kind {
            KimiBackupEntryKind::Directory if entry.byte_len != 0 || entry.sha256.is_some() => {
                anyhow::bail!("Kimi directory manifest entry is invalid");
            }
            KimiBackupEntryKind::File if entry.sha256.is_none() => {
                anyhow::bail!("Kimi file manifest entry is missing a hash");
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_rename_manifest(
    backup_path: &Path,
    metadata: &KimiSessionBackupMetadata,
) -> Result<()> {
    if metadata.entries.len() != 1
        || metadata.entries[0].relative_path != Path::new("state.json")
        || metadata.entries[0].kind != KimiBackupEntryKind::File
    {
        anyhow::bail!("Kimi rename backup manifest is invalid");
    }
    let bytes = std::fs::read(backup_path.join(KIMI_STATE_BACKUP_PATH))?;
    let entry = &metadata.entries[0];
    let sha256 = sha256_bytes(&bytes);
    if u64::try_from(bytes.len()).ok() != Some(entry.byte_len)
        || entry.sha256.as_deref() != Some(sha256.as_str())
    {
        anyhow::bail!("Kimi state.json backup does not match its manifest");
    }
    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        anyhow::bail!("Kimi backup manifest contains an unsafe relative path");
    }
    Ok(())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
pub(super) fn set_test_backup_failure(enabled: bool) {
    *TEST_KIMI_BACKUP_FAILURE
        .get_or_init(|| std::sync::Mutex::new(false))
        .lock()
        .expect("test Kimi backup failure lock") = enabled;
}

#[cfg(test)]
fn fail_kimi_backup_after_capture() -> Result<()> {
    let mut enabled = TEST_KIMI_BACKUP_FAILURE
        .get_or_init(|| std::sync::Mutex::new(false))
        .lock()
        .expect("test Kimi backup failure lock");
    if *enabled {
        *enabled = false;
        anyhow::bail!("injected Kimi backup failure after native capture");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_kimi_backup_after_capture() -> Result<()> {
    Ok(())
}
