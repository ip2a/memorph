use super::{
    ProviderSettingContext, ProviderSettingModule, ProviderSettingOutput, SettingDefinition,
    SettingKind, SettingScope,
};

pub struct CursorSettingModule;

const SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "install_hook",
        title: "Install memorph hook",
        description: "Install memorph runtime hooks into Cursor hooks.json.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "verify_hook",
        title: "Verify memorph hook",
        description: "Check whether Cursor hook entries are installed and current.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "repair_hook",
        title: "Repair memorph hook",
        description: "Repair missing or stale Cursor hook entries managed by memorph.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "uninstall_hook",
        title: "Uninstall memorph hook",
        description: "Remove memorph-managed hook entries from Cursor hooks.json.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
];

impl ProviderSettingModule for CursorSettingModule {
    fn provider_id(&self) -> &'static str {
        "cursor"
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
            "install_hook" => crate::hooks::installer::install("cursor")?,
            "verify_hook" => crate::hooks::installer::verify("cursor")?,
            "repair_hook" => crate::hooks::installer::repair("cursor")?,
            "uninstall_hook" => crate::hooks::installer::uninstall("cursor")?,
            _ => anyhow::bail!("Unknown Cursor setting action: {}", setting_id),
        };
        Ok(ProviderSettingOutput::HookOperation(report))
    }
}
