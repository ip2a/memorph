use super::{ProviderSettingModule, SettingDefinition};

pub struct ClineSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for ClineSettingModule {
    fn provider_id(&self) -> &'static str {
        "cline"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
