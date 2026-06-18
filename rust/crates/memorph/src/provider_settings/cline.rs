use super::{
    ProviderSettingContext, ProviderSettingModule, ProviderSettingOutput, SettingDefinition,
    SettingKind, SettingScope,
};

pub struct ClineSettingModule;

const SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "install_hook",
        title: "Install memorph hook",
        description: "Install memorph runtime hooks into Cline's global Rules/Hooks directory.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "verify_hook",
        title: "Verify memorph hook",
        description: "Check whether Cline hook files are installed and current.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "repair_hook",
        title: "Repair memorph hook",
        description: "Repair missing or stale Cline hook files managed by memorph.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "uninstall_hook",
        title: "Uninstall memorph hook",
        description: "Remove memorph-managed Cline hook files.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
];

impl ProviderSettingModule for ClineSettingModule {
    fn provider_id(&self) -> &'static str {
        "cline"
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
            "install_hook" => crate::hooks::installer::install("cline")?,
            "verify_hook" => crate::hooks::installer::verify("cline")?,
            "repair_hook" => crate::hooks::installer::repair("cline")?,
            "uninstall_hook" => crate::hooks::installer::uninstall("cline")?,
            _ => anyhow::bail!("Unknown Cline setting action: {}", setting_id),
        };
        Ok(ProviderSettingOutput::HookOperation(report))
    }
}
