use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::canonical::{LocalSessionState, SessionLocator, WorkspaceSessionState};

use super::{atomic_write, session_overrides};

static STATE_CACHE: RwLock<Option<SessionStateStore>> = RwLock::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStateStore {
    #[serde(default = "current_version")]
    pub version: u32,
    #[serde(default)]
    pub sessions: BTreeMap<String, BTreeMap<String, LocalSessionState>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ResolvedLocalSessionState {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_title: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preferred_targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct SessionLocalStateUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_title: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_targets: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_override: Option<WorkspaceLocalStateUpdate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WorkspaceLocalStateUpdate {
    pub workspace_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hidden: Option<Option<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<Option<bool>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_targets: Option<Option<Vec<String>>>,
}

fn current_version() -> u32 {
    2
}

impl Default for SessionStateStore {
    fn default() -> Self {
        Self {
            version: current_version(),
            sessions: BTreeMap::new(),
        }
    }
}

pub fn session_state_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Unable to locate user home directory")?;
    Ok(home.join(".memorph").join("session_state.json"))
}

pub fn load_state_store() -> Result<SessionStateStore> {
    {
        let cache = STATE_CACHE.read().unwrap();
        if let Some(store) = cache.as_ref() {
            return Ok(store.clone());
        }
    }

    let state_path = session_state_path()?;
    let legacy_path = session_overrides::overrides_path()?;
    let (store, migrated) = load_state_store_from_paths(&state_path, Some(&legacy_path))?;
    if migrated {
        save_state_store_to_path(&state_path, &store)?;
    }

    let mut cache = STATE_CACHE.write().unwrap();
    *cache = Some(store.clone());

    Ok(store)
}

pub fn save_state_store(store: &SessionStateStore) -> Result<()> {
    save_state_store_to_path(&session_state_path()?, store)?;
    let mut cache = STATE_CACHE.write().unwrap();
    *cache = Some(store.clone());
    Ok(())
}

/// Force-clear the in-process session-state cache.
pub fn clear_state_cache() {
    let mut cache = STATE_CACHE.write().unwrap();
    *cache = None;
}

pub fn get_session_state<'a>(
    store: &'a SessionStateStore,
    provider_id: &str,
    session_id: &str,
) -> Option<&'a LocalSessionState> {
    store
        .sessions
        .get(provider_id)
        .and_then(|sessions| sessions.get(session_id))
}

pub fn resolve_session_state(
    store: &SessionStateStore,
    provider_id: &str,
    session_id: &str,
    workspace_dir: Option<&str>,
) -> ResolvedLocalSessionState {
    let Some(state) = get_session_state(store, provider_id, session_id) else {
        return ResolvedLocalSessionState::default();
    };

    let mut resolved = ResolvedLocalSessionState {
        display_title: state
            .display_title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(str::to_string),
        archived: state.archived,
        hidden: state.hidden,
        pinned: state.pinned,
        notes: state.notes.clone(),
        tags: state.tags.clone(),
        preferred_targets: state.preferred_targets.clone(),
    };

    if let Some(workspace_dir) = workspace_dir {
        if let Some(workspace_state) = state
            .workspace_overrides
            .iter()
            .find(|entry| workspace_override_matches(&entry.workspace_dir, workspace_dir))
        {
            if let Some(hidden) = workspace_state.hidden {
                resolved.hidden = hidden;
            }
            if let Some(pinned) = workspace_state.pinned {
                resolved.pinned = pinned;
            }
            if let Some(preferred_targets) = workspace_state.preferred_targets.as_ref() {
                resolved.preferred_targets = preferred_targets.clone();
            }
        }
    }

    resolved
}

pub fn set_session_state(state: LocalSessionState) -> Result<()> {
    let mut store = load_state_store()?;
    set_session_state_in_store(&mut store, state);
    save_state_store(&store)
}

pub fn set_display_title(provider_id: &str, session_id: &str, title: &str) -> Result<()> {
    let mut store = load_state_store()?;
    set_display_title_in_store(&mut store, provider_id, session_id, title);
    save_state_store(&store)
}

pub fn update_session_state(
    provider_id: &str,
    session_id: &str,
    update: &SessionLocalStateUpdate,
) -> Result<ResolvedLocalSessionState> {
    let mut store = load_state_store()?;
    update_session_state_in_store(&mut store, provider_id, session_id, update);
    let resolved = resolve_session_state(
        &store,
        provider_id,
        session_id,
        update
            .workspace_override
            .as_ref()
            .map(|workspace| workspace.workspace_dir.as_str()),
    );
    save_state_store(&store)?;
    Ok(resolved)
}

