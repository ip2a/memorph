pub mod adapter;
pub mod backup;
pub mod hook;
pub mod load;
pub mod management;
pub mod write;
use self::backup::*;
use self::load::*;
use self::management::*;
use self::write::*;

use crate::core::compression::{self, CompressedSegment};
use crate::provider::{
    compression_retrieval_hint, event_is_visible_message, event_visible_message_role,
    export_result, session_title, visible_block_text, CompressionProjection, PageStrategy,
    Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionBackup, ProviderSessionImportPage,
    ProviderSessionSummary, ProviderSourceFingerprint, ProviderSourceMutation, ProviderWriteRisk,
    ResumeQuality, ScanStrategy, StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::session::{
    Block, Context, Event, EventKind, ExportedSession, Fidelity, Identity, ImportedSession, Links,
    MappingDirection, MappingIssue, MappingIssueLevel, MappingReport, Metadata, Provenance,
    ProviderRef, Role, Schema, Session, TurnOutcome, Usage,
};
use crate::session_projection::project_session_turns;
use anyhow::{Context as _, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct OpenCodeProvider;

const PROVIDER_ID: &str = "opencode";
const OPENCODE_VERSION: &str = "1.3.17";
const OPENCODE_BACKUP_FORMAT: &str = "opencode-session-backup-v1";
const OPENCODE_BACKUP_MIME: &str = "application/vnd.memorph.opencode-session-backup";
const OPENCODE_BACKUP_DB_PATH: &str = "sqlite/opencode-session.db";

#[cfg(test)]
static TEST_OPENCODE_STATE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_OPENCODE_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_OPENCODE_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    opencode_dir: PathBuf,
    db_path: PathBuf,
    database_present: bool,
    sqlite_tables: Vec<OpenCodeSqliteTableManifest>,
    filesystem_entries: Vec<OpenCodeFilesystemEntryBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeSqliteTableManifest {
    table: String,
    columns: Vec<String>,
    row_count: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum OpenCodeFilesystemEntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OpenCodeFilesystemEntryBackup {
    source_path: PathBuf,
    relative_path: PathBuf,
    kind: OpenCodeFilesystemEntryKind,
    present: bool,
}

#[derive(Debug, Clone)]
struct OpenCodeForeignKey {
    child_table: String,
    child_columns: Vec<String>,
    parent_table: String,
    parent_columns: Vec<Option<String>>,
    on_delete: String,
}

#[derive(Debug, Clone)]
struct OpenCodeMutationPaths {
    session_files: Vec<PathBuf>,
    message_dir: PathBuf,
    part_dirs: Vec<PathBuf>,
}

impl Provider for OpenCodeProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "OpenCode"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: true,
            lightweight_scan: false,
            single_session_lookup: false,
            scan_strategy: ScanStrategy::Hybrid,
            page_strategy: PageStrategy::NativePage,
            storage_shape: StorageShape::Mixed,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Preserved),
                tool_call: Some(Fidelity::Preserved),
                tool_result: Some(Fidelity::Preserved),
                patch: Some(Fidelity::Preserved),
                image: Some(Fidelity::Normalized),
                file: Some(Fidelity::Normalized),
                compressed: Some(Fidelity::Downgraded),
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
                compressed: Some(Fidelity::Preserved),
                provider_payload: Some(Fidelity::Dropped),
            },
            resume_quality: ResumeQuality::Native,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: true,
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
        }
    }

    fn detects_native_compression_source(&self) -> bool {
        true
    }

    fn compression_projection(&self) -> CompressionProjection {
        CompressionProjection::Native
    }

    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>> {
        let mut sessions = BTreeMap::new();

        // OpenCode exposes two native source planes. A session id collision is
        // represented once using the database locator, which is stable across
        // filesystem layout changes.
        for session in scan_sessions_from_db()? {
            sessions.insert(session.session_id.clone(), session);
        }
        for session in scan_sessions_from_filesystem()? {
            sessions
                .entry(session.session_id.clone())
                .or_insert(session);
        }

        Ok(sessions.into_values().collect())
    }

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        Ok(self
            .scan_sessions()?
            .into_iter()
            .find(|session| session.session_id == session_id))
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let session_id = opencode_session_id_from_source_locator(source_path)?;
        let mut imported = import_canonical_session_from_source(&session_id, source_path)?;
        imported.provenance.primary_source.source_path = Some(source_path.to_string());
        Ok(imported)
    }

    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        opencode_session_source_fingerprint(source_path)
    }

    fn import_session_page(
        &self,
        source_path: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<ProviderSessionImportPage> {
        import_opencode_session_page(source_path, event_offset, event_limit)
    }

    fn supports_native_session_replace(&self) -> bool {
        true
    }

    fn replace_session(&self, session_id: &str, session: &Session) -> Result<()> {
        replace_opencode_session(session_id, session)
    }

    fn export_session(&self, session: &Session, target_dir: &Path) -> Result<ExportedSession> {
        let session_id = export_canonical_session(session, target_dir)?;
        Ok(export_result(
            PROVIDER_ID,
            session_id.clone(),
            self.resume_command(&session_id),
            session,
            self.capabilities(),
        ))
    }

    fn delete_session(&self, session_id: &str) -> Result<()> {
        delete_opencode_session(session_id)
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        rename_opencode_session(session_id, new_title)
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        create_opencode_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        restore_opencode_session_backup(backup)
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("opencode --session {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        if let Some(total) = opencode_session_files_size(session_id)? {
            return Ok(total);
        }

        if let Ok(size) = opencode_session_db_size(session_id) {
            return Ok(size);
        }

        Ok(0)
    }

    fn session_sizes(&self, session_ids: &[&str]) -> HashMap<String, u64> {
        let mut sizes = HashMap::new();
        let mut missing_db = Vec::new();
        for session_id in session_ids {
            match opencode_session_files_size(session_id) {
                Ok(Some(size)) if size > 0 => {
                    sizes.insert((*session_id).to_string(), size);
                }
                _ => missing_db.push(*session_id),
            }
        }

        if missing_db.is_empty() {
            return sizes;
        }

        let db_path = get_db_path();
        let Ok(conn) = Connection::open(&db_path) else {
            return sizes;
        };
        for session_id in missing_db {
            if let Ok(size) = opencode_session_db_size_with_conn(&conn, session_id) {
                if size > 0 {
                    sizes.insert(session_id.to_string(), size);
                }
            }
        }
        sizes
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        vec![get_db_path(), get_opencode_dir()]
    }
}

fn get_opencode_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_OPENCODE_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test opencode dir lock")
        .clone()
    {
        return path;
    }

    // OpenCode uses ~/.local/share/opencode even on macOS
    dirs::home_dir()
        .map(|h| h.join(".local/share/opencode"))
        .unwrap_or_else(|| PathBuf::from(".local/share/opencode"))
}

#[cfg(test)]
pub(crate) fn lock_test_opencode_state() -> std::sync::MutexGuard<'static, ()> {
    TEST_OPENCODE_STATE_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .expect("test opencode state lock")
}

#[cfg(test)]
pub(crate) fn set_test_opencode_dir(path: Option<PathBuf>) {
    *TEST_OPENCODE_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test opencode dir lock") = path;
}

#[cfg(test)]
fn set_test_opencode_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_OPENCODE_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test opencode mutation failure lock") = mutation;
}

#[cfg(test)]
fn fail_opencode_mutation_after_database_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_OPENCODE_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test opencode mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected OpenCode mutation failure after database write");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_opencode_mutation_after_database_write(_mutation: ProviderSourceMutation) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
