use std::fs;
use std::path::PathBuf;

use anyhow::{Context as _, Result};
use serde_json::{json, Value};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

pub struct ClaudeHook;

pub static CLAUDE_HOOK: ClaudeHook = ClaudeHook;

#[derive(Debug, Clone, Copy)]
struct ClaudeHookEvent {
    name: &'static str,
    timeout: u64,
    blocking: bool,
}

const EVENTS: &[ClaudeHookEvent] = &[
    event("UserPromptSubmit", 5, false),
    event("PreToolUse", 5, true),
    event("PostToolUse", 5, false),
    event("PostToolUseFailure", 5, false),
    event("PermissionRequest", 86400, true),
    event("Stop", 5, false),
    event("SubagentStart", 5, false),
    event("SubagentStop", 5, false),
    event("SessionStart", 5, false),
    event("SessionEnd", 5, false),
    event("Notification", 86400, false),
    event("PreCompact", 5, false),
];

const fn event(name: &'static str, timeout: u64, blocking: bool) -> ClaudeHookEvent {
    ClaudeHookEvent {
        name,
        timeout,
        blocking,
    }
}

impl ProviderHook for ClaudeHook {
    fn provider_id(&self) -> &'static str {
        "claude"
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

fn claude_settings_path() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".claude")
        .join("settings.json")
}

fn status() -> Result<HookInstallStatus> {
    let path = claude_settings_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "claude".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some("Claude settings.json does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("claude"),
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
            "Claude memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Claude memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "claude".to_string(),
        status: health,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at("claude"),
    })
}

fn install() -> Result<HookOperationReport> {
    let path = claude_settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Claude config directory: {}",
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
            "{} --managed-version {} --provider claude --event {}{}",
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
        provider: "claude".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Claude hook entries installed.".to_string()),
        status,
    })
}

fn uninstall() -> Result<HookOperationReport> {
    let path = claude_settings_path();
    if !path.exists() {
        let status = status()?;
        return Ok(HookOperationReport {
            provider: "claude".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Claude settings file does not exist.".to_string()),
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
        provider: "claude".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Claude memorph hook entries removed.".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = CLAUDE_HOOK.descriptor().expect("claude descriptor");
        assert_eq!(descriptor.provider(), CLAUDE_HOOK.provider_id());
    }
}
