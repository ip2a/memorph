use super::{ProviderSettingModule, SettingDefinition};

pub struct TraeSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for TraeSettingModule {
    fn provider_id(&self) -> &'static str {
        "trae"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
