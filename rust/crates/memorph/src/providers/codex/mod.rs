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
    canonical_event_visible_message_role, canonical_event_visible_message_text,
    canonical_event_visible_text, canonical_export_result,
    canonical_session_instruction_context_text, canonical_session_title,
    canonical_visible_block_text, compression_retrieval_hint, CompressionProjection, PageStrategy,
    Provider, ProviderActivitySupport, ProviderBackupSupport, ProviderCapabilities,
    ProviderContentFidelity, ProviderSessionBackup, ProviderSessionImportPage,
    ProviderSessionSummary, ProviderSourceMutation, ProviderWriteRisk, ResumeQuality, ScanStrategy,
    StorageShape, TurnQuality, WriteRiskLevel,
};
use crate::session::{
    Block, Context, Event, EventKind, ExportedSession, Fidelity, Identity, ImportedSession, Links,
    MappingDirection, MappingIssue, MappingIssueLevel, MappingReport, Metadata, Provenance,
    ProviderRef, Role, Schema, Session, TurnOutcome,
};
use crate::storage::{
    activity_store::{
        ActivityActor, ActivityCompletion, ActivityOperationKind, ActivityStore, NewActivity,
    },
    artifact_store::{ArtifactStore, BackupRecord, NewBackupRecord},
    event_index, local_store, session_state,
};
use crate::utils;
use anyhow::{Context as _, Result};
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
    turn_id: Option<String>,
    turn_outcome: Option<TurnOutcome>,
}

impl CodexTurnLink {
    fn apply_to(self, event: &mut Event) {
        event.links.turn_id = self.turn_id;
        event.links.turn_outcome = self.turn_outcome;
    }
}

#[derive(Debug, Default)]
struct CodexTurnTracker {
    active_turn_id: Option<String>,
}

impl CodexTurnTracker {
    fn observe_line(&mut self, line: &Value) -> CodexTurnLink {
        let line_type = line.get("type").and_then(Value::as_str);
        let payload = line.get("payload");
        let payload_type = payload
            .and_then(|value| value.get("type"))
            .and_then(Value::as_str);
        let turn_outcome = match (line_type, payload_type) {
            (Some("event_msg"), Some("task_complete")) => Some(TurnOutcome::Completed),
            (Some("event_msg"), Some("turn_aborted")) => Some(TurnOutcome::Interrupted),
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
        let turn_id = explicit_turn_id.or_else(|| self.active_turn_id.clone());
        let closes_turn = matches!(
            turn_outcome,
            Some(TurnOutcome::Completed | TurnOutcome::Failed | TurnOutcome::Interrupted)
        );
        if closes_turn && self.active_turn_id.as_ref() == turn_id.as_ref() {
            self.active_turn_id = None;
        }

        CodexTurnLink {
            turn_id,
            turn_outcome,
        }
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
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Normalized),
                tool_call: Some(Fidelity::Preserved),
                tool_result: Some(Fidelity::Preserved),
                patch: Some(Fidelity::Unsupported),
                image: Some(Fidelity::Preserved),
                file: Some(Fidelity::Unsupported),
                compressed: Some(Fidelity::Normalized),
                provider_payload: Some(Fidelity::Preserved),
            },
            export_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Downgraded),
                tool_call: Some(Fidelity::Downgraded),
                tool_result: Some(Fidelity::Downgraded),
                patch: Some(Fidelity::Downgraded),
                image: Some(Fidelity::Normalized),
                file: Some(Fidelity::Downgraded),
                compressed: Some(Fidelity::Normalized),
                provider_payload: Some(Fidelity::Dropped),
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
            let created_at = rollout
                .created_at
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .map(|value| value.timestamp_millis());

            sessions.push(ProviderSessionSummary {
                session_id,
                title,
                project_dir,
                created_at,
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
            let created_at = source_path
                .as_deref()
                .and_then(|path| read_codex_rollout_summary(path).ok().flatten())
                .and_then(|summary| summary.created_at)
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
                .map(|value| value.timestamp_millis());
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
                created_at,
                last_active_at: updated_at,
                source_path: source_path.map(|p| p.to_string_lossy().to_string()),
            }));
        }

        Ok(None)
    }

    fn import_session(&self, source_path: &str) -> Result<ImportedSession> {
        let source_path = Path::new(source_path);
        let mut imported = import_canonical_session(source_path)?;
        imported.session.identity.title = resolve_codex_projection_title(
            source_path,
            &imported.session.identity.id,
            imported.session.identity.title.as_deref(),
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
        page.imported.session.identity.title = resolve_codex_projection_title(
            source_path,
            &page.imported.session.identity.id,
            page.imported.session.identity.title.as_deref(),
        )?;
        Ok(page)
    }

    fn supports_native_session_replace(&self) -> bool {
        true
    }

    fn replace_session(&self, session_id: &str, session: &Session) -> Result<()> {
        replace_codex_session(session_id, session)
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

fn first_user_message(session: &Session) -> Option<String> {
    session
        .events
        .iter()
        .filter(|event| canonical_event_visible_message_role(event) == Some(Role::User))
        .find_map(|event| {
            let text = canonical_event_visible_message_text(event)?;
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
}

fn has_user_event(session: &Session) -> bool {
    session
        .events
        .iter()
        .any(|event| canonical_event_visible_message_role(event) == Some(Role::User))
}

struct CodexSqliteUpdate<'a> {
    codex_dir: &'a Path,
    session_id: &'a str,
    rollout_path: &'a str,
    cwd: &'a Path,
    title: &'a str,
    first_user_message: Option<&'a str>,
    has_user_event: bool,
    now: &'a chrono::DateTime<Utc>,
}

fn update_codex_sqlite(update: CodexSqliteUpdate<'_>) -> Result<()> {
    let CodexSqliteUpdate {
        codex_dir,
        session_id,
        rollout_path,
        cwd,
        title,
        first_user_message,
        has_user_event,
        now,
    } = update;
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
    let sandbox_json = "{\"type\":\"workspace-write\",\"writable_roots\":[],\"network_access\":false,\"exclude_tmpdir_env_var\":false,\"exclude_slash_tmp\":false}".to_string();

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
mod tests;
