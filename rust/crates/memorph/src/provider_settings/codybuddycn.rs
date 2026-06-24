use super::{ProviderSettingModule, SettingDefinition};

pub struct CodyBuddyCnSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for CodyBuddyCnSettingModule {
    fn provider_id(&self) -> &'static str {
        "codybuddycn"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
