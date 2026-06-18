use crate::canonical::{
    CanonicalSession, EventBlock, EventRole, ExportedSession, ImportedSession, MappingDirection,
    MappingDisposition, MappingReport, SessionEvent, SessionEventKind,
};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ProviderSessionSummary {
    pub session_id: String,
    pub title: Option<String>,
    pub project_dir: Option<String>,
    pub last_active_at: Option<i64>,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCapabilities {
    pub scan: bool,
    pub import: bool,
    pub export: bool,
    pub delete: bool,
    pub rename: bool,
    pub resume: bool,
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

/// Provider trait: each AI coding tool implements this interface
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
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

pub fn canonical_export_report(provider_id: &str) -> MappingReport {
    MappingReport {
        provider_id: provider_id.to_string(),
        direction: MappingDirection::Export,
        overall: MappingDisposition::Preserved,
        issues: Vec::new(),
    }
}

pub fn canonical_export_result(
    provider_id: &str,
    session_id: String,
    resume_command: Option<String>,
) -> ExportedSession {
    ExportedSession {
        provider_id: provider_id.to_string(),
        session_id,
        resume_command,
        report: canonical_export_report(provider_id),
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
