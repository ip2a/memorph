use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use super::super::theme::Theme;
use memorph::{config::UiLanguage, i18n};

/// Draw the compact 3-line header.
/// Line 1: title + workspace + session count
/// Line 2: provider tabs
/// Line 3: separator (built into border)
pub fn draw(
    frame: &mut Frame,
    workspace: Option<&str>,
    session_count: usize,
    provider_tabs: &[String],
    selected_tab: usize,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(2)])
        .split(area);

    // Line 1: title bar
    let ws_display = workspace.unwrap_or(i18n::text(language, "workspaceEmpty"));
    let title_line = Line::from(vec![
        Span::styled(
            " memorph ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("│ ", Style::default().fg(theme.border)),
        Span::styled(ws_display, Style::default().fg(theme.text)),
        Span::styled("│ ", Style::default().fg(theme.border)),
        Span::styled(
            i18n::format(
                language,
                "sessionGroupCount",
                &[("count", &session_count.to_string())],
            ),
            Style::default().fg(theme.text_dim),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(title_line).style(Style::default().bg(theme.background)),
        chunks[0],
    );

    // Line 2: provider tabs
    let mut tab_spans: Vec<Span> = Vec::new();
    for (i, tab) in provider_tabs.iter().enumerate() {
        if i > 0 {
            tab_spans.push(Span::styled("  ", Style::default().fg(theme.text_dim)));
        }
        let style = if i == selected_tab {
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD)
                .bg(theme.highlight)
        } else {
            Style::default().fg(theme.text).bg(theme.background)
        };
        tab_spans.push(Span::styled(format!(" {} ", tab), style));
    }

    let tabs_para = Paragraph::new(Line::from(tab_spans))
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(Style::default().fg(theme.border))
                .style(Style::default().bg(theme.background)),
        )
        .style(Style::default().bg(theme.background));
    frame.render_widget(tabs_para, chunks[1]);
}
