use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};
use crate::storage::atomic_write;

pub struct KimiHook;

pub static KIMI_HOOK: KimiHook = KimiHook;

#[derive(Debug, Clone, Copy)]
struct KimiHookEvent {
    name: &'static str,
    timeout: u64,
    matcher: Option<&'static str>,
    blocking: bool,
}

const EVENTS: &[KimiHookEvent] = &[
    event("UserPromptSubmit", 5, None, true),
    event("PreToolUse", 5, Some(".*"), false),
    event("PostToolUse", 5, Some(".*"), true),
    event("PostToolUseFailure", 5, Some(".*"), true),
    event("Stop", 5, None, true),
    event("SubagentStart", 5, None, true),
    event("SubagentStop", 5, None, true),
    event("SessionStart", 5, None, false),
    event("SessionEnd", 5, None, true),
    event("Notification", 600, None, false),
    event("PreCompact", 5, None, true),
];

const fn event(
    name: &'static str,
    timeout: u64,
    matcher: Option<&'static str>,
    blocking: bool,
) -> KimiHookEvent {
    KimiHookEvent {
        name,
        timeout,
        matcher,
        blocking,
    }
}

impl ProviderHook for KimiHook {
    fn provider_id(&self) -> &'static str {
        "kimi"
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

pub(crate) fn config_path() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join(".kimi")
        .join("config.toml")
}

fn status() -> Result<HookInstallStatus> {
    let path = config_path();
    let config_path = Some(path.display().to_string());
    if !path.exists() {
        return Ok(HookInstallStatus {
            provider: "kimi".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some("Kimi config.toml does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("kimi"),
        });
    }

    let contents = fs::read_to_string(&path)?;
    let missing: Vec<&str> = EVENTS
        .iter()
        .map(|event| event.name)
        .filter(|event| !contents_contains_memorph_hook(&contents, event))
        .collect();
    let installed_version = crate::hooks::health::summarize_versions(
        EVENTS
            .iter()
            .filter_map(|event| event_memorph_hook_version(&contents, event.name)),
    );
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
            "Kimi memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Kimi memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook events: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "kimi".to_string(),
        status: health,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at("kimi"),
    })
}

fn install() -> Result<HookOperationReport> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create Kimi config directory: {}",
                parent.display()
            )
        })?;
    }

    let original = fs::read_to_string(&path).unwrap_or_default();
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let mut updated = remove_hooks(&original);
    let command_base = crate::hooks::shared::bridge_command_base()?;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    if !updated.trim().is_empty() {
        updated.push('\n');
    }
    updated.push_str(&hook_blocks(&command_base)?);

    let changed = updated != original;
    atomic_write::write_string_atomic(&path, &updated)?;
    let status = status()?;
    Ok(HookOperationReport {
        provider: "kimi".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Kimi hook entries installed.".to_string()),
        status,
    })
}

fn uninstall() -> Result<HookOperationReport> {
    let path = config_path();
    if !path.exists() {
        let status = status()?;
        return Ok(HookOperationReport {
            provider: "kimi".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Kimi config.toml file does not exist.".to_string()),
        });
    }

    let original = fs::read_to_string(&path).unwrap_or_default();
    let backup_path = crate::hooks::shared::backup_if_exists(&path)?;
    let updated = remove_hooks(&original);
    let changed = updated != original;
    if changed {
        atomic_write::write_string_atomic(&path, &updated)?;
    }
    let status = status()?;
    Ok(HookOperationReport {
        provider: "kimi".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Kimi memorph hook entries removed.".to_string()),
    })
}

