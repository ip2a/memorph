use crate::canonical::{
    CanonicalSession, EventBlock, EventRole, ExportedSession, ImportedSession, MappingDirection,
    MappingDisposition, MappingIssue, MappingIssueLevel, MappingReport, SessionEvent,
    SessionEventKind,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ProviderSessionSummary {
    pub session_id: String,
    pub title: Option<String>,
    pub project_dir: Option<String>,
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
    pub text: Option<MappingDisposition>,
    pub thinking: Option<MappingDisposition>,
    pub tool_call: Option<MappingDisposition>,
    pub tool_result: Option<MappingDisposition>,
    pub patch: Option<MappingDisposition>,
    pub image: Option<MappingDisposition>,
    pub file: Option<MappingDisposition>,
    pub compressed: Option<MappingDisposition>,
    pub provider_payload: Option<MappingDisposition>,
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
    fn replace_session(&self, session_id: &str, session: &CanonicalSession) -> Result<()> {
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
            .filter(|event| canonical_event_is_visible_message(event))
            .count();
        let turn_count = crate::session_projection::project_session_turns(
            &imported.session.identity.canonical_id,
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
            &imported.session.identity.canonical_id,
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
    fn export_session(
        &self,
        session: &CanonicalSession,
        target_dir: &Path,
    ) -> Result<ExportedSession> {
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

    /// Get metadata for a single session by ID.
    /// Default implementation falls back to scan_sessions; providers should override.
    fn get_session_meta(&self, session_id: &str) -> Result<Option<ProviderSessionSummary>> {
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

pub fn canonical_export_report(
    provider_id: &str,
    session: &CanonicalSession,
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
            Some(MappingDisposition::Preserved) => {}
            Some(disposition) => report.push_issue(MappingIssue {
                level: if disposition == MappingDisposition::Normalized {
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
                disposition: MappingDisposition::Unsupported,
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

pub fn canonical_export_result(
    provider_id: &str,
    session_id: String,
    resume_command: Option<String>,
    session: &CanonicalSession,
    capabilities: ProviderCapabilities,
) -> ExportedSession {
    ExportedSession {
        provider_id: provider_id.to_string(),
        session_id,
        resume_command,
        report: canonical_export_report(provider_id, session, capabilities),
    }
}

fn export_block_fidelity(
    block: &EventBlock,
    fidelity: ProviderContentFidelity,
) -> (&'static str, Option<MappingDisposition>) {
    match block {
        EventBlock::Text { .. } => ("text", fidelity.text),
        EventBlock::Thinking { .. } => ("thinking", fidelity.thinking),
        EventBlock::ToolCall { .. } | EventBlock::Command { .. } => {
            ("tool_call", fidelity.tool_call)
        }
        EventBlock::ToolResult { .. } | EventBlock::CommandResult { .. } => {
            ("tool_result", fidelity.tool_result)
        }
        EventBlock::Patch { .. } => ("patch", fidelity.patch),
        EventBlock::Image { .. } => ("image", fidelity.image),
        EventBlock::File { .. } => ("file", fidelity.file),
        EventBlock::Compressed { .. } => ("compressed", fidelity.compressed),
        EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. } => {
            ("provider_payload", fidelity.provider_payload)
        }
    }
}

fn disposition_name(disposition: MappingDisposition) -> &'static str {
    match disposition {
        MappingDisposition::Preserved => "preserved",
        MappingDisposition::Normalized => "normalized",
        MappingDisposition::Downgraded => "downgraded",
        MappingDisposition::Dropped => "dropped",
        MappingDisposition::Unsupported => "unsupported",
    }
}

pub fn canonical_session_title(session: &CanonicalSession) -> String {
    if let Some(title) = session
        .identity
        .source_title
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
                canonical_event_visible_message_role(event),
                Some(EventRole::User | EventRole::Assistant)
            ) {
                return None;
            }
            canonical_event_visible_message_text(event)?
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Imported session".to_string())
}

pub fn canonical_event_visible_message_role(event: &SessionEvent) -> Option<EventRole> {
    if matches!(
        event.kind,
        SessionEventKind::Lifecycle | SessionEventKind::Unknown
    ) {
        return None;
    }
    match event.role {
        EventRole::User | EventRole::Assistant | EventRole::Tool => Some(event.role),
        EventRole::System | EventRole::Developer | EventRole::Unknown => None,
    }
}

pub fn canonical_event_is_visible_message(event: &SessionEvent) -> bool {
    canonical_event_visible_message_role(event).is_some()
        && !canonical_event_visible_text(event).trim().is_empty()
}

pub fn canonical_event_visible_message_text(event: &SessionEvent) -> Option<String> {
    canonical_event_visible_message_role(event)?;
    let text = canonical_event_visible_text(event);
    (!text.trim().is_empty()).then_some(text)
}

pub fn canonical_event_instruction_context_text(event: &SessionEvent) -> Option<String> {
    if matches!(
        event.kind,
        SessionEventKind::Lifecycle | SessionEventKind::Unknown
    ) {
        return None;
    }
    if !matches!(event.role, EventRole::System | EventRole::Developer) {
        return None;
    }
    let text = canonical_event_visible_text(event);
    (!text.trim().is_empty()).then_some(text)
}

pub fn canonical_session_instruction_context_text(session: &CanonicalSession) -> Option<String> {
    let text = session
        .events
        .iter()
        .filter_map(canonical_event_instruction_context_text)
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.trim().is_empty()).then_some(text)
}

pub fn canonical_event_text(event: &SessionEvent) -> String {
    event
        .blocks
        .iter()
        .map(canonical_block_text)
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn canonical_visible_block_text(block: &EventBlock) -> Option<String> {
    if matches!(
        block,
        EventBlock::ProviderPayload { .. } | EventBlock::Unknown { .. }
    ) {
        return None;
    }
    let text = canonical_block_text(block);
    (!text.trim().is_empty()).then_some(text)
}

pub fn canonical_event_visible_text(event: &SessionEvent) -> String {
    event
        .blocks
        .iter()
        .filter_map(canonical_visible_block_text)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn canonical_block_text(block: &EventBlock) -> String {
    match block {
        EventBlock::Text { text } => text.clone(),
        EventBlock::Thinking { text, .. } => text.clone(),
        EventBlock::ToolCall {
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
        EventBlock::ToolResult {
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
        EventBlock::Patch {
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
        EventBlock::Command { command, argv, cwd } => {
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
        EventBlock::CommandResult {
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
        EventBlock::File { path, content, .. } => content
            .as_ref()
            .map(|content| format!("[File: {}]\n{}", path, content))
            .unwrap_or_else(|| format!("[File: {}]", path)),
        EventBlock::Image {
            mime_type,
            data,
            path,
        } => path
            .clone()
            .or_else(|| data.clone())
            .map(|value| format!("[Image: {}]\n{}", mime_type, value))
            .unwrap_or_else(|| format!("[Image: {}]", mime_type)),
        EventBlock::ProviderPayload { kind, payload } => {
            format!("[Provider payload: {}]\n{}", kind, payload)
        }
        EventBlock::Compressed {
            source_provider_id,
            summary,
            source_event_ids,
            source_event_count,
            archive_ref,
        } => {
            let mut parts = vec![
                format!("[Compressed session segment from {}]", source_provider_id),
                summary.clone(),
            ];
            let source_event_count = source_event_count.unwrap_or(source_event_ids.len());
            if source_event_count > 0 {
                parts.push(format!("Source event count: {}", source_event_count));
            }
            if let Some(archive_ref) = archive_ref {
                parts.push(format!("Archive: {}", archive_ref));
                parts.push(compression_retrieval_hint(archive_ref));
            }
            parts.join("\n")
        }
        EventBlock::Unknown { raw } => format!("[Unknown]\n{}", raw),
    }
}

pub fn compression_retrieval_hint(archive_ref: &str) -> String {
    format!(
        "Retrieve specific details with: memorph compression retrieve {} --query <terms> --max-results 5",
        archive_ref
    )
}

pub fn canonical_event_role_label(role: EventRole) -> &'static str {
    match role {
        EventRole::User => "user",
        EventRole::Assistant => "assistant",
        EventRole::Tool => "tool",
        EventRole::System => "system",
        EventRole::Developer => "developer",
        EventRole::Unknown => "unknown",
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
    fn compressed_block_text_keeps_provider_projection_concise() {
        let source_event_ids = (0..20)
            .map(|idx| format!("source-event-{}", idx))
            .collect::<Vec<_>>();
        let text = canonical_block_text(&EventBlock::Compressed {
            source_provider_id: "opencode".to_string(),
            summary: "compressed summary".to_string(),
            source_event_ids,
            source_event_count: None,
            archive_ref: Some("memorph-archive://session/archive.json".to_string()),
        });

        assert!(text.contains("compressed summary"));
        assert!(text.contains("Source event count: 20"));
        assert!(text.contains("memorph-archive://session/archive.json"));
        assert!(text.contains("memorph compression retrieve memorph-archive://session/archive.json --query <terms> --max-results 5"));
        assert!(!text.contains("source-event-19"));
    }

    #[test]
    fn canonical_visible_block_text_drops_provider_internal_blocks() {
        assert!(canonical_visible_block_text(&EventBlock::ProviderPayload {
            kind: "token_count".to_string(),
            payload: serde_json::json!({"input_tokens": 10}),
        })
        .is_none());
        assert!(canonical_visible_block_text(&EventBlock::Unknown {
            raw: serde_json::json!({"type": "mystery"}),
        })
        .is_none());
    }

    #[test]
    fn canonical_event_visible_text_omits_provider_internal_blocks() {
        let event = SessionEvent {
            id: "event-1".to_string(),
            kind: crate::canonical::SessionEventKind::Message,
            role: EventRole::User,
            timestamp: chrono::Utc::now(),
            links: crate::canonical::EventLinks::default(),
            blocks: vec![
                EventBlock::Text {
                    text: "hello".to_string(),
                },
                EventBlock::ProviderPayload {
                    kind: "token_count".to_string(),
                    payload: serde_json::json!({"input_tokens": 10}),
                },
                EventBlock::Unknown {
                    raw: serde_json::json!({"type": "mystery"}),
                },
            ],
            metadata: crate::canonical::EventMetadata {
                source: crate::canonical::EventSource {
                    provider_id: "codex".to_string(),
                    original_id: None,
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: std::collections::BTreeMap::new(),
            },
        };

        assert_eq!(canonical_event_visible_text(&event), "hello");
    }

    #[test]
    fn canonical_visible_message_role_excludes_internal_events() {
        let lifecycle = test_event(
            "lifecycle",
            crate::canonical::SessionEventKind::Lifecycle,
            EventRole::System,
            vec![EventBlock::Text {
                text: "internal".to_string(),
            }],
        );
        let developer = test_event(
            "developer",
            crate::canonical::SessionEventKind::Message,
            EventRole::Developer,
            vec![EventBlock::Text {
                text: "developer".to_string(),
            }],
        );
        let unknown = test_event(
            "unknown",
            crate::canonical::SessionEventKind::Unknown,
            EventRole::User,
            vec![EventBlock::Text {
                text: "unknown".to_string(),
            }],
        );
        let user = test_event(
            "user",
            crate::canonical::SessionEventKind::Message,
            EventRole::User,
            vec![EventBlock::Text {
                text: "hello".to_string(),
            }],
        );

        assert_eq!(canonical_event_visible_message_role(&lifecycle), None);
        assert_eq!(canonical_event_visible_message_role(&developer), None);
        assert_eq!(canonical_event_visible_message_role(&unknown), None);
        assert_eq!(
            canonical_event_visible_message_role(&user),
            Some(EventRole::User)
        );
        assert_eq!(
            canonical_event_visible_message_text(&user).as_deref(),
            Some("hello")
        );
    }

    #[test]
    fn canonical_export_report_uses_actual_block_kinds_and_target_fidelity() {
        let session = CanonicalSession {
            schema: crate::canonical::CanonicalSchema::default(),
            identity: crate::canonical::SessionIdentity {
                canonical_id: "canonical-1".to_string(),
                source_title: None,
            },
            provenance: crate::canonical::SessionProvenance {
                imported_at: chrono::Utc::now(),
                imported_by: None,
                primary_source: crate::canonical::ProviderSessionRef {
                    provider_id: "source".to_string(),
                    session_id: "session-1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: crate::canonical::SessionContext::default(),
            events: vec![test_event(
                "assistant",
                crate::canonical::SessionEventKind::Message,
                EventRole::Assistant,
                vec![
                    EventBlock::Text {
                        text: "answer".to_string(),
                    },
                    EventBlock::Thinking {
                        text: "reasoning".to_string(),
                        signature: None,
                    },
                    EventBlock::Command {
                        command: "cargo test".to_string(),
                        argv: Vec::new(),
                        cwd: None,
                    },
                ],
            )],
            artifacts: Vec::new(),
            extensions: std::collections::BTreeMap::new(),
        };
        let capabilities = ProviderCapabilities {
            export_fidelity: ProviderContentFidelity {
                text: Some(MappingDisposition::Preserved),
                thinking: Some(MappingDisposition::Normalized),
                tool_call: Some(MappingDisposition::Downgraded),
                tool_result: None,
                patch: None,
                image: None,
                file: None,
                compressed: None,
                provider_payload: None,
            },
            ..ProviderCapabilities::default()
        };

        let report = canonical_export_report("target", &session, capabilities);

        assert_eq!(report.overall, MappingDisposition::Downgraded);
        assert_eq!(report.issues.len(), 2);
        assert_eq!(report.issues[0].code, "thinking_export_normalized");
        assert_eq!(report.issues[1].code, "tool_call_export_downgraded");
    }

    #[test]
    fn canonical_session_title_uses_visible_user_or_assistant_message() {
        let session = CanonicalSession {
            schema: crate::canonical::CanonicalSchema {
                name: crate::canonical::CANONICAL_SCHEMA_NAME.to_string(),
                version: 1,
            },
            identity: crate::canonical::SessionIdentity {
                canonical_id: "canonical-1".to_string(),
                source_title: None,
            },
            provenance: crate::canonical::SessionProvenance {
                imported_at: chrono::Utc::now(),
                imported_by: None,
                primary_source: crate::canonical::ProviderSessionRef {
                    provider_id: "test".to_string(),
                    session_id: "session-1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: crate::canonical::SessionContext {
                workspace_dir: None,
                created_at: None,
                last_active_at: None,
                tags: Vec::new(),
            },
            events: vec![
                test_event(
                    "internal",
                    crate::canonical::SessionEventKind::Lifecycle,
                    EventRole::System,
                    vec![EventBlock::ProviderPayload {
                        kind: "internal".to_string(),
                        payload: serde_json::json!({"id": "should-not-title"}),
                    }],
                ),
                test_event(
                    "prompt",
                    crate::canonical::SessionEventKind::Message,
                    EventRole::User,
                    vec![EventBlock::Text {
                        text: "real prompt".to_string(),
                    }],
                ),
            ],
            artifacts: Vec::new(),
            extensions: std::collections::BTreeMap::new(),
        };

        assert_eq!(canonical_session_title(&session), "real prompt");
    }

    #[test]
    fn canonical_instruction_context_uses_only_system_or_developer_messages() {
        let session = CanonicalSession {
            schema: crate::canonical::CanonicalSchema {
                name: crate::canonical::CANONICAL_SCHEMA_NAME.to_string(),
                version: 1,
            },
            identity: crate::canonical::SessionIdentity {
                canonical_id: "canonical-1".to_string(),
                source_title: None,
            },
            provenance: crate::canonical::SessionProvenance {
                imported_at: chrono::Utc::now(),
                imported_by: None,
                primary_source: crate::canonical::ProviderSessionRef {
                    provider_id: "test".to_string(),
                    session_id: "session-1".to_string(),
                    source_path: None,
                },
                aliases: Vec::new(),
            },
            context: crate::canonical::SessionContext::default(),
            events: vec![
                test_event(
                    "system",
                    crate::canonical::SessionEventKind::Message,
                    EventRole::System,
                    vec![EventBlock::Text {
                        text: "system instructions".to_string(),
                    }],
                ),
                test_event(
                    "developer",
                    crate::canonical::SessionEventKind::Message,
                    EventRole::Developer,
                    vec![EventBlock::Text {
                        text: "developer instructions".to_string(),
                    }],
                ),
                test_event(
                    "internal",
                    crate::canonical::SessionEventKind::Lifecycle,
                    EventRole::System,
                    vec![EventBlock::Text {
                        text: "runtime context".to_string(),
                    }],
                ),
                test_event(
                    "payload",
                    crate::canonical::SessionEventKind::Message,
                    EventRole::System,
                    vec![EventBlock::ProviderPayload {
                        kind: "internal".to_string(),
                        payload: serde_json::json!({"text": "provider payload"}),
                    }],
                ),
                test_event(
                    "user",
                    crate::canonical::SessionEventKind::Message,
                    EventRole::User,
                    vec![EventBlock::Text {
                        text: "user prompt".to_string(),
                    }],
                ),
            ],
            artifacts: Vec::new(),
            extensions: std::collections::BTreeMap::new(),
        };

        assert_eq!(
            canonical_session_instruction_context_text(&session).as_deref(),
            Some("system instructions\n\ndeveloper instructions")
        );
    }

    fn test_event(
        id: &str,
        kind: crate::canonical::SessionEventKind,
        role: EventRole,
        blocks: Vec<EventBlock>,
    ) -> SessionEvent {
        SessionEvent {
            id: id.to_string(),
            kind,
            role,
            timestamp: chrono::Utc::now(),
            links: crate::canonical::EventLinks::default(),
            blocks,
            metadata: crate::canonical::EventMetadata {
                source: crate::canonical::EventSource {
                    provider_id: "test".to_string(),
                    original_id: None,
                    original_role: None,
                    phase: None,
                },
                model: None,
                usage: None,
                fidelity: MappingDisposition::Preserved,
                provider_ext: std::collections::BTreeMap::new(),
            },
        }
    }
}
