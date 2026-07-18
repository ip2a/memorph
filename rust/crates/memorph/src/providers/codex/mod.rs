pub mod adapter;
pub mod hook;

use crate::canonical::{
    CanonicalSchema, CanonicalSession, EventBlock, EventLinks, EventMetadata, EventRole,
    EventSource, ExportedSession, ImportedSession, MappingDirection, MappingDisposition,
    MappingIssue, MappingIssueLevel, MappingReport, ProviderSessionRef, SessionContext,
    SessionEvent, SessionEventKind, SessionIdentity, SessionProvenance, TurnBoundary,
};
use crate::core::compression::{self, CompressedSegment};
use crate::provider::{
    canonical_event_visible_message_role, canonical_event_visible_message_text,
    canonical_event_visible_text, canonical_export_result,
    canonical_session_instruction_context_text, canonical_session_title,
    canonical_visible_block_text, compression_retrieval_hint, CompressionProjection, PageStrategy,
    Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionBackup, ProviderSessionImportPage,
    ProviderSessionSummary, ProviderSourceMutation, ProviderWriteRisk, ResumeQuality, ScanStrategy,
    StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::storage::{
    activity_store::{
        ActivityActor, ActivityCompletion, ActivityOperationKind, ActivityStore, NewActivity,
    },
    artifact_store::{ArtifactStore, BackupRecord, NewBackupRecord},
    event_index, local_store, session_state,
};
use crate::utils;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;
use walkdir::WalkDir;

pub struct CodexProvider;

const PROVIDER_ID: &str = "codex";
const CODEX_SESSION_BACKUP_FORMAT: &str = "codex-session-backup-v1";
const CODEX_SESSION_BACKUP_MIME: &str = "application/vnd.memorph.codex-session-backup";
const CODEX_SESSION_BACKUP_DB_PATH: &str = "sqlite/codex-session.db";
const CODEX_SYNC_BACKUP_NAMESPACE: &str = "provider-sync";
const DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT: usize = 5;
const CODEX_SYNC_SESSION_DIRS: &[&str] = &["sessions", "archived_sessions"];
const CODEX_SQLITE_FILE_BASENAME: &str = "state_5.sqlite";
const CODEX_GLOBAL_STATE_FILE_BASENAME: &str = ".codex-global-state.json";
const CODEX_GLOBAL_STATE_BACKUP_FILE_BASENAME: &str = ".codex-global-state.json.bak";
const CODEX_INTERNAL_DEVELOPER_TAGS: &[&str] = &[
    "<model_switch>",
    "<collaboration_mode>",
    "<permissions instructions>",
    "<skills_instructions>",
    "<personality_spec>",
];
const CODEX_INTERNAL_USER_CONTEXT_TAGS: &[(&str, &str)] = &[
    ("<environment_context>", "</environment_context>"),
    ("<codex_internal_context", "</codex_internal_context>"),
];

#[cfg(test)]
static TEST_CODEX_DIR: std::sync::OnceLock<std::sync::Mutex<Option<PathBuf>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static TEST_CODEX_MUTATION_FAILURE: std::sync::OnceLock<
    std::sync::Mutex<Option<ProviderSourceMutation>>,
> = std::sync::OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexSessionBackupMetadata {
    version: u32,
    provider_id: String,
    mutation: ProviderSourceMutation,
    operation_id: String,
    provider_session_id: String,
    codex_home: PathBuf,
    db_path: PathBuf,
    database_present: bool,
    session_index: CodexFileBackup,
    rollout: CodexFileBackup,
    sqlite_tables: Vec<CodexSqliteTableManifest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexFileBackup {
    source_path: Option<PathBuf>,
    relative_path: PathBuf,
    present: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum CodexSqliteRestoreMode {
    FullRows,
    AssignedThread,
    ThreadTitle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodexSqliteTableManifest {
    table: String,
    columns: Vec<String>,
    row_count: usize,
    restore_mode: CodexSqliteRestoreMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexInternalMessageKind {
    LifecycleSentinel,
    RuntimeContext,
    ProviderControl,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CodexTurnLink {
    provider_turn_id: Option<String>,
    turn_index: Option<u32>,
    turn_boundary: Option<TurnBoundary>,
}

impl CodexTurnLink {
    fn apply_to(self, event: &mut SessionEvent) {
        event.links.provider_turn_id = self.provider_turn_id;
        event.links.turn_index = self.turn_index;
        event.links.turn_boundary = self.turn_boundary;
    }
}

#[derive(Debug, Default)]
struct CodexTurnTracker {
    active_turn_id: Option<String>,
    turn_indices: HashMap<String, u32>,
    next_turn_index: u32,
}

impl CodexTurnTracker {
    fn observe_line(&mut self, line: &Value) -> CodexTurnLink {
        let line_type = line.get("type").and_then(Value::as_str);
        let payload = line.get("payload");
        let payload_type = payload
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str);
        let boundary = match (line_type, payload_type) {
            (Some("event_msg"), Some("task_started")) => Some(TurnBoundary::Started),
            (Some("event_msg"), Some("task_complete")) => Some(TurnBoundary::Completed),
            (Some("event_msg"), Some("turn_aborted")) => Some(TurnBoundary::Interrupted),
            _ => None,
        };
        let explicit_turn_id = payload
            .and_then(|value| value.get("turn_id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        if let Some(turn_id) = explicit_turn_id.as_ref() {
            self.active_turn_id = Some(turn_id.clone());
        }
        let provider_turn_id = explicit_turn_id.or_else(|| self.active_turn_id.clone());
        let turn_index = provider_turn_id
            .as_ref()
            .map(|turn_id| self.turn_index(turn_id));
        let closes_turn = matches!(
            boundary,
            Some(TurnBoundary::Completed | TurnBoundary::Failed | TurnBoundary::Interrupted)
        );
        if closes_turn && self.active_turn_id.as_ref() == provider_turn_id.as_ref() {
            self.active_turn_id = None;
        }

        CodexTurnLink {
            provider_turn_id,
            turn_index,
            turn_boundary: boundary,
        }
    }

    fn turn_index(&mut self, turn_id: &str) -> u32 {
        if let Some(index) = self.turn_indices.get(turn_id) {
            return *index;
        }
        let index = self.next_turn_index;
        self.next_turn_index = self.next_turn_index.saturating_add(1);
        self.turn_indices.insert(turn_id.to_string(), index);
        index
    }
}

impl CodexInternalMessageKind {
    fn class(self) -> &'static str {
        match self {
            Self::LifecycleSentinel => "lifecycle_sentinel",
            Self::RuntimeContext => "runtime_context",
            Self::ProviderControl => "provider_control",
        }
    }

    fn payload_kind(self) -> &'static str {
        match self {
            Self::LifecycleSentinel => "turn_aborted_sentinel",
            Self::RuntimeContext => "user_context_message",
            Self::ProviderControl => "developer_control_message",
        }
    }

    fn issue_code(self) -> &'static str {
        match self {
            Self::LifecycleSentinel => "codex_turn_aborted_sentinel_hidden",
            Self::RuntimeContext => "codex_internal_user_context_hidden",
            Self::ProviderControl => "codex_internal_developer_message_hidden",
        }
    }

    fn issue_message(self) -> &'static str {
        match self {
            Self::LifecycleSentinel => {
                "Codex synthetic <turn_aborted> message was normalized into a hidden lifecycle event"
            }
            Self::RuntimeContext => {
                "Codex runtime-injected user context was normalized into a hidden lifecycle event"
            }
            Self::ProviderControl => {
                "Codex internal developer control message was normalized into a hidden lifecycle event"
            }
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodexWorkspaceRepairItem {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub rollout_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_model_provider: Option<String>,
    pub current_model_provider: String,
    pub updated_model_provider: bool,
    pub added_to_index: bool,
    pub updated_index_title: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct CodexWorkspaceRepairReport {
    pub workspace_dir: String,
    pub current_model_provider: String,
    pub scanned_rollouts: usize,
    pub workspace_session_count: usize,
    pub hidden_session_count: usize,
    pub repaired_session_count: usize,
    pub reindexed_session_count: usize,
    #[serde(default)]
    pub retitled_session_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_artifact_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_id: Option<String>,
    pub sqlite_rows_updated: usize,
    pub sqlite_provider_rows_updated: usize,
    pub sqlite_user_event_rows_updated: usize,
    pub sqlite_cwd_rows_updated: usize,
    pub pruned_backup_count: usize,
    #[serde(default)]
    pub skipped_rollout_files: Vec<String>,
    pub touched_sessions: Vec<CodexWorkspaceRepairItem>,
}

#[derive(Debug, Clone, Default)]
struct CodexWorkspaceSqliteStats {
    rows_updated: usize,
    provider_rows_updated: usize,
    user_event_rows_updated: usize,
    cwd_rows_updated: usize,
}

#[derive(Debug, Clone)]
struct CodexWorkspaceSyncCandidate {
    rollout_path: PathBuf,
    session: CodexRolloutSummary,
}

#[derive(Debug, Clone)]
struct CodexSessionFileMeta {
    path: PathBuf,
    size_bytes: u64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct CodexSyncBackupMetadata {
    version: u8,
    namespace: String,
    operation_id: String,
    codex_home: String,
    workspace_dir: String,
    target_provider: String,
    created_at: String,
    session_index_present: bool,
    session_files: Vec<String>,
    db_files: Vec<String>,
    global_state_files: Vec<String>,
}

impl Provider for CodexProvider {
    fn id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn name(&self) -> &'static str {
        "codex"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: true,
            scan_strategy: ScanStrategy::Indexed,
            page_strategy: PageStrategy::IndexedPage,
            storage_shape: StorageShape::Mixed,
            turn_quality: TurnQuality::Inferred,
            import_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Normalized),
                tool_call: Some(MappingDisposition::Preserved),
                tool_result: Some(MappingDisposition::Preserved),
                patch: Some(MappingDisposition::Unsupported),
                image: Some(MappingDisposition::Preserved),
                file: Some(MappingDisposition::Unsupported),
                compressed: Some(MappingDisposition::Normalized),
                provider_payload: Some(MappingDisposition::Preserved),
            },
            export_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Downgraded),
                tool_call: Some(MappingDisposition::Downgraded),
                tool_result: Some(MappingDisposition::Downgraded),
                patch: Some(MappingDisposition::Downgraded),
                image: Some(MappingDisposition::Normalized),
                file: Some(MappingDisposition::Downgraded),
                compressed: Some(MappingDisposition::Normalized),
                provider_payload: Some(MappingDisposition::Dropped),
            },
            resume_quality: ResumeQuality::Native,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::High,
                multiple_files: true,
                sqlite: true,
                sidecar_files: false,
                index_repair: true,
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
        let codex_dir = get_codex_dir();
        let index_path = codex_dir.join("session_index.jsonl");
        if !index_path.exists() {
            return Ok(Vec::new());
        }

        // Build a lookup from SQLite for fast access
        let sqlite_lookup = build_sqlite_thread_metadata_lookup(&codex_dir).unwrap_or_default();

        let file = File::open(&index_path).with_context(|| {
            format!(
                "Failed to open Codex session index: {}",
                index_path.display()
            )
        })?;
        let reader = BufReader::new(file);
        let mut index_entries = Vec::new();

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let id = value
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let thread_name = value
                .get("thread_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let updated_at = value
                .get("updated_at")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis());

            if let Some(id) = id {
                index_entries.push((id, thread_name, updated_at));
            }
        }

        let missing_file_ids: Vec<String> = index_entries
            .iter()
            .filter(|(id, _, _)| {
                sqlite_lookup
                    .get(id)
                    .and_then(|meta| meta.rollout_path.as_deref())
                    .is_none()
            })
            .map(|(id, _, _)| id.clone())
            .collect();
        let file_lookup = build_session_file_lookup(&codex_dir, &missing_file_ids);
        let mut sessions = Vec::with_capacity(index_entries.len());

        for (id, thread_name, updated_at) in index_entries {
            let file_meta = file_lookup.get(&id);
            let sqlite_meta = sqlite_lookup.get(&id);
            let project_dir = sqlite_meta
                .and_then(|meta| clean_non_empty(meta.cwd.as_deref()))
                .map(str::to_string)
                .or_else(|| file_meta.and_then(|meta| extract_cwd_from_session_path(&meta.path)));
            let title = select_codex_display_title(
                thread_name.as_deref(),
                sqlite_meta.and_then(|meta| meta.title.as_deref()),
                None,
                &id,
            );

            sessions.push(ProviderSessionSummary {
                session_id: id,
                title,
                project_dir,
                last_active_at: updated_at,
                source_path: sqlite_meta
                    .and_then(|meta| meta.rollout_path.clone())
                    .or_else(|| file_meta.map(|meta| meta.path.to_string_lossy().to_string())),
            });
        }

        Ok(sessions)
    }

    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        let index_path = get_codex_dir().join("session_index.jsonl");
        if !index_path.exists() {
            return Ok(None);
        }

        let sqlite_lookup =
            build_sqlite_thread_metadata_lookup(&get_codex_dir()).unwrap_or_default();

        let file = File::open(&index_path).with_context(|| {
            format!(
                "Failed to open Codex session index: {}",
                index_path.display()
            )
        })?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let id = value.get("id").and_then(|v| v.as_str());
            if id != Some(session_id) {
                continue;
            }

            let thread_name = value
                .get("thread_name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let updated_at = value
                .get("updated_at")
                .and_then(|v| v.as_str())
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.timestamp_millis());

            let source_path = find_session_file(session_id);
            let sqlite_meta = sqlite_lookup.get(session_id);
            let project_dir = sqlite_meta
                .and_then(|meta| clean_non_empty(meta.cwd.as_deref()))
                .map(str::to_string)
                .or_else(|| extract_cwd_from_session_file(session_id));
            let title = select_codex_display_title(
                thread_name.as_deref(),
                sqlite_meta.and_then(|meta| meta.title.as_deref()),
                None,
                session_id,
            );

            return Ok(Some(ProviderSessionSummary {
                session_id: session_id.to_string(),
                title,
                project_dir,
                last_active_at: updated_at,
                source_path: source_path.map(|p| p.to_string_lossy().to_string()),
            }));
        }

        Ok(None)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let source_path = Path::new(source_path);
        let mut imported = import_canonical_session(source_path)?;
        imported.session.identity.source_title = resolve_codex_projection_title(
            source_path,
            &imported.session.identity.canonical_id,
            imported.session.identity.source_title.as_deref(),
        )?;
        Ok(imported)
    }

    fn import_session_page(
        &self,
        source_path: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<ProviderSessionImportPage> {
        let source_path = Path::new(source_path);
        let mut page = import_canonical_session_page(source_path, event_offset, event_limit)?;
        page.imported.session.identity.source_title = resolve_codex_projection_title(
            source_path,
            &page.imported.session.identity.canonical_id,
            page.imported.session.identity.source_title.as_deref(),
        )?;
        Ok(page)
    }

    fn supports_native_session_replace(&self) -> bool {
        true
    }

    fn replace_session(&self, session_id: &str, session: &CanonicalSession) -> Result<()> {
        replace_codex_session(session_id, session)
    }

    fn export_session(
        &self,
        session: &CanonicalSession,
        target_dir: &Path,
    ) -> Result<ExportedSession> {
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
        delete_codex_session(session_id)
    }

    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        rename_codex_session(session_id, new_title)
    }

    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        create_codex_session_backup(mutation, operation_id, session_id, backup_root)
    }

    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        restore_codex_session_backup(backup)
    }

    fn resume_command(&self, session_id: &str) -> Option<String> {
        Some(format!("codex resume {}", session_id))
    }

    fn session_size(&self, session_id: &str) -> Result<u64> {
        if let Some(path) = find_session_file(session_id) {
            if path.exists() {
                return Ok(std::fs::metadata(path)?.len());
            }
        }
        Ok(0)
    }

    fn session_sizes(&self, session_ids: &[&str]) -> HashMap<String, u64> {
        let sqlite_lookup =
            build_sqlite_thread_metadata_lookup(&get_codex_dir()).unwrap_or_default();
        let mut sizes = HashMap::new();
        let mut missing_ids = Vec::new();

        for session_id in session_ids {
            let Some(path) = sqlite_lookup
                .get(*session_id)
                .and_then(|meta| meta.rollout_path.as_deref())
            else {
                missing_ids.push((*session_id).to_string());
                continue;
            };
            match std::fs::metadata(path) {
                Ok(metadata) if metadata.is_file() && metadata.len() > 0 => {
                    sizes.insert((*session_id).to_string(), metadata.len());
                }
                _ => missing_ids.push((*session_id).to_string()),
            }
        }

        if missing_ids.is_empty() {
            return sizes;
        }

        let ids: Vec<String> = session_ids.iter().map(|id| (*id).to_string()).collect();
        build_session_file_lookup(&get_codex_dir(), &ids)
            .into_iter()
            .filter(|(id, _)| missing_ids.contains(id))
            .filter_map(|(id, meta)| (meta.size_bytes > 0).then_some((id, meta.size_bytes)))
            .for_each(|(id, size)| {
                sizes.insert(id, size);
            });
        sizes
    }

    fn data_source_paths(&self) -> Vec<PathBuf> {
        vec![get_codex_dir()]
    }
}

fn create_codex_session_backup(
    mutation: ProviderSourceMutation,
    operation_id: &str,
    session_id: &str,
    backup_root: &Path,
) -> Result<ProviderSessionBackup> {
    let codex_dir = get_codex_dir();
    let source_path = codex_dir.canonicalize().with_context(|| {
        format!(
            "Failed to resolve Codex data directory: {}",
            codex_dir.display()
        )
    })?;
    let index_path = source_path.join("session_index.jsonl");
    let rollout_path = find_session_file(session_id)
        .map(|path| path.canonicalize())
        .transpose()
        .with_context(|| format!("Failed to resolve Codex rollout for session {session_id}"))?;
    if rollout_path
        .as_deref()
        .is_some_and(|path| !path.starts_with(&source_path))
    {
        anyhow::bail!("Codex rollout is outside the Codex data directory");
    }

    match mutation {
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
            if rollout_path.is_none() =>
        {
            anyhow::bail!("Codex session not found: {session_id}");
        }
        ProviderSourceMutation::Rename => {
            if !index_path.exists() {
                anyhow::bail!("Codex session index not found");
            }
            if !codex_index_contains_session(&index_path, session_id)? {
                anyhow::bail!("Codex session not found in index: {session_id}");
            }
        }
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {}
    }

    let provider_backup_root = backup_root.join(PROVIDER_ID);
    std::fs::create_dir_all(&provider_backup_root).with_context(|| {
        format!(
            "Failed to create Codex backup root: {}",
            provider_backup_root.display()
        )
    })?;
    let backup_path = provider_backup_root.join(operation_id);
    std::fs::create_dir(&backup_path).with_context(|| {
        format!(
            "Failed to create Codex session backup: {}",
            backup_path.display()
        )
    })?;

    let session_index = capture_codex_file(
        Some(index_path.clone()),
        PathBuf::from("session_index.jsonl"),
        &backup_path,
    )?;
    let rollout = capture_codex_file(
        rollout_path.clone(),
        PathBuf::from("rollout").join("session.jsonl"),
        &backup_path,
    )?;

    let db_path = source_path.join(CODEX_SQLITE_FILE_BASENAME);
    let database_present = db_path.exists();
    let sqlite_tables = if database_present {
        std::fs::create_dir(backup_path.join("sqlite"))?;
        let mut conn = Connection::open(&db_path)
            .with_context(|| format!("Failed to open Codex SQLite: {}", db_path.display()))?;
        capture_codex_sqlite_backup(
            &mut conn,
            mutation,
            session_id,
            &backup_path.join(CODEX_SESSION_BACKUP_DB_PATH),
        )?
    } else {
        Vec::new()
    };

    let metadata = CodexSessionBackupMetadata {
        version: 1,
        provider_id: PROVIDER_ID.to_string(),
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        codex_home: source_path.clone(),
        db_path,
        database_present,
        session_index,
        rollout,
        sqlite_tables,
    };
    std::fs::write(
        backup_path.join("metadata.json"),
        serde_json::to_vec_pretty(&metadata)?,
    )
    .with_context(|| {
        format!(
            "Failed to write Codex backup metadata: {}",
            backup_path.display()
        )
    })?;

    Ok(ProviderSessionBackup {
        mutation,
        operation_id: operation_id.to_string(),
        provider_session_id: session_id.to_string(),
        source_path,
        backup_path,
        restore_hint:
            "Restore this backup with memorph's Codex native session restore flow before reopening Codex."
                .to_string(),
        mime_type: CODEX_SESSION_BACKUP_MIME.to_string(),
        format: CODEX_SESSION_BACKUP_FORMAT.to_string(),
        artifact_metadata: serde_json::json!({
            "role": "codex_prewrite_session_backup",
            "mutation": mutation,
            "sqlite_table_count": metadata.sqlite_tables.len(),
            "session_index_present": metadata.session_index.present,
            "rollout_present": metadata.rollout.present,
        }),
        restore_metadata: serde_json::json!({
            "restore_mode": "codex_session_restore",
            "metadata_file": "metadata.json",
            "mutation": mutation,
        }),
    })
}

fn restore_codex_session_backup(backup: &ProviderSessionBackup) -> Result<()> {
    if backup.format != CODEX_SESSION_BACKUP_FORMAT {
        anyhow::bail!("Unsupported Codex session backup format: {}", backup.format);
    }
    if backup.mime_type != CODEX_SESSION_BACKUP_MIME {
        anyhow::bail!(
            "Unsupported Codex session backup MIME type: {}",
            backup.mime_type
        );
    }

    let metadata_path = backup.backup_path.join("metadata.json");
    let metadata: CodexSessionBackupMetadata =
        serde_json::from_slice(&std::fs::read(&metadata_path).with_context(|| {
            format!(
                "Failed to read Codex backup metadata: {}",
                metadata_path.display()
            )
        })?)?;
    if metadata.version != 1
        || metadata.provider_id != PROVIDER_ID
        || metadata.operation_id != backup.operation_id
        || metadata.provider_session_id != backup.provider_session_id
        || metadata.mutation != backup.mutation
        || metadata.codex_home != backup.source_path
        || metadata.db_path != backup.source_path.join(CODEX_SQLITE_FILE_BASENAME)
    {
        anyhow::bail!(
            "Codex backup metadata does not match the registered restore context: {}",
            backup.backup_path.display()
        );
    }
    validate_codex_file_manifest(&metadata)?;

    if metadata.database_present {
        restore_codex_sqlite_backup(
            &metadata.db_path,
            &backup.backup_path.join(CODEX_SESSION_BACKUP_DB_PATH),
            metadata.mutation,
            &metadata.provider_session_id,
            &metadata.sqlite_tables,
        )?;
    } else if !metadata.sqlite_tables.is_empty() {
        anyhow::bail!("Codex backup contains SQLite rows without a source database");
    }

    restore_codex_file(&backup.backup_path, &metadata.session_index)?;
    restore_codex_file(&backup.backup_path, &metadata.rollout)?;
    Ok(())
}

fn codex_index_contains_session(index_path: &Path, session_id: &str) -> Result<bool> {
    let content = std::fs::read_to_string(index_path)?;
    for line in content.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line).with_context(|| {
            format!(
                "Failed to parse Codex session index: {}",
                index_path.display()
            )
        })?;
        if value.get("id").and_then(Value::as_str) == Some(session_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn capture_codex_file(
    source_path: Option<PathBuf>,
    relative_path: PathBuf,
    backup_path: &Path,
) -> Result<CodexFileBackup> {
    let Some(source_path) = source_path else {
        return Ok(CodexFileBackup {
            source_path: None,
            relative_path,
            present: false,
        });
    };
    let metadata = match std::fs::symlink_metadata(&source_path) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let Some(metadata) = metadata else {
        return Ok(CodexFileBackup {
            source_path: Some(source_path),
            relative_path,
            present: false,
        });
    };
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        anyhow::bail!(
            "Codex backup source is not a regular file: {}",
            source_path.display()
        );
    }
    let destination = backup_path.join(&relative_path);
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&source_path, &destination).with_context(|| {
        format!(
            "Failed to copy Codex backup source: {}",
            source_path.display()
        )
    })?;
    Ok(CodexFileBackup {
        source_path: Some(source_path),
        relative_path,
        present: true,
    })
}

