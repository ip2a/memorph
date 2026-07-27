use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};

use memorph::providers;
use crate::tui::app::{App, AppResult, Screen};
use crate::tui::theme::Theme;

/// Draw migration wizard page
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(70, 60, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(format!(" Switch Session - Step {}/3 ", app.switch_step + 1))
        .borders(Borders::ALL)
        .border_style(theme.border_focused);

    frame.render_widget(&block, popup_area);

    let inner = popup_area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });

    match app.switch_step {
        0 => draw_step_select_target(frame, app, inner, theme),
        1 => draw_step_confirm(frame, app, inner, theme),
        2 => draw_step_result(frame, app, inner, theme),
        _ => {}
    }
}

fn draw_step_select_target(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let selected_provider = app
        .selected_session
        .as_ref()
        .map(|s| s.provider_id.as_str())
        .unwrap_or("");

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(area);

    let info = Paragraph::new(format!(
        "Source: {} ({})\nSelect target provider:",
        selected_provider.to_uppercase(),
        app.selected_session
            .as_ref()
            .and_then(|s| s.title.as_deref())
            .unwrap_or("(untitled)")
    ))
    .wrap(Wrap { trim: true });

    frame.render_widget(info, chunks[0]);

    let items: Vec<ListItem> = providers::all_provider_ids()
        .iter()
        .filter(|p| **p != selected_provider)
        .enumerate()
        .map(|(i, &name)| {
            let style = if Some(i) == app.switch_selection {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            ListItem::new(Line::from(Span::styled(
                format!(
                    "  {} {}",
                    if Some(i) == app.switch_selection {
                        "▸"
                    } else {
                        " "
                    },
                    name.to_uppercase()
                ),
                style,
            )))
        })
        .collect();

    let list = List::new(items).block(Block::default().borders(Borders::NONE));
    frame.render_widget(list, chunks[1]);

    let hints = Paragraph::new("↑↓:Select  Enter:Confirm  Esc:Cancel")
        .style(Style::default().fg(theme.text_dim));
    frame.render_widget(hints, chunks[2]);
}

fn draw_step_confirm(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let source = app.selected_session.as_ref().unwrap();
    let target_provider = app.switch_target.as_deref().unwrap_or("?");

    let text = Text::from(vec![
        Line::from(Span::styled(
            "Confirm Switch",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("From: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                source.provider_id.to_uppercase(),
                Style::default().fg(theme.provider_color(&source.provider_id)),
            ),
        ]),
        Line::from(vec![
            Span::styled("Session: ", Style::default().fg(theme.text_dim)),
            Span::raw(source.title.as_deref().unwrap_or("(untitled)")),
        ]),
        Line::from(vec![
            Span::styled("ID: ", Style::default().fg(theme.text_dim)),
            Span::raw(&source.session_id),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("To: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                target_provider.to_uppercase(),
                Style::default().fg(theme.provider_color(target_provider)),
            ),
        ]),
        Line::from(vec![
            Span::styled("Target Dir: ", Style::default().fg(theme.text_dim)),
            Span::raw(app.workspace.as_deref().unwrap_or(".")),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "⚠ External sessions may not be recognized by the target tool.",
            Style::default().fg(theme.warning),
        )),
        Line::from(""),
        Line::from("Confirm?"),
    ]);

    let confirm = Paragraph::new(text).wrap(Wrap { trim: true });
    frame.render_widget(confirm, area);
}

fn draw_step_result(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let text = if let Some(ref result) = app.switch_result {
        Text::from(vec![
            Line::from(Span::styled(
                "✓ Switch Complete",
                Style::default()
                    .fg(theme.success)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled("From: ", Style::default().fg(theme.text_dim)),
                Span::raw(&result.from_name),
            ]),
            Line::from(vec![
                Span::styled("To: ", Style::default().fg(theme.text_dim)),
                Span::raw(&result.to_name),
            ]),
            Line::from(vec![
                Span::styled("New Session ID: ", Style::default().fg(theme.text_dim)),
                Span::raw(&result.target_session_id),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("Resume: ", Style::default().fg(theme.text_dim)),
                Span::raw(result.resume_command.as_deref().unwrap_or("N/A")),
            ]),
        ])
    } else if let Some(ref err) = app.switch_error {
        Text::from(vec![
            Line::from(Span::styled(
                "✗ Switch Failed",
                Style::default()
                    .fg(theme.error)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(err, Style::default().fg(theme.error))),
        ])
    } else {
        Text::from(vec![Line::from("Processing...")])
    };

    let result_widget = Paragraph::new(text).wrap(Wrap { trim: true });
    frame.render_widget(result_widget, area);
}

/// Handle migration wizard key events
pub fn handle_key(app: &mut App, key: KeyEvent) -> AppResult {
    match app.switch_step {
        0 => handle_step0(app, key),
        1 => handle_step1(app, key),
        2 => handle_step2(app, key),
        _ => AppResult::Continue,
    }
}

fn handle_step0(app: &mut App, key: KeyEvent) -> AppResult {
    let available_targets: Vec<usize> = providers::all_provider_ids()
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            app.selected_session
                .as_ref()
                .map(|s| s.provider_id.as_str() != **p)
                .unwrap_or(true)
        })
        .map(|(i, _)| i)
        .collect();

    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            if let Some(current) = app.switch_selection {
                let pos = available_targets
                    .iter()
                    .position(|x| *x == current)
                    .unwrap_or(0);
                if pos > 0 {
                    app.switch_selection = Some(available_targets[pos - 1]);
                }
            }
            AppResult::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let Some(current) = app.switch_selection {
                let pos = available_targets
                    .iter()
                    .position(|x| *x == current)
                    .unwrap_or(0);
                if pos + 1 < available_targets.len() {
                    app.switch_selection = Some(available_targets[pos + 1]);
                }
            } else if !available_targets.is_empty() {
                app.switch_selection = Some(available_targets[0]);
            }
            AppResult::Continue
        }
        KeyCode::Enter => {
            if let Some(selected) = app.switch_selection {
                app.switch_target = Some(providers::all_provider_ids()[selected].to_string());
                app.switch_step = 1;
            }
            AppResult::Continue
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.current_screen = Screen::SessionList;
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn handle_step1(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Enter => {
            app.execute_switch();
            app.switch_step = 2;
            AppResult::Continue
        }
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('n') => {
            app.current_screen = Screen::SessionList;
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn handle_step2(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
            app.current_screen = Screen::SessionList;
            app.switch_result = None;
            app.switch_error = None;
            app.switch_target = None;
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
