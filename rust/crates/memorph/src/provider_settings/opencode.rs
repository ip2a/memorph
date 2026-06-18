use super::{
    ProviderSettingContext, ProviderSettingModule, ProviderSettingOutput, SettingDefinition,
    SettingKind, SettingScope,
};

pub struct OpenCodeSettingModule;

const SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "show_subagents",
        title: "Show subagents",
        description: "Expose OpenCode subagent sessions in provider-specific views.",
        scope: SettingScope::Global,
        kind: SettingKind::Toggle,
    },
    SettingDefinition {
        id: "install_hook",
        title: "Install memorph hook",
        description: "Install the memorph OpenCode plugin for runtime session events.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "verify_hook",
        title: "Verify memorph hook",
        description: "Check whether the memorph OpenCode plugin is installed and registered.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "repair_hook",
        title: "Repair memorph hook",
        description: "Repair the memorph OpenCode plugin and config registration.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "uninstall_hook",
        title: "Uninstall memorph hook",
        description: "Remove the memorph OpenCode plugin and config registration.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
];

impl ProviderSettingModule for OpenCodeSettingModule {
    fn provider_id(&self) -> &'static str {
        "opencode"
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
            "install_hook" => crate::hooks::installer::install("opencode")?,
            "verify_hook" => crate::hooks::installer::verify("opencode")?,
            "repair_hook" => crate::hooks::installer::repair("opencode")?,
            "uninstall_hook" => crate::hooks::installer::uninstall("opencode")?,
            _ => anyhow::bail!("Unknown OpenCode setting action: {}", setting_id),
        };
        Ok(ProviderSettingOutput::HookOperation(report))
    }
}
