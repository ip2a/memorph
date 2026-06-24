use super::{ProviderSettingModule, SettingDefinition};

pub struct ClaudeSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for ClaudeSettingModule {
    fn provider_id(&self) -> &'static str {
        "claude"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
