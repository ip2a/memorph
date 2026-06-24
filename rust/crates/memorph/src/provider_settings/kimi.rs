use super::{ProviderSettingModule, SettingDefinition};

pub struct KimiSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for KimiSettingModule {
    fn provider_id(&self) -> &'static str {
        "kimi"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
