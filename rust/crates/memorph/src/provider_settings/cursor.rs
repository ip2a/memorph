use super::{ProviderSettingModule, SettingDefinition};

pub struct CursorSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for CursorSettingModule {
    fn provider_id(&self) -> &'static str {
        "cursor"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