fn hook_blocks(command_base: &str) -> Result<String> {
    let mut blocks = Vec::new();
    for event in EVENTS {
        let command = format!(
            "{} --managed-version {} --provider kimi --event {}{}",
            command_base,
            crate::hooks::shared::current_hook_managed_version(),
            event.name,
            if event.blocking { " --blocking" } else { "" }
        );
        let mut block = format!(
            "[[hooks]]\nevent = {}\ncommand = {}\ntimeout = {}",
            serde_json::to_string(event.name)?,
            serde_json::to_string(&command)?,
            event.timeout
        );
        if let Some(matcher) = event.matcher {
            block.push_str(&format!("\nmatcher = {}", serde_json::to_string(matcher)?));
        }
        blocks.push(block);
    }
    Ok(blocks.join("\n\n") + "\n")
}

pub(crate) fn remove_hooks(contents: &str) -> String {
    crate::hooks::config_formats::toml_blocks::remove_blocks_matching(
        contents,
        "[[hooks]]",
        crate::hooks::config_formats::toml_blocks::block_contains_memorph_command,
    )
}

pub(crate) fn contents_contains_memorph_hook(contents: &str, event: &str) -> bool {
    crate::hooks::config_formats::toml_blocks::blocks_from_contents(contents, "[[hooks]]")
        .into_iter()
        .any(|block| {
            crate::hooks::config_formats::toml_blocks::block_string_assignment(&block, "event")
                .as_deref()
                == Some(event)
                && crate::hooks::config_formats::toml_blocks::block_contains_memorph_command(&block)
        })
}

pub(crate) fn event_memorph_hook_version(contents: &str, event: &str) -> Option<Option<String>> {
    crate::hooks::config_formats::toml_blocks::blocks_from_contents(contents, "[[hooks]]")
        .into_iter()
        .find(|block| {
            crate::hooks::config_formats::toml_blocks::block_string_assignment(block, "event")
                .as_deref()
                == Some(event)
                && crate::hooks::config_formats::toml_blocks::block_contains_memorph_command(block)
        })
        .map(|block| {
            crate::hooks::config_formats::toml_blocks::block_memorph_command_version(&block)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::test_support::TestHookHomeGuard;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = KIMI_HOOK.descriptor().expect("kimi descriptor");
        assert_eq!(descriptor.provider(), KIMI_HOOK.provider_id());
    }
    #[test]
    fn detects_and_removes_kimi_toml_hook_blocks() {
        let contents = r#"
model = "kimi"

[[hooks]]
event = "PreToolUse"
command = "memorph __hook-bridge --managed-version hook-v1 --provider kimi --event PreToolUse"
timeout = 5
matcher = ".*"

[[hooks]]
event = "UserPromptSubmit"
command = "echo keep"
timeout = 5
"#;
        assert!(contents_contains_memorph_hook(contents, "PreToolUse"));
        assert_eq!(
            event_memorph_hook_version(contents, "PreToolUse")
                .flatten()
                .as_deref(),
            Some(crate::hooks::shared::HOOK_MANAGED_VERSION)
        );
        let cleaned = remove_hooks(contents);
        assert!(!cleaned.contains("__hook-bridge"));
        assert!(cleaned.contains("echo keep"));
    }
    #[test]
    fn kimi_install_uninstall_preserves_foreign_toml_hook_blocks() {
        let _home = TestHookHomeGuard::new();
        let path = config_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = r#"model = "kimi"

[[hooks]]
event = "PreToolUse"
command = "echo keep"
timeout = 5

[ui]
theme = "dark"
"#;
        crate::storage::atomic_write::write_string_atomic(&path, original).unwrap();

        KIMI_HOOK.install().unwrap();
        let installed = std::fs::read_to_string(&path).unwrap();
        assert!(installed.contains("command = \"echo keep\""));
        assert!(installed.contains("theme = \"dark\""));
        assert!(contents_contains_memorph_hook(&installed, "PreToolUse"));

        KIMI_HOOK.uninstall().unwrap();
        let removed = std::fs::read_to_string(&path).unwrap();
        assert!(removed.contains("command = \"echo keep\""));
        assert!(removed.contains("theme = \"dark\""));
        assert!(!contents_contains_memorph_hook(&removed, "PreToolUse"));
    }
}
