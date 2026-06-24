use super::{ProviderSettingModule, SettingDefinition};

pub struct WorkBuddySettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for WorkBuddySettingModule {
    fn provider_id(&self) -> &'static str {
        "workbuddy"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
