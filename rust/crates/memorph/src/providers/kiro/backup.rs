use super::{fail_kiro_mutation_after_write, write_file_atomically, PROVIDER_ID};
use crate::provider::{ProviderSessionBackup, ProviderSourceMutation};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};

const KIRO_BACKUP_FORMAT: &str = "kiro-session-backup-v1";
const KIRO_BACKUP_MIME: &str = "application/vnd.memorph.kiro-session-backup";

#[cfg(test)]
static TEST_KIRO_BACKUP_FAILURE: std::sync::OnceLock<std::sync::Mutex<bool>> =
    std::sync::OnceLock::new();

#[derive(Debug)]
struct KiroMutationPlan {
    global_dir: PathBuf,
    scopes: Vec<KiroMutationScope>,
}

#[derive(Debug)]
struct KiroMutationScope {
    scope_dir: PathBuf,
    list_path: PathBuf,
    entries: Vec<Value>,
    target_entry: Option<(usize, Value)>,
    session_path: PathBuf,
    session_bytes: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    global_dir: PathBuf,
    scopes: Vec<KiroBackupScope>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroBackupScope {
    scope_dir: PathBuf,
    index_entry: Option<KiroIndexEntryBackup>,
    session_file: KiroFileBackup,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroIndexEntryBackup {
    position: usize,
    relative_path: PathBuf,
    byte_len: u64,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KiroFileBackup {
    present: bool,
    relative_path: Option<PathBuf>,
    byte_len: u64,
    sha256: Option<String>,
}

enum KiroRestoreWrite {
    Write(PathBuf, Vec<u8>),
}

pub(super) fn create_session_backup(
    global_dir: &Path,
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    validate_operation_id(operation_id)?;
    let plan = build_mutation_plan(global_dir, session_id)?;
    let provider_backup_root = backup_root.join(PROVIDER_ID);
    std::fs::create_dir_all(&provider_backup_root).with_context(|| {
        format!(
            "Failed to create Kiro backup root: {}",
            provider_backup_root.display()
        )
    })?;
    let backup_path = provider_backup_root.join(operation_id);
    std::fs::create_dir(&backup_path).with_context(|| {
        format!(
            "Failed to create Kiro session backup: {}",
            backup_path.display()
        )
    })?;

    let create_result = (|| -> Result<ProviderSessionBackup> {
        std::fs::create_dir(backup_path.join("files"))?;
        let mut scopes = Vec::with_capacity(plan.scopes.len());
        for (scope_index, scope) in plan.scopes.iter().enumerate() {
            let index_entry = match &scope.target_entry {
                Some((position, entry)) => {
                    let bytes = serde_json::to_vec(entry)?;
                    let relative_path =
                        PathBuf::from(format!("files/{scope_index:04}-index-entry.json"));
                    std::fs::write(backup_path.join(&relative_path), &bytes)?;
                    Some(KiroIndexEntryBackup {
                        position: *position,
                        relative_path,
                        byte_len: byte_len(&bytes)?,
                        sha256: sha256_bytes(&bytes),
                    })
                }
                None => None,
            };
            let session_file = match &scope.session_bytes {
                Some(bytes) => {
                    let relative_path =
                        PathBuf::from(format!("files/{scope_index:04}-session.json"));
                    std::fs::write(backup_path.join(&relative_path), bytes)?;
                    KiroFileBackup {
                        present: true,
                        relative_path: Some(relative_path),
                        byte_len: byte_len(bytes)?,
                        sha256: Some(sha256_bytes(bytes)),
                    }
                }
                None => KiroFileBackup {
                    present: false,
                    relative_path: None,
                    byte_len: 0,
                    sha256: None,
                },
            };
            scopes.push(KiroBackupScope {
                scope_dir: scope
                    .scope_dir
                    .strip_prefix(&plan.global_dir)
                    .context("Kiro scope is outside the global storage root")?
                    .to_path_buf(),
                index_entry,
                session_file,
            });
        }
        fail_kiro_backup_after_capture()?;

        let metadata = KiroSessionBackupMetadata {
            version: 1,
            provider_id: PROVIDER_ID.to_string(),
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            global_dir: plan.global_dir.clone(),
            scopes,
        };
        std::fs::write(
            backup_path.join("metadata.json"),
            serde_json::to_vec_pretty(&metadata)?,
        )?;

        Ok(ProviderSessionBackup {
            mutation,
            operation_id: operation_id.to_string(),
            provider_session_id: session_id.to_string(),
            source_path: plan.global_dir,
            backup_path: backup_path.clone(),
            restore_hint:
                "Restore this backup with memorph's Kiro native session restore flow before reopening Kiro."
                    .to_string(),
            mime_type: KIRO_BACKUP_MIME.to_string(),
            format: KIRO_BACKUP_FORMAT.to_string(),
            artifact_metadata: serde_json::json!({
                "role": "kiro_prewrite_session_backup",
                "mutation": mutation,
                "scope_count": metadata.scopes.len(),
                "multi_file": true,
            }),
            restore_metadata: serde_json::json!({
                "restore_mode": "kiro_session_restore",
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
    if backup.format != KIRO_BACKUP_FORMAT {
        anyhow::bail!("Unsupported Kiro session backup format: {}", backup.format);
    }
    if backup.mime_type != KIRO_BACKUP_MIME {
        anyhow::bail!(
            "Unsupported Kiro session backup MIME type: {}",
            backup.mime_type
        );
    }

    let metadata_path = backup.backup_path.join("metadata.json");
    let metadata: KiroSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read Kiro backup metadata: {}",
                metadata_path.display()
            )
        })?)?;
    validate_restore_context(backup, &metadata)?;
    let writes = prepare_restore_writes(backup, &metadata)?;
    for write in writes {
        match write {
            KiroRestoreWrite::Write(path, bytes) => write_file_atomically(&path, &bytes)?,
        }
    }
    Ok(())
}

pub(super) fn delete_session(global_dir: &Path, session_id: &str) -> Result<()> {
    let plan = build_mutation_plan(global_dir, session_id)?;
    let mut wrote = false;
    for scope in plan.scopes {
        if scope.target_entry.is_some() {
            let entries = scope
                .entries
                .into_iter()
                .filter(|entry| entry.get("sessionId").and_then(Value::as_str) != Some(session_id))
                .collect::<Vec<_>>();
            write_file_atomically(&scope.list_path, &serde_json::to_vec_pretty(&entries)?)?;
            if !wrote {
                wrote = true;
                fail_kiro_mutation_after_write(ProviderSourceMutation::Delete)?;
            }
        }
        if scope.session_bytes.is_some() {
            std::fs::remove_file(&scope.session_path)?;
            if !wrote {
                wrote = true;
                fail_kiro_mutation_after_write(ProviderSourceMutation::Delete)?;
            }
        }
    }
    Ok(())
}

pub(super) fn rename_session(global_dir: &Path, session_id: &str, new_title: &str) -> Result<()> {
    let plan = build_mutation_plan(global_dir, session_id)?;
    let mut wrote = false;
    for mut scope in plan.scopes {
        if let Some((position, _)) = scope.target_entry {
            let object = scope.entries[position]
                .as_object_mut()
                .context("Kiro target session index entry is not a JSON object")?;
            object.insert("title".to_string(), Value::String(new_title.to_string()));
            write_file_atomically(
                &scope.list_path,
                &serde_json::to_vec_pretty(&scope.entries)?,
            )?;
            if !wrote {
                wrote = true;
                fail_kiro_mutation_after_write(ProviderSourceMutation::Rename)?;
            }
        }
        if let Some(bytes) = scope.session_bytes {
            let mut value: Value = serde_json::from_slice(&bytes)?;
            value
                .as_object_mut()
                .context("Kiro session file must contain a JSON object")?
                .insert("title".to_string(), Value::String(new_title.to_string()));
            write_file_atomically(&scope.session_path, &serde_json::to_vec_pretty(&value)?)?;
            if !wrote {
                fail_kiro_mutation_after_write(ProviderSourceMutation::Rename)?;
            }
        }
    }
    Ok(())
}

fn build_mutation_plan(global_dir: &Path, session_id: &str) -> Result<KiroMutationPlan> {
    validate_session_id(session_id)?;
    let global_metadata = std::fs::symlink_metadata(global_dir).with_context(|| {
        format!(
            "Kiro global storage directory not found: {}",
            global_dir.display()
        )
    })?;
    if !global_metadata.file_type().is_dir() || global_metadata.file_type().is_symlink() {
        anyhow::bail!("Kiro global storage root is not a regular directory");
    }
    let canonical_global = global_dir.canonicalize()?;
    let list_paths = validated_session_list_paths(&canonical_global)?;
    let mut scopes = Vec::new();

    for list_path in list_paths {
        let scope_dir = list_path
            .parent()
            .context("Kiro session list has no parent directory")?
            .to_path_buf();
        let entries = read_validated_session_list(&list_path)?;
        let matches = entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.get("sessionId").and_then(Value::as_str) == Some(session_id))
            .map(|(position, entry)| (position, entry.clone()))
            .collect::<Vec<_>>();
        if matches.len() > 1 {
            anyhow::bail!(
                "Kiro session list contains duplicate entries for {session_id}: {}",
                list_path.display()
            );
        }

        let session_path = scope_dir.join(format!("{session_id}.json"));
        let session_bytes = if path_exists_without_following(&session_path)? {
            let metadata = std::fs::symlink_metadata(&session_path)?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "Kiro session source is not a regular file: {}",
                    session_path.display()
                );
            }
            let canonical_session = session_path.canonicalize()?;
            if !canonical_session.starts_with(&canonical_global) {
                anyhow::bail!("Kiro session source escapes the global storage root");
            }
            let bytes = std::fs::read(&session_path)?;
            validate_session_file(&bytes, session_id, &session_path)?;
            Some(bytes)
        } else {
            None
        };

        if !matches.is_empty() || session_bytes.is_some() {
            scopes.push(KiroMutationScope {
                scope_dir,
                list_path,
                entries,
                target_entry: matches.into_iter().next(),
                session_path,
                session_bytes,
            });
        }
    }

    if scopes.is_empty() {
        anyhow::bail!("Kiro session not found: {session_id}");
    }
    Ok(KiroMutationPlan {
        global_dir: canonical_global,
        scopes,
    })
}

fn validated_session_list_paths(global_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let global_sessions = global_dir.join("sessions");
    collect_list_path(global_dir, &global_sessions, &mut paths)?;

    let workspace_root = global_dir.join("workspace-sessions");
    if path_exists_without_following(&workspace_root)? {
        let metadata = std::fs::symlink_metadata(&workspace_root)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            anyhow::bail!("Kiro workspace-sessions root is not a regular directory");
        }
        let mut entries = std::fs::read_dir(&workspace_root)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                anyhow::bail!(
                    "Kiro workspace session scope cannot be a symlink: {}",
                    entry.path().display()
                );
            }
            if metadata.file_type().is_dir() {
                collect_list_path(global_dir, &entry.path(), &mut paths)?;
            }
        }
    }
    Ok(paths)
}

