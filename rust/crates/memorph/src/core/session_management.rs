use super::transfer::ExportResult;
use anyhow::{Context as _, Result};
use rusqlite::Connection;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::session::{
    Context, Identity, ImportedSession, Provenance, ProviderRef, Schema, Session,
};
use crate::core::compression;
use crate::format;
use crate::provider;
use crate::provider::{ProviderSessionBackup, ProviderSourceMutation};
use crate::providers;
use crate::storage::{
    activity_store::ActivityActor,
    artifact_store::{
        ArtifactManifestKind, ArtifactStore, ArtifactVerification, ArtifactVerificationStatus,
        BackupEntry, BackupQuery, BackupRecord, BackupRestoreRecord, BackupRestoreStatus,
        NewBackupRecord,
    },
    local_store, session_state,
};

use super::compression_application::{
    ExpandCompressionSessionParams, RestoreCompressionArchiveParams,
};
use super::session_mutation::RenameResult;

pub fn normalized_workspace_key(provider_id: &str, workspace: Option<&str>) -> Option<String> {
    providers::find_provider(provider_id)
        .map(|provider| provider.normalized_workspace_key(workspace))
        .unwrap_or_else(|| provider::default_normalized_workspace_key(workspace))
}

pub fn workspace_matches(
    provider_id: &str,
    session_workspace: Option<&str>,
    requested_workspace: Option<&str>,
) -> bool {
    providers::find_provider(provider_id)
        .map(|provider| provider.workspace_matches(session_workspace, requested_workspace))
        .unwrap_or_else(|| {
            provider::default_workspace_matches(session_workspace, requested_workspace)
        })
}

pub fn resolve_existing_target_dir(provider_id: &str, input: Option<&str>) -> Result<PathBuf> {
    let provider = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    provider.resolve_workspace_dir(input)
}

pub fn delete_sessions(
    provider_id: &str,
    session_ids: &[&str],
    operation_ids: &[String],
    backup_root: &Path,
    artifact_conn: &mut Connection,
) -> Vec<Result<()>> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id));
    let prov = match prov {
        Ok(provider) => provider,
        Err(err) => {
            let message = err.to_string();
            return session_ids
                .iter()
                .map(|session_id| {
                    Err(anyhow::anyhow!(
                        "Failed to delete session {}: {}",
                        session_id,
                        message
                    ))
                })
                .collect();
        }
    };
    delete_sessions_with_provider(
        prov.as_ref(),
        provider_id,
        session_ids,
        operation_ids,
        backup_root,
        artifact_conn,
    )
}

fn delete_sessions_with_provider(
    prov: &dyn provider::Provider,
    provider_id: &str,
    session_ids: &[&str],
    operation_ids: &[String],
    backup_root: &Path,
    artifact_conn: &mut Connection,
) -> Vec<Result<()>> {
    if !prov.capabilities().delete {
        return session_ids
            .iter()
            .map(|session_id| {
                Err(anyhow::anyhow!(
                    "Provider does not support deleting sessions: {} ({})",
                    provider_id,
                    session_id
                ))
            })
            .collect();
    }
    if operation_ids.len() != session_ids.len() {
        return session_ids
            .iter()
            .map(|session_id| {
                Err(anyhow::anyhow!(
                    "Delete operation identity count does not match session count for {} ({})",
                    provider_id,
                    session_id
                ))
            })
            .collect();
    }

    let mut backups = Vec::with_capacity(session_ids.len());
    for (session_id, operation_id) in session_ids.iter().zip(operation_ids) {
        match register_provider_session_backup(
            prov,
            provider_id,
            ProviderSourceMutation::Delete,
            operation_id,
            session_id,
            backup_root,
            artifact_conn,
        ) {
            Ok(backup) => backups.push(backup),
            Err(error) => {
                let message = format!(
                    "Delete cancelled before provider write because the native backup failed for {} ({}): {error:#}",
                    provider_id, session_id
                );
                return session_ids
                    .iter()
                    .map(|_| Err(anyhow::anyhow!(message.clone())))
                    .collect();
            }
        }
    }

    let provider_results = prov.delete_sessions(session_ids);
    if provider_results.len() != session_ids.len() {
        let restore_errors = restore_provider_session_backups(prov, &backups);
        let message = format!(
            "Provider {} returned {} delete results for {} sessions{}",
            provider_id,
            provider_results.len(),
            session_ids.len(),
            format_restore_errors(&restore_errors)
        );
        return session_ids
            .iter()
            .map(|_| Err(anyhow::anyhow!(message.clone())))
            .collect();
    }

    provider_results
        .into_iter()
        .zip(session_ids)
        .zip(backups)
        .map(|((result, session_id), backup)| match result {
            Ok(()) => match session_state::remove_session(provider_id, session_id) {
                Ok(()) => Ok(()),
                Err(error) => Err(restore_provider_session_after_failure(
                    prov,
                    backup.as_ref(),
                    error.context("Provider delete succeeded but local session cleanup failed"),
                )),
            },
            Err(error) => Err(restore_provider_session_after_failure(
                prov,
                backup.as_ref(),
                error,
            )),
        })
        .collect()
}