fn restore_codex_file(backup_path: &Path, file: &CodexFileBackup) -> Result<()> {
    let Some(source_path) = &file.source_path else {
        if file.present {
            anyhow::bail!("Codex backup marks a pathless file as present");
        }
        return Ok(());
    };
    match std::fs::symlink_metadata(source_path) {
        Ok(metadata) if metadata.is_file() || metadata.file_type().is_symlink() => {
            std::fs::remove_file(source_path)?;
        }
        Ok(_) => {
            anyhow::bail!(
                "Codex restore target is not a file: {}",
                source_path.display()
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if !file.present {
        return Ok(());
    }

    let captured_path = backup_path.join(&file.relative_path);
    if !captured_path.is_file() {
        anyhow::bail!(
            "Codex backup file does not exist: {}",
            captured_path.display()
        );
    }
    if let Some(parent) = source_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(&captured_path, source_path)
        .with_context(|| format!("Failed to restore Codex file: {}", source_path.display()))?;
    Ok(())
}

fn validate_codex_file_manifest(metadata: &CodexSessionBackupMetadata) -> Result<()> {
    let expected_index = metadata.codex_home.join("session_index.jsonl");
    if metadata.session_index.source_path.as_deref() != Some(expected_index.as_path())
        || metadata.session_index.relative_path != Path::new("session_index.jsonl")
    {
        anyhow::bail!("Codex backup session index manifest is invalid");
    }
    if metadata.rollout.relative_path != Path::new("rollout/session.jsonl") {
        anyhow::bail!("Codex backup rollout manifest is invalid");
    }
    if let Some(rollout_path) = metadata.rollout.source_path.as_deref() {
        if !rollout_path.starts_with(&metadata.codex_home) {
            anyhow::bail!("Codex backup rollout path is outside the Codex data directory");
        }
    } else if metadata.rollout.present {
        anyhow::bail!("Codex backup marks a pathless rollout as present");
    }
    if matches!(
        metadata.mutation,
        ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
    ) && !metadata.rollout.present
    {
        anyhow::bail!("Codex full-source backup does not contain a rollout");
    }
    Ok(())
}

fn capture_codex_sqlite_backup(
    conn: &mut Connection,
    mutation: ProviderSourceMutation,
    session_id: &str,
    backup_db_path: &Path,
) -> Result<Vec<CodexSqliteTableManifest>> {
    if backup_db_path.exists() {
        anyhow::bail!(
            "Codex SQLite backup already exists: {}",
            backup_db_path.display()
        );
    }
    let backup_db_path_str = backup_db_path.to_str().with_context(|| {
        format!(
            "Codex SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    conn.execute("ATTACH DATABASE ?1 AS memorph_backup", [backup_db_path_str])?;

    let capture_result = (|| -> Result<Vec<CodexSqliteTableManifest>> {
        let tx = conn.transaction()?;
        let mut manifests = Vec::new();
        match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
                capture_codex_full_table(
                    &tx,
                    "threads",
                    &["id"],
                    "id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                capture_codex_full_table(
                    &tx,
                    "thread_dynamic_tools",
                    &["thread_id"],
                    "thread_id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                capture_codex_full_table(
                    &tx,
                    "thread_goals",
                    &["thread_id"],
                    "thread_id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                capture_codex_full_table(
                    &tx,
                    "thread_spawn_edges",
                    &["parent_thread_id", "child_thread_id"],
                    "parent_thread_id = ?1 OR child_thread_id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                capture_codex_full_table(
                    &tx,
                    "stage1_outputs",
                    &["thread_id"],
                    "thread_id = ?1",
                    session_id,
                    &mut manifests,
                )?;
                if has_table(&tx, "agent_job_items")?
                    && has_columns(&tx, "agent_job_items", &["assigned_thread_id"])?
                    && !has_columns(
                        &tx,
                        "agent_job_items",
                        &["job_id", "item_id", "assigned_thread_id"],
                    )?
                {
                    anyhow::bail!(
                        "Codex table agent_job_items cannot restore assigned_thread_id without job_id and item_id"
                    );
                }
                capture_codex_selected_table(
                    &tx,
                    "agent_job_items",
                    &["job_id", "item_id", "assigned_thread_id"],
                    "assigned_thread_id = ?1",
                    session_id,
                    CodexSqliteRestoreMode::AssignedThread,
                    &mut manifests,
                )?;
            }
            ProviderSourceMutation::Rename => {
                capture_codex_selected_table(
                    &tx,
                    "threads",
                    &["id", "title"],
                    "id = ?1",
                    session_id,
                    CodexSqliteRestoreMode::ThreadTitle,
                    &mut manifests,
                )?;
            }
        }
        tx.commit()?;
        Ok(manifests)
    })();

    let detach_result = conn.execute_batch("DETACH DATABASE memorph_backup;");
    match capture_result {
        Ok(manifests) => {
            detach_result?;
            Ok(manifests)
        }
        Err(error) => {
            let _ = detach_result;
            let _ = std::fs::remove_file(backup_db_path);
            Err(error)
        }
    }
}

fn capture_codex_full_table(
    conn: &Connection,
    table: &str,
    required_columns: &[&str],
    where_clause: &str,
    session_id: &str,
    manifests: &mut Vec<CodexSqliteTableManifest>,
) -> Result<()> {
    if !has_table(conn, table)? {
        return Ok(());
    }
    if !has_columns(conn, table, required_columns)? {
        anyhow::bail!("Codex table {table} is missing required session columns");
    }
    let quoted_table = quote_codex_sqlite_identifier(table);
    conn.execute(
        &format!(
            "CREATE TABLE memorph_backup.{quoted_table} AS
             SELECT * FROM main.{quoted_table} WHERE {where_clause}"
        ),
        [session_id],
    )?;
    manifests.push(codex_sqlite_table_manifest(
        conn,
        table,
        CodexSqliteRestoreMode::FullRows,
    )?);
    Ok(())
}

fn capture_codex_selected_table(
    conn: &Connection,
    table: &str,
    columns: &[&str],
    where_clause: &str,
    session_id: &str,
    restore_mode: CodexSqliteRestoreMode,
    manifests: &mut Vec<CodexSqliteTableManifest>,
) -> Result<()> {
    if !has_table(conn, table)? {
        return Ok(());
    }
    if !has_columns(conn, table, columns)? {
        return Ok(());
    }
    let quoted_table = quote_codex_sqlite_identifier(table);
    let column_list = columns
        .iter()
        .map(|column| quote_codex_sqlite_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute(
        &format!(
            "CREATE TABLE memorph_backup.{quoted_table} AS
             SELECT {column_list} FROM main.{quoted_table} WHERE {where_clause}"
        ),
        [session_id],
    )?;
    manifests.push(codex_sqlite_table_manifest(conn, table, restore_mode)?);
    Ok(())
}

fn codex_sqlite_table_manifest(
    conn: &Connection,
    table: &str,
    restore_mode: CodexSqliteRestoreMode,
) -> Result<CodexSqliteTableManifest> {
    let quoted_table = quote_codex_sqlite_identifier(table);
    let row_count: i64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM memorph_backup.{quoted_table}"),
        [],
        |row| row.get(0),
    )?;
    Ok(CodexSqliteTableManifest {
        table: table.to_string(),
        columns: codex_table_columns_in_schema(conn, "memorph_backup", table)?,
        row_count: usize::try_from(row_count)
            .context("Codex backup row count does not fit in usize")?,
        restore_mode,
    })
}

fn restore_codex_sqlite_backup(
    db_path: &Path,
    backup_db_path: &Path,
    mutation: ProviderSourceMutation,
    session_id: &str,
    manifests: &[CodexSqliteTableManifest],
) -> Result<()> {
    if !db_path.exists() {
        anyhow::bail!(
            "Codex database required by backup no longer exists: {}",
            db_path.display()
        );
    }
    if !backup_db_path.exists() {
        anyhow::bail!(
            "Codex SQLite backup does not exist: {}",
            backup_db_path.display()
        );
    }
    let backup_db_path_str = backup_db_path.to_str().with_context(|| {
        format!(
            "Codex SQLite backup path is not valid UTF-8: {}",
            backup_db_path.display()
        )
    })?;
    let mut conn = Connection::open(db_path)?;
    conn.execute("ATTACH DATABASE ?1 AS memorph_backup", [backup_db_path_str])?;

    let restore_result = (|| -> Result<()> {
        validate_codex_sqlite_backup(&conn, mutation, manifests)?;
        let tx = conn.transaction()?;
        match mutation {
            ProviderSourceMutation::Delete | ProviderSourceMutation::Replace => {
                delete_codex_sqlite_rows(&tx, session_id)?;
                if let Some(manifest) = manifests
                    .iter()
                    .find(|manifest| manifest.table == "threads")
                {
                    insert_codex_full_backup_table(&tx, manifest)?;
                }
                for manifest in manifests.iter().filter(|manifest| {
                    manifest.restore_mode == CodexSqliteRestoreMode::FullRows
                        && manifest.table != "threads"
                }) {
                    insert_codex_full_backup_table(&tx, manifest)?;
                }
                if let Some(manifest) = manifests.iter().find(|manifest| {
                    manifest.restore_mode == CodexSqliteRestoreMode::AssignedThread
                }) {
                    restore_codex_assigned_threads(&tx, manifest)?;
                }
            }
            ProviderSourceMutation::Rename => {
                if let Some(manifest) = manifests.first() {
                    restore_codex_thread_title(&tx, manifest)?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    })();
    let detach_result = conn.execute_batch("DETACH DATABASE memorph_backup;");
    if let Err(error) = restore_result {
        return Err(error);
    }
    detach_result?;
    Ok(())
}

fn validate_codex_sqlite_backup(
    conn: &Connection,
    mutation: ProviderSourceMutation,
    manifests: &[CodexSqliteTableManifest],
) -> Result<()> {
    validate_codex_sqlite_manifest_contract(mutation, manifests)?;

    let mut tables_stmt = conn.prepare(
        "SELECT name
         FROM memorph_backup.sqlite_master
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
         ORDER BY name",
    )?;
    let backup_tables = tables_stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<HashSet<_>, _>>()?;
    let manifest_tables = manifests
        .iter()
        .map(|manifest| manifest.table.clone())
        .collect::<HashSet<_>>();
    if backup_tables != manifest_tables || manifest_tables.len() != manifests.len() {
        anyhow::bail!("Codex SQLite backup table manifest does not match backup database");
    }

    for manifest in manifests {
        let live_columns = table_columns(conn, &manifest.table)?;
        let backup_columns =
            codex_table_columns_in_schema(conn, "memorph_backup", &manifest.table)?;
        if backup_columns != manifest.columns {
            anyhow::bail!(
                "Codex backup table {} schema does not match its manifest",
                manifest.table
            );
        }
        match manifest.restore_mode {
            CodexSqliteRestoreMode::FullRows if live_columns != manifest.columns => {
                anyhow::bail!("Codex table {} schema changed since backup", manifest.table);
            }
            CodexSqliteRestoreMode::AssignedThread | CodexSqliteRestoreMode::ThreadTitle => {
                let live_columns = live_columns.into_iter().collect::<HashSet<_>>();
                if !manifest
                    .columns
                    .iter()
                    .all(|column| live_columns.contains(column))
                {
                    anyhow::bail!("Codex table {} schema changed since backup", manifest.table);
                }
            }
            CodexSqliteRestoreMode::FullRows => {}
        }

        let quoted_table = quote_codex_sqlite_identifier(&manifest.table);
        let row_count: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM memorph_backup.{quoted_table}"),
            [],
            |row| row.get(0),
        )?;
        if usize::try_from(row_count).ok() != Some(manifest.row_count) {
            anyhow::bail!(
                "Codex SQLite backup row count mismatch for table {}",
                manifest.table
            );
        }
    }
    Ok(())
}

fn validate_codex_sqlite_manifest_contract(
    mutation: ProviderSourceMutation,
    manifests: &[CodexSqliteTableManifest],
) -> Result<()> {
    let mut seen = HashSet::new();
    for manifest in manifests {
        if !seen.insert(manifest.table.as_str()) {
            anyhow::bail!("Codex SQLite backup contains duplicate table manifests");
        }
        let expected = match manifest.table.as_str() {
            "threads"
                if matches!(
                    mutation,
                    ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
                ) =>
            {
                (CodexSqliteRestoreMode::FullRows, None)
            }
            "threads" if mutation == ProviderSourceMutation::Rename => (
                CodexSqliteRestoreMode::ThreadTitle,
                Some(&["id", "title"][..]),
            ),
            "thread_dynamic_tools" | "thread_goals" | "thread_spawn_edges" | "stage1_outputs"
                if matches!(
                    mutation,
                    ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
                ) =>
            {
                (CodexSqliteRestoreMode::FullRows, None)
            }
            "agent_job_items"
                if matches!(
                    mutation,
                    ProviderSourceMutation::Delete | ProviderSourceMutation::Replace
                ) =>
            {
                (
                    CodexSqliteRestoreMode::AssignedThread,
                    Some(&["job_id", "item_id", "assigned_thread_id"][..]),
                )
            }
            _ => anyhow::bail!(
                "Codex SQLite backup contains an unexpected table: {}",
                manifest.table
            ),
        };
        if manifest.restore_mode != expected.0
            || expected
                .1
                .is_some_and(|columns| manifest.columns != columns)
        {
            anyhow::bail!(
                "Codex SQLite backup has an invalid restore contract for table {}",
                manifest.table
            );
        }
    }
    if mutation == ProviderSourceMutation::Rename
        && (manifests.len() > 1
            || manifests
                .first()
                .is_some_and(|manifest| manifest.table != "threads"))
    {
        anyhow::bail!("Codex rename backup may contain only the threads title projection");
    }
    Ok(())
}

fn insert_codex_full_backup_table(
    conn: &Connection,
    manifest: &CodexSqliteTableManifest,
) -> Result<()> {
    if manifest.row_count == 0 {
        return Ok(());
    }
    let column_list = manifest
        .columns
        .iter()
        .map(|column| quote_codex_sqlite_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute(
        &format!(
            "INSERT INTO main.{} ({column_list})
             SELECT {column_list} FROM memorph_backup.{}",
            quote_codex_sqlite_identifier(&manifest.table),
            quote_codex_sqlite_identifier(&manifest.table),
        ),
        [],
    )?;
    Ok(())
}

fn restore_codex_assigned_threads(
    conn: &Connection,
    manifest: &CodexSqliteTableManifest,
) -> Result<()> {
    if manifest.row_count == 0 {
        return Ok(());
    }
    let updated = conn.execute(
        "UPDATE main.agent_job_items
         SET assigned_thread_id = (
             SELECT backup.assigned_thread_id
             FROM memorph_backup.agent_job_items AS backup
             WHERE backup.job_id = main.agent_job_items.job_id
               AND backup.item_id = main.agent_job_items.item_id
         )
         WHERE EXISTS (
             SELECT 1
             FROM memorph_backup.agent_job_items AS backup
             WHERE backup.job_id = main.agent_job_items.job_id
               AND backup.item_id = main.agent_job_items.item_id
         )",
        [],
    )?;
    if updated != manifest.row_count {
        anyhow::bail!("Codex restore could not find every agent job item captured by the backup");
    }
    Ok(())
}

fn restore_codex_thread_title(
    conn: &Connection,
    manifest: &CodexSqliteTableManifest,
) -> Result<()> {
    if manifest.row_count == 0 {
        return Ok(());
    }
    let updated = conn.execute(
        "UPDATE main.threads
         SET title = (
             SELECT backup.title
             FROM memorph_backup.threads AS backup
             WHERE backup.id = main.threads.id
         )
         WHERE EXISTS (
             SELECT 1
             FROM memorph_backup.threads AS backup
             WHERE backup.id = main.threads.id
         )",
        [],
    )?;
    if updated != manifest.row_count {
        anyhow::bail!("Codex restore could not find the thread captured by the rename backup");
    }
    Ok(())
}

fn codex_table_columns_in_schema(
    conn: &Connection,
    schema: &str,
    table: &str,
) -> Result<Vec<String>> {
    let pragma = format!(
        "PRAGMA {}.table_info({})",
        quote_codex_sqlite_identifier(schema),
        quote_codex_sqlite_identifier(table)
    );
    let mut stmt = conn.prepare(&pragma)?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if columns.is_empty() {
        anyhow::bail!("Codex database table not found: {schema}.{table}");
    }
    Ok(columns)
}

fn quote_codex_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
fn set_test_codex_mutation_failure(mutation: Option<ProviderSourceMutation>) {
    *TEST_CODEX_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Codex mutation failure lock") = mutation;
}

#[cfg(test)]
fn fail_codex_mutation_after_file_write(mutation: ProviderSourceMutation) -> Result<()> {
    let mut failure = TEST_CODEX_MUTATION_FAILURE
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Codex mutation failure lock");
    if *failure == Some(mutation) {
        *failure = None;
        anyhow::bail!("injected Codex mutation failure after file write");
    }
    Ok(())
}

#[cfg(not(test))]
fn fail_codex_mutation_after_file_write(_mutation: ProviderSourceMutation) -> Result<()> {
    Ok(())
}

fn delete_codex_session(session_id: &str) -> Result<()> {
    let session_path = find_session_file(session_id)
        .with_context(|| format!("Codex session not found: {session_id}"))?;
    std::fs::remove_file(&session_path)
        .with_context(|| format!("Failed to remove session file: {}", session_path.display()))?;

    let index_path = get_codex_dir().join("session_index.jsonl");
    if index_path.exists() {
        let content = std::fs::read_to_string(&index_path)?;
        let mut new_lines = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_str::<Value>(line) {
                if value.get("id").and_then(Value::as_str) == Some(session_id) {
                    continue;
                }
            }
            new_lines.push(line.to_string());
        }
        std::fs::write(&index_path, new_lines.join("\n") + "\n")?;
    }

    fail_codex_mutation_after_file_write(ProviderSourceMutation::Delete)?;

    let db_path = get_codex_dir().join(CODEX_SQLITE_FILE_BASENAME);
    if db_path.exists() {
        let mut conn = Connection::open(&db_path)?;
        let tx = conn.transaction()?;
        delete_codex_sqlite_rows(&tx, session_id)?;
        tx.commit()?;
    }
    Ok(())
}

fn rename_codex_session(session_id: &str, new_title: &str) -> Result<()> {
    let index_path = get_codex_dir().join("session_index.jsonl");
    if !index_path.exists() {
        anyhow::bail!("Codex session index not found");
    }

    let content = std::fs::read_to_string(&index_path)?;
    let mut new_lines = Vec::new();
    let mut found = false;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut value: Value = serde_json::from_str(line)?;
        if value.get("id").and_then(Value::as_str) == Some(session_id) {
            if let Value::Object(ref mut map) = value {
                map.insert(
                    "thread_name".to_string(),
                    Value::String(new_title.to_string()),
                );
                found = true;
            }
            new_lines.push(serde_json::to_string(&value)?);
        } else {
            new_lines.push(line.to_string());
        }
    }
    if !found {
        anyhow::bail!("Codex session not found in index: {session_id}");
    }

    std::fs::write(&index_path, new_lines.join("\n") + "\n")?;
    if let Some(session_path) = find_session_file(session_id) {
        update_rollout_session_meta_title(&session_path, new_title)?;
    }

    fail_codex_mutation_after_file_write(ProviderSourceMutation::Rename)?;

    let db_path = get_codex_dir().join(CODEX_SQLITE_FILE_BASENAME);
    if db_path.exists() {
        let conn = Connection::open(&db_path)?;
        if has_table(&conn, "threads")? && has_columns(&conn, "threads", &["id", "title"])? {
            conn.execute(
                "UPDATE threads SET title = ?1 WHERE id = ?2",
                [new_title, session_id],
            )?;
        }
    }
    Ok(())
}

fn delete_codex_sqlite_rows(conn: &Connection, session_id: &str) -> Result<()> {
    delete_related_rows(conn, "thread_dynamic_tools", "thread_id = ?1", session_id)?;
    delete_related_rows(conn, "thread_goals", "thread_id = ?1", session_id)?;
    delete_related_rows(
        conn,
        "thread_spawn_edges",
        "parent_thread_id = ?1 OR child_thread_id = ?1",
        session_id,
    )?;
    delete_related_rows(conn, "stage1_outputs", "thread_id = ?1", session_id)?;
    if has_table(conn, "agent_job_items")?
        && has_columns(conn, "agent_job_items", &["assigned_thread_id"])?
    {
        conn.execute(
            "UPDATE agent_job_items
             SET assigned_thread_id = NULL
             WHERE assigned_thread_id = ?1",
            [session_id],
        )?;
    }
    if has_table(conn, "threads")? {
        conn.execute("DELETE FROM threads WHERE id = ?1", [session_id])?;
    }
    Ok(())
}

pub fn sync_workspace_sessions(
    workspace: Option<&str>,
    codex_home: Option<&Path>,
    keep_backups: usize,
    actor: ActivityActor,
) -> Result<CodexWorkspaceRepairReport> {
    let codex_dir = codex_home
        .map(Path::to_path_buf)
        .unwrap_or_else(get_codex_dir);
    let backup_root = crate::config::memorph_dir()?
        .join("artifacts")
        .join("backups")
        .join("codex-sync");
    let mut activity_conn = local_store::open_database()?;
    let input_details = serde_json::json!({
        "workspace": workspace,
        "codex_home": utils::user_visible_path(&codex_dir.to_string_lossy()),
        "keep_backups": keep_backups,
    });
    let activity_id = ActivityStore::new(&activity_conn).start(NewActivity {
        provider_id: Some(PROVIDER_ID.to_string()),
        provider_session_id: None,
        workspace_dir: workspace.map(str::to_string),
        operation_kind: ActivityOperationKind::Sync,
        actor,
        summary: "Synchronizing Codex workspace sessions".to_string(),
        details: input_details.clone(),
    })?;
    let result = sync_workspace_sessions_in_codex_home(
        &mut activity_conn,
        &activity_id,
        &backup_root,
        &codex_dir,
        workspace,
        keep_backups,
    );
    match result {
        Ok(report) => {
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion {
                    workspace_dir: Some(report.workspace_dir.clone()),
                    ..ActivityCompletion::success(
                        "Synchronized Codex workspace sessions",
                        serde_json::json!({"report": &report}),
                    )
                },
            )?;
            Ok(report)
        }
        Err(error) => {
            let message = format!("{error:#}");
            ActivityStore::new(&activity_conn).finish(
                &activity_id,
                ActivityCompletion::failed(
                    "Failed to synchronize Codex workspace sessions",
                    input_details,
                    &message,
                ),
            )?;
            Err(error)
        }
    }
}

pub fn repair_workspace_sessions(
    workspace: Option<&str>,
    actor: ActivityActor,
) -> Result<CodexWorkspaceRepairReport> {
    sync_workspace_sessions(workspace, None, DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT, actor)
}

fn sync_workspace_sessions_in_codex_home(
    activity_conn: &mut Connection,
    operation_id: &str,
    backup_root: &Path,
    codex_dir: &Path,
    workspace: Option<&str>,
    keep_backups: usize,
) -> Result<CodexWorkspaceRepairReport> {
    if keep_backups < 1 {
        anyhow::bail!("keep_backups must be at least 1");
    }

    let workspace_root = crate::config::resolve_workspace(workspace)?;
    let workspace_key = crate::provider::default_normalized_workspace_key(workspace_root.to_str())
        .with_context(|| {
            format!(
                "Failed to normalize workspace path: {}",
                workspace_root.display()
            )
        })?;
    let current_model_provider = read_codex_model_provider(codex_dir);
    let mut report = CodexWorkspaceRepairReport {
        workspace_dir: utils::user_visible_path(&workspace_key),
        current_model_provider: current_model_provider.clone(),
        scanned_rollouts: 0,
        workspace_session_count: 0,
        hidden_session_count: 0,
        repaired_session_count: 0,
        reindexed_session_count: 0,
        retitled_session_count: 0,
        backup_dir: None,
        backup_artifact_id: None,
        backup_id: None,
        sqlite_rows_updated: 0,
        sqlite_provider_rows_updated: 0,
        sqlite_user_event_rows_updated: 0,
        sqlite_cwd_rows_updated: 0,
        pruned_backup_count: 0,
        skipped_rollout_files: Vec::new(),
        touched_sessions: Vec::new(),
    };

    let index_path = codex_dir.join("session_index.jsonl");
    let mut indexed_session_entries = load_session_index_entries(&index_path)?;
    let sqlite_lookup = build_sqlite_thread_metadata_lookup(codex_dir)?;
    let session_states = session_state::load_state_store().unwrap_or_default();
    let mut candidates = Vec::new();

    for dir_name in CODEX_SYNC_SESSION_DIRS {
        let root = codex_dir.join(dir_name);
        if !root.exists() {
            continue;
        }

        for entry in WalkDir::new(&root)
            .max_depth(5)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }

            report.scanned_rollouts += 1;
            let Some(session) = (match read_codex_rollout_summary(path) {
                Ok(session) => session,
                Err(error) if is_rollout_file_busy_error(&error) => {
                    report
                        .skipped_rollout_files
                        .push(utils::user_visible_path(&path.to_string_lossy()));
                    continue;
                }
                Err(error) => return Err(error),
            }) else {
                continue;
            };

            if !crate::provider::default_workspace_matches(
                session.workspace_dir.as_deref(),
                Some(&workspace_key),
            ) {
                continue;
            }

            report.workspace_session_count += 1;
            if session.model_provider.as_deref() != Some(current_model_provider.as_str()) {
                report.hidden_session_count += 1;
            }

            candidates.push(CodexWorkspaceSyncCandidate {
                rollout_path: path.to_path_buf(),
                session,
            });
        }
    }

    if candidates.is_empty() && !codex_dir.join(CODEX_GLOBAL_STATE_FILE_BASENAME).exists() {
        return Ok(report);
    }

    let provider_session_ids = candidates
        .iter()
        .map(|candidate| candidate.session.session_id.clone())
        .collect::<Vec<_>>();
    let backup_dir = create_codex_sync_backup(
        backup_root,
        operation_id,
        codex_dir,
        &workspace_key,
        &current_model_provider,
        &candidates
            .iter()
            .map(|candidate| candidate.rollout_path.clone())
            .collect::<Vec<_>>(),
    )?;
    let backup = register_codex_sync_backup(
        activity_conn,
        operation_id,
        codex_dir,
        &backup_dir,
        &workspace_key,
        &current_model_provider,
        &provider_session_ids,
    )
    .with_context(|| {
        format!(
            "Failed to register Codex pre-write backup: {}",
            backup_dir.display()
        )
    })?;
    report.backup_dir = Some(utils::user_visible_path(&backup_dir.to_string_lossy()));
    report.backup_artifact_id = Some(backup.artifact.id);
    report.backup_id = Some(backup.id);

    let sync_result: Result<()> = (|| {
        let mut synced_sessions = Vec::new();

        for candidate in candidates {
            let mut session = candidate.session;
            let provider_mismatch =
                session.model_provider.as_deref() != Some(current_model_provider.as_str());
            let mut updated_model_provider = false;

            if provider_mismatch {
                match rewrite_rollout_model_provider(
                    &candidate.rollout_path,
                    &current_model_provider,
                ) {
                    Ok(()) => {
                        session.model_provider = Some(current_model_provider.clone());
                        updated_model_provider = true;
                        report.repaired_session_count += 1;
                    }
                    Err(error) if is_rollout_file_busy_error(&error) => {
                        report.skipped_rollout_files.push(utils::user_visible_path(
                            &candidate.rollout_path.to_string_lossy(),
                        ));
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }

            let mut added_to_index = false;
            let mut updated_index_title = false;
            let existing_index_title = indexed_session_entries
                .get(&session.session_id)
                .map(String::as_str);

            if existing_index_title.is_none() {
                let index_title = resolve_codex_reindex_title(
                    &session,
                    sqlite_lookup.get(&session.session_id),
                    &session_states,
                );
                append_session_index_entry(
                    &index_path,
                    &session.session_id,
                    &index_title,
                    session.updated_at.as_deref(),
                )?;
                session.title = Some(index_title.clone());
                indexed_session_entries.insert(session.session_id.clone(), index_title);
                added_to_index = true;
                report.reindexed_session_count += 1;
            } else if existing_index_title == Some(&session.session_id) {
                let better_title = resolve_codex_reindex_title(
                    &session,
                    sqlite_lookup.get(&session.session_id),
                    &session_states,
                );
                if !better_title.is_empty() && better_title != session.session_id {
                    update_session_index_entry(&index_path, &session.session_id, &better_title)?;
                    indexed_session_entries
                        .insert(session.session_id.clone(), better_title.clone());
                    session.title = Some(better_title);
                    updated_index_title = true;
                    report.retitled_session_count += 1;
                }
            }

            if updated_model_provider || added_to_index || updated_index_title {
                report.touched_sessions.push(CodexWorkspaceRepairItem {
                    session_id: session.session_id.clone(),
                    title: session.title.clone(),
                    rollout_path: utils::user_visible_path(
                        &candidate.rollout_path.to_string_lossy(),
                    ),
                    workspace_dir: session
                        .workspace_dir
                        .as_deref()
                        .map(utils::user_visible_path),
                    previous_model_provider: session.original_model_provider.clone(),
                    current_model_provider: current_model_provider.clone(),
                    updated_model_provider,
                    added_to_index,
                    updated_index_title,
                });
            }

            synced_sessions.push(session);
        }

        let sqlite_stats =
            sync_workspace_sqlite_metadata(codex_dir, &current_model_provider, &synced_sessions)?;
        report.sqlite_rows_updated = sqlite_stats.rows_updated;
        report.sqlite_provider_rows_updated = sqlite_stats.provider_rows_updated;
        report.sqlite_user_event_rows_updated = sqlite_stats.user_event_rows_updated;
        report.sqlite_cwd_rows_updated = sqlite_stats.cwd_rows_updated;

        update_codex_global_state_file_if_exists(codex_dir, &workspace_root)?;
        Ok(())
    })();

    if let Err(error) = sync_result {
        restore_codex_sync_backup(codex_dir, &backup_dir).with_context(|| {
            format!(
                "Failed to restore Codex sync backup after error: {}",
                backup_dir.display()
            )
        })?;
        return Err(error);
    }

    report.pruned_backup_count = prune_codex_sync_backups(
        activity_conn,
        backup_root,
        codex_dir,
        operation_id,
        keep_backups,
    )?;
    Ok(report)
}

fn sync_workspace_sqlite_metadata(
    codex_dir: &Path,
    target_provider: &str,
    sessions: &[CodexRolloutSummary],
) -> Result<CodexWorkspaceSqliteStats> {
    if sessions.is_empty() {
        return Ok(CodexWorkspaceSqliteStats::default());
    }

    let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
    if !sqlite_path.exists() {
        return Ok(CodexWorkspaceSqliteStats::default());
    }

    let conn = Connection::open(&sqlite_path)
        .with_context(|| format!("Failed to open Codex SQLite: {}", sqlite_path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute("BEGIN IMMEDIATE", [])
        .with_context(|| "Failed to lock Codex SQLite for workspace sync")?;

    let sync_result: Result<CodexWorkspaceSqliteStats> = (|| {
        if !has_table(&conn, "threads")? {
            return Ok(CodexWorkspaceSqliteStats::default());
        }

        let has_provider_column = has_columns(&conn, "threads", &["model_provider"])?;
        let has_user_event_column = has_columns(&conn, "threads", &["has_user_event"])?;
        let has_cwd_column = has_columns(&conn, "threads", &["cwd"])?;

        let mut stats = CodexWorkspaceSqliteStats::default();
        let mut seen_ids = HashSet::new();

        let mut provider_stmt = if has_provider_column {
            Some(conn.prepare(
                "UPDATE threads SET model_provider = ?1 WHERE id = ?2 AND COALESCE(model_provider, '') <> ?1",
            )?)
        } else {
            None
        };
        let mut user_event_stmt = if has_user_event_column {
            Some(conn.prepare(
                "UPDATE threads SET has_user_event = 1 WHERE id = ?1 AND COALESCE(has_user_event, 0) <> 1",
            )?)
        } else {
            None
        };
        let mut cwd_stmt =
            if has_cwd_column {
                Some(conn.prepare(
                    "UPDATE threads SET cwd = ?1 WHERE id = ?2 AND COALESCE(cwd, '') <> ?1",
                )?)
            } else {
                None
            };

        for session in sessions {
            if !seen_ids.insert(session.session_id.clone()) {
                continue;
            }

            if let Some(stmt) = provider_stmt.as_mut() {
                stats.provider_rows_updated +=
                    stmt.execute(rusqlite::params![target_provider, &session.session_id])?;
            }

            if session.has_user_event {
                if let Some(stmt) = user_event_stmt.as_mut() {
                    stats.user_event_rows_updated +=
                        stmt.execute(rusqlite::params![&session.session_id])?;
                }
            }

            if let Some(workspace_dir) = session.workspace_dir.as_deref() {
                if !workspace_dir.trim().is_empty() {
                    if let Some(stmt) = cwd_stmt.as_mut() {
                        stats.cwd_rows_updated +=
                            stmt.execute(rusqlite::params![workspace_dir, &session.session_id])?;
                    }
                }
            }
        }

        stats.rows_updated =
            stats.provider_rows_updated + stats.user_event_rows_updated + stats.cwd_rows_updated;
        Ok(stats)
    })();

    match sync_result {
        Ok(stats) => {
            conn.execute("COMMIT", [])?;
            Ok(stats)
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", []);
            Err(error)
        }
    }
}

fn create_codex_sync_backup(
    backup_root: &Path,
    operation_id: &str,
    codex_dir: &Path,
    workspace_dir: &str,
    target_provider: &str,
    rollout_paths: &[PathBuf],
) -> Result<PathBuf> {
    std::fs::create_dir_all(backup_root).with_context(|| {
        format!(
            "Failed to create Codex sync backup root: {}",
            backup_root.display()
        )
    })?;
    let backup_dir = backup_root.join(operation_id);
    std::fs::create_dir(&backup_dir).with_context(|| {
        format!(
            "Failed to create Codex sync backup directory: {}",
            backup_dir.display()
        )
    })?;
    let rollouts_dir = backup_dir.join("rollouts");
    let db_dir = backup_dir.join("db");
    std::fs::create_dir_all(&rollouts_dir)?;
    std::fs::create_dir_all(&db_dir)?;

    let session_index_path = codex_dir.join("session_index.jsonl");
    let session_index_present =
        copy_if_present(&session_index_path, &backup_dir.join("session_index.jsonl"))?;

    let mut session_files = Vec::new();
    for rollout_path in rollout_paths {
        let relative = rollout_path.strip_prefix(codex_dir).with_context(|| {
            format!(
                "Failed to compute Codex rollout backup path: {}",
                rollout_path.display()
            )
        })?;
        let backup_path = rollouts_dir.join(relative);
        if let Some(parent) = backup_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(rollout_path, &backup_path).with_context(|| {
            format!(
                "Failed to back up Codex rollout file: {}",
                rollout_path.display()
            )
        })?;
        session_files.push(relative.to_string_lossy().to_string());
    }

    let mut db_files = Vec::new();
    for file_name in [
        CODEX_SQLITE_FILE_BASENAME,
        "state_5.sqlite-shm",
        "state_5.sqlite-wal",
    ] {
        let source = codex_dir.join(file_name);
        let destination = db_dir.join(file_name);
        if copy_if_present(&source, &destination)? {
            db_files.push(file_name.to_string());
        }
    }

    let mut global_state_files = Vec::new();
    for file_name in [
        CODEX_GLOBAL_STATE_FILE_BASENAME,
        CODEX_GLOBAL_STATE_BACKUP_FILE_BASENAME,
    ] {
        let source = codex_dir.join(file_name);
        let destination = backup_dir.join(file_name);
        if copy_if_present(&source, &destination)? {
            global_state_files.push(file_name.to_string());
        }
    }

    let metadata = CodexSyncBackupMetadata {
        version: 1,
        namespace: CODEX_SYNC_BACKUP_NAMESPACE.to_string(),
        operation_id: operation_id.to_string(),
        codex_home: codex_dir.to_string_lossy().to_string(),
        workspace_dir: workspace_dir.to_string(),
        target_provider: target_provider.to_string(),
        created_at: Utc::now().to_rfc3339(),
        session_index_present,
        session_files,
        db_files,
        global_state_files,
    };
    std::fs::write(
        backup_dir.join("metadata.json"),
        serde_json::to_string_pretty(&metadata)?,
    )?;

    Ok(backup_dir)
}

fn register_codex_sync_backup(
    conn: &mut Connection,
    operation_id: &str,
    codex_dir: &Path,
    backup_dir: &Path,
    workspace_dir: &str,
    target_provider: &str,
    provider_session_ids: &[String],
) -> Result<BackupRecord> {
    ArtifactStore::new(conn).register_backup(NewBackupRecord {
        operation_id: Some(operation_id.to_string()),
        provider_id: Some(PROVIDER_ID.to_string()),
        provider_session_id: None,
        session_id: None,
        source_path: Some(codex_dir.to_path_buf()),
        backup_path: backup_dir.to_path_buf(),
        restore_hint: Some(
            "Restore this backup with memorph's Codex sync restore flow before reopening Codex."
                .to_string(),
        ),
        mime_type: Some("application/vnd.memorph.codex-sync-backup".to_string()),
        format: Some("codex-sync-backup-v1".to_string()),
        artifact_metadata: serde_json::json!({
            "role": "codex_prewrite_sync_backup",
            "workspace_dir": workspace_dir,
            "target_provider": target_provider,
            "provider_session_ids": provider_session_ids,
        }),
        backup_metadata: serde_json::json!({
            "restore_mode": "codex_sync_restore",
            "metadata_file": "metadata.json",
        }),
    })
}

fn restore_codex_sync_backup(codex_dir: &Path, backup_dir: &Path) -> Result<()> {
    let metadata_path = backup_dir.join("metadata.json");
    let metadata: CodexSyncBackupMetadata =
        serde_json::from_str(&std::fs::read_to_string(&metadata_path).with_context(|| {
            format!(
                "Failed to read Codex sync backup metadata: {}",
                metadata_path.display()
            )
        })?)?;

    if metadata.codex_home != codex_dir.to_string_lossy() {
        anyhow::bail!(
            "Codex sync backup belongs to another home: {}",
            metadata.codex_home
        );
    }

    let session_index_path = codex_dir.join("session_index.jsonl");
    if metadata.session_index_present {
        std::fs::copy(backup_dir.join("session_index.jsonl"), &session_index_path).with_context(
            || {
                format!(
                    "Failed to restore Codex session index from backup: {}",
                    session_index_path.display()
                )
            },
        )?;
    } else if session_index_path.exists() {
        std::fs::remove_file(&session_index_path).with_context(|| {
            format!(
                "Failed to remove newly created Codex session index: {}",
                session_index_path.display()
            )
        })?;
    }

    for relative in &metadata.session_files {
        let source = backup_dir.join("rollouts").join(relative);
        let target = codex_dir.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &target).with_context(|| {
            format!(
                "Failed to restore Codex rollout file from backup: {}",
                target.display()
            )
        })?;
    }

    let known_db_files = [
        CODEX_SQLITE_FILE_BASENAME,
        "state_5.sqlite-shm",
        "state_5.sqlite-wal",
    ];
    for file_name in known_db_files {
        let target = codex_dir.join(file_name);
        if metadata.db_files.iter().any(|entry| entry == file_name) {
            std::fs::copy(backup_dir.join("db").join(file_name), &target).with_context(|| {
                format!(
                    "Failed to restore Codex SQLite backup: {}",
                    target.display()
                )
            })?;
        } else if target.exists() {
            std::fs::remove_file(&target).with_context(|| {
                format!(
                    "Failed to remove SQLite sidecar created during sync: {}",
                    target.display()
                )
            })?;
        }
    }

    for file_name in &metadata.global_state_files {
        let source = backup_dir.join(file_name);
        let target = codex_dir.join(file_name);
        std::fs::copy(&source, &target).with_context(|| {
            format!(
                "Failed to restore Codex global state backup: {}",
                target.display()
            )
        })?;
    }

    Ok(())
}

fn prune_codex_sync_backups(
    conn: &mut Connection,
    backup_root: &Path,
    codex_dir: &Path,
    current_operation_id: &str,
    keep_backups: usize,
) -> Result<usize> {
    if !backup_root.exists() {
        return Ok(0);
    }

    let mut managed_dirs = std::fs::read_dir(&backup_root)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter_map(|path| {
            let metadata = load_managed_codex_sync_backup(&path)?;
            (metadata.codex_home == codex_dir.to_string_lossy()).then_some((path, metadata))
        })
        .collect::<Vec<_>>();
    managed_dirs.sort_by(|left, right| {
        if left.1.operation_id == current_operation_id {
            return std::cmp::Ordering::Less;
        }
        if right.1.operation_id == current_operation_id {
            return std::cmp::Ordering::Greater;
        }
        right
            .1
            .created_at
            .cmp(&left.1.created_at)
            .then_with(|| right.0.cmp(&left.0))
    });

    let mut deleted = 0;
    for (stale, _) in managed_dirs.into_iter().skip(keep_backups) {
        let backup_record = ArtifactStore::new(conn).find_backup_by_artifact_path(&stale)?;
        std::fs::remove_dir_all(&stale).with_context(|| {
            format!(
                "Failed to remove stale Codex sync backup: {}",
                stale.display()
            )
        })?;
        if let Some(backup_record) = backup_record {
            ArtifactStore::new(conn).delete_backup_metadata(&backup_record.id)?;
        }
        deleted += 1;
    }

    Ok(deleted)
}

fn load_managed_codex_sync_backup(path: &Path) -> Option<CodexSyncBackupMetadata> {
    if !path.is_dir() {
        return None;
    }
    let metadata_path = path.join("metadata.json");
    let content = std::fs::read_to_string(metadata_path).ok()?;
    let metadata = serde_json::from_str::<CodexSyncBackupMetadata>(&content).ok()?;
    (metadata.version == 1 && metadata.namespace == CODEX_SYNC_BACKUP_NAMESPACE).then_some(metadata)
}

fn copy_if_present(source: &Path, destination: &Path) -> Result<bool> {
    if !source.exists() {
        return Ok(false);
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::copy(source, destination).with_context(|| {
        format!(
            "Failed to copy backup file: {} -> {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(true)
}

fn is_rollout_file_busy_error(error: &anyhow::Error) -> bool {
    let message = format!("{:#}", error).to_lowercase();
    message.contains("resource busy")
        || message.contains("being used by another process")
        || message.contains("currently in use")
        || message.contains("permission denied")
}

fn import_canonical_session(path: &Path) -> Result<ImportedSession> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::new();
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at: Option<chrono::DateTime<Utc>> = None;
    let mut last_active_at: Option<chrono::DateTime<Utc>> = None;
    let mut source_title: Option<String> = None;
    let mut extensions = BTreeMap::new();
    let mut turn_tracker = CodexTurnTracker::default();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Codex session line: {}", error),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(Value::String(line)),
                });
                continue;
            }
        };

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let timestamp = codex_line_timestamp(&value, line_idx + 1);
        last_active_at = Some(timestamp);
        let first_new_event = events.len();
        let turn_link = turn_tracker.observe_line(&value);

        match line_type.as_str() {
            "session_meta" => {
                if let Some(payload) = value.get("payload") {
                    session_id = payload
                        .get("id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(session_id);
                    project_dir = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(project_dir);
                    created_at = payload
                        .get("timestamp")
                        .and_then(|v| v.as_str())
                        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                        .map(|dt| dt.with_timezone(&Utc))
                        .or(created_at);

                    if let Some(text) = payload
                        .get("base_instructions")
                        .and_then(|v| v.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        events.push(SessionEvent {
                            id: format!("codex:base_instructions:{}", line_idx + 1),
                            kind: SessionEventKind::Lifecycle,
                            role: EventRole::System,
                            timestamp,
                            links: EventLinks::default(),
                            blocks: vec![
                                EventBlock::Text {
                                    text: text.to_string(),
                                },
                                EventBlock::ProviderPayload {
                                    kind: "session_meta".to_string(),
                                    payload: payload.clone(),
                                },
                            ],
                            metadata: EventMetadata {
                                source: EventSource {
                                    provider_id: PROVIDER_ID.to_string(),
                                    original_id: None,
                                    original_role: Some("developer".to_string()),
                                    phase: None,
                                },
                                model: payload
                                    .get("model")
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string),
                                usage: None,
                                fidelity: MappingDisposition::Preserved,
                                provider_ext: {
                                    let mut ext = BTreeMap::new();
                                    ext.insert("codex_raw_line".to_string(), value.clone());
                                    ext
                                },
                            },
                        });
                    } else {
                        events.push(provider_payload_event(
                            format!("codex:session_meta:{}", line_idx + 1),
                            SessionEventKind::Lifecycle,
                            EventRole::System,
                            timestamp,
                            "session_meta",
                            payload.clone(),
                            value.clone(),
                            None,
                        ));
                    }

                    source_title = payload
                        .get("title")
                        .or_else(|| payload.get("thread_name"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(source_title);
                    extensions.insert("codex_session_meta".to_string(), payload.clone());
                }
            }
            "turn_context" => {
                if let Some(payload) = value.get("payload") {
                    project_dir = payload
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(project_dir);
                    events.push(provider_payload_event(
                        format!("codex:turn_context:{}", line_idx + 1),
                        SessionEventKind::Lifecycle,
                        EventRole::System,
                        timestamp,
                        "turn_context",
                        payload.clone(),
                        value.clone(),
                        None,
                    ));
                }
            }
            "event_msg" => {
                if let Some(payload) = value.get("payload") {
                    events.push(codex_event_msg_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                    ));
                }
            }
            "response_item" => {
                if let Some(payload) = value.get("payload") {
                    let msg_type = payload.get("type").and_then(|v| v.as_str());
                    if msg_type == Some("token_count") {
                        continue;
                    }
                    events.push(codex_response_item_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                        &mut report,
                    ));
                }
            }
            "compacted" => {
                if let Some(payload) = value.get("payload") {
                    events.push(codex_compacted_event(
                        payload,
                        timestamp,
                        line_idx + 1,
                        value.clone(),
                    ));
                }
            }
            other => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Info,
                    disposition: MappingDisposition::Normalized,
                    code: "unknown_codex_line".to_string(),
                    message: format!("Preserved unknown Codex line type '{}'", other),
                    path: Some(format!("line:{}", line_idx + 1)),
                    raw: Some(value.clone()),
                });
                events.push(provider_payload_event(
                    format!("codex:unknown:{}", line_idx + 1),
                    SessionEventKind::Unknown,
                    EventRole::Unknown,
                    timestamp,
                    other,
                    value.get("payload").cloned().unwrap_or(Value::Null),
                    value,
                    None,
                ));
            }
        }
        for event in &mut events[first_new_event..] {
            turn_link.clone().apply_to(event);
        }
    }

    let canonical_id = session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_title =
        select_codex_display_title(None, None, source_title.as_deref(), &canonical_id);

    Ok(ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: canonical_id.clone(),
                source_title,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: canonical_id,
                    source_path: Some(path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: project_dir,
                created_at,
                last_active_at,
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions,
        },
        report,
    })
}

