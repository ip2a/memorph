use super::{ProviderSettingModule, SettingDefinition};

pub struct AntiGravitySettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for AntiGravitySettingModule {
    fn provider_id(&self) -> &'static str {
        "antigravity"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