fn collect_list_path(global_dir: &Path, scope_dir: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    if !path_exists_without_following(scope_dir)? {
        return Ok(());
    }
    let scope_metadata = std::fs::symlink_metadata(scope_dir)?;
    if !scope_metadata.file_type().is_dir() || scope_metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kiro session scope is not a regular directory: {}",
            scope_dir.display()
        );
    }
    let canonical_scope = scope_dir.canonicalize()?;
    if !canonical_scope.starts_with(global_dir) {
        anyhow::bail!("Kiro session scope escapes the global storage root");
    }
    let list_path = scope_dir.join("sessions.json");
    if !path_exists_without_following(&list_path)? {
        return Ok(());
    }
    let list_metadata = std::fs::symlink_metadata(&list_path)?;
    if !list_metadata.file_type().is_file() || list_metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kiro session list is not a regular file: {}",
            list_path.display()
        );
    }
    let canonical_list = list_path.canonicalize()?;
    if !canonical_list.starts_with(global_dir) {
        anyhow::bail!("Kiro session list escapes the global storage root");
    }
    paths.push(canonical_list);
    Ok(())
}

fn read_validated_session_list(path: &Path) -> Result<Vec<Value>> {
    let bytes = std::fs::read(path)?;
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("Failed to parse Kiro session list: {}", path.display()))?;
    value.as_array().cloned().with_context(|| {
        format!(
            "Kiro session list must contain a JSON array: {}",
            path.display()
        )
    })
}

