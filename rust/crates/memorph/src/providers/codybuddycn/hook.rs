use std::path::PathBuf;

use anyhow::Result;

use crate::hooks::contract::ProviderHook;
use crate::hooks::json_settings_hook::{event, JsonSettingsHookEvent, JsonSettingsHookSpec};
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

pub struct CodyBuddyCnHook;

pub static CODYBUDDYCN_HOOK: CodyBuddyCnHook = CodyBuddyCnHook;

const EVENTS: &[JsonSettingsHookEvent] = &[
    event("UserPromptSubmit", 5, true),
    event("PreToolUse", 5, false),
    event("PostToolUse", 5, true),
    event("SessionStart", 5, false),
    event("SessionEnd", 5, true),
    event("Stop", 5, true),
    event("SubagentStart", 5, true),
    event("SubagentStop", 5, true),
    event("Notification", 86400, false),
    event("PreCompact", 5, true),
];

impl ProviderHook for CodyBuddyCnHook {
    fn provider_id(&self) -> &'static str {
        "codybuddycn"
    }

    fn status(&self) -> Result<HookInstallStatus> {
        crate::hooks::json_settings_hook::status(spec())
    }

    fn install(&self) -> Result<HookOperationReport> {
        crate::hooks::json_settings_hook::install(spec())
    }

    fn verify(&self) -> Result<HookOperationReport> {
        let status = self.status()?;
        Ok(HookOperationReport {
            provider: self.provider_id().to_string(),
            operation: "verify".to_string(),
            changed: false,
            backup_path: None,
            message: status.message.clone(),
            status,
        })
    }

    fn repair(&self) -> Result<HookOperationReport> {
        let before = self.status()?;
        let mut report = self.install()?;
        report.operation = "repair".to_string();
        report.changed = before.status != HookHealthStatus::InstalledOk;
        Ok(report)
    }

    fn uninstall(&self) -> Result<HookOperationReport> {
        crate::hooks::json_settings_hook::uninstall(spec())
    }
}

pub(crate) fn settings_path() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".codybuddycn")
        .join("settings.json")
}

fn spec() -> JsonSettingsHookSpec {
    JsonSettingsHookSpec {
        provider: "codybuddycn",
        display_name: "CodyBuddyCN",
        settings_path,
        events: EVENTS,
        missing_config_message: "CodyBuddyCN settings.json does not exist.",
        install_message: "CodyBuddyCN hook entries installed.",
        uninstall_missing_message: "CodyBuddyCN settings file does not exist.",
        uninstall_message: "CodyBuddyCN memorph hook entries removed.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = CODYBUDDYCN_HOOK
            .descriptor()
            .expect("codybuddycn descriptor");
        assert_eq!(descriptor.provider(), CODYBUDDYCN_HOOK.provider_id());
    }
}
