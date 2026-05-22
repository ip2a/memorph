use crate::canonical::{
    CanonicalSession, EventBlock, EventRole, ExportedSession, ImportedSession, MappingDirection,
    MappingDisposition, MappingReport, SessionEvent,
};
use anyhow::Result;
use std::path::Path;

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

/// Provider trait: each AI coding tool implements this interface
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities::default()
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

    /// Estimate the storage size (in bytes) of a single session.
    /// Default returns 0 (unknown).
    fn session_size(&self, session_id: &str) -> Result<u64> {
        let _ = session_id;
        Ok(0)
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
            canonical_event_text(event)
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "Imported session".to_string())
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
        EventBlock::Unknown { raw } => format!("[Unknown]\n{}", raw),
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
