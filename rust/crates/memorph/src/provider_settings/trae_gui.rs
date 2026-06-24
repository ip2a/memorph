use super::{ProviderSettingModule, SettingDefinition};

pub struct TraeGuiSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for TraeGuiSettingModule {
    fn provider_id(&self) -> &'static str {
        "trae_gui"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