fn codex_line_timestamp(value: &Value, line_number: usize) -> chrono::DateTime<Utc> {
    value
        .get("timestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|| {
            chrono::DateTime::from_timestamp_millis(line_number as i64)
                .expect("Codex source line number is a valid timestamp")
        })
}

pub fn import_canonical_session_page(
    path: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<ProviderSessionImportPage> {
    let (state, locations) = load_or_build_codex_event_index_page(path, event_offset, event_limit)?;

    let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
    let mut events = Vec::with_capacity(locations.len());
    let mut file = File::open(path)
        .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;

    for location in locations {
        file.seek(SeekFrom::Start(location.byte_offset))
            .with_context(|| format!("Failed to seek Codex session: {}", path.display()))?;
        let mut line_bytes = vec![0u8; location.byte_length as usize];
        file.read_exact(&mut line_bytes)
            .with_context(|| format!("Failed to read Codex session: {}", path.display()))?;
        let line = String::from_utf8_lossy(&line_bytes);
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(error) => {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Dropped,
                    code: "invalid_jsonl_line".to_string(),
                    message: format!("Failed to parse Codex session line: {}", error),
                    path: Some(format!("line:{}", location.line_no)),
                    raw: Some(Value::String(line.into_owned())),
                });
                continue;
            }
        };

        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let payload = value.get("payload");
        let timestamp = codex_line_timestamp(&value, location.line_no);

        if let Some(mut event) = codex_event_from_line(
            line_type,
            payload,
            timestamp,
            location.line_no,
            value.clone(),
            &mut report,
        ) {
            CodexTurnLink {
                provider_turn_id: location.provider_turn_id,
                turn_index: location.turn_index,
                turn_boundary: location.turn_boundary,
            }
            .apply_to(&mut event);
            events.push(event);
        }
    }

    let imported = ImportedSession {
        session: CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: state.session_id.clone(),
                source_title: state.source_title.clone(),
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: Some("memorph-cli".to_string()),
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: state.session_id.clone(),
                    source_path: Some(path.to_string_lossy().to_string()),
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: state.workspace_dir.clone(),
                created_at: state
                    .created_at_ms
                    .and_then(chrono::DateTime::from_timestamp_millis),
                last_active_at: state
                    .last_active_at_ms
                    .and_then(chrono::DateTime::from_timestamp_millis),
                tags: Vec::new(),
            },
            events,
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        },
        report,
    };

    let turns = crate::session_projection::project_session_turns(
        &imported.session.identity.canonical_id,
        &imported.session.events,
        TurnQuality::Exact,
    );
    let turn_count = (event_offset == 0 && imported.session.events.len() == state.event_count)
        .then_some(turns.len());
    Ok(ProviderSessionImportPage {
        imported,
        event_count: state.event_count,
        message_count: state.message_count,
        turn_count,
        turns,
    })
}

