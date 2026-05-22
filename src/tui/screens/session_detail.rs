use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::canonical::{EventBlock, EventRole};
use crate::tui::app::{App, AppResult};
use crate::tui::theme::{self, Theme};

/// Draw session detail page
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    let selected = match &app.selected_session {
        Some(s) => s,
        None => {
            let msg = Paragraph::new("No session selected. Press Esc to go back.");
            frame.render_widget(msg, area);
            return;
        }
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);

    draw_header(frame, selected, chunks[0], theme);
    draw_messages(frame, app, chunks[1], theme);
}

fn draw_header(frame: &mut Frame, selected: &crate::core::SessionItem, area: Rect, theme: &Theme) {
    let title = selected.title.as_deref().unwrap_or("(untitled)");
    let provider_color = theme.provider_color(&selected.provider_id);

    let header_text = Text::from(vec![
        Line::from(vec![
            Span::styled(
                selected.provider_id.to_uppercase(),
                Style::default()
                    .fg(provider_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                title,
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("ID: ", Style::default().fg(theme.text_dim)),
            Span::raw(&selected.session_id),
        ]),
        Line::from(vec![
            Span::styled("Dir: ", Style::default().fg(theme.text_dim)),
            Span::raw(selected.project_dir.as_deref().unwrap_or("-")),
        ]),
    ]);

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.border),
    );

    frame.render_widget(header, area);
}

fn draw_messages(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    let session = match &app.loaded_session {
        Some(s) => s,
        None => {
            let msg = Paragraph::new("Loading session...");
            frame.render_widget(msg, area);
            return;
        }
    };

    let mut lines: Vec<Line> = Vec::new();

    for event in &session.events {
        let role_color = match event.role {
            EventRole::User => theme.accent,
            EventRole::Assistant => theme.primary,
            EventRole::Tool => theme.secondary,
            EventRole::System => theme.text_dim,
            EventRole::Developer => theme.text_dim,
            EventRole::Unknown => theme.warning,
        };

        let role_name = serde_json::to_string(&event.role)
            .unwrap_or_else(|_| "\"unknown\"".to_string())
            .trim_matches('"')
            .to_string();
        let kind_name = serde_json::to_string(&event.kind)
            .unwrap_or_else(|_| "\"unknown\"".to_string())
            .trim_matches('"')
            .to_string();
        let time_str = event.timestamp.format("%H:%M:%S").to_string();

        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", role_name.to_uppercase()),
                Style::default().fg(role_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {} ", kind_name),
                Style::default().fg(theme.text_dim),
            ),
            Span::styled(
                format!(" {} ", time_str),
                Style::default().fg(theme.text_dim),
            ),
        ]));
        lines.push(Line::from(Span::styled(
            "─".repeat(40),
            Style::default().fg(theme.border),
        )));

        for block in &event.blocks {
            match block {
                EventBlock::Text { text } => {
                    for line in text.lines() {
                        lines.push(Line::from(Span::raw(line.to_string())));
                    }
                }
                EventBlock::Thinking { text, .. } => {
                    lines.push(Line::from(vec![
                        Span::styled("[thinking] ", Style::default().fg(theme.text_dim)),
                        Span::styled(
                            theme::truncate(text, 80),
                            Style::default().fg(theme.text_dim),
                        ),
                    ]));
                }
                EventBlock::ToolCall { name, input, .. } => {
                    lines.push(Line::from(vec![
                        Span::styled("Tool: ", Style::default().fg(theme.secondary)),
                        Span::styled(name, Style::default().fg(theme.text)),
                    ]));
                    if let Some(input) = input {
                        let input_str = serde_json::to_string(input).unwrap_or_default();
                        lines.push(Line::from(vec![Span::styled(
                            theme::truncate(&input_str, 60),
                            Style::default().fg(theme.text_dim),
                        )]));
                    }
                }
                EventBlock::ToolResult {
                    content, is_error, ..
                } => {
                    let label = if *is_error { "Error: " } else { "Result: " };
                    let color = if *is_error {
                        theme.error
                    } else {
                        theme.success
                    };
                    lines.push(Line::from(vec![
                        Span::styled(label, Style::default().fg(color)),
                        Span::styled(
                            theme::truncate(content, 60),
                            Style::default().fg(theme.text_dim),
                        ),
                    ]));
                }
                EventBlock::Patch {
                    files, diff_text, ..
                } => {
                    let preview = if !files.is_empty() {
                        format!("Patch: {}", files.join(", "))
                    } else {
                        format!(
                            "Patch: {}",
                            theme::truncate(diff_text.as_deref().unwrap_or("(no diff)"), 72)
                        )
                    };
                    lines.push(Line::from(Span::styled(
                        preview,
                        Style::default().fg(theme.secondary),
                    )));
                }
                EventBlock::Command { command, .. } => {
                    lines.push(Line::from(vec![
                        Span::styled("Command: ", Style::default().fg(theme.secondary)),
                        Span::raw(command),
                    ]));
                }
                EventBlock::CommandResult {
                    exit_code, stdout, ..
                } => {
                    let preview = stdout.as_deref().unwrap_or("(no output)");
                    lines.push(Line::from(vec![
                        Span::styled(
                            format!("Exit {}: ", exit_code.unwrap_or_default()),
                            Style::default().fg(theme.success),
                        ),
                        Span::styled(
                            theme::truncate(preview, 72),
                            Style::default().fg(theme.text_dim),
                        ),
                    ]));
                }
                EventBlock::File { path, .. } => {
                    lines.push(Line::from(vec![
                        Span::styled("File: ", Style::default().fg(theme.secondary)),
                        Span::raw(path),
                    ]));
                }
                EventBlock::Image { .. } => {
                    lines.push(Line::from(Span::styled(
                        "[Image]",
                        Style::default().fg(theme.text_dim),
                    )));
                }
                EventBlock::ProviderPayload { kind, .. } => {
                    lines.push(Line::from(vec![
                        Span::styled("Payload: ", Style::default().fg(theme.text_dim)),
                        Span::raw(kind),
                    ]));
                }
                EventBlock::Unknown { .. } => {
                    lines.push(Line::from(Span::styled(
                        "[Unknown payload]",
                        Style::default().fg(theme.warning),
                    )));
                }
            }
        }
    }

    if lines.is_empty() {
        lines.push(Line::from("No messages in this session."));
    }

    let messages_widget = Paragraph::new(Text::from(lines))
        .block(Block::default().borders(Borders::NONE))
        .wrap(Wrap { trim: true });

    frame.render_widget(messages_widget, area);
}

/// Handle session detail page key events
pub fn handle_key(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.detail_scroll = app.detail_scroll.saturating_sub(3);
            AppResult::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.detail_scroll = app.detail_scroll.saturating_add(3);
            AppResult::Continue
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.current_screen = crate::tui::app::Screen::SessionList;
            AppResult::Continue
        }
        KeyCode::Char('s') => {
            app.current_screen = crate::tui::app::Screen::SessionList;
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}
