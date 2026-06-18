use super::{
    ProviderSettingContext, ProviderSettingModule, ProviderSettingOutput, SettingDefinition,
    SettingKind, SettingScope,
};

pub struct OmpSettingModule;

const SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "install_hook",
        title: "Install memorph hook",
        description: "Install memorph runtime hooks into Oh My Pi extension.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "verify_hook",
        title: "Verify memorph hook",
        description: "Check whether Oh My Pi hook entries are installed and current.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "repair_hook",
        title: "Repair memorph hook",
        description: "Repair missing or stale Oh My Pi hook entries managed by memorph.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "uninstall_hook",
        title: "Uninstall memorph hook",
        description: "Remove memorph-managed hook entries from Oh My Pi extension.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
];

impl ProviderSettingModule for OmpSettingModule {
    fn provider_id(&self) -> &'static str {
        "omp"
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
            "install_hook" => crate::hooks::installer::install("omp")?,
            "verify_hook" => crate::hooks::installer::verify("omp")?,
            "repair_hook" => crate::hooks::installer::repair("omp")?,
            "uninstall_hook" => crate::hooks::installer::uninstall("omp")?,
            _ => anyhow::bail!("Unknown Oh My Pi setting action: {}", setting_id),
        };
        Ok(ProviderSettingOutput::HookOperation(report))
    }
}