fn load_or_build_codex_event_index_page(
    path: &Path,
    event_offset: usize,
    event_limit: Option<usize>,
) -> Result<(
    event_index::IndexedSessionState,
    Vec<event_index::IndexedEventLocation>,
)> {
    let source_path = path.to_string_lossy().to_string();
    let fingerprint = event_index::source_file_fingerprint(path)?;
    let mut conn = event_index::open_database()?;
    let mut state =
        match event_index::load_fresh_session_state(&conn, PROVIDER_ID, &source_path, fingerprint)?
        {
            Some(state) => state,
            None => {
                let (state, locations) = build_codex_event_index(path, fingerprint)?;
                event_index::replace_session_index(&mut conn, &state, &locations)?;
                state
            }
        };
    let mut locations = event_index::load_event_locations(
        &conn,
        PROVIDER_ID,
        &source_path,
        fingerprint,
        event_offset,
        event_limit,
    )?;

    if locations.is_empty() && event_offset < state.event_count && event_limit != Some(0) {
        let (rebuilt_state, rebuilt_locations) = build_codex_event_index(path, fingerprint)?;
        event_index::replace_session_index(&mut conn, &rebuilt_state, &rebuilt_locations)?;
        state = rebuilt_state;
        locations = event_index::load_event_locations(
            &conn,
            PROVIDER_ID,
            &source_path,
            fingerprint,
            event_offset,
            event_limit,
        )?;
    }

    Ok((state, locations))
}

fn build_codex_event_index(
    path: &Path,
    fingerprint: event_index::SourceFileFingerprint,
) -> Result<(
    event_index::IndexedSessionState,
    Vec<event_index::IndexedEventLocation>,
)> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex session: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut byte_offset = 0u64;
    let mut line_no = 0usize;
    let mut event_count = 0usize;
    let mut message_count = 0usize;
    let mut session_id: Option<String> = None;
    let mut project_dir: Option<String> = None;
    let mut created_at_ms: Option<i64> = None;
    let mut last_active_at_ms: Option<i64> = None;
    let mut source_title: Option<String> = None;
    let mut locations = Vec::new();
    let mut turn_tracker = CodexTurnTracker::default();

    loop {
        line.clear();
        let byte_length = reader
            .read_line(&mut line)
            .with_context(|| format!("Failed to read Codex session: {}", path.display()))?;
        if byte_length == 0 {
            break;
        }
        line_no += 1;
        let line_offset = byte_offset;
        byte_offset += byte_length as u64;

        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let line_type = value
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let payload = value.get("payload");
        let timestamp = codex_line_timestamp(&value, line_no);
        last_active_at_ms = Some(timestamp.timestamp_millis());
        let turn_link = turn_tracker.observe_line(&value);

        if line_type == "session_meta" {
            if let Some(payload) = payload {
                session_id = payload
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or(session_id);
                project_dir = payload
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or(project_dir);
                created_at_ms = payload
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.timestamp_millis())
                    .or(created_at_ms);
                source_title = payload
                    .get("title")
                    .or_else(|| payload.get("thread_name"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or(source_title);
            }
        } else if line_type == "turn_context" {
            if let Some(payload) = payload {
                project_dir = payload
                    .get("cwd")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .or(project_dir);
            }
        }

        if !codex_line_produces_event(line_type, payload) {
            continue;
        }

        if codex_line_is_visible_message(line_type, payload) {
            message_count += 1;
        }

        locations.push(event_index::IndexedEventLocation {
            event_index: event_count,
            byte_offset: line_offset,
            byte_length: byte_length as u64,
            line_no,
            provider_turn_id: turn_link.provider_turn_id,
            turn_index: turn_link.turn_index,
            turn_boundary: turn_link.turn_boundary,
        });
        event_count += 1;
    }

    let session_id = session_id
        .or_else(|| {
            path.file_stem()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let source_title = select_codex_display_title(None, None, source_title.as_deref(), &session_id);

    Ok((
        event_index::IndexedSessionState {
            provider_id: PROVIDER_ID.to_string(),
            session_id,
            source_path: path.to_string_lossy().to_string(),
            file_fingerprint: fingerprint,
            workspace_dir: project_dir,
            created_at_ms,
            last_active_at_ms,
            source_title,
            event_count,
            message_count,
        },
        locations,
    ))
}

fn codex_line_produces_event(line_type: &str, payload: Option<&Value>) -> bool {
    match line_type {
        "session_meta" | "turn_context" | "event_msg" | "compacted" => payload.is_some(),
        "response_item" => {
            payload.is_some()
                && payload.and_then(|p| p.get("type")).and_then(Value::as_str)
                    != Some("token_count")
        }
        _ => true,
    }
}

fn codex_line_is_visible_message(line_type: &str, payload: Option<&Value>) -> bool {
    match line_type {
        "response_item" => {
            let Some(payload) = payload else {
                return false;
            };
            match payload.get("type").and_then(Value::as_str) {
                Some("function_call" | "function_call_output") => true,
                Some("message") => {
                    let role = payload.get("role").and_then(Value::as_str);
                    if !matches!(role, Some("user" | "assistant" | "tool")) {
                        return false;
                    }
                    codex_response_message_has_visible_content(payload)
                }
                _ => false,
            }
        }
        "compacted" => true,
        _ => false,
    }
}

fn codex_response_message_has_visible_content(payload: &Value) -> bool {
    if payload
        .get("content")
        .and_then(Value::as_str)
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
    {
        return true;
    }
    payload
        .get("content")
        .and_then(Value::as_array)
        .map(|blocks| {
            blocks.iter().any(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("input_text" | "output_text" | "refusal" | "input_image")
                )
            })
        })
        .unwrap_or(false)
}

fn codex_event_from_line(
    line_type: &str,
    payload: Option<&Value>,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
    report: &mut MappingReport,
) -> Option<SessionEvent> {
    match line_type {
        "session_meta" => payload.map(|payload| {
            if let Some(text) = payload
                .get("base_instructions")
                .and_then(|v| v.get("text"))
                .and_then(Value::as_str)
            {
                SessionEvent {
                    id: format!("codex:base_instructions:{}", line_no),
                    kind: SessionEventKind::Lifecycle,
                    role: EventRole::System,
                    timestamp,
                    links: EventLinks::default(),
                    blocks: vec![
                        EventBlock::Text {
                            text: text.to_string(),
                        },
                        EventBlock::ProviderPayload {
                            kind: "session_meta".to_string(),
                            payload: payload.clone(),
                        },
                    ],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: PROVIDER_ID.to_string(),
                            original_id: None,
                            original_role: Some("developer".to_string()),
                            phase: None,
                        },
                        model: payload
                            .get("model")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: {
                            let mut ext = BTreeMap::new();
                            ext.insert("codex_raw_line".to_string(), raw_line);
                            ext
                        },
                    },
                }
            } else {
                provider_payload_event(
                    format!("codex:session_meta:{}", line_no),
                    SessionEventKind::Lifecycle,
                    EventRole::System,
                    timestamp,
                    "session_meta",
                    payload.clone(),
                    raw_line,
                    None,
                )
            }
        }),
        "turn_context" => payload.map(|payload| {
            provider_payload_event(
                format!("codex:turn_context:{}", line_no),
                SessionEventKind::Lifecycle,
                EventRole::System,
                timestamp,
                "turn_context",
                payload.clone(),
                raw_line,
                None,
            )
        }),
        "event_msg" => {
            payload.map(|payload| codex_event_msg_event(payload, timestamp, line_no, raw_line))
        }
        "response_item" => {
            let payload = payload?;
            if payload.get("type").and_then(Value::as_str) == Some("token_count") {
                None
            } else {
                Some(codex_response_item_event(
                    payload, timestamp, line_no, raw_line, report,
                ))
            }
        }
        "compacted" => {
            payload.map(|payload| codex_compacted_event(payload, timestamp, line_no, raw_line))
        }
        other => {
            report.push_issue(MappingIssue {
                level: MappingIssueLevel::Info,
                disposition: MappingDisposition::Normalized,
                code: "unknown_codex_line".to_string(),
                message: format!("Preserved unknown Codex line type '{}'", other),
                path: Some(format!("line:{}", line_no)),
                raw: Some(raw_line.clone()),
            });
            Some(provider_payload_event(
                format!("codex:unknown:{}", line_no),
                SessionEventKind::Unknown,
                EventRole::Unknown,
                timestamp,
                other,
                raw_line.get("payload").cloned().unwrap_or(Value::Null),
                raw_line,
                None,
            ))
        }
    }
}

fn codex_compacted_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
) -> SessionEvent {
    let memorph = payload.get("memorph").and_then(Value::as_object);
    let source_provider_id = memorph
        .and_then(|value| value.get("source_provider_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(PROVIDER_ID)
        .to_string();
    let summary = memorph
        .and_then(|value| value.get("summary"))
        .and_then(Value::as_str)
        .or_else(|| payload.get("message").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    let source_event_ids = memorph
        .and_then(|value| value.get("source_event_ids"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let source_event_count = memorph
        .and_then(|value| value.get("source_event_count"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .or_else(|| (!source_event_ids.is_empty()).then_some(source_event_ids.len()));
    let archive_ref = memorph
        .and_then(|value| value.get("archive_ref"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    let mut provider_ext = BTreeMap::new();
    provider_ext.insert("codex_payload".to_string(), payload.clone());
    provider_ext.insert("codex_raw_line".to_string(), raw_line);
    provider_ext.insert(
        "memorph_compression".to_string(),
        serde_json::json!({
            "source_provider_id": source_provider_id.clone(),
            "source_event_count": source_event_count,
            "archive_ref": archive_ref.clone(),
            "native": "codex",
        }),
    );

    SessionEvent {
        id: format!("codex:compacted:{}", line_no),
        kind: SessionEventKind::Message,
        role: EventRole::Assistant,
        timestamp,
        links: EventLinks::default(),
        blocks: vec![EventBlock::Compressed {
            source_provider_id,
            summary,
            source_event_ids,
            source_event_count,
            archive_ref,
        }],
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: Some("assistant".to_string()),
                phase: Some("compression".to_string()),
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Normalized,
            provider_ext,
        },
    }
}

fn codex_response_item_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
    report: &mut MappingReport,
) -> SessionEvent {
    let role_str = payload.get("role").and_then(|v| v.as_str());
    let msg_type = payload.get("type").and_then(|v| v.as_str());
    let phase = payload
        .get("phase")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let event_id = payload
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| format!("codex:response_item:{}", line_no));

    if msg_type == Some("function_call") {
        let name = payload
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let call_id = payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let input = payload.get("arguments").cloned();
        let role = match role_str {
            Some("assistant") | None => EventRole::Assistant,
            _ => EventRole::Unknown,
        };
        return SessionEvent {
            id: event_id,
            kind: SessionEventKind::ToolCall,
            role,
            timestamp,
            links: EventLinks::default(),
            blocks: vec![EventBlock::ToolCall {
                tool_call_id: call_id.to_string(),
                name: name.to_string(),
                input,
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: PROVIDER_ID.to_string(),
                    original_id: None,
                    original_role: role_str.map(str::to_string),
                    phase: phase.clone(),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: {
                    let mut ext = BTreeMap::new();
                    ext.insert("codex_payload".to_string(), payload.clone());
                    ext.insert("codex_raw_line".to_string(), raw_line);
                    ext
                },
            },
        };
    }

    if msg_type == Some("function_call_output") {
        let call_id = payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = payload
            .get("output")
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| value.to_string())
            })
            .unwrap_or_default();
        return SessionEvent {
            id: event_id,
            kind: SessionEventKind::ToolResult,
            role: EventRole::Tool,
            timestamp,
            links: EventLinks::default(),
            blocks: vec![EventBlock::ToolResult {
                tool_call_id: call_id.to_string(),
                content,
                is_error: false,
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: PROVIDER_ID.to_string(),
                    original_id: None,
                    original_role: Some("tool".to_string()),
                    phase: phase.clone(),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: {
                    let mut ext = BTreeMap::new();
                    ext.insert("codex_payload".to_string(), payload.clone());
                    ext.insert("codex_raw_line".to_string(), raw_line);
                    ext
                },
            },
        };
    }

    if msg_type != Some("message") {
        return provider_payload_event(
            event_id,
            SessionEventKind::Unknown,
            EventRole::Unknown,
            timestamp,
            msg_type.unwrap_or("response_item"),
            payload.clone(),
            raw_line,
            phase,
        );
    }

    let mut blocks = Vec::new();
    if let Some(content_arr) = payload.get("content").and_then(|v| v.as_array()) {
        for (idx, block) in content_arr.iter().enumerate() {
            let Some(block_type) = block.get("type").and_then(|v| v.as_str()) else {
                report.push_issue(MappingIssue {
                    level: MappingIssueLevel::Warning,
                    disposition: MappingDisposition::Normalized,
                    code: "codex_block_missing_type".to_string(),
                    message: "Codex content block without a type was preserved as unknown"
                        .to_string(),
                    path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                    raw: Some(block.clone()),
                });
                blocks.push(EventBlock::Unknown { raw: block.clone() });
                continue;
            };
            match block_type {
                "input_text" | "output_text" => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        blocks.push(EventBlock::Text {
                            text: text.to_string(),
                        });
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Warning,
                            disposition: MappingDisposition::Normalized,
                            code: "codex_text_block_missing_text".to_string(),
                            message:
                                "Codex text block without text was preserved as provider payload"
                                    .to_string(),
                            path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                            raw: Some(block.clone()),
                        });
                        blocks.push(EventBlock::ProviderPayload {
                            kind: block_type.to_string(),
                            payload: block.clone(),
                        });
                    }
                }
                "refusal" => {
                    if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                        blocks.push(EventBlock::Text {
                            text: text.to_string(),
                        });
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Warning,
                            disposition: MappingDisposition::Normalized,
                            code: "codex_refusal_block_missing_text".to_string(),
                            message:
                                "Codex refusal block without text was preserved as provider payload"
                                    .to_string(),
                            path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                            raw: Some(block.clone()),
                        });
                        blocks.push(EventBlock::ProviderPayload {
                            kind: block_type.to_string(),
                            payload: block.clone(),
                        });
                    }
                }
                "input_image" => {
                    if let Some(image_block) = codex_image_block(block) {
                        blocks.push(image_block);
                    } else {
                        report.push_issue(MappingIssue {
                            level: MappingIssueLevel::Info,
                            disposition: MappingDisposition::Normalized,
                            code: "codex_input_image_preserved_raw".to_string(),
                            message: "Codex input_image block was preserved as provider payload"
                                .to_string(),
                            path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                            raw: Some(block.clone()),
                        });
                        blocks.push(EventBlock::ProviderPayload {
                            kind: "input_image".to_string(),
                            payload: block.clone(),
                        });
                    }
                }
                "reasoning" => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: MappingDisposition::Normalized,
                        code: "codex_reasoning_preserved_as_provider_payload".to_string(),
                        message: "Codex reasoning block was preserved as provider payload instead of being exposed as user-visible thinking"
                            .to_string(),
                        path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                        raw: Some(block.clone()),
                    });
                    blocks.push(EventBlock::ProviderPayload {
                        kind: "reasoning".to_string(),
                        payload: block.clone(),
                    });
                }
                other => {
                    report.push_issue(MappingIssue {
                        level: MappingIssueLevel::Info,
                        disposition: MappingDisposition::Normalized,
                        code: "codex_unknown_block_preserved".to_string(),
                        message: format!("Preserved unknown Codex content block '{}'", other),
                        path: Some(format!("response_item:{}:block:{}", line_no, idx)),
                        raw: Some(block.clone()),
                    });
                    blocks.push(EventBlock::Unknown { raw: block.clone() });
                }
            }
        }
    } else if let Some(text) = payload.get("content").and_then(|v| v.as_str()) {
        blocks.push(EventBlock::Text {
            text: text.to_string(),
        });
    } else {
        blocks.push(EventBlock::ProviderPayload {
            kind: "message_without_content".to_string(),
            payload: payload.clone(),
        });
    }

    if blocks.is_empty() {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Warning,
            disposition: MappingDisposition::Normalized,
            code: "codex_message_without_mappable_blocks".to_string(),
            message:
                "Codex message had no mappable content blocks and was preserved as provider payload"
                    .to_string(),
            path: Some(format!("response_item:{}", line_no)),
            raw: Some(payload.clone()),
        });
        blocks.push(EventBlock::ProviderPayload {
            kind: "message_without_mappable_blocks".to_string(),
            payload: payload.clone(),
        });
    }

    if phase.as_deref() == Some("commentary") && blocks.len() == 1 {
        if let EventBlock::Text { text } = &blocks[0] {
            blocks[0] = EventBlock::Thinking {
                text: text.clone(),
                signature: None,
            };
        }
    }

    let role = match role_str {
        Some("user") => EventRole::User,
        Some("assistant") => EventRole::Assistant,
        Some("developer") => EventRole::Developer,
        Some("system") => EventRole::System,
        Some("tool") => EventRole::Tool,
        _ => EventRole::Unknown,
    };

    if let Some(internal_kind) = codex_internal_message_kind(role_str, &blocks) {
        report.push_issue(MappingIssue {
            level: MappingIssueLevel::Info,
            disposition: MappingDisposition::Normalized,
            code: internal_kind.issue_code().to_string(),
            message: internal_kind.issue_message().to_string(),
            path: Some(format!("response_item:{}", line_no)),
            raw: Some(payload.clone()),
        });
        return codex_hidden_response_item_event(
            event_id,
            timestamp,
            internal_kind,
            payload.clone(),
            raw_line,
            phase,
            role_str,
        );
    }

    SessionEvent {
        id: event_id,
        kind: SessionEventKind::Message,
        role,
        timestamp,
        links: EventLinks::default(),
        blocks,
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: role_str.map(str::to_string),
                phase,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("codex_payload".to_string(), payload.clone());
                ext.insert("codex_raw_line".to_string(), raw_line);
                ext
            },
        },
    }
}

fn codex_hidden_response_item_event(
    id: String,
    timestamp: chrono::DateTime<Utc>,
    internal_kind: CodexInternalMessageKind,
    payload: Value,
    raw_line: Value,
    phase: Option<String>,
    original_role: Option<&str>,
) -> SessionEvent {
    let mut event = provider_payload_event(
        id,
        SessionEventKind::Lifecycle,
        EventRole::System,
        timestamp,
        internal_kind.payload_kind(),
        payload,
        raw_line,
        phase,
    );
    event.metadata.fidelity = MappingDisposition::Normalized;
    event.metadata.source.original_role = original_role.map(str::to_string);
    event.metadata.provider_ext.insert(
        "codex_internal_message".to_string(),
        serde_json::json!({
            "class": internal_kind.class(),
            "payload_kind": internal_kind.payload_kind(),
        }),
    );
    event
}

fn codex_internal_message_kind(
    role_str: Option<&str>,
    blocks: &[EventBlock],
) -> Option<CodexInternalMessageKind> {
    if codex_is_turn_aborted_sentinel(role_str, blocks) {
        return Some(CodexInternalMessageKind::LifecycleSentinel);
    }
    if codex_is_internal_user_context_message(role_str, blocks) {
        return Some(CodexInternalMessageKind::RuntimeContext);
    }
    if codex_is_internal_developer_control_message(role_str, blocks) {
        return Some(CodexInternalMessageKind::ProviderControl);
    }
    None
}

fn codex_text_blocks(blocks: &[EventBlock]) -> impl Iterator<Item = &str> {
    blocks.iter().filter_map(|block| match block {
        EventBlock::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

fn codex_is_turn_aborted_sentinel(role_str: Option<&str>, blocks: &[EventBlock]) -> bool {
    if role_str != Some("user") {
        return false;
    }
    let mut text_blocks = codex_text_blocks(blocks);
    let Some(text) = text_blocks.next() else {
        return false;
    };
    text_blocks.next().is_none()
        && text.trim_start().starts_with("<turn_aborted>")
        && text.trim_end().ends_with("</turn_aborted>")
}

fn codex_is_internal_developer_control_message(
    role_str: Option<&str>,
    blocks: &[EventBlock],
) -> bool {
    if role_str != Some("developer") {
        return false;
    }
    let mut saw_text = false;
    for text in codex_text_blocks(blocks) {
        saw_text = true;
        if !CODEX_INTERNAL_DEVELOPER_TAGS
            .iter()
            .any(|tag| text.trim_start().starts_with(tag))
        {
            return false;
        }
    }
    saw_text
}

fn codex_is_internal_user_context_message(role_str: Option<&str>, blocks: &[EventBlock]) -> bool {
    if role_str != Some("user") {
        return false;
    }
    let mut saw_text = false;
    for text in codex_text_blocks(blocks) {
        saw_text = true;
        if !codex_is_internal_user_context_text(text) {
            return false;
        }
    }
    saw_text
}

fn codex_is_internal_user_context_text(text: &str) -> bool {
    let trimmed = text.trim_start();
    CODEX_INTERNAL_USER_CONTEXT_TAGS
        .iter()
        .any(|(start_tag, end_tag)| {
            trimmed.starts_with(start_tag) && text.trim_end().ends_with(end_tag)
        })
        || codex_is_agents_instructions_text(trimmed)
}

fn codex_is_agents_instructions_text(text: &str) -> bool {
    text.starts_with("# AGENTS.md instructions") && text.contains("<INSTRUCTIONS>")
}

fn codex_event_msg_event(
    payload: &Value,
    timestamp: chrono::DateTime<Utc>,
    line_no: usize,
    raw_line: Value,
) -> SessionEvent {
    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("event_msg");
    let role = match event_type {
        "user_message" => EventRole::User,
        "agent_message" => EventRole::Assistant,
        _ => EventRole::System,
    };

    let mut blocks = Vec::new();
    let message_text = payload.get("message").and_then(|v| v.as_str());
    let last_agent_text = payload.get("last_agent_message").and_then(|v| v.as_str());

    if let Some(text) = message_text {
        blocks.push(EventBlock::Text {
            text: text.to_string(),
        });
    }
    if let Some(text) = last_agent_text {
        if message_text != Some(text) && !text.trim().is_empty() {
            blocks.push(EventBlock::Text {
                text: text.to_string(),
            });
        }
    }
    blocks.push(EventBlock::ProviderPayload {
        kind: event_type.to_string(),
        payload: payload.clone(),
    });

    let mut event = provider_payload_event(
        format!("codex:event_msg:{}:{}", event_type, line_no),
        SessionEventKind::Lifecycle,
        role,
        timestamp,
        event_type,
        payload.clone(),
        raw_line,
        payload
            .get("phase")
            .and_then(|v| v.as_str())
            .map(str::to_string),
    );
    event.blocks = blocks;
    event
}

fn codex_image_block(block: &Value) -> Option<EventBlock> {
    let mime_type = block
        .get("mime_type")
        .or_else(|| block.get("mimeType"))
        .and_then(|v| v.as_str())
        .unwrap_or("image/*")
        .to_string();
    let image_url = block
        .get("image_url")
        .or_else(|| block.get("url"))
        .or_else(|| block.get("source"))
        .and_then(|v| v.as_str())?;
    if let Some((mime, data)) = parse_data_uri(image_url) {
        return Some(EventBlock::Image {
            mime_type: mime.to_string(),
            data: Some(data.to_string()),
            path: None,
        });
    }
    Some(EventBlock::Image {
        mime_type,
        data: None,
        path: Some(image_url.to_string()),
    })
}

fn parse_data_uri(uri: &str) -> Option<(&str, &str)> {
    let rest = uri.strip_prefix("data:")?;
    let (mime, data) = rest.split_once(";base64,")?;
    Some((mime, data))
}

fn provider_payload_event(
    id: String,
    kind: SessionEventKind,
    role: EventRole,
    timestamp: chrono::DateTime<Utc>,
    payload_kind: &str,
    payload: Value,
    raw_line: Value,
    phase: Option<String>,
) -> SessionEvent {
    SessionEvent {
        id,
        kind,
        role,
        timestamp,
        links: EventLinks::default(),
        blocks: vec![EventBlock::ProviderPayload {
            kind: payload_kind.to_string(),
            payload: payload.clone(),
        }],
        metadata: EventMetadata {
            source: EventSource {
                provider_id: PROVIDER_ID.to_string(),
                original_id: None,
                original_role: None,
                phase,
            },
            model: None,
            usage: None,
            fidelity: MappingDisposition::Preserved,
            provider_ext: {
                let mut ext = BTreeMap::new();
                ext.insert("codex_payload".to_string(), payload);
                ext.insert("codex_raw_line".to_string(), raw_line);
                ext
            },
        },
    }
}

#[derive(Debug, Clone, Default)]
struct CodexSqliteThreadMetadata {
    cwd: Option<String>,
    title: Option<String>,
    rollout_path: Option<String>,
}

fn build_sqlite_thread_metadata_lookup(
    codex_dir: &Path,
) -> Result<HashMap<String, CodexSqliteThreadMetadata>> {
    let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
    if !sqlite_path.exists() {
        return Ok(HashMap::new());
    }
    let conn = rusqlite::Connection::open(&sqlite_path)?;
    if !has_table(&conn, "threads")? {
        return Ok(HashMap::new());
    }

    let has_cwd = has_columns(&conn, "threads", &["cwd"])?;
    let has_title = has_columns(&conn, "threads", &["title"])?;
    let has_rollout_path = has_columns(&conn, "threads", &["rollout_path"])?;
    if !has_cwd && !has_title && !has_rollout_path {
        return Ok(HashMap::new());
    }

    let mut map = HashMap::new();
    let query = match (has_cwd, has_title, has_rollout_path) {
        (true, true, true) => "SELECT id, cwd, title, rollout_path FROM threads",
        (true, true, false) => "SELECT id, cwd, title, NULL AS rollout_path FROM threads",
        (true, false, true) => "SELECT id, cwd, NULL AS title, rollout_path FROM threads",
        (true, false, false) => "SELECT id, cwd, NULL AS title, NULL AS rollout_path FROM threads",
        (false, true, true) => "SELECT id, NULL AS cwd, title, rollout_path FROM threads",
        (false, true, false) => "SELECT id, NULL AS cwd, title, NULL AS rollout_path FROM threads",
        (false, false, true) => "SELECT id, NULL AS cwd, NULL AS title, rollout_path FROM threads",
        (false, false, false) => unreachable!(),
    };
    let mut stmt = conn.prepare(query)?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            CodexSqliteThreadMetadata {
                cwd: row.get::<_, Option<String>>(1)?,
                title: row.get::<_, Option<String>>(2)?,
                rollout_path: row.get::<_, Option<String>>(3)?,
            },
        ))
    })?;
    for row in rows {
        if let Ok((id, metadata)) = row {
            map.insert(id, metadata);
        }
    }
    Ok(map)
}

