use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, HighlightSpacing, List, ListItem},
    Frame,
};

use crate::tui::app::{App, AppResult, Screen};
use crate::tui::theme::{self, Theme};

/// 绘制会话列表页面
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    draw_provider_tabs(frame, app, chunks[0], theme);
    draw_session_list(frame, app, chunks[1], theme);
}

fn draw_provider_tabs(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let providers = vec!["All", "claude", "codex", "OpenCode", "Cursor", "Kiro"];
    let tabs: Vec<Line> = providers
        .iter()
        .enumerate()
        .map(|(i, &name)| {
            let style = if i == app.selected_provider_tab {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_dim)
            };
            Line::from(Span::styled(format!(" {} ", name), style))
        })
        .collect();

    let tabs_widget = ratatui::widgets::Tabs::new(tabs)
        .select(app.selected_provider_tab)
        .block(
            Block::default()
                .borders(Borders::BOTTOM)
                .border_style(theme.border),
        )
        .highlight_style(Style::default().add_modifier(Modifier::UNDERLINED));

    frame.render_widget(tabs_widget, area);
}

fn draw_session_list(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    let total_items: usize = app.session_groups.iter().map(|g| g.sessions.len()).sum();

    if app.list_state.selected().is_none() && total_items > 0 {
        app.list_state.select(Some(0));
    }

    let items = build_list_items(&app.session_groups, theme);

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default().bg(theme.background)),
        )
        .highlight_style(
            Style::default()
                .bg(theme.highlight)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(list, area, &mut app.list_state);
}

fn build_list_items<'a>(
    groups: &'a [crate::core::SessionGroup],
    theme: &'a Theme,
) -> Vec<ListItem<'a>> {
    let mut items = Vec::new();

    for group in groups {
        let provider_style = Style::default()
            .fg(theme.provider_color(&group.provider_id))
            .add_modifier(Modifier::BOLD);

        items.push(ListItem::new(Line::from(vec![
            Span::raw("▶ "),
            Span::styled(&group.provider_name, provider_style),
            Span::styled(
                format!(" ({} sessions)", group.sessions.len()),
                Style::default().fg(theme.text_dim),
            ),
        ])));

        for session in &group.sessions {
            let title = session.title.as_deref().unwrap_or("(untitled)");
            let dir = session.project_dir.as_deref().unwrap_or("(no dir)");
            let time_str = theme::format_relative_time(session.last_active_at);

            items.push(ListItem::new(Line::from(vec![
                Span::raw("    "),
                Span::styled(
                    theme::truncate(session.session_id.as_str(), 12),
                    Style::default().fg(theme.text_dim),
                ),
                Span::raw(" │ "),
                Span::styled(theme::truncate(title, 30), Style::default().fg(theme.text)),
                Span::raw(" │ "),
                Span::styled(
                    theme::truncate(dir, 25),
                    Style::default().fg(theme.text_dim),
                ),
                Span::raw(" │ "),
                Span::styled(time_str, Style::default().fg(theme.text_dim)),
            ])));
        }

        items.push(ListItem::new(Line::from("")));
    }

    if items.is_empty() {
        items.push(ListItem::new(Line::from(vec![Span::styled(
            "No sessions found.",
            Style::default().fg(theme.text_dim),
        )])));
        items.push(ListItem::new(Line::from("")));
        items.push(ListItem::new(Line::from(vec![Span::styled(
            "Press 'a' to show all workspaces, or check your workspace path.",
            Style::default().fg(theme.text_dim),
        )])));
    }

    items
}

/// 处理会话列表页面的按键事件
pub fn handle_key(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.select_previous();
            AppResult::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.select_next();
            AppResult::Continue
        }
        KeyCode::Enter => {
            if let Some(selected) = app.get_selected_session() {
                app.selected_session = Some(selected.clone());
                app.current_screen = Screen::SessionDetail;
                app.detail_scroll = 0;
            }
            AppResult::Continue
        }
        KeyCode::Char('s') => {
            if app.get_selected_session().is_some() {
                app.current_screen = Screen::SwitchFlow;
                app.switch_step = 0;
            }
            AppResult::Continue
        }
        KeyCode::Char('e') => {
            if let Some(selected) = app.get_selected_session() {
                app.show_error(format!(
                    "Export: {} - not yet implemented in TUI",
                    selected.session_id
                ));
            }
            AppResult::Continue
        }
        KeyCode::Char('d') => {
            if let Some(selected) = app.get_selected_session() {
                app.show_error(format!(
                    "Delete: {} - not yet implemented in TUI",
                    selected.session_id
                ));
            }
            AppResult::Continue
        }
        KeyCode::Char('r') => {
            if let Some(selected) = app.get_selected_session() {
                app.show_error(format!(
                    "Rename: {} - not yet implemented in TUI",
                    selected.session_id
                ));
            }
            AppResult::Continue
        }
        KeyCode::Char('/') => {
            app.show_error("Search - not yet implemented in TUI".to_string());
            AppResult::Continue
        }
        KeyCode::Char('a') => {
            app.toggle_show_all();
            AppResult::Continue
        }
        KeyCode::Char('\t') => {
            app.next_provider_tab();
            AppResult::Continue
        }
        KeyCode::Char('1') => {
            app.select_provider_tab(0);
            AppResult::Continue
        }
        KeyCode::Char('2') => {
            app.select_provider_tab(1);
            AppResult::Continue
        }
        KeyCode::Char('3') => {
            app.select_provider_tab(2);
            AppResult::Continue
        }
        KeyCode::Char('4') => {
            app.select_provider_tab(3);
            AppResult::Continue
        }
        KeyCode::Char('5') => {
            app.select_provider_tab(4);
            AppResult::Continue
        }
        KeyCode::Char('6') => {
            app.select_provider_tab(5);
            AppResult::Continue
        }
        KeyCode::Char('q') | KeyCode::Esc => AppResult::Quit,
        _ => AppResult::Continue,
    }
}
