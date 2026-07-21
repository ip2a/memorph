use super::{
    ProviderSettingContext, ProviderSettingModule, ProviderSettingOutput, SettingDefinition,
    SettingKind, SettingScope,
};

pub struct CodexSettingModule;

const SETTINGS: &[SettingDefinition] = &[SettingDefinition {
    id: "repair_workspace_sessions",
    title: "Sync workspace sessions",
    description:
        "Sync Codex sessions for the current workspace when provider filtering hides them.",
    scope: SettingScope::Workspace,
    kind: SettingKind::Action,
}];

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
                crate::providers::codex::management::repair_workspace_sessions(
                    context.workspace.as_deref(),
                    context.actor,
                )?,
            )),
            _ => anyhow::bail!("Unknown Codex setting action: {}", setting_id),
        }
    }
}
