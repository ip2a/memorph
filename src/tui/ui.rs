use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::app::{provider_label, provider_tabs, App, MainFocus, SettingsField, SETTINGS_FIELDS};
use super::theme::{self, Theme};
use crate::config;
use crate::web_assets::MEMORPH_ASCII;

/// Main rendering entry point
pub fn draw(frame: &mut Frame, app: &mut App) {
    let theme = Theme::default();
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().fg(theme.text).bg(theme.background)),
        area,
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),
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
    if app.workspace_modal_open {
        draw_workspace_modal(frame, app, &theme);
    } else if app.settings_modal_open {
        draw_settings_modal(frame, app, &theme);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let workspace = app.workspace.as_deref().unwrap_or("(no workspace)");
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(5),
            Constraint::Length(3),
        ])
        .split(area);

    let rule = Paragraph::new("─".repeat(area.width as usize))
        .style(Style::default().fg(theme.border).bg(theme.background));
    frame.render_widget(rule, chunks[0]);

    let ascii = Paragraph::new(MEMORPH_ASCII)
        .style(
            Style::default()
                .fg(theme.primary)
                .bg(theme.background)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center);
    frame.render_widget(ascii, chunks[2]);

    let controls = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(24), Constraint::Length(16)])
        .split(chunks[3]);

    let workspace_block = Paragraph::new(theme::truncate(
        workspace,
        (controls[0].width.saturating_sub(4) as usize).max(8),
    ))
    .style(Style::default().fg(theme.text).bg(theme.background))
    .alignment(Alignment::Left)
    .block(top_block(
        "Workspace",
        app.main_focus == MainFocus::Workspace,
        theme,
    ));
    frame.render_widget(workspace_block, controls[0]);

    let settings_block = Paragraph::new(" Settings ")
        .style(Style::default().fg(theme.text).bg(theme.background))
        .alignment(Alignment::Center)
        .block(top_block(
            "Settings",
            app.main_focus == MainFocus::Settings,
            theme,
        ));
    frame.render_widget(settings_block, controls[1]);
}

fn draw_main(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    super::screens::session_list::draw(frame, app, area, theme);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let hints = if let Some(message) = app.error_message.as_deref() {
        message
    } else if app.action_modal_open {
        if app.action_dialog.is_some() {
            "↑↓ Select  Enter Save  Esc Cancel"
        } else {
            "↑↓ Focus  ←→ Action  Enter Open/Run  Esc Close"
        }
    } else if app.workspace_modal_open {
        "Type Workspace  ↑↓ Suggestions  Enter Save  Esc Close"
    } else if app.settings_modal_open {
        "Esc Close"
    } else if app.search_modal_open {
        "Type Query  ←→ Scope  ↑↓ Results  Enter Jump  Esc Close"
    } else if app.main_focus == MainFocus::Workspace {
        "←→ Top Control  Enter Edit Workspace  ↓ Sessions  Esc Sessions"
    } else if app.main_focus == MainFocus::Settings {
        "←→ Top Control  Enter Settings  ↓ Sessions  Esc Sessions"
    } else {
        "←→ Provider  ↑↓ Session  Enter Actions  F Search  Q Quit"
    };
    let footer = Paragraph::new(hints)
        .style(
            Style::default()
                .fg(if app.error_message.is_some() {
                    theme.error
                } else {
                    theme.text_dim
                })
                .bg(theme.background),
        )
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
        Line::from("  ↑ / ↓     Select session, field, or popup option"),
        Line::from("  ← / →     Switch provider, action, or search scope"),
        Line::from("  Enter     Open actions, open pickers, or run action"),
        Line::from("  F         Open search"),
        Line::from("  Q         Quit from the main table"),
        Line::from("  Esc       Close modal or search"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "Actions",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  Switch    Create a session in another agent"),
        Line::from("  Export    Write a JSON export file"),
        Line::from("  Rename    Rename the provider session"),
        Line::from("  Delete    Delete the provider session"),
        Line::from("  Details   Open a dedicated session detail popup"),
        Line::from(""),
        Line::from(vec![Span::styled(
            "General",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from("  ?         Show this help"),
    ]);

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .style(Style::default().fg(theme.text).bg(theme.surface))
                .border_style(theme.border_focused),
        )
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });

    frame.render_widget(help, popup_area);
}

