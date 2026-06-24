use super::{ProviderSettingModule, SettingDefinition};

pub struct DroidSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for DroidSettingModule {
    fn provider_id(&self) -> &'static str {
        "droid"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
