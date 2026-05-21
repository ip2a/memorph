use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};
use crossterm::event::KeyEvent;
use ratatui::widgets::TableState;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::config::UiLanguage;
use crate::core::{self, ExportParams, SessionGroup, SessionItem, SwitchParams};
use crate::i18n;
use crate::model::MemorphSession;
use crate::{config, providers};

pub fn provider_tabs(language: UiLanguage) -> Vec<String> {
    let mut tabs = vec![i18n::text(language, "all").to_string()];
    for filter in provider_tab_filters().into_iter().skip(1) {
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

fn provider_tab_filters() -> Vec<Vec<String>> {
    let prefs = config::web_preferences().unwrap_or_default();
    let mut filters = vec![Vec::new()];
    for id in config::primary_provider_ids(&prefs) {
        filters.push(vec![id]);
    }
    let folded = config::folded_provider_ids(&prefs);
    if !folded.is_empty() {
        filters.push(folded);
    }
    filters
}

pub const ACTION_OPTIONS: [SessionAction; 5] = [
    SessionAction::Switch,
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
    Export,
    Rename,
    Delete,
    Details,
}

impl SessionAction {
    pub fn label(self, language: UiLanguage) -> &'static str {
        match self {
            Self::Switch => i18n::text(language, "switch"),
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
    Settings,
    Sessions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsField {
    Language,
    SessionsPerProvider,
    ShowOpenCodeSubagents,
    AutoRefreshAfterDelete,
    PrimaryAgents,
    Save,
}

impl SettingsField {
    pub fn label(self, language: UiLanguage) -> &'static str {
        match self {
            Self::Language => i18n::text(language, "interfaceLanguage"),
            Self::SessionsPerProvider => i18n::text(language, "sessionsPerProvider"),
            Self::ShowOpenCodeSubagents => i18n::text(language, "showSubagents"),
            Self::AutoRefreshAfterDelete => i18n::text(language, "autoRefreshDelete"),
            Self::PrimaryAgents => i18n::text(language, "primaryAgents"),
            Self::Save => i18n::text(language, "save"),
        }
    }
}

pub const SETTINGS_FIELDS: [SettingsField; 6] = [
    SettingsField::Language,
    SettingsField::SessionsPerProvider,
    SettingsField::ShowOpenCodeSubagents,
    SettingsField::AutoRefreshAfterDelete,
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

/// TUI application state machine
pub struct App {
    pub current_screen: Screen,
    pub session_groups: Vec<SessionGroup>,
    pub selected_session: Option<SessionItem>,
    pub loaded_session: Option<MemorphSession>,
    pub workspace: Option<String>,
    pub show_all: bool,
    pub show_help: bool,
    pub error_message: Option<String>,
    pub error_timeout: Option<std::time::Instant>,
    pub ui_language: UiLanguage,

    // List navigation
    pub table_state: TableState,
    pub selected_provider_tab: usize,
    pub main_focus: MainFocus,

    // Top-level modal state
    pub workspace_modal_open: bool,
    pub settings_modal_open: bool,
    pub workspace_input: String,
    pub workspace_modal_index: usize,
    pub settings_selection: usize,
    pub settings_language: UiLanguage,
    pub settings_sessions_per_provider: usize,
    pub settings_show_opencode_subagents: bool,
    pub settings_auto_refresh_after_delete: bool,
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
    pub action_result: Option<ActionResult>,

    // Search modal state
    pub search_modal_open: bool,
    pub search_query: String,
    pub search_scope_index: usize,
    pub search_match_index: usize,

    // Detail modal state
    pub detail_modal_open: bool,
    pub detail_scroll: usize,
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
            selected_provider_tab: 0,
            main_focus: MainFocus::Sessions,
            workspace_modal_open: false,
            settings_modal_open: false,
            workspace_input: cwd_str.clone(),
            workspace_modal_index: 0,
            settings_selection: 0,
            settings_language: prefs.language,
            settings_sessions_per_provider: prefs.sessions_per_provider,
            settings_show_opencode_subagents: prefs.show_opencode_subagents,
            settings_auto_refresh_after_delete: prefs.auto_refresh_after_delete,
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
            action_result: None,
            search_modal_open: false,
            search_query: String::new(),
            search_scope_index: 0,
            search_match_index: 0,
            detail_modal_open: false,
            detail_scroll: 0,
        };

        app.refresh_workspace_options(None);
        app.load_sessions()?;
        Ok(app)
    }

    pub fn load_sessions(&mut self) -> Result<()> {
        let providers = self.get_filtered_providers();
        let params = core::SessionListParams {
            all: self.show_all,
            providers,
            cwd: self.workspace.clone(),
        };

        self.session_groups = core::list_sessions(&params)?;
        self.ensure_selected_row();
        Ok(())
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
        if self.selected_provider_tab == 0 {
            let prefs = config::web_preferences().unwrap_or_default();
            config::ordered_provider_ids(&prefs)
        } else {
            provider_tab_filters()
                .get(self.selected_provider_tab)
                .cloned()
                .unwrap_or_default()
        }
    }

    #[allow(dead_code)]
    pub fn toggle_show_all(&mut self) {
        self.show_all = !self.show_all;
        if let Err(e) = self.load_sessions() {
            self.show_error(self.tf("failedLoadSessions", &[("error", &e.to_string())]));
        }
    }

    pub fn next_provider_tab(&mut self) {
        let tab_count = provider_tabs(self.ui_language).len();
        self.selected_provider_tab = (self.selected_provider_tab + 1) % tab_count;
        self.save_provider_tab();
        if let Err(e) = self.load_sessions() {
            self.show_error(self.tf("failedLoadSessions", &[("error", &e.to_string())]));
        }
    }

    pub fn previous_provider_tab(&mut self) {
        let tab_count = provider_tabs(self.ui_language).len();
        self.selected_provider_tab = (self.selected_provider_tab + tab_count - 1) % tab_count;
        self.save_provider_tab();
        if let Err(e) = self.load_sessions() {
            self.show_error(self.tf("failedLoadSessions", &[("error", &e.to_string())]));
        }
    }

    #[allow(dead_code)]
    pub fn select_provider_tab(&mut self, tab: usize) {
        if tab < provider_tabs(self.ui_language).len() {
            self.selected_provider_tab = tab;
            self.save_provider_tab();
            if let Err(e) = self.load_sessions() {
                self.show_error(self.tf("failedLoadSessions", &[("error", &e.to_string())]));
            }
        }
    }

    fn save_provider_tab(&self) {
        let Some(ref workspace) = self.workspace else {
            return;
        };
        let providers = tab_to_providers(self.selected_provider_tab);
        let _ = config::set_workspace_providers(workspace, providers);
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
            self.loaded_session = Some(core::get_session(
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
            MainFocus::Settings => MainFocus::Workspace,
            MainFocus::Sessions => MainFocus::Sessions,
        };
    }

    pub fn focus_next_top_control(&mut self) {
        self.main_focus = match self.main_focus {
            MainFocus::Workspace => MainFocus::Settings,
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
    }

    pub fn close_workspace_modal(&mut self) {
        self.workspace_modal_open = false;
    }

    pub fn open_settings_modal(&mut self) {
        match config::web_preferences() {
            Ok(prefs) => {
                self.settings_language = prefs.language;
                self.settings_sessions_per_provider = prefs.sessions_per_provider;
                self.settings_show_opencode_subagents = prefs.show_opencode_subagents;
                self.settings_auto_refresh_after_delete = prefs.auto_refresh_after_delete;
                self.settings_agent_order = config::ordered_provider_ids(&prefs);
                self.settings_primary_agents = config::primary_provider_ids(&prefs);
                self.settings_agent_index = self
                    .settings_agent_index
                    .min(self.settings_agent_order.len().saturating_sub(1));
            }
            Err(e) => self.show_error(e.to_string()),
        }
        self.settings_selection = 0;
        self.settings_modal_open = true;
        self.workspace_modal_open = false;
    }

    pub fn close_settings_modal(&mut self) {
        self.settings_modal_open = false;
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
            SettingsField::ShowOpenCodeSubagents => {
                self.settings_show_opencode_subagents = !self.settings_show_opencode_subagents;
            }
            SettingsField::AutoRefreshAfterDelete => {
                self.settings_auto_refresh_after_delete = !self.settings_auto_refresh_after_delete;
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
            Some(self.settings_show_opencode_subagents),
            Some(self.settings_auto_refresh_after_delete),
        )
        .and_then(|_| {
            config::update_agent_display_preferences(
                self.settings_agent_order.clone(),
                self.settings_primary_agents.clone(),
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
                    .min(provider_tabs(self.ui_language).len().saturating_sub(1));
            }
            Err(e) => self.show_error(e.to_string()),
        }
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
        if let Err(e) = self.load_sessions() {
            self.show_error(self.tf("failedLoadSessions", &[("error", &e.to_string())]));
        }
    }

    pub fn open_detail_modal(&mut self) {
        let Some(selected) = self.selected_session.clone() else {
            return;
        };
        match core::get_session(&selected.provider_id, &selected.session_id) {
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
            .map(|session| session.messages.len().saturating_sub(1))
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
                    .map(|provider| provider.capabilities().write)
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
                self.action_field = ActionField::TargetWorkspace;
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
        };

        match core::switch_session(&params) {
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
        };

        match core::export_session(&params) {
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

        match core::rename_session(&selected.provider_id, &selected.session_id, &new_title) {
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

        match core::delete_session(&selected.provider_id, &selected.session_id) {
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
        if let Err(e) = self.load_sessions() {
            self.show_error(self.tf("failedRefreshSessions", &[("error", &e.to_string())]));
        }
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
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> AppResult {
        match self.current_screen {
            Screen::SessionList => super::screens::session_list::handle_key(self, key),
        }
    }
}

/// Map TUI tab index to provider list for config persistence.
fn tab_to_providers(tab: usize) -> Vec<String> {
    provider_tab_filters().get(tab).cloned().unwrap_or_default()
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
