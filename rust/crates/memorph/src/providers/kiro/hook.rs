use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

pub struct KiroHook;

pub static KIRO_HOOK: KiroHook = KiroHook;

#[derive(Debug, Clone, Copy)]
struct KiroHookEvent {
    name: &'static str,
    timeout_ms: u64,
    blocking: bool,
}

const EVENTS: &[KiroHookEvent] = &[
    event("agentSpawn", 5000, false),
    event("userPromptSubmit", 5000, true),
    event("preToolUse", 5000, false),
    event("postToolUse", 5000, true),
    event("stop", 5000, true),
];

const fn event(name: &'static str, timeout_ms: u64, blocking: bool) -> KiroHookEvent {
    KiroHookEvent {
        name,
        timeout_ms,
        blocking,
    }
}

impl ProviderHook for KiroHook {
    fn provider_id(&self) -> &'static str {
        "kiro"
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

fn kiro_agent_path() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".kiro")
        .join("agents")
        .join("memorph.json")
}

fn status() -> Result<HookInstallStatus> {
    let path = kiro_agent_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "kiro".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some("Kiro memorph agent file does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("kiro"),
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
            "Kiro memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Kiro memorph hooks are installed. Launch with `kiro --agent memorph`.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "kiro".to_string(),
        status: health,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at("kiro"),
    })
}

fn install() -> Result<HookOperationReport> {
    let path = kiro_agent_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Kiro agent directory: {}",
                parent.display()
            )
        })?;
    }

    let original = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let mut root = original.clone();
    if !root.contains_key("name") {
        root.insert("name".to_string(), Value::String("memorph".to_string()));
    }
    if !root.contains_key("description") {
        root.insert(
            "description".to_string(),
            Value::String(
                "Auto-generated by memorph. Launch with `kiro --agent memorph` to relay hook events."
                    .to_string(),
            ),
        );
    }
    let command_base = crate::hooks::shared::bridge_command_base()?;

    let hooks = crate::hooks::config_formats::json_hooks::ensure_object_field(&mut root, "hooks");
    for event in EVENTS {
        let entries =
            crate::hooks::config_formats::json_hooks::ensure_array_field(hooks, event.name);
        entries.retain(|entry| {
            !crate::hooks::config_formats::json_hooks::entry_contains_memorph_hook(entry)
        });
        let command = format!(
            "{} --managed-version {} --provider kiro --event {}{}",
            command_base,
            crate::hooks::shared::current_hook_managed_version(),
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "command": command,
            "matcher": "*",
            "timeout_ms": event.timeout_ms
        }));
    }

    let changed = root != original;
    crate::hooks::config_formats::json_hooks::write_json_object(&path, &root)?;
    let status = status()?;
    Ok(HookOperationReport {
        provider: "kiro".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some(
            "Kiro memorph agent hooks installed. Launch with `kiro --agent memorph`.".to_string(),
        ),
        status,
    })
}

fn uninstall() -> Result<HookOperationReport> {
    let path = kiro_agent_path();
    if !path.exists() {
        let status = status()?;
        return Ok(HookOperationReport {
            provider: "kiro".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Kiro memorph agent file does not exist.".to_string()),
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
        provider: "kiro".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Kiro memorph hook entries removed.".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = KIRO_HOOK.descriptor().expect("kiro descriptor");
        assert_eq!(descriptor.provider(), KIRO_HOOK.provider_id());
    }
}
