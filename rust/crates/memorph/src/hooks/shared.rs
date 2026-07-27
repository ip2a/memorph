//! Shared hook-management helpers that are not provider-specific.

use std::fs;
use std::path::{Path, PathBuf};

use std::sync::{OnceLock, RwLock};

use anyhow::{Context, Result};
use chrono::Utc;

pub const HOOK_COMMAND_MARKER: &str = "__hook-bridge";
pub const HOOK_MANAGED_VERSION: &str = "hook-v1";
const SETTINGS_BACKUP_SUFFIX: &str = "memorph-hook-backup";

static TEST_HOME_DIR: OnceLock<RwLock<Option<PathBuf>>> = OnceLock::new();

pub fn current_hook_managed_version() -> &'static str {
    HOOK_MANAGED_VERSION
}

pub fn hook_home_dir() -> PathBuf {
        if let Some(path) = TEST_HOME_DIR
        .get_or_init(|| RwLock::new(None))
        .read()
        .unwrap()
        .clone()
    {
        return path;
    }

    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn set_test_home_dir(path: Option<PathBuf>) {
    *TEST_HOME_DIR
        .get_or_init(|| RwLock::new(None))
        .write()
        .unwrap() = path;
}

pub fn bridge_command_base() -> Result<String> {
    let exe = std::env::current_exe().context("Failed to resolve current memorph executable")?;
    let exe = shell_quote(&exe.to_string_lossy());
    Ok(format!("{exe} {HOOK_COMMAND_MARKER}"))
}

pub fn command_contains_memorph_hook(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("memorph") && lower.contains(HOOK_COMMAND_MARKER)
}

pub fn backup_if_exists(path: &Path) -> Result<Option<PathBuf>> {
    if !path.exists() {
        return Ok(None);
    }
    let backup = path.with_extension(format!(
        "json.{SETTINGS_BACKUP_SUFFIX}.{}",
        Utc::now().format("%Y%m%d%H%M%S")
    ));
    fs::copy(path, &backup)
        .with_context(|| format!("Failed to write hook config backup: {}", backup.display()))?;
    Ok(Some(backup))
}

fn shell_quote(value: &str) -> String {
    if value.chars().any(char::is_whitespace) {
        format!("'{}'", value.replace('\'', "'\\''"))
    } else {
        value.to_string()
    }
}