fn clean_non_empty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn select_codex_display_title(
    index_title: Option<&str>,
    sqlite_title: Option<&str>,
    rollout_title: Option<&str>,
    session_id: &str,
) -> Option<String> {
    clean_non_empty(index_title)
        .filter(|title| *title != session_id)
        .or_else(|| clean_non_empty(sqlite_title).filter(|title| *title != session_id))
        .or_else(|| clean_non_empty(rollout_title).filter(|title| *title != session_id))
        .map(str::to_string)
}

fn resolve_codex_projection_title(
    source_path: &Path,
    session_id: &str,
    rollout_title: Option<&str>,
) -> Result<Option<String>> {
    let codex_dir = source_path
        .ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some("sessions"))
        .and_then(Path::parent);
    let Some(codex_dir) = codex_dir else {
        return Ok(select_codex_display_title(
            None,
            None,
            rollout_title,
            session_id,
        ));
    };

    let index_entries = load_session_index_entries(&codex_dir.join("session_index.jsonl"))?;
    let sqlite_metadata = build_sqlite_thread_metadata_lookup(codex_dir)?;
    Ok(select_codex_display_title(
        index_entries.get(session_id).map(String::as_str),
        sqlite_metadata
            .get(session_id)
            .and_then(|metadata| metadata.title.as_deref()),
        rollout_title,
        session_id,
    ))
}

fn resolve_codex_reindex_title(
    session: &CodexRolloutSummary,
    sqlite_metadata: Option<&CodexSqliteThreadMetadata>,
    session_states: &session_state::SessionStateStore,
) -> String {
    session_state::resolve_session_state(
        session_states,
        PROVIDER_ID,
        &session.session_id,
        session.workspace_dir.as_deref(),
    )
    .display_title
    .as_deref()
    .and_then(|title| clean_non_empty(Some(title)))
    .filter(|title| *title != session.session_id)
    .or_else(|| {
        sqlite_metadata
            .and_then(|metadata| clean_non_empty(metadata.title.as_deref()))
            .filter(|title| *title != session.session_id)
    })
    .or_else(|| {
        clean_non_empty(session.title.as_deref()).filter(|title| *title != session.session_id)
    })
    .or_else(|| clean_non_empty(session.title.as_deref()))
    .unwrap_or(&session.session_id)
    .to_string()
}

#[derive(Debug, Clone)]
struct CodexRolloutSummary {
    session_id: String,
    title: Option<String>,
    workspace_dir: Option<String>,
    model_provider: Option<String>,
    original_model_provider: Option<String>,
    updated_at: Option<String>,
    has_user_event: bool,
}

fn read_codex_rollout_summary(path: &Path) -> Result<Option<CodexRolloutSummary>> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex rollout file: {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut session_id = None;
    let mut title = None;
    let mut workspace_dir = None;
    let mut model_provider = None;
    let mut updated_at = None;
    let mut has_user_event = false;

    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        if !has_user_event && rollout_value_has_user_event(&value) {
            has_user_event = true;
        }

        if let Some(timestamp) = value.get("timestamp").and_then(|value| value.as_str()) {
            updated_at = Some(timestamp.to_string());
        }

        if value.get("type").and_then(|value| value.as_str()) != Some("session_meta") {
            continue;
        }

        let Some(payload) = value.get("payload") else {
            continue;
        };
        session_id = payload
            .get("id")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(session_id);
        title = payload
            .get("title")
            .or_else(|| payload.get("thread_name"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(title);
        workspace_dir = payload
            .get("cwd")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(workspace_dir);
        model_provider = payload
            .get("model_provider")
            .and_then(|value| value.as_str())
            .map(str::to_string)
            .or(model_provider);
    }

    let Some(session_id) = session_id else {
        return Ok(None);
    };

    Ok(Some(CodexRolloutSummary {
        session_id,
        title,
        workspace_dir,
        original_model_provider: model_provider.clone(),
        model_provider,
        updated_at,
        has_user_event,
    }))
}

fn rollout_value_has_user_event(value: &Value) -> bool {
    if value.get("type").and_then(|value| value.as_str()) == Some("event_msg") {
        if value
            .get("payload")
            .and_then(|payload| payload.get("type"))
            .and_then(|value| value.as_str())
            == Some("user_message")
        {
            return true;
        }
    }

    let Some(payload) = value.get("payload") else {
        return false;
    };
    if payload.get("type").and_then(|value| value.as_str()) == Some("message") {
        if payload.get("role").and_then(|value| value.as_str()) == Some("user") {
            return true;
        }
    }

    false
}

fn load_session_index_entries(index_path: &Path) -> Result<HashMap<String, String>> {
    if !index_path.exists() {
        return Ok(HashMap::new());
    }

    let file = File::open(index_path).with_context(|| {
        format!(
            "Failed to open Codex session index: {}",
            index_path.display()
        )
    })?;
    let reader = BufReader::new(file);
    let mut entries = HashMap::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let id = value.get("id").and_then(|value| value.as_str());
        let thread_name = value
            .get("thread_name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if let Some(id) = id {
            entries.insert(id.to_string(), thread_name.to_string());
        }
    }
    Ok(entries)
}

fn append_session_index_entry(
    index_path: &Path,
    session_id: &str,
    title: &str,
    updated_at: Option<&str>,
) -> Result<()> {
    if let Some(parent) = index_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut index_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(index_path)?;
    let updated_at = updated_at
        .map(str::to_string)
        .unwrap_or_else(|| Utc::now().to_rfc3339());
    writeln!(
        index_file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "id": session_id,
            "thread_name": title,
            "updated_at": updated_at,
        }))?
    )?;
    Ok(())
}

fn update_session_index_entry(index_path: &Path, session_id: &str, new_title: &str) -> Result<()> {
    if !index_path.exists() {
        anyhow::bail!("Codex session index not found");
    }

    let content = std::fs::read_to_string(index_path)?;
    let mut new_lines = Vec::new();
    let mut found = false;

    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut value: Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => {
                new_lines.push(line.to_string());
                continue;
            }
        };
        if value.get("id").and_then(|value| value.as_str()) == Some(session_id) {
            if let Value::Object(ref mut map) = value {
                map.insert(
                    "thread_name".to_string(),
                    Value::String(new_title.to_string()),
                );
                found = true;
            }
            new_lines.push(serde_json::to_string(&value)?);
        } else {
            new_lines.push(line.to_string());
        }
    }

    if !found {
        anyhow::bail!("Codex session not found in index: {}", session_id);
    }

    std::fs::write(index_path, new_lines.join("\n") + "\n")?;
    Ok(())
}

fn rewrite_rollout_model_provider(path: &Path, model_provider: &str) -> Result<()> {
    let file = File::open(path)
        .with_context(|| format!("Failed to open Codex rollout file: {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut updated = false;

    for line in reader.lines() {
        let line = line?;
        if updated || line.trim().is_empty() {
            lines.push(line);
            continue;
        }
        let mut value: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                lines.push(line);
                continue;
            }
        };
        if value.get("type").and_then(|value| value.as_str()) == Some("session_meta") {
            if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                payload.insert(
                    "model_provider".to_string(),
                    Value::String(model_provider.to_string()),
                );
                updated = true;
                lines.push(serde_json::to_string(&value)?);
                continue;
            }
        }
        lines.push(line);
    }

    if !updated {
        anyhow::bail!(
            "Codex rollout file is missing session_meta payload: {}",
            path.display()
        );
    }

    std::fs::write(path, lines.join("\n") + "\n")
        .with_context(|| format!("Failed to write Codex rollout file: {}", path.display()))?;
    Ok(())
}

fn extract_cwd_from_session_file(id: &str) -> Option<String> {
    let path = find_session_file(id)?;
    extract_cwd_from_session_path(&path)
}

fn extract_cwd_from_session_path(path: &Path) -> Option<String> {
    let file = File::open(&path).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines().take(5) {
        let line = line.ok()?;
        let value: Value = serde_json::from_str(&line).ok()?;
        if value.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
            return value
                .get("payload")
                .and_then(|p| p.get("cwd"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
        }
    }
    None
}

fn get_codex_dir() -> PathBuf {
    #[cfg(test)]
    if let Some(path) = TEST_CODEX_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Codex dir lock")
        .clone()
    {
        return path;
    }

    dirs::home_dir()
        .map(|h| h.join(".codex"))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

#[cfg(test)]
pub(crate) fn set_test_codex_dir(path: Option<PathBuf>) {
    *TEST_CODEX_DIR
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .expect("test Codex dir lock") = path;
}

fn export_canonical_session(session: &CanonicalSession, target_dir: &Path) -> Result<String> {
    export_canonical_session_in_codex_dir(session, target_dir, &get_codex_dir())
}

fn export_canonical_session_in_codex_dir(
    session: &CanonicalSession,
    target_dir: &Path,
    codex_dir: &Path,
) -> Result<String> {
    let session_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let timestamp_str = now.format("%Y-%m-%dT%H-%M-%S").to_string();
    let filename = format!("rollout-{}-{}.jsonl", timestamp_str, session_id);
    let sessions_dir = codex_dir
        .join("sessions")
        .join(now.format("%Y").to_string())
        .join(now.format("%m").to_string())
        .join(now.format("%d").to_string());
    std::fs::create_dir_all(&sessions_dir)?;
    let file_path = sessions_dir.join(filename);
    write_canonical_codex_rollout(
        session,
        target_dir,
        codex_dir,
        &session_id,
        &file_path,
        now,
        true,
    )?;
    Ok(session_id)
}

fn write_canonical_codex_rollout(
    session: &CanonicalSession,
    target_dir: &Path,
    codex_dir: &Path,
    session_id: &str,
    file_path: &Path,
    now: chrono::DateTime<Utc>,
    update_registry: bool,
) -> Result<()> {
    let rollout_path = file_path.to_string_lossy().to_string();
    let mut file = File::create(file_path)?;
    let git_info = get_git_info(target_dir);
    let codex_version = get_codex_version_in_codex_dir(codex_dir);
    let codex_model_provider = read_codex_model_provider(codex_dir);
    let target_dir_str = target_dir.to_string_lossy().to_string();
    let title = canonical_session_title(session);
    let first_user_message = first_user_message(session);
    let has_user_event = has_user_event(session);
    let base_instructions = canonical_session_instruction_context_text(session);

    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": now.to_rfc3339(),
            "type": "session_meta",
            "payload": {
                "id": session_id,
                "timestamp": now.to_rfc3339(),
                "cwd": target_dir_str,
                "originator": "memorph-cli",
                "cli_version": codex_version,
                "source": "cli",
                "model_provider": codex_model_provider,
                "title": title,
                "base_instructions": base_instructions.as_ref().map(|text| {
                    serde_json::json!({ "text": text })
                }).unwrap_or(Value::Null),
                "git": {
                    "commit_hash": git_info.as_ref().and_then(|git| git.commit_hash.clone()).unwrap_or_default(),
                    "branch": git_info.as_ref().and_then(|git| git.branch.clone()).unwrap_or_default(),
                }
            }
        }))?
    )?;

    let turn_id = Uuid::new_v4().to_string();
    let first_ts = session
        .events
        .first()
        .map(|event| event.timestamp)
        .unwrap_or(now);
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": first_ts.to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "task_started",
                "turn_id": turn_id,
                "started_at": first_ts.timestamp(),
                "collaboration_mode_kind": "default"
            }
        }))?
    )?;
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": first_ts.to_rfc3339(),
            "type": "turn_context",
            "payload": {
                "turn_id": turn_id,
                "cwd": target_dir_str,
                "current_date": first_ts.format("%Y-%m-%d").to_string(),
                "timezone": "Asia/Shanghai"
            }
        }))?
    )?;

    let mut wrote_user_event = false;
    let mut last_agent_message = String::new();
    for event in &session.events {
        if let Some(segment) = compression::compressed_segment(event) {
            write_codex_compacted_rollout_item(&mut file, event, segment)?;
            if event.role == EventRole::Assistant {
                last_agent_message = segment.summary.to_string();
            }
            continue;
        }
        let Some(visible_role) = canonical_event_visible_message_role(event) else {
            continue;
        };
        let role = match visible_role {
            EventRole::Assistant => "assistant",
            EventRole::User | EventRole::Tool => "user",
            EventRole::System | EventRole::Developer | EventRole::Unknown => continue,
        };
        let content = canonical_event_to_codex_content(event);
        if content.is_empty() {
            continue;
        }
        let mut payload = serde_json::json!({
            "type": "message",
            "role": role,
            "content": content,
        });
        if event.role == EventRole::Assistant {
            payload["phase"] = Value::String("final_answer".to_string());
            last_agent_message = canonical_event_visible_text(event);
            writeln!(
                file,
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "timestamp": event.timestamp.to_rfc3339(),
                    "type": "event_msg",
                    "payload": {
                        "type": "agent_message",
                        "message": last_agent_message,
                        "phase": "final_answer",
                        "memory_citation": null
                    }
                }))?
            )?;
        }
        writeln!(
            file,
            "{}",
            serde_json::to_string(&serde_json::json!({
                "timestamp": event.timestamp.to_rfc3339(),
                "type": "response_item",
                "payload": payload,
            }))?
        )?;
        if visible_role == EventRole::User && !wrote_user_event {
            let user_text = canonical_event_visible_text(event);
            writeln!(
                file,
                "{}",
                serde_json::to_string(&serde_json::json!({
                    "timestamp": event.timestamp.to_rfc3339(),
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": user_text,
                        "images": [],
                        "local_images": [],
                        "text_elements": []
                    }
                }))?
            )?;
            wrote_user_event = true;
        }
    }

    let last_ts = session
        .events
        .last()
        .map(|event| event.timestamp)
        .unwrap_or(now);
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": last_ts.to_rfc3339(),
            "type": "event_msg",
            "payload": {
                "type": "task_complete",
                "turn_id": turn_id,
                "last_agent_message": last_agent_message,
                "completed_at": last_ts.timestamp(),
                "duration_ms": 1000
            }
        }))?
    )?;

    file.flush()?;
    file.sync_all()?;
    if update_registry {
        let index_path = codex_dir.join("session_index.jsonl");
        let mut index_file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&index_path)?;
        writeln!(
            index_file,
            "{}",
            serde_json::to_string(&serde_json::json!({
                "id": session_id,
                "thread_name": title,
                "updated_at": now.to_rfc3339(),
            }))?
        )?;
        update_codex_sqlite(
            codex_dir,
            session_id,
            &rollout_path,
            target_dir,
            &title,
            first_user_message.as_deref(),
            has_user_event,
            &now,
        )?;
        update_codex_global_state_file_if_exists(codex_dir, target_dir)?;
    }
    Ok(())
}

fn replace_codex_session(session_id: &str, session: &CanonicalSession) -> Result<()> {
    let codex_dir = get_codex_dir();
    let rollout_path = find_session_file(session_id)
        .with_context(|| format!("Codex session not found: {session_id}"))?;
    let target_dir = extract_cwd_from_session_path(&rollout_path)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let temp_path = rollout_path.with_extension(format!("jsonl.memorph-{}.tmp", Uuid::new_v4()));
    let result = (|| -> Result<()> {
        write_canonical_codex_rollout(
            session,
            &target_dir,
            &codex_dir,
            session_id,
            &temp_path,
            Utc::now(),
            false,
        )?;
        let imported = import_canonical_session(&temp_path)?;
        if imported.session.identity.canonical_id != session_id {
            anyhow::bail!("Codex replacement validation changed session identity");
        }
        std::fs::rename(&temp_path, &rollout_path).with_context(|| {
            format!(
                "Failed to atomically replace Codex rollout: {}",
                rollout_path.display()
            )
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn write_codex_compacted_rollout_item(
    file: &mut impl Write,
    event: &SessionEvent,
    segment: CompressedSegment<'_>,
) -> Result<()> {
    let model_visible_summary = codex_compacted_history_text(segment);
    let source_event_count = segment.source_event_count.or_else(|| {
        (!segment.source_event_ids.is_empty()).then_some(segment.source_event_ids.len())
    });
    writeln!(
        file,
        "{}",
        serde_json::to_string(&serde_json::json!({
            "timestamp": event.timestamp.to_rfc3339(),
            "type": "compacted",
            "payload": {
                "message": segment.summary,
                "replacement_history": [
                    {
                        "type": "message",
                        "role": "user",
                        "content": [
                            {
                                "type": "input_text",
                                "text": model_visible_summary,
                            }
                        ]
                    }
                ],
                "memorph": {
                    "source_provider_id": segment.source_provider_id,
                    "summary": segment.summary,
                    "source_event_ids": segment.source_event_ids,
                    "source_event_count": source_event_count,
                    "archive_ref": segment.archive_ref,
                }
            }
        }))?
    )?;
    Ok(())
}

fn codex_compacted_history_text(segment: CompressedSegment<'_>) -> String {
    let mut parts = vec![
        format!(
            "[Compressed session segment from {}]",
            segment.source_provider_id
        ),
        segment.summary.to_string(),
    ];
    let source_event_count = segment
        .source_event_count
        .unwrap_or(segment.source_event_ids.len());
    if source_event_count > 0 {
        parts.push(format!("Source event count: {}", source_event_count));
    }
    if let Some(archive_ref) = segment.archive_ref {
        parts.push(format!("Archive: {}", archive_ref));
        parts.push(compression_retrieval_hint(archive_ref));
    }
    parts.join("\n")
}

fn canonical_event_to_codex_content(event: &SessionEvent) -> Vec<Value> {
    event
        .blocks
        .iter()
        .filter_map(|block| match block {
            EventBlock::Text { text } => Some(serde_json::json!({
                "type": if event.role == EventRole::Assistant { "output_text" } else { "input_text" },
                "text": text,
            })),
            EventBlock::Thinking { text, .. } => Some(serde_json::json!({
                "type": "output_text",
                "text": format!("[Thinking]\n{}", text),
            })),
            EventBlock::Image { data: Some(data), .. } if event.role != EventRole::Assistant => {
                Some(serde_json::json!({
                    "type": "input_image",
                    "image_url": data,
                }))
            }
            EventBlock::ProviderPayload { .. } => None,
            _ => {
                let text = canonical_visible_block_text(block)?;
                (!text.trim().is_empty()).then(|| serde_json::json!({
                    "type": if event.role == EventRole::Assistant { "output_text" } else { "input_text" },
                    "text": text,
                }))
            }
        })
        .collect()
}

fn find_session_file(id: &str) -> Option<PathBuf> {
    // Search both active sessions and archived sessions
    let dirs = [
        get_codex_dir().join("sessions"),
        get_codex_dir().join("archived_sessions"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.contains(id))
                .unwrap_or(false)
            {
                return Some(path.to_path_buf());
            }
        }
    }

    None
}

fn build_session_file_lookup(
    codex_dir: &Path,
    session_ids: &[String],
) -> HashMap<String, CodexSessionFileMeta> {
    let mut lookup = HashMap::new();
    if session_ids.is_empty() {
        return lookup;
    }

    let mut remaining: HashSet<String> = session_ids.iter().cloned().collect();
    let dirs = [
        codex_dir.join("sessions"),
        codex_dir.join("archived_sessions"),
    ];

    for dir in &dirs {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir)
            .max_depth(5)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if remaining.is_empty() {
                return lookup;
            }

            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some(session_id) = remaining
                .iter()
                .find(|id| file_name.contains(id.as_str()))
                .cloned()
            else {
                continue;
            };
            let size_bytes = entry.metadata().map(|metadata| metadata.len()).unwrap_or(0);
            lookup.insert(
                session_id.clone(),
                CodexSessionFileMeta {
                    path: path.to_path_buf(),
                    size_bytes,
                },
            );
            remaining.remove(&session_id);
        }
    }

    lookup
}

#[derive(Default)]
struct GitInfo {
    commit_hash: Option<String>,
    branch: Option<String>,
}

fn get_git_info(dir: &Path) -> Option<GitInfo> {
    let mut info = GitInfo::default();

    let branch_output = std::process::Command::new("git")
        .args([
            "-C",
            &dir.to_string_lossy(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
        .ok()?;
    if branch_output.status.success() {
        info.branch = Some(
            String::from_utf8_lossy(&branch_output.stdout)
                .trim()
                .to_string(),
        );
    }

    let hash_output = std::process::Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "rev-parse", "HEAD"])
        .output()
        .ok()?;
    if hash_output.status.success() {
        info.commit_hash = Some(
            String::from_utf8_lossy(&hash_output.stdout)
                .trim()
                .to_string(),
        );
    }

    Some(info)
}

fn first_user_message(session: &CanonicalSession) -> Option<String> {
    session
        .events
        .iter()
        .filter(|event| canonical_event_visible_message_role(event) == Some(EventRole::User))
        .find_map(|event| {
            let text = canonical_event_visible_message_text(event)?;
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
}

fn has_user_event(session: &CanonicalSession) -> bool {
    session
        .events
        .iter()
        .any(|event| canonical_event_visible_message_role(event) == Some(EventRole::User))
}

fn update_codex_sqlite(
    codex_dir: &Path,
    session_id: &str,
    rollout_path: &str,
    cwd: &Path,
    title: &str,
    first_user_message: Option<&str>,
    has_user_event: bool,
    now: &chrono::DateTime<Utc>,
) -> Result<()> {
    let sqlite_path = codex_dir.join("state_5.sqlite");
    if !sqlite_path.exists() {
        // SQLite not present, skip
        return Ok(());
    }

    let conn = rusqlite::Connection::open(&sqlite_path)
        .with_context(|| format!("Failed to open Codex SQLite: {}", sqlite_path.display()))?;

    let created_at = now.timestamp();
    let created_at_ms = now.timestamp_millis();
    let cwd_str = cwd.to_string_lossy().to_string();
    let codex_version = get_codex_version_in_codex_dir(codex_dir);
    let codex_model_provider = read_codex_model_provider(codex_dir);
    let (codex_model, codex_reasoning) = get_codex_model_config_in_codex_dir(codex_dir);
    let sandbox_json = format!(
        "{{\"type\":\"workspace-write\",\"writable_roots\":[],\"network_access\":false,\"exclude_tmpdir_env_var\":false,\"exclude_slash_tmp\":false}}"
    );

    conn.execute(
        "INSERT INTO threads (
            id, rollout_path, created_at, updated_at, source, model_provider,
            cwd, title, sandbox_policy, approval_mode, tokens_used, has_user_event,
            archived, cli_version, first_user_message, memory_mode, git_branch,
            model, reasoning_effort, created_at_ms, updated_at_ms
        ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21
        ) ON CONFLICT(id) DO UPDATE SET
            updated_at = excluded.updated_at,
            updated_at_ms = excluded.updated_at_ms",
        rusqlite::params![
            session_id,
            rollout_path,
            created_at,
            created_at,
            "cli",
            codex_model_provider,
            cwd_str,
            title,
            sandbox_json,
            "on-request",
            0,
            if has_user_event { 1 } else { 0 },
            0,
            codex_version,
            first_user_message.unwrap_or(title),
            "enabled",
            get_git_branch(cwd).unwrap_or_else(|| "main".to_string()),
            codex_model,
            codex_reasoning,
            created_at_ms,
            created_at_ms,
        ],
    )
    .with_context(|| "Failed to insert thread into Codex SQLite")?;

    Ok(())
}

fn update_codex_global_state_file_if_exists(codex_dir: &Path, workspace_root: &Path) -> Result<()> {
    let global_state_path = codex_dir.join(".codex-global-state.json");
    if !global_state_path.exists() {
        return Ok(());
    }
    update_codex_global_state_file(&global_state_path, workspace_root)
}

fn update_codex_global_state_file(global_state_path: &Path, workspace_root: &Path) -> Result<()> {
    let content = std::fs::read_to_string(global_state_path).with_context(|| {
        format!(
            "Failed to read Codex global state: {}",
            global_state_path.display()
        )
    })?;
    let mut value: Value = serde_json::from_str(&content).with_context(|| {
        format!(
            "Failed to parse Codex global state: {}",
            global_state_path.display()
        )
    })?;
    let workspace = workspace_root.to_string_lossy().to_string();
    ensure_unique_string_array_entry(&mut value, "electron-saved-workspace-roots", &workspace);
    ensure_unique_string_array_entry(&mut value, "project-order", &workspace);
    let serialized = serde_json::to_string(&value)?;
    std::fs::write(global_state_path, serialized).with_context(|| {
        format!(
            "Failed to write Codex global state: {}",
            global_state_path.display()
        )
    })?;
    Ok(())
}

fn ensure_unique_string_array_entry(value: &mut Value, key: &str, entry: &str) {
    let Some(map) = value.as_object_mut() else {
        return;
    };
    let field = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Some(items) = field.as_array_mut() else {
        return;
    };
    if items.iter().any(|item| item.as_str() == Some(entry)) {
        return;
    }
    items.push(Value::String(entry.to_string()));
}

fn get_codex_version_in_codex_dir(codex_dir: &Path) -> String {
    let version_path = codex_dir.join("version.json");
    if let Ok(content) = std::fs::read_to_string(&version_path) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(ver) = v.get("latest_version").and_then(|v| v.as_str()) {
                return ver.to_string();
            }
        }
    }
    "0.124.0".to_string()
}

fn update_rollout_session_meta_title(path: &Path, new_title: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut new_lines = Vec::new();
    let mut updated = false;

    for line in content.lines() {
        if !updated && !line.trim().is_empty() {
            if let Ok(mut value) = serde_json::from_str::<Value>(line) {
                if value.get("type").and_then(|v| v.as_str()) == Some("session_meta") {
                    if let Some(payload) = value.get_mut("payload").and_then(Value::as_object_mut) {
                        payload.insert("title".to_string(), Value::String(new_title.to_string()));
                        new_lines.push(serde_json::to_string(&value)?);
                        updated = true;
                        continue;
                    }
                }
            }
        }
        new_lines.push(line.to_string());
    }

    if updated {
        std::fs::write(path, new_lines.join("\n") + "\n")?;
    }

    Ok(())
}

fn has_table(conn: &Connection, table: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .is_ok())
}

fn has_columns(conn: &Connection, table: &str, columns: &[&str]) -> Result<bool> {
    let existing: HashSet<String> = table_columns(conn, table)?.into_iter().collect();
    Ok(columns.iter().all(|column| existing.contains(*column)))
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "\"\"")
    ))?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn delete_related_rows(
    conn: &Connection,
    table: &str,
    where_clause: &str,
    session_id: &str,
) -> Result<()> {
    if has_table(conn, table)? {
        conn.execute(
            &format!("DELETE FROM \"{table}\" WHERE {where_clause}"),
            [session_id],
        )?;
    }
    Ok(())
}

