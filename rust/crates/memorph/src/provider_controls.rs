use anyhow::Result;
use serde_json::Value;

pub use crate::provider_settings::{
    ProviderSettingContext as ProviderControlContext, ProviderSettingItem as ProviderControl,
    ProviderSettingOutput as ProviderControlOutput, SettingKind as ControlKind,
    SettingScope as ControlScope,
};

pub fn list_provider_controls(provider_id: &str) -> Result<Vec<ProviderControl>> {
    crate::provider_settings::list_provider_settings(provider_id)
}

pub fn get_provider_control(provider_id: &str, control_id: &str) -> Result<ProviderControl> {
    crate::provider_settings::get_provider_setting(provider_id, control_id)
}

pub fn update_provider_control(
    provider_id: &str,
    control_id: &str,
    value: Option<Value>,
) -> Result<ProviderControl> {
    crate::provider_settings::update_provider_setting(provider_id, control_id, value)
}

pub fn run_provider_control(
    provider_id: &str,
    control_id: &str,
    context: ProviderControlContext,
) -> Result<ProviderControlOutput> {
    crate::provider_settings::run_provider_setting(provider_id, control_id, context)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_controls_stay_compatible_for_opencode_toggle() {
        let controls = list_provider_controls("opencode").unwrap();
        assert!(controls
            .iter()
            .any(|control| control.id == "show_subagents"));
    }

    #[test]
    fn provider_controls_stay_compatible_for_codex_action() {
        let controls = list_provider_controls("codex").unwrap();
        let control = controls
            .iter()
            .find(|control| control.id == "repair_workspace_sessions")
            .expect("missing codex repair control");
        assert_eq!(control.kind, ControlKind::Action);
        assert_eq!(control.scope, ControlScope::Workspace);
    }
}
