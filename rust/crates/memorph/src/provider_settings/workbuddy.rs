use super::{
    ProviderSettingContext, ProviderSettingModule, ProviderSettingOutput, SettingDefinition,
    SettingKind, SettingScope,
};

pub struct WorkBuddySettingModule;

const SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "install_hook",
        title: "Install memorph hook",
        description: "Install memorph runtime hooks into WorkBuddy settings.json.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "verify_hook",
        title: "Verify memorph hook",
        description: "Check whether WorkBuddy hook entries are installed and current.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "repair_hook",
        title: "Repair memorph hook",
        description: "Repair missing or stale WorkBuddy hook entries managed by memorph.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "uninstall_hook",
        title: "Uninstall memorph hook",
        description: "Remove memorph-managed hook entries from WorkBuddy settings.json.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
];

impl ProviderSettingModule for WorkBuddySettingModule {
    fn provider_id(&self) -> &'static str {
        "workbuddy"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }

    fn run(
        &self,
        setting_id: &str,
        _context: ProviderSettingContext,
    ) -> anyhow::Result<ProviderSettingOutput> {
        let report = match setting_id {
            "install_hook" => crate::hooks::installer::install("workbuddy")?,
            "verify_hook" => crate::hooks::installer::verify("workbuddy")?,
            "repair_hook" => crate::hooks::installer::repair("workbuddy")?,
            "uninstall_hook" => crate::hooks::installer::uninstall("workbuddy")?,
            _ => anyhow::bail!("Unknown WorkBuddy setting action: {}", setting_id),
        };
        Ok(ProviderSettingOutput::HookOperation(report))
    }
}