fn read_codex_model_provider(codex_dir: &Path) -> String {
    let config_path = codex_dir.join("config.toml");
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("model_provider") && !trimmed.starts_with("model_providers") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    return val.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    "openai".to_string()
}

fn get_codex_model_config_in_codex_dir(codex_dir: &Path) -> (String, String) {
    let config_path = codex_dir.join("config.toml");
    let mut model = String::new();
    let mut reasoning = String::new();
    if let Ok(content) = std::fs::read_to_string(&config_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if model.is_empty() && trimmed.starts_with("model ") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    model = val.trim().trim_matches('"').to_string();
                }
            }
            if reasoning.is_empty() && trimmed.starts_with("model_reasoning_effort") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    reasoning = val.trim().trim_matches('"').to_string();
                }
            }
        }
    }
    if model.is_empty() {
        model = "gpt-5.3-codex".to_string();
    }
    if reasoning.is_empty() {
        reasoning = "xhigh".to_string();
    }
    (model, reasoning)
}

fn get_git_branch(dir: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &dir.to_string_lossy(),
            "rev-parse",
            "--abbrev-ref",
            "HEAD",
        ])
        .output()
        .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::active_compression::{
        apply_active_compression_with_archive_dir, ActiveCompressionApplyParams,
        ActiveCompressionMode, ActiveCompressionPolicy,
    };
    use crate::core::session_management;
    use crate::storage::artifact_store::{ArtifactManifestKind, ArtifactStorageKind};
    use serde_json::json;
    use tempfile::{tempdir, NamedTempFile};

    static TEST_CODEX_TEST_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    struct TestCodexDirGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for TestCodexDirGuard {
        fn drop(&mut self) {
            crate::cache::global_cache().invalidate(PROVIDER_ID);
            set_test_codex_mutation_failure(None);
            set_test_codex_dir(None);
        }
    }

    fn use_test_codex_dir(path: PathBuf) -> TestCodexDirGuard {
        let lock = TEST_CODEX_TEST_LOCK
            .get_or_init(|| std::sync::Mutex::new(()))
            .lock()
            .expect("test Codex serial lock");
        set_test_codex_dir(Some(path));
        crate::cache::global_cache().invalidate(PROVIDER_ID);
        TestCodexDirGuard { _lock: lock }
    }

    struct NativeCodexFixture {
        index_path: PathBuf,
        rollout_path: PathBuf,
        original_index_bytes: Vec<u8>,
        original_rollout_bytes: Vec<u8>,
    }

    fn write_native_codex_fixture(codex_dir: &Path, session_id: &str) -> NativeCodexFixture {
        let sessions_dir = codex_dir.join("sessions/2026/07/09");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let index_path = codex_dir.join("session_index.jsonl");
        let rollout_path =
            sessions_dir.join(format!("rollout-2026-07-09T12-00-00-{session_id}.jsonl"));
        let original_index_bytes = format!(
            "{{\"id\":\"{session_id}\",\"thread_name\":\"Before\",\"updated_at\":\"2026-07-09T12:00:00Z\"}}\n\n{{\"id\":\"session-other\",\"thread_name\":\"Other\",\"updated_at\":\"2026-07-09T13:00:00Z\"}}\n"
        )
        .into_bytes();
        let original_rollout_bytes = [
            serde_json::to_string(&json!({
                "timestamp": "2026-07-09T12:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": session_id,
                    "timestamp": "2026-07-09T12:00:00Z",
                    "cwd": "/tmp/project",
                    "title": "Before"
                }
            }))
            .unwrap(),
            serde_json::to_string(&json!({
                "timestamp": "2026-07-09T12:01:00Z",
                "type": "event_msg",
                "payload": {
                    "type": "user_message",
                    "message": "hello"
                }
            }))
            .unwrap(),
        ]
        .join("\n")
        .into_bytes();
        let mut original_rollout_bytes = original_rollout_bytes;
        original_rollout_bytes.push(b'\n');
        std::fs::write(&index_path, &original_index_bytes).unwrap();
        std::fs::write(&rollout_path, &original_rollout_bytes).unwrap();

        let conn = Connection::open(codex_dir.join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
        conn.execute_batch(
            "
            CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                cwd TEXT NOT NULL,
                updated_at INTEGER NOT NULL,
                unrelated_note TEXT NOT NULL
            );
            CREATE TABLE thread_dynamic_tools (
                thread_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                tool_name TEXT NOT NULL,
                payload BLOB,
                PRIMARY KEY (thread_id, position)
            );
            CREATE TABLE thread_goals (
                thread_id TEXT NOT NULL,
                goal_id TEXT NOT NULL,
                objective TEXT NOT NULL,
                PRIMARY KEY (thread_id, goal_id)
            );
            CREATE TABLE thread_spawn_edges (
                parent_thread_id TEXT NOT NULL,
                child_thread_id TEXT NOT NULL PRIMARY KEY,
                status TEXT NOT NULL
            );
            CREATE TABLE stage1_outputs (
                thread_id TEXT NOT NULL,
                output_id TEXT NOT NULL,
                output TEXT NOT NULL,
                PRIMARY KEY (thread_id, output_id)
            );
            CREATE TABLE agent_job_items (
                job_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                assigned_thread_id TEXT,
                payload TEXT NOT NULL,
                status TEXT NOT NULL,
                PRIMARY KEY (job_id, item_id)
            );
            ",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, title, cwd, updated_at, unrelated_note)
             VALUES (?1, 'Before', '/tmp/project', 100, 'preserve target columns')",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, title, cwd, updated_at, unrelated_note)
             VALUES ('session-other', 'Other', '/tmp/other', 200, 'unrelated thread')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO thread_dynamic_tools (thread_id, position, tool_name, payload)
             VALUES (?1, 0, 'shell', ?2)",
            rusqlite::params![session_id, vec![0_u8, 1, 127, 128, 255]],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO thread_goals (thread_id, goal_id, objective)
             VALUES (?1, 'goal-1', 'finish exact restore')",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status)
             VALUES (?1, 'session-other', 'completed')",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO stage1_outputs (thread_id, output_id, output)
             VALUES (?1, 'output-1', 'captured output')",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_job_items (
                job_id, item_id, assigned_thread_id, payload, status
             ) VALUES ('job-1', 'item-1', ?1, 'keep payload', 'running')",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_job_items (
                job_id, item_id, assigned_thread_id, payload, status
             ) VALUES ('job-2', 'item-2', 'session-other', 'other payload', 'queued')",
            [],
        )
        .unwrap();

        NativeCodexFixture {
            index_path,
            rollout_path,
            original_index_bytes,
            original_rollout_bytes,
        }
    }

    fn codex_session_row_counts(codex_dir: &Path, session_id: &str) -> Vec<i64> {
        let conn = Connection::open(codex_dir.join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
        [
            ("threads", "id = ?1"),
            ("thread_dynamic_tools", "thread_id = ?1"),
            ("thread_goals", "thread_id = ?1"),
            (
                "thread_spawn_edges",
                "parent_thread_id = ?1 OR child_thread_id = ?1",
            ),
            ("stage1_outputs", "thread_id = ?1"),
            ("agent_job_items", "assigned_thread_id = ?1"),
        ]
        .into_iter()
        .map(|(table, where_clause)| {
            conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {where_clause}"),
                [session_id],
                |row| row.get(0),
            )
            .unwrap()
        })
        .collect()
    }

    fn test_sync_context(codex_dir: &Path, workspace: &Path) -> (Connection, PathBuf, String) {
        let mut conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&conn).unwrap();
        local_store::apply_schema(&mut conn).unwrap();
        let activity_id = ActivityStore::new(&conn)
            .start(NewActivity {
                provider_id: Some(PROVIDER_ID.to_string()),
                provider_session_id: None,
                workspace_dir: Some(workspace.to_string_lossy().to_string()),
                operation_kind: ActivityOperationKind::Sync,
                actor: ActivityActor::System,
                summary: "Synchronizing Codex workspace sessions".to_string(),
                details: serde_json::json!({}),
            })
            .unwrap();
        let backup_root = codex_dir
            .parent()
            .unwrap()
            .join("memorph-artifacts")
            .join("backups")
            .join("codex-sync");
        (conn, backup_root, activity_id)
    }

    fn run_test_workspace_sync(
        codex_dir: &Path,
        workspace: &Path,
        keep_backups: usize,
    ) -> (CodexWorkspaceRepairReport, Connection, PathBuf, String) {
        let (mut conn, backup_root, activity_id) = test_sync_context(codex_dir, workspace);
        let report = sync_workspace_sessions_in_codex_home(
            &mut conn,
            &activity_id,
            &backup_root,
            codex_dir,
            Some(workspace.to_str().unwrap()),
            keep_backups,
        )
        .unwrap();
        (report, conn, backup_root, activity_id)
    }

    #[test]
    fn delete_backup_restores_exact_codex_files_and_database_rows() {
        let codex_dir = tempdir().unwrap();
        let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
        let session_id = "session-delete-backup";
        let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
        let backup = create_codex_session_backup(
            ProviderSourceMutation::Delete,
            "operation-delete-1",
            session_id,
            &codex_dir.path().join("backups"),
        )
        .unwrap();

        delete_codex_session(session_id).unwrap();
        assert!(!fixture.rollout_path.exists());
        assert!(!std::fs::read(&fixture.index_path)
            .unwrap()
            .windows(session_id.len())
            .any(|window| window == session_id.as_bytes()));
        assert_eq!(
            codex_session_row_counts(codex_dir.path(), session_id),
            vec![0, 0, 0, 0, 0, 0]
        );

        restore_codex_session_backup(&backup).unwrap();

        assert_eq!(
            std::fs::read(&fixture.index_path).unwrap(),
            fixture.original_index_bytes
        );
        assert_eq!(
            std::fs::read(&fixture.rollout_path).unwrap(),
            fixture.original_rollout_bytes
        );
        assert_eq!(
            codex_session_row_counts(codex_dir.path(), session_id),
            vec![1, 1, 1, 1, 1, 1]
        );
        let conn = Connection::open(codex_dir.path().join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
        let payload: Vec<u8> = conn
            .query_row(
                "SELECT payload FROM thread_dynamic_tools
                 WHERE thread_id = ?1 AND position = 0",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(payload, vec![0_u8, 1, 127, 128, 255]);
        let job: (Option<String>, String, String) = conn
            .query_row(
                "SELECT assigned_thread_id, payload, status
                 FROM agent_job_items
                 WHERE job_id = 'job-1' AND item_id = 'item-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            job,
            (
                Some(session_id.to_string()),
                "keep payload".to_string(),
                "running".to_string()
            )
        );

        let metadata: CodexSessionBackupMetadata = serde_json::from_slice(
            &std::fs::read(backup.backup_path.join("metadata.json")).unwrap(),
        )
        .unwrap();
        let tables = metadata
            .sqlite_tables
            .iter()
            .map(|manifest| manifest.table.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(
            tables,
            HashSet::from([
                "threads",
                "thread_dynamic_tools",
                "thread_goals",
                "thread_spawn_edges",
                "stage1_outputs",
                "agent_job_items",
            ])
        );
        assert_eq!(
            metadata
                .sqlite_tables
                .iter()
                .find(|manifest| manifest.table == "agent_job_items")
                .unwrap()
                .columns,
            vec!["job_id", "item_id", "assigned_thread_id"]
        );
    }

    #[test]
    fn native_replace_preserves_codex_session_identity_and_path() {
        let codex_dir = tempdir().unwrap();
        let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
        let session_id = "session-native-replace";
        let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
        let original_index = std::fs::read(&fixture.index_path).unwrap();
        let mut session = import_canonical_session(&fixture.rollout_path)
            .unwrap()
            .session;
        session.events.clear();

        CodexProvider.replace_session(session_id, &session).unwrap();

        assert!(fixture.rollout_path.exists());
        assert_eq!(std::fs::read(&fixture.index_path).unwrap(), original_index);
        assert_eq!(
            import_canonical_session(&fixture.rollout_path)
                .unwrap()
                .session
                .identity
                .canonical_id,
            session_id
        );
        assert_eq!(
            codex_session_row_counts(codex_dir.path(), session_id),
            vec![1, 1, 1, 1, 1, 1]
        );
    }

    #[test]
    fn replace_backup_restores_exact_codex_source() {
        let codex_dir = tempdir().unwrap();
        let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
        let session_id = "session-replace-backup";
        let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
        let backup = create_codex_session_backup(
            ProviderSourceMutation::Replace,
            "operation-replace-1",
            session_id,
            &codex_dir.path().join("backups"),
        )
        .unwrap();

        delete_codex_session(session_id).unwrap();
        restore_codex_session_backup(&backup).unwrap();

        assert_eq!(
            std::fs::read(&fixture.index_path).unwrap(),
            fixture.original_index_bytes
        );
        assert_eq!(
            std::fs::read(&fixture.rollout_path).unwrap(),
            fixture.original_rollout_bytes
        );
        assert_eq!(
            codex_session_row_counts(codex_dir.path(), session_id),
            vec![1, 1, 1, 1, 1, 1]
        );
    }

    #[test]
    fn rename_backup_restores_only_codex_title_owned_state() {
        let codex_dir = tempdir().unwrap();
        let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
        let session_id = "session-rename-backup";
        let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
        let backup = create_codex_session_backup(
            ProviderSourceMutation::Rename,
            "operation-rename-1",
            session_id,
            &codex_dir.path().join("backups"),
        )
        .unwrap();

        rename_codex_session(session_id, "After").unwrap();
        let conn = Connection::open(codex_dir.path().join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
        conn.execute(
            "UPDATE threads
             SET cwd = '/tmp/changed', updated_at = 999, unrelated_note = 'changed independently'
             WHERE id = ?1",
            [session_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE agent_job_items
             SET payload = 'changed job payload', status = 'completed'
             WHERE job_id = 'job-1' AND item_id = 'item-1'",
            [],
        )
        .unwrap();
        drop(conn);

        restore_codex_session_backup(&backup).unwrap();

        assert_eq!(
            std::fs::read(&fixture.index_path).unwrap(),
            fixture.original_index_bytes
        );
        assert_eq!(
            std::fs::read(&fixture.rollout_path).unwrap(),
            fixture.original_rollout_bytes
        );
        let conn = Connection::open(codex_dir.path().join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
        let thread: (String, String, i64, String) = conn
            .query_row(
                "SELECT title, cwd, updated_at, unrelated_note
                 FROM threads
                 WHERE id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            thread,
            (
                "Before".to_string(),
                "/tmp/changed".to_string(),
                999,
                "changed independently".to_string()
            )
        );
        let job: (String, String) = conn
            .query_row(
                "SELECT payload, status
                 FROM agent_job_items
                 WHERE job_id = 'job-1' AND item_id = 'item-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            job,
            ("changed job payload".to_string(), "completed".to_string())
        );

        let metadata: CodexSessionBackupMetadata = serde_json::from_slice(
            &std::fs::read(backup.backup_path.join("metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata.sqlite_tables.len(), 1);
        assert_eq!(metadata.sqlite_tables[0].table, "threads");
        assert_eq!(metadata.sqlite_tables[0].columns, vec!["id", "title"]);
        assert_eq!(
            metadata.sqlite_tables[0].restore_mode,
            CodexSqliteRestoreMode::ThreadTitle
        );
    }

    #[test]
    fn codex_backup_contract_and_capabilities_are_truthful() {
        let codex_dir = tempdir().unwrap();
        let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
        let session_id = "session-backup-contract";
        write_native_codex_fixture(codex_dir.path(), session_id);
        let backup = create_codex_session_backup(
            ProviderSourceMutation::Delete,
            "operation-contract-1",
            session_id,
            &codex_dir.path().join("backups"),
        )
        .unwrap();

        let capabilities = CodexProvider.capabilities();
        assert!(capabilities.backup_support.before_write);
        assert!(capabilities.backup_support.restore);
        assert!(!capabilities.backup_support.sync_only);
        assert_eq!(backup.mutation, ProviderSourceMutation::Delete);
        assert_eq!(backup.operation_id, "operation-contract-1");
        assert_eq!(backup.provider_session_id, session_id);
        assert_eq!(backup.source_path, codex_dir.path().canonicalize().unwrap());
        assert_eq!(backup.format, CODEX_SESSION_BACKUP_FORMAT);
        assert_eq!(backup.mime_type, CODEX_SESSION_BACKUP_MIME);
        assert_eq!(
            backup
                .restore_metadata
                .get("restore_mode")
                .and_then(Value::as_str),
            Some("codex_session_restore")
        );
    }

    #[test]
    fn backup_registration_failure_prevents_codex_provider_write() {
        let codex_dir = tempdir().unwrap();
        let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
        let session_id = "session-registration-failure";
        let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
        let backup_root = codex_dir.path().join("backups");
        let mut artifact_conn = Connection::open_in_memory().unwrap();

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-registration-failure".to_string()],
            &backup_root,
            &mut artifact_conn,
        );

        let error = results.into_iter().next().unwrap().unwrap_err();
        assert!(error
            .to_string()
            .contains("Delete cancelled before provider write"));
        assert_eq!(
            std::fs::read(&fixture.index_path).unwrap(),
            fixture.original_index_bytes
        );
        assert_eq!(
            std::fs::read(&fixture.rollout_path).unwrap(),
            fixture.original_rollout_bytes
        );
        assert_eq!(
            codex_session_row_counts(codex_dir.path(), session_id),
            vec![1, 1, 1, 1, 1, 1]
        );
        assert!(backup_root
            .join(PROVIDER_ID)
            .join("operation-registration-failure")
            .exists());
    }

    #[test]
    fn partial_codex_delete_failure_restores_registered_backup() {
        let codex_dir = tempdir().unwrap();
        let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
        let session_id = "session-partial-delete";
        let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
        let backup_root = codex_dir.path().join("backups");
        let mut artifact_conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();
        set_test_codex_mutation_failure(Some(ProviderSourceMutation::Delete));

        let results = session_management::delete_sessions(
            PROVIDER_ID,
            &[session_id],
            &["operation-partial-delete".to_string()],
            &backup_root,
            &mut artifact_conn,
        );

        let error = results.into_iter().next().unwrap().unwrap_err();
        assert!(error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            std::fs::read(&fixture.index_path).unwrap(),
            fixture.original_index_bytes
        );
        assert_eq!(
            std::fs::read(&fixture.rollout_path).unwrap(),
            fixture.original_rollout_bytes
        );
        assert_eq!(
            codex_session_row_counts(codex_dir.path(), session_id),
            vec![1, 1, 1, 1, 1, 1]
        );
    }

    #[test]
    fn partial_codex_rename_failure_restores_registered_backup() {
        let codex_dir = tempdir().unwrap();
        let _guard = use_test_codex_dir(codex_dir.path().to_path_buf());
        let session_id = "session-partial-rename";
        let fixture = write_native_codex_fixture(codex_dir.path(), session_id);
        let backup_root = codex_dir.path().join("backups");
        let mut artifact_conn = Connection::open_in_memory().unwrap();
        local_store::configure_connection(&artifact_conn).unwrap();
        local_store::apply_schema(&mut artifact_conn).unwrap();
        set_test_codex_mutation_failure(Some(ProviderSourceMutation::Rename));

        let error = session_management::rename_session(
            PROVIDER_ID,
            session_id,
            "After",
            "operation-partial-rename",
            &backup_root,
            &mut artifact_conn,
        )
        .unwrap_err();

        assert!(error
            .to_string()
            .contains("Provider source was restored from registered backup"));
        assert_eq!(
            std::fs::read(&fixture.index_path).unwrap(),
            fixture.original_index_bytes
        );
        assert_eq!(
            std::fs::read(&fixture.rollout_path).unwrap(),
            fixture.original_rollout_bytes
        );
        let conn = Connection::open(codex_dir.path().join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
        let title: String = conn
            .query_row(
                "SELECT title FROM threads WHERE id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(title, "Before");
    }

    #[test]
    fn project_session_title_uses_codex_native_precedence_and_prompt_fallback() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let sessions_dir = codex_dir.join("sessions/2026/07/15");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let session_id = "codex-title-precedence";
        let source_path = sessions_dir.join(format!("rollout-{session_id}.jsonl"));
        let write_rollout = |title: &str| {
            std::fs::write(
                &source_path,
                [
                    serde_json::to_string(&json!({
                        "timestamp": "2026-07-15T10:00:00Z",
                        "type": "session_meta",
                        "payload": {
                            "id": session_id,
                            "cwd": "/tmp/project",
                            "title": title
                        }
                    }))
                    .unwrap(),
                    serde_json::to_string(&json!({
                        "timestamp": "2026-07-15T10:00:01Z",
                        "type": "response_item",
                        "payload": {
                            "type": "message",
                            "role": "user",
                            "content": [
                                {
                                    "type": "input_text",
                                    "text": "Prompt title"
                                }
                            ]
                        }
                    }))
                    .unwrap(),
                ]
                .join("\n")
                    + "\n",
            )
            .unwrap();
        };
        let write_index = |title: &str| {
            std::fs::write(
                codex_dir.join("session_index.jsonl"),
                serde_json::to_string(&json!({
                    "id": session_id,
                    "thread_name": title,
                    "updated_at": "2026-07-15T10:00:01Z"
                }))
                .unwrap()
                    + "\n",
            )
            .unwrap();
        };

        write_rollout("Rollout title");
        write_index("Index title");
        let sqlite = Connection::open(codex_dir.join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
        sqlite
            .execute("CREATE TABLE threads (id TEXT PRIMARY KEY, title TEXT)", [])
            .unwrap();
        sqlite
            .execute(
                "INSERT INTO threads (id, title) VALUES (?1, ?2)",
                rusqlite::params![session_id, "SQLite title"],
            )
            .unwrap();

        let imported_title = || -> String {
            let imported = CodexProvider
                .import_session(source_path.to_string_lossy().as_ref())
                .unwrap();
            canonical_session_title(&imported.session)
        };

        assert_eq!(imported_title(), "Index title");

        write_index(session_id);
        assert_eq!(imported_title(), "SQLite title");

        sqlite
            .execute(
                "UPDATE threads SET title = ?1 WHERE id = ?2",
                rusqlite::params![session_id, session_id],
            )
            .unwrap();
        assert_eq!(imported_title(), "Rollout title");

        write_rollout(session_id);
        assert_eq!(imported_title(), "Prompt title");
    }

    #[test]
    fn codex_import_and_event_index_use_stable_source_order_for_missing_timestamps() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "type": "session_meta",
                "payload": {
                    "id": "session-stable",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "not-a-timestamp",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": "Build this" }]
                }
            })
        )
        .unwrap();
        file.flush().unwrap();

        let first = import_canonical_session(file.path()).unwrap();
        let second = import_canonical_session(file.path()).unwrap();
        let fingerprint = event_index::source_file_fingerprint(file.path()).unwrap();
        let (first_index, first_locations) =
            build_codex_event_index(file.path(), fingerprint).unwrap();
        let (second_index, second_locations) =
            build_codex_event_index(file.path(), fingerprint).unwrap();

        assert_eq!(
            serde_json::to_value(&first.session.events).unwrap(),
            serde_json::to_value(&second.session.events).unwrap()
        );
        assert_eq!(first.session.events[0].timestamp.timestamp_millis(), 1);
        assert_eq!(first.session.events[1].timestamp.timestamp_millis(), 2);
        assert_eq!(first_index, second_index);
        assert_eq!(first_locations, second_locations);
        assert_eq!(first_index.last_active_at_ms, Some(2));
    }

    #[test]
    fn import_canonical_session_preserves_codex_runtime_and_message_events() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-1",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project",
                    "base_instructions": { "text": "Be careful." },
                    "model": "gpt-5.3-codex"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "turn_context",
                "payload": {
                    "turn_id": "turn-1",
                    "cwd": "/tmp/project",
                    "current_date": "2026-05-21",
                    "timezone": "Asia/Shanghai"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_started",
                    "turn_id": "turn-1",
                    "started_at": 1747821602
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:03Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "developer",
                    "content": [
                        { "type": "input_text", "text": "# AGENTS.md instructions" }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:04Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "phase": "commentary",
                    "content": [
                        { "type": "output_text", "text": "Thinking out loud" }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:05Z",
                "type": "response_item",
                "payload": {
                    "type": "function_call",
                    "name": "shell",
                    "call_id": "call_1",
                    "arguments": "{\"cmd\":\"echo hello\"}"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:05Z",
                "type": "response_item",
                "payload": {
                    "type": "function_call_output",
                    "call_id": "call_1",
                    "output": "hello"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:06Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "Done."
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let events = &imported.session.events;

        assert_eq!(imported.session.identity.canonical_id, "session-1");
        assert_eq!(
            imported.session.context.workspace_dir.as_deref(),
            Some("/tmp/project")
        );
        assert!(events.iter().any(|event| {
            event.role == EventRole::System
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "Be careful."
                )
        }));
        assert!(events.iter().any(|event| {
            event.role == EventRole::Developer
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "# AGENTS.md instructions"
                )
        }));
        assert!(events.iter().any(|event| {
            event.role == EventRole::Assistant
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Thinking { text, .. }) if text == "Thinking out loud"
                )
        }));
        assert!(events.iter().any(|event| {
            event.id == "codex:response_item:6"
                && event.role == EventRole::Assistant
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ToolCall { name, tool_call_id, .. })
                        if name == "shell" && tool_call_id == "call_1"
                )
        }));
        assert!(events.iter().any(|event| {
            event.id == "codex:response_item:7"
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ToolResult { content, tool_call_id, .. })
                        if content == "hello" && tool_call_id == "call_1"
                )
        }));
        assert!(events.iter().any(|event| {
            event.id == "codex:event_msg:task_complete:8"
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::Text { text }) if text == "Done."
                )
        }));
        let started = events
            .iter()
            .find(|event| event.id == "codex:event_msg:task_started:3")
            .unwrap();
        let completed = events
            .iter()
            .find(|event| event.id == "codex:event_msg:task_complete:8")
            .unwrap();
        assert_eq!(started.links.provider_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(started.links.turn_index, Some(0));
        assert_eq!(started.links.turn_boundary, Some(TurnBoundary::Started));
        assert_eq!(completed.links.provider_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(completed.links.turn_index, Some(0));
        assert_eq!(completed.links.turn_boundary, Some(TurnBoundary::Completed));
        assert!(events
            .iter()
            .filter(|event| event.id != "codex:base_instructions:1")
            .all(|event| event.links.provider_turn_id.as_deref() == Some("turn-1")));
    }

    #[test]
    fn paged_import_preserves_native_turn_context_when_page_starts_mid_turn() {
        assert_eq!(
            CodexProvider.capabilities().page_strategy,
            PageStrategy::IndexedPage
        );
        let home = tempdir().unwrap();
        crate::config::set_test_home_dir(home.path().to_path_buf());
        let mut file = NamedTempFile::new().unwrap();
        for line in [
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {"id": "paged-turn", "cwd": "/tmp/project"}
            }),
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "turn_context",
                "payload": {"turn_id": "turn-page", "cwd": "/tmp/project"}
            }),
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "event_msg",
                "payload": {"type": "task_started", "turn_id": "turn-page"}
            }),
            json!({
                "timestamp": "2026-05-21T10:00:03Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Working"}]
                }
            }),
            json!({
                "timestamp": "2026-05-21T10:00:04Z",
                "type": "event_msg",
                "payload": {"type": "task_complete", "turn_id": "turn-page"}
            }),
        ] {
            writeln!(file, "{}", line).unwrap();
        }

        let page = import_canonical_session_page(file.path(), 3, Some(1)).unwrap();
        crate::config::reset_test_home_dir();

        assert_eq!(page.imported.session.events.len(), 1);
        assert_eq!(page.turn_count, None);
        let event = &page.imported.session.events[0];
        assert_eq!(event.links.provider_turn_id.as_deref(), Some("turn-page"));
        assert_eq!(event.links.turn_index, Some(0));
        assert_eq!(event.links.turn_boundary, None);
    }

    #[test]
    fn import_canonical_session_hides_turn_aborted_and_internal_developer_controls() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-hidden-controls",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00.500Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "# AGENTS.md instructions for /tmp/project\n\n<INSTRUCTIONS>\nBe careful.\n</INSTRUCTIONS>"
                        },
                        {
                            "type": "input_text",
                            "text": "<environment_context>\n  <cwd>/tmp/project</cwd>\n  <shell>zsh</shell>\n  <current_date>2026-05-21</current_date>\n  <timezone>Asia/Shanghai</timezone>\n</environment_context>"
                        }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00.750Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "<codex_internal_context source=\"goal\">\nContinue working toward the active thread goal.\n</codex_internal_context>"
                        }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "<turn_aborted>\nInterrupted.\n</turn_aborted>"
                        }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "event_msg",
                "payload": {
                    "type": "turn_aborted",
                    "turn_id": "turn-1",
                    "reason": "interrupted"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:03Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "developer",
                    "content": [
                        {
                            "type": "input_text",
                            "text": "<model_switch>\nSwitch context\n</model_switch>"
                        },
                        {
                            "type": "input_text",
                            "text": "<collaboration_mode># Collaboration Mode: Default\n</collaboration_mode>"
                        }
                    ]
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let events = &imported.session.events;

        assert!(!events.iter().any(|event| {
            event.role == EventRole::User
                && event
                    .blocks
                    .iter()
                    .any(|block| matches!(block, EventBlock::Text { text } if text.contains("<turn_aborted>")))
        }));
        assert!(!events.iter().any(|event| {
            event.role == EventRole::Developer
                && event
                    .blocks
                    .iter()
                    .any(|block| matches!(block, EventBlock::Text { text } if text.contains("<model_switch>")))
        }));
        assert!(!events.iter().any(|event| {
            event.role == EventRole::User
                && event.blocks.iter().any(|block| {
                    matches!(block, EventBlock::Text { text } if text.contains("<environment_context>") || text.contains("# AGENTS.md instructions") || text.contains("<codex_internal_context"))
                })
        }));
        assert!(events.iter().any(|event| {
            event.kind == SessionEventKind::Lifecycle
                && event.role == EventRole::System
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ProviderPayload { kind, .. }) if kind == "turn_aborted_sentinel"
                )
                && event
                    .metadata
                    .provider_ext
                    .get("codex_internal_message")
                    .and_then(|value| value.get("class"))
                    .and_then(Value::as_str)
                    == Some("lifecycle_sentinel")
        }));
        assert!(events.iter().any(|event| {
            event.kind == SessionEventKind::Lifecycle
                && event.role == EventRole::System
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ProviderPayload { kind, .. }) if kind == "developer_control_message"
                )
                && event
                    .metadata
                    .provider_ext
                    .get("codex_internal_message")
                    .and_then(|value| value.get("class"))
                    .and_then(Value::as_str)
                    == Some("provider_control")
        }));
        assert!(events.iter().any(|event| {
            event.kind == SessionEventKind::Lifecycle
                && event.role == EventRole::System
                && matches!(
                    event.blocks.first(),
                    Some(EventBlock::ProviderPayload { kind, .. }) if kind == "user_context_message"
                )
                && event
                    .metadata
                    .provider_ext
                    .get("codex_internal_message")
                    .and_then(|value| value.get("class"))
                    .and_then(Value::as_str)
                    == Some("runtime_context")
        }));
        let aborted = events
            .iter()
            .find(|event| event.id == "codex:event_msg:turn_aborted:5")
            .unwrap();
        assert_eq!(aborted.links.provider_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(aborted.links.turn_boundary, Some(TurnBoundary::Interrupted));
    }

    #[test]
    fn import_canonical_session_decodes_input_image_data_uri() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-2",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "user",
                    "content": [
                        {
                            "type": "input_image",
                            "mime_type": "image/png",
                            "image_url": "data:image/png;base64,QUJD"
                        }
                    ]
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let image_block = imported
            .session
            .events
            .iter()
            .flat_map(|event| event.blocks.iter())
            .find_map(|block| match block {
                EventBlock::Image {
                    mime_type,
                    data,
                    path,
                } => Some((mime_type, data, path)),
                _ => None,
            })
            .expect("expected image block");

        assert_eq!(image_block.0, "image/png");
        assert_eq!(image_block.1.as_deref(), Some("QUJD"));
        assert_eq!(image_block.2, &None);
    }

    #[test]
    fn codex_response_blocks_preserve_reasoning_and_json_tool_output() {
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let output = codex_response_item_event(
            &json!({
                "type": "function_call_output",
                "call_id": "call-1",
                "output": {"status": "ok", "items": [1, 2]}
            }),
            Utc::now(),
            1,
            json!({}),
            &mut report,
        );
        assert!(matches!(
            output.blocks.as_slice(),
            [EventBlock::ToolResult { content, .. }]
                if content == r#"{"items":[1,2],"status":"ok"}"#
        ));

        let reasoning = codex_response_item_event(
            &json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "reasoning", "summary": "internal"}]
            }),
            Utc::now(),
            2,
            json!({}),
            &mut report,
        );
        assert!(matches!(
            reasoning.blocks.as_slice(),
            [EventBlock::ProviderPayload { kind, payload }]
                if kind == "reasoning" && payload == &json!({"type": "reasoning", "summary": "internal"})
        ));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "codex_reasoning_preserved_as_provider_payload"));
    }

    #[test]
    fn codex_text_block_without_text_is_not_silently_dropped() {
        let mut report = MappingReport::new(PROVIDER_ID, MappingDirection::Import);
        let event = codex_response_item_event(
            &json!({
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text"}]
            }),
            Utc::now(),
            3,
            json!({}),
            &mut report,
        );

        assert!(matches!(
            event.blocks.as_slice(),
            [EventBlock::ProviderPayload { kind, .. }]
                if kind == "output_text"
        ));
        assert!(report
            .issues
            .iter()
            .any(|issue| issue.code == "codex_text_block_missing_text"));
    }

    #[test]
    fn import_canonical_session_maps_native_compacted_to_compressed_block() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-compacted",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "compacted",
                "payload": {
                    "message": "compressed summary",
                    "replacement_history": [
                        {
                            "type": "message",
                            "role": "user",
                            "content": [
                                {
                                    "type": "input_text",
                                    "text": "[Compressed session segment from claude]\ncompressed summary\nSource event count: 2\nArchive: memorph-archive://s1/archive.json.gz"
                                }
                            ]
                        }
                    ],
                    "memorph": {
                        "source_provider_id": "claude",
                        "summary": "compressed summary",
                        "source_event_ids": ["old-event-1", "old-event-2"],
                        "source_event_count": 2,
                        "archive_ref": "memorph-archive://s1/archive.json.gz"
                    }
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let compressed = imported
            .session
            .events
            .iter()
            .find_map(|event| {
                event.blocks.iter().find_map(|block| match block {
                    EventBlock::Compressed {
                        source_provider_id,
                        summary,
                        source_event_ids,
                        source_event_count,
                        archive_ref,
                    } => Some((
                        source_provider_id,
                        summary,
                        source_event_ids,
                        source_event_count,
                        archive_ref,
                    )),
                    _ => None,
                })
            })
            .expect("expected compressed block");

        assert_eq!(compressed.0, "claude");
        assert_eq!(compressed.1, "compressed summary");
        assert_eq!(
            compressed.2,
            &vec!["old-event-1".to_string(), "old-event-2".to_string()]
        );
        assert_eq!(*compressed.3, Some(2));
        assert_eq!(
            compressed.4.as_deref(),
            Some("memorph-archive://s1/archive.json.gz")
        );
    }

    #[test]
    fn compressed_segment_exports_as_native_codex_compacted_rollout() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"test-provider\"\n",
        )
        .unwrap();

        let session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "session-native-compact".to_string(),
                source_title: Some("Native Compact".to_string()),
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: None,
                primary_source: ProviderSessionRef {
                    provider_id: "claude".to_string(),
                    session_id: "session-native-compact".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: Some(workspace.to_string_lossy().to_string()),
                created_at: None,
                last_active_at: None,
                tags: Vec::new(),
            },
            events: vec![
                SessionEvent {
                    id: "compressed-source".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::Assistant,
                    timestamp: Utc::now(),
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Compressed {
                        source_provider_id: "claude".to_string(),
                        summary: "compressed summary".to_string(),
                        source_event_ids: vec![
                            "old-event-1".to_string(),
                            "old-event-2".to_string(),
                            "old-event-3".to_string(),
                        ],
                        source_event_count: None,
                        archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: "memorph".to_string(),
                            original_id: None,
                            original_role: Some("assistant".to_string()),
                            phase: Some("compression".to_string()),
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Normalized,
                        provider_ext: BTreeMap::new(),
                    },
                },
                SessionEvent {
                    id: "tail-user".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: Utc::now(),
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "latest request".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: "memorph".to_string(),
                            original_id: None,
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };

        let session_id =
            export_canonical_session_in_codex_dir(&session, &workspace, &codex_dir).unwrap();
        let rollout_path = WalkDir::new(codex_dir.join("sessions"))
            .into_iter()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.into_path())
            .find(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("jsonl")
                    && path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.contains(&session_id))
            })
            .expect("exported rollout");
        let lines = std::fs::read_to_string(&rollout_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        let compacted = lines
            .iter()
            .find(|line| line.get("type").and_then(Value::as_str) == Some("compacted"))
            .expect("native compacted line");
        let payload = compacted.get("payload").expect("compacted payload");
        assert_eq!(
            payload.get("message").and_then(Value::as_str),
            Some("compressed summary")
        );
        assert_eq!(
            payload
                .pointer("/memorph/source_provider_id")
                .and_then(Value::as_str),
            Some("claude")
        );
        assert_eq!(
            payload
                .pointer("/memorph/source_event_count")
                .and_then(Value::as_u64),
            Some(3)
        );
        let model_visible_text = payload
            .pointer("/replacement_history/0/content/0/text")
            .and_then(Value::as_str)
            .expect("replacement history text");
        assert!(model_visible_text.contains("[Compressed session segment from claude]"));
        assert!(model_visible_text.contains("compressed summary"));
        assert!(model_visible_text.contains("Source event count: 3"));
        assert!(model_visible_text.contains("Archive: memorph-archive://s1/archive.json.gz"));
        assert!(model_visible_text.contains("memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"));
        assert!(!model_visible_text.contains("old-event-1"));

        let compressed_response_item = lines.iter().any(|line| {
            line.get("type").and_then(Value::as_str) == Some("response_item")
                && line
                    .to_string()
                    .contains("[Compressed session segment from claude]")
        });
        assert!(!compressed_response_item);
        assert!(lines.iter().any(|line| {
            line.get("type").and_then(Value::as_str) == Some("response_item")
                && line.to_string().contains("latest request")
        }));
    }

    #[test]
    fn active_compression_export_round_trips_as_native_codex_compacted_rollout() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        let archive_dir = temp.path().join("archives");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"test-provider\"\n",
        )
        .unwrap();

        let now = Utc::now();
        let mut source_session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "active-to-codex".to_string(),
                source_title: Some("Active to Codex".to_string()),
            },
            provenance: SessionProvenance {
                imported_at: now,
                imported_by: None,
                primary_source: ProviderSessionRef {
                    provider_id: "claude".to_string(),
                    session_id: "active-to-codex".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: Some(workspace.to_string_lossy().to_string()),
                created_at: None,
                last_active_at: None,
                tags: Vec::new(),
            },
            events: Vec::new(),
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };
        source_session.events.push(SessionEvent {
            id: "old-user".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::User,
            timestamp: now,
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: "historical context that should be archived ".repeat(80),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "claude".to_string(),
                    original_id: Some("old-user".to_string()),
                    original_role: Some("user".to_string()),
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: BTreeMap::new(),
            },
        });
        source_session.events.push(SessionEvent {
            id: "recent-user".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::User,
            timestamp: now,
            links: EventLinks::default(),
            blocks: vec![EventBlock::Text {
                text: "latest request".to_string(),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "claude".to_string(),
                    original_id: Some("recent-user".to_string()),
                    original_role: Some("user".to_string()),
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: BTreeMap::new(),
            },
        });

        let applied = apply_active_compression_with_archive_dir(
            &source_session,
            ActiveCompressionApplyParams {
                source_provider_id: "claude".to_string(),
                target_provider_id: "codex".to_string(),
                policy: ActiveCompressionPolicy {
                    protect_recent_message_events: 1,
                    min_candidate_bytes: 16,
                    min_savings_ratio_percent: 20,
                    mode: ActiveCompressionMode::Auto,
                },
                candidate_ids: Vec::new(),
            },
            &archive_dir,
        )
        .unwrap();
        assert_eq!(applied.report.archive_refs.len(), 1);
        let archive_ref = applied.report.archive_refs[0].clone();

        let session_id =
            export_canonical_session_in_codex_dir(&applied.session, &workspace, &codex_dir)
                .unwrap();
        let rollout_path = WalkDir::new(codex_dir.join("sessions"))
            .into_iter()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.into_path())
            .find(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("jsonl")
                    && path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.contains(&session_id))
            })
            .expect("exported rollout");
        let lines = std::fs::read_to_string(&rollout_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .collect::<Vec<_>>();

        let compacted = lines
            .iter()
            .find(|line| line.get("type").and_then(Value::as_str) == Some("compacted"))
            .expect("native compacted line");
        let payload = compacted.get("payload").expect("compacted payload");
        assert_eq!(
            payload
                .pointer("/memorph/source_provider_id")
                .and_then(Value::as_str),
            Some("claude")
        );
        assert_eq!(
            payload
                .pointer("/memorph/source_event_ids/0")
                .and_then(Value::as_str),
            Some("old-user")
        );
        assert_eq!(
            payload
                .pointer("/memorph/archive_ref")
                .and_then(Value::as_str),
            Some(archive_ref.as_str())
        );
        let model_visible_text = payload
            .pointer("/replacement_history/0/content/0/text")
            .and_then(Value::as_str)
            .expect("replacement history text");
        assert!(model_visible_text.contains("[Compressed session segment from claude]"));
        assert!(model_visible_text.contains(&format!("Archive: {archive_ref}")));
        assert!(model_visible_text.contains(&format!(
            "memorph compression retrieve {archive_ref} --query <terms> --max-results 5"
        )));

        let old_source_response_item = lines.iter().any(|line| {
            line.get("type").and_then(Value::as_str) == Some("response_item")
                && line
                    .to_string()
                    .contains("historical context that should be archived")
        });
        assert!(!old_source_response_item);

        let imported = import_canonical_session(&rollout_path).unwrap();
        let imported_compressed = imported
            .session
            .events
            .iter()
            .find_map(|event| {
                event.blocks.iter().find_map(|block| match block {
                    EventBlock::Compressed {
                        source_provider_id,
                        source_event_ids,
                        archive_ref,
                        ..
                    } => Some((source_provider_id, source_event_ids, archive_ref)),
                    _ => None,
                })
            })
            .expect("imported compressed block");
        assert_eq!(imported_compressed.0, "claude");
        assert_eq!(imported_compressed.1, &vec!["old-user".to_string()]);
        assert_eq!(imported_compressed.2.as_deref(), Some(archive_ref.as_str()));
    }

    #[test]
    fn compressed_segment_content_fallback_stays_portable_for_non_native_paths() {
        let event = SessionEvent {
            id: "compressed-source".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![EventBlock::Compressed {
                source_provider_id: "opencode".to_string(),
                summary: "compressed summary".to_string(),
                source_event_ids: vec![
                    "old-event-1".to_string(),
                    "old-event-2".to_string(),
                    "old-event-3".to_string(),
                ],
                source_event_count: None,
                archive_ref: Some("memorph-archive://s1/archive.json.gz".to_string()),
            }],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "memorph".to_string(),
                    original_id: None,
                    original_role: Some("assistant".to_string()),
                    phase: Some("compression".to_string()),
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Normalized,
                provider_ext: BTreeMap::new(),
            },
        };

        let content = canonical_event_to_codex_content(&event);

        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0].get("type").and_then(Value::as_str),
            Some("output_text")
        );
        let text = content[0]
            .get("text")
            .and_then(Value::as_str)
            .expect("portable compressed text");
        assert!(text.contains("[Compressed session segment from opencode]"));
        assert!(text.contains("compressed summary"));
        assert!(text.contains("Source event count: 3"));
        assert!(text.contains("Archive: memorph-archive://s1/archive.json.gz"));
        assert!(text.contains("memorph compression retrieve memorph-archive://s1/archive.json.gz --query <terms> --max-results 5"));
        assert!(!text.contains("old-event-1"));
        assert!(!text.contains("old-event-2"));
        assert!(!text.contains("old-event-3"));
    }

    #[test]
    fn first_user_message_skips_empty_user_events_but_has_user_event_stays_true() {
        let session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "session-3".to_string(),
                source_title: None,
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: None,
                primary_source: ProviderSessionRef {
                    provider_id: PROVIDER_ID.to_string(),
                    session_id: "session-3".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: None,
                created_at: None,
                last_active_at: None,
                tags: Vec::new(),
            },
            events: vec![
                SessionEvent {
                    id: "user-empty".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: Utc::now(),
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "   ".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: PROVIDER_ID.to_string(),
                            original_id: None,
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
                SessionEvent {
                    id: "user-real".to_string(),
                    kind: SessionEventKind::Message,
                    role: EventRole::User,
                    timestamp: Utc::now(),
                    links: EventLinks::default(),
                    blocks: vec![EventBlock::Text {
                        text: "real prompt".to_string(),
                    }],
                    metadata: EventMetadata {
                        source: EventSource {
                            provider_id: PROVIDER_ID.to_string(),
                            original_id: None,
                            original_role: Some("user".to_string()),
                            phase: None,
                        },
                        model: None,
                        usage: None,
                        fidelity: MappingDisposition::Preserved,
                        provider_ext: BTreeMap::new(),
                    },
                },
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };

        assert!(has_user_event(&session));
        assert_eq!(first_user_message(&session).as_deref(), Some("real prompt"));
    }

    #[test]
    fn update_codex_global_state_file_remembers_workspace_without_switching_active_root() {
        let dir = tempdir().unwrap();
        let path = dir.path().join(".codex-global-state.json");
        let workspace_a = "/tmp/a";
        let workspace_b = "/tmp/b";
        std::fs::write(
            &path,
            serde_json::to_string(&json!({
                "electron-saved-workspace-roots": [workspace_a],
                "active-workspace-roots": [workspace_a],
                "project-order": [workspace_a],
            }))
            .unwrap(),
        )
        .unwrap();

        update_codex_global_state_file(&path, Path::new(workspace_b)).unwrap();

        let updated: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            updated["electron-saved-workspace-roots"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![workspace_a, workspace_b]
        );
        assert_eq!(
            updated["project-order"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![workspace_a, workspace_b]
        );
        assert_eq!(
            updated["active-workspace-roots"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>(),
            vec![workspace_a]
        );
    }

    #[test]
    fn sync_workspace_sessions_registers_prewrite_backup_with_activity_identity() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        let sessions_dir = codex_dir.join("sessions/2026/05/27");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"custom-provider\"\n",
        )
        .unwrap();
        std::fs::write(
            codex_dir.join(".codex-global-state.json"),
            serde_json::to_string(&json!({
                "electron-saved-workspace-roots": [],
                "project-order": [],
                "active-workspace-roots": [],
            }))
            .unwrap(),
        )
        .unwrap();

        let session_path = sessions_dir.join("rollout-2026-05-27T12-00-00-session-1.jsonl");
        std::fs::write(
            &session_path,
            [
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-27T12:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": "session-1",
                        "timestamp": "2026-05-27T12:00:00Z",
                        "cwd": workspace.to_string_lossy(),
                        "model_provider": "openai",
                        "title": "Repair me"
                    }
                }))
                .unwrap(),
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-27T12:05:00Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": "hello"
                    }
                }))
                .unwrap(),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        let (report, mut activity_conn, backup_root, activity_id) =
            run_test_workspace_sync(&codex_dir, &workspace, DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT);

        assert_eq!(report.current_model_provider, "custom-provider");
        assert_eq!(report.workspace_session_count, 1);
        assert_eq!(report.hidden_session_count, 1);
        assert_eq!(report.repaired_session_count, 1);
        assert_eq!(report.reindexed_session_count, 1);
        assert_eq!(report.sqlite_rows_updated, 0);
        assert!(report.backup_dir.is_some());
        assert_eq!(report.touched_sessions.len(), 1);
        assert_eq!(
            report.touched_sessions[0]
                .previous_model_provider
                .as_deref(),
            Some("openai")
        );

        let backup_id = report.backup_id.as_deref().unwrap();
        let backup = ArtifactStore::new(&mut activity_conn)
            .get_backup(backup_id)
            .unwrap()
            .unwrap();
        let canonical_codex_dir = codex_dir.canonicalize().unwrap();
        let canonical_workspace = workspace.canonicalize().unwrap();
        let canonical_backup_root = backup_root.canonicalize().unwrap();
        assert_eq!(backup.operation_id.as_deref(), Some(activity_id.as_str()));
        assert_eq!(
            backup.artifact.operation_id.as_deref(),
            Some(activity_id.as_str())
        );
        assert_eq!(
            backup.artifact.artifact_kind,
            ArtifactManifestKind::SessionBackup
        );
        assert_eq!(backup.artifact.storage_kind, ArtifactStorageKind::Directory);
        assert!(backup.artifact.content_hash.starts_with("sha256-tree-v1:"));
        assert_eq!(
            backup.source_path.as_deref(),
            Some(canonical_codex_dir.as_path())
        );
        assert!(backup.artifact.path.starts_with(&canonical_backup_root));
        assert_eq!(
            backup.artifact.mime_type.as_deref(),
            Some("application/vnd.memorph.codex-sync-backup")
        );
        assert_eq!(
            backup.artifact.format.as_deref(),
            Some("codex-sync-backup-v1")
        );
        assert_eq!(
            backup.artifact.metadata,
            json!({
                "role": "codex_prewrite_sync_backup",
                "workspace_dir": canonical_workspace.to_string_lossy(),
                "target_provider": "custom-provider",
                "provider_session_ids": ["session-1"],
            })
        );
        assert_eq!(
            backup.metadata,
            json!({
                "restore_mode": "codex_sync_restore",
                "metadata_file": "metadata.json",
            })
        );
        assert_eq!(
            report.backup_artifact_id.as_deref(),
            Some(backup.artifact.id.as_str())
        );
        assert_eq!(report.backup_id.as_deref(), Some(backup.id.as_str()));

        let updated_rollout = std::fs::read_to_string(&session_path).unwrap();
        assert!(updated_rollout.contains("\"model_provider\":\"custom-provider\""));

        let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
        assert!(index.contains("\"id\":\"session-1\""));

        let global_state: Value = serde_json::from_str(
            &std::fs::read_to_string(codex_dir.join(".codex-global-state.json")).unwrap(),
        )
        .unwrap();
        let saved = global_state["electron-saved-workspace-roots"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>();
        assert_eq!(saved, vec![canonical_workspace.to_string_lossy().as_ref()]);
    }

    #[test]
    fn codex_sync_backup_registration_conflict_keeps_backup_and_source_unchanged() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        let sessions_dir = codex_dir.join("sessions/2026/05/27");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let session_path = sessions_dir.join("rollout-2026-05-27T12-00-00-session-1.jsonl");
        let original_rollout = serde_json::to_string(&json!({
            "timestamp": "2026-05-27T12:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "session-1",
                "timestamp": "2026-05-27T12:00:00Z",
                "cwd": workspace.to_string_lossy(),
                "model_provider": "openai",
                "title": "Unchanged"
            }
        }))
        .unwrap()
            + "\n";
        std::fs::write(&session_path, &original_rollout).unwrap();

        let (mut activity_conn, backup_root, activity_id) =
            test_sync_context(&codex_dir, &workspace);
        let canonical_workspace = workspace.canonicalize().unwrap();
        let backup_dir = create_codex_sync_backup(
            &backup_root,
            &activity_id,
            &codex_dir,
            canonical_workspace.to_string_lossy().as_ref(),
            "custom-provider",
            std::slice::from_ref(&session_path),
        )
        .unwrap();
        register_codex_sync_backup(
            &mut activity_conn,
            &activity_id,
            &codex_dir,
            &backup_dir,
            canonical_workspace.to_string_lossy().as_ref(),
            "custom-provider",
            &["different-session".to_string()],
        )
        .unwrap();

        let error = register_codex_sync_backup(
            &mut activity_conn,
            &activity_id,
            &codex_dir,
            &backup_dir,
            canonical_workspace.to_string_lossy().as_ref(),
            "custom-provider",
            &["session-1".to_string()],
        )
        .unwrap_err();

        assert!(format!("{error:#}")
            .contains("Artifact path was already registered with conflicting context"));
        assert!(backup_dir.exists());
        assert_eq!(
            std::fs::read_to_string(&session_path).unwrap(),
            original_rollout
        );
    }

    #[test]
    fn sync_workspace_sessions_reindexes_with_sqlite_title_when_rollout_has_none() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        let sessions_dir = codex_dir.join("sessions/2026/05/27");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"custom-provider\"\n",
        )
        .unwrap();

        let session_path =
            sessions_dir.join("rollout-2026-05-27T12-00-00-sqlite-title-session.jsonl");
        std::fs::write(
            &session_path,
            [
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-27T12:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": "sqlite-title-session",
                        "timestamp": "2026-05-27T12:00:00Z",
                        "cwd": workspace.to_string_lossy(),
                        "model_provider": "openai"
                    }
                }))
                .unwrap(),
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-27T12:05:00Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": "hello"
                    }
                }))
                .unwrap(),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
        let conn = Connection::open(&sqlite_path).unwrap();
        conn.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                cwd TEXT,
                has_user_event INTEGER,
                title TEXT
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, model_provider, cwd, has_user_event, title) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "sqlite-title-session",
                "openai",
                workspace.to_string_lossy().to_string(),
                0,
                "SQLite title"
            ],
        )
        .unwrap();

        let (report, _, _, _) =
            run_test_workspace_sync(&codex_dir, &workspace, DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT);

        assert_eq!(report.reindexed_session_count, 1);
        assert_eq!(
            report.touched_sessions[0].title.as_deref(),
            Some("SQLite title")
        );
        let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
        assert!(index.contains("\"id\":\"sqlite-title-session\""));
        assert!(index.contains("\"thread_name\":\"SQLite title\""));
        assert!(!index.contains("\"thread_name\":\"sqlite-title-session\""));
    }

    #[test]
    fn sync_workspace_sessions_updates_archived_rollouts_sqlite_and_prunes_backups() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        let sessions_dir = codex_dir.join("sessions/2026/05/27");
        let archived_dir = codex_dir.join("archived_sessions/2026/05/20");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&archived_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"custom-provider\"\n",
        )
        .unwrap();
        std::fs::write(
            codex_dir.join(CODEX_GLOBAL_STATE_FILE_BASENAME),
            serde_json::to_string(&json!({
                "electron-saved-workspace-roots": [],
                "project-order": [],
                "active-workspace-roots": [],
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            codex_dir.join("session_index.jsonl"),
            serde_json::to_string(&json!({
                "id": "session-active",
                "thread_name": "Existing index",
                "updated_at": "2026-05-27T12:05:00Z",
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();

        let active_path = sessions_dir.join("rollout-2026-05-27T12-00-00-session-active.jsonl");
        std::fs::write(
            &active_path,
            [
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-27T12:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": "session-active",
                        "timestamp": "2026-05-27T12:00:00Z",
                        "cwd": workspace.to_string_lossy(),
                        "model_provider": "openai",
                        "title": "Active hidden"
                    }
                }))
                .unwrap(),
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-27T12:05:00Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": "hello"
                    }
                }))
                .unwrap(),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        let archived_path = archived_dir.join("rollout-2026-05-20T08-00-00-session-archived.jsonl");
        std::fs::write(
            &archived_path,
            [
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-20T08:00:00Z",
                    "type": "session_meta",
                    "payload": {
                        "id": "session-archived",
                        "timestamp": "2026-05-20T08:00:00Z",
                        "cwd": workspace.to_string_lossy(),
                        "model_provider": "openai",
                        "title": "Archived hidden"
                    }
                }))
                .unwrap(),
                serde_json::to_string(&json!({
                    "timestamp": "2026-05-20T08:01:00Z",
                    "type": "response_item",
                    "payload": {
                        "type": "message",
                        "role": "user",
                        "content": [
                            { "type": "input_text", "text": "need sync" }
                        ]
                    }
                }))
                .unwrap(),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
        let conn = Connection::open(&sqlite_path).unwrap();
        conn.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                cwd TEXT,
                has_user_event INTEGER
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, model_provider, cwd, has_user_event) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["session-active", "openai", "/tmp/other", 0],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, model_provider, cwd, has_user_event) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params!["session-archived", "openai", "/tmp/other", 0],
        )
        .unwrap();

        let (mut activity_conn, backup_root, current_activity_id) =
            test_sync_context(&codex_dir, &workspace);
        let stale_activity_id = ActivityStore::new(&activity_conn)
            .start(NewActivity {
                provider_id: Some(PROVIDER_ID.to_string()),
                provider_session_id: None,
                workspace_dir: Some(workspace.to_string_lossy().to_string()),
                operation_kind: ActivityOperationKind::Sync,
                actor: ActivityActor::System,
                summary: "Previous Codex workspace sync".to_string(),
                details: serde_json::json!({}),
            })
            .unwrap();
        let canonical_workspace = workspace.canonicalize().unwrap();
        let stale_backup_dir = create_codex_sync_backup(
            &backup_root,
            &stale_activity_id,
            &codex_dir,
            canonical_workspace.to_string_lossy().as_ref(),
            "openai",
            &[],
        )
        .unwrap();
        let stale_backup = register_codex_sync_backup(
            &mut activity_conn,
            &stale_activity_id,
            &codex_dir,
            &stale_backup_dir,
            canonical_workspace.to_string_lossy().as_ref(),
            "openai",
            &[],
        )
        .unwrap();
        let report = sync_workspace_sessions_in_codex_home(
            &mut activity_conn,
            &current_activity_id,
            &backup_root,
            &codex_dir,
            Some(workspace.to_str().unwrap()),
            1,
        )
        .unwrap();

        assert_eq!(report.scanned_rollouts, 2);
        assert_eq!(report.workspace_session_count, 2);
        assert_eq!(report.hidden_session_count, 2);
        assert_eq!(report.repaired_session_count, 2);
        assert_eq!(report.reindexed_session_count, 1);
        assert_eq!(report.sqlite_provider_rows_updated, 2);
        assert_eq!(report.sqlite_user_event_rows_updated, 2);
        assert_eq!(report.sqlite_cwd_rows_updated, 2);
        assert_eq!(report.sqlite_rows_updated, 6);
        assert_eq!(report.pruned_backup_count, 1);
        assert!(report.skipped_rollout_files.is_empty());

        let backup_dir = PathBuf::from(report.backup_dir.clone().unwrap());
        assert!(backup_dir.exists());
        assert!(!stale_backup_dir.exists());
        assert!(ArtifactStore::new(&mut activity_conn)
            .get_backup(&stale_backup.id)
            .unwrap()
            .is_none());
        assert!(ArtifactStore::new(&mut activity_conn)
            .get(&stale_backup.artifact.id)
            .unwrap()
            .is_none());
        assert!(ArtifactStore::new(&mut activity_conn)
            .get_backup(report.backup_id.as_deref().unwrap())
            .unwrap()
            .is_some());

        let active_rollout = std::fs::read_to_string(&active_path).unwrap();
        assert!(active_rollout.contains("\"model_provider\":\"custom-provider\""));
        let archived_rollout = std::fs::read_to_string(&archived_path).unwrap();
        assert!(archived_rollout.contains("\"model_provider\":\"custom-provider\""));

        let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
        assert!(index.contains("\"id\":\"session-active\""));
        assert!(index.contains("\"id\":\"session-archived\""));

        let verify_conn = Connection::open(&sqlite_path).unwrap();
        let mut stmt = verify_conn
            .prepare("SELECT model_provider, cwd, has_user_event FROM threads WHERE id = ?1")
            .unwrap();
        let active_row = stmt
            .query_row(rusqlite::params!["session-active"], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .unwrap();
        assert_eq!(active_row.0, "custom-provider");
        assert_eq!(active_row.1, workspace.to_string_lossy().to_string());
        assert_eq!(active_row.2, 1);
        let archived_row = stmt
            .query_row(rusqlite::params!["session-archived"], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
            .unwrap();
        assert_eq!(archived_row.0, "custom-provider");
        assert_eq!(archived_row.1, workspace.to_string_lossy().to_string());
        assert_eq!(archived_row.2, 1);

        let backup_entries = std::fs::read_dir(&backup_root)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .collect::<Vec<_>>();
        assert_eq!(backup_entries.len(), 1);
    }

    #[test]
    fn sync_workspace_sessions_fixes_index_title_equal_to_session_id() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        let sessions_dir = codex_dir.join("sessions/2026/06/08");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "model_provider = \"custom-provider\"\n",
        )
        .unwrap();

        let session_id = "019ea6e7-session-id-as-title";
        let session_path =
            sessions_dir.join(format!("rollout-2026-06-08T19-03-56-{}.jsonl", session_id));
        std::fs::write(
            &session_path,
            [
                serde_json::to_string(&json!({
                    "timestamp": "2026-06-08T19:03:56Z",
                    "type": "session_meta",
                    "payload": {
                        "id": session_id,
                        "timestamp": "2026-06-08T19:03:56Z",
                        "cwd": workspace.to_string_lossy(),
                        "model_provider": "custom-provider"
                    }
                }))
                .unwrap(),
                serde_json::to_string(&json!({
                    "timestamp": "2026-06-08T19:04:00Z",
                    "type": "event_msg",
                    "payload": {
                        "type": "user_message",
                        "message": "hello"
                    }
                }))
                .unwrap(),
            ]
            .join("\n")
                + "\n",
        )
        .unwrap();

        std::fs::write(
            codex_dir.join("session_index.jsonl"),
            serde_json::to_string(&json!({
                "id": session_id,
                "thread_name": session_id,
                "updated_at": "2026-06-08T19:04:01Z",
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();

        let sqlite_path = codex_dir.join(CODEX_SQLITE_FILE_BASENAME);
        let conn = Connection::open(&sqlite_path).unwrap();
        conn.execute(
            "CREATE TABLE threads (
                id TEXT PRIMARY KEY,
                model_provider TEXT,
                cwd TEXT,
                has_user_event INTEGER,
                title TEXT
            )",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, model_provider, cwd, has_user_event, title) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                session_id,
                "custom-provider",
                workspace.to_string_lossy().to_string(),
                1,
                "Real Title"
            ],
        )
        .unwrap();

        let (report, _, _, _) =
            run_test_workspace_sync(&codex_dir, &workspace, DEFAULT_CODEX_SYNC_BACKUP_KEEP_COUNT);

        assert_eq!(report.workspace_session_count, 1);
        assert_eq!(report.repaired_session_count, 0);
        assert_eq!(report.reindexed_session_count, 0);
        assert_eq!(report.retitled_session_count, 1);
        assert_eq!(report.touched_sessions.len(), 1);
        assert!(report.touched_sessions[0].updated_index_title);
        assert!(!report.touched_sessions[0].added_to_index);

        let index = std::fs::read_to_string(codex_dir.join("session_index.jsonl")).unwrap();
        assert!(index.contains("\"id\":\"019ea6e7-session-id-as-title\""));
        assert!(index.contains("\"thread_name\":\"Real Title\""));
        assert!(!index.contains("\"thread_name\":\"019ea6e7-session-id-as-title\""));
    }

    #[test]
    fn import_canonical_session_drops_token_count() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-tc",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": [
                        { "type": "output_text", "text": "Hello" }
                    ]
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "response_item",
                "payload": {
                    "type": "token_count",
                    "info": {
                        "input_tokens": 100,
                        "output_tokens": 50
                    }
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        // session_meta + message = 2 events; token_count is dropped
        assert_eq!(imported.session.events.len(), 2);
        assert!(!imported.session.events.iter().any(|event| {
            event.blocks.iter().any(|block| matches!(block, EventBlock::ProviderPayload { kind, .. } if kind == "token_count"))
        }));
    }

    #[test]
    fn import_canonical_session_dedupes_last_agent_message() {
        let mut file = NamedTempFile::new().unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:00Z",
                "type": "session_meta",
                "payload": {
                    "id": "session-dedup",
                    "timestamp": "2026-05-21T10:00:00Z",
                    "cwd": "/tmp/project"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:01Z",
                "type": "event_msg",
                "payload": {
                    "type": "agent_message",
                    "message": "Same text",
                    "last_agent_message": "Same text",
                    "phase": "final_answer"
                }
            })
        )
        .unwrap();
        writeln!(
            file,
            "{}",
            json!({
                "timestamp": "2026-05-21T10:00:02Z",
                "type": "event_msg",
                "payload": {
                    "type": "task_complete",
                    "turn_id": "turn-1",
                    "last_agent_message": "Different text"
                }
            })
        )
        .unwrap();

        let imported = import_canonical_session(file.path()).unwrap();
        let events: Vec<_> = imported.session.events.iter().collect();

        let agent_msg = events
            .iter()
            .find(|e| e.id == "codex:event_msg:agent_message:2")
            .unwrap();
        let text_blocks: Vec<_> = agent_msg
            .blocks
            .iter()
            .filter_map(|b| match b {
                EventBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_blocks, vec!["Same text"]);

        let complete_msg = events
            .iter()
            .find(|e| e.id == "codex:event_msg:task_complete:3")
            .unwrap();
        let text_blocks: Vec<_> = complete_msg
            .blocks
            .iter()
            .filter_map(|b| match b {
                EventBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text_blocks, vec!["Different text"]);
    }

    #[test]
    fn provider_payload_block_is_skipped_in_codex_export() {
        let event = SessionEvent {
            id: "test".to_string(),
            kind: SessionEventKind::Message,
            role: EventRole::Assistant,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks: vec![
                EventBlock::Text {
                    text: "Hello".to_string(),
                },
                EventBlock::ProviderPayload {
                    kind: "task_complete".to_string(),
                    payload: serde_json::json!({"type": "task_complete"}),
                },
            ],
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: "codex".to_string(),
                    original_id: None,
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: BTreeMap::new(),
            },
        };

        let content = canonical_event_to_codex_content(&event);
        assert_eq!(content.len(), 1);
        assert_eq!(
            content[0].get("text").and_then(Value::as_str),
            Some("Hello")
        );
    }

    #[test]
    fn codex_base_instructions_use_instruction_context_not_lifecycle() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        let workspace = temp.path().join("repo");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(codex_dir.join("config.toml"), "model_provider = \"test\"\n").unwrap();

        let session = CanonicalSession {
            schema: CanonicalSchema::default(),
            identity: SessionIdentity {
                canonical_id: "session-base-instructions".to_string(),
                source_title: Some("Base Instructions".to_string()),
            },
            provenance: SessionProvenance {
                imported_at: Utc::now(),
                imported_by: None,
                primary_source: ProviderSessionRef {
                    provider_id: "claude".to_string(),
                    session_id: "session-base-instructions".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: SessionContext {
                workspace_dir: Some(workspace.to_string_lossy().to_string()),
                created_at: None,
                last_active_at: None,
                tags: Vec::new(),
            },
            events: vec![
                codex_test_event(
                    "system",
                    SessionEventKind::Message,
                    EventRole::System,
                    vec![EventBlock::Text {
                        text: "system instructions".to_string(),
                    }],
                ),
                codex_test_event(
                    "runtime",
                    SessionEventKind::Lifecycle,
                    EventRole::System,
                    vec![EventBlock::Text {
                        text: "runtime context".to_string(),
                    }],
                ),
                codex_test_event(
                    "developer",
                    SessionEventKind::Message,
                    EventRole::Developer,
                    vec![EventBlock::Text {
                        text: "developer instructions".to_string(),
                    }],
                ),
                codex_test_event(
                    "payload",
                    SessionEventKind::Message,
                    EventRole::System,
                    vec![EventBlock::ProviderPayload {
                        kind: "internal".to_string(),
                        payload: serde_json::json!({"text": "provider payload"}),
                    }],
                ),
                codex_test_event(
                    "user",
                    SessionEventKind::Message,
                    EventRole::User,
                    vec![EventBlock::Text {
                        text: "real prompt".to_string(),
                    }],
                ),
            ],
            artifacts: Vec::new(),
            extensions: BTreeMap::new(),
        };

        let session_id =
            export_canonical_session_in_codex_dir(&session, &workspace, &codex_dir).unwrap();
        let rollout_path = WalkDir::new(codex_dir.join("sessions"))
            .into_iter()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.into_path())
            .find(|path| {
                path.extension().and_then(|value| value.to_str()) == Some("jsonl")
                    && path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.contains(&session_id))
            })
            .expect("exported rollout");
        let session_meta = std::fs::read_to_string(&rollout_path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .find(|line| line.get("type").and_then(Value::as_str) == Some("session_meta"))
            .expect("session_meta line");
        let instructions = session_meta
            .pointer("/payload/base_instructions/text")
            .and_then(Value::as_str)
            .expect("base instructions");

        assert_eq!(
            instructions,
            "system instructions\n\ndeveloper instructions"
        );
        assert!(!instructions.contains("runtime context"));
        assert!(!instructions.contains("provider payload"));
        assert!(!instructions.contains("real prompt"));
    }

    fn codex_test_event(
        id: &str,
        kind: SessionEventKind,
        role: EventRole,
        blocks: Vec<EventBlock>,
    ) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind,
            role,
            timestamp: Utc::now(),
            links: EventLinks::default(),
            blocks,
            metadata: EventMetadata {
                source: EventSource {
                    provider_id: PROVIDER_ID.to_string(),
                    original_id: None,
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: BTreeMap::new(),
            },
        }
    }
}
