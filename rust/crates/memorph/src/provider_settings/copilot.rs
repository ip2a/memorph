use super::{ProviderSettingModule, SettingDefinition};

pub struct CopilotSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for CopilotSettingModule {
    fn provider_id(&self) -> &'static str {
        "copilot"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
