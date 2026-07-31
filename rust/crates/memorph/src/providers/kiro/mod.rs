pub mod adapter;
pub mod hook;
pub mod load;
mod management;

use self::load::*;

use crate::session::{
    Block, Context, Event, EventKind, Fidelity, Identity, ImportedSession, Links, MappingDirection,
    MappingIssue, MappingIssueLevel, MappingReport, Metadata, Provenance, ProviderRef, Role,
    Schema, Session, TurnOutcome,
};
use crate::provider::{
    event_is_visible_message, PageStrategy, Provider, ProviderActivitySupport,
    ProviderBackupSupport, ProviderCapabilities, ProviderContentFidelity, ProviderSessionBackup,
    ProviderSessionImportPage, ProviderSessionSummary, ProviderSourceFingerprint,
    ProviderSourceMutation, ProviderWriteRisk, ScanStrategy, StorageShape, TurnQuality,
    WriteRiskLevel,
};
use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub struct KiroProvider;

const PROVIDER_ID: &str = "kiro";
const CURRENT_SCHEMA_VERSION: &str = "1.0.0";
const CURRENT_DATA_MODEL_VERSION: u64 = 1;

#[cfg(test)]
static TEST_KIRO_SESSIONS_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

impl Provider for KiroProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Kiro"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            import: true,
            delete: true,
            rename: true,
            scan_strategy: ScanStrategy::FullScan,
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Directory,
            turn_quality: TurnQuality::Exact,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::Medium,
                multiple_files: true,
                sqlite: false,
                sidecar_files: true,
                index_repair: false,
            },
            backup_support: ProviderBackupSupport {
                before_write: true,
                restore: true,
                sync_only: false,
            },
            activity_support: ProviderActivitySupport {
                hook_events: true,
                runtime_endpoint: true,
                session_activity: true,
            },
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Preserved),
                tool_result: Some(Fidelity::Preserved),
                patch: Some(Fidelity::Unsupported),
                image: Some(Fidelity::Unsupported),
                file: Some(Fidelity::Unsupported),
                compressed: Some(Fidelity::Unsupported),
                provider_payload: Some(Fidelity::Preserved),
            },
            ..ProviderCapabilities::default()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let sessions_root = kiro_sessions_dir()?;
        if !sessions_root.exists() {
            return Ok(Vec::new());
        }
        scan_sessions_in(&sessions_root)
    }

    fn find_session_by_id(
        &self,
        session_id: &str,
    ) -> Result<Option<ProviderSessionSummary>> {
        let Some(session_dir) = find_session_dir(session_id)? else {
            return Ok(None);
        };
        session_summary_from_dir(&session_dir).map(Some)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        import_canonical_session_from_dir(Path::new(source_path))
    }

    fn import_session_page(
        &self,
        source_path: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<ProviderSessionImportPage> {
        import_kiro_session_page(Path::new(source_path), event_offset, event_limit)
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        kiro_session_source_fingerprint(Path::new(source_path))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let session_dir = management::validate_mutation_source(session_id)?;
        std::fs::remove_dir_all(&session_dir)
            .with_context(|| format!("Failed to delete Kiro session: {}", session_dir.display()))?;
        fail_kiro_mutation_after_write(ProviderSourceMutation::Delete)?;
        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let new_title = new_title.trim();
        if new_title.is_empty() {
            anyhow::bail!("Kiro session title cannot be empty");
        }
        let session_dir = management::validate_mutation_source(session_id)?;
        let metadata_path = session_dir.join("session.json");
        let mut metadata: Value = serde_json::from_slice(&std::fs::read(&metadata_path)?)
            .with_context(|| {
                format!("Failed to parse Kiro metadata: {}", metadata_path.display())
            })?;
        metadata
            .as_object_mut()
            .context("Kiro session metadata must contain a JSON object")?
            .insert("title".to_string(), Value::String(new_title.to_string()));
        let updated = serde_json::to_string_pretty(&metadata)? + "\n";
        crate::storage::atomic_write::write_string_atomic(&metadata_path, &updated)?;
        fail_kiro_mutation_after_write(ProviderSourceMutation::Rename)?;
        Ok(())
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        management::create_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        management::restore_session_backup(backup)
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let Some(session_dir) = find_session_dir(session_id)? else {
            return Ok(0);
        };
        let mut total = 0_u64;
        for entry in WalkDir::new(&session_dir).follow_links(false) {
            let entry = entry.with_context(|| {
                format!("Failed to walk Kiro session: {}", session_dir.display())
            })?;
            if entry.file_type().is_file() {
                total = total.saturating_add(entry.metadata()?.len());
            }
        }
        Ok(total)
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        kiro_sessions_dir().ok().into_iter().collect()
    }
}

#[cfg(test)]
static TEST_KIRO_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

fn fail_kiro_mutation_after_write(mutation: ProviderSourceMutation) -> Result<()> {
    #[cfg(test)]
    {
        let mut configured = TEST_KIRO_MUTATION_FAILURE
            .get_or_init(|| std::sync::Mutex::new(None))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if configured.as_ref() == Some(&mutation) {
            *configured = None;
            anyhow::bail!("injected Kiro mutation failure after provider write");
        }
    }
    let _ = mutation;
    Ok(())
}

#[cfg(test)]
fn set_test_kiro_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_KIRO_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = mutation;
}

#[derive(Debug, Deserialize)]
struct KiroSessionMetadata {
    #[serde(rename = "schemaVersion")]
    schema_version: String,
    #[serde(rename = "dataModelVersion")]
    data_model_version: u64,
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(rename = "workspacePaths", default)]
    workspace_paths: Vec<String>,
    #[serde(rename = "createdAt", default)]
    created_at: Option<String>,
    #[serde(rename = "lastModifiedAt", default)]
    last_modified_at: Option<String>,
}

#[cfg(test)]
mod tests;