pub fn rename_session(
    provider_id: &str,
    session_id: &str,
    new_title: &str,
    operation_id: &str,
    backup_root: &Path,
    artifact_conn: &mut Connection,
) -> Result<RenameResult> {
    let new_title = new_title.trim();
    if new_title.is_empty() {
        anyhow::bail!("Session title cannot be empty");
    }

    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {}", provider_id))?;
    rename_session_with_provider(
        prov.as_ref(),
        provider_id,
        session_id,
        new_title,
        operation_id,
        backup_root,
        artifact_conn,
    )
}

fn rename_session_with_provider(
    prov: &dyn provider::Provider,
    provider_id: &str,
    session_id: &str,
    new_title: &str,
    operation_id: &str,
    backup_root: &Path,
    artifact_conn: &mut Connection,
) -> Result<RenameResult> {
    let capabilities = prov.capabilities();
    if capabilities.scan {
        let cache = crate::cache::global_cache();
        let exists = cache
            .get_or_refresh(provider_id, || prov.scan_sessions())?
            .into_iter()
            .any(|session| session.session_id == session_id);
        if !exists {
            anyhow::bail!("Session not found: {}", session_id);
        }
    }

    let mut warning = None;
    let (native_updated, backup) = if capabilities.rename {
        let backup = register_provider_session_backup(
            prov,
            provider_id,
            ProviderSourceMutation::Rename,
            operation_id,
            session_id,
            backup_root,
            artifact_conn,
        )?;
        if let Err(error) = prov.rename_session(session_id, new_title) {
            return Err(restore_provider_session_after_failure(
                prov,
                backup.as_ref(),
                error,
            ));
        }
        (true, backup)
    } else {
        warning = Some(format!(
            "Provider does not support native rename; memorph display title was saved: {}",
            provider_id
        ));
        (false, None)
    };

    if let Err(error) = session_state::set_display_title(provider_id, session_id, new_title) {
        return Err(restore_provider_session_after_failure(
            prov,
            backup.as_ref(),
            error.context("Provider rename succeeded but local display title update failed"),
        ));
    }
    Ok(RenameResult {
        provider_name: prov.name().to_string(),
        session_id: session_id.to_string(),
        display_title: new_title.to_string(),
        native_updated,
        warning,
    })
}

#[derive(Debug)]
pub struct NativeSessionReplaceResult {
    pub imported: ImportedSession,
    pub source_bytes_before: u64,
    pub source_bytes_after: u64,
}

pub fn replace_native_session(
    provider_id: &str,
    session_id: &str,
    session: &Session,
    expected_archive_refs: &[String],
    operation_id: &str,
    backup_root: &Path,
    artifact_conn: &mut Connection,
) -> Result<NativeSessionReplaceResult> {
    let prov = providers::find_provider(provider_id)
        .with_context(|| format!("Unknown provider: {provider_id}"))?;
    if !prov.supports_native_session_replace() {
        anyhow::bail!("Provider does not support native session replacement: {provider_id}");
    }
    let provider_id = providers::canonical_provider_id(provider_id);
    let busy = crate::hooks::runtime_state::runtime_sessions_snapshot()
        .into_iter()
        .any(|runtime| {
            providers::canonical_provider_id(&runtime.provider) == provider_id
                && runtime.provider_session_id.as_deref() == Some(session_id)
                && !matches!(
                    runtime.status,
                    crate::hooks::model::RuntimeSessionStatus::Completed
                        | crate::hooks::model::RuntimeSessionStatus::Failed
                        | crate::hooks::model::RuntimeSessionStatus::Orphaned
                )
        });
    if busy {
        anyhow::bail!("Cannot replace active provider session: {provider_id}/{session_id}");
    }
    let support = prov.capabilities().backup_support;
    if !support.before_write || !support.restore || support.sync_only {
        anyhow::bail!("Provider cannot safely back up native session replacement: {provider_id}");
    }
    let source = prov
        .get_session_meta(session_id)?
        .and_then(|meta| meta.source_path)
        .with_context(|| {
            format!("Provider session source not found: {provider_id}/{session_id}")
        })?;
    let source_bytes_before = prov.session_size(session_id)?;
    let backup = register_provider_session_backup(
        prov.as_ref(),
        &provider_id,
        ProviderSourceMutation::Replace,
        operation_id,
        session_id,
        backup_root,
        artifact_conn,
    )?
    .context("Native session replacement requires a registered backup")?;

    let result = (|| -> Result<NativeSessionReplaceResult> {
        prov.replace_session(session_id, session)?;
        let imported = prov.import_session(&source)?;
        if imported.session.identity.id != session.identity.id {
            anyhow::bail!("Native replacement changed canonical session identity");
        }
        let mut actual_refs = compression::compressed_archive_refs(&imported.session);
        let mut expected_refs = expected_archive_refs.to_vec();
        actual_refs.sort();
        expected_refs.sort();
        if actual_refs != expected_refs {
            anyhow::bail!("Native replacement validation found different compression archive refs");
        }
        Ok(NativeSessionReplaceResult {
            source_bytes_before,
            source_bytes_after: prov.session_size(session_id)?,
            imported,
        })
    })();
    result.map_err(|error| {
        restore_provider_session_after_failure(prov.as_ref(), Some(&backup), error)
    })
}

