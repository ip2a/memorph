use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

pub struct QoderHook;

pub static QODER_HOOK: QoderHook = QoderHook;

#[derive(Debug, Clone, Copy)]
struct QoderHookEvent {
    name: &'static str,
    timeout: u64,
    blocking: bool,
}

const EVENTS: &[QoderHookEvent] = &[
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

const fn event(name: &'static str, timeout: u64, blocking: bool) -> QoderHookEvent {
    QoderHookEvent {
        name,
        timeout,
        blocking,
    }
}

impl ProviderHook for QoderHook {
    fn provider_id(&self) -> &'static str {
        "qoder"
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

pub(crate) fn settings_path() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".qoder")
        .join("settings.json")
}

fn status() -> Result<HookInstallStatus> {
    let path = settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "qoder".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some("Qoder settings.json does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("qoder"),
        });
    }

    let root = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = EVENTS
        .iter()
        .map(|event| event.name)
        .filter(|event| {
            !crate::hooks::config_formats::json_hooks::event_has_memorph_hook(&root, event)
        })
        .collect();
    let installed_version =
        crate::hooks::health::summarize_versions(EVENTS.iter().filter_map(|event| {
            crate::hooks::config_formats::json_hooks::event_memorph_hook_version(&root, event.name)
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
            "Qoder memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Qoder memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "qoder".to_string(),
        status: health,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at("qoder"),
    })
}

fn install() -> Result<HookOperationReport> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Qoder config directory: {}",
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
            "{} --managed-version {} --provider qoder --event {}{}",
            command_base,
            crate::hooks::shared::current_hook_managed_version(),
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "matcher": "*",
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let changed = root != original;
    crate::hooks::config_formats::json_hooks::write_json_object(&path, &root)?;
    let status = status()?;
    Ok(HookOperationReport {
        provider: "qoder".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Qoder hook entries installed.".to_string()),
        status,
    })
}

fn uninstall() -> Result<HookOperationReport> {
    let path = settings_path();
    if !path.exists() {
        let status = status()?;
        return Ok(HookOperationReport {
            provider: "qoder".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Qoder settings file does not exist.".to_string()),
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
        provider: "qoder".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Qoder memorph hook entries removed.".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::test_support::TestHookHomeGuard;
    use serde_json::{Value, json};

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = QODER_HOOK.descriptor().expect("qoder descriptor");
        assert_eq!(descriptor.provider(), QODER_HOOK.provider_id());
    }
    #[test]
    fn repairs_stale_codeisland_claude_fork_provider_hook_version() {
        let _home = TestHookHomeGuard::new();
        QODER_HOOK.install().unwrap();
        let path = settings_path();
        let stale = std::fs::read_to_string(&path)
            .unwrap()
            .replace(crate::hooks::shared::HOOK_MANAGED_VERSION, "hook-old");
        crate::storage::atomic_write::write_string_atomic(&path, &stale).unwrap();

        assert_eq!(
            QODER_HOOK.verify().unwrap().status.status,
            HookHealthStatus::InstalledStaleBinary
        );
        let repaired = QODER_HOOK.repair().unwrap();
        assert_eq!(repaired.status.status, HookHealthStatus::InstalledOk);
        assert_eq!(
            QODER_HOOK
                .verify()
                .unwrap()
                .status
                .installed_version
                .as_deref(),
            Some(crate::hooks::shared::HOOK_MANAGED_VERSION)
        );
    }

    #[test]
    fn claude_fork_install_uninstall_preserves_foreign_json_hooks() {
        let _home = TestHookHomeGuard::new();
        let path = settings_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        crate::storage::atomic_write::write_string_atomic(
            &path,
            &serde_json::to_string_pretty(&json!({
                "theme": "dark",
                "hooks": {
                    "PreToolUse": [{
                        "matcher": "Bash",
                        "hooks": [{"type": "command", "command": "echo keep", "timeout": 1}]
                    }],
                    "CustomEvent": [{"command": "echo custom"}]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        QODER_HOOK.install().unwrap();
        let installed =
            crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path).unwrap();
        assert_eq!(installed.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(
            installed["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert!(
            crate::hooks::config_formats::json_hooks::event_has_memorph_hook(
                &installed,
                "PreToolUse"
            )
        );

        QODER_HOOK.uninstall().unwrap();
        let removed =
            crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path).unwrap();
        assert_eq!(removed.get("theme").and_then(Value::as_str), Some("dark"));
        assert_eq!(
            removed["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert_eq!(
            removed["hooks"]["CustomEvent"][0]["command"].as_str(),
            Some("echo custom")
        );
        assert!(
            !crate::hooks::config_formats::json_hooks::event_has_memorph_hook(
                &removed,
                "PreToolUse"
            )
        );
    }
}
