use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::storage::atomic_write;

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
    false
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
    pub export: bool,
    #[serde(default = "default_false")]
    pub share: bool,
    #[serde(default = "default_false")]
    pub delete: bool,
}

impl Default for HomeButtonConfig {
    fn default() -> Self {
        Self {
            switch: true,
            view: true,
            export: true,
            share: false,
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
}

pub fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Unable to locate user home directory")?;
    Ok(home.join(".memorph").join("config.json"))
}

pub fn memorph_dir() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Unable to locate user home directory")?;
    Ok(home.join(".memorph"))
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

pub fn remember_workspace(path: &Path) -> Result<()> {
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "Workspace does not exist or is inaccessible: {}",
            path.display()
        )
    })?;
    let workspace = canonical.to_string_lossy().to_string();
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
        });
    }

    config
        .workspaces
        .sort_by_key(|entry| std::cmp::Reverse(entry.last_viewed_at));
    save_config(&config)
}

pub fn web_preferences() -> Result<WebPreferences> {
    Ok(load_config()?.web)
}

pub fn selected_workspace() -> Result<Option<String>> {
    Ok(load_config()?.selected_workspace)
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
        config.web.show_opencode_subagents = value;
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

pub fn update_agent_display_preferences(order: Vec<String>, primary: Vec<String>) -> Result<()> {
    let mut config = load_config()?;
    config.web.agent_display.order = normalize_provider_ids(order);
    config.web.agent_display.primary = normalize_provider_ids(primary);
    save_config(&config)
}

pub fn update_home_button_config(home_buttons: HomeButtonConfig) -> Result<()> {
    let mut config = load_config()?;
    config.web.home_buttons = home_buttons;
    save_config(&config)
}

pub fn ordered_provider_ids(prefs: &WebPreferences) -> Vec<String> {
    let mut ordered = normalize_provider_ids(prefs.agent_display.order.clone());
    for id in crate::providers::all_provider_ids() {
        if !ordered.iter().any(|existing| existing == id) {
            ordered.push((*id).to_string());
        }
    }
    ordered
}

pub fn primary_provider_ids(prefs: &WebPreferences) -> Vec<String> {
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
        let provider_id = provider_id.trim();
        if provider_id.is_empty()
            || !crate::providers::all_provider_ids()
                .iter()
                .any(|known| *known == provider_id)
            || normalized.iter().any(|existing| existing == provider_id)
        {
            continue;
        }
        normalized.push(provider_id.to_string());
    }
    normalized
}

pub fn known_workspaces() -> Result<Vec<WorkspaceEntry>> {
    let mut workspaces = load_config()?.workspaces;
    workspaces.sort_by_key(|entry| std::cmp::Reverse(entry.last_viewed_at));
    Ok(workspaces)
}

pub fn remove_workspace_history(workspace: &str) -> Result<Vec<WorkspaceEntry>> {
    let mut config = load_config()?;
    config.workspaces.retain(|entry| entry.path != workspace);
    if config.selected_workspace.as_deref() == Some(workspace) {
        config.selected_workspace = None;
    }
    save_config(&config)?;
    known_workspaces()
}

/// Get saved provider list for a workspace; returns the default list when unset.
pub fn workspace_providers(workspace: &str) -> Result<Vec<String>> {
    let config = load_config()?;
    let entry = config.workspaces.iter().find(|e| e.path == workspace);
    let providers = entry
        .and_then(|e| {
            let p = &e.providers;
            if p.is_empty() {
                None
            } else {
                Some(p.clone())
            }
        })
        .unwrap_or_else(|| {
            crate::providers::all_provider_ids()
                .iter()
                .map(|s| s.to_string())
                .collect()
        });
    Ok(providers)
}

/// Save provider list for a workspace into config.
pub fn set_workspace_providers(workspace: &str, providers: Vec<String>) -> Result<()> {
    let mut config = load_config()?;
    let workspace = workspace.to_string();

    if let Some(existing) = config.workspaces.iter_mut().find(|e| e.path == workspace) {
        existing.providers = providers;
    } else {
        config.workspaces.push(WorkspaceEntry {
            path: workspace,
            last_viewed_at: chrono::Utc::now().timestamp_millis(),
            providers,
        });
    }

    save_config(&config)
}
