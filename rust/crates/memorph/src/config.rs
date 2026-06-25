use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
#[cfg(test)]
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{storage::atomic_write, utils};

pub const DEFAULT_SESSIONS_PER_PROVIDER: usize = 12;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UiLanguage {
    Zh,
    En,
}

impl Default for UiLanguage {
    fn default() -> Self {
        Self::En
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebPreferences {
    #[serde(default = "default_sessions_per_provider")]
    pub sessions_per_provider: usize,
    #[serde(default)]
    pub language: UiLanguage,
    #[serde(default = "default_show_opencode_subagents")]
    pub show_opencode_subagents: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub provider_prefs: BTreeMap<String, Value>,
    #[serde(default = "default_sort_providers_by_session_count")]
    pub sort_providers_by_session_count: bool,
    #[serde(default = "default_backup_dir")]
    pub default_backup_dir: String,
    #[serde(default)]
    pub logging: LogPreferences,
    #[serde(default)]
    pub home_buttons: HomeButtonConfig,
    #[serde(default)]
    pub agent_display: AgentDisplayPreferences,
}

impl Default for WebPreferences {
    fn default() -> Self {
        Self {
            sessions_per_provider: DEFAULT_SESSIONS_PER_PROVIDER,
            language: UiLanguage::default(),
            show_opencode_subagents: default_show_opencode_subagents(),
            provider_prefs: BTreeMap::new(),
            sort_providers_by_session_count: default_sort_providers_by_session_count(),
            default_backup_dir: default_backup_dir(),
            logging: LogPreferences::default(),
            home_buttons: HomeButtonConfig::default(),
            agent_display: AgentDisplayPreferences::default(),
        }
    }
}

fn default_sessions_per_provider() -> usize {
    DEFAULT_SESSIONS_PER_PROVIDER
}

fn default_show_opencode_subagents() -> bool {
    crate::providers::legacy_web_preference_default_bool("show_opencode_subagents").unwrap_or(false)
}

fn default_sort_providers_by_session_count() -> bool {
    true
}

fn default_backup_dir() -> String {
    "./backups".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogPreferences {
    #[serde(default = "default_log_max_size_bytes")]
    pub max_size_bytes: u64,
    #[serde(default)]
    pub retention_days: Option<u32>,
}

impl Default for LogPreferences {
    fn default() -> Self {
        Self {
            max_size_bytes: default_log_max_size_bytes(),
            retention_days: None,
        }
    }
}

fn default_log_max_size_bytes() -> u64 {
    5 * 1024 * 1024
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HomeButtonConfig {
    #[serde(default = "default_true")]
    pub switch: bool,
    #[serde(default = "default_true")]
    pub view: bool,
    #[serde(default = "default_true")]
    pub compress: bool,
    #[serde(default = "default_true")]
    pub export: bool,
    #[serde(default = "default_false")]
    pub sync: bool,
    #[serde(default = "default_false")]
    pub delete: bool,
}

impl Default for HomeButtonConfig {
    fn default() -> Self {
        Self {
            switch: true,
            view: true,
            compress: true,
            export: true,
            sync: false,
            delete: false,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_false() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentDisplayPreferences {
    #[serde(default)]
    pub order: Vec<String>,
    #[serde(default)]
    pub primary: Vec<String>,
    #[serde(default)]
    pub sort_order: ProviderDisplayOrder,
    #[serde(default)]
    pub hidden_state: ProviderDisplayHidden,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderDisplayOrder {
    #[serde(default)]
    pub global: Vec<String>,
    #[serde(default)]
    pub workspace: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderDisplayHidden {
    #[serde(default)]
    pub global: Vec<String>,
    #[serde(default)]
    pub workspace: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DesktopWindowState {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesktopPreferences {
    #[serde(default)]
    pub window: Option<DesktopWindowState>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemorphConfig {
    #[serde(default)]
    pub workspaces: Vec<WorkspaceEntry>,
    #[serde(default)]
    pub selected_workspace: Option<String>,
    #[serde(default)]
    pub web: WebPreferences,
    #[serde(default)]
    pub desktop: DesktopPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub path: String,
    pub last_viewed_at: i64,
    #[serde(default)]
    pub providers: Vec<String>,
    #[serde(default)]
    pub sort_order: Vec<String>,
    #[serde(default)]
    pub hidden_state: Vec<String>,
}

#[cfg(test)]
thread_local! {
    static TEST_HOME_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn home_dir() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_HOME_DIR.with(|cell| cell.borrow().clone()) {
        return Ok(path);
    }

    dirs::home_dir().context("Unable to locate user home directory")
}

pub fn config_path() -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(home.join(".memorph").join("config.json"))
}

pub fn memorph_dir() -> Result<PathBuf> {
    let home = home_dir()?;
    Ok(home.join(".memorph"))
}

#[cfg(test)]
pub(crate) fn set_test_home_dir(path: PathBuf) {
    TEST_HOME_DIR.with(|cell| *cell.borrow_mut() = Some(path));
}

#[cfg(test)]
pub(crate) fn reset_test_home_dir() {
    TEST_HOME_DIR.with(|cell| *cell.borrow_mut() = None);
}

pub fn load_config() -> Result<MemorphConfig> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(MemorphConfig::default());
    }

    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))
}

pub fn save_config(config: &MemorphConfig) -> Result<()> {
    let path = config_path()?;
    let dir = path
        .parent()
        .context("Config file path has no parent directory")?;
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create config directory: {}", dir.display()))?;
    let raw = serde_json::to_string_pretty(config)?;
    atomic_write::write_string_atomic(&path, &raw)
        .with_context(|| format!("Failed to write config file: {}", path.display()))?;
    Ok(())
}

pub fn resolve_workspace(input: Option<&str>) -> Result<PathBuf> {
    let path = match input.map(str::trim).filter(|s| !s.is_empty()) {
        Some(value) => PathBuf::from(value),
        None => std::env::current_dir().context("Failed to read current working directory")?,
    };

    path.canonicalize().with_context(|| {
        format!(
            "Workspace does not exist or is inaccessible: {}",
            path.display()
        )
    })
}

fn canonical_workspace_string(path: &Path) -> Result<String> {
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "Workspace does not exist or is inaccessible: {}",
            path.display()
        )
    })?;
    Ok(canonical.to_string_lossy().to_string())
}

pub(crate) fn normalize_workspace_key(workspace: &str) -> Result<String> {
    let workspace = workspace.trim();
    if workspace.is_empty() {
        anyhow::bail!("Workspace path cannot be empty");
    }

    canonical_workspace_string(Path::new(workspace))
        .or_else(|_| Ok(PathBuf::from(workspace).to_string_lossy().to_string()))
}

pub fn remember_workspace(path: &Path) -> Result<()> {
    let workspace = canonical_workspace_string(path)?;
    let mut config = load_config()?;
    let now = chrono::Utc::now().timestamp_millis();
    config.selected_workspace = Some(workspace.clone());

    if let Some(existing) = config
        .workspaces
        .iter_mut()
        .find(|entry| entry.path == workspace)
    {
        existing.last_viewed_at = now;
    } else {
        config.workspaces.push(WorkspaceEntry {
            path: workspace,
            last_viewed_at: now,
            providers: Vec::new(),
            sort_order: Vec::new(),
            hidden_state: Vec::new(),
        });
    }

    config
        .workspaces
        .sort_by_key(|entry| std::cmp::Reverse(entry.last_viewed_at));
    save_config(&config)
}

pub fn web_preferences() -> Result<WebPreferences> {
    let mut prefs = load_config()?.web;
    hydrate_legacy_provider_preferences(&mut prefs);
    Ok(prefs)
}

pub fn selected_workspace() -> Result<Option<String>> {
    Ok(load_config()?
        .selected_workspace
        .map(|path| utils::user_visible_path(&path)))
}

pub fn desktop_window_state() -> Result<Option<DesktopWindowState>> {
    Ok(load_config()?.desktop.window)
}

pub fn set_desktop_window_state(state: DesktopWindowState) -> Result<()> {
    let mut config = load_config()?;
    config.desktop.window = Some(state);
    save_config(&config)
}

pub fn update_web_preferences(
    sessions_per_provider: Option<usize>,
    language: Option<UiLanguage>,
    show_opencode_subagents: Option<bool>,
    sort_providers_by_session_count: Option<bool>,
    backup_dir: Option<String>,
    logging: Option<LogPreferences>,
) -> Result<()> {
    let mut config = load_config()?;

    if let Some(value) = sessions_per_provider {
        config.web.sessions_per_provider = value.clamp(1, 200);
    }
    if let Some(value) = language {
        config.web.language = value;
    }
    if let Some(value) = show_opencode_subagents {
        let handled = crate::providers::apply_legacy_web_preference(
            &mut config.web,
            "show_opencode_subagents",
            &Value::Bool(value),
        )?;
        if !handled {
            anyhow::bail!(
                "Missing provider compatibility handler for legacy setting: show_opencode_subagents"
            );
        }
    }
    if let Some(value) = sort_providers_by_session_count {
        config.web.sort_providers_by_session_count = value;
    }
    if let Some(value) = backup_dir {
        let value = value.trim();
        config.web.default_backup_dir = if value.is_empty() {
            default_backup_dir()
        } else {
            value.to_string()
        };
    }
    if let Some(value) = logging {
        config.web.logging = value;
    }

    save_config(&config)
}

pub fn provider_preference(provider_id: &str, key: &str) -> Result<Option<Value>> {
    let prefs = web_preferences()?;
    Ok(provider_preference_from_prefs(&prefs, provider_id, key).cloned())
}

pub fn set_provider_preference(provider_id: &str, key: &str, value: Option<Value>) -> Result<()> {
    let mut config = load_config()?;
    set_provider_preference_in_prefs(&mut config.web, provider_id, key, value.clone())?;
    crate::providers::sync_legacy_field_from_provider_preference(
        &mut config.web,
        provider_id,
        key,
        value.as_ref(),
    );

    save_config(&config)
}

pub fn update_agent_display_preferences(
    sort_order: ProviderDisplayOrder,
    hidden_state: ProviderDisplayHidden,
) -> Result<()> {
    let mut config = load_config()?;
    config.web.agent_display.sort_order = ProviderDisplayOrder {
        global: normalize_provider_ids(sort_order.global),
        workspace: normalize_provider_ids(sort_order.workspace),
    };
    config.web.agent_display.hidden_state = ProviderDisplayHidden {
        global: normalize_provider_ids(hidden_state.global),
        workspace: normalize_provider_ids(hidden_state.workspace),
    };
    // Keep legacy `order` in sync with global sort order for backward compatibility.
    config.web.agent_display.order = config.web.agent_display.sort_order.global.clone();
    save_config(&config)
}

pub fn update_workspace_catalog_preferences(
    workspace: &str,
    sort_order: Vec<String>,
    hidden_state: Vec<String>,
) -> Result<()> {
    let workspace = normalize_workspace_key(workspace)?;
    let mut config = load_config()?;
    let now = chrono::Utc::now().timestamp_millis();

    if let Some(entry) = config
        .workspaces
        .iter_mut()
        .find(|entry| entry.path == workspace)
    {
        entry.sort_order = normalize_provider_ids(sort_order);
        entry.hidden_state = normalize_provider_ids(hidden_state);
        entry.last_viewed_at = now;
    } else {
        config.workspaces.push(WorkspaceEntry {
            path: workspace,
            last_viewed_at: now,
            providers: Vec::new(),
            sort_order: normalize_provider_ids(sort_order),
            hidden_state: normalize_provider_ids(hidden_state),
        });
    }

    config
        .workspaces
        .sort_by_key(|entry| std::cmp::Reverse(entry.last_viewed_at));
    save_config(&config)
}

pub fn update_home_button_config(home_buttons: HomeButtonConfig) -> Result<()> {
    let mut config = load_config()?;
    config.web.home_buttons = home_buttons;
    save_config(&config)
}

pub fn ordered_provider_ids(prefs: &WebPreferences) -> Vec<String> {
    let mut ordered = normalize_provider_ids(prefs.agent_display.sort_order.global.clone());
    // Migration: legacy `order` field feeds into global sort order when new field is empty.
    if ordered.is_empty() {
        ordered = normalize_provider_ids(prefs.agent_display.order.clone());
    }
    for id in crate::providers::all_provider_ids() {
        if !ordered.iter().any(|existing| existing == id) {
            ordered.push((*id).to_string());
        }
    }
    ordered
}

pub fn workspace_ordered_provider_ids(workspace: Option<&str>) -> Vec<String> {
    let Some(workspace) = workspace.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    let config = match load_config() {
        Ok(config) => config,
        Err(_) => return Vec::new(),
    };
    let workspace = match normalize_workspace_key(workspace) {
        Ok(path) => path,
        Err(_) => return Vec::new(),
    };
    config
        .workspaces
        .iter()
        .find(|entry| entry.path == workspace)
        .map(|entry| normalize_provider_ids(entry.sort_order.clone()))
        .unwrap_or_default()
}

pub fn global_hidden_provider_ids(prefs: &WebPreferences) -> Vec<String> {
    normalize_provider_ids(prefs.agent_display.hidden_state.global.clone())
}

pub fn workspace_hidden_provider_ids(workspace: Option<&str>) -> Vec<String> {
    let Some(workspace) = workspace.map(str::trim).filter(|value| !value.is_empty()) else {
        return Vec::new();
    };
    let config = match load_config() {
        Ok(config) => config,
        Err(_) => return Vec::new(),
    };
    let workspace = match normalize_workspace_key(workspace) {
        Ok(path) => path,
        Err(_) => return Vec::new(),
    };
    config
        .workspaces
        .iter()
        .find(|entry| entry.path == workspace)
        .map(|entry| normalize_provider_ids(entry.hidden_state.clone()))
        .unwrap_or_default()
}

pub fn primary_provider_ids(prefs: &WebPreferences) -> Vec<String> {
    // `primary` is deprecated in favor of explicit sort_order; kept for backward reads.
    let ordered = ordered_provider_ids(prefs);
    let primary = normalize_provider_ids(prefs.agent_display.primary.clone());
    if primary.is_empty() {
        return ordered;
    }

    ordered
        .into_iter()
        .filter(|id| primary.iter().any(|selected| selected == id))
        .collect()
}

pub fn folded_provider_ids(prefs: &WebPreferences) -> Vec<String> {
    let primary = normalize_provider_ids(prefs.agent_display.primary.clone());
    if primary.is_empty() {
        return Vec::new();
    }

    ordered_provider_ids(prefs)
        .into_iter()
        .filter(|id| !primary.iter().any(|selected| selected == id))
        .collect()
}

pub fn sort_provider_ids_by_display(
    prefs: &WebPreferences,
    provider_ids: &[String],
) -> Vec<String> {
    let provider_ids = normalize_provider_ids(provider_ids.to_vec());
    if provider_ids.is_empty() {
        return ordered_provider_ids(prefs);
    }

    let mut sorted = Vec::new();
    for id in ordered_provider_ids(prefs) {
        if provider_ids.iter().any(|provider| provider == &id) {
            sorted.push(id);
        }
    }
    sorted
}

pub fn normalize_provider_ids(provider_ids: Vec<String>) -> Vec<String> {
    let mut normalized = Vec::new();
    for provider_id in provider_ids {
        let provider_id = crate::providers::canonical_provider_id(&provider_id);
        if provider_id.is_empty()
            || !crate::providers::all_provider_ids()
                .iter()
                .any(|known| *known == provider_id)
            || normalized.iter().any(|existing| existing == &provider_id)
        {
            continue;
        }
        normalized.push(provider_id);
    }
    normalized
}

pub(crate) fn provider_preference_from_prefs<'a>(
    prefs: &'a WebPreferences,
    provider_id: &str,
    key: &str,
) -> Option<&'a Value> {
    let provider_id = crate::providers::canonical_provider_id(provider_id);
    prefs
        .provider_prefs
        .get(&provider_id)?
        .as_object()?
        .get(key)
}

fn hydrate_legacy_provider_preferences(prefs: &mut WebPreferences) {
    crate::providers::hydrate_legacy_preferences(prefs);
}

fn ensure_known_provider(provider_id: &str) -> Result<()> {
    let provider_id = crate::providers::canonical_provider_id(provider_id);
    if crate::providers::all_provider_ids()
        .iter()
        .any(|known| *known == provider_id)
    {
        return Ok(());
    }

    anyhow::bail!("Unknown provider: {}", provider_id);
}

pub(crate) fn set_provider_preference_in_prefs(
    prefs: &mut WebPreferences,
    provider_id: &str,
    key: &str,
    value: Option<Value>,
) -> Result<()> {
    ensure_known_provider(provider_id)?;

    let provider_id = crate::providers::canonical_provider_id(provider_id);
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("Provider preference key cannot be empty");
    }

    match value {
        Some(value) => {
            let entry = prefs
                .provider_prefs
                .entry(provider_id.clone())
                .or_insert_with(|| Value::Object(Map::new()));
            let object = entry.as_object_mut().with_context(|| {
                format!("Provider preferences are not an object: {}", provider_id)
            })?;
            object.insert(key.to_string(), value);
        }
        None => {
            let remove_provider = if let Some(entry) = prefs.provider_prefs.get_mut(&provider_id) {
                let object = entry.as_object_mut().with_context(|| {
                    format!("Provider preferences are not an object: {}", provider_id)
                })?;
                object.remove(key);
                object.is_empty()
            } else {
                false
            };
            if remove_provider {
                prefs.provider_prefs.remove(&provider_id);
            }
        }
    }

    Ok(())
}

pub fn known_workspaces() -> Result<Vec<WorkspaceEntry>> {
    let mut workspaces = load_config()?.workspaces;
    workspaces.sort_by_key(|entry| std::cmp::Reverse(entry.last_viewed_at));
    for entry in &mut workspaces {
        entry.path = utils::user_visible_path(&entry.path);
    }
    Ok(workspaces)
}

pub fn remove_workspace_history(workspace: &str) -> Result<Vec<WorkspaceEntry>> {
    let workspace = normalize_workspace_key(workspace)?;
    let mut config = load_config()?;
    config.workspaces.retain(|entry| entry.path != workspace);
    if config.selected_workspace.as_deref() == Some(workspace.as_str()) {
        config.selected_workspace = None;
    }
    save_config(&config)?;
    known_workspaces()
}

/// Get saved provider list for a workspace; returns the default list when unset.
pub fn workspace_providers(workspace: &str) -> Result<Vec<String>> {
    if let Some(providers) = workspace_provider_override(workspace)? {
        return Ok(providers);
    }

    Ok(crate::providers::all_provider_ids()
        .iter()
        .map(|s| s.to_string())
        .collect())
}

pub fn workspace_provider_override(workspace: &str) -> Result<Option<Vec<String>>> {
    let workspace = normalize_workspace_key(workspace)?;
    let config = load_config()?;
    Ok(config
        .workspaces
        .iter()
        .find(|e| e.path == workspace)
        .and_then(|e| {
            let providers = normalize_provider_ids(e.providers.clone());
            if providers.is_empty() {
                None
            } else {
                Some(providers)
            }
        }))
}

/// Save provider list for a workspace into config.
pub fn set_workspace_providers(workspace: &str, providers: Vec<String>) -> Result<()> {
    let mut config = load_config()?;
    let workspace = normalize_workspace_key(workspace)?;
    let providers = normalize_provider_ids(providers);

    if let Some(existing) = config.workspaces.iter_mut().find(|e| e.path == workspace) {
        existing.providers = providers;
    } else {
        config.workspaces.push(WorkspaceEntry {
            path: workspace,
            last_viewed_at: chrono::Utc::now().timestamp_millis(),
            providers,
            sort_order: Vec::new(),
            hidden_state: Vec::new(),
        });
    }

    save_config(&config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn normalize_workspace_key_canonicalizes_existing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let alias = dir.path().join(".");
        let normalized = normalize_workspace_key(alias.to_str().unwrap()).unwrap();
        assert_eq!(
            normalized,
            dir.path().canonicalize().unwrap().to_string_lossy()
        );
    }

    #[test]
    fn normalize_workspace_key_preserves_missing_path_string() {
        let path = "relative/missing-workspace";
        assert_eq!(normalize_workspace_key(path).unwrap(), path);
    }

    #[test]
    fn provider_preference_updates_remove_empty_provider_bucket() {
        let mut prefs = WebPreferences::default();

        set_provider_preference_in_prefs(
            &mut prefs,
            "codex",
            "sample_toggle",
            Some(Value::Bool(true)),
        )
        .unwrap();
        set_provider_preference_in_prefs(&mut prefs, "codex", "sample_toggle", None).unwrap();

        assert!(!prefs.provider_prefs.contains_key("codex"));
    }

    #[test]
    fn ordered_provider_ids_prefers_sort_order_global() {
        let mut prefs = WebPreferences::default();
        prefs.agent_display.sort_order.global = vec!["opencode".into(), "claude".into()];
        prefs.agent_display.order = vec!["codex".into()]; // legacy should be ignored

        let ordered = ordered_provider_ids(&prefs);
        assert_eq!(ordered[0], "opencode");
        assert_eq!(ordered[1], "claude");
        assert!(ordered.contains(&"codex".to_string()));
    }

    #[test]
    fn ordered_provider_ids_migrates_legacy_order() {
        let mut prefs = WebPreferences::default();
        prefs.agent_display.order = vec!["codex".into(), "claude".into()];

        let ordered = ordered_provider_ids(&prefs);
        assert_eq!(ordered[0], "codex");
        assert_eq!(ordered[1], "claude");
    }

    #[test]
    fn global_hidden_provider_ids_reads_hidden_state_global() {
        let mut prefs = WebPreferences::default();
        prefs.agent_display.hidden_state.global = vec!["cursor".into()];

        let hidden = global_hidden_provider_ids(&prefs);
        assert_eq!(hidden, vec!["cursor".to_string()]);
    }

    #[test]
    fn normalize_provider_ids_canonicalizes_aliases() {
        assert_eq!(
            normalize_provider_ids(vec![
                "factory".to_string(),
                "droid".to_string(),
                "oh-my-pi".to_string(),
                "omp".to_string(),
            ]),
            vec!["droid".to_string(), "omp".to_string()]
        );
    }
}
