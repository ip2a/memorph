use anyhow::Result;

use super::{ProviderSettingOutput, SettingDefinition, SettingKind, SettingScope};

const COMMON_HOOK_SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "install_hook",
        title: "Install memorph hook",
        description: "Install memorph-managed runtime hooks for this provider.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "verify_hook",
        title: "Verify memorph hook",
        description: "Check whether memorph-managed provider hooks are installed and current.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "repair_hook",
        title: "Repair memorph hook",
        description: "Repair missing or stale memorph-managed provider hook entries.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "uninstall_hook",
        title: "Uninstall memorph hook",
        description: "Remove memorph-managed provider hook entries.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
];

pub(super) fn definitions_for(provider_id: &str) -> Vec<&'static SettingDefinition> {
    if !crate::hooks::registry::supports_provider(provider_id) {
        return Vec::new();
    }

    COMMON_HOOK_SETTINGS
        .iter()
        .filter(|setting| crate::hooks::capabilities::supports_setting(provider_id, setting.id))
        .collect()
}

pub(super) fn is_common_hook_setting(setting_id: &str) -> bool {
    COMMON_HOOK_SETTINGS
        .iter()
        .any(|setting| setting.id == setting_id)
}

pub(super) fn run(provider_id: &str, setting_id: &str) -> Result<ProviderSettingOutput> {
    if !crate::hooks::capabilities::supports_setting(provider_id, setting_id) {
        anyhow::bail!(
            "Hook setting is not supported for provider: {}.{}",
            provider_id,
            setting_id
        );
    }

    let report = crate::hooks::operations::run_setting_operation(provider_id, setting_id)?;
    Ok(ProviderSettingOutput::HookOperation(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_hook_actions_follow_hook_descriptor_capabilities() {
        for descriptor in crate::hooks::registry::all() {
            let settings = definitions_for(descriptor.provider());
            for setting in &settings {
                assert!(crate::hooks::capabilities::supports_setting(
                    descriptor.provider(),
                    setting.id
                ));
                assert_eq!(setting.kind, SettingKind::Action);
                assert_eq!(setting.scope, SettingScope::Global);
            }
        }
    }
}