pub fn remove_session(provider_id: &str, session_id: &str) -> Result<()> {
    let mut store = load_state_store()?;
    remove_session_in_store(&mut store, provider_id, session_id);
    save_state_store(&store)
}

pub fn set_session_state_in_store(store: &mut SessionStateStore, mut state: LocalSessionState) {
    state.updated_at = Utc::now();
    store.version = current_version();
    store
        .sessions
        .entry(state.locator.provider_id.clone())
        .or_default()
        .insert(state.locator.session_id.clone(), state);
}

pub fn set_display_title_in_store(
    store: &mut SessionStateStore,
    provider_id: &str,
    session_id: &str,
    title: &str,
) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }

    let mut state = get_session_state(store, provider_id, session_id)
        .cloned()
        .unwrap_or_else(|| LocalSessionState {
            locator: SessionLocator {
                provider_id: provider_id.to_string(),
                session_id: session_id.to_string(),
            },
            display_title: None,
            archived: false,
            hidden: false,
            pinned: false,
            notes: None,
            tags: Vec::new(),
            preferred_targets: Vec::new(),
            workspace_overrides: Vec::new(),
            updated_at: Utc::now(),
        });
    state.display_title = Some(title.to_string());
    set_session_state_in_store(store, state);
}

pub fn update_session_state_in_store(
    store: &mut SessionStateStore,
    provider_id: &str,
    session_id: &str,
    update: &SessionLocalStateUpdate,
) {
    let mut state = get_session_state(store, provider_id, session_id)
        .cloned()
        .unwrap_or_else(|| LocalSessionState {
            locator: SessionLocator {
                provider_id: provider_id.to_string(),
                session_id: session_id.to_string(),
            },
            display_title: None,
            archived: false,
            hidden: false,
            pinned: false,
            notes: None,
            tags: Vec::new(),
            preferred_targets: Vec::new(),
            workspace_overrides: Vec::new(),
            updated_at: Utc::now(),
        });

    if let Some(display_title) = &update.display_title {
        state.display_title = display_title
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    if let Some(hidden) = update.hidden {
        state.hidden = hidden;
    }
    if let Some(pinned) = update.pinned {
        state.pinned = pinned;
    }
    if let Some(notes) = &update.notes {
        state.notes = notes
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
    }
    if let Some(tags) = &update.tags {
        state.tags = normalize_tags(tags);
    }
    if let Some(preferred_targets) = &update.preferred_targets {
        state.preferred_targets = crate::config::normalize_provider_ids(preferred_targets.clone());
    }
    if let Some(workspace_override) = &update.workspace_override {
        apply_workspace_override_update(&mut state, workspace_override);
    }

    set_session_state_in_store(store, state);
}

pub fn remove_session_in_store(store: &mut SessionStateStore, provider_id: &str, session_id: &str) {
    let Some(sessions) = store.sessions.get_mut(provider_id) else {
        return;
    };
    sessions.remove(session_id);
    if sessions.is_empty() {
        store.sessions.remove(provider_id);
    }
}

fn load_state_store_from_paths(
    state_path: &Path,
    legacy_overrides_path: Option<&Path>,
) -> Result<(SessionStateStore, bool)> {
    if state_path.exists() {
        let raw = std::fs::read_to_string(state_path)
            .with_context(|| format!("Failed to read session state: {}", state_path.display()))?;
        let store = serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse session state: {}", state_path.display()))?;
        return Ok((store, false));
    }

    if let Some(legacy_overrides_path) = legacy_overrides_path {
        if legacy_overrides_path.exists() {
            return migrate_legacy_overrides(legacy_overrides_path).map(|store| (store, true));
        }
    }

    Ok((SessionStateStore::default(), false))
}

fn save_state_store_to_path(path: &Path, store: &SessionStateStore) -> Result<()> {
    let dir = path
        .parent()
        .with_context(|| format!("Session state path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(dir).with_context(|| {
        format!(
            "Failed to create session state directory: {}",
            dir.display()
        )
    })?;
    let raw = serde_json::to_string_pretty(store)?;
    atomic_write::write_string_atomic(path, &raw)
        .with_context(|| format!("Failed to write session state: {}", path.display()))
}

fn migrate_legacy_overrides(path: &Path) -> Result<SessionStateStore> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read session overrides: {}", path.display()))?;
    let legacy: session_overrides::SessionOverrides = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse session overrides: {}", path.display()))?;

    let mut store = SessionStateStore {
        version: current_version(),
        sessions: BTreeMap::new(),
    };
    for (provider_id, sessions) in legacy.sessions {
        let provider_bucket = store.sessions.entry(provider_id.clone()).or_default();
        for (session_id, session) in sessions {
            provider_bucket.insert(
                session_id.clone(),
                LocalSessionState {
                    locator: SessionLocator {
                        provider_id: provider_id.clone(),
                        session_id,
                    },
                    display_title: session.display_title,
                    archived: false,
                    hidden: false,
                    pinned: false,
                    notes: None,
                    tags: Vec::new(),
                    preferred_targets: Vec::new(),
                    workspace_overrides: Vec::<WorkspaceSessionState>::new(),
                    updated_at: chrono::DateTime::from_timestamp_millis(session.updated_at)
                        .unwrap_or_else(Utc::now),
                },
            );
        }
    }

    Ok(store)
}

fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() || normalized.iter().any(|existing| existing == tag) {
            continue;
        }
        normalized.push(tag.to_string());
    }
    normalized
}

fn workspace_override_matches(stored_workspace: &str, requested_workspace: &str) -> bool {
    stored_workspace == requested_workspace
        || crate::provider::default_workspace_matches(
            Some(stored_workspace),
            Some(requested_workspace),
        )
}

fn apply_workspace_override_update(
    state: &mut LocalSessionState,
    update: &WorkspaceLocalStateUpdate,
) {
    let workspace_dir = update.workspace_dir.trim();
    if workspace_dir.is_empty() {
        return;
    }

    let existing_idx = state
        .workspace_overrides
        .iter()
        .position(|entry| workspace_override_matches(&entry.workspace_dir, workspace_dir));
    let workspace_idx = match existing_idx {
        Some(idx) => idx,
        None => {
            state.workspace_overrides.push(WorkspaceSessionState {
                workspace_dir: workspace_dir.to_string(),
                hidden: None,
                pinned: None,
                preferred_targets: None,
            });
            state.workspace_overrides.len() - 1
        }
    };

    {
        let workspace_state = &mut state.workspace_overrides[workspace_idx];
        workspace_state.workspace_dir = workspace_dir.to_string();
        if let Some(hidden) = update.hidden {
            workspace_state.hidden = hidden;
        }
        if let Some(pinned) = update.pinned {
            workspace_state.pinned = pinned;
        }
        if let Some(preferred_targets) = &update.preferred_targets {
            workspace_state.preferred_targets = preferred_targets
                .as_ref()
                .map(|providers| crate::config::normalize_provider_ids(providers.clone()))
                .filter(|providers| !providers.is_empty());
        }
    }

    if workspace_override_is_empty(&state.workspace_overrides[workspace_idx]) {
        state.workspace_overrides.remove(workspace_idx);
    }
}

