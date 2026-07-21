use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use crossterm::event::KeyEvent;
use ratatui::widgets::TableState;
use serde_json::Value;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};

use crate::config::UiLanguage;
use crate::core::transfer::{ExportParams, SwitchParams};
use crate::core::{self, SessionDetailView, SessionGroup, SessionItem};
use crate::i18n;
use crate::storage::activity_store::ActivityActor;
use crate::{config, provider_settings, providers};

pub const ACTION_OPTIONS: [SessionAction; 6] = [
    SessionAction::Switch,
    SessionAction::Compress,
    SessionAction::Export,
    SessionAction::Rename,
    SessionAction::Delete,
    SessionAction::Details,
];
pub const SEARCH_SCOPE_OPTIONS: [SearchScope; 4] = [
    SearchScope::All,
    SearchScope::Title,
    SearchScope::SessionId,
    SearchScope::Workspace,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAction {
    Switch,
    Compress,
    Export,
    Rename,
    Delete,
    Details,
}

impl SessionAction {
    pub fn label(self, language: UiLanguage) -> &'static str {
        match self {
            Self::Switch => i18n::text(language, "switch"),
            Self::Compress => i18n::text(language, "compression"),
            Self::Export => i18n::text(language, "export"),
            Self::Rename => i18n::text(language, "rename"),
            Self::Delete => i18n::text(language, "remove"),
            Self::Details => i18n::text(language, "details"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionField {
    Action,
    TargetAgent,
    TargetWorkspace,
    CompressionCandidates,
    ExportPath,
    RenameTitle,
    Execute,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionDialog {
    TargetAgent,
    TargetWorkspace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainFocus {
    Workspace,
    Agents,
    Settings,
    Sessions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentManagementFocus {
    Providers,
    Actions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentManagementActionKind {
    Detect,
    Toggle,
    Action,
}

#[derive(Debug, Clone)]
pub struct AgentManagementAction {
    pub id: String,
    pub label: String,
    pub description: String,
    pub kind: AgentManagementActionKind,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Language,
    SessionsPerProvider,
    SortProvidersBySessionCount,
    PrimaryAgents,
    Save,
}

impl SettingsField {
    pub fn label(self, language: UiLanguage) -> &'static str {
        match self {
            Self::Language => i18n::text(language, "interfaceLanguage"),
            Self::SessionsPerProvider => i18n::text(language, "sessionsPerProvider"),
            Self::SortProvidersBySessionCount => {
                i18n::text(language, "sortProvidersBySessionCount")
            }
            Self::PrimaryAgents => i18n::text(language, "primaryAgents"),
            Self::Save => i18n::text(language, "save"),
        }
    }
}

pub const SETTINGS_FIELDS: [SettingsField; 5] = [
    SettingsField::Language,
    SettingsField::SessionsPerProvider,
    SettingsField::SortProvidersBySessionCount,
    SettingsField::PrimaryAgents,
    SettingsField::Save,
];

#[derive(Debug, Clone)]
pub struct ActionResult {
    pub title: String,
    pub lines: Vec<String>,
    pub is_error: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    All,
    Title,
    SessionId,
    Workspace,
}

impl SearchScope {
    pub fn label(self, language: UiLanguage) -> &'static str {
        match self {
            Self::All => i18n::text(language, "all"),
            Self::Title => i18n::text(language, "title"),
            Self::SessionId => i18n::text(language, "session"),
            Self::Workspace => i18n::text(language, "workspace"),
        }
    }
}

/// Currently displayed screen
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    SessionList,
}

/// Application event handling result
#[derive(Debug)]
pub enum AppResult {
    Continue,
    Quit,
    #[allow(dead_code)]
    Error(anyhow::Error),
}

struct SessionLoadPayload {
    provider_filters_cache: Vec<Vec<String>>,
    selected_provider_tab: usize,
    session_groups: Vec<SessionGroup>,
}

type SessionLoadMessage = std::result::Result<SessionLoadPayload, String>;

struct PendingSessionLoad {
    receiver: Receiver<SessionLoadMessage>,
    error_key: &'static str,
}

/// TUI application state machine
pub struct App {
    pub current_screen: Screen,
    pub session_groups: Vec<SessionGroup>,
    pub selected_session: Option<SessionItem>,
    pub loaded_session: Option<SessionDetailView>,
    pub workspace: Option<String>,
    pub show_all: bool,
    pub show_help: bool,
    pub error_message: Option<String>,
    pub error_timeout: Option<std::time::Instant>,
    pub ui_language: UiLanguage,

    // List navigation
    pub table_state: TableState,
    pub provider_filters_cache: Vec<Vec<String>>,
    pub selected_provider_tab: usize,
    pub main_focus: MainFocus,
    pub agent_management_entries: Vec<crate::agent_management::AgentManagementEntry>,
    pub agent_management_index: usize,
    pub agent_management_focus: AgentManagementFocus,
    pub agent_management_action_index: usize,

    // Top-level modal state
    pub workspace_modal_open: bool,
    pub agents_modal_open: bool,
    pub settings_modal_open: bool,
    pub workspace_input: String,
    pub workspace_modal_index: usize,
    pub settings_selection: usize,
    pub settings_language: UiLanguage,
    pub settings_sessions_per_provider: usize,
    pub settings_show_opencode_subagents: bool,
    pub settings_sort_providers_by_session_count: bool,
    pub settings_agent_order: Vec<String>,
    pub settings_primary_agents: Vec<String>,
    pub settings_agent_index: usize,

    // Action modal state
    pub action_modal_open: bool,
    pub action_selection: usize,
    pub action_field: ActionField,
    pub action_dialog: Option<ActionDialog>,
    pub switch_target_index: usize,
    pub workspace_options: Vec<String>,
    pub target_workspace: String,
    pub workspace_picker_index: usize,
    pub export_output_prefix: String,
    pub rename_input: String,
    pub compression_plan: Option<core::active_compression::ActiveCompressionReport>,
    pub compression_plan_error: Option<String>,
    pub compression_candidate_index: usize,
    pub compression_selected_candidate_ids: Vec<String>,
    pub action_result: Option<ActionResult>,
    pub agent_management_result: Option<ActionResult>,

    // Search modal state
    pub search_modal_open: bool,
    pub search_query: String,
    pub search_scope_index: usize,
    pub search_match_index: usize,

    // Detail modal state
    pub detail_modal_open: bool,
    pub detail_scroll: usize,
    pending_session_load: Option<PendingSessionLoad>,
    loading_frame: usize,
}

impl App {
    pub fn new() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let cwd_str = cwd.to_string_lossy().to_string();
        let prefs = config::web_preferences().unwrap_or_default();

        let mut app = Self {
            current_screen: Screen::SessionList,
            session_groups: Vec::new(),
            selected_session: None,
            loaded_session: None,
            workspace: Some(cwd_str.clone()),
            show_all: false,
            show_help: false,
            error_message: None,
            error_timeout: None,
            ui_language: prefs.language,
            table_state: TableState::default(),
            provider_filters_cache: Vec::new(),
            selected_provider_tab: 0,
            main_focus: MainFocus::Sessions,
            agent_management_entries: Vec::new(),
            agent_management_index: 0,
            agent_management_focus: AgentManagementFocus::Providers,
            agent_management_action_index: 0,
            workspace_modal_open: false,
            agents_modal_open: false,
            settings_modal_open: false,
            workspace_input: cwd_str.clone(),
            workspace_modal_index: 0,
            settings_selection: 0,
            settings_language: prefs.language,
            settings_sessions_per_provider: prefs.sessions_per_provider,
            settings_show_opencode_subagents: config::provider_preference_from_prefs(
                &prefs,
                "opencode",
                "show_subagents",
            )
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
            settings_sort_providers_by_session_count: prefs.sort_providers_by_session_count,
            settings_agent_order: config::ordered_provider_ids(&prefs),
            settings_primary_agents: config::primary_provider_ids(&prefs),
            settings_agent_index: 0,
            action_modal_open: false,
            action_selection: 0,
            action_field: ActionField::Action,
            action_dialog: None,
            switch_target_index: 0,
            workspace_options: vec![cwd_str.clone()],
            target_workspace: cwd_str.clone(),
            workspace_picker_index: 0,
            export_output_prefix: cwd_str.clone(),
            rename_input: String::new(),
            compression_plan: None,
            compression_plan_error: None,
            compression_candidate_index: 0,
            compression_selected_candidate_ids: Vec::new(),
            action_result: None,
            agent_management_result: None,
            search_modal_open: false,
            search_query: String::new(),
            search_scope_index: 0,
            search_match_index: 0,
            detail_modal_open: false,
            detail_scroll: 0,
            pending_session_load: None,
            loading_frame: 0,
        };

        app.refresh_workspace_options(None);
        app.start_load_sessions("failedLoadSessions");
        Ok(app)
    }

    fn start_load_sessions(&mut self, error_key: &'static str) {
        let workspace = self.workspace.clone();
        let show_all = self.show_all;
        let selected_provider_tab = self.selected_provider_tab;
        let (sender, receiver) = mpsc::channel();

        self.pending_session_load = Some(PendingSessionLoad {
            receiver,
            error_key,
        });
        self.loading_frame = 0;

        std::thread::spawn(move || {
            let result = build_session_load_payload(workspace, show_all, selected_provider_tab)
                .map_err(|error| error.to_string());
            let _ = sender.send(result);
        });
    }

    fn apply_session_load_payload(&mut self, payload: SessionLoadPayload) {
        let selected_identity = self
            .selected_session
            .as_ref()
            .map(|session| (session.provider_id.clone(), session.session_id.clone()));
        self.provider_filters_cache = payload.provider_filters_cache;
        self.selected_provider_tab = payload.selected_provider_tab;
        self.session_groups = payload.session_groups;

        if let Some((provider_id, session_id)) = selected_identity {
            let mut flat_index = 0;
            let mut restored = None;
            for group in &self.session_groups {
                for session in &group.sessions {
                    if session.provider_id == provider_id && session.session_id == session_id {
                        restored = Some((flat_index, session.clone()));
                        break;
                    }
                    flat_index += 1;
                }
                if restored.is_some() {
                    break;
                }
            }
            if let Some((index, session)) = restored {
                self.table_state.select(Some(index));
                self.selected_session = Some(session);
            } else {
                self.selected_session = None;
                self.loaded_session = None;
            }
        }
        self.ensure_selected_row();
    }

    pub fn is_loading(&self) -> bool {
        self.pending_session_load.is_some()
    }

    pub fn loading_spinner(&self) -> &'static str {
        const FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
        FRAMES[self.loading_frame % FRAMES.len()]
    }

    pub fn language(&self) -> UiLanguage {
        self.ui_language
    }

    pub fn t(&self, key: &'static str) -> &'static str {
        i18n::text(self.ui_language, key)
    }

    pub fn tf(&self, key: &'static str, replacements: &[(&str, &str)]) -> String {
        i18n::format(self.ui_language, key, replacements)
    }

    fn get_filtered_providers(&self) -> Vec<String> {
        self.provider_filters_cache
            .get(self.selected_provider_tab)
            .cloned()
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn toggle_show_all(&mut self) {
        self.show_all = !self.show_all;
        self.start_load_sessions("failedLoadSessions");
    }

    pub fn next_provider_tab(&mut self) {
        let tab_count = self.provider_tabs().len();
        self.selected_provider_tab = (self.selected_provider_tab + 1) % tab_count;
        self.start_load_sessions("failedLoadSessions");
    }

    pub fn previous_provider_tab(&mut self) {
        let tab_count = self.provider_tabs().len();
        self.selected_provider_tab = (self.selected_provider_tab + tab_count - 1) % tab_count;
        self.start_load_sessions("failedLoadSessions");
    }

    #[allow(dead_code)]
    pub fn select_provider_tab(&mut self, tab: usize) {
        if tab < self.provider_tabs().len() {
            self.selected_provider_tab = tab;
            self.start_load_sessions("failedLoadSessions");
        }
    }

    pub fn select_next(&mut self) {
        if self.main_focus != MainFocus::Sessions {
            self.main_focus = MainFocus::Sessions;
            return;
        }

        let flat_len = self.session_count();

        let current = self.table_state.selected().unwrap_or(0);
        if current + 1 < flat_len {
            self.table_state.select(Some(current + 1));
        }
    }

    pub fn select_previous(&mut self) {
        if self.table_state.selected().unwrap_or(0) == 0 {
            self.main_focus = MainFocus::Workspace;
            return;
        }

        let current = self.table_state.selected().unwrap_or(0);
        if current > 0 {
            self.table_state.select(Some(current - 1));
        }
    }

    pub fn get_selected_session(&self) -> Option<&SessionItem> {
        let selected_idx = self.table_state.selected()?;
        let mut current_idx = 0;

        for group in &self.session_groups {
            for session in &group.sessions {
                if current_idx == selected_idx {
                    return Some(session);
                }
                current_idx += 1;
            }
        }

        None
    }

    pub fn session_count(&self) -> usize {
        self.session_groups.iter().map(|g| g.sessions.len()).sum()
    }

    fn ensure_selected_row(&mut self) {
        let flat_len = self.session_count();
        if flat_len == 0 {
            self.table_state.select(None);
            return;
        }

        let selected = self
            .table_state
            .selected()
            .unwrap_or(0)
            .min(flat_len.saturating_sub(1));
        self.table_state.select(Some(selected));
    }

    #[allow(dead_code)]
    pub fn load_selected_session(&mut self) -> Result<()> {
        if let Some(selected) = &self.selected_session {
            self.loaded_session = Some(core::sessions::get_session_detail_view(
                &selected.provider_id,
                &selected.session_id,
            )?);
        }
        Ok(())
    }

    pub fn open_action_modal(&mut self) {
        if self.main_focus == MainFocus::Workspace {
            self.open_workspace_modal();
            return;
        }
        if self.main_focus == MainFocus::Agents {
            self.open_agents_modal();
            return;
        }
        if self.main_focus == MainFocus::Settings {
            self.open_settings_modal();
            return;
        }

        let Some(selected) = self.get_selected_session().cloned() else {
            return;
        };

        self.selected_session = Some(selected.clone());
        self.loaded_session = None;
        self.action_modal_open = true;
        self.action_selection = 0;
        self.action_field = ActionField::TargetAgent;
        self.action_dialog = None;
        self.switch_target_index = 0;
        self.rename_input = selected.title.clone().unwrap_or_default();
        self.action_result = None;
        self.refresh_workspace_options(Some(&selected));
        self.target_workspace = self
            .workspace
            .clone()
            .or_else(|| selected.project_dir.clone())
            .unwrap_or_else(|| ".".to_string());
        self.export_output_prefix = default_export_prefix(&selected, self.workspace.as_deref());
        self.compression_plan = None;
        self.compression_plan_error = None;
        self.compression_candidate_index = 0;
        self.compression_selected_candidate_ids.clear();
        self.sync_workspace_picker();
    }

    pub fn close_action_modal(&mut self) {
        self.action_modal_open = false;
        self.action_dialog = None;
        self.action_result = None;
        self.loaded_session = None;
    }

    pub fn focus_previous_top_control(&mut self) {
        self.main_focus = match self.main_focus {
            MainFocus::Workspace => MainFocus::Settings,
            MainFocus::Agents => MainFocus::Workspace,
            MainFocus::Settings => MainFocus::Agents,
            MainFocus::Sessions => MainFocus::Sessions,
        };
    }

    pub fn focus_next_top_control(&mut self) {
        self.main_focus = match self.main_focus {
            MainFocus::Workspace => MainFocus::Agents,
            MainFocus::Agents => MainFocus::Settings,
            MainFocus::Settings => MainFocus::Workspace,
            MainFocus::Sessions => MainFocus::Sessions,
        };
    }

    pub fn open_workspace_modal(&mut self) {
        self.refresh_workspace_options(None);
        self.workspace_input = self.workspace.clone().unwrap_or_else(|| ".".to_string());
        self.sync_main_workspace_picker();
        self.workspace_modal_open = true;
        self.settings_modal_open = false;
        self.agents_modal_open = false;
    }

    pub fn close_workspace_modal(&mut self) {
        self.workspace_modal_open = false;
    }

    pub fn open_settings_modal(&mut self) {
        match config::web_preferences() {
            Ok(prefs) => self.reload_settings_preferences(&prefs),
            Err(e) => self.show_error(e.to_string()),
        }
        self.settings_selection = 0;
        self.settings_modal_open = true;
        self.workspace_modal_open = false;
        self.agents_modal_open = false;
    }

    pub fn close_settings_modal(&mut self) {
        self.settings_modal_open = false;
    }

    pub fn open_agents_modal(&mut self) {
        if let Ok(prefs) = config::web_preferences() {
            self.reload_settings_preferences(&prefs);
        }
        self.agent_management_result = None;
        self.agent_management_focus = AgentManagementFocus::Providers;
        self.agent_management_action_index = 0;
        match crate::agent_management::list_agent_management_entries() {
            Ok(entries) => {
                let preferred = self
                    .get_filtered_providers()
                    .into_iter()
                    .find(|provider| provider != "all");
                self.agent_management_entries = entries;
                if let Some(provider) = preferred {
                    if let Some(index) = self
                        .agent_management_entries
                        .iter()
                        .position(|entry| entry.provider_id == provider)
                    {
                        self.agent_management_index = index;
                    }
                }
                self.agent_management_index = self
                    .agent_management_index
                    .min(self.agent_management_entries.len().saturating_sub(1));
                self.agent_management_action_index = self.agent_management_action_index.min(
                    self.current_agent_management_actions()
                        .len()
                        .saturating_sub(1),
                );
            }
            Err(error) => {
                self.agent_management_entries.clear();
                self.agent_management_index = 0;
                self.agent_management_action_index = 0;
                self.agent_management_result = Some(ActionResult {
                    title: self.t("agents").to_string(),
                    lines: vec![error.to_string()],
                    is_error: true,
                });
            }
        }
        self.agents_modal_open = true;
        self.workspace_modal_open = false;
        self.settings_modal_open = false;
    }

    pub fn close_agents_modal(&mut self) {
        self.agents_modal_open = false;
        self.agent_management_result = None;
    }

    pub fn run_primary_agent_management_action(&mut self) {
        self.run_selected_agent_management_action();
    }

    pub fn run_selected_agent_management_action(&mut self) {
        let Some(provider_id) = self.current_managed_provider_id() else {
            self.agent_management_result = Some(ActionResult {
                title: self.t("agents").to_string(),
                lines: vec![self.t("noProviders").to_string()],
                is_error: true,
            });
            return;
        };
        let Some(action) = self.selected_agent_management_action() else {
            self.agent_management_result = Some(ActionResult {
                title: self.t("agents").to_string(),
                lines: vec![self.t("noAgentManagementActionForProvider").to_string()],
                is_error: true,
            });
            return;
        };

        self.agent_management_result = Some(match action.kind {
            AgentManagementActionKind::Detect => {
                match crate::agent_management::detect_agent_management_entry(&provider_id) {
                    Ok(entry) => {
                        if let Some(index) = self
                            .agent_management_entries
                            .iter()
                            .position(|current| current.provider_id == provider_id)
                        {
                            self.agent_management_entries[index] = entry.clone();
                            self.agent_management_index = index;
                        }
                        self.agent_management_action_index =
                            self.agent_management_action_index.min(
                                self.current_agent_management_actions()
                                    .len()
                                    .saturating_sub(1),
                            );
                        ActionResult {
                            title: self.t("agents").to_string(),
                            lines: vec![
                                format!("{}: {}", self.t("provider"), provider_label(&provider_id)),
                                format!(
                                    "{}: {}",
                                    self.t("agentInstallStatus"),
                                    if entry.environment.installed {
                                        self.t("installed")
                                    } else {
                                        self.t("notDetected")
                                    }
                                ),
                                format!(
                                    "{}: {}",
                                    self.t("agentInstallMethod"),
                                    if entry.environment.install_method.trim().is_empty() {
                                        self.t("unknown")
                                    } else {
                                        entry.environment.install_method.as_str()
                                    }
                                ),
                                format!(
                                    "{}: {}",
                                    self.t("agentExecutablePath"),
                                    entry.environment.executable_path.as_deref().unwrap_or("—")
                                ),
                            ],
                            is_error: false,
                        }
                    }
                    Err(error) => ActionResult {
                        title: self.t("agents").to_string(),
                        lines: vec![error.to_string()],
                        is_error: true,
                    },
                }
            }
            AgentManagementActionKind::Action => {
                match provider_settings::run_provider_setting(
                    &provider_id,
                    &action.id,
                    provider_settings::ProviderSettingContext {
                        workspace: self.workspace.clone(),
                        actor: crate::storage::activity_store::ActivityActor::Tui,
                    },
                ) {
                    Ok(provider_settings::ProviderSettingOutput::HookOperation(report)) => {
                        let mut lines = vec![
                            format!("Provider: {}", report.provider),
                            format!("Operation: {}", report.operation),
                            format!("Changed: {}", report.changed),
                            format!("Status: {:?}", report.status.status),
                        ];
                        if let Some(message) = report.message {
                            lines.push(format!("Message: {}", message));
                        }
                        if let Some(config_path) = report.status.config_path {
                            lines.push(format!("Config: {}", config_path));
                        }
                        if let Some(backup_path) = report.backup_path {
                            lines.push(format!("Backup: {}", backup_path));
                        }
                        if let Ok(entries) =
                            crate::agent_management::list_agent_management_entries()
                        {
                            self.agent_management_entries = entries;
                            self.agent_management_index = self
                                .agent_management_index
                                .min(self.agent_management_entries.len().saturating_sub(1));
                        }
                        ActionResult {
                            title: self.t("agents").to_string(),
                            lines,
                            is_error: false,
                        }
                    }
                    Ok(provider_settings::ProviderSettingOutput::CodexWorkspaceRepair(report)) => {
                        let mut lines = vec![
                            format!("Workspace: {}", report.workspace_dir),
                            format!("Current provider: {}", report.current_model_provider),
                            format!("Scanned rollout files: {}", report.scanned_rollouts),
                            format!("Workspace sessions: {}", report.workspace_session_count),
                            format!("Hidden sessions: {}", report.hidden_session_count),
                            format!("Repaired sessions: {}", report.repaired_session_count),
                            format!("Reindexed sessions: {}", report.reindexed_session_count),
                            format!("Updated SQLite rows: {}", report.sqlite_rows_updated),
                        ];
                        if let Some(backup_dir) = &report.backup_dir {
                            lines.push(format!("Backup: {}", backup_dir));
                        }
                        if report.pruned_backup_count > 0 {
                            lines.push(format!("Pruned backups: {}", report.pruned_backup_count));
                        }
                        if !report.skipped_rollout_files.is_empty() {
                            lines.push(format!(
                                "Skipped rollout files: {}",
                                report.skipped_rollout_files.len()
                            ));
                        }
                        if report.touched_sessions.is_empty() {
                            lines.push("No Codex sessions needed sync.".to_string());
                        } else {
                            lines.push(String::new());
                            for item in report.touched_sessions {
                                lines.push(format!(
                                    "{} | {} | {} -> {} | index_added={}",
                                    item.session_id,
                                    item.title.unwrap_or_else(|| "(untitled)".to_string()),
                                    item.previous_model_provider
                                        .unwrap_or_else(|| "(none)".to_string()),
                                    item.current_model_provider,
                                    item.added_to_index
                                ));
                            }
                        }
                        if !report.skipped_rollout_files.is_empty() {
                            lines.push(String::new());
                            for path in report.skipped_rollout_files {
                                lines.push(format!("skipped rollout: {}", path));
                            }
                        }
                        self.start_load_sessions("failedRefreshSessions");
                        if let Ok(entries) =
                            crate::agent_management::list_agent_management_entries()
                        {
                            self.agent_management_entries = entries;
                            self.agent_management_index = self
                                .agent_management_index
                                .min(self.agent_management_entries.len().saturating_sub(1));
                        }
                        ActionResult {
                            title: self.t("agents").to_string(),
                            lines,
                            is_error: false,
                        }
                    }
                    Err(error) => ActionResult {
                        title: self.t("agents").to_string(),
                        lines: vec![error.to_string()],
                        is_error: true,
                    },
                }
            }
            AgentManagementActionKind::Toggle => {
                let next = !action.enabled.unwrap_or(false);
                match provider_settings::update_provider_setting(
                    &provider_id,
                    &action.id,
                    Some(Value::Bool(next)),
                ) {
                    Ok(_) => {
                        if let Ok(prefs) = config::web_preferences() {
                            self.reload_settings_preferences(&prefs);
                        }
                        self.start_load_sessions("failedRefreshSessions");
                        if let Ok(entries) =
                            crate::agent_management::list_agent_management_entries()
                        {
                            self.agent_management_entries = entries;
                            self.agent_management_index = self
                                .agent_management_index
                                .min(self.agent_management_entries.len().saturating_sub(1));
                            self.agent_management_action_index =
                                self.agent_management_action_index.min(
                                    self.current_agent_management_actions()
                                        .len()
                                        .saturating_sub(1),
                                );
                        }
                        ActionResult {
                            title: self.t("agents").to_string(),
                            lines: vec![format!(
                                "{}: {}",
                                action.label,
                                if next {
                                    self.t("enabled")
                                } else {
                                    self.t("disabled")
                                }
                            )],
                            is_error: false,
                        }
                    }
                    Err(error) => ActionResult {
                        title: self.t("agents").to_string(),
                        lines: vec![error.to_string()],
                        is_error: true,
                    },
                }
            }
        });
    }

    pub fn current_managed_provider_id(&self) -> Option<String> {
        self.selected_agent_management_entry()
            .map(|entry| entry.provider_id.clone())
    }

    pub fn current_agent_management_actions(&self) -> Vec<AgentManagementAction> {
        let Some(entry) = self.selected_agent_management_entry() else {
            return Vec::new();
        };
        let mut actions = vec![AgentManagementAction {
            id: "detect".to_string(),
            label: self.t("detect").to_string(),
            description: String::new(),
            kind: AgentManagementActionKind::Detect,
            enabled: None,
        }];
        for setting in &entry.settings {
            let kind = match setting.kind {
                provider_settings::SettingKind::Toggle => AgentManagementActionKind::Toggle,
                provider_settings::SettingKind::Action => AgentManagementActionKind::Action,
                provider_settings::SettingKind::View => continue,
            };
            actions.push(AgentManagementAction {
                id: setting.id.clone(),
                label: agent_management_setting_label(
                    self.ui_language,
                    &setting.id,
                    &setting.title,
                ),
                description: setting.description.clone(),
                kind,
                enabled: setting.value.as_ref().and_then(Value::as_bool),
            });
        }
        actions
    }

    pub fn selected_agent_management_action(&self) -> Option<AgentManagementAction> {
        let actions = self.current_agent_management_actions();
        actions
            .get(
                self.agent_management_action_index
                    .min(actions.len().saturating_sub(1)),
            )
            .cloned()
    }

    pub fn selected_agent_management_entry(
        &self,
    ) -> Option<&crate::agent_management::AgentManagementEntry> {
        self.agent_management_entries.get(
            self.agent_management_index
                .min(self.agent_management_entries.len().saturating_sub(1)),
        )
    }

    pub fn step_agent_management_selection(&mut self, forward: bool) {
        if self.agent_management_entries.is_empty() {
            self.agent_management_index = 0;
            return;
        }
        self.agent_management_index = cycle_index(
            self.agent_management_index,
            self.agent_management_entries.len(),
            forward,
        );
        self.agent_management_action_index = 0;
    }

    pub fn step_agent_management_action(&mut self, forward: bool) {
        let actions = self.current_agent_management_actions();
        if actions.is_empty() {
            self.agent_management_action_index = 0;
            return;
        }
        self.agent_management_action_index =
            cycle_index(self.agent_management_action_index, actions.len(), forward);
    }

    pub fn selected_settings_field(&self) -> SettingsField {
        SETTINGS_FIELDS
            .get(self.settings_selection)
            .copied()
            .unwrap_or(SettingsField::Language)
    }

    pub fn move_settings_previous(&mut self) {
        self.settings_selection =
            cycle_index(self.settings_selection, SETTINGS_FIELDS.len(), false);
    }

    pub fn move_settings_next(&mut self) {
        self.settings_selection = cycle_index(self.settings_selection, SETTINGS_FIELDS.len(), true);
    }

    pub fn cycle_settings_value(&mut self, forward: bool) {
        match self.selected_settings_field() {
            SettingsField::Language => {
                self.settings_language = match self.settings_language {
                    UiLanguage::Zh => UiLanguage::En,
                    UiLanguage::En => UiLanguage::Zh,
                };
            }
            SettingsField::SessionsPerProvider => {
                if forward {
                    self.settings_sessions_per_provider =
                        (self.settings_sessions_per_provider + 1).min(200);
                } else {
                    self.settings_sessions_per_provider =
                        self.settings_sessions_per_provider.saturating_sub(1).max(1);
                }
            }
            SettingsField::SortProvidersBySessionCount => {
                self.settings_sort_providers_by_session_count =
                    !self.settings_sort_providers_by_session_count;
            }
            SettingsField::PrimaryAgents => {
                if self.settings_agent_order.is_empty() {
                    return;
                }
                self.settings_agent_index = cycle_index(
                    self.settings_agent_index,
                    self.settings_agent_order.len(),
                    forward,
                );
            }
            SettingsField::Save => {}
        }
    }

    pub fn edit_settings_number(&mut self, key: crossterm::event::KeyCode) {
        if self.selected_settings_field() != SettingsField::SessionsPerProvider {
            return;
        }

        match key {
            crossterm::event::KeyCode::Char(ch) if ch.is_ascii_digit() => {
                let mut raw = self.settings_sessions_per_provider.to_string();
                if raw == "0" {
                    raw.clear();
                }
                raw.push(ch);
                if let Ok(value) = raw.parse::<usize>() {
                    self.settings_sessions_per_provider = value.clamp(1, 200);
                }
            }
            crossterm::event::KeyCode::Backspace => {
                let mut raw = self.settings_sessions_per_provider.to_string();
                raw.pop();
                self.settings_sessions_per_provider =
                    raw.parse::<usize>().unwrap_or(1).clamp(1, 200);
            }
            _ => {}
        }
    }

    pub fn activate_settings_field(&mut self) {
        if self.selected_settings_field() == SettingsField::Save {
            self.save_settings();
        } else if self.selected_settings_field() == SettingsField::PrimaryAgents {
            self.toggle_selected_settings_agent();
        } else {
            self.cycle_settings_value(true);
        }
    }

    pub fn toggle_selected_settings_agent(&mut self) {
        let Some(agent) = self
            .settings_agent_order
            .get(self.settings_agent_index)
            .cloned()
        else {
            return;
        };
        if let Some(index) = self
            .settings_primary_agents
            .iter()
            .position(|provider| provider == &agent)
        {
            self.settings_primary_agents.remove(index);
        } else {
            self.settings_primary_agents.push(agent);
            self.settings_primary_agents =
                config::normalize_provider_ids(self.settings_primary_agents.clone());
        }
    }

    pub fn save_settings(&mut self) {
        let result = config::update_web_preferences(
            Some(self.settings_sessions_per_provider),
            Some(self.settings_language),
            None,
            Some(self.settings_sort_providers_by_session_count),
            None,
            None,
        )
        .and_then(|_| {
            config::update_agent_display_preferences(
                config::ProviderDisplayOrder {
                    global: self.settings_agent_order.clone(),
                    workspace: Vec::new(),
                },
                config::ProviderDisplayHidden {
                    global: Vec::new(),
                    workspace: Vec::new(),
                },
            )
        });

        match result {
            Ok(()) => {
                self.close_settings_modal();
                self.ui_language = self.settings_language;
                self.show_error(
                    self.tf("settingsSavedPath", &[("path", "~/.memorph/config.json")]),
                );
                self.selected_provider_tab = self
                    .selected_provider_tab
                    .min(self.provider_tabs().len().saturating_sub(1));
                self.start_load_sessions("failedLoadSessions");
            }
            Err(e) => self.show_error(e.to_string()),
        }
    }

    fn reload_settings_preferences(&mut self, prefs: &config::WebPreferences) {
        self.settings_language = prefs.language;
        self.settings_sessions_per_provider = prefs.sessions_per_provider;
        self.settings_show_opencode_subagents =
            config::provider_preference_from_prefs(prefs, "opencode", "show_subagents")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
        self.settings_sort_providers_by_session_count = prefs.sort_providers_by_session_count;
        self.settings_agent_order = config::ordered_provider_ids(prefs);
        self.settings_primary_agents = config::primary_provider_ids(prefs);
        self.settings_agent_index = self
            .settings_agent_index
            .min(self.settings_agent_order.len().saturating_sub(1));
    }

    pub fn filtered_main_workspace_options(&self) -> Vec<String> {
        let query = self.workspace_input.trim().to_lowercase();
        if query.is_empty()
            || self
                .workspace_options
                .iter()
                .any(|workspace| workspace.eq_ignore_ascii_case(self.workspace_input.trim()))
        {
            return self.workspace_options.clone();
        }

        self.workspace_options
            .iter()
            .filter(|workspace| workspace.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    pub fn edit_main_workspace_input(&mut self, key: crossterm::event::KeyCode) {
        match key {
            crossterm::event::KeyCode::Char(ch) => {
                if !ch.is_control() {
                    self.workspace_input.push(ch);
                }
            }
            crossterm::event::KeyCode::Backspace => {
                self.workspace_input.pop();
            }
            _ => {}
        }
        self.sync_main_workspace_picker();
    }

    pub fn step_main_workspace_picker(&mut self, forward: bool) {
        let options = self.filtered_main_workspace_options();
        if options.is_empty() {
            self.workspace_modal_index = 0;
            return;
        }

        self.workspace_modal_index =
            cycle_index(self.workspace_modal_index, options.len(), forward);
        if let Some(option) = options.get(self.workspace_modal_index) {
            self.workspace_input = option.clone();
        }
    }

    pub fn confirm_workspace_modal(&mut self) {
        let workspace = self.workspace_input.trim();
        if workspace.is_empty() {
            self.show_error(self.t("workspaceEmptyError").to_string());
            return;
        }

        if let Err(e) = config::remember_workspace(Path::new(workspace)) {
            self.show_error(e.to_string());
            return;
        }

        match config::selected_workspace() {
            Ok(Some(workspace)) => {
                self.workspace = Some(workspace.clone());
                self.workspace_input = workspace;
            }
            _ => {
                self.workspace = Some(workspace.to_string());
            }
        }
        self.close_workspace_modal();
        self.refresh_workspace_options(None);
        self.start_load_sessions("failedLoadSessions");
    }

    pub fn open_detail_modal(&mut self) {
        let Some(selected) = self.selected_session.clone() else {
            return;
        };
        match core::sessions::get_session_detail_view(&selected.provider_id, &selected.session_id) {
            Ok(session) => {
                self.loaded_session = Some(session);
                self.detail_modal_open = true;
                self.detail_scroll = 0;
                self.action_modal_open = false;
                self.action_dialog = None;
                self.action_result = None;
            }
            Err(e) => self.set_action_error(self.t("detailsFailed"), vec![e.to_string()]),
        }
    }

    pub fn close_detail_modal(&mut self) {
        self.detail_modal_open = false;
        self.loaded_session = None;
        self.detail_scroll = 0;
    }

    pub fn detail_scroll_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(1);
    }

    pub fn detail_scroll_down(&mut self) {
        let max_scroll = self
            .loaded_session
            .as_ref()
            .map(|session| session.events.len().saturating_sub(1))
            .unwrap_or(0);
        self.detail_scroll = (self.detail_scroll + 1).min(max_scroll);
    }

    pub fn open_search_modal(&mut self) {
        self.search_modal_open = true;
        self.search_query.clear();
        self.search_scope_index = 0;
        self.search_match_index = self.table_state.selected().unwrap_or(0);
        self.sync_search_selection();
    }

    pub fn close_search_modal(&mut self) {
        self.search_modal_open = false;
        self.search_query.clear();
        self.search_match_index = 0;
    }

    pub fn current_search_scope(&self) -> SearchScope {
        SEARCH_SCOPE_OPTIONS
            .get(self.search_scope_index)
            .copied()
            .unwrap_or(SearchScope::All)
    }

    pub fn cycle_search_scope(&mut self, forward: bool) {
        self.search_scope_index =
            cycle_index(self.search_scope_index, SEARCH_SCOPE_OPTIONS.len(), forward);
        self.search_match_index = 0;
        self.sync_search_selection();
    }

    pub fn edit_search_query(&mut self, key: crossterm::event::KeyCode) {
        match key {
            crossterm::event::KeyCode::Char(ch) => {
                if !ch.is_control() {
                    self.search_query.push(ch);
                }
            }
            crossterm::event::KeyCode::Backspace => {
                self.search_query.pop();
            }
            _ => {}
        }
        self.search_match_index = 0;
        self.sync_search_selection();
    }

    pub fn next_search_match(&mut self) {
        let matches = self.search_matches();
        if matches.is_empty() {
            return;
        }
        self.search_match_index = (self.search_match_index + 1) % matches.len();
        self.sync_search_selection();
    }

    pub fn previous_search_match(&mut self) {
        let matches = self.search_matches();
        if matches.is_empty() {
            return;
        }
        self.search_match_index = (self.search_match_index + matches.len() - 1) % matches.len();
        self.sync_search_selection();
    }

    pub fn accept_search_selection(&mut self) {
        self.sync_search_selection();
        self.close_search_modal();
    }

    pub fn search_matches(&self) -> Vec<usize> {
        let query = self.search_query.trim().to_lowercase();
        let scope = self.current_search_scope();

        self.flattened_sessions()
            .iter()
            .enumerate()
            .filter(|(_, session)| session_matches(session, scope, &query))
            .map(|(index, _)| index)
            .collect()
    }

    fn sync_search_selection(&mut self) {
        let matches = self.search_matches();
        if matches.is_empty() {
            return;
        }
        let selected = matches[self.search_match_index.min(matches.len() - 1)];
        self.table_state.select(Some(selected));
    }

    pub fn flattened_sessions(&self) -> Vec<&SessionItem> {
        self.session_groups
            .iter()
            .flat_map(|group| group.sessions.iter())
            .collect()
    }

    pub fn current_action(&self) -> SessionAction {
        ACTION_OPTIONS
            .get(self.action_selection)
            .copied()
            .unwrap_or(SessionAction::Switch)
    }

    pub fn modal_fields(&self) -> Vec<ActionField> {
        match self.current_action() {
            SessionAction::Switch => vec![
                ActionField::Action,
                ActionField::TargetAgent,
                ActionField::TargetWorkspace,
                ActionField::Execute,
            ],
            SessionAction::Compress => vec![
                ActionField::Action,
                ActionField::TargetAgent,
                ActionField::CompressionCandidates,
                ActionField::Execute,
            ],
            SessionAction::Rename => vec![
                ActionField::Action,
                ActionField::RenameTitle,
                ActionField::Execute,
            ],
            SessionAction::Export => {
                vec![
                    ActionField::Action,
                    ActionField::ExportPath,
                    ActionField::Execute,
                ]
            }
            SessionAction::Delete | SessionAction::Details => {
                vec![ActionField::Action, ActionField::Execute]
            }
        }
    }

    pub fn move_modal_field_next(&mut self) {
        let fields = self.modal_fields();
        let pos = fields
            .iter()
            .position(|field| *field == self.action_field)
            .unwrap_or(0);
        self.action_field = fields[(pos + 1) % fields.len()];
    }

    pub fn move_modal_field_previous(&mut self) {
        let fields = self.modal_fields();
        let pos = fields
            .iter()
            .position(|field| *field == self.action_field)
            .unwrap_or(0);
        self.action_field = fields[(pos + fields.len() - 1) % fields.len()];
    }

    pub fn cycle_modal_value(&mut self, forward: bool) {
        match self.action_field {
            ActionField::Action => {
                self.action_selection =
                    cycle_index(self.action_selection, ACTION_OPTIONS.len(), forward);
                self.normalize_action_field();
                if self.current_action() == SessionAction::Compress {
                    self.refresh_compression_plan();
                }
            }
            ActionField::CompressionCandidates => {
                self.step_compression_candidate(forward);
            }
            ActionField::TargetAgent
            | ActionField::TargetWorkspace
            | ActionField::ExportPath
            | ActionField::RenameTitle
            | ActionField::Execute => {}
        }
    }

    pub fn target_provider_options(&self) -> Vec<&'static str> {
        let source = self
            .selected_session
            .as_ref()
            .map(|session| session.provider_id.as_str());

        providers::all_provider_ids()
            .iter()
            .copied()
            .filter(|provider| Some(*provider) != source)
            .filter(|provider| {
                providers::find_provider(provider)
                    .map(|provider| provider.capabilities().export)
                    .unwrap_or(false)
            })
            .collect()
    }

    pub fn selected_target_provider(&self) -> Option<&'static str> {
        let options = self.target_provider_options();
        options.get(self.switch_target_index).copied()
    }

    pub fn selected_target_workspace(&self) -> Option<String> {
        let workspace = self.target_workspace.trim();
        if workspace.is_empty() {
            self.workspace.clone().or_else(|| Some(".".to_string()))
        } else {
            Some(workspace.to_string())
        }
    }

    pub fn active_compression_candidates(
        &self,
    ) -> &[core::active_compression::CompressionCandidateReport] {
        self.compression_plan
            .as_ref()
            .map(|report| report.candidates.as_slice())
            .unwrap_or(&[])
    }

    pub fn selected_compression_candidate_id(&self) -> Option<&str> {
        self.active_compression_candidates()
            .get(
                self.compression_candidate_index
                    .min(self.active_compression_candidates().len().saturating_sub(1)),
            )
            .map(|candidate| candidate.id.as_str())
    }

    pub fn compression_candidate_selected(&self, candidate_id: &str) -> bool {
        self.compression_selected_candidate_ids
            .iter()
            .any(|id| id == candidate_id)
    }

    pub fn step_compression_candidate(&mut self, forward: bool) {
        let len = self.active_compression_candidates().len();
        self.compression_candidate_index =
            cycle_index(self.compression_candidate_index, len, forward);
    }

    pub fn toggle_selected_compression_candidate(&mut self) {
        let Some(candidate_id) = self.selected_compression_candidate_id().map(str::to_string)
        else {
            return;
        };
        if let Some(index) = self
            .compression_selected_candidate_ids
            .iter()
            .position(|id| id == &candidate_id)
        {
            self.compression_selected_candidate_ids.remove(index);
        } else {
            self.compression_selected_candidate_ids.push(candidate_id);
        }
    }

    fn refresh_compression_plan(&mut self) {
        self.compression_plan = None;
        self.compression_plan_error = None;
        self.compression_candidate_index = 0;
        self.compression_selected_candidate_ids.clear();

        let Some(selected) = self.selected_session.clone() else {
            self.compression_plan_error = Some(self.t("noSessionSelected").to_string());
            return;
        };
        let Some(target) = self.selected_target_provider() else {
            self.compression_plan_error = Some(self.t("noTargetAgentSelected").to_string());
            return;
        };

        let mut policy = core::active_compression::ActiveCompressionPolicy::default();
        policy.mode = core::active_compression::ActiveCompressionMode::PlanOnly;
        match core::compression_application::active_compression_dry_run(&core::compression_application::ActiveCompressionDryRunParams {
            source_provider_id: selected.provider_id,
            target_provider_id: target.to_string(),
            session_id: Some(selected.session_id),
            file: None,
            policy,
        }) {
            Ok(report) => {
                self.compression_selected_candidate_ids = report
                    .candidates
                    .iter()
                    .map(|candidate| candidate.id.clone())
                    .collect();
                self.compression_plan = Some(report);
            }
            Err(error) => {
                self.compression_plan_error = Some(error.to_string());
            }
        }
    }

    pub fn filtered_workspace_options(&self) -> Vec<String> {
        let query = self.target_workspace.trim().to_lowercase();
        if query.is_empty()
            || self
                .workspace_options
                .iter()
                .any(|workspace| workspace.eq_ignore_ascii_case(self.target_workspace.trim()))
        {
            return self.workspace_options.clone();
        }

        self.workspace_options
            .iter()
            .filter(|workspace| workspace.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    pub fn open_action_dialog(&mut self) {
        match self.action_field {
            ActionField::TargetAgent => {
                if !self.target_provider_options().is_empty() {
                    self.action_dialog = Some(ActionDialog::TargetAgent);
                }
            }
            ActionField::TargetWorkspace => {
                if self.target_workspace.trim().is_empty() {
                    self.target_workspace =
                        self.workspace.clone().unwrap_or_else(|| ".".to_string());
                }
                self.sync_workspace_picker();
                self.action_dialog = Some(ActionDialog::TargetWorkspace);
            }
            _ => {}
        }
    }

    pub fn close_action_dialog(&mut self) {
        self.action_dialog = None;
    }

    pub fn activate_modal_field(&mut self) {
        match self.action_field {
            ActionField::Action => self.move_modal_field_next(),
            ActionField::TargetAgent | ActionField::TargetWorkspace => self.open_action_dialog(),
            ActionField::CompressionCandidates => self.toggle_selected_compression_candidate(),
            ActionField::ExportPath | ActionField::RenameTitle | ActionField::Execute => {
                self.execute_modal_action()
            }
        }
    }

    pub fn cycle_action_dialog_selection(&mut self, forward: bool) {
        match self.action_dialog {
            Some(ActionDialog::TargetAgent) => {
                let len = self.target_provider_options().len();
                self.switch_target_index = cycle_index(self.switch_target_index, len, forward);
            }
            Some(ActionDialog::TargetWorkspace) => self.step_workspace_picker(forward),
            None => {}
        }
    }

    pub fn confirm_action_dialog(&mut self) {
        match self.action_dialog {
            Some(ActionDialog::TargetAgent) => {
                self.action_dialog = None;
                if self.current_action() == SessionAction::Compress {
                    self.refresh_compression_plan();
                    self.action_field = ActionField::CompressionCandidates;
                } else {
                    self.action_field = ActionField::TargetWorkspace;
                }
            }
            Some(ActionDialog::TargetWorkspace) => {
                if self.target_workspace.trim().is_empty() {
                    self.target_workspace =
                        self.workspace.clone().unwrap_or_else(|| ".".to_string());
                }
                self.action_dialog = None;
                self.action_field = ActionField::Execute;
            }
            None => {}
        }
    }

    pub fn edit_rename_input(&mut self, key: crossterm::event::KeyCode) {
        match key {
            crossterm::event::KeyCode::Char(ch) => {
                if !ch.is_control() {
                    self.rename_input.push(ch);
                }
            }
            crossterm::event::KeyCode::Backspace => {
                self.rename_input.pop();
            }
            _ => {}
        }
    }

    pub fn edit_workspace_input(&mut self, key: crossterm::event::KeyCode) {
        match key {
            crossterm::event::KeyCode::Char(ch) => {
                if !ch.is_control() {
                    self.target_workspace.push(ch);
                }
            }
            crossterm::event::KeyCode::Backspace => {
                self.target_workspace.pop();
            }
            _ => {}
        }
        self.sync_workspace_picker();
    }

    pub fn edit_export_output_prefix(&mut self, key: crossterm::event::KeyCode) {
        match key {
            crossterm::event::KeyCode::Char(ch) => {
                if !ch.is_control() {
                    self.export_output_prefix.push(ch);
                }
            }
            crossterm::event::KeyCode::Backspace => {
                self.export_output_prefix.pop();
            }
            _ => {}
        }
    }

    pub fn execute_modal_action(&mut self) {
        if self.action_result.is_some() {
            self.close_action_modal();
            return;
        }

        if self.current_action() == SessionAction::Delete
            && self.action_field != ActionField::Execute
        {
            self.action_field = ActionField::Execute;
            return;
        }

        match self.current_action() {
            SessionAction::Switch => self.execute_modal_switch(),
            SessionAction::Compress => self.execute_modal_compress(),
            SessionAction::Export => self.execute_modal_export(),
            SessionAction::Rename => self.execute_modal_rename(),
            SessionAction::Delete => self.execute_modal_delete(),
            SessionAction::Details => self.execute_modal_details(),
        }
    }

    fn execute_modal_switch(&mut self) {
        let Some(selected) = self.selected_session.clone() else {
            self.set_action_error(
                self.t("switchFailed"),
                vec![self.t("noSessionSelected").to_string()],
            );
            return;
        };
        let Some(target) = self.selected_target_provider() else {
            self.set_action_error(
                self.t("switchFailed"),
                vec![self.t("noTargetAgentSelected").to_string()],
            );
            return;
        };

        let params = SwitchParams {
            from: selected.provider_id.clone(),
            to: target.to_string(),
            session_id: Some(selected.session_id.clone()),
            to_dir: self.selected_target_workspace(),
            target_title: None,
            move_original: false,
        };

        match core::transfer::switch_session(&params) {
            Ok(result) => {
                let resume = result
                    .resume_command
                    .as_deref()
                    .unwrap_or(self.t("resumeNotAvailable"));
                let mut lines = vec![
                    format!("{}: {}", self.t("fromLabel"), result.from_name),
                    format!("{}: {}", self.t("toLabel"), result.to_name),
                    format!("{}: {}", self.t("source"), result.source_session_id),
                    format!("{}: {}", self.t("target"), result.target_session_id),
                    format!("{}: {}", self.t("resume"), resume),
                ];
                if let Some(command) = result.resume_command.as_deref() {
                    match copy_to_clipboard(command) {
                        Ok(()) => lines.push(self.t("resumeCopied").to_string()),
                        Err(e) => {
                            lines.push(self.tf("clipboardCopyFailed", &[("error", &e.to_string())]))
                        }
                    }
                }
                self.set_action_success(self.t("switchComplete"), lines);
                self.reload_after_action();
            }
            Err(e) => self.set_action_error(self.t("switchFailed"), vec![e.to_string()]),
        }
    }

    fn execute_modal_compress(&mut self) {
        let Some(selected) = self.selected_session.clone() else {
            self.set_action_error(
                self.t("compressionTitle"),
                vec![self.t("noSessionSelected").to_string()],
            );
            return;
        };
        let Some(target) = self.selected_target_provider() else {
            self.set_action_error(
                self.t("compressionTitle"),
                vec![self.t("noTargetAgentSelected").to_string()],
            );
            return;
        };
        if self.compression_selected_candidate_ids.is_empty() {
            self.set_action_error(
                self.t("compressionTitle"),
                vec!["No compression candidates selected.".to_string()],
            );
            return;
        }

        let mut policy = core::active_compression::ActiveCompressionPolicy::default();
        policy.mode = core::active_compression::ActiveCompressionMode::Auto;
        let params = core::compression_application::ActiveCompressionApplyCommandParams {
            source_provider_id: selected.provider_id.clone(),
            target_provider_id: target.to_string(),
            session_id: Some(selected.session_id.clone()),
            file: None,
            policy,
            candidate_ids: self.compression_selected_candidate_ids.clone(),
            output_prefix: Some(default_compression_prefix(
                &selected,
                self.workspace.as_deref(),
            )),
            format: "json".to_string(),
        };

        match core::compression_application::active_compression_apply(&params, ActivityActor::Tui) {
            Ok(result) => {
                let mut lines = vec![
                    format!("Applied candidates: {}", result.report.candidates.len()),
                    format!(
                        "Estimated bytes saved: {}",
                        result.report.estimated_bytes_saved
                    ),
                    format!(
                        "Estimated tokens saved: {}",
                        result.report.estimated_tokens_saved
                    ),
                ];
                for file in result.files {
                    lines.push(format!("File: {}", file));
                }
                for archive_ref in result.archive_refs {
                    lines.push(format!("Archive: {}", archive_ref));
                }
                self.set_action_success(self.t("compressionTitle"), lines);
                self.reload_after_action();
            }
            Err(error) => {
                self.set_action_error(self.t("compressionTitle"), vec![error.to_string()])
            }
        }
    }

    fn execute_modal_export(&mut self) {
        let Some(selected) = self.selected_session.clone() else {
            self.set_action_error(
                self.t("exportFailed"),
                vec![self.t("noSessionSelected").to_string()],
            );
            return;
        };

        let output_prefix = self.export_output_prefix.trim();
        if output_prefix.is_empty() {
            self.set_action_error(
                self.t("exportFailed"),
                vec![self.t("outputPrefixEmpty").to_string()],
            );
            return;
        }

        let params = ExportParams {
            provider: selected.provider_id,
            session_id: selected.session_id,
            output_prefix: Some(output_prefix.to_string()),
            format: "json".to_string(),
            output_dir: None,
        };

        match core::transfer::export_session(&params, ActivityActor::Tui) {
            Ok(result) => self.set_action_success(self.t("exportComplete"), result.files),
            Err(e) => self.set_action_error(self.t("exportFailed"), vec![e.to_string()]),
        }
    }

    fn execute_modal_rename(&mut self) {
        let Some(selected) = self.selected_session.clone() else {
            self.set_action_error(
                self.t("renameFailed"),
                vec![self.t("noSessionSelected").to_string()],
            );
            return;
        };
        let new_title = self.rename_input.trim().to_string();
        if new_title.is_empty() {
            self.set_action_error(
                self.t("renameFailed"),
                vec![self.t("titleEmpty").to_string()],
            );
            return;
        }

        match core::session_mutation::rename_session(
            &selected.provider_id,
            &selected.session_id,
            &new_title,
            ActivityActor::Tui,
        ) {
            Ok(result) => {
                let mut lines = vec![
                    format!("Display title: {}", result.display_title),
                    format!("Native title updated: {}", result.native_updated),
                ];
                if let Some(warning) = result.warning {
                    lines.push(format!("Warning: {}", warning));
                }
                self.set_action_success(self.t("renameComplete"), lines);
                self.reload_after_action();
            }
            Err(e) => self.set_action_error(self.t("renameFailed"), vec![e.to_string()]),
        }
    }

    fn execute_modal_delete(&mut self) {
        let Some(selected) = self.selected_session.clone() else {
            self.set_action_error(
                self.t("deleteFailed"),
                vec![self.t("noSessionSelected").to_string()],
            );
            return;
        };

        match core::session_mutation::delete_session(
            &selected.provider_id,
            &selected.session_id,
            ActivityActor::Tui,
        ) {
            Ok(()) => {
                self.set_action_success(self.t("deleteComplete"), vec![selected.session_id]);
                self.reload_after_action();
            }
            Err(e) => self.set_action_error(self.t("deleteFailed"), vec![e.to_string()]),
        }
    }

    fn execute_modal_details(&mut self) {
        if self.selected_session.is_none() {
            self.set_action_error(
                self.t("detailsFailed"),
                vec![self.t("noSessionSelected").to_string()],
            );
            return;
        }
        self.open_detail_modal();
    }

    fn refresh_workspace_options(&mut self, selected: Option<&SessionItem>) {
        let mut options = Vec::new();
        push_unique(&mut options, self.workspace.clone());
        if let Some(session) = selected {
            push_unique(&mut options, session.project_dir.clone());
        }
        if let Ok(workspaces) = config::known_workspaces() {
            for workspace in workspaces {
                push_unique(&mut options, Some(workspace.path));
            }
        }
        if options.is_empty() {
            options.push(".".to_string());
        }

        self.workspace_options = options;
        self.sync_workspace_picker();
    }

    fn normalize_action_field(&mut self) {
        let fields = self.modal_fields();
        if !fields.contains(&self.action_field) {
            self.action_field = ActionField::Action;
        }
    }

    fn step_workspace_picker(&mut self, forward: bool) {
        let options = self.filtered_workspace_options();
        if options.is_empty() {
            self.workspace_picker_index = 0;
            return;
        }

        self.workspace_picker_index =
            cycle_index(self.workspace_picker_index, options.len(), forward);
        if let Some(option) = options.get(self.workspace_picker_index) {
            self.target_workspace = option.clone();
        }
    }

    fn sync_workspace_picker(&mut self) {
        let options = self.filtered_workspace_options();
        if options.is_empty() {
            self.workspace_picker_index = 0;
            return;
        }

        if let Some(index) = options
            .iter()
            .position(|option| option == &self.target_workspace)
        {
            self.workspace_picker_index = index;
        } else {
            self.workspace_picker_index = self
                .workspace_picker_index
                .min(options.len().saturating_sub(1));
        }
    }

    fn sync_main_workspace_picker(&mut self) {
        let options = self.filtered_main_workspace_options();
        if options.is_empty() {
            self.workspace_modal_index = 0;
            return;
        }

        if let Some(index) = options
            .iter()
            .position(|option| option == &self.workspace_input)
        {
            self.workspace_modal_index = index;
        } else {
            self.workspace_modal_index = self
                .workspace_modal_index
                .min(options.len().saturating_sub(1));
        }
    }

    fn reload_after_action(&mut self) {
        self.start_load_sessions("failedRefreshSessions");
    }

    fn set_action_success(&mut self, title: impl Into<String>, lines: Vec<String>) {
        self.action_result = Some(ActionResult {
            title: title.into(),
            lines,
            is_error: false,
        });
    }

    fn set_action_error(&mut self, title: impl Into<String>, lines: Vec<String>) {
        self.action_result = Some(ActionResult {
            title: title.into(),
            lines,
            is_error: true,
        });
    }

    pub fn show_error(&mut self, msg: String) {
        self.error_message = Some(msg);
        self.error_timeout = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
    }

    pub fn clear_error(&mut self) {
        self.error_message = None;
        self.error_timeout = None;
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
    }

    pub fn on_tick(&mut self) {
        if let Some(timeout) = self.error_timeout {
            if std::time::Instant::now() >= timeout {
                self.clear_error();
            }
        }

        if let Some((error_key, result)) = self.pending_session_load.as_ref().and_then(|pending| {
            match pending.receiver.try_recv() {
                Ok(result) => Some((pending.error_key, result)),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => Some((
                    pending.error_key,
                    Err("session loader disconnected".to_string()),
                )),
            }
        }) {
            self.pending_session_load = None;
            match result {
                Ok(payload) => self.apply_session_load_payload(payload),
                Err(error) => self.show_error(self.tf(error_key, &[("error", &error)])),
            }
        } else if self.pending_session_load.is_some() {
            self.loading_frame = self.loading_frame.wrapping_add(1);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppResult {
        match self.current_screen {
            Screen::SessionList => super::screens::session_list::handle_key(self, key),
        }
    }

    pub fn provider_tabs(&self) -> Vec<String> {
        provider_tabs_from_filters(self.ui_language, &self.provider_filters_cache)
    }

    pub fn provider_filters(&self) -> Vec<Vec<String>> {
        self.provider_filters_cache.clone()
    }
}

fn provider_tabs_from_filters(language: UiLanguage, filters: &[Vec<String>]) -> Vec<String> {
    let mut tabs = vec![i18n::text(language, "all").to_string()];
    for filter in filters.iter().skip(1) {
        if filter.len() == 1 {
            let id = &filter[0];
            tabs.push(
                providers::find_provider(id)
                    .map(|p| p.name().to_string())
                    .unwrap_or_else(|| id.clone()),
            );
        } else {
            tabs.push(i18n::text(language, "more").to_string());
        }
    }
    tabs
}

fn build_session_load_payload(
    workspace: Option<String>,
    show_all: bool,
    selected_provider_tab: usize,
) -> Result<SessionLoadPayload> {
    let prefs = config::web_preferences().unwrap_or_default();
    let workspace_provider_override = match workspace.as_deref() {
        Some(workspace) => config::workspace_provider_override(workspace)?,
        None => None,
    };
    let base_provider_ids = workspace_provider_override
        .as_ref()
        .map(|provider_ids| config::sort_provider_ids_by_display(&prefs, provider_ids))
        .unwrap_or_else(|| config::ordered_provider_ids(&prefs));
    let should_sort_by_session_count =
        workspace_provider_override.is_none() && prefs.sort_providers_by_session_count;
    let primary_provider_ids = config::normalize_provider_ids(prefs.agent_display.primary.clone());

    let (provider_filters_cache, selected_provider_tab, session_groups) =
        if should_sort_by_session_count {
            let params = core::SessionListParams {
                all: show_all,
                providers: base_provider_ids.clone(),
                cwd: workspace,
                include_message_counts: false,
                limit: None,
                offset: None,
                sort: core::SessionListSort::Recent,
                hook_filter: core::SessionHookFilter::All,
            };
            let mut session_groups = core::projection::list_sessions(&params)?;
            apply_session_group_preferences(&mut session_groups, &prefs);
            let counts = session_groups
                .iter()
                .map(|group| (group.provider_id.clone(), group.sessions.len()))
                .collect();
            let ordered_provider_ids =
                sort_provider_ids_by_session_counts(base_provider_ids, &counts);
            let provider_filters_cache =
                build_provider_tab_filters(ordered_provider_ids, primary_provider_ids);
            let tab_count = provider_filters_cache.len();
            let selected_provider_tab = selected_provider_tab.min(tab_count.saturating_sub(1));
            let selected_providers = provider_filters_cache
                .get(selected_provider_tab)
                .cloned()
                .unwrap_or_default();
            let session_groups = select_session_groups(session_groups, &selected_providers);
            (
                provider_filters_cache,
                selected_provider_tab,
                session_groups,
            )
        } else {
            let provider_filters_cache =
                build_provider_tab_filters(base_provider_ids, primary_provider_ids);
            let tab_count = provider_filters_cache.len();
            let selected_provider_tab = selected_provider_tab.min(tab_count.saturating_sub(1));
            let selected_providers = provider_filters_cache
                .get(selected_provider_tab)
                .cloned()
                .unwrap_or_default();
            let params = core::SessionListParams {
                all: show_all,
                providers: selected_providers.clone(),
                cwd: workspace,
                include_message_counts: false,
                limit: None,
                offset: None,
                sort: core::SessionListSort::Recent,
                hook_filter: core::SessionHookFilter::All,
            };
            let mut session_groups = core::projection::list_sessions(&params)?;
            apply_session_group_preferences(&mut session_groups, &prefs);
            let session_groups = select_session_groups(session_groups, &selected_providers);
            (
                provider_filters_cache,
                selected_provider_tab,
                session_groups,
            )
        };

    Ok(SessionLoadPayload {
        provider_filters_cache,
        selected_provider_tab,
        session_groups,
    })
}

fn sort_provider_ids_by_session_counts(
    mut provider_ids: Vec<String>,
    counts: &HashMap<String, usize>,
) -> Vec<String> {
    let index_map: HashMap<String, usize> = provider_ids
        .iter()
        .enumerate()
        .map(|(index, provider_id)| (provider_id.clone(), index))
        .collect();
    provider_ids.sort_by(|left, right| {
        let left_count = counts.get(left).copied().unwrap_or(0);
        let right_count = counts.get(right).copied().unwrap_or(0);
        right_count
            .cmp(&left_count)
            .then_with(|| index_map.get(left).cmp(&index_map.get(right)))
    });
    provider_ids
}

fn build_provider_tab_filters(ordered: Vec<String>, primary: Vec<String>) -> Vec<Vec<String>> {
    let mut filters = vec![ordered.clone()];

    if primary.is_empty() {
        for id in ordered {
            filters.push(vec![id]);
        }
        return filters;
    }

    for id in &ordered {
        if primary.iter().any(|selected| selected == id) {
            filters.push(vec![id.clone()]);
        }
    }

    let folded: Vec<String> = ordered
        .into_iter()
        .filter(|id| !primary.iter().any(|selected| selected == id))
        .collect();
    if !folded.is_empty() {
        filters.push(folded);
    }

    filters
}

fn apply_session_group_preferences(
    session_groups: &mut Vec<SessionGroup>,
    prefs: &config::WebPreferences,
) {
    let show_opencode_subagents =
        config::provider_preference_from_prefs(prefs, "opencode", "show_subagents")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    if !show_opencode_subagents {
        for group in session_groups.iter_mut() {
            if group.provider_id == "opencode" {
                group
                    .sessions
                    .retain(|session| !is_opencode_subagent_title(session.title.as_deref()));
            }
        }
    }

    session_groups.retain(|group| !group.sessions.is_empty());
}

fn select_session_groups(
    mut session_groups: Vec<SessionGroup>,
    selected_providers: &[String],
) -> Vec<SessionGroup> {
    let order: HashMap<String, usize> = selected_providers
        .iter()
        .enumerate()
        .map(|(index, provider_id)| (provider_id.clone(), index))
        .collect();

    session_groups.retain(|group| order.contains_key(&group.provider_id));
    session_groups
        .sort_by_key(|group| order.get(&group.provider_id).copied().unwrap_or(usize::MAX));
    session_groups
}

fn copy_to_clipboard(text: &str) -> Result<()> {
    if try_platform_clipboard(text).is_ok() {
        return Ok(());
    }

    let encoded = general_purpose::STANDARD.encode(text.as_bytes());
    let mut stdout = std::io::stdout();
    write!(stdout, "\x1b]52;c;{}\x07", encoded)?;
    stdout.flush()?;
    Ok(())
}

fn try_platform_clipboard(text: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if write_clipboard_command("pbcopy", &[], text).is_ok() {
            return Ok(());
        }
    }

    #[cfg(target_os = "windows")]
    {
        if write_clipboard_command("cmd", &["/C", "clip"], text).is_ok() {
            return Ok(());
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        for (program, args) in [
            ("wl-copy", &[][..]),
            ("xclip", &["-selection", "clipboard"][..]),
            ("xsel", &["--clipboard", "--input"][..]),
        ] {
            if write_clipboard_command(program, args, text).is_ok() {
                return Ok(());
            }
        }
    }

    anyhow::bail!(i18n::text(UiLanguage::En, "noPlatformClipboard"))
}

fn write_clipboard_command(program: &str, args: &[&str], text: &str) -> Result<()> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(text.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("{} exited with {}", program, status)
    }
}

pub fn provider_label(provider_id: &str) -> &'static str {
    providers::find_provider(provider_id)
        .map(|p| p.name())
        .unwrap_or("Unknown")
}

fn agent_management_setting_label(language: UiLanguage, setting_id: &str, title: &str) -> String {
    match setting_id {
        "repair_workspace_sessions" => i18n::text(language, "repairCurrentWorkspaceSessions"),
        "show_subagents" => i18n::text(language, "showSubagents"),
        _ if !title.trim().is_empty() => title,
        _ => setting_id,
    }
    .to_string()
}

fn is_opencode_subagent_title(title: Option<&str>) -> bool {
    let Some(title) = title else {
        return false;
    };
    title.contains("(@") && title.contains(" subagent)")
}

fn push_unique(options: &mut Vec<String>, value: Option<String>) {
    let Some(value) = value else { return };
    if value.trim().is_empty() || options.iter().any(|existing| existing == &value) {
        return;
    }
    options.push(value);
}

fn default_export_prefix(session: &SessionItem, workspace: Option<&str>) -> String {
    let base = workspace
        .filter(|value| !value.trim().is_empty())
        .or(session.project_dir.as_deref());

    match base {
        Some(dir) => PathBuf::from(dir)
            .join(&session.session_id)
            .display()
            .to_string(),
        None => session.session_id.clone(),
    }
}

fn default_compression_prefix(session: &SessionItem, workspace: Option<&str>) -> String {
    let base = workspace
        .filter(|value| !value.trim().is_empty())
        .or(session.project_dir.as_deref());
    let filename = format!("{}_active_compressed", session.session_id);

    match base {
        Some(dir) => PathBuf::from(dir).join(filename).display().to_string(),
        None => filename,
    }
}

fn cycle_index(current: usize, len: usize, forward: bool) -> usize {
    if len == 0 {
        return 0;
    }

    if forward {
        (current + 1) % len
    } else {
        (current + len - 1) % len
    }
}

fn session_matches(session: &SessionItem, scope: SearchScope, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }

    let title = session.title.as_deref().unwrap_or("").to_lowercase();
    let native_title = session.native_title.as_deref().unwrap_or("").to_lowercase();
    let session_id = session.session_id.to_lowercase();
    let workspace = session.project_dir.as_deref().unwrap_or("").to_lowercase();

    match scope {
        SearchScope::All => {
            title.contains(query)
                || native_title.contains(query)
                || session_id.contains(query)
                || workspace.contains(query)
        }
        SearchScope::Title => title.contains(query) || native_title.contains(query),
        SearchScope::SessionId => session_id.contains(query),
        SearchScope::Workspace => workspace.contains(query),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_provider_ids_by_session_counts_descending() {
        let provider_ids = vec![
            "claude".to_string(),
            "codex".to_string(),
            "cursor".to_string(),
            "opencode".to_string(),
        ];
        let counts = HashMap::from([
            ("cursor".to_string(), 5_usize),
            ("claude".to_string(), 2_usize),
            ("opencode".to_string(), 2_usize),
        ]);

        let sorted = sort_provider_ids_by_session_counts(provider_ids, &counts);

        assert_eq!(
            sorted,
            vec![
                "cursor".to_string(),
                "claude".to_string(),
                "opencode".to_string(),
                "codex".to_string(),
            ]
        );
    }

    #[test]
    fn builds_provider_tabs_with_primary_and_folded_groups() {
        let ordered = vec![
            "cursor".to_string(),
            "claude".to_string(),
            "opencode".to_string(),
            "codex".to_string(),
        ];
        let primary = vec!["claude".to_string(), "codex".to_string()];

        let filters = build_provider_tab_filters(ordered, primary);

        assert_eq!(
            filters,
            vec![
                vec![
                    "cursor".to_string(),
                    "claude".to_string(),
                    "opencode".to_string(),
                    "codex".to_string(),
                ],
                vec!["claude".to_string()],
                vec!["codex".to_string()],
                vec!["cursor".to_string(), "opencode".to_string()],
            ]
        );
    }

    #[test]
    fn action_options_include_compression_after_switch() {
        assert_eq!(ACTION_OPTIONS[0], SessionAction::Switch);
        assert_eq!(ACTION_OPTIONS[1], SessionAction::Compress);
    }

    #[test]
    fn compression_action_uses_target_candidates_and_execute_fields() {
        let mut app = App::new().unwrap();
        app.action_modal_open = true;
        app.action_selection = ACTION_OPTIONS
            .iter()
            .position(|action| *action == SessionAction::Compress)
            .unwrap();
        app.action_field = ActionField::Action;

        assert_eq!(
            app.modal_fields(),
            vec![
                ActionField::Action,
                ActionField::TargetAgent,
                ActionField::CompressionCandidates,
                ActionField::Execute,
            ]
        );
    }

    #[test]
    fn toggles_selected_compression_candidate() {
        let mut app = App::new().unwrap();
        app.compression_plan = Some(core::active_compression::ActiveCompressionReport {
            source_provider_id: "claude".to_string(),
            target_provider_id: "codex".to_string(),
            dry_run: true,
            policy: core::active_compression::ActiveCompressionPolicy::default(),
            token_estimator: core::active_compression::CompressionTokenEstimatorReport::default(),
            session_event_count: 1,
            message_event_count: 1,
            already_compressed_event_count: 0,
            original_estimated_bytes: 1000,
            original_estimated_tokens: 250,
            compressed_estimated_bytes: 500,
            compressed_estimated_tokens: 125,
            estimated_bytes_saved: 500,
            estimated_tokens_saved: 125,
            candidates: vec![core::active_compression::CompressionCandidateReport {
                id: "candidate-0001".to_string(),
                kind: core::active_compression::CompressionCandidateKind::LargeToolOutput,
                event_ids: vec!["event-1".to_string()],
                start_event_index: 0,
                end_event_index: 0,
                reason: core::active_compression::CompressionSelectionReason::LargeToolOutput,
                risk: core::active_compression::CompressionRisk::Low,
                original_estimated_bytes: 1000,
                original_estimated_tokens: 250,
                compressed_estimated_bytes: 500,
                compressed_estimated_tokens: 125,
                estimated_bytes_saved: 500,
                estimated_tokens_saved: 125,
                archive_refs: Vec::new(),
            }],
            skipped: Vec::new(),
            archive_refs: Vec::new(),
        });
        app.compression_selected_candidate_ids = vec!["candidate-0001".to_string()];

        app.toggle_selected_compression_candidate();
        assert!(app.compression_selected_candidate_ids.is_empty());

        app.toggle_selected_compression_candidate();
        assert_eq!(
            app.compression_selected_candidate_ids,
            vec!["candidate-0001".to_string()]
        );
    }
}
