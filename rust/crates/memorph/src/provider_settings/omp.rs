use super::{ProviderSettingModule, SettingDefinition};

pub struct OmpSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for OmpSettingModule {
    fn provider_id(&self) -> &'static str {
        "omp"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
