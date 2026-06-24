use super::{ProviderSettingModule, SettingDefinition};

pub struct CodeBuddySettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for CodeBuddySettingModule {
    fn provider_id(&self) -> &'static str {
        "codebuddy"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
