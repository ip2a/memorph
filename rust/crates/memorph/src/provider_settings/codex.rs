use super::{
    ProviderSettingContext, ProviderSettingModule, ProviderSettingOutput, SettingDefinition,
    SettingKind, SettingScope,
};

pub struct CodexSettingModule;

const SETTINGS: &[SettingDefinition] = &[
    SettingDefinition {
        id: "repair_workspace_sessions",
        title: "Sync workspace sessions",
        description:
            "Sync Codex sessions for the current workspace when provider filtering hides them.",
        scope: SettingScope::Workspace,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "install_hook",
        title: "Install memorph hook",
        description: "Install memorph runtime hooks into Codex hooks.json and enable hooks.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "verify_hook",
        title: "Verify memorph hook",
        description: "Check whether Codex hook entries are installed and enabled.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "repair_hook",
        title: "Repair memorph hook",
        description: "Repair missing or disabled Codex hook entries managed by memorph.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
    SettingDefinition {
        id: "uninstall_hook",
        title: "Uninstall memorph hook",
        description: "Remove memorph-managed hook entries from Codex hooks.json.",
        scope: SettingScope::Global,
        kind: SettingKind::Action,
    },
];

impl ProviderSettingModule for CodexSettingModule {
    fn provider_id(&self) -> &'static str {
        "codex"
    }

    fn settings(&self) -> &'static [SettingDefinition] {
        SETTINGS
    }

    fn run(
        &self,
        setting_id: &str,
        context: ProviderSettingContext,
    ) -> anyhow::Result<ProviderSettingOutput> {
        match setting_id {
            "repair_workspace_sessions" => Ok(ProviderSettingOutput::CodexWorkspaceRepair(
                crate::providers::codex::repair_workspace_sessions(context.workspace.as_deref())?,
            )),
            "install_hook" => Ok(ProviderSettingOutput::HookOperation(
                crate::hooks::installer::install("codex")?,
            )),
            "verify_hook" => Ok(ProviderSettingOutput::HookOperation(
                crate::hooks::installer::verify("codex")?,
            )),
            "repair_hook" => Ok(ProviderSettingOutput::HookOperation(
                crate::hooks::installer::repair("codex")?,
            )),
            "uninstall_hook" => Ok(ProviderSettingOutput::HookOperation(
                crate::hooks::installer::uninstall("codex")?,
            )),
            _ => anyhow::bail!("Unknown Codex setting action: {}", setting_id),
        }
    }
}
