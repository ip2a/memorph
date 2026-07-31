use crate::session::{
    Block, Event, EventKind, ExportedSession, Fidelity, ImportedSession, MappingDirection,
    MappingIssue, MappingIssueLevel, MappingReport, Role, Session,
};
use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSessionSummary {
    pub session_id: String,
    pub title: Option<String>,
    pub project_dir: Option<String>,
    pub created_at: Option<i64>,
    pub last_active_at: Option<i64>,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSourceFingerprint {
    pub modified_at_ms: i64,
    pub size_bytes: i64,
    pub value: String,
}

#[derive(Debug, Clone)]
pub struct ProviderSessionImportPage {
    pub imported: ImportedSession,
    pub event_count: usize,
    pub message_count: usize,
    /// Complete-session turn count when the provider can establish it from this read.
    pub turn_count: Option<usize>,
    /// Turn projections derived only from the events returned in this page.
    pub turns: Vec<crate::session_projection::TurnProjection>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSourceMutation {
    Delete,
    Rename,
    Replace,
}

#[derive(Debug, Clone)]
pub struct ProviderSessionBackup {
    pub mutation: ProviderSourceMutation,
    pub operation_id: String,
    pub provider_session_id: String,
    pub source_path: PathBuf,
    pub backup_path: PathBuf,
    pub restore_hint: String,
    pub mime_type: String,
    pub format: String,
    pub artifact_metadata: Value,
    pub restore_metadata: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProviderCapabilities {
    pub scan: bool,
    pub import: bool,
    pub export: bool,
    pub delete: bool,
    pub rename: bool,
    pub resume: bool,
    /// Provider implements scan_sessions_lightweight with a genuinely cheaper
    /// path than scan_sessions (bounded read, index-only, etc.). False means
    /// the lightweight method falls back to the full scan.
    pub lightweight_scan: bool,
    /// Provider implements find_session_by_id without falling back to a full
    /// provider scan. False means single-session lookup degrades to scan+filter.
    pub single_session_lookup: bool,
    pub scan_strategy: ScanStrategy,
    pub page_strategy: PageStrategy,
    pub storage_shape: StorageShape,
    pub turn_quality: TurnQuality,
    pub import_fidelity: ProviderContentFidelity,
    pub export_fidelity: ProviderContentFidelity,
    pub resume_quality: ResumeQuality,
    pub write_risk: ProviderWriteRisk,
    pub backup_support: ProviderBackupSupport,
    pub activity_support: ProviderActivitySupport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanStrategy {
    Unknown,
    FullScan,
    Indexed,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PageStrategy {
    Unknown,
    FullImport,
    IndexedPage,
    NativePage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageShape {
    Unknown,
    Jsonl,
    Sqlite,
    Directory,
    Mixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnQuality {
    Unknown,
    Exact,
    Inferred,
    Grouped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProviderContentFidelity {
    pub text: Option<Fidelity>,
    pub thinking: Option<Fidelity>,
    pub tool_call: Option<Fidelity>,
    pub tool_result: Option<Fidelity>,
    pub patch: Option<Fidelity>,
    pub image: Option<Fidelity>,
    pub file: Option<Fidelity>,
    pub compressed: Option<Fidelity>,
    pub provider_payload: Option<Fidelity>,
}

impl ProviderContentFidelity {
    pub const fn unknown() -> Self {
        Self {
            text: None,
            thinking: None,
            tool_call: None,
            tool_result: None,
            patch: None,
            image: None,
            file: None,
            compressed: None,
            provider_payload: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeQuality {
    None,
    Native,
    Imported,
    TextOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderWriteRisk {
    pub level: WriteRiskLevel,
    pub multiple_files: bool,
    pub sqlite: bool,
    pub sidecar_files: bool,
    pub index_repair: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteRiskLevel {
    Unknown,
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProviderBackupSupport {
    pub before_write: bool,
    pub restore: bool,
    pub sync_only: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ProviderActivitySupport {
    pub hook_events: bool,
    pub runtime_endpoint: bool,
    pub session_activity: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompressionProjection {
    Native,
    Portable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderCompressionSupport {
    pub provider_id: String,
    pub detects_native_source: bool,
    pub native_target_projection: bool,
    pub native_session_replace: bool,
    pub native_session_restore: bool,
    pub default_projection: CompressionProjection,
}

impl ProviderCapabilities {
    pub const fn full_session_management() -> Self {
        Self {
            scan: true,
            import: true,
            export: true,
            delete: true,
            rename: true,
            resume: true,
            ..Self::unknown_quality()
        }
    }

    const fn unknown_quality() -> Self {
        Self {
            scan: false,
            import: false,
            export: false,
            delete: false,
            rename: false,
            resume: false,
            lightweight_scan: false,
            single_session_lookup: false,
            scan_strategy: ScanStrategy::Unknown,
            page_strategy: PageStrategy::Unknown,
            storage_shape: StorageShape::Unknown,
            turn_quality: TurnQuality::Unknown,
            import_fidelity: ProviderContentFidelity::unknown(),
            export_fidelity: ProviderContentFidelity::unknown(),
            resume_quality: ResumeQuality::None,
            write_risk: ProviderWriteRisk {
                level: WriteRiskLevel::Unknown,
                multiple_files: false,
                sqlite: false,
                sidecar_files: false,
                index_repair: false,
            },
            backup_support: ProviderBackupSupport {
                before_write: false,
                restore: false,
                sync_only: false,
            },
            activity_support: ProviderActivitySupport {
                hook_events: false,
                runtime_endpoint: false,
                session_activity: false,
            },
        }
    }
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            scan: true,
            import: true,
            export: false,
            delete: false,
            rename: false,
            resume: false,
            ..Self::unknown_quality()
        }
    }
}

pub fn default_normalized_workspace_key(workspace: Option<&str>) -> Option<String> {
    let workspace = workspace.map(str::trim).filter(|value| !value.is_empty())?;
    crate::config::resolve_workspace(Some(workspace))
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .or_else(|| Some(PathBuf::from(workspace).to_string_lossy().to_string()))
}

pub fn default_workspace_matches(
    session_workspace: Option<&str>,
    requested_workspace: Option<&str>,
) -> bool {
    let Some(requested_workspace) = default_normalized_workspace_key(requested_workspace) else {
        return true;
    };
    let Some(session_workspace) = default_normalized_workspace_key(session_workspace) else {
        return false;
    };
    session_workspace == requested_workspace
}

pub fn default_resolve_workspace_dir(input: Option<&str>) -> Result<PathBuf> {
    crate::config::resolve_workspace(input)
}

fn file_source_fingerprint(path: &Path) -> Result<Option<ProviderSourceFingerprint>> {
    let path_text = path.to_string_lossy();
    let file_path = match path_text.split_once('#') {
        Some((file_path, fragment)) if fragment.starts_with("session=") => PathBuf::from(file_path),
        _ => path.to_path_buf(),
    };
    let metadata = match std::fs::metadata(&file_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Failed to read session source metadata: {}",
                    file_path.display()
                )
            })
        }
    };
    let mut modified_at_ms = source_metadata_modified_ms(&metadata);
    let mut size_bytes = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
    let mut wal_modified_at_ms = 0;
    let mut wal_size_bytes = 0;

    if file_path.extension().and_then(|value| value.to_str()) == Some("db")
        || path_text.contains("#session=")
    {
        let wal_path = PathBuf::from(format!("{}-wal", file_path.to_string_lossy()));
        if let Ok(wal_metadata) = std::fs::metadata(&wal_path) {
            wal_modified_at_ms = source_metadata_modified_ms(&wal_metadata);
            wal_size_bytes = i64::try_from(wal_metadata.len()).unwrap_or(i64::MAX);
            modified_at_ms = modified_at_ms.max(wal_modified_at_ms);
            size_bytes = size_bytes.saturating_add(wal_size_bytes);
        }
    }

    Ok(Some(ProviderSourceFingerprint {
        modified_at_ms,
        size_bytes,
        value: format!(
            "metadata-v1:{modified_at_ms}:{size_bytes}:{wal_modified_at_ms}:{wal_size_bytes}"
        ),
    }))
}

fn source_metadata_modified_ms(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

/// Provider trait: each AI coding tool implements this interface
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
    }

    /// Whether this provider importer can recognize provider-native compressed session data.
    fn detects_native_compression_source(&self) -> bool {
        false
    }

    /// How this provider exporter maps canonical compressed segments by default.
    fn compression_projection(&self) -> CompressionProjection {
        CompressionProjection::Portable
    }

    /// Whether this provider can replace an existing native session without changing its identity.
    fn supports_native_session_replace(&self) -> bool {
        false
    }

    /// Replace an existing provider-native session with the supplied canonical session.
    fn replace_session(&self, session_id: &str, session: &Session) -> Result<()> {
        let _ = session_id;
        let _ = session;
        anyhow::bail!(
            "Native session replacement is not supported for provider: {}",
            self.id()
        )
    }

    /// Normalize a workspace identifier into the provider's canonical scope key.
    fn normalized_workspace_key(&self, workspace: Option<&str>) -> Option<String> {
        default_normalized_workspace_key(workspace)
    }

    /// Match two workspace identifiers using the provider's native workspace semantics.
    fn workspace_matches(
        &self,
        session_workspace: Option<&str>,
        requested_workspace: Option<&str>,
    ) -> bool {
        let Some(requested_workspace) = self.normalized_workspace_key(requested_workspace) else {
            return true;
        };
        let Some(session_workspace) = self.normalized_workspace_key(session_workspace) else {
            return false;
        };
        session_workspace == requested_workspace
    }

    /// Resolve a target workspace directory for provider-side session creation.
    fn resolve_workspace_dir(&self, input: Option<&str>) -> Result<PathBuf> {
        default_resolve_workspace_dir(input)
    }

    /// Scan all session metadata
    fn scan_sessions(&self) -> Result<Vec<ProviderSessionSummary>>;

    /// Scan only the metadata needed for listing, skipping heavier per-session work.
    ///
    /// Default falls back to scan_sessions. Providers with a genuinely cheaper
    /// path (bounded head/tail read, index-only lookup) override this and set
    /// capabilities().lightweight_scan = true so callers can pick the fast path
    /// without try-and-see.
    fn scan_sessions_lightweight(&self) -> Result<Vec<ProviderSessionSummary>> {
        self.scan_sessions()
    }

    /// Scan sessions within a workspace directory scope.
    ///
    /// Providers that can cheaply enumerate sessions under one workspace
    /// override this. The default returns an empty vector to signal that the
    /// provider does not support workspace-scoped enumeration; callers must
    /// fall back to scan_sessions and filter.
    fn scan_workspace(&self, _workspace_dir: &Path) -> Result<Vec<ProviderSessionSummary>> {
        Ok(Vec::new())
    }

    /// Resolve metadata for a single session by id without a full scan.
    ///
    /// Providers that keep an index or can derive the source path from the
    /// session id override this. The default returns None so callers know to
    /// fall back to scan_sessions and filter.
    fn find_session_by_id(
        &self,
        _session_id: &str,
    ) -> Result<Option<ProviderSessionSummary>> {
        Ok(None)
    }

    /// Load a high-fidelity canonical session plus a mapping report.
    fn import_session(&self, source_path: &str) -> Result<ImportedSession>;

    /// Fingerprint the provider-native source addressed by this session locator.
    ///
    /// Single-file providers use the default metadata/WAL fingerprint. Providers whose
    /// session source spans multiple files must override this contract.
    fn session_source_fingerprint(
        &self,
        source_path: &str,
    ) -> Result<Option<ProviderSourceFingerprint>> {
        file_source_fingerprint(Path::new(source_path))
    }

    /// Load a page of canonical events while preserving total event/message counts.
    ///
    /// Providers with native append-only or indexed storage should override this.
    /// The default path imports the full session and slices in memory.
    fn import_session_page(
        &self,
        source_path: &str,
        event_offset: usize,
        event_limit: Option<usize>,
    ) -> Result<ProviderSessionImportPage> {
        let mut imported = self.import_session(source_path)?;
        let event_count = imported.session.events.len();
        let message_count = imported
            .session
            .events
            .iter()
            .filter(|event| event_is_visible_message(event))
            .count();
        let turn_count = crate::session_projection::project_session_turns(
            &imported.session.identity.id,
            &imported.session.events,
            self.capabilities().turn_quality,
        )
        .len();
        let offset = event_offset.min(imported.session.events.len());

        if let Some(limit) = event_limit {
            imported.session.events = imported
                .session
                .events
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect();
        } else if offset > 0 {
            imported.session.events = imported.session.events.into_iter().skip(offset).collect();
        }

        let turns = crate::session_projection::project_session_turns(
            &imported.session.identity.id,
            &imported.session.events,
            self.capabilities().turn_quality,
        );
        Ok(ProviderSessionImportPage {
            imported,
            event_count,
            message_count,
            turn_count: Some(turn_count),
            turns,
        })
    }

    /// Write a canonical session into the target tool and return the mapping report.
    fn export_session(&self, session: &Session, target_dir: &Path) -> Result<ExportedSession> {
        let _ = session;
        let _ = target_dir;
        anyhow::bail!(
            "Canonical export is not implemented for provider: {}",
            self.id()
        )
    }

    /// Delete a session
    fn delete_session(&self, session_id: &str) -> Result<()> {
        let _ = session_id;
        anyhow::bail!("Delete not supported for provider: {}", self.id())
    }

    /// Delete multiple sessions. Providers can override this to batch database work.
    fn delete_sessions(&self, session_ids: &[&str]) -> Vec<Result<()>> {
        session_ids
            .iter()
            .map(|session_id| self.delete_session(session_id))
            .collect()
    }

    /// Rename a session
    fn rename_session(&self, session_id: &str, new_title: &str) -> Result<()> {
        let _ = session_id;
        let _ = new_title;
        anyhow::bail!("Rename not supported for provider: {}", self.id())
    }

    /// Capture the exact provider-native source that a delete or rename can modify.
    ///
    /// Core registers the returned artifact before invoking the provider mutation.
    fn create_session_backup(
        &self,
        mutation: ProviderSourceMutation,
        operation_id: &str,
        session_id: &str,
        backup_root: &Path,
    ) -> Result<ProviderSessionBackup> {
        let _ = mutation;
        let _ = operation_id;
        let _ = session_id;
        let _ = backup_root;
        anyhow::bail!(
            "Native session backup is not supported for provider: {}",
            self.id()
        )
    }

    /// Restore an exact provider-native backup created by `create_session_backup`.
    fn restore_session_backup(&self, backup: &ProviderSessionBackup) -> Result<()> {
        let _ = backup;
        anyhow::bail!(
            "Native session backup restore is not supported for provider: {}",
            self.id()
        )
    }

    /// Build the provider-specific command used to resume a session.
    fn resume_command(&self, session_id: &str) -> Option<String> {
        let _ = session_id;
        None
    }

    /// Return paths that should be watched for cache invalidation.
    fn data_source_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Resolve metadata for a single session by ID.
    ///
    /// Tries the provider's direct lookup first, then falls back to a full
    /// scan. Providers that implement direct lookup override find_session_by_id.
    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
        if let Some(meta) = self.find_session_by_id(session_id)? {
            return Ok(Some(meta));
        }
        self.scan_sessions()
            .map(|sessions| sessions.into_iter().find(|s| s.session_id == session_id))
    }

    /// Estimate the storage size (in bytes) of a single session.
    /// Default returns 0 (unknown).
    fn session_size(&self, session_id: &str) -> Result<u64> {
        let _ = session_id;
        Ok(0)
    }

    fn session_sizes(&self, session_ids: &[&str]) -> HashMap<String, u64> {
        session_ids
            .iter()
            .filter_map(|session_id| {
                self.session_size(session_id)
                    .ok()
                    .filter(|size| *size > 0)
                    .map(|size| ((*session_id).to_string(), size))
            })
            .collect()
    }
}

pub fn export_report(
    provider_id: &str,
    session: &Session,
    capabilities: ProviderCapabilities,
) -> MappingReport {
    let mut report = MappingReport::new(provider_id, MappingDirection::Export);
    let mut assessed = HashSet::new();

    for block in session.events.iter().flat_map(|event| &event.blocks) {
        let (content_kind, disposition) =
            export_block_fidelity(block, capabilities.export_fidelity);
        if !assessed.insert(content_kind) {
            continue;
        }
        match disposition {
            Some(Fidelity::Preserved) => {}
            Some(disposition) => report.push_issue(MappingIssue {
                level: if disposition == Fidelity::Normalized {
                    MappingIssueLevel::Info
                } else {
                    MappingIssueLevel::Warning
                },
                disposition,
                code: format!("{content_kind}_export_{disposition}", disposition = disposition_name(disposition)),
                message: format!(
                    "{provider_id} exports canonical {content_kind} content as {}.",
                    disposition_name(disposition)
                ),
                path: Some(format!("events.blocks.{content_kind}")),
                raw: None,
            }),
            None => report.push_issue(MappingIssue {
                level: MappingIssueLevel::Warning,
                disposition: Fidelity::Unsupported,
                code: format!("{content_kind}_export_capability_unknown"),
                message: format!(
                    "{provider_id} export fidelity for canonical {content_kind} content is not cataloged."
                ),
                path: Some(format!("events.blocks.{content_kind}")),
                raw: None,
            }),
        }
    }

    report
}

pub fn export_result(
    provider_id: &str,
    session_id: String,
    resume_command: Option<String>,
    session: &Session,
    capabilities: ProviderCapabilities,
) -> ExportedSession {
    ExportedSession {
        provider_id: provider_id.to_string(),
        session_id,
        resume_command,
        report: export_report(provider_id, session, capabilities),
    }
}

fn export_block_fidelity(
    block: &Block,
    fidelity: ProviderContentFidelity,
) -> (&'static str, Option<Fidelity>) {
    match block {
        Block::Text { .. } => ("text", fidelity.text),
        Block::Thinking { .. } => ("thinking", fidelity.thinking),
        Block::ToolCall { .. } | Block::Command { .. } => ("tool_call", fidelity.tool_call),
        Block::ToolResult { .. } | Block::CommandResult { .. } => {
            ("tool_result", fidelity.tool_result)
        }
        Block::Patch { .. } => ("patch", fidelity.patch),
        Block::Image { .. } => ("image", fidelity.image),
        Block::File { .. } => ("file", fidelity.file),
        Block::Compressed { .. } => ("compressed", fidelity.text),
        Block::Other { .. } => ("other", fidelity.provider_payload),
    }
}

fn disposition_name(disposition: Fidelity) -> &'static str {
    match disposition {
        Fidelity::Preserved => "preserved",
        Fidelity::Normalized => "normalized",
        Fidelity::Downgraded => "downgraded",
        Fidelity::Dropped => "dropped",
        Fidelity::Unsupported => "unsupported",
    }
}

pub fn session_title(session: &Session) -> String {
    if let Some(title) = session
        .identity
        .title
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return title.to_string();
    }

    session
        .events
        .iter()
        .find_map(|event| {
            if !matches!(
                event_visible_message_role(event),
                Some(Role::User | Role::Assistant)
            ) {
                return None;
            }
            event_visible_message_text(event)?
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Imported session".to_string())
}

pub fn event_visible_message_role(event: &Event) -> Option<Role> {
    if matches!(event.kind, EventKind::Lifecycle | EventKind::Other) {
        return None;
    }
    match event.role {
        Role::User | Role::Assistant | Role::Tool => Some(event.role),
        Role::System | Role::Developer | _ => None,
    }
}

pub fn event_is_visible_message(event: &Event) -> bool {
    event_visible_message_role(event).is_some()
        && !event_visible_text(event).trim().is_empty()
}

pub fn event_visible_message_text(event: &Event) -> Option<String> {
    event_visible_message_role(event)?;
    let text = event_visible_text(event);
    (!text.trim().is_empty()).then_some(text)
}

pub fn event_instruction_context_text(event: &Event) -> Option<String> {
    if matches!(event.kind, EventKind::Lifecycle | EventKind::Other) {
        return None;
    }
    if !matches!(event.role, Role::System | Role::Developer) {
        return None;
    }
    let text = event_visible_text(event);
    (!text.trim().is_empty()).then_some(text)
}

pub fn session_instruction_context_text(session: &Session) -> Option<String> {
    let text = session
        .events
        .iter()
        .filter_map(event_instruction_context_text)
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

pub fn event_text(event: &Event) -> String {
    event
        .blocks
        .iter()
        .map(block_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn visible_block_text(block: &Block) -> Option<String> {
    if matches!(block, Block::Other { .. }) {
        return None;
    }
    let text = block_text(block);
    (!text.trim().is_empty()).then_some(text)
}

pub fn event_visible_text(event: &Event) -> String {
    event
        .blocks
        .iter()
        .filter_map(visible_block_text)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn block_text(block: &Block) -> String {
    match block {
        Block::Text { text } => text.clone(),
        Block::Thinking { text, .. } => text.clone(),
        Block::ToolCall {
            tool_call_id,
            name,
            input,
        } => format!(
            "[Tool use: {} ({})]\n{}",
            name,
            tool_call_id,
            input
                .as_ref()
                .map(|value| value.to_string())
                .unwrap_or_default()
        ),
        Block::ToolResult {
            tool_call_id,
            content,
            is_error,
        } => {
            let label = if *is_error {
                "Tool error"
            } else {
                "Tool result"
            };
            format!("[{}: {}]\n{}", label, tool_call_id, content)
        }
        Block::Patch {
            summary,
            diff_text,
            files,
            ..
        } => {
            let mut parts = Vec::new();
            if let Some(summary) = summary {
                parts.push(summary.clone());
            }
            if !files.is_empty() {
                parts.push(format!("Files: {}", files.join(", ")));
            }
            if let Some(diff) = diff_text {
                parts.push(diff.clone());
            }
            parts.join("\n")
        }
        Block::Command { command, argv, cwd } => {
            let mut text = command.clone();
            if !argv.is_empty() {
                text.push('\n');
                text.push_str(&argv.join(" "));
            }
            if let Some(cwd) = cwd {
                text.push_str(&format!("\nCWD: {}", cwd));
            }
            text
        }
        Block::CommandResult {
            command,
            exit_code,
            stdout,
            stderr,
        } => {
            let mut parts = Vec::new();
            if let Some(command) = command {
                parts.push(format!("Command: {}", command));
            }
            if let Some(exit_code) = exit_code {
                parts.push(format!("Exit: {}", exit_code));
            }
            if let Some(stdout) = stdout {
                parts.push(stdout.clone());
            }
            if let Some(stderr) = stderr {
                parts.push(stderr.clone());
            }
            parts.join("\n")
        }
        Block::File { path, content, .. } => content
            .as_ref()
            .map(|content| format!("[File: {}]\n{}", path, content))
            .unwrap_or_else(|| format!("[File: {}]", path)),
        Block::Image {
            mime_type,
            data,
            path,
        } => path
            .clone()
            .or_else(|| data.clone())
            .map(|value| format!("[Image: {}]\n{}", mime_type, value))
            .unwrap_or_else(|| format!("[Image: {}]", mime_type)),
        Block::Compressed { raw } => {
            let Some(source_provider_id) = raw.get("source_provider_id").and_then(Value::as_str)
            else {
                return format!("[Compressed]\n{}", raw);
            };
            let Some(summary) = raw.get("summary").and_then(Value::as_str) else {
                return format!("[Compressed]\n{}", raw);
            };
            let mut parts = vec![
                format!("[Compressed session segment from {}]", source_provider_id),
                summary.to_string(),
            ];
            if let Some(count) = raw.get("source_event_count").and_then(Value::as_u64) {
                parts.push(format!("Source event count: {}", count));
            }
            if let Some(archive_ref) = raw.get("archive_ref").and_then(Value::as_str) {
                parts.push(format!("Archive: {}", archive_ref));
                parts.push(compression_retrieval_hint(archive_ref));
            }
            parts.join("\n")
        }
        Block::Other { raw } => format!("[Other]\n{}", raw),
    }
}

pub fn compression_retrieval_hint(archive_ref: &str) -> String {
    format!(
        "Retrieve specific details with: memorph compression retrieve {} --query <terms> --max-results 5",
        archive_ref
    )
}

pub fn event_role_label(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::System => "system",
        Role::Developer => "developer",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_workspace_matches_canonicalizes_equivalent_existing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let alias = dir.path().join(".");

        assert!(default_workspace_matches(
            alias.to_str(),
            dir.path().to_str()
        ));
    }

    #[test]
    fn portable_compression_text_is_preserved_for_provider_projection() {
        let text = block_text(&Block::Text {
            text: "[Compressed session segment from opencode]\ncompressed summary\nSource event count: 20\nArchive: memorph-archive://session/archive.json".to_string(),
        });

        assert!(text.contains("compressed summary"));
        assert!(text.contains("Source event count: 20"));
        assert!(text.contains("memorph-archive://session/archive.json"));
    }

    #[test]
    fn visible_block_text_drops_provider_internal_blocks() {
        assert!(visible_block_text(&Block::Other {
            raw: serde_json::json!({"type": "token_count", "input_tokens": 10}),
        })
        .is_none());
        assert!(visible_block_text(&Block::Other {
            raw: serde_json::json!({"type": "mystery"}),
        })
        .is_none());
    }

    #[test]
    fn event_visible_text_omits_provider_internal_blocks() {
        let event = Event {
            id: "event-1".to_string(),
            kind: crate::session::EventKind::Message,
            role: Role::User,
            timestamp: chrono::Utc::now(),
            links: crate::session::Links::default(),
            blocks: vec![
                Block::Text {
                    text: "hello".to_string(),
                },
                Block::Other {
                    raw: serde_json::json!({"type": "token_count", "input_tokens": 10}),
                },
                Block::Other {
                    raw: serde_json::json!({"type": "mystery"}),
                },
            ],
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: crate::session::Metadata {
                model: None,
                usage: None,
            },
        };

        assert_eq!(event_visible_text(&event), "hello");
    }

    #[test]
    fn visible_message_role_excludes_internal_events() {
        let lifecycle = test_event(
            "lifecycle",
            crate::session::EventKind::Lifecycle,
            Role::System,
            vec![Block::Text {
                text: "internal".to_string(),
            }],
        );
        let developer = test_event(
            "developer",
            crate::session::EventKind::Message,
            Role::Developer,
            vec![Block::Text {
                text: "developer".to_string(),
            }],
        );
        let unknown = test_event(
            "unknown",
            crate::session::EventKind::Other,
            Role::User,
            vec![Block::Text {
                text: "unknown".to_string(),
            }],
        );
        let user = test_event(
            "user",
            crate::session::EventKind::Message,
            Role::User,
            vec![Block::Text {
                text: "hello".to_string(),
            }],
        );

        assert_eq!(event_visible_message_role(&lifecycle), None);
        assert_eq!(event_visible_message_role(&developer), None);
        assert_eq!(event_visible_message_role(&unknown), None);
        assert_eq!(
            event_visible_message_role(&user),
            Some(Role::User)
        );
        assert_eq!(
            event_visible_message_text(&user).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn export_report_uses_actual_block_kinds_and_target_fidelity() {
        let session = Session {
            schema: crate::session::Schema::default(),
            identity: crate::session::Identity {
                id: "canonical-1".to_string(),
                title: None,
            },
            context: crate::session::Context::default(),
            events: vec![test_event(
                "assistant",
                crate::session::EventKind::Message,
                Role::Assistant,
                vec![
                    Block::Text {
                        text: "answer".to_string(),
                    },
                    Block::Thinking {
                        text: "reasoning".to_string(),
                        signature: None,
                    },
                    Block::Command {
                        command: "cargo test".to_string(),
                        argv: Vec::new(),
                        cwd: None,
                    },
                ],
            )],
            extensions: Default::default(),
        };
        let capabilities = ProviderCapabilities {
            export_fidelity: ProviderContentFidelity {
                text: Some(Fidelity::Preserved),
                thinking: Some(Fidelity::Normalized),
                tool_call: Some(Fidelity::Downgraded),
                tool_result: None,
                patch: None,
                image: None,
                file: None,
                compressed: None,
                provider_payload: None,
            },
            ..ProviderCapabilities::default()
        };

        let report = export_report("target", &session, capabilities);

        assert_eq!(report.overall, Fidelity::Downgraded);
        assert_eq!(report.issues.len(), 2);
        assert_eq!(report.issues[0].code, "thinking_export_normalized");
        assert_eq!(report.issues[1].code, "tool_call_export_downgraded");
    }

    #[test]
    fn canonical_session_title_uses_visible_user_or_assistant_message() {
        let session = Session {
            schema: crate::session::Schema {
                name: crate::session::OASF_SCHEMA_NAME.to_string(),
                version: 1,
            },
            identity: crate::session::Identity {
                id: "canonical-1".to_string(),
                title: None,
            },
            context: crate::session::Context::default(),
            events: vec![
                test_event(
                    "internal",
                    crate::session::EventKind::Lifecycle,
                    Role::System,
                    vec![Block::Other {
                        raw: serde_json::json!({"type": "internal", "id": "should-not-title"}),
                    }],
                ),
                test_event(
                    "prompt",
                    crate::session::EventKind::Message,
                    Role::User,
                    vec![Block::Text {
                        text: "real prompt".to_string(),
                    }],
                ),
            ],
            extensions: Default::default(),
        };

        assert_eq!(session_title(&session), "real prompt");
    }

    #[test]
    fn instruction_context_uses_only_system_or_developer_messages() {
        let session = Session {
            schema: crate::session::Schema {
                name: crate::session::OASF_SCHEMA_NAME.to_string(),
                version: 1,
            },
            identity: crate::session::Identity {
                id: "canonical-1".to_string(),
                title: None,
            },
            context: crate::session::Context::default(),
            events: vec![
                test_event(
                    "system",
                    crate::session::EventKind::Message,
                    Role::System,
                    vec![Block::Text {
                        text: "system instructions".to_string(),
                    }],
                ),
                test_event(
                    "developer",
                    crate::session::EventKind::Message,
                    Role::Developer,
                    vec![Block::Text {
                        text: "developer instructions".to_string(),
                    }],
                ),
                test_event(
                    "internal",
                    crate::session::EventKind::Lifecycle,
                    Role::System,
                    vec![Block::Text {
                        text: "runtime context".to_string(),
                    }],
                ),
                test_event(
                    "payload",
                    crate::session::EventKind::Message,
                    Role::System,
                    vec![Block::Other {
                        raw: serde_json::json!({"type": "internal", "text": "provider payload"}),
                    }],
                ),
                test_event(
                    "user",
                    crate::session::EventKind::Message,
                    Role::User,
                    vec![Block::Text {
                        text: "user prompt".to_string(),
                    }],
                ),
            ],
            extensions: Default::default(),
        };

        assert_eq!(
            session_instruction_context_text(&session).as_deref(),
            Some("system instructions\n\ndeveloper instructions")
        );
    }

    fn test_event(
        id: &str,
        kind: crate::session::EventKind,
        role: Role,
        blocks: Vec<Block>,
    ) -> Event {
        Event {
            id: id.to_string(),
            kind,
            role,
            timestamp: chrono::Utc::now(),
            links: crate::session::Links::default(),
            blocks,
            tags: Vec::new(),
            extensions: Default::default(),
            metadata: crate::session::Metadata {
                model: None,
                usage: None,
            },
        }
    }
}
