use super::{ProviderSettingModule, SettingDefinition};

pub struct HermesSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for HermesSettingModule {
    fn provider_id(&self) -> &'static str {
        "hermes"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