fn workspace_override_is_empty(workspace_state: &WorkspaceSessionState) -> bool {
    workspace_state.hidden.is_none()
        && workspace_state.pinned.is_none()
        && workspace_state.preferred_targets.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_workspace_specific_overrides() {
        let mut store = SessionStateStore::default();
        set_session_state_in_store(
            &mut store,
            LocalSessionState {
                locator: SessionLocator {
                    provider_id: "codex".to_string(),
                    session_id: "abc".to_string(),
                },
                display_title: Some("Renamed".to_string()),
                archived: false,
                hidden: false,
                pinned: true,
                notes: Some("note".to_string()),
                tags: vec!["tag".to_string()],
                preferred_targets: vec!["claude".to_string()],
                workspace_overrides: vec![WorkspaceSessionState {
                    workspace_dir: "/tmp/project".to_string(),
                    hidden: Some(true),
                    pinned: Some(false),
                    preferred_targets: Some(vec!["codex".to_string(), "kiro".to_string()]),
                }],
                updated_at: Utc::now(),
            },
        );

        let resolved = resolve_session_state(&store, "codex", "abc", Some("/tmp/project"));

        assert_eq!(resolved.display_title.as_deref(), Some("Renamed"));
        assert!(resolved.hidden);
        assert!(!resolved.pinned);
        assert_eq!(resolved.preferred_targets, vec!["codex", "kiro"]);
        assert_eq!(resolved.notes.as_deref(), Some("note"));
        assert_eq!(resolved.tags, vec!["tag"]);
    }

    #[test]
    fn migrates_legacy_display_titles() {
        let dir = tempdir().unwrap();
        let state_path = dir.path().join("session_state.json");
        let legacy_path = dir.path().join("session_overrides.json");
        std::fs::write(
            &legacy_path,
            serde_json::json!({
                "version": 1,
                "sessions": {
                    "codex": {
                        "abc": {
                            "display_title": "Renamed",
                            "updated_at": 1_700_000_000_000_i64
                        }
                    }
                }
            })
            .to_string(),
        )
        .unwrap();

        let (store, migrated) =
            load_state_store_from_paths(&state_path, Some(&legacy_path)).unwrap();
        let resolved = resolve_session_state(&store, "codex", "abc", None);

        assert!(migrated);
        assert_eq!(resolved.display_title.as_deref(), Some("Renamed"));
    }

    #[test]
    fn removes_empty_provider_bucket_after_session_cleanup() {
        let mut store = SessionStateStore::default();
        set_display_title_in_store(&mut store, "codex", "abc", "Renamed");

        remove_session_in_store(&mut store, "codex", "abc");

        assert!(resolve_session_state(&store, "codex", "abc", None)
            .display_title
            .is_none());
        assert!(!store.sessions.contains_key("codex"));
    }

    #[test]
    fn applies_local_state_update_fields() {
        let mut store = SessionStateStore::default();
        update_session_state_in_store(
            &mut store,
            "codex",
            "abc",
            &SessionLocalStateUpdate {
                display_title: Some(Some(" Renamed ".to_string())),
                hidden: Some(true),
                pinned: Some(true),
                notes: Some(Some(" note ".to_string())),
                tags: Some(vec!["a".to_string(), "a".to_string(), " ".to_string()]),
                preferred_targets: Some(vec![
                    "claude".to_string(),
                    "claude".to_string(),
                    "missing".to_string(),
                ]),
                workspace_override: None,
            },
        );

        let resolved = resolve_session_state(&store, "codex", "abc", None);
        assert_eq!(resolved.display_title.as_deref(), Some("Renamed"));
        assert!(resolved.hidden);
        assert!(resolved.pinned);
        assert_eq!(resolved.notes.as_deref(), Some("note"));
        assert_eq!(resolved.tags, vec!["a"]);
        assert_eq!(resolved.preferred_targets, vec!["claude"]);
    }

    #[test]
    fn applies_and_clears_workspace_override_updates() {
        let mut store = SessionStateStore::default();
        let dir = tempdir().unwrap();
        let workspace = dir.path().join(".");
        let workspace_key = dir.path().to_string_lossy().to_string();
        update_session_state_in_store(
            &mut store,
            "codex",
            "abc",
            &SessionLocalStateUpdate {
                display_title: None,
                hidden: None,
                pinned: Some(true),
                notes: None,
                tags: None,
                preferred_targets: Some(vec!["claude".to_string()]),
                workspace_override: Some(WorkspaceLocalStateUpdate {
                    workspace_dir: workspace.to_string_lossy().to_string(),
                    hidden: Some(Some(true)),
                    pinned: Some(Some(false)),
                    preferred_targets: Some(Some(vec!["codex".to_string(), "missing".to_string()])),
                }),
            },
        );

        let resolved = resolve_session_state(&store, "codex", "abc", Some(&workspace_key));
        assert!(resolved.hidden);
        assert!(!resolved.pinned);
        assert_eq!(resolved.preferred_targets, vec!["codex"]);

        update_session_state_in_store(
            &mut store,
            "codex",
            "abc",
            &SessionLocalStateUpdate {
                display_title: None,
                hidden: None,
                pinned: None,
                notes: None,
                tags: None,
                preferred_targets: None,
                workspace_override: Some(WorkspaceLocalStateUpdate {
                    workspace_dir: workspace_key.clone(),
                    hidden: Some(None),
                    pinned: Some(None),
                    preferred_targets: Some(None),
                }),
            },
        );

        let resolved = resolve_session_state(&store, "codex", "abc", Some(&workspace_key));
        assert!(!resolved.hidden);
        assert!(resolved.pinned);
        assert_eq!(resolved.preferred_targets, vec!["claude"]);
        assert!(get_session_state(&store, "codex", "abc")
            .unwrap()
            .workspace_overrides
            .is_empty());
    }
}
