use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{draw_overlay_frame, Theme};
use memorph::{config::UiLanguage, i18n};

/// What kind of input is open — determines confirm behavior.
#[derive(Debug, Clone, Copy)]
pub enum InputKind {
    Rename,
    ExportPath,
    WorkspacePath,
}

/// Unified text input state.
#[derive(Debug, Clone)]
pub struct InputState {
    pub kind: InputKind,
    pub title: String,
    pub prompt: String,
    pub value: String,
    pub placeholder: String,
}

impl InputState {
    pub fn new(
        kind: InputKind,
        title: impl Into<String>,
        prompt: impl Into<String>,
        initial_value: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            title: title.into(),
            prompt: prompt.into(),
            value: initial_value.into(),
            placeholder: String::new(),
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> InputAction {
        match key.code {
            KeyCode::Esc => InputAction::Cancel,
            KeyCode::Enter => {
                if self.value.trim().is_empty() {
                    InputAction::Continue
                } else {
                    InputAction::Confirm
                }
            }
            KeyCode::Char(ch) if !ch.is_control() => {
                self.value.push(ch);
                InputAction::Continue
            }
            KeyCode::Backspace => {
                self.value.pop();
                InputAction::Continue
            }
            _ => InputAction::Continue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputAction {
    Continue,
    Confirm,
    Cancel,
}

/// Draw the input overlay.
pub fn draw(
    frame: &mut Frame,
    state: &InputState,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let inner = draw_overlay_frame(frame, &state.title, 60, 30, area, theme);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(inner);

    // Prompt text
    let prompt = Paragraph::new(Line::from(Span::styled(
        &state.prompt,
        Style::default().fg(theme.text_dim),
    )))
    .style(Style::default().bg(theme.background));
    frame.render_widget(prompt, chunks[0]);

    // Input value with cursor
    let display_value = if state.value.is_empty() {
        Span::styled(&state.placeholder, Style::default().fg(theme.text_dim))
    } else {
        Span::styled(&state.value, Style::default().fg(theme.text))
    };
    let input_line = Line::from(vec![
        Span::styled("> ", Style::default().fg(theme.primary)),
        display_value,
        Span::styled("█", Style::default().fg(theme.primary)),
    ]);
    frame.render_widget(
        Paragraph::new(input_line).style(Style::default().bg(theme.background)),
        chunks[2],
    );

    // Footer hint
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("[Enter] ", Style::default().fg(theme.primary)),
        Span::styled(
            format!("{}  ", i18n::text(language, "confirm")),
            Style::default().fg(theme.text_dim),
        ),
        Span::styled("[Esc] ", Style::default().fg(theme.primary)),
        Span::styled(
            i18n::text(language, "cancel"),
            Style::default().fg(theme.text_dim),
        ),
    ]))
    .style(Style::default().bg(theme.background));
    frame.render_widget(footer, chunks[4]);
}
