use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{draw_overlay_frame, Theme};
use memorph::{config::UiLanguage, i18n};

/// What kind of confirmation is needed.
#[derive(Debug, Clone, Copy)]
pub enum ConfirmKind {
    DeleteSession,
}

/// Unified confirmation dialog state.
#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub kind: ConfirmKind,
    pub title: String,
    pub message: String,
    pub detail: String,
    pub confirm_label: String,
    /// Extra context to carry (e.g. session id, provider id).
    pub context: ConfirmContext,
}

#[derive(Debug, Clone, Default)]
pub struct ConfirmContext {
    pub provider_id: String,
    pub session_id: String,
}

impl ConfirmState {
    pub fn delete_session(
        title: impl Into<String>,
        message: impl Into<String>,
        detail: impl Into<String>,
        confirm_label: impl Into<String>,
        provider_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            kind: ConfirmKind::DeleteSession,
            title: title.into(),
            message: message.into(),
            detail: detail.into(),
            confirm_label: confirm_label.into(),
            context: ConfirmContext {
                provider_id: provider_id.into(),
                session_id: session_id.into(),
            },
        }
    }

    pub fn handle_key(&self, key: KeyEvent) -> ConfirmAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => ConfirmAction::Cancel,
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => ConfirmAction::Confirm,
            _ => ConfirmAction::Continue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Continue,
    Confirm,
    Cancel,
}

/// Draw the confirmation overlay.
pub fn draw(
    frame: &mut Frame,
    state: &ConfirmState,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let inner = draw_overlay_frame(frame, &state.title, 50, 30, area, theme);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    // Warning message
    let message = Paragraph::new(Line::from(Span::styled(
        &state.message,
        Style::default()
            .fg(theme.warning)
            .add_modifier(Modifier::BOLD),
    )))
    .style(Style::default().bg(theme.background));
    frame.render_widget(message, chunks[0]);

    // Detail
    if !state.detail.is_empty() {
        let detail = Paragraph::new(Line::from(Span::styled(
            &state.detail,
            Style::default().fg(theme.text_dim),
        )))
        .style(Style::default().bg(theme.background));
        frame.render_widget(detail, chunks[1]);
    }

    // Footer: confirm / cancel hints
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("[y/Enter] ", Style::default().fg(theme.error)),
        Span::styled(
            format!("{}  ", state.confirm_label),
            Style::default().fg(theme.text_dim),
        ),
        Span::styled("[n/Esc] ", Style::default().fg(theme.primary)),
        Span::styled(
            i18n::text(language, "cancel"),
            Style::default().fg(theme.text_dim),
        ),
    ]))
    .style(Style::default().bg(theme.background));
    frame.render_widget(footer, chunks[3]);
}
