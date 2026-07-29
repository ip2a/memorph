pub mod adapter;
pub mod hook;
pub mod load;
pub mod write;

use self::load::*;
use self::write::*;

use crate::session::{
    Block, Context, Event, EventKind, ExportedSession, Fidelity, Identity, ImportedSession, Links,
    MappingDirection, MappingIssue, MappingIssueLevel, MappingReport, Metadata, Provenance,
    ProviderRef, Role, Schema, Session, Source, TurnOutcome,
};
use crate::provider::{
    canonical_event_is_visible_message, canonical_event_visible_message_role,
    canonical_event_visible_message_text, canonical_export_result, canonical_session_title,
    canonical_visible_block_text, PageStrategy, Provider, ProviderActivitySupport,
    ProviderBackupSupport, ProviderCapabilities, ProviderContentFidelity, ProviderSessionBackup,
    ProviderSessionImportPage, ProviderSessionSummary, ProviderSourceFingerprint,
    ProviderSourceMutation, ProviderWriteRisk, ResumeQuality, ScanStrategy, StorageShape,
    TurnQuality, WriteRiskLevel,
};
use anyhow::{Context as _, Result};
use chrono::Utc;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

mod backup;

pub struct KimiProvider;

const PROVIDER_ID: &str = "kimi";
const TITLE_MAX_CHARS: usize = 80;

#[cfg(test)]
static TEST_KIMI_SESSIONS_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_KIMI_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

impl Provider for KimiProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "Kimi"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan_strategy: ScanStrategy::Hybrid,
            page_strategy: PageStrategy::FullImport,
            storage_shape: StorageShape::Directory,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Downgraded),
                tool_result: Some(Fidelity::Downgraded),
                patch: Some(Fidelity::Unsupported),
                image: Some(Fidelity::Normalized),
                file: Some(Fidelity::Downgraded),
                compressed: Some(Fidelity::Unsupported),
                provider_payload: Some(Fidelity::Preserved),
            },
            export_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Downgraded),
                tool_result: Some(Fidelity::Downgraded),
                patch: Some(Fidelity::Downgraded),
                image: Some(Fidelity::Downgraded),
                file: Some(Fidelity::Downgraded),
                compressed: Some(Fidelity::Downgraded),
                provider_payload: Some(Fidelity::Dropped),
            },
            resume_quality: ResumeQuality::Native,
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
            ..ProviderCapabilities::full_session_management()
        }
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let root = get_kimi_sessions_dir();
        if !root.exists() {
            return Ok(Vec::new());
        }

        let work_dirs = load_work_dir_map()?;
        let mut seen_session_ids = BTreeMap::new();
        let mut sessions = Vec::new();
        let mut work_dir_paths = std::fs::read_dir(&root)
            .with_context(|| format!("Failed to read Kimi sessions: {}", root.display()))?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        work_dir_paths.sort();

        for sessions_dir in work_dir_paths {
            let work_dir_key = sessions_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let project_dir = work_dirs
                .get(work_dir_key)
                .map(|work_dir| work_dir.project_dir.clone());
            let entries = std::fs::read_dir(&sessions_dir).with_context(|| {
                format!(
                    "Failed to read Kimi work-dir sessions: {}",
                    sessions_dir.display()
                )
            })?;
            let mut session_dirs = entries
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|path| path.is_dir() && path.join("context.jsonl").is_file())
                .collect::<Vec<_>>();
            session_dirs.sort();

            for session_dir in session_dirs {
                let Some(session_id) = session_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .filter(|session_id| !session_id.is_empty())
                    .map(str::to_string)
                else {
                    continue;
                };
                if let Some(previous) =
                    seen_session_ids.insert(session_id.clone(), session_dir.clone())
                {
                    anyhow::bail!(
                        "Ambiguous Kimi session id {session_id}: {} and {}",
                        previous.display(),
                        session_dir.display()
                    );
                }
                if let Some(summary) =
                    kimi_session_summary(&session_dir, session_id, project_dir.clone())?
                {
                    sessions.push(summary);
                }
            }
        }

        sessions.sort_by(|left, right| {
            right
                .last_active_at
                .cmp(&left.last_active_at)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
        Ok(sessions)
    }

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        let Some(session_dir) = find_session_dir(session_id)? else {
            return Ok(None);
        };
        if !session_dir.join("context.jsonl").is_file() {
            return Ok(None);
        }

        let work_dir_key = session_dir
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str());
        let work_dirs = load_work_dir_map()?;
        let project_dir = work_dir_key
            .and_then(|key| work_dirs.get(key))
            .map(|work_dir| work_dir.project_dir.clone());
        kimi_session_summary(&session_dir, session_id.to_string(), project_dir)
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
        import_kimi_session_page(Path::new(source_path), event_offset, event_limit)
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        kimi_session_source_fingerprint(Path::new(source_path))
    }

    fn export_session(&self, session: &Session, target_dir: &Path) -> Result<ExportedSession> {
        let session_id = export_canonical_session(session, target_dir)?;
        Ok(canonical_export_result(
            PROVIDER_ID,
            session_id.clone(),
            self.resume_command(&session_id),
            session,
            self.capabilities(),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        let dir = backup::validate_mutation_source(ProviderSourceMutation::Delete, session_id)?;
        std::fs::remove_dir_all(&dir)
            .with_context(|| format!("Failed to delete Kimi session dir: {}", dir.display()))?;
        fail_kimi_mutation_after_write(ProviderSourceMutation::Delete)?;
        Ok(())
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let dir = backup::validate_mutation_source(ProviderSourceMutation::Rename, session_id)?;
        let state_path = dir.join("state.json");

        let raw = std::fs::read_to_string(&state_path)?;
        let mut state: Value = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse state.json: {}", state_path.display()))?;

        if let Some(obj) = state.as_object_mut() {
            obj.insert(
                "custom_title".to_string(),
                Value::String(new_title.to_string()),
            );
        }

        let updated = serde_json::to_vec_pretty(&state)?;
        write_kimi_state_atomically(&state_path, &updated)?;
        fail_kimi_mutation_after_write(ProviderSourceMutation::Rename)?;
        Ok(())
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        backup::create_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        backup::restore_session_backup(backup)
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("kimi --resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        let dir = find_session_dir(session_id)?
            .with_context(|| format!("Kimi session not found: {}", session_id))?;
        let mut total: u64 = 0;
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total += meta.len();
                }
            }
        }
        Ok(total)
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        vec![get_kimi_sessions_dir()]
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
