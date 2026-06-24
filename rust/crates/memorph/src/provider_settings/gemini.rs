use super::{ProviderSettingModule, SettingDefinition};

pub struct GeminiSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for GeminiSettingModule {
    fn provider_id(&self) -> &'static str {
        "gemini"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
