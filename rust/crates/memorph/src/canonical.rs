use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const CANONICAL_SCHEMA_NAME: &str = "memorph-canonical";
pub const CANONICAL_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CanonicalSchema {
    pub name: String,
    pub version: u32,
}

impl Default for CanonicalSchema {
    fn default() -> Self {
        Self {
            name: CANONICAL_SCHEMA_NAME.to_string(),
            version: CANONICAL_SCHEMA_VERSION,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalSession {
    #[serde(default)]
    pub schema: CanonicalSchema,
    pub identity: SessionIdentity,
    pub provenance: SessionProvenance,
    #[serde(default)]
    pub context: SessionContext,
    #[serde(default)]
    pub events: Vec<SessionEvent>,
    #[serde(default)]
    pub artifacts: Vec<SessionArtifact>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

impl CanonicalSession {
    pub fn primary_title(&self) -> Option<&str> {
        self.identity
            .source_title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionIdentity {
    pub canonical_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionProvenance {
    pub imported_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_by: Option<String>,
    pub primary_source: ProviderSessionRef,
    #[serde(default)]
    pub aliases: Vec<ProviderSessionRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderSessionRef {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEvent {
    pub id: String,
    pub kind: SessionEventKind,
    pub role: EventRole,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub links: EventLinks,
    #[serde(default)]
    pub blocks: Vec<EventBlock>,
    pub metadata: EventMetadata,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEventKind {
    Message,
    ToolCall,
    ToolResult,
    Command,
    CommandResult,
    Patch,
    Lifecycle,
    Artifact,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventRole {
    User,
    Assistant,
    Tool,
    System,
    Developer,
    Unknown,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventLinks {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_index: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_event_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMetadata {
    pub source: EventSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageStats>,
    pub fidelity: MappingDisposition,
    #[serde(default)]
    pub provider_ext: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventSource {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_role: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventBlock {
    Text {
        text: String,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolCall {
        tool_call_id: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    Patch {
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        diff_text: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        hash: Option<String>,
    },
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        argv: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    CommandResult {
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stdout: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stderr: Option<String>,
    },
    File {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    Image {
        mime_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    ProviderPayload {
        kind: String,
        payload: Value,
    },
    Compressed {
        source_provider_id: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        source_event_ids: Vec<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        source_event_count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        archive_ref: Option<String>,
    },
    Unknown {
        raw: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionArtifact {
    pub id: String,
    pub kind: ArtifactKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    File,
    Image,
    Patch,
    Attachment,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalSessionState {
    pub locator: SessionLocator,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_targets: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspace_overrides: Vec<WorkspaceSessionState>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionLocator {
    pub provider_id: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSessionState {
    pub workspace_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preferred_targets: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedSession {
    pub session: CanonicalSession,
    pub report: MappingReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSession {
    pub provider_id: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    pub report: MappingReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingReport {
    pub provider_id: String,
    pub direction: MappingDirection,
    pub overall: MappingDisposition,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<MappingIssue>,
}

impl MappingReport {
    pub fn new(provider_id: impl Into<String>, direction: MappingDirection) -> Self {
        Self {
            provider_id: provider_id.into(),
            direction,
            overall: MappingDisposition::Preserved,
            issues: Vec::new(),
        }
    }

    pub fn push_issue(&mut self, issue: MappingIssue) {
        self.overall = self.overall.worst(issue.disposition);
        self.issues.push(issue);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingDirection {
    Import,
    Export,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum MappingDisposition {
    Preserved,
    Normalized,
    Downgraded,
    Dropped,
    Unsupported,
}

impl MappingDisposition {
    pub fn worst(self, other: Self) -> Self {
        self.max(other)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingIssue {
    pub level: MappingIssueLevel,
    pub disposition: MappingDisposition,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingIssueLevel {
    Info,
    Warning,
    Error,
}
