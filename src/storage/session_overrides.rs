use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::atomic_write;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionOverrides {
    #[serde(default = "current_version")]
    pub version: u32,
    #[serde(default)]
    pub sessions: BTreeMap<String, BTreeMap<String, SessionOverride>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SessionOverride {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    pub updated_at: i64,
}

fn current_version() -> u32 {
    1
}

pub fn overrides_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Unable to locate user home directory")?;
    Ok(home.join(".memorph").join("session_overrides.json"))
}

pub fn load_overrides() -> Result<SessionOverrides> {
    load_overrides_from_path(&overrides_path()?)
}

pub fn save_overrides(overrides: &SessionOverrides) -> Result<()> {
    save_overrides_to_path(&overrides_path()?, overrides)
}

pub fn set_display_title(provider_id: &str, session_id: &str, title: &str) -> Result<()> {
    let mut overrides = load_overrides()?;
    set_display_title_in_overrides(&mut overrides, provider_id, session_id, title);
    save_overrides(&overrides)
}

pub fn remove_session(provider_id: &str, session_id: &str) -> Result<()> {
    let mut overrides = load_overrides()?;
    remove_session_in_overrides(&mut overrides, provider_id, session_id);
    save_overrides(&overrides)
}

pub fn get_display_title<'a>(
    overrides: &'a SessionOverrides,
    provider_id: &str,
    session_id: &str,
) -> Option<&'a str> {
    overrides
        .sessions
        .get(provider_id)
        .and_then(|sessions| sessions.get(session_id))
        .and_then(|entry| entry.display_title.as_deref())
        .filter(|title| !title.trim().is_empty())
}

pub fn set_display_title_in_overrides(
    overrides: &mut SessionOverrides,
    provider_id: &str,
    session_id: &str,
    title: &str,
) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }

    overrides.version = current_version();
    overrides
        .sessions
        .entry(provider_id.to_string())
        .or_default()
        .insert(
            session_id.to_string(),
            SessionOverride {
                display_title: Some(title.to_string()),
                updated_at: chrono::Utc::now().timestamp_millis(),
            },
        );
}

pub fn remove_session_in_overrides(
    overrides: &mut SessionOverrides,
    provider_id: &str,
    session_id: &str,
) {
    let Some(sessions) = overrides.sessions.get_mut(provider_id) else {
        return;
    };
    sessions.remove(session_id);
    if sessions.is_empty() {
        overrides.sessions.remove(provider_id);
    }
}

fn load_overrides_from_path(path: &Path) -> Result<SessionOverrides> {
    if !path.exists() {
        return Ok(SessionOverrides {
            version: current_version(),
            sessions: BTreeMap::new(),
        });
    }

    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read session overrides: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse session overrides: {}", path.display()))
}

fn save_overrides_to_path(path: &Path, overrides: &SessionOverrides) -> Result<()> {
    let dir = path
        .parent()
        .with_context(|| format!("Session overrides path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create session overrides directory: {}", dir.display()))?;
    let raw = serde_json::to_string_pretty(overrides)?;
    atomic_write::write_string_atomic(path, &raw)
        .with_context(|| format!("Failed to write session overrides: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_display_title_by_provider_and_session() {
        let mut overrides = SessionOverrides::default();
        set_display_title_in_overrides(&mut overrides, "codex", "abc", "Renamed");

        assert_eq!(
            get_display_title(&overrides, "codex", "abc"),
            Some("Renamed")
        );
        assert_eq!(get_display_title(&overrides, "claude", "abc"), None);
    }

    #[test]
    fn removes_empty_provider_bucket_after_session_cleanup() {
        let mut overrides = SessionOverrides::default();
        set_display_title_in_overrides(&mut overrides, "codex", "abc", "Renamed");

        remove_session_in_overrides(&mut overrides, "codex", "abc");

        assert_eq!(get_display_title(&overrides, "codex", "abc"), None);
        assert!(!overrides.sessions.contains_key("codex"));
    }
}
