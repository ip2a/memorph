use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl Default for SessionOverrides {
    fn default() -> Self {
        Self {
            version: current_version(),
            sessions: BTreeMap::new(),
        }
    }
}

pub fn overrides_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Unable to locate user home directory")?;
    Ok(home.join(".memorph").join("session_overrides.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_display_title<'a>(
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

    fn set_display_title_in_overrides(
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

    fn remove_session_in_overrides(
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
