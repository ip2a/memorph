use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::app::{App, Screen};
use super::theme::Theme;

/// 主渲染入口
pub fn draw(frame: &mut Frame, app: &mut App) {
    let theme = Theme::default();
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, app, chunks[0], &theme);
    draw_main(frame, app, chunks[1], &theme);
    draw_footer(frame, app, chunks[2], &theme);

    if app.show_help {
        draw_help_overlay(frame, app, &theme);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let title = format!("memorph v{}", env!("CARGO_PKG_VERSION"));
    let workspace = app.workspace.as_deref().unwrap_or("(no workspace)");

    let header_text = Text::from(vec![
        Line::from(vec![
            Span::styled(
                title,
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(workspace, Style::default().fg(theme.text_dim)),
        ]),
        Line::from(if app.error_message.is_some() {
            Span::styled(
                app.error_message.as_deref().unwrap_or(""),
                Style::default().fg(theme.error),
            )
        } else {
            Span::styled(
                "TUI Mode - Interactive Session Manager",
                Style::default().fg(theme.text_dim),
            )
        }),
    ]);

    let header = Paragraph::new(header_text).block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(theme.border),
    );

    frame.render_widget(header, area);
}

fn draw_main(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    match app.current_screen {
        Screen::SessionList => {
            super::screens::session_list::draw(frame, app, area, theme);
        }
        Screen::SessionDetail => {
            super::screens::session_detail::draw(frame, app, area, theme);
        }
        Screen::SwitchFlow => {
            super::screens::switch_flow::draw(frame, app, area, theme);
        }
    }
}

fn draw_footer(frame: &mut Frame, _app: &App, area: Rect, theme: &Theme) {
    let hints = "q:Quit  ↑↓:Navigate  Enter:Select  s:Switch  e:Export  d:Delete  r:Rename  /:Search  ?:Help";
    let footer = Paragraph::new(hints)
        .style(Style::default().fg(theme.text_dim))
        .alignment(Alignment::Center);

    frame.render_widget(footer, area);
}

fn draw_help_overlay(frame: &mut Frame, _app: &App, theme: &Theme) {
    let area = frame.area();
    let popup_area = centered_rect(60, 70, area);

    frame.render_widget(Clear, popup_area);

    let help_text = Text::from(vec![
        Line::from(Span::styled(
            "Keyboard Shortcuts",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Navigation",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  ↑ / k     Move up"),
        Line::from("  ↓ / j     Move down"),
        Line::from("  Enter     Select / Confirm"),
        Line::from("  Esc / q   Back / Cancel"),
        Line::from("  Tab       Switch panel / focus"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Actions",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  s         Switch session to another provider"),
        Line::from("  e         Export session to file"),
        Line::from("  d         Delete session"),
        Line::from("  r         Rename session"),
        Line::from("  /         Search / filter sessions"),
        Line::from("  i         Import session"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "General",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  ? / h     Show this help"),
        Line::from("  Ctrl+C    Quit application"),
    ]);

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(theme.border_focused),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(help, popup_area);
}

/// 计算居中矩形
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