fn validate_session_file(bytes: &[u8], session_id: &str, path: &Path) -> Result<()> {
    let value: Value = serde_json::from_slice(bytes)
        .with_context(|| format!("Failed to parse Kiro session file: {}", path.display()))?;
    let object = value.as_object().with_context(|| {
        format!(
            "Kiro session file must contain a JSON object: {}",
            path.display()
        )
    })?;
    if object.get("sessionId").and_then(Value::as_str) != Some(session_id) {
        anyhow::bail!(
            "Kiro session file identity does not match {session_id}: {}",
            path.display()
        );
    }
    Ok(())
}

fn validate_restore_context(
    backup: &ProviderSessionBackup,
    metadata: &KiroSessionBackupMetadata,
) -> Result<()> {
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.mutation != backup.mutation
        || metadata.global_dir != backup.source_path
    {
        anyhow::bail!(
            "Kiro backup metadata does not match the registered restore context: {}",
            backup.backup_path.display()
        );
    }
    validate_session_id(&metadata.provider_session_id)?;
    let global_metadata = std::fs::symlink_metadata(&metadata.global_dir)?;
    if !global_metadata.file_type().is_dir() || global_metadata.file_type().is_symlink() {
        anyhow::bail!("Kiro restore root is not a regular directory");
    }
    if metadata.global_dir.canonicalize()? != metadata.global_dir {
        anyhow::bail!("Kiro restore root does not match its canonical source path");
    }
    if metadata.scopes.is_empty() {
        anyhow::bail!("Kiro backup does not contain any mutation scopes");
    }
    Ok(())
}

