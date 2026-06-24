use super::{ProviderSettingModule, SettingDefinition};

pub struct TraeCnSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for TraeCnSettingModule {
    fn provider_id(&self) -> &'static str {
        "traecn"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