fn register_provider_session_backup(
    prov: &dyn provider::Provider,
    provider_id: &str,
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
    artifact_conn: &mut Connection,
) -> Result<Option<ProviderSessionBackup>> {
    let backup_support = prov.capabilities().backup_support;
    if !backup_support.before_write {
        return Ok(None);
    }
    if !backup_support.restore {
        anyhow::bail!(
            "Provider {} advertises pre-write backup without restore support",
            provider_id
        );
    }

    let backup = prov.create_session_backup(mutation, operation_id, session_id, backup_root)?;
    if backup.mutation != mutation
        || backup.operation_id != operation_id
        || backup.provider_session_id != session_id
    {
        anyhow::bail!(
            "Provider {} returned a backup with mismatched mutation identity",
            provider_id
        );
    }
    ArtifactStore::new(artifact_conn).register_backup(NewBackupRecord {
        operation_id: Some(operation_id.to_string()),
        provider_id: Some(provider_id.to_string()),
        provider_session_id: Some(session_id.to_string()),
        session_id: None,
        source_path: Some(backup.source_path.clone()),
        backup_path: backup.backup_path.clone(),
        restore_hint: Some(backup.restore_hint.clone()),
        mime_type: Some(backup.mime_type.clone()),
        format: Some(backup.format.clone()),
        artifact_metadata: backup.artifact_metadata.clone(),
        backup_metadata: backup.restore_metadata.clone(),
    })?;
    Ok(Some(backup))
}

fn restore_provider_session_after_failure(
    prov: &dyn provider::Provider,
    backup: Option<&ProviderSessionBackup>,
    mutation_error: anyhow::Error,
) -> anyhow::Error {
    let Some(backup) = backup else {
        return mutation_error;
    };
    match prov.restore_session_backup(backup) {
        Ok(()) => mutation_error.context(format!(
            "Provider source was restored from registered backup {}",
            backup.backup_path.display()
        )),
        Err(restore_error) => anyhow::anyhow!(
            "Provider mutation failed: {mutation_error:#}; native backup restore also failed for {}: {restore_error:#}",
            backup.backup_path.display()
        ),
    }
}

fn restore_provider_session_backups(
    prov: &dyn provider::Provider,
    backups: &[Option<ProviderSessionBackup>],
) -> Vec<String> {
    backups
        .iter()
        .filter_map(|backup| {
            let backup = backup.as_ref()?;
            prov.restore_session_backup(backup)
                .err()
                .map(|error| format!("{}: {error:#}", backup.backup_path.display()))
        })
        .collect()
}

