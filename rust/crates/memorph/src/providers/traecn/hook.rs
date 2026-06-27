use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FlatHookEvent {
    name: &'static str,
    blocking: bool,
}

const EVENTS: &[FlatHookEvent] = &[
    event("beforeSubmitPrompt", false),
    event("beforeShellExecution", false),
    event("afterShellExecution", false),
    event("beforeReadFile", false),
    event("afterFileEdit", false),
    event("beforeMCPExecution", false),
    event("afterMCPExecution", false),
    event("afterAgentThought", false),
    event("afterAgentResponse", false),
    event("stop", false),
];

const fn event(name: &'static str, blocking: bool) -> FlatHookEvent {
    FlatHookEvent { name, blocking }
}

pub struct TraeCnHook;

pub static TRAECN_HOOK: TraeCnHook = TraeCnHook;

impl ProviderHook for TraeCnHook {
    fn provider_id(&self) -> &'static str {
        "traecn"
    }

    fn status(&self) -> Result<HookInstallStatus> {
        status()
    }

    fn install(&self) -> Result<HookOperationReport> {
        install()
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
        uninstall()
    }
}

pub(crate) fn hooks_path() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".trae-cn")
        .join("hooks.json")
}

fn required_events() -> impl Iterator<Item = &'static str> {
    EVENTS.iter().map(|event| event.name)
}

fn status() -> Result<HookInstallStatus> {
    let path = hooks_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "traecn".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some("Trae CN hooks.json does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("traecn"),
        });
    }

    let root = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = required_events()
        .filter(|event| {
            !crate::hooks::config_formats::json_hooks::event_has_memorph_hook(&root, event)
        })
        .collect();
    let installed_version =
        crate::hooks::health::summarize_versions(required_events().filter_map(|event| {
            crate::hooks::config_formats::json_hooks::event_memorph_hook_version(&root, event)
        }));
    let current_version = Some(crate::hooks::shared::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref()
            != Some(crate::hooks::shared::current_hook_managed_version());
    let health = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == EVENTS.len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "Trae CN memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Trae CN memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "traecn".to_string(),
        status: health,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at("traecn"),
    })
}

fn install() -> Result<HookOperationReport> {
    let path = hooks_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Trae CN hook directory: {}",
                parent.display()
            )
        })?;
    }

    let original = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = crate::hooks::shared::bridge_command_base()?;

    let hooks = crate::hooks::config_formats::json_hooks::ensure_object_field(&mut root, "hooks");
    for event in EVENTS {
        let entries =
            crate::hooks::config_formats::json_hooks::ensure_array_field(hooks, event.name);
        entries.retain(|entry| {
            !crate::hooks::config_formats::json_hooks::entry_contains_memorph_hook(entry)
        });
        let command = format!(
            "{} --managed-version {} --provider traecn --event {}{}",
            command_base,
            crate::hooks::shared::current_hook_managed_version(),
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({ "command": command }));
    }

    let changed = root != original;
    crate::hooks::config_formats::json_hooks::write_json_object(&path, &root)?;
    let status = status()?;
    Ok(HookOperationReport {
        provider: "traecn".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Trae CN hook entries installed.".to_string()),
        status,
    })
}

fn uninstall() -> Result<HookOperationReport> {
    let path = hooks_path();
    if !path.exists() {
        let status = status()?;
        return Ok(HookOperationReport {
            provider: "traecn".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Trae CN hooks.json file does not exist.".to_string()),
        });
    }

    let original = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let mut root = original.clone();
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        let keys: Vec<String> = hooks.keys().cloned().collect();
        for key in keys {
            if let Some(entries) = hooks.get_mut(&key).and_then(Value::as_array_mut) {
                entries.retain(|entry| {
                    !crate::hooks::config_formats::json_hooks::entry_contains_memorph_hook(entry)
                });
                if entries.is_empty() {
                    hooks.remove(&key);
                }
            }
        }
        if hooks.is_empty() {
            root.remove("hooks");
        }
    }

    let changed = root != original;
    if changed {
        crate::hooks::config_formats::json_hooks::write_json_object(&path, &root)?;
    }
    let status = status()?;
    Ok(HookOperationReport {
        provider: "traecn".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Trae CN memorph hook entries removed.".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::HookHealthStatus;
    use crate::hooks::test_support::TestHookHomeGuard;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = TRAECN_HOOK.descriptor().expect("traecn descriptor");
        assert_eq!(descriptor.provider(), TRAECN_HOOK.provider_id());
    }
    #[test]
    fn installs_and_uninstalls_traecn_flat_hooks() {
        let _home = TestHookHomeGuard::new();
        assert_eq!(
            TRAECN_HOOK.verify().unwrap().status.status,
            HookHealthStatus::NotInstalled
        );
        let installed = TRAECN_HOOK.install().unwrap();
        assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
        assert!(installed.changed);

        let path = hooks_path();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("__hook-bridge"));
        assert!(contents.contains("--provider traecn"));

        let removed = TRAECN_HOOK.uninstall().unwrap();
        assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
    }
}
