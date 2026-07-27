use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::hooks::contract::ProviderHook;
use crate::hooks::model::{HookHealthStatus, HookInstallStatus, HookOperationReport};
use crate::storage::atomic_write;

const CLINE_HOOK_MARKER: &str = "memorph-cline-hook";
const CLINE_HOOK_VERSION: &str = crate::hooks::shared::HOOK_MANAGED_VERSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClineHookEvent {
    name: &'static str,
    blocking: bool,
}

const EVENTS: &[ClineHookEvent] = &[
    event("UserPromptSubmit", false),
    event("PreToolUse", true),
    event("PostToolUse", false),
    event("TaskStart", false),
    event("TaskResume", false),
    event("TaskCancel", false),
    event("TaskComplete", false),
];

const fn event(name: &'static str, blocking: bool) -> ClineHookEvent {
    ClineHookEvent { name, blocking }
}

pub struct ClineHook;

pub static CLINE_HOOK: ClineHook = ClineHook;

impl ProviderHook for ClineHook {
    fn provider_id(&self) -> &'static str {
        "cline"
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

pub(crate) fn hooks_dir() -> PathBuf {
    crate::hooks::shared::hook_home_dir()
        .join("Documents")
        .join("Cline")
        .join("Rules")
        .join("Hooks")
}

fn hooks_dirs() -> Vec<PathBuf> {
    vec![hooks_dir()]
}

fn required_events() -> impl Iterator<Item = &'static str> {
    EVENTS.iter().map(|event| event.name)
}

fn status() -> Result<HookInstallStatus> {
    let dirs = hooks_dirs();
    let config_path = Some(
        dirs.iter()
            .map(|dir| dir.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    );
    if !dirs.iter().any(|dir| dir.exists()) {
        return Ok(HookInstallStatus {
            provider: "cline".to_string(),
            status: HookHealthStatus::NotInstalled,
            config_path,
            installed_version: None,
            current_version: Some(crate::hooks::shared::current_hook_managed_version().to_string()),
            message: Some("Cline hooks directory does not exist.".to_string()),
            last_event_at: crate::hooks::health::last_event_at("cline"),
        });
    }

    let missing: Vec<&str> = required_events()
        .filter(|event| !event_has_memorph_hook(event))
        .collect();
    let installed_version = crate::hooks::health::summarize_versions(
        required_events().filter_map(event_memorph_hook_version),
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
            "Cline memorph hooks are installed but stale: installed {}, current {}.",
            installed_version.as_deref().unwrap_or("unknown"),
            crate::hooks::shared::current_hook_managed_version()
        ))
    } else if missing.is_empty() {
        Some("Cline memorph hooks are installed.".to_string())
    } else {
        Some(format!(
            "Missing memorph hook files: {}",
            missing.join(", ")
        ))
    };

    Ok(HookInstallStatus {
        provider: "cline".to_string(),
        status: health,
        config_path,
        installed_version,
        current_version,
        message,
        last_event_at: crate::hooks::health::last_event_at("cline"),
    })
}

fn install() -> Result<HookOperationReport> {
    let command_base = crate::hooks::shared::bridge_command_base()?;
    let mut changed = false;
    let mut backup_path = None;
    for dir in hooks_dirs() {
        fs::create_dir_all(&dir).with_context(|| {
            format!("Failed to create Cline hooks directory: {}", dir.display())
        })?;
        for event in EVENTS {
            let path = dir.join(event.name);
            let original = fs::read_to_string(&path).ok();
            let preserved_path = preserved_hook_path(&path);
            if let Some(contents) = original.as_deref() {
                if !contents.contains(CLINE_HOOK_MARKER)
                    && !crate::hooks::shared::command_contains_memorph_hook(contents)
                    && !preserved_path.exists()
                {
                    if backup_path.is_none() && path.exists() {
                        backup_path = crate::hooks::shared::backup_if_exists(&path)?;
                    }
                    atomic_write::write_string_atomic(&preserved_path, contents)?;
                    make_executable(&preserved_path)?;
                }
            }
            let rendered = hook_script(
                &command_base,
                event,
                preserved_path.exists().then_some(preserved_path.as_path()),
            )?;
            if original.as_deref() != Some(rendered.as_str()) {
                if backup_path.is_none() && path.exists() {
                    backup_path = crate::hooks::shared::backup_if_exists(&path)?;
                }
                atomic_write::write_string_atomic(&path, &rendered)?;
                make_executable(&path)?;
                changed = true;
            } else {
                make_executable(&path)?;
            }
        }
    }

    let status = status()?;
    Ok(HookOperationReport {
        provider: "cline".to_string(),
        operation: "install".to_string(),
        changed,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Cline file-based hook entries installed.".to_string()),
        status,
    })
}