fn format_restore_errors(errors: &[String]) -> String {
    if errors.is_empty() {
        String::new()
    } else {
        format!("; backup restore failures: {}", errors.join("; "))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct BackupView {
    pub entry: BackupEntry,
    pub verification: ArtifactVerification,
}

pub fn list_registered_backups(query: BackupQuery) -> Result<Vec<BackupView>> {
    let mut conn = local_store::open_database()?;
    let entries = ArtifactStore::new(&mut conn).query_backups(query)?;
    entries
        .into_iter()
        .map(|entry| backup_view(&mut conn, entry))
        .collect()
}

pub fn get_registered_backup(backup_id: &str) -> Result<Option<BackupView>> {
    let mut conn = local_store::open_database()?;
    let Some(entry) = ArtifactStore::new(&mut conn).get_backup_entry(backup_id)? else {
        return Ok(None);
    };
    backup_view(&mut conn, entry).map(Some)
}

pub fn restore_registered_backup(
    backup_id: &str,
    actor: ActivityActor,
) -> Result<BackupRestoreRecord> {
    let mut conn = local_store::open_database()?;
    restore_registered_backup_with(&mut conn, backup_id, actor, |conn, backup| {
        let provider_id = backup
            .provider_id
            .as_deref()
            .context("Registered backup is missing provider identity")?;
        let prov = providers::find_provider(provider_id)
            .with_context(|| format!("Unknown provider for registered backup: {provider_id}"))?;
        restore_registered_backup_payload(conn, prov.as_ref(), provider_id, backup)
    })
}

fn backup_view(conn: &mut Connection, entry: BackupEntry) -> Result<BackupView> {
    let verification = ArtifactStore::new(conn)
        .verify(&entry.backup.artifact.id)?
        .context("Registered backup artifact manifest is missing")?;
    Ok(BackupView {
        entry,
        verification,
    })
}

fn restore_registered_backup_with<F>(
    conn: &mut Connection,
    backup_id: &str,
    actor: ActivityActor,
    restore: F,
) -> Result<BackupRestoreRecord>
where
    F: FnOnce(&mut Connection, &BackupRecord) -> Result<()>,
{
    let backup = ArtifactStore::new(conn)
        .get_backup(backup_id)?
        .with_context(|| format!("Unknown backup: {backup_id}"))?;
    let attempt = ArtifactStore::new(conn).start_backup_restore(backup_id, actor)?;
    let result = restore(conn, &backup);
    match result {
        Ok(()) => ArtifactStore::new(conn).finish_backup_restore(
            &attempt.id,
            BackupRestoreStatus::Success,
            None,
        ),
        Err(error) => {
            let message = format!("{error:#}");
            ArtifactStore::new(conn).finish_backup_restore(
                &attempt.id,
                BackupRestoreStatus::Failed,
                Some(&message),
            )?;
            Err(error)
        }
    }
}

fn restore_registered_backup_payload(
    conn: &mut Connection,
    prov: &dyn provider::Provider,
    expected_provider_id: &str,
    backup: &BackupRecord,
) -> Result<()> {
    let native_backup = reconstruct_provider_session_backup(backup, expected_provider_id)?;
    let support = prov.capabilities().backup_support;
    if !support.before_write || !support.restore || support.sync_only {
        anyhow::bail!(
            "Provider does not support manual native session restore: {}",
            expected_provider_id
        );
    }
    if prov.id() != expected_provider_id {
        anyhow::bail!(
            "Registered backup provider mismatch: expected {}, resolved {}",
            expected_provider_id,
            prov.id()
        );
    }
    let verification = ArtifactStore::new(conn)
        .verify(&backup.artifact.id)?
        .context("Registered backup artifact manifest is missing")?;
    if verification.status != ArtifactVerificationStatus::Verified {
        anyhow::bail!(
            "Registered backup artifact failed integrity verification: {} ({})",
            backup.id,
            verification.status
        );
    }
    prov.restore_session_backup(&native_backup)
}

fn reconstruct_provider_session_backup(
    backup: &BackupRecord,
    expected_provider_id: &str,
) -> Result<ProviderSessionBackup> {
    if backup.artifact.artifact_kind != ArtifactManifestKind::SessionBackup {
        anyhow::bail!("Registered backup does not reference a session backup artifact");
    }
    if backup.provider_id.as_deref() != Some(expected_provider_id)
        || backup.artifact.provider_id.as_deref() != Some(expected_provider_id)
        || backup.operation_id != backup.artifact.operation_id
        || backup.provider_session_id != backup.artifact.provider_session_id
    {
        anyhow::bail!("Registered backup identity does not match its artifact manifest");
    }
    let operation_id = backup
        .operation_id
        .clone()
        .context("Registered backup is missing operation identity")?;
    let provider_session_id = backup
        .provider_session_id
        .clone()
        .context("Registered backup is missing provider session identity")?;
    let source_path = backup
        .source_path
        .clone()
        .context("Registered backup is not a native provider session backup")?;
    let restore_hint = backup
        .restore_hint
        .clone()
        .context("Registered backup is missing restore instructions")?;
    let mime_type = backup
        .artifact
        .mime_type
        .clone()
        .context("Registered backup is missing MIME identity")?;
    let format = backup
        .artifact
        .format
        .clone()
        .context("Registered backup is missing format identity")?;
    let artifact_mutation = backup
        .artifact
        .metadata
        .get("mutation")
        .cloned()
        .context("Registered backup artifact is missing mutation identity")?;
    let restore_mutation = backup
        .metadata
        .get("mutation")
        .cloned()
        .context("Registered backup restore metadata is missing mutation identity")?;
    let artifact_mutation: ProviderSourceMutation =
        serde_json::from_value(artifact_mutation).context("Invalid artifact mutation identity")?;
    let restore_mutation: ProviderSourceMutation =
        serde_json::from_value(restore_mutation).context("Invalid restore mutation identity")?;
    if artifact_mutation != restore_mutation {
        anyhow::bail!("Registered backup mutation identities do not match");
    }

    Ok(ProviderSessionBackup {
        mutation: artifact_mutation,
        operation_id,
        provider_session_id,
        source_path,
        backup_path: backup.artifact.path.clone(),
        restore_hint,
        mime_type,
        format,
        artifact_metadata: backup.artifact.metadata.clone(),
        restore_metadata: backup.metadata.clone(),
    })
}

pub fn prepare_session_for_export(
    session: &Session,
    source_provider_id: &str,
    target_provider_id: &str,
) -> Result<(Session, compression::CompressionReport)> {
    let policy = compression::CompressionPolicy::preserve(source_provider_id, target_provider_id);
    compression::prepare_for_export_with_archive(session, &policy)
}

pub fn prepare_session_for_target_provider(
    session: &Session,
    target_provider_id: &str,
) -> Result<(Session, compression::CompressionReport)> {
    let source_provider_id = session.provenance.primary_source.provider_id.trim();
    let source_provider_id = if source_provider_id.is_empty() {
        target_provider_id
    } else {
        source_provider_id
    };
    prepare_session_for_export(session, source_provider_id, target_provider_id)
}

pub fn expand_compression_session(
    params: &ExpandCompressionSessionParams,
    session: &Session,
) -> Result<ExportResult> {
    let source_provider_id = session.provenance.primary_source.provider_id.trim();
    let source_provider_id = if source_provider_id.is_empty() {
        "memorph"
    } else {
        source_provider_id
    };
    let policy = compression::CompressionPolicy::expand(source_provider_id, source_provider_id);
    let (expanded, _) = compression::prepare_for_export_with_archive(session, &policy)?;
    let default_prefix = Path::new(&params.file)
        .file_stem()
        .and_then(|value| value.to_str())
        .map(|value| format!("{}_expanded", value))
        .unwrap_or_else(|| format!("{}_expanded", session.identity.id));
    let prefix = params.output_prefix.as_deref().unwrap_or(&default_prefix);
    write_session_export_files(&expanded, prefix, &params.format, None)
}

pub fn restore_compression_archive(
    params: &RestoreCompressionArchiveParams,
    session: &Session,
) -> Result<ExportResult> {
    let default_prefix = format!("{}_compression_archive", session.identity.id);
    let prefix = params.output_prefix.as_deref().unwrap_or(&default_prefix);
    write_session_export_files(session, prefix, &params.format, None)
}

pub fn list_compression_archives(
    workspace: Option<&str>,
) -> Result<Vec<compression::CompressionArchiveSummary>> {
    compression::list_archives_for_workspace(workspace)
}

pub fn list_compression_provider_support() -> Vec<crate::provider::ProviderCompressionSupport> {
    providers::all_provider_ids()
        .iter()
        .filter_map(|provider_id| {
            let provider = providers::find_provider(provider_id)?;
            let default_projection = provider.compression_projection();
            Some(crate::provider::ProviderCompressionSupport {
                provider_id: (*provider_id).to_string(),
                detects_native_source: provider.detects_native_compression_source(),
                native_target_projection: default_projection
                    == crate::provider::CompressionProjection::Native,
                native_session_replace: provider.supports_native_session_replace(),
                native_session_restore: provider.supports_native_session_replace(),
                default_projection,
            })
        })
        .collect()
}

pub fn read_session_export_file(file: &str) -> Result<Session> {
    let path = Path::new(file);
    if file.ends_with(".morph") {
        format::read_session(path)
    } else if file.ends_with(".json") {
        let json = std::fs::read_to_string(path)?;
        serde_json::from_str(&json).with_context(|| format!("Failed to parse JSON: {}", file))
    } else if file.ends_with(".md") {
        format::read_markdown(path)
    } else if file.ends_with(".html") {
        format::read_html(path)
    } else {
        anyhow::bail!(
            "Unsupported session file: {}. Use .json, .md, .html, or .morph",
            file
        );
    }
}

pub fn write_session_export_files(
    session: &Session,
    prefix: &str,
    format_name: &str,
    output_dir: Option<&Path>,
) -> Result<ExportResult> {
    let mut files = Vec::new();

    let write_morph = format_name == "morph" || format_name == "both";
    let write_json = format_name == "json" || format_name == "both";
    let write_markdown = format_name == "md" || format_name == "markdown";
    let write_html = format_name == "html";

    if !write_morph && !write_json && !write_markdown && !write_html {
        anyhow::bail!(
            "Unsupported format: {}. Use 'json', 'md', 'html', 'morph', or 'both'",
            format_name
        );
    }

    if let Some(dir) = output_dir {
        if !dir.exists() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("Failed to create output directory: {}", dir.display()))?;
        }
    }

    let base = output_dir.map(Path::to_path_buf).unwrap_or_default();

    if write_morph {
        let path = base.join(format!("{}.morph", prefix));
        format::write_session(&path, session)?;
        files.push(path.display().to_string());
    }
    if write_json {
        let path = base.join(format!("{}.json", prefix));
        let json = serde_json::to_string_pretty(session)?;
        std::fs::write(&path, json)?;
        files.push(path.display().to_string());
    }
    if write_markdown {
        let path = base.join(format!("{}.md", prefix));
        format::write_markdown(&path, session)?;
        files.push(path.display().to_string());
    }
    if write_html {
        let path = base.join(format!("{}.html", prefix));
        format::write_html(&path, session)?;
        files.push(path.display().to_string());
    }

    Ok(ExportResult { files })
}