fn draw_workspace_modal(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let popup_area = centered_rect(80, 68, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block("Workspace", theme), popup_area);

    let inner = popup_area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(inner);

    let input = Paragraph::new(Text::from(vec![
        Line::from(Span::styled(
            format!(" {} ", app.workspace_input),
            Style::default()
                .fg(theme.primary)
                .bg(theme.highlight)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )),
        Line::from(Span::styled(
            "Type a local workspace path.",
            Style::default().fg(theme.text),
        )),
    ]))
    .block(section_block("Workspace Path", true, theme))
    .style(Style::default().fg(theme.text).bg(theme.surface))
    .wrap(Wrap { trim: true });
    frame.render_widget(input, chunks[0]);

    let options = app.filtered_main_workspace_options();
    let selected = app
        .workspace_modal_index
        .min(options.len().saturating_sub(1));
    let mut lines = Vec::new();
    if options.is_empty() {
        lines.push(Line::from(Span::styled(
            "No saved workspace matches this path.",
            Style::default().fg(theme.warning),
        )));
    } else {
        for (index, workspace) in options.iter().take(6).enumerate() {
            if index > 0 {
                lines.push(Line::from(""));
            }
            let style = if index == selected {
                Style::default()
                    .fg(theme.primary)
                    .bg(theme.highlight)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                Style::default().fg(theme.text).bg(theme.surface)
            };
            lines.push(Line::from(Span::styled(format!(" {} ", workspace), style)));
        }
    }
    let suggestions = Paragraph::new(Text::from(lines))
        .block(section_block("Saved Workspaces", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(suggestions, chunks[1]);

    let footer = Paragraph::new("Type path  ↑↓ Suggestions  Enter Save  Esc Cancel")
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[2]);
}

fn draw_settings_modal(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let popup_area = centered_rect(78, 76, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block("Settings", theme), popup_area);

    let inner = popup_area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(5),
            Constraint::Length(2),
        ])
        .split(inner);

    let config_path = config::config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|e| e.to_string());

    let mut rows = Vec::new();
    for field in SETTINGS_FIELDS {
        rows.push(settings_row(field, app, theme));
        rows.push(Line::from(""));
    }
    if rows.last().map(|line| line.width()).unwrap_or(0) == 0 {
        rows.pop();
    }

    let settings = Paragraph::new(Text::from(rows))
        .block(section_block("Editable Settings", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(settings, chunks[0]);

    let provider = provider_tabs()
        .get(app.selected_provider_tab)
        .cloned()
        .unwrap_or_else(|| "All".to_string());
    let info = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled("Version", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("  {}", env!("CARGO_PKG_VERSION"))),
        ]),
        Line::from(vec![
            Span::styled(
                "Provider Filter",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}", provider)),
        ]),
        Line::from(vec![
            Span::styled("Config", Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!("  {}", config_path)),
        ]),
    ]))
    .block(section_block("Config", false, theme))
    .style(Style::default().fg(theme.text).bg(theme.surface))
    .wrap(Wrap { trim: true });
    frame.render_widget(info, chunks[1]);

    let footer = Paragraph::new(
        "↑↓ Select  ←→ Change/Choose Agent  Digits Edit Count  Enter Toggle/Save  Esc Close",
    )
    .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[2]);
}

fn settings_row(field: SettingsField, app: &App, theme: &Theme) -> Line<'static> {
    let selected = app.selected_settings_field() == field;
    let label_style = if selected {
        highlighted_value_style(theme)
    } else {
        Style::default()
            .fg(theme.text)
            .bg(theme.surface)
            .add_modifier(Modifier::BOLD)
    };
    let value_style = if selected {
        highlighted_value_style(theme)
    } else {
        Style::default().fg(theme.text).bg(theme.surface)
    };
    let value = match field {
        SettingsField::Language => match app.settings_language {
            config::UiLanguage::Zh => "中文",
            config::UiLanguage::En => "English",
        }
        .to_string(),
        SettingsField::SessionsPerProvider => app.settings_sessions_per_provider.to_string(),
        SettingsField::ShowOpenCodeSubagents => enabled_label(app.settings_show_opencode_subagents),
        SettingsField::AutoRefreshAfterDelete => {
            enabled_label(app.settings_auto_refresh_after_delete)
        }
        SettingsField::PrimaryAgents => settings_agent_label(app),
        SettingsField::Save => "Write config".to_string(),
    };

    Line::from(vec![
        Span::styled(format!(" {} ", field.label()), label_style),
        Span::raw("  "),
        Span::styled(value, value_style),
    ])
}

fn settings_agent_label(app: &App) -> String {
    let Some(agent) = app.settings_agent_order.get(app.settings_agent_index) else {
        return "All visible".to_string();
    };
    let state = if app
        .settings_primary_agents
        .iter()
        .any(|provider| provider == agent)
    {
        "Visible"
    } else {
        "Folded"
    };
    format!("{}: {}", provider_label(agent), state)
}

fn enabled_label(enabled: bool) -> String {
    if enabled {
        "Enabled".to_string()
    } else {
        "Disabled".to_string()
    }
}

fn highlighted_value_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.primary)
        .bg(theme.highlight)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

fn top_block(title: &str, focused: bool, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .style(Style::default().fg(theme.text).bg(theme.background))
        .border_style(if focused {
            theme.border_focused
        } else {
            theme.border
        })
}

fn modal_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .border_style(theme.border_focused)
}

fn section_block(title: &str, focused: bool, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .border_style(if focused {
            theme.border_focused
        } else {
            theme.border
        })
}

/// Compute centered rectangle
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
