use super::{ProviderSettingModule, SettingDefinition};
use crate::provider_config::claude;

/// Claude Code has no toggle/action settings of its own; its `View`-kind settings
/// (MCP servers, plugins, status line) are declared where they are inspected, in
/// [`crate::provider_config::claude`], so the two cannot drift apart.
pub struct ClaudeSettingModule;

impl ProviderSettingModule for ClaudeSettingModule {
    fn provider_id(&self) -> &'static str {
        "claude"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        claude::VIEW_SETTINGS
    }
}
