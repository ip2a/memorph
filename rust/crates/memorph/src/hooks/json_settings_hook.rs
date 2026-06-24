//! Shared operations for Claude-style JSON `settings.json` hook providers.
//!
//! Provider modules own their paths, events, and user-facing labels. This module
//! only owns the common JSON hook read/write/status/install/uninstall mechanics.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonSettingsHookEvent {
    pub name: &'static str,
    pub timeout: u64,
    pub blocking: bool,
}

pub const fn event(name: &'static str, timeout: u64, blocking: bool) -> JsonSettingsHookEvent {
    JsonSettingsHookEvent {
        name,
        timeout,
        blocking,
    }
}

#[derive(Clone, Copy)]
pub struct JsonSettingsHookSpec {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub settings_path: fn() -> PathBuf,
    pub events: &'static [JsonSettingsHookEvent],
    pub missing_config_message: &'static str,
    pub install_message: &'static str,
    pub uninstall_missing_message: &'static str,
    pub uninstall_message: &'static str,
}

pub fn status(spec: JsonSettingsHookSpec) -> Result<HookInstallStatus> {
    let path = (spec.settings_path)();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: spec.provider.to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some(spec.missing_config_message.to_string()),
            last_event_at: crate::hooks::health::last_event_at(spec.provider),
        });
    }

    let root = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    let missing: Vec<&str> = spec
        .events
        .iter()
        .map(|event| event.name)
        .filter(|event| {
            !crate::hooks::config_formats::json_hooks::event_has_memorph_hook(&root, event)
        })
        .collect();
    let installed_version =
        crate::hooks::health::summarize_versions(spec.events.iter().filter_map(|event| {
            crate::hooks::config_formats::json_hooks::event_memorph_hook_version(&root, event.name)
        }));
    let current_version = Some(crate::hooks::shared::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref()
            != Some(crate::hooks::shared::current_hook_managed_version());
    let health = if missing.is_empty() && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == spec.events.len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };
    let message = if stale {
        Some(format!(
            "{} memorph hooks are installed but stale: installed {}, current {}.",
            spec.display_name,
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some(format!(
            "{} memorph hooks are installed.",
            spec.display_name
        ))
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: spec.provider.to_string(),
        status: health,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at(spec.provider),
    })
}

pub fn install(spec: JsonSettingsHookSpec) -> Result<HookOperationReport> {
    let path = (spec.settings_path)();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create {} config directory: {}",
                spec.display_name,
                parent.display()
            )
        })?;
    }

    let original = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&path)?;
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let mut root = original.clone();
    let command_base = crate::hooks::shared::bridge_command_base()?;

    let hooks = crate::hooks::config_formats::json_hooks::ensure_object_field(&mut root, "hooks");
    for event in spec.events {
        let entries =
            crate::hooks::config_formats::json_hooks::ensure_array_field(hooks, event.name);
        entries.retain(|entry| {
            !crate::hooks::config_formats::json_hooks::entry_contains_memorph_hook(entry)
        });
        let command = format!(
            "{} --managed-version {} --provider {} --event {}{}",
            command_base,
            crate::hooks::shared::current_hook_managed_version(),
            spec.provider,
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
    let status = status(spec)?;
    Ok(HookOperationReport {
        provider: spec.provider.to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some(spec.install_message.to_string()),
        status,
    })
}

pub fn uninstall(spec: JsonSettingsHookSpec) -> Result<HookOperationReport> {
    let path = (spec.settings_path)();
    if !path.exists() {
        let status = status(spec)?;
        return Ok(HookOperationReport {
            provider: spec.provider.to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some(spec.uninstall_missing_message.to_string()),
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
    let status = status(spec)?;
    Ok(HookOperationReport {
        provider: spec.provider.to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some(spec.uninstall_message.to_string()),
    })
}
