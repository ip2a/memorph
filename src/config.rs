use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_SESSIONS_PER_PROVIDER: usize = 12;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum UiLanguage {
    Zh,
    En,
}

impl Default for UiLanguage {
    fn default() -> Self {
        Self::Zh
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
}

impl Default for WebPreferences {
    fn default() -> Self {
        Self {
            sessions_per_provider: DEFAULT_SESSIONS_PER_PROVIDER,
            language: UiLanguage::default(),
            show_opencode_subagents: default_show_opencode_subagents(),
        }
    }
}

fn default_sessions_per_provider() -> usize {
    DEFAULT_SESSIONS_PER_PROVIDER
}

fn default_show_opencode_subagents() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemorphConfig {
    #[serde(default)]
    pub workspaces: Vec<WorkspaceEntry>,
    #[serde(default)]
    pub selected_workspace: Option<String>,
    #[serde(default)]
    pub web: WebPreferences,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub path: String,
    pub last_viewed_at: i64,
}

pub fn config_path() -> Result<PathBuf> {
    let home = dirs::home_dir().context("Unable to locate user home directory")?;
    Ok(home.join(".memorph").join("config.json"))
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
    std::fs::write(&path, raw)
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

pub fn update_web_preferences(
    sessions_per_provider: Option<usize>,
    language: Option<UiLanguage>,
    show_opencode_subagents: Option<bool>,
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

    save_config(&config)
}

pub fn known_workspaces() -> Result<Vec<WorkspaceEntry>> {
    let mut workspaces = load_config()?.workspaces;
    workspaces.sort_by_key(|entry| std::cmp::Reverse(entry.last_viewed_at));
    Ok(workspaces)
}