fn prepare_restore_writes(
    backup: &ProviderSessionBackup,
    metadata: &KiroSessionBackupMetadata,
) -> Result<Vec<KiroRestoreWrite>> {
    let mut writes = Vec::new();
    let mut seen_scopes = std::collections::HashSet::new();
    for (scope_index, scope) in metadata.scopes.iter().enumerate() {
        validate_scope_dir(&scope.scope_dir)?;
        if !seen_scopes.insert(scope.scope_dir.clone()) {
            anyhow::bail!("Kiro backup contains duplicate scope paths");
        }
        let scope_dir = metadata.global_dir.join(&scope.scope_dir);
        validate_restore_scope(&metadata.global_dir, &scope_dir)?;
        let list_path = scope_dir.join("sessions.json");
        let session_path = scope_dir.join(format!("{}.json", metadata.provider_session_id));

        if let Some(index) = &scope.index_entry {
            let expected_path = PathBuf::from(format!("files/{scope_index:04}-index-entry.json"));
            if index.relative_path != expected_path {
                anyhow::bail!("Kiro backup index payload path does not match its scope");
            }
            let backup_entry = read_manifest_json(
                &backup.backup_path,
                &index.relative_path,
                index.byte_len,
                &index.sha256,
            )?;
            if backup_entry.get("sessionId").and_then(Value::as_str)
                != Some(&metadata.provider_session_id)
            {
                anyhow::bail!("Kiro backup index entry has a mismatched session identity");
            }
            if !backup_entry.is_object() {
                anyhow::bail!("Kiro backup index entry must contain a JSON object");
            }
            if let Some(bytes) = prepare_index_restore(
                &list_path,
                metadata.mutation,
                &metadata.provider_session_id,
                index.position,
                &backup_entry,
            )? {
                writes.push(KiroRestoreWrite::Write(list_path, bytes));
            }
        }

        match (
            &scope.session_file.present,
            &scope.session_file.relative_path,
        ) {
            (true, Some(relative_path)) => {
                let expected_path = PathBuf::from(format!("files/{scope_index:04}-session.json"));
                if relative_path != &expected_path {
                    anyhow::bail!("Kiro backup session payload path does not match its scope");
                }
                let expected_hash = scope
                    .session_file
                    .sha256
                    .as_deref()
                    .context("Kiro session backup manifest is missing its hash")?;
                let bytes = read_manifest_bytes(
                    &backup.backup_path,
                    relative_path,
                    scope.session_file.byte_len,
                    expected_hash,
                )?;
                validate_session_file(
                    &bytes,
                    &metadata.provider_session_id,
                    &backup.backup_path.join(relative_path),
                )?;
                match metadata.mutation {
                    ProviderSourceMutation::Delete => {
                        validate_optional_restore_file(
                            &metadata.global_dir,
                            &scope_dir,
                            &session_path,
                        )?;
                        writes.push(KiroRestoreWrite::Write(session_path, bytes));
                    }
                    ProviderSourceMutation::Rename => {
                        if let Some(current_bytes) = read_optional_restore_file(
                            &metadata.global_dir,
                            &scope_dir,
                            &session_path,
                        )? {
                            validate_session_file(
                                &current_bytes,
                                &metadata.provider_session_id,
                                &session_path,
                            )?;
                            let restored =
                                restore_title(&current_bytes, &bytes, "Kiro session file")?;
                            writes.push(KiroRestoreWrite::Write(session_path, restored));
                        }
                    }
                }
            }
            (false, None)
                if scope.session_file.byte_len == 0 && scope.session_file.sha256.is_none() => {}
            _ => anyhow::bail!("Kiro session backup presence manifest is inconsistent"),
        }
    }
    Ok(writes)
}