fn uninstall() -> Result<HookOperationReport> {
    let dirs = hooks_dirs();
    if !dirs.iter().any(|dir| dir.exists()) {
        let status = status()?;
        return Ok(HookOperationReport {
            provider: "cline".to_string(),
            operation: "uninstall".to_string(),
            changed: false,
            status,
            backup_path: None,
            message: Some("Cline hooks directory does not exist.".to_string()),
        });
    }

    let mut changed = false;
    let mut backup_path = None;
    for dir in dirs {
        for event in EVENTS {
            let path = dir.join(event.name);
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            if !contents.contains(CLINE_HOOK_MARKER)
                || !crate::hooks::shared::command_contains_memorph_hook(&contents)
            {
                continue;
            }
            if backup_path.is_none() {
                backup_path = crate::hooks::shared::backup_if_exists(&path)?;
            }
            let preserved_path = preserved_hook_path(&path);
            if preserved_path.exists() {
                fs::copy(&preserved_path, &path).with_context(|| {
                    format!(
                        "Failed to restore preserved Cline hook file: {}",
                        path.display()
                    )
                })?;
                make_executable(&path)?;
                fs::remove_file(&preserved_path).with_context(|| {
                    format!(
                        "Failed to remove preserved Cline hook file: {}",
                        preserved_path.display()
                    )
                })?;
            } else {
                fs::remove_file(&path).with_context(|| {
                    format!("Failed to remove Cline hook file: {}", path.display())
                })?;
            }
            changed = true;
        }
    }

    let status = status()?;
    Ok(HookOperationReport {
        provider: "cline".to_string(),
        operation: "uninstall".to_string(),
        changed,
        status,
        backup_path: backup_path.map(|path| path.display().to_string()),
        message: Some("Cline memorph hook entries removed.".to_string()),
    })
}

fn event_has_memorph_hook(event: &str) -> bool {
    event_memorph_hook_version(event).is_some()
}

fn event_memorph_hook_version(event: &str) -> Option<Option<String>> {
    hooks_dirs().into_iter().find_map(|dir| {
        let path = dir.join(event);
        let contents = fs::read_to_string(path).ok()?;
        if !contents.contains(CLINE_HOOK_MARKER)
            || !crate::hooks::shared::command_contains_memorph_hook(&contents)
        {
            return None;
        }
        Some(command_managed_version(&contents))
    })
}

pub(crate) fn preserved_hook_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("hook");
    path.with_file_name(format!("{file_name}.memorph-original"))
}

