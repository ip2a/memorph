use super::{ProviderSettingModule, SettingDefinition};

pub struct PiSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for PiSettingModule {
    fn provider_id(&self) -> &'static str {
        "pi"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
