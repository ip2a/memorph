//! Read-only inspection of provider configuration surfaces — MCP servers, plugins,
//! status line, and the like. Each provider declares the surfaces it exposes as
//! `View`-kind provider settings; this module produces the structured, secret-redacted
//! content those views render on the agent page.
//!
//! ## Architecture
//!
//! - **Declaration = inspection source of truth.** Each provider module owns a
//!   `VIEW_SETTINGS: &[SettingDefinition]` table (the `View` settings the agent page
//!   advertises) alongside the inspectors that fill them. `provider_settings` hands
//!   that slice out unchanged, so what the UI lists and what the backend can inspect
//!   can never drift apart.
//! - **Inspection is read-only and best-effort.** A missing config file yields an
//!   empty view with an advisory issue, never an error — the page still renders.
//!   Only an unsupported provider or view id is an error.
//! - **Non-blocking by construction.** Declarations ride the existing agent-detail
//!   payload (cheap static metadata); the content is fetched on demand through a
//!   dedicated lazy endpoint, so opening a panel never blocks the page.
//! - **Secrets never surface.** Model endpoints, URLs and API keys are not declared
//!   as views. [`redaction`] masks anything credential-labelled as a backstop.
//!
//! ## Acceptance
//!
//! A declared view for a provider resolves to a `ConfigView`; secret-labelled values
//! are masked in the serialized output; other providers report unsupported cleanly.

pub mod claude;
pub mod redaction;

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// One read-only configuration surface rendered as labeled sections of facts.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub provider_id: String,
    pub view_id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<ConfigSource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sections: Vec<ConfigSection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub issues: Vec<ConfigIssue>,
}

/// A file the view was assembled from, with its scope and existence.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSource {
    pub path: String,
    pub scope: &'static str,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSection {
    pub label: String,
    pub rows: Vec<ConfigRow>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRow {
    pub label: String,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<ConfigTone>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigTone {
    Ok,
    Warning,
    Danger,
    Muted,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigIssue {
    pub message: String,
    pub tone: ConfigTone,
}

impl ConfigView {
    pub fn new(provider_id: &str, view_id: &str, title: &str) -> Self {
        Self {
            provider_id: provider_id.to_string(),
            view_id: view_id.to_string(),
            title: title.to_string(),
            sources: Vec::new(),
            sections: Vec::new(),
            issues: Vec::new(),
        }
    }

    pub fn push_section(&mut self, label: impl Into<String>, rows: Vec<ConfigRow>) {
        self.sections.push(ConfigSection {
            label: label.into(),
            rows,
        });
    }

    pub fn push_issue(&mut self, tone: ConfigTone, message: impl Into<String>) {
        self.issues.push(ConfigIssue {
            tone,
            message: message.into(),
        });
    }
}

impl ConfigRow {
    pub fn fact(label: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            hint: None,
            tone: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_tone(mut self, tone: ConfigTone) -> Self {
        self.tone = Some(tone);
        self
    }
}

/// Inspect one configuration surface. The result is always redacted before return.
///
/// Returns an error only for an unsupported provider or unknown view id; a provider
/// whose config files are absent gets back an empty view with an advisory issue.
pub fn inspect(provider_id: &str, view_id: &str) -> Result<ConfigView> {
    let canonical = crate::providers::canonical_provider_id(provider_id);
    let mut view = match canonical.as_str() {
        "claude" => claude::inspect(view_id)?,
        other => anyhow::bail!("Config inspection is not supported for provider: {other}"),
    };
    redaction::redact(&mut view);
    Ok(view)
}

/// Read a JSON file, returning `None` if it is missing or unparseable.
pub(crate) fn read_json(path: &Path) -> Option<Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Render a path in `~/`-prefixed user-visible form.
pub(crate) fn user_visible(path: &Path) -> String {
    crate::utils::user_visible_path(&path.to_string_lossy())
}