fn hook_script(
    command_base: &str,
    event: &ClineHookEvent,
    preserved_hook_path: Option<&Path>,
) -> Result<String> {
    let command = format!(
        "{} --managed-version {} --provider cline --event {}{}",
        command_base,
        crate::hooks::shared::current_hook_managed_version(),
        event.name,
        if event.blocking { " --blocking" } else { "" }
    );
    let preserved_hook = preserved_hook_path
        .map(|path| shell_quote(&path.to_string_lossy()))
        .unwrap_or_default();
    Ok(format!(
        "#!/bin/bash\n# {marker}\n# version: {version}\nINPUT=$(cat)\nMEMORPH_OUTPUT=$(printf '%s' \"$INPUT\" | {command} 2>/dev/null)\nORIGINAL_HOOK={preserved_hook}\nORIGINAL_OUTPUT=\"\"\nif [ -n \"$ORIGINAL_HOOK\" ] && [ -x \"$ORIGINAL_HOOK\" ]; then\n  ORIGINAL_OUTPUT=$(printf '%s' \"$INPUT\" | \"$ORIGINAL_HOOK\" 2>/dev/null)\nfi\nif printf '%s' \"$MEMORPH_OUTPUT\" | grep -q '\"cancel\"[[:space:]]*:[[:space:]]*true'; then\n  printf '%s' \"$MEMORPH_OUTPUT\"\nelif [ -n \"$ORIGINAL_OUTPUT\" ]; then\n  printf '%s' \"$ORIGINAL_OUTPUT\"\nelif [ -n \"$MEMORPH_OUTPUT\" ]; then\n  printf '%s' \"$MEMORPH_OUTPUT\"\nelse\n  printf '{{\"cancel\":false}}'\nfi\n",
        marker = CLINE_HOOK_MARKER,
        version = CLINE_HOOK_VERSION,
        command = command,
        preserved_hook = preserved_hook,
    ))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .with_context(|| format!("Failed to chmod Cline hook file: {}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn command_managed_version(command: &str) -> Option<String> {
    let mut parts = command.split_whitespace();
    while let Some(part) = parts.next() {
        if part == "--managed-version" {
            return parts.next().map(ToString::to_string);
        }
        if let Some(value) = part.strip_prefix("--managed-version=") {
            return Some(value.to_string());
        }
    }
    None
}

fn shell_quote(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("'{}'", value.replace('\'', "'\\''"))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::model::HookHealthStatus;
    use crate::hooks::test_support::TestHookHomeGuard;

    #[test]
    fn descriptor_matches_hook_registry() {
        let descriptor = CLINE_HOOK.descriptor().expect("cline descriptor");
        assert_eq!(descriptor.provider(), CLINE_HOOK.provider_id());
    }

    #[test]
    fn renders_hook_script_with_valid_fallback_response() {
        let script = hook_script(
            "memorph __hook-bridge",
            &ClineHookEvent {
                name: "PreToolUse",
                blocking: true,
            },
            None,
        )
        .unwrap();
        assert!(script.contains(CLINE_HOOK_MARKER));
        assert!(script.contains("--provider cline --event PreToolUse --blocking"));
        assert!(script.contains("printf '{\"cancel\":false}'"));
    }
    #[test]
    fn cline_install_preserves_and_restores_existing_user_hook_file() {
        let _home = TestHookHomeGuard::new();
        let dir = hooks_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let hook_path = dir.join("PreToolUse");
        let original = "#!/bin/bash\nprintf '{\"cancel\":true,\"reason\":\"user hook\"}'\n";
        crate::storage::atomic_write::write_string_atomic(&hook_path, original).unwrap();

        let installed = CLINE_HOOK.install().unwrap();
        assert_eq!(installed.status.status, HookHealthStatus::InstalledOk);
        let preserved_path = preserved_hook_path(&hook_path);
        assert_eq!(std::fs::read_to_string(&preserved_path).unwrap(), original);
        let installed_script = std::fs::read_to_string(&hook_path).unwrap();
        assert!(installed_script.contains("memorph-cline-hook"));
        assert!(installed_script.contains("ORIGINAL_HOOK="));
        assert!(installed_script.contains("PreToolUse.memorph-original"));

        let removed = CLINE_HOOK.uninstall().unwrap();
        assert_eq!(removed.status.status, HookHealthStatus::NotInstalled);
        assert_eq!(std::fs::read_to_string(&hook_path).unwrap(), original);
        assert!(!preserved_path.exists());
    }
}