fn prepare_index_restore(
    list_path: &Path,
    mutation: ProviderSourceMutation,
    session_id: &str,
    position: usize,
    backup_entry: &Value,
) -> Result<Option<Vec<u8>>> {
    if !path_exists_without_following(list_path)? {
        return match mutation {
            ProviderSourceMutation::Delete => {
                Ok(Some(serde_json::to_vec_pretty(
                    &vec![backup_entry.clone()],
                )?))
            }
            ProviderSourceMutation::Rename => Ok(None),
        };
    }
    let metadata = std::fs::symlink_metadata(list_path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kiro restore session list is not a regular file: {}",
            list_path.display()
        );
    }
    let mut entries = read_validated_session_list(list_path)?;
    let matches = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.get("sessionId").and_then(Value::as_str) == Some(session_id))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if matches.len() > 1 {
        anyhow::bail!(
            "Kiro restore session list contains duplicate entries for {session_id}: {}",
            list_path.display()
        );
    }
    match mutation {
        ProviderSourceMutation::Delete => {
            entries
                .retain(|entry| entry.get("sessionId").and_then(Value::as_str) != Some(session_id));
            entries.insert(position.min(entries.len()), backup_entry.clone());
            Ok(Some(serde_json::to_vec_pretty(&entries)?))
        }
        ProviderSourceMutation::Rename => {
            let Some(current_position) = matches.first().copied() else {
                return Ok(None);
            };
            restore_value_field(
                &mut entries[current_position],
                backup_entry,
                "title",
                "Kiro session index entry",
            )?;
            Ok(Some(serde_json::to_vec_pretty(&entries)?))
        }
    }
}

fn restore_title(current: &[u8], backup: &[u8], label: &str) -> Result<Vec<u8>> {
    let mut current_value: Value = serde_json::from_slice(current)?;
    let backup_value: Value = serde_json::from_slice(backup)?;
    restore_value_field(&mut current_value, &backup_value, "title", label)?;
    Ok(serde_json::to_vec_pretty(&current_value)?)
}

fn restore_value_field(
    current: &mut Value,
    backup: &Value,
    field: &str,
    label: &str,
) -> Result<()> {
    let current_object = current
        .as_object_mut()
        .with_context(|| format!("{label} is not a JSON object"))?;
    let backup_object = backup
        .as_object()
        .with_context(|| format!("Backup {label} is not a JSON object"))?;
    match backup_object.get(field) {
        Some(value) => {
            current_object.insert(field.to_string(), value.clone());
        }
        None => {
            current_object.remove(field);
        }
    }
    Ok(())
}

fn read_manifest_json(
    backup_root: &Path,
    relative_path: &Path,
    byte_len: u64,
    sha256: &str,
) -> Result<Value> {
    let bytes = read_manifest_bytes(backup_root, relative_path, byte_len, sha256)?;
    serde_json::from_slice(&bytes).context("Failed to parse Kiro backup JSON payload")
}

