use super::{ProviderSettingModule, SettingDefinition, SettingKind, SettingScope};

pub struct OpenCodeSettingModule;

const SETTINGS: &[SettingDefinition] = &[SettingDefinition {
    id: "show_subagents",
    title: "Show subagents",
    description: "Expose OpenCode subagent sessions in provider-specific views.",
    scope: SettingScope::Global,
    kind: SettingKind::Toggle,
}];

impl ProviderSettingModule for OpenCodeSettingModule {
    fn provider_id(&self) -> &'static str {
        "opencode"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
