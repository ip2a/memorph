use super::{ProviderSettingModule, SettingDefinition};

pub struct QwenSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for QwenSettingModule {
    fn provider_id(&self) -> &'static str {
        "qwen"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
