use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Unified memory model: Memorph session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorphSession {
    pub meta: MemorphMeta,
    pub session: SessionInfo,
    pub messages: Vec<MemorphMessage>,
}

/// Memorph format metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorphMeta {
    pub version: String,
    pub converted_from: String,
    pub converted_at: DateTime<Utc>,
    pub memorph_version: String,
    pub source_session_id: String,
    pub source_provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub converted_by: Option<String>,
}

impl Default for MemorphMeta {
    fn default() -> Self {
        Self {
            version: "1.0".to_string(),
            converted_from: String::new(),
            converted_at: Utc::now(),
            memorph_version: env!("CARGO_PKG_VERSION").to_string(),
            source_session_id: String::new(),
            source_provider: String::new(),
            converted_by: Some("memorph-cli".to_string()),
        }
    }
}

/// Basic session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_active_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Unified message model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorphMessage {
    pub id: String,
    pub role: MemorphRole,
    pub content: Vec<ContentBlock>,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MessageMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemorphRole {
    User,
    Assistant,
    Tool,
    System,
    Developer,
}

impl std::fmt::Display for MemorphRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemorphRole::User => write!(f, "user"),
            MemorphRole::Assistant => write!(f, "assistant"),
            MemorphRole::Tool => write!(f, "tool"),
            MemorphRole::System => write!(f, "system"),
            MemorphRole::Developer => write!(f, "developer"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    #[serde(flatten)]
    pub extra: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceMetadata {
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_role: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u64>,
}

/// Content block enum
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<Value>,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
    Image {
        mime_type: String,
        data: String,
    },
    File {
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        content: Option<String>,
    },
}

#[allow(dead_code)]
impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn thinking(thinking: impl Into<String>) -> Self {
        Self::Thinking {
            thinking: thinking.into(),
            signature: None,
        }
    }

    pub fn tool_use(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self::ToolUse {
            id: id.into(),
            name: name.into(),
            input: None,
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::ToolResult {
            tool_use_id: tool_use_id.into(),
            content: content.into(),
            is_error: Some(false),
        }
    }
}

/// Raw session scan metadata
#[derive(Debug, Clone)]
pub struct SessionMeta {
    pub session_id: String,
    pub title: Option<String>,
    pub project_dir: Option<String>,
    pub last_active_at: Option<i64>,
    pub source_path: Option<String>,
}
