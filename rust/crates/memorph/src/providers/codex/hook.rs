use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};

pub struct CodexHook;

pub static CODEX_HOOK: CodexHook = CodexHook;

#[derive(Debug, Clone, Copy)]
struct CodexHookEvent {
    name: &'static str,
    timeout: u64,
    blocking: bool,
}

const EVENTS: &[CodexHookEvent] = &[
    event("SessionStart", 5, false),
    event("SessionEnd", 5, false),
    event("UserPromptSubmit", 5, false),
    event("PreToolUse", 5, false),
    event("PostToolUse", 5, false),
    event("PermissionRequest", 86400, true),
    event("Stop", 5, false),
];

const fn event(name: &'static str, timeout: u64, blocking: bool) -> CodexHookEvent {
    CodexHookEvent {
        name,
        timeout,
        blocking,
    }
}

impl ProviderHook for CodexHook {
    fn provider_id(&self) -> &'static str {
        "codex"
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

pub(crate) fn home() -> PathBuf {
    std::env::var("CODEX_HOME")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::hooks::shared::hook_home_dir().join(".codex"))
}

pub(crate) fn hooks_path() -> PathBuf {
    home().join("hooks.json")
}

pub(crate) fn config_path() -> PathBuf {
    home().join("config.toml")
}

fn status() -> Result<HookInstallStatus> {
    let hooks_path = hooks_path();
    let config_path = config_path();
    let config_path_text = Some(hooks_path.display().to_string());
    if !hooks_path.exists() {
        return Ok(HookInstallStatus {
            provider: "codex".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path: config_path_text,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some("Codex hooks.json does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("codex"),
        });
    }

    let root = crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&hooks_path)?;
    let missing: Vec<&str> = EVENTS
        .iter()
        .map(|event| event.name)
        .filter(|event| {
            !crate::hooks::config_formats::json_hooks::event_has_memorph_hook(&root, event)
        })
        .collect();
    let feature_enabled = fs::read_to_string(&config_path)
        .ok()
        .map(|contents| {
            crate::hooks::config_formats::toml_hooks::features_bool_enabled(&contents, "hooks")
        })
        .unwrap_or(false);
    let installed_version =
        crate::hooks::health::summarize_versions(EVENTS.iter().filter_map(|event| {
            crate::hooks::config_formats::json_hooks::event_memorph_hook_version(&root, event.name)
        }));
    let current_version = Some(crate::hooks::shared::current_hook_managed_version().to_string());
    let stale = missing.is_empty()
        && installed_version.as_deref()
            != Some(crate::hooks::shared::current_hook_managed_version());

    let health = if missing.is_empty() && feature_enabled && !stale {
        HookHealthStatus::InstalledOk
    } else if missing.len() == EVENTS.len() {
        HookHealthStatus::NotInstalled
    } else if stale {
        HookHealthStatus::InstalledStaleBinary
    } else {
        HookHealthStatus::Repairable
    };

    let message = match (missing.is_empty(), feature_enabled, stale) {
        (true, true, false) => Some("Codex memorph hooks are installed and enabled.".to_string()),
        (true, _, true) => Some(format!(
            "Codex memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        )),
        (false, true, _) => Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        )),
        (true, false, _) => Some(format!(
            "Codex hook entries exist, but hooks = true is missing in {}.",
            config_path.display()
        )),
        (false, false, _) => Some(format!(
            "Missing memorph hook events: {}; hooks = true is missing in {}.",
            missing.join(", "),
            config_path.display()
        )),
    };

    Ok(HookInstallStatus {
        provider: "codex".to_string(),
        status: health,
        config_path: config_path_text,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at("codex"),
    })
}

fn install() -> Result<HookOperationReport> {
    let path = hooks_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Codex hook directory: {}",
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
            "{} --managed-version {} --provider codex --event {}{}",
            command_base,
            crate::hooks::shared::current_hook_managed_version(),
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        entries.push(json!({
            "hooks": [{
                "type": "command",
                "command": command,
                "timeout": event.timeout
            }]
        }));
    }

    let hooks_changed = root != original;
    crate::hooks::config_formats::json_hooks::write_json_object(&path, &root)?;
    let flag_changed =
        crate::hooks::config_formats::toml_hooks::enable_bool_feature(&config_path(), "hooks")?;
    let status = status()?;
    Ok(HookOperationReport {
        provider: "codex".to_string(),
        operation: "install".to_string(),
        changed: hooks_changed || flag_changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Codex hook entries installed and hooks feature enabled.".to_string()),
        status,
    })
}

fn uninstall() -> Result<HookOperationReport> {
    let path = hooks_path();
    if !path.exists() {
        let status = status()?;
        return Ok(HookOperationReport {
            provider: "codex".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Codex hooks.json file does not exist.".to_string()),
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
        provider: "codex".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Codex memorph hook entries removed.".to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::test_support::TestHookHomeGuard;
    use serde_json::json;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = CODEX_HOOK.descriptor().expect("codex descriptor");
        assert_eq!(descriptor.provider(), CODEX_HOOK.provider_id());
    }
    #[test]
    fn codex_install_uninstall_preserves_foreign_hooks_and_config_toml() {
        let _home = TestHookHomeGuard::new();
        std::fs::create_dir_all(home()).unwrap();
        crate::storage::atomic_write::write_string_atomic(
            &hooks_path(),
            &serde_json::to_string_pretty(&json!({
                "hooks": {
                    "PreToolUse": [{"hooks": [{"type": "command", "command": "echo keep", "timeout": 1}]}],
                    "CustomEvent": [{"command": "echo custom"}]
                },
                "metadata": {"owner": "user"}
            }))
            .unwrap(),
        )
        .unwrap();
        crate::storage::atomic_write::write_string_atomic(
            &config_path(),
            "model = \"gpt\"\n\n[features]\nexperimental = true\nhooks = false\n\n[workspace]\ntrust = true\n",
        )
        .unwrap();

        CODEX_HOOK.install().unwrap();
        let installed_hooks =
            crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&hooks_path())
                .unwrap();
        assert_eq!(installed_hooks["metadata"]["owner"].as_str(), Some("user"));
        assert_eq!(
            installed_hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert!(
            crate::hooks::config_formats::json_hooks::event_has_memorph_hook(
                &installed_hooks,
                "PreToolUse"
            )
        );
        let installed_config = std::fs::read_to_string(config_path()).unwrap();
        assert!(installed_config.contains("model = \"gpt\""));
        assert!(installed_config.contains("experimental = true"));
        assert!(installed_config.contains("hooks = true"));
        assert!(installed_config.contains("[workspace]"));

        CODEX_HOOK.uninstall().unwrap();
        let removed_hooks =
            crate::hooks::config_formats::json_hooks::read_json_object_or_empty(&hooks_path())
                .unwrap();
        assert_eq!(
            removed_hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"].as_str(),
            Some("echo keep")
        );
        assert_eq!(
            removed_hooks["hooks"]["CustomEvent"][0]["command"].as_str(),
            Some("echo custom")
        );
        assert!(
            !crate::hooks::config_formats::json_hooks::event_has_memorph_hook(
                &removed_hooks,
                "PreToolUse"
            )
        );
        let removed_config = std::fs::read_to_string(config_path()).unwrap();
        assert!(removed_config.contains("hooks = true"));
        assert!(removed_config.contains("[workspace]"));
    }
}
