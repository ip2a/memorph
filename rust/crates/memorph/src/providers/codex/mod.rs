pub mod adapter;
pub mod backup;
pub mod hook;
pub mod load;
pub mod management;

use self::backup::*;
use self::load::*;
use self::management::*;

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
        let index_titles = load_session_index_entries(&codex_dir.join("session_index.jsonl"))?;
        let sqlite_lookup = build_sqlite_thread_metadata_lookup(&codex_dir).unwrap_or_default();
        let mut sessions = Vec::new();

        for (path, rollout) in discover_codex_rollouts(&codex_dir)? {
            let session_id = rollout.session_id.clone();
            let sqlite_meta = sqlite_lookup.get(&session_id);
            let project_dir = sqlite_meta
                .and_then(|meta| clean_non_empty(meta.cwd.as_deref()))
                .map(str::to_string)
                .or(rollout.workspace_dir.clone());
            let title = select_codex_display_title(
                index_titles.get(&session_id).map(String::as_str),
                sqlite_meta.and_then(|meta| meta.title.as_deref()),
                rollout.title.as_deref(),
                &session_id,
            );
            let last_active_at = rollout
                .updated_at
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.timestamp_millis());

            sessions.push(ProviderSessionSummary {
                session_id,
                title,
                project_dir,
                last_active_at,
                source_path: Some(path.to_string_lossy().to_string()),
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

    #[test]
    fn scan_sessions_includes_sqlite_threads_missing_from_session_index() {
        let temp = tempdir().unwrap();
        let codex_dir = temp.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let _guard = use_test_codex_dir(codex_dir.clone());
        let rollout_path = codex_dir.join("sessions/rollout-unindexed.jsonl");
        std::fs::create_dir_all(rollout_path.parent().unwrap()).unwrap();
        std::fs::write(
            &rollout_path,
            serde_json::to_string(&json!({
                "type": "session_meta",
                "payload": {
                    "id": "unindexed-session",
                    "cwd": "/tmp/rollout-project",
                    "title": "Rollout title"
                }
            }))
            .unwrap()
                + "\n",
        )
        .unwrap();

        let conn = Connection::open(codex_dir.join(CODEX_SQLITE_FILE_BASENAME)).unwrap();
        conn.execute_batch(
            "CREATE TABLE threads (id TEXT PRIMARY KEY, cwd TEXT, title TEXT, rollout_path TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, cwd, title, rollout_path) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "unindexed-session",
                "/tmp/project",
                "Current Codex session",
                rollout_path.to_string_lossy().as_ref()
            ],
        )
        .unwrap();

        let sessions = CodexProvider.scan_sessions().unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].session_id, "unindexed-session");
        assert_eq!(sessions[0].project_dir.as_deref(), Some("/tmp/project"));
        assert_eq!(sessions[0].title.as_deref(), Some("Current Codex session"));
        assert_eq!(sessions[0].source_path.as_deref(), rollout_path.to_str());
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
