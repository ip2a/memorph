use super::{ProviderSettingModule, SettingDefinition};

pub struct KiroSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for KiroSettingModule {
    fn provider_id(&self) -> &'static str {
        "kiro"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
