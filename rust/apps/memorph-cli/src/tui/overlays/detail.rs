use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use memorph::core::SessionDetailView;
use memorph::session::{Block as EventBlock, Event, Role};
use memorph::{config::UiLanguage, i18n};

use super::Theme;

/// Detail expand state — replaces the main session list area.
#[derive(Debug, Clone)]
pub struct DetailState {
    pub scroll: usize,
    pub provider_id: String,
    pub session_id: String,
    pub detail: SessionDetailView,
}

impl DetailState {
    pub fn new(provider_id: String, session_id: String, detail: SessionDetailView) -> Self {
        Self {
            scroll: 0,
            provider_id,
            session_id,
            detail,
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> DetailAction {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => DetailAction::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_sub(1);
                DetailAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.detail.events.len().saturating_sub(1);
                self.scroll = (self.scroll + 1).min(max);
                DetailAction::Continue
            }
            KeyCode::Home | KeyCode::Char('g') => {
                self.scroll = 0;
                DetailAction::Continue
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.scroll = self.detail.events.len().saturating_sub(1);
                DetailAction::Continue
            }
            _ => DetailAction::Continue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailAction {
    Continue,
    Close,
}

/// Draw the detail view (takes over the main area, not a popup).
pub fn draw(
    frame: &mut Frame,
    state: &DetailState,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    // Header: session metadata
    draw_detail_header(frame, state, language, chunks[0], theme);

    // Events
    draw_events(frame, state, language, chunks[1], theme);

    // Footer: navigation hints
    let footer = Paragraph::new(Line::from(vec![
        Span::styled("[↑/↓] ", Style::default().fg(theme.primary)),
        Span::styled(
            format!("{}  ", i18n::text(language, "scroll")),
            Style::default().fg(theme.text_dim),
        ),
        Span::styled("[g/G] ", Style::default().fg(theme.primary)),
        Span::styled(
            format!("{}  ", i18n::text(language, "topBottom")),
            Style::default().fg(theme.text_dim),
        ),
        Span::styled("[Esc] ", Style::default().fg(theme.primary)),
        Span::styled(
            i18n::text(language, "back"),
            Style::default().fg(theme.text_dim),
        ),
    ]))
    .style(Style::default().bg(theme.background));
    frame.render_widget(footer, chunks[2]);
}

fn draw_detail_header(
    frame: &mut Frame,
    state: &DetailState,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let detail = &state.detail;
    let title = detail
        .title
        .as_deref()
        .unwrap_or(i18n::text(language, "untitled"));
    let info = format!(
        "{}  │  {}  │  {} {}",
        title,
        state.session_id,
        detail.events.len(),
        i18n::text(language, "events")
    );
    let header = Paragraph::new(Line::from(Span::styled(
        info,
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::BOLD),
    )))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(theme.border)),
    )
    .style(Style::default().bg(theme.background));
    frame.render_widget(header, area);
}

fn draw_events(
    frame: &mut Frame,
    state: &DetailState,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let visible_height = area.height as usize;
    let events = &state.detail.events;

    let lines: Vec<Line> = events
        .iter()
        .skip(state.scroll)
        .take(visible_height)
        .map(|event| format_event_line(event, language, theme))
        .collect();

    let para = Paragraph::new(lines)
        .style(Style::default().bg(theme.background))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, area);
}

fn format_event_line<'a>(event: &Event, language: UiLanguage, theme: &Theme) -> Line<'a> {
    let role_str = match event.role {
        Role::User => "user",
        Role::Assistant => "asst",
        Role::System => "sys ",
        Role::Tool => "tool",
        Role::Developer => "dev ",
        Role::Other => "other",
    };
    let role_color = match event.role {
        Role::User => theme.primary,
        Role::Assistant => theme.success,
        Role::System | Role::Developer | Role::Other => theme.text_dim,
        Role::Tool => theme.warning,
    };

    let content_preview = event
        .blocks
        .first()
        .map(|block| match block {
            EventBlock::Text { text } => {
                let preview: String = text.chars().take(120).collect();
                if text.len() > 120 {
                    format!("{}...", preview)
                } else {
                    preview
                }
            }
            EventBlock::ToolCall { name, .. } => {
                format!("[{}: {}]", i18n::text(language, "toolCall"), name)
            }
            EventBlock::ToolResult { content, .. } => {
                let preview: String = content.chars().take(100).collect();
                format!("[{}: {}]", i18n::text(language, "toolResultLabel"), preview)
            }
            _ => "[...]".to_string(),
        })
        .unwrap_or_else(|| i18n::text(language, "empty").to_string());

    Line::from(vec![
        Span::styled(format!("{} │ ", role_str), Style::default().fg(role_color)),
        Span::styled(content_preview, Style::default().fg(theme.text)),
    ])
}
