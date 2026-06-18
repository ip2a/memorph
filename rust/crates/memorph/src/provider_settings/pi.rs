use super::{
    ProviderSettingContext, ProviderSettingModule, ProviderSettingOutput, SettingDefinition,
    SettingKind, SettingScope,
};

pub struct PiSettingModule;

const SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "install_hook",
        title: "Install memorph hook",
        description: "Install memorph runtime hooks into pi extension.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "verify_hook",
        title: "Verify memorph hook",
        description: "Check whether pi hook entries are installed and current.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "repair_hook",
        title: "Repair memorph hook",
        description: "Repair missing or stale pi hook entries managed by memorph.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "uninstall_hook",
        title: "Uninstall memorph hook",
        description: "Remove memorph-managed hook entries from pi extension.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
];

impl ProviderSettingModule for PiSettingModule {
    fn provider_id(&self) -> &'static str {
        "pi"
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
            "install_hook" => crate::hooks::installer::install("pi")?,
            "verify_hook" => crate::hooks::installer::verify("pi")?,
            "repair_hook" => crate::hooks::installer::repair("pi")?,
            "uninstall_hook" => crate::hooks::installer::uninstall("pi")?,
            _ => anyhow::bail!("Unknown pi setting action: {}", setting_id),
        };
        Ok(ProviderSettingOutput::HookOperation(report))
    }
}
