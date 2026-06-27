use std::path::PathBuf;

use anyhow::Result;

use crate::hooks::contract::ProviderHook;
use crate::hooks::json_settings_hook::{event, JsonSettingsHookEvent, JsonSettingsHookSpec};
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

pub struct AntiGravityHook;

pub static ANTIGRAVITY_HOOK: AntiGravityHook = AntiGravityHook;

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

impl ProviderHook for AntiGravityHook {
    fn provider_id(&self) -> &'static str {
        "antigravity"
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
        .join(".antigravity")
        .join("settings.json")
}

fn spec() -> JsonSettingsHookSpec {
    JsonSettingsHookSpec {
        provider: "antigravity",
        display_name: "AntiGravity",
        settings_path,
        events: EVENTS,
        missing_config_message: "AntiGravity settings.json does not exist.",
        install_message: "AntiGravity hook entries installed.",
        uninstall_missing_message: "AntiGravity settings file does not exist.",
        uninstall_message: "AntiGravity memorph hook entries removed.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = ANTIGRAVITY_HOOK
            .descriptor()
            .expect("antigravity descriptor");
        assert_eq!(descriptor.provider(), ANTIGRAVITY_HOOK.provider_id());
    }
}