fn read_manifest_bytes(
    backup_root: &Path,
    relative_path: &Path,
    expected_byte_len: u64,
    sha256: &str,
) -> Result<Vec<u8>> {
    validate_backup_relative_path(relative_path)?;
    let path = backup_root.join(relative_path);
    let metadata = std::fs::symlink_metadata(&path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kiro backup payload is not a regular file: {}",
            path.display()
        );
    }
    let bytes = std::fs::read(&path)?;
    if byte_len(&bytes)? != expected_byte_len || sha256_bytes(&bytes) != sha256 {
        anyhow::bail!(
            "Kiro backup payload does not match its manifest: {}",
            path.display()
        );
    }
    Ok(bytes)
}

fn validate_scope_dir(path: &Path) -> Result<()> {
    let components = path.components().collect::<Vec<_>>();
    let valid = matches!(
        components.as_slice(),
        [Component::Normal(root)]
            if *root == std::ffi::OsStr::new("sessions")
    ) || matches!(
        components.as_slice(),
        [Component::Normal(root), Component::Normal(_)]
            if *root == std::ffi::OsStr::new("workspace-sessions")
    );
    if !valid {
        anyhow::bail!("Kiro backup scope path is invalid: {}", path.display());
    }
    Ok(())
}

fn validate_restore_scope(global_dir: &Path, scope_dir: &Path) -> Result<()> {
    if !path_exists_without_following(scope_dir)? {
        anyhow::bail!(
            "Kiro restore scope no longer exists: {}",
            scope_dir.display()
        );
    }
    let metadata = std::fs::symlink_metadata(scope_dir)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kiro restore scope is not a regular directory: {}",
            scope_dir.display()
        );
    }
    if !scope_dir.canonicalize()?.starts_with(global_dir) {
        anyhow::bail!("Kiro restore scope escapes the global storage root");
    }
    Ok(())
}

fn validate_optional_restore_file(global_dir: &Path, scope_dir: &Path, path: &Path) -> Result<()> {
    if !path_exists_without_following(path)? {
        return Ok(());
    }
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Kiro restore target is not a regular file: {}",
            path.display()
        );
    }
    if !scope_dir.canonicalize()?.starts_with(global_dir) {
        anyhow::bail!("Kiro restore target escapes the global storage root");
    }
    Ok(())
}

fn read_optional_restore_file(
    global_dir: &Path,
    scope_dir: &Path,
    path: &Path,
) -> Result<Option<Vec<u8>>> {
    if !path_exists_without_following(path)? {
        return Ok(None);
    }
    validate_optional_restore_file(global_dir, scope_dir, path)?;
    Ok(Some(std::fs::read(path)?))
}

fn validate_backup_relative_path(path: &Path) -> Result<()> {
    let mut components = path.components();
    if components.next() != Some(Component::Normal(std::ffi::OsStr::new("files")))
        || components.clone().count() != 1
        || !components.all(|component| matches!(component, Component::Normal(_)))
    {
        anyhow::bail!("Kiro backup payload path is invalid: {}", path.display());
    }
    Ok(())
}

fn validate_session_id(session_id: &str) -> Result<()> {
    if session_id.is_empty()
        || Path::new(session_id).components().count() != 1
        || !matches!(
            Path::new(session_id).components().next(),
            Some(Component::Normal(_))
        )
    {
        anyhow::bail!("Kiro session id is not a safe file identity");
    }
    Ok(())
}

fn validate_operation_id(operation_id: &str) -> Result<()> {
    if operation_id.is_empty()
        || Path::new(operation_id).components().count() != 1
        || !matches!(
            Path::new(operation_id).components().next(),
            Some(Component::Normal(_))
        )
    {
        anyhow::bail!("Kiro operation id is not a safe backup identity");
    }
    Ok(())
}

fn path_exists_without_following(path: &Path) -> Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn byte_len(bytes: &[u8]) -> Result<u64> {
    u64::try_from(bytes.len()).context("Kiro backup payload is too large")
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn fail_kiro_backup_after_capture() -> Result<()> {
    #[cfg(test)]
    {
        let mut configured = TEST_KIRO_BACKUP_FAILURE
            .get_or_init(|| std::sync::Mutex::new(false))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *configured {
            *configured = false;
            anyhow::bail!("injected Kiro backup failure after capture");
        }
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn set_test_backup_failure(enabled: bool) {
    *TEST_KIRO_BACKUP_FAILURE
        .get_or_init(|| std::sync::Mutex::new(false))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = enabled;
}
