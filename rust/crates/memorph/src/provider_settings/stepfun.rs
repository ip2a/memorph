use super::{ProviderSettingModule, SettingDefinition};

pub struct StepFunSettingModule;

const SETTINGS: &[SettingDefinition] = &[];

impl ProviderSettingModule for StepFunSettingModule {
    fn provider_id(&self) -> &'static str {
        "stepfun"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }
}
