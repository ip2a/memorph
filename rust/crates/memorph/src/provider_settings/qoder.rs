use super::{ProviderSettingModule, SettingDefinition};

pub struct QoderSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for QoderSettingModule {
    fn provider_id(&self) -> &'static str {
        "qoder"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
