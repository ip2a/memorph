use ratatui::{
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Cell, HighlightSpacing, Row, Table, TableState},
    Frame,
};

use memorph::config::UiLanguage;
use memorph::core::SessionGroup;
use memorph::i18n;

use crate::tui::app::provider_label;
use crate::tui::theme::{self, Theme};

/// Draw the session table.
pub fn draw(
    frame: &mut Frame,
    groups: &[SessionGroup],
    table_state: &mut TableState,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let total_items: usize = groups.iter().map(|g| g.sessions.len()).sum();
    if total_items == 0 {
        table_state.select(None);
        let empty = ratatui::widgets::Paragraph::new(i18n::text(language, "cliNoSessionsFound"))
            .style(Style::default().fg(theme.text_dim).bg(theme.background))
            .alignment(ratatui::layout::Alignment::Center);
        frame.render_widget(empty, area);
        return;
    }

    if table_state.selected().is_none() {
        table_state.select(Some(0));
    }

    let rows = build_rows(groups, table_state.selected(), language, theme);
    let widths = [
        Constraint::Length(11),
        Constraint::Percentage(28),
        Constraint::Length(14),
        Constraint::Percentage(35),
        Constraint::Length(12),
    ];

    let header = Row::new(vec![
        Cell::from(i18n::text(language, "tableAgent")),
        Cell::from(i18n::text(language, "title")),
        Cell::from(i18n::text(language, "session")),
        Cell::from(i18n::text(language, "workspace")),
        Cell::from(i18n::text(language, "active")),
    ])
    .style(
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
    )
    .bottom_margin(1);

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::NONE)
                .style(Style::default().bg(theme.background)),
        )
        .style(Style::default().fg(theme.text).bg(theme.background))
        .highlight_spacing(HighlightSpacing::Never);

    frame.render_stateful_widget(table, area, table_state);
}

fn build_rows(
    groups: &[SessionGroup],
    selected_row: Option<usize>,
    language: UiLanguage,
    theme: &Theme,
) -> Vec<Row<'static>> {
    let mut rows = Vec::new();
    let mut row_index = 0;

    for group in groups {
        for session in &group.sessions {
            let title = session
                .title
                .as_deref()
                .unwrap_or(i18n::text(language, "untitled"));
            let dir = session
                .project_dir
                .as_deref()
                .unwrap_or(i18n::text(language, "noDir"));
            let time_str = theme::format_relative_time(session.last_active_at, language);
            let provider = provider_label(&session.provider_id);
            let selected = selected_row == Some(row_index);

            let value_style = if selected {
                Style::default()
                    .fg(theme.text)
                    .bg(theme.highlight)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text).bg(theme.background)
            };
            let muted_style = if selected {
                Style::default().fg(theme.text_dim).bg(theme.highlight)
            } else {
                Style::default().fg(theme.text_dim).bg(theme.background)
            };
            let provider_style = if selected {
                Style::default()
                    .fg(theme.provider_color(&group.provider_id))
                    .bg(theme.highlight)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
                    .fg(theme.provider_color(&group.provider_id))
                    .bg(theme.background)
            };

            rows.push(Row::new(vec![
                Cell::from(Span::styled(theme::truncate(provider, 10), provider_style)),
                Cell::from(Span::styled(theme::truncate(title, 42), value_style)),
                Cell::from(Span::styled(
                    theme::truncate(&session.session_id, 12),
                    muted_style,
                )),
                Cell::from(Span::styled(theme::truncate(dir, 56), muted_style)),
                Cell::from(Span::styled(time_str, muted_style)),
            ]));
            row_index += 1;
        }
    }

    rows
}
