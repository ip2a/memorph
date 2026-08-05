use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::Theme;
use memorph::{config::UiLanguage, i18n};

/// Inline filter state — renders as a single line above/below the session table.
#[derive(Debug, Clone)]
pub struct FilterState {
    pub query: String,
    pub scope: FilterScope,
    pub match_count: usize,
    pub current_match: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterScope {
    All,
    Title,
    SessionId,
    Workspace,
}

impl FilterScope {
    pub fn label(self, language: UiLanguage) -> &'static str {
        match self {
            Self::All => i18n::text(language, "all"),
            Self::Title => i18n::text(language, "title"),
            Self::SessionId => i18n::text(language, "sessionId"),
            Self::Workspace => i18n::text(language, "workspacePath"),
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Title,
            Self::Title => Self::SessionId,
            Self::SessionId => Self::Workspace,
            Self::Workspace => Self::All,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::All => Self::Workspace,
            Self::Title => Self::All,
            Self::SessionId => Self::Title,
            Self::Workspace => Self::SessionId,
        }
    }
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            query: String::new(),
            scope: FilterScope::All,
            match_count: 0,
            current_match: 0,
        }
    }
}

impl FilterState {
    pub fn handle_key(&mut self, key: KeyEvent) -> FilterAction {
        match key.code {
            KeyCode::Esc => FilterAction::Close,
            KeyCode::Enter => FilterAction::Accept,
            KeyCode::Tab | KeyCode::Right => {
                self.scope = self.scope.next();
                FilterAction::Refilter
            }
            KeyCode::BackTab | KeyCode::Left => {
                self.scope = self.scope.prev();
                FilterAction::Refilter
            }
            KeyCode::Up => FilterAction::PrevMatch,
            KeyCode::Down => FilterAction::NextMatch,
            KeyCode::Char(ch) if !ch.is_control() => {
                self.query.push(ch);
                FilterAction::Refilter
            }
            KeyCode::Backspace => {
                self.query.pop();
                if self.query.is_empty() {
                    FilterAction::Close
                } else {
                    FilterAction::Refilter
                }
            }
            _ => FilterAction::Continue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterAction {
    Continue,
    Refilter,
    NextMatch,
    PrevMatch,
    Accept,
    Close,
}

/// Draw the inline filter bar (one line).
pub fn draw(
    frame: &mut Frame,
    state: &FilterState,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let match_info = if state.query.is_empty() {
        String::new()
    } else if state.match_count == 0 {
        format!(" ({})", i18n::text(language, "noMatchingSessions"))
    } else {
        format!(" ({}/{})", state.current_match + 1, state.match_count)
    };

    let line = Line::from(vec![
        Span::styled(
            format!("[{}] ", state.scope.label(language)),
            Style::default().fg(theme.secondary),
        ),
        Span::styled("/ ", Style::default().fg(theme.primary)),
        Span::styled(&state.query, Style::default().fg(theme.text)),
        Span::styled("█", Style::default().fg(theme.primary)),
        Span::styled(match_info, Style::default().fg(theme.text_dim)),
    ]);

    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(theme.background)),
        area,
    );
}