pub(crate) fn session_from_compression_archive(
    archive_ref: &str,
    archive: compression::CompressionArchive,
) -> Result<Session> {
    let created_at = archive.events.first().map(|event| event.timestamp);
    let last_active_at = archive.events.last().map(|event| event.timestamp);
    let archive_value = serde_json::to_value(&archive)?;

    Ok(Session {
        schema: Schema::default(),
        identity: Identity {
            canonical_id: archive.canonical_id.clone(),
            source_title: Some(format!("Compression archive {}", archive.canonical_id)),
        },
        provenance: Provenance {
            imported_at: chrono::Utc::now(),
            imported_by: Some("memorph-cli".to_string()),
            primary_source: ProviderRef {
                provider_id: "memorph".to_string(),
                session_id: archive.summary_event_id.clone(),
                source_path: Some(archive_ref.to_string()),
            },
            aliases: vec![ProviderRef {
                provider_id: archive.source_provider_id.clone(),
                session_id: archive.canonical_id.clone(),
                source_path: None,
            }],
        },
        context: Context {
            workspace_dir: None,
            created_at,
            last_active_at,
            tags: vec!["compression-archive".to_string()],
        },
        events: archive.events,
        artifacts: Vec::new(),
        extensions: BTreeMap::from([("compression_archive".to_string(), archive_value)]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::{
        Block, Event, EventKind, Fidelity, ImportedSession, Links, Metadata, Role, Source,
    };
    use crate::provider::{ProviderBackupSupport, ProviderCapabilities, ProviderSessionSummary};
    use crate::storage::{artifact_store::ArtifactQuery, local_store};
    use chrono::Utc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct BackupTestProvider {
        source_path: PathBuf,
        backup_failure_session_id: Option<&'static str>,
        restore_failure: bool,
        restore_supported: bool,
        mutation_count: AtomicUsize,
        restore_count: AtomicUsize,
    }

    impl BackupTestProvider {
        fn new(source_path: PathBuf) -> Self {
            Self {
                source_path,
                backup_failure_session_id: None,
                restore_failure: false,
                restore_supported: true,
                mutation_count: AtomicUsize::new(0),
                restore_count: AtomicUsize::new(0),
            }
        }

        fn failing_backup_for(mut self, session_id: &'static str) -> Self {
            self.backup_failure_session_id = Some(session_id);
            self
        }

        fn failing_restore(mut self) -> Self {
            self.restore_failure = true;
            self
        }

        fn without_restore_support(mut self) -> Self {
            self.restore_supported = false;
            self
        }
    }

    impl provider::Provider for BackupTestProvider {
        fn id(&self) -> &'static str {
            "backup-test"
        }

        fn name(&self) -> &'static str {
            "Backup Test"
        }

        fn capabilities(&self) -> ProviderCapabilities {
            let mut capabilities = ProviderCapabilities::full_session_management();
            capabilities.scan = false;
            capabilities.backup_support = ProviderBackupSupport {
                before_write: self.restore_supported,
                restore: self.restore_supported,
                sync_only: false,
            };
            capabilities
        }

        fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
            Ok(Vec::new())
        }

        fn import_session(&self, _source_path: &str) -> Result<ImportedSession> {
            anyhow::bail!("unused")
        }

        fn delete_session(&self, _session_id: &str) -> Result<()> {
            self.mutation_count.fetch_add(1, Ordering::SeqCst);
            std::fs::remove_file(&self.source_path)?;
            anyhow::bail!("injected provider delete failure")
        }

        fn rename_session(&self, _session_id: &str, _new_title: &str) -> Result<()> {
            self.mutation_count.fetch_add(1, Ordering::SeqCst);
            std::fs::write(&self.source_path, b"partially renamed provider bytes")?;
            anyhow::bail!("injected provider rename failure")
        }

        fn create_session_backup(
            &self,
            mutation: ProviderSourceMutation,
            operation_id: &str,
            session_id: &str,
            backup_root: &Path,
        ) -> Result<ProviderSessionBackup> {
            if self.backup_failure_session_id == Some(session_id) {
                anyhow::bail!("injected backup failure for {session_id}");
            }
            let backup_path = backup_root.join(operation_id);
            std::fs::create_dir_all(&backup_path)?;
            std::fs::copy(&self.source_path, backup_path.join("source"))?;
            std::fs::write(backup_path.join("metadata.json"), b"{}")?;
            Ok(ProviderSessionBackup {
                mutation,
                operation_id: operation_id.to_string(),
                provider_session_id: session_id.to_string(),
                source_path: self.source_path.canonicalize()?,
                backup_path,
                restore_hint: "restore exact source".to_string(),
                mime_type: "application/vnd.memorph.test-backup".to_string(),
                format: "test-backup-v1".to_string(),
                artifact_metadata: serde_json::json!({
                    "role": "test_prewrite_backup",
                    "mutation": mutation,
                }),
                restore_metadata: serde_json::json!({
                    "restore_mode": "test_restore",
                    "mutation": mutation,
                }),
            })
        }

        fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
            self.restore_count.fetch_add(1, Ordering::SeqCst);
            if self.restore_failure {
                anyhow::bail!("injected native restore failure");
            }
            std::fs::copy(backup.backup_path.join("source"), &backup.source_path)?;
            Ok(())
        }
    }

    fn register_test_backup(
        provider: &BackupTestProvider,
        conn: &mut Connection,
        backup_root: &Path,
    ) -> String {
        register_provider_session_backup(
            provider,
            "backup-test",
            ProviderSourceMutation::Delete,
            "operation-1",
            "session-1",
            backup_root,
            conn,
        )
        .unwrap()
        .unwrap();
        ArtifactStore::new(conn)
            .query_backups(BackupQuery {
                operation_id: Some("operation-1".to_string()),
                ..Default::default()
            })
            .unwrap()
            .remove(0)
            .backup
            .id
    }

    #[test]
    fn provider_delete_failure_restores_registered_native_backup() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let backup_root = dir.path().join("backups");
        let provider = BackupTestProvider::new(source_path.clone());
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let results = delete_sessions_with_provider(
            &provider,
            "backup-test",
            &["session-1"],
            &["operation-1".to_string()],
            &backup_root,
            &mut conn,
        );

        assert_eq!(results.len(), 1);
        assert!(format!("{:#}", results[0].as_ref().unwrap_err())
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            std::fs::read(&source_path).unwrap(),
            b"original provider bytes"
        );
        assert_eq!(provider.mutation_count.load(Ordering::SeqCst), 1);
        assert_eq!(provider.restore_count.load(Ordering::SeqCst), 1);

        let manifests = ArtifactStore::new(&mut conn)
            .query(ArtifactQuery {
                operation_id: Some("operation-1".to_string()),
                provider_id: Some("backup-test".to_string()),
                provider_session_id: Some("session-1".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(manifests.len(), 1);
        let record = ArtifactStore::new(&mut conn)
            .find_backup_by_artifact_path(&manifests[0].path)
            .unwrap()
            .unwrap();
        assert_eq!(record.operation_id.as_deref(), Some("operation-1"));
        assert_eq!(record.provider_id.as_deref(), Some("backup-test"));
        assert_eq!(record.provider_session_id.as_deref(), Some("session-1"));
        assert_eq!(
            record.source_path.as_deref(),
            Some(source_path.canonicalize().unwrap().as_path())
        );
        assert_eq!(record.restore_hint.as_deref(), Some("restore exact source"));
        assert!(record.artifact.content_hash.starts_with("sha256-tree-v1:"));
    }

    #[test]
    fn backup_registration_failure_prevents_provider_delete() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let backup_root = dir.path().join("backups");
        let provider = BackupTestProvider::new(source_path.clone());
        let mut conn = Connection::open_in_memory().unwrap();

        let results = delete_sessions_with_provider(
            &provider,
            "backup-test",
            &["session-1"],
            &["operation-1".to_string()],
            &backup_root,
            &mut conn,
        );

        assert_eq!(results.len(), 1);
        assert!(format!("{:#}", results[0].as_ref().unwrap_err())
            .contains("cancelled before provider write"));
        assert_eq!(provider.mutation_count.load(Ordering::SeqCst), 0);
        assert_eq!(provider.restore_count.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read(&source_path).unwrap(),
            b"original provider bytes"
        );
        assert!(backup_root.join("operation-1").exists());
    }

    #[test]
    fn batch_backup_failure_prevents_all_provider_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let backup_root = dir.path().join("backups");
        let provider = BackupTestProvider::new(source_path.clone()).failing_backup_for("session-2");
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let results = delete_sessions_with_provider(
            &provider,
            "backup-test",
            &["session-1", "session-2"],
            &["operation-1".to_string(), "operation-2".to_string()],
            &backup_root,
            &mut conn,
        );

        assert_eq!(results.len(), 2);
        assert!(results
            .iter()
            .all(|result| format!("{:#}", result.as_ref().unwrap_err())
                .contains("cancelled before provider write")));
        assert_eq!(provider.mutation_count.load(Ordering::SeqCst), 0);
        assert_eq!(
            std::fs::read(&source_path).unwrap(),
            b"original provider bytes"
        );
        assert!(backup_root.join("operation-1").exists());
        assert!(!backup_root.join("operation-2").exists());

        let first_manifest = ArtifactStore::new(&mut conn)
            .query(ArtifactQuery {
                operation_id: Some("operation-1".to_string()),
                provider_id: Some("backup-test".to_string()),
                provider_session_id: Some("session-1".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(first_manifest.len(), 1);
    }

    #[test]
    fn provider_rename_failure_restores_registered_native_backup() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let backup_root = dir.path().join("backups");
        let provider = BackupTestProvider::new(source_path.clone());
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let result = rename_session_with_provider(
            &provider,
            "backup-test",
            "session-1",
            "Renamed",
            "operation-1",
            &backup_root,
            &mut conn,
        );

        assert!(format!("{:#}", result.unwrap_err())
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            std::fs::read(&source_path).unwrap(),
            b"original provider bytes"
        );
        assert_eq!(provider.mutation_count.load(Ordering::SeqCst), 1);
        assert_eq!(provider.restore_count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn manual_native_restore_is_recorded_and_repeatable() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let provider = BackupTestProvider::new(source_path.clone());
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let backup_id = register_test_backup(&provider, &mut conn, &dir.path().join("backups"));

        for replacement in [
            b"first replacement".as_slice(),
            b"second replacement".as_slice(),
        ] {
            std::fs::write(&source_path, replacement).unwrap();
            let restored = restore_registered_backup_with(
                &mut conn,
                &backup_id,
                ActivityActor::Cli,
                |conn, backup| {
                    restore_registered_backup_payload(conn, &provider, "backup-test", backup)
                },
            )
            .unwrap();
            assert_eq!(restored.status, BackupRestoreStatus::Success);
            assert_eq!(
                std::fs::read(&source_path).unwrap(),
                b"original provider bytes"
            );
        }

        assert_eq!(provider.restore_count.load(Ordering::SeqCst), 2);
        let entries = ArtifactStore::new(&mut conn)
            .query_backups(BackupQuery {
                restore_status: Some(BackupRestoreStatus::Success),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].latest_restore.as_ref().unwrap().status,
            BackupRestoreStatus::Success
        );
    }

    #[test]
    fn manual_native_restore_failure_is_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let provider = BackupTestProvider::new(source_path).failing_restore();
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let backup_id = register_test_backup(&provider, &mut conn, &dir.path().join("backups"));

        let error = restore_registered_backup_with(
            &mut conn,
            &backup_id,
            ActivityActor::Api,
            |conn, backup| {
                restore_registered_backup_payload(conn, &provider, "backup-test", backup)
            },
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("injected native restore failure"));
        let entry = ArtifactStore::new(&mut conn)
            .get_backup_entry(&backup_id)
            .unwrap()
            .unwrap();
        let restore = entry.latest_restore.unwrap();
        assert_eq!(restore.status, BackupRestoreStatus::Failed);
        assert_eq!(restore.actor, ActivityActor::Api);
        assert!(restore
            .error
            .as_deref()
            .unwrap()
            .contains("injected native restore failure"));
    }

    #[test]
    fn manual_native_restore_rejects_tampered_artifact_before_provider_write() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let provider = BackupTestProvider::new(source_path);
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let backup_id = register_test_backup(&provider, &mut conn, &dir.path().join("backups"));
        let backup = ArtifactStore::new(&mut conn)
            .get_backup(&backup_id)
            .unwrap()
            .unwrap();
        std::fs::write(backup.artifact.path.join("source"), b"tampered").unwrap();

        let error = restore_registered_backup_with(
            &mut conn,
            &backup_id,
            ActivityActor::Cli,
            |conn, backup| {
                restore_registered_backup_payload(conn, &provider, "backup-test", backup)
            },
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("failed integrity verification"));
        assert_eq!(provider.restore_count.load(Ordering::SeqCst), 0);
        assert_eq!(
            ArtifactStore::new(&mut conn)
                .get_backup_entry(&backup_id)
                .unwrap()
                .unwrap()
                .latest_restore
                .unwrap()
                .status,
            BackupRestoreStatus::Failed
        );
    }

    #[test]
    fn manual_native_restore_rejects_provider_identity_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let provider = BackupTestProvider::new(source_path);
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let backup_id = register_test_backup(&provider, &mut conn, &dir.path().join("backups"));
        conn.execute(
            "UPDATE artifact_manifests SET provider_id = 'other' WHERE id = (
                SELECT artifact_id FROM backups WHERE id = ?1
             )",
            [&backup_id],
        )
        .unwrap();

        let error = restore_registered_backup_with(
            &mut conn,
            &backup_id,
            ActivityActor::Cli,
            |conn, backup| {
                restore_registered_backup_payload(conn, &provider, "backup-test", backup)
            },
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("identity does not match"));
        assert_eq!(provider.restore_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn manual_native_restore_rejects_unsupported_provider() {
        let dir = tempfile::tempdir().unwrap();
        let source_path = dir.path().join("session.native");
        std::fs::write(&source_path, b"original provider bytes").unwrap();
        let provider = BackupTestProvider::new(source_path.clone());
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let backup_id = register_test_backup(&provider, &mut conn, &dir.path().join("backups"));
        let unsupported_provider = BackupTestProvider::new(source_path).without_restore_support();

        let error = restore_registered_backup_with(
            &mut conn,
            &backup_id,
            ActivityActor::Cli,
            |conn, backup| {
                restore_registered_backup_payload(
                    conn,
                    &unsupported_provider,
                    "backup-test",
                    backup,
                )
            },
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("does not support manual native session restore"));
        assert_eq!(unsupported_provider.restore_count.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn manual_native_restore_rejects_unknown_backup_without_attempt_record() {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();

        let error = restore_registered_backup_with(
            &mut conn,
            "missing",
            ActivityActor::Cli,
            |_conn, _backup| Ok(()),
        )
        .unwrap_err();

        assert!(format!("{error:#}").contains("Unknown backup: missing"));
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM backup_restores", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn workspace_matches_canonicalizes_equivalent_existing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let session_workspace = dir.path().join(".");
        assert!(workspace_matches(
            "codex",
            session_workspace.to_str(),
            dir.path().to_str(),
        ));
    }

    #[test]
    fn prepare_session_for_target_provider_uses_session_source_provider() {
        let session = sample_opencode_compacted_session();
        let (prepared, report) = prepare_session_for_target_provider(&session, "codex").unwrap();

        assert_eq!(report.normalized_segments, 1);
        assert_eq!(report.target_provider_id, "codex");
        assert!(matches!(
            prepared
                .events
                .first()
                .and_then(|event| event.blocks.first()),
            Some(Block::Compressed { .. })
        ));
    }

    fn sample_opencode_compacted_session() -> Session {
        let now = Utc::now();
        Session {
            schema: Schema::default(),
            identity: Identity {
                canonical_id: "s1".to_string(),
                source_title: None,
            },
            provenance: Provenance {
                imported_at: now,
                imported_by: Some("test".to_string()),
                primary_source: ProviderRef {
                    provider_id: "opencode".to_string(),
                    session_id: "s1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: Context::default(),
            events: vec![
                text_event("old-user", Role::User, "large old context", false),
                compaction_event("compact-marker"),
                text_event("summary", Role::Assistant, "compressed summary", true),
                text_event("tail", Role::User, "new request", false),
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        }
    }

    fn text_event(id: &str, role: Role, text: &str, summary: bool) -> Event {
        let mut provider_ext = BTreeMap::new();
        provider_ext.insert(
            "opencode_message".to_string(),
            serde_json::json!({ "summary": summary }),
        );
        Event {
            id: id.to_string(),
            kind: EventKind::Message,
            role,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::Text {
                text: text.to_string(),
            }],
            metadata: Metadata {
                source: Source {
                    provider_id: "opencode".to_string(),
                    original_id: Some(id.to_string()),
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: Fidelity::Preserved,
                provider_ext,
            },
        }
    }

    fn compaction_event(id: &str) -> Event {
        Event {
            id: id.to_string(),
            kind: EventKind::Other,
            role: Role::User,
            timestamp: Utc::now(),
            links: Links::default(),
            blocks: vec![Block::ProviderPayload {
                kind: "compaction".to_string(),
                payload: serde_json::json!({ "type": "compaction" }),
            }],
            metadata: Metadata {
                source: Source {
                    provider_id: "opencode".to_string(),
                    original_id: Some(id.to_string()),
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: Fidelity::Preserved,
                provider_ext: BTreeMap::new(),
            },
        }
    }
}
