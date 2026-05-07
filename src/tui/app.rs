use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::widgets::ListState;

use crate::config;
use crate::core::{self, SessionGroup, SessionItem, SwitchParams, SwitchResult};
use crate::model::MemorphSession;

/// 当前显示的页面
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    SessionList,
    SessionDetail,
    SwitchFlow,
}

/// 应用事件处理结果
#[derive(Debug)]
pub enum AppResult {
    Continue,
    Quit,
    #[allow(dead_code)]
    Error(anyhow::Error),
}

/// TUI 应用状态机
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

    // 列表导航
    pub list_state: ListState,
    pub selected_provider_tab: usize,

    // 详情页滚动
    pub detail_scroll: usize,

    // 迁移向导状态
    pub switch_step: usize,
    pub switch_selection: Option<usize>,
    pub switch_target: Option<String>,
    pub switch_result: Option<SwitchResult>,
    pub switch_error: Option<String>,
}

impl App {
    pub fn new() -> Result<Self> {
        let cwd = std::env::current_dir()?;
        let cwd_str = cwd.to_string_lossy().to_string();

        let selected_provider_tab = config::workspace_providers(&cwd_str)
            .map(|providers| providers_to_tab(&providers))
            .unwrap_or(0);

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
            list_state: ListState::default(),
            selected_provider_tab,
            detail_scroll: 0,
            switch_step: 0,
            switch_selection: Some(0),
            switch_target: None,
            switch_result: None,
            switch_error: None,
        };

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
        self.list_state.select(None);
        Ok(())
    }

    fn get_filtered_providers(&self) -> Vec<String> {
        match self.selected_provider_tab {
            1 => vec!["claude".to_string()],
            2 => vec!["codex".to_string()],
            3 => vec!["opencode".to_string()],
            4 => vec!["cursor".to_string()],
            _ => Vec::new(),
        }
    }

    pub fn toggle_show_all(&mut self) {
        self.show_all = !self.show_all;
        if let Err(e) = self.load_sessions() {
            self.show_error(format!("Failed to load sessions: {}", e));
        }
    }

    pub fn next_provider_tab(&mut self) {
        self.selected_provider_tab = (self.selected_provider_tab + 1) % 5;
        self.save_provider_tab();
        if let Err(e) = self.load_sessions() {
            self.show_error(format!("Failed to load sessions: {}", e));
        }
    }

    pub fn select_provider_tab(&mut self, tab: usize) {
        if tab < 5 {
            self.selected_provider_tab = tab;
            self.save_provider_tab();
            if let Err(e) = self.load_sessions() {
                self.show_error(format!("Failed to load sessions: {}", e));
            }
        }
    }

    fn save_provider_tab(&self) {
        let Some(ref workspace) = self.workspace else { return };
        let providers = tab_to_providers(self.selected_provider_tab);
        let _ = config::set_workspace_providers(workspace, providers);
    }

    pub fn select_next(&mut self) {
        let flat_len: usize = self.session_groups.iter().map(|g| g.sessions.len()).sum();

        let current = self.list_state.selected().unwrap_or(0);
        if current + 1 < flat_len {
            self.list_state.select(Some(current + 1));
        }
    }

    pub fn select_previous(&mut self) {
        let current = self.list_state.selected().unwrap_or(0);
        if current > 0 {
            self.list_state.select(Some(current - 1));
        }
    }

    pub fn get_selected_session(&self) -> Option<&SessionItem> {
        let selected_idx = self.list_state.selected()?;
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

    pub fn execute_switch(&mut self) {
        let Some(selected) = &self.selected_session else {
            self.switch_error = Some("No session selected".to_string());
            return;
        };
        let Some(target) = &self.switch_target else {
            self.switch_error = Some("No target selected".to_string());
            return;
        };

        let params = SwitchParams {
            from: selected.provider_id.clone(),
            to: target.clone(),
            session_id: Some(selected.session_id.clone()),
            to_dir: self.workspace.clone(),
        };

        match core::switch_session(&params) {
            Ok(result) => {
                self.switch_result = Some(result);
            }
            Err(e) => {
                self.switch_error = Some(format!("Switch failed: {}", e));
            }
        }
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
            Screen::SessionDetail => super::screens::session_detail::handle_key(self, key),
            Screen::SwitchFlow => super::screens::switch_flow::handle_key(self, key),
        }
    }
}

/// 将 provider 列表映射到 TUI tab 索引。
/// 单选时返回对应 tab，多选或空列表时返回 0（All）。
fn providers_to_tab(providers: &[String]) -> usize {
    if providers.len() != 1 {
        return 0;
    }
    match providers[0].as_str() {
        "claude" => 1,
        "codex" => 2,
        "opencode" => 3,
        "cursor" => 4,
        _ => 0,
    }
}

/// 将 TUI tab 索引映射为 provider 列表，用于持久化到配置。
fn tab_to_providers(tab: usize) -> Vec<String> {
    match tab {
        1 => vec!["claude".to_string()],
        2 => vec!["codex".to_string()],
        3 => vec!["opencode".to_string()],
        4 => vec!["cursor".to_string()],
        _ => Vec::new(),
    }
}
