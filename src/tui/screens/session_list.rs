use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Cell, Clear, HighlightSpacing, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::core::{SessionGroup, SessionItem};
use crate::model::{ContentBlock, MemorphMessage};
use crate::tui::app::{
    provider_label, provider_tabs, ActionDialog, ActionField, ActionResult, App, AppResult,
    MainFocus, SessionAction, ACTION_OPTIONS, SEARCH_SCOPE_OPTIONS,
};
use crate::tui::theme::{self, Theme};

/// Draw session table page
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    draw_provider_tabs(frame, app, chunks[0], theme);
    draw_session_table(frame, app, chunks[1], theme);

    if app.detail_modal_open {
        draw_detail_modal(frame, app, area, theme);
    } else if app.action_modal_open {
        draw_action_modal(frame, app, area, theme);
    } else if app.search_modal_open {
        draw_search_modal(frame, app, area, theme);
    }
}

fn draw_provider_tabs(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let tabs = provider_tabs();
    draw_chip_row(
        frame,
        "Providers",
        &tabs,
        app.selected_provider_tab,
        false,
        area,
        theme,
    );
}

fn draw_session_table(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    let total_items = app.session_count();
    if total_items == 0 {
        app.table_state.select(None);
        let empty = Paragraph::new("No sessions found in this workspace.")
            .style(Style::default().fg(theme.text_dim).bg(theme.background))
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::NONE));
        frame.render_widget(empty, area);
        return;
    }

    if app.table_state.selected().is_none() {
        app.table_state.select(Some(0));
    }

    let rows = build_table_rows(&app.session_groups, app.table_state.selected(), theme);
    let widths = [
        Constraint::Length(11),
        Constraint::Percentage(28),
        Constraint::Length(14),
        Constraint::Percentage(36),
        Constraint::Length(12),
    ];

    let header = Row::new(vec![
        Cell::from("AI"),
        Cell::from("Title"),
        Cell::from("Session"),
        Cell::from("Workspace"),
        Cell::from("Active"),
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
            Block::default()
                .borders(Borders::NONE)
                .style(Style::default().bg(theme.background)),
        )
        .style(Style::default().fg(theme.text).bg(theme.background))
        .highlight_spacing(HighlightSpacing::Never);

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn build_table_rows(
    groups: &[SessionGroup],
    selected_row: Option<usize>,
    theme: &Theme,
) -> Vec<Row<'static>> {
    let mut rows = Vec::new();
    let mut row_index = 0;

    for group in groups {
        for session in &group.sessions {
            let title = session.title.as_deref().unwrap_or("(untitled)");
            let dir = session.project_dir.as_deref().unwrap_or("(no dir)");
            let time_str = theme::format_relative_time(session.last_active_at);
            let provider = provider_label(&session.provider_id);
            let selected = selected_row == Some(row_index);
            let value_style = if selected {
                selected_row_style(theme)
            } else {
                Style::default().fg(theme.text).bg(theme.background)
            };
            let muted_style = if selected {
                selected_row_style(theme)
            } else {
                Style::default().fg(theme.text_dim).bg(theme.background)
            };
            let provider_style = if selected {
                selected_row_style(theme)
            } else {
                Style::default()
                    .fg(theme.provider_color(&group.provider_id))
                    .bg(theme.background)
            };

            rows.push(Row::new(vec![
                table_cell(provider.to_string(), provider_style, 10, theme),
                table_cell(theme::truncate(title, 42), value_style, 24, theme),
                table_cell(
                    theme::truncate(&session.session_id, 12),
                    muted_style,
                    12,
                    theme,
                ),
                table_cell(theme::truncate(dir, 56), muted_style, 32, theme),
                table_cell(time_str, muted_style, 10, theme),
            ]));
            row_index += 1;
        }
    }

    rows
}

fn table_cell(
    value: String,
    value_style: Style,
    rule_width: usize,
    theme: &Theme,
) -> Cell<'static> {
    Cell::from(Text::from(vec![
        Line::from(Span::styled(value, value_style)),
        Line::from(Span::styled(
            "─".repeat(rule_width),
            Style::default().fg(theme.border).bg(theme.background),
        )),
    ]))
}

fn draw_chip_row(
    frame: &mut Frame,
    title: &str,
    options: &[&str],
    selected: usize,
    focused: bool,
    area: Rect,
    theme: &Theme,
) {
    let mut spans = Vec::new();

    for (index, option) in options.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }

        let style = if index == selected {
            highlighted_value_style(theme)
        } else {
            Style::default().fg(theme.text).bg(theme.surface)
        };
        spans.push(Span::styled(format!(" {} ", option), style));
    }

    let row = Paragraph::new(Line::from(spans))
        .block(section_block(title, focused, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(row, area);
}

fn draw_action_modal(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(84, 82, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block("Session Actions", theme), popup_area);

    let inner = popup_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(inner);

    let selected = app
        .selected_session
        .as_ref()
        .or_else(|| app.get_selected_session());
    draw_session_summary(frame, selected, chunks[0], theme);
    draw_action_tabs(frame, app, chunks[1], theme);

    if let Some(result) = &app.action_result {
        draw_action_result(frame, result, chunks[2], theme);
    } else {
        draw_action_body(frame, app, chunks[2], theme);
    }

    let footer = Paragraph::new(action_footer_text(app))
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[3]);

    if let Some(dialog) = app.action_dialog {
        draw_action_dialog(frame, app, dialog, area, theme);
    }
}

fn draw_action_tabs(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let labels: Vec<&str> = ACTION_OPTIONS.iter().map(|action| action.label()).collect();
    draw_chip_row(
        frame,
        "Actions",
        &labels,
        app.action_selection,
        app.action_field == ActionField::Action,
        area,
        theme,
    );
}

fn draw_action_body(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    match app.current_action() {
        SessionAction::Switch => draw_switch_panel(frame, app, area, theme),
        SessionAction::Rename => draw_rename_panel(frame, app, area, theme),
        SessionAction::Delete => draw_delete_panel(frame, app, area, theme),
        SessionAction::Export => draw_export_panel(frame, app, area, theme),
        SessionAction::Details => draw_details_panel(frame, app, area, theme),
    }
}

fn draw_switch_panel(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Min(0),
        ])
        .split(area);

    draw_picker_block(
        frame,
        "Target Agent",
        app.selected_target_provider()
            .map(provider_label)
            .unwrap_or("-"),
        "Press Enter to choose the agent to switch into.",
        app.action_field == ActionField::TargetAgent,
        chunks[0],
        theme,
    );
    draw_picker_block(
        frame,
        "Workspace",
        app.selected_target_workspace()
            .as_deref()
            .unwrap_or("(no workspace)"),
        "Press Enter to choose or edit the target workspace.",
        app.action_field == ActionField::TargetWorkspace,
        chunks[1],
        theme,
    );
    draw_execute_block(
        frame,
        "Run Switch",
        app.action_field == ActionField::Execute,
        chunks[2],
        theme,
    );

    let note = Paragraph::new(
        "Memorph writes a new session for the selected agent in the chosen workspace.",
    )
    .block(section_block("What Happens", false, theme))
    .style(Style::default().fg(theme.text).bg(theme.surface))
    .wrap(Wrap { trim: true });
    frame.render_widget(note, chunks[3]);
}

fn draw_rename_panel(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Min(0),
        ])
        .split(area);

    draw_picker_block(
        frame,
        "New Title",
        if app.rename_input.is_empty() {
            "(empty)"
        } else {
            &app.rename_input
        },
        "Type directly in this field.",
        app.action_field == ActionField::RenameTitle,
        chunks[0],
        theme,
    );
    draw_execute_block(
        frame,
        "Run Rename",
        app.action_field == ActionField::Execute,
        chunks[1],
        theme,
    );

    let note = Paragraph::new("Type the new title, then move to Execute and press Enter.")
        .block(section_block("How To", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(note, chunks[2]);
}

fn draw_delete_panel(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(4)])
        .split(area);

    let warning = Paragraph::new(
        "Delete removes the provider session selected in the table. This action cannot be undone.",
    )
    .block(section_block("Warning", false, theme))
    .style(Style::default().fg(theme.warning).bg(theme.surface))
    .wrap(Wrap { trim: true });
    frame.render_widget(warning, chunks[0]);

    draw_execute_block(
        frame,
        "Run Delete",
        app.action_field == ActionField::Execute,
        chunks[1],
        theme,
    );
}

fn draw_export_panel(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Min(0),
        ])
        .split(area);

    draw_picker_block(
        frame,
        "Output Prefix",
        if app.export_output_prefix.is_empty() {
            "(empty)"
        } else {
            &app.export_output_prefix
        },
        "Type the full output path without the .json suffix.",
        app.action_field == ActionField::ExportPath,
        chunks[0],
        theme,
    );

    draw_execute_block(
        frame,
        "Run Export",
        app.action_field == ActionField::Execute,
        chunks[1],
        theme,
    );

    let info = Paragraph::new(
        "Export writes a JSON file to the selected local path and appends the .json suffix automatically.",
    )
    .block(section_block("Export", false, theme))
    .style(Style::default().fg(theme.text).bg(theme.surface))
    .wrap(Wrap { trim: true });
    frame.render_widget(info, chunks[2]);
}

fn draw_details_panel(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(4)])
        .split(area);

    let info = Paragraph::new(
        "Open a dedicated session detail popup with metadata and a scrollable message preview.",
    )
    .block(section_block("Details", false, theme))
    .style(Style::default().fg(theme.text).bg(theme.surface))
    .wrap(Wrap { trim: true });
    frame.render_widget(info, chunks[0]);

    draw_execute_block(
        frame,
        "Open Details",
        app.action_field == ActionField::Execute,
        chunks[1],
        theme,
    );
}

fn draw_search_modal(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(78, 62, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block("Search Sessions", theme), popup_area);

    let inner = popup_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Length(8),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner);

    draw_search_query(frame, app, chunks[0], theme);
    draw_search_scope_tabs(frame, app, chunks[1], theme);

    let matches = app.search_matches();
    draw_search_results(frame, app, &matches, chunks[2], theme);
    draw_search_preview(frame, app, &matches, chunks[3], theme);

    let footer = Paragraph::new("Type query  ←→ Scope  ↑↓ Results  Enter Jump  Esc Close")
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[4]);
}

fn draw_detail_modal(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(86, 84, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block("Session Details", theme), popup_area);

    let inner = popup_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(4),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .split(inner);

    let selected = app.selected_session.as_ref();
    draw_session_summary(frame, selected, chunks[0], theme);
    draw_detail_metadata(frame, app, chunks[1], theme);
    draw_detail_messages(frame, app, chunks[2], theme);

    let footer = Paragraph::new("↑↓ Scroll  Esc Close")
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[3]);
}

fn draw_action_dialog(
    frame: &mut Frame,
    app: &App,
    dialog: ActionDialog,
    area: Rect,
    theme: &Theme,
) {
    match dialog {
        ActionDialog::TargetAgent => draw_target_agent_dialog(frame, app, area, theme),
        ActionDialog::TargetWorkspace => draw_workspace_dialog(frame, app, area, theme),
    }
}

fn draw_target_agent_dialog(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(42, 58, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block("Target Agent", theme), popup_area);

    let inner = popup_area.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(5), Constraint::Length(2)])
        .split(inner);

    let providers = app.target_provider_options();
    let selected = app
        .switch_target_index
        .min(providers.len().saturating_sub(1));
    let mut lines = Vec::new();

    if providers.is_empty() {
        lines.push(Line::from(Span::styled(
            "No agent is available for switching.",
            Style::default().fg(theme.warning),
        )));
    } else {
        for (index, provider) in providers.iter().enumerate() {
            let style = if index == selected {
                highlighted_value_style(theme)
            } else {
                Style::default().fg(theme.text).bg(theme.surface)
            };
            lines.push(Line::from(Span::styled(
                format!(" {} ", provider_label(provider)),
                style,
            )));
        }
    }

    let body = Paragraph::new(Text::from(lines))
        .block(section_block("Agents", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(body, chunks[0]);

    let footer = Paragraph::new("↑↓ Select target  Enter Save  Esc Cancel")
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[1]);
}

fn draw_workspace_dialog(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(80, 68, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block("Workspace", theme), popup_area);

    let inner = popup_area.inner(Margin {
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
            format!(" {} ", app.target_workspace),
            highlighted_value_style(theme),
        )),
        Line::from(Span::styled(
            "Type to edit the path directly.",
            Style::default().fg(theme.text),
        )),
    ]))
    .block(section_block("Workspace Path", false, theme))
    .style(Style::default().fg(theme.text).bg(theme.surface))
    .wrap(Wrap { trim: true });
    frame.render_widget(input, chunks[0]);

    let matches = app.filtered_workspace_options();
    let selected = app
        .workspace_picker_index
        .min(matches.len().saturating_sub(1));
    let mut lines = Vec::new();

    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "No matching saved workspace. Enter saves the typed path.",
            Style::default().fg(theme.warning),
        )));
    } else {
        for (index, workspace) in matches.iter().take(6).enumerate() {
            if index > 0 {
                lines.push(Line::from(""));
            }
            let style = if index == selected {
                highlighted_value_style(theme)
            } else {
                Style::default().fg(theme.text).bg(theme.surface)
            };
            lines.push(Line::from(Span::styled(format!(" {} ", workspace), style)));
        }
    }

    let suggestions = Paragraph::new(Text::from(lines))
        .block(section_block("Suggestions", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(suggestions, chunks[1]);

    let footer = Paragraph::new("Type path  ↑↓ Suggestions  Enter Save  Esc Cancel")
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[2]);
}

fn draw_session_summary(
    frame: &mut Frame,
    selected: Option<&SessionItem>,
    area: Rect,
    theme: &Theme,
) {
    let text = if let Some(session) = selected {
        Text::from(vec![
            Line::from(vec![
                Span::styled(
                    provider_label(&session.provider_id),
                    Style::default()
                        .fg(theme.provider_color(&session.provider_id))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    session.title.as_deref().unwrap_or("(untitled)"),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Session", Style::default().fg(theme.text_dim)),
                Span::raw(format!("  {}", session.session_id)),
            ]),
            Line::from(vec![
                Span::styled("Workspace", Style::default().fg(theme.text_dim)),
                Span::raw(format!(
                    "  {}",
                    session.project_dir.as_deref().unwrap_or("(no dir)")
                )),
            ]),
        ])
    } else {
        Text::from(Line::from("No session selected."))
    };

    let summary = Paragraph::new(text)
        .block(section_block("Session", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(summary, area);
}

fn draw_detail_metadata(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let Some(session) = &app.loaded_session else {
        let placeholder = Paragraph::new("Session metadata is unavailable.")
            .block(section_block("Metadata", false, theme))
            .style(Style::default().fg(theme.text_dim).bg(theme.surface));
        frame.render_widget(placeholder, area);
        return;
    };

    let created = session
        .session
        .created_at
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string());
    let active = session
        .session
        .last_active_at
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string());

    let text = Text::from(vec![
        Line::from(vec![
            Span::styled("Created", Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", created)),
            Span::raw("    "),
            Span::styled("Last Active", Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", active)),
        ]),
        Line::from(vec![
            Span::styled("Messages", Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", session.messages.len())),
            Span::raw("    "),
            Span::styled("Source", Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", session.meta.source_provider)),
        ]),
    ]);

    let block = Paragraph::new(text)
        .block(section_block("Metadata", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(block, area);
}

fn draw_detail_messages(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let Some(session) = &app.loaded_session else {
        let placeholder = Paragraph::new("Session messages are unavailable.")
            .block(section_block("Message Preview", false, theme))
            .style(Style::default().fg(theme.text_dim).bg(theme.surface));
        frame.render_widget(placeholder, area);
        return;
    };

    let total = session.messages.len();
    if total == 0 {
        let empty = Paragraph::new("This session has no messages.")
            .block(section_block("Message Preview", false, theme))
            .style(Style::default().fg(theme.text_dim).bg(theme.surface));
        frame.render_widget(empty, area);
        return;
    }

    let start = app.detail_scroll.min(total.saturating_sub(1));
    let end = (start + 5).min(total);
    let mut lines = vec![Line::from(vec![
        Span::styled("Showing", Style::default().fg(theme.text_dim)),
        Span::raw(format!("  {}-{} of {}", start + 1, end, total)),
    ])];

    for message in session.messages.iter().skip(start).take(5) {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", message.role.to_string().to_uppercase()),
                role_style(message, theme),
            ),
            Span::styled(
                format!(" {}", message.timestamp.format("%H:%M:%S")),
                Style::default().fg(theme.text_dim),
            ),
        ]));
        lines.push(Line::from(Span::raw(content_preview(message))));
    }

    let block = Paragraph::new(Text::from(lines))
        .block(section_block("Message Preview", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(block, area);
}

fn draw_picker_block(
    frame: &mut Frame,
    title: &str,
    value: &str,
    hint: &str,
    focused: bool,
    area: Rect,
    theme: &Theme,
) {
    let text = Text::from(vec![
        Line::from(Span::styled(
            format!(" {} ", value),
            if focused {
                highlighted_value_style(theme)
            } else {
                Style::default().fg(theme.text).bg(theme.surface)
            },
        )),
        Line::from(Span::styled(hint, Style::default().fg(theme.text))),
    ]);

    let block = Paragraph::new(text)
        .block(section_block(title, focused, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(block, area);
}

fn draw_execute_block(frame: &mut Frame, label: &str, focused: bool, area: Rect, theme: &Theme) {
    let style = if focused {
        highlighted_value_style(theme)
    } else {
        Style::default().fg(theme.text).bg(theme.surface)
    };

    if area.height >= 3 && area.width >= 8 {
        let label_width = label.chars().count() as u16;
        let button_width = (label_width + 6).min(area.width);
        let button_area = Rect {
            x: area.x + area.width.saturating_sub(button_width) / 2,
            y: area.y + area.height.saturating_sub(3) / 2,
            width: button_width,
            height: 3,
        };
        let border = Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(theme.text).bg(theme.surface))
            .border_style(if focused {
                theme.border_focused
            } else {
                theme.border
            });
        frame.render_widget(border, button_area);

        let inner = button_area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });
        let button = Paragraph::new(Line::from(Span::styled(format!(" {} ", label), style)))
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text).bg(theme.surface));
        frame.render_widget(button, inner);
        return;
    }

    let button = Paragraph::new(Line::from(Span::styled(format!(" {} ", label), style)))
        .alignment(Alignment::Center)
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(button, area);
}

fn draw_action_result(frame: &mut Frame, result: &ActionResult, area: Rect, theme: &Theme) {
    let color = if result.is_error {
        theme.error
    } else {
        theme.success
    };
    let mut lines = vec![
        Line::from(Span::styled(
            &result.title,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
    ];

    for line in &result.lines {
        lines.push(Line::from(Span::raw(line.clone())));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Enter or Esc closes this result.",
        Style::default().fg(theme.text),
    )));

    let result = Paragraph::new(Text::from(lines))
        .block(section_block("Result", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(result, area);
}

fn draw_search_query(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let query = if app.search_query.is_empty() {
        " ".to_string()
    } else {
        app.search_query.clone()
    };

    let query_line = Paragraph::new(Text::from(vec![Line::from(vec![
        Span::styled("Type", Style::default().fg(theme.text_dim)),
        Span::raw("  "),
        Span::styled(
            format!(" {} ", query),
            Style::default()
                .fg(theme.text)
                .bg(theme.highlight)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
    ])]))
    .block(section_block("Query", true, theme))
    .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(query_line, area);
}

fn draw_search_scope_tabs(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let labels: Vec<&str> = SEARCH_SCOPE_OPTIONS
        .iter()
        .map(|scope| scope.label())
        .collect();
    draw_chip_row(
        frame,
        "Scope",
        &labels,
        app.search_scope_index,
        false,
        area,
        theme,
    );
}

fn draw_search_results(frame: &mut Frame, app: &App, matches: &[usize], area: Rect, theme: &Theme) {
    let lines = search_match_lines(app, matches, theme);
    let results = Paragraph::new(Text::from(lines))
        .block(section_block("Matches", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(results, area);
}

fn draw_search_preview(frame: &mut Frame, app: &App, matches: &[usize], area: Rect, theme: &Theme) {
    let lines = search_preview_lines(app, matches, theme);
    let preview = Paragraph::new(Text::from(lines))
        .block(section_block("Preview", false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(preview, area);
}

fn search_match_lines(app: &App, matches: &[usize], theme: &Theme) -> Vec<Line<'static>> {
    if matches.is_empty() {
        return vec![Line::from(Span::styled(
            "No matching sessions.",
            Style::default().fg(theme.warning),
        ))];
    }

    let active = app.search_match_index.min(matches.len() - 1);
    let sessions = app.flattened_sessions();
    let start = active.saturating_sub(2);
    let end = (start + 5).min(matches.len());
    let mut lines = vec![Line::from(vec![
        Span::styled("Matches", Style::default().fg(theme.text_dim)),
        Span::raw(format!("  {} total", matches.len())),
    ])];

    for (offset, match_pos) in (start..end).enumerate() {
        let session = sessions[matches[match_pos]];
        let style = if match_pos == active {
            Style::default()
                .fg(theme.primary)
                .bg(theme.highlight)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(theme.text).bg(theme.surface)
        };
        let title = session.title.as_deref().unwrap_or("(untitled)");
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(" {}. {}", start + offset + 1, theme::truncate(title, 44)),
            style,
        )));
    }

    lines
}

fn search_preview_lines(app: &App, matches: &[usize], theme: &Theme) -> Vec<Line<'static>> {
    if matches.is_empty() {
        return vec![Line::from(
            "Search works inside the sessions currently shown in the table.",
        )];
    }

    let active = app.search_match_index.min(matches.len() - 1);
    let sessions = app.flattened_sessions();
    let session = sessions[matches[active]];

    vec![
        Line::from(vec![
            Span::styled("Title", Style::default().fg(theme.text_dim)),
            Span::raw(format!(
                "  {}",
                session.title.as_deref().unwrap_or("(untitled)")
            )),
        ]),
        Line::from(vec![
            Span::styled("AI", Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", provider_label(&session.provider_id))),
            Span::raw("    "),
            Span::styled("Active", Style::default().fg(theme.text_dim)),
            Span::raw(format!(
                "  {}",
                theme::format_relative_time(session.last_active_at)
            )),
        ]),
        Line::from(vec![
            Span::styled("Session", Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", theme::truncate(&session.session_id, 34))),
        ]),
        Line::from(vec![
            Span::styled("Workspace", Style::default().fg(theme.text_dim)),
            Span::raw(format!(
                "  {}",
                session.project_dir.as_deref().unwrap_or("(no dir)")
            )),
        ]),
    ]
}

/// Handle session table page key events
pub fn handle_key(app: &mut App, key: KeyEvent) -> AppResult {
    if app.workspace_modal_open {
        return handle_workspace_modal_key(app, key);
    }
    if app.settings_modal_open {
        return handle_settings_modal_key(app, key);
    }
    if app.detail_modal_open {
        return handle_detail_key(app, key);
    }
    if app.action_modal_open {
        return handle_modal_key(app, key);
    }
    if app.search_modal_open {
        return handle_search_key(app, key);
    }

    match key.code {
        KeyCode::Up => {
            app.select_previous();
            AppResult::Continue
        }
        KeyCode::Down => {
            app.select_next();
            AppResult::Continue
        }
        KeyCode::Left => {
            if app.main_focus == MainFocus::Sessions {
                app.previous_provider_tab();
            } else {
                app.focus_previous_top_control();
            }
            AppResult::Continue
        }
        KeyCode::Right => {
            if app.main_focus == MainFocus::Sessions {
                app.next_provider_tab();
            } else {
                app.focus_next_top_control();
            }
            AppResult::Continue
        }
        KeyCode::Enter => {
            app.open_action_modal();
            AppResult::Continue
        }
        KeyCode::Esc if app.main_focus != MainFocus::Sessions => {
            app.main_focus = MainFocus::Sessions;
            AppResult::Continue
        }
        KeyCode::Char('f')
            if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.open_search_modal();
            AppResult::Continue
        }
        KeyCode::Char('q') if key.modifiers.is_empty() => AppResult::Quit,
        _ => AppResult::Continue,
    }
}

fn handle_workspace_modal_key(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Up => {
            app.step_main_workspace_picker(false);
            AppResult::Continue
        }
        KeyCode::Down => {
            app.step_main_workspace_picker(true);
            AppResult::Continue
        }
        KeyCode::Enter => {
            app.confirm_workspace_modal();
            AppResult::Continue
        }
        KeyCode::Esc => {
            app.close_workspace_modal();
            AppResult::Continue
        }
        KeyCode::Backspace | KeyCode::Char(_) => {
            app.edit_main_workspace_input(key.code);
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn handle_settings_modal_key(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Up => {
            app.move_settings_previous();
            AppResult::Continue
        }
        KeyCode::Down => {
            app.move_settings_next();
            AppResult::Continue
        }
        KeyCode::Left => {
            app.cycle_settings_value(false);
            AppResult::Continue
        }
        KeyCode::Right => {
            app.cycle_settings_value(true);
            AppResult::Continue
        }
        KeyCode::Enter => {
            app.activate_settings_field();
            AppResult::Continue
        }
        KeyCode::Esc => {
            app.close_settings_modal();
            AppResult::Continue
        }
        KeyCode::Backspace | KeyCode::Char(_) => {
            app.edit_settings_number(key.code);
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn handle_modal_key(app: &mut App, key: KeyEvent) -> AppResult {
    if app.action_result.is_some() {
        match key.code {
            KeyCode::Enter | KeyCode::Esc => app.close_action_modal(),
            _ => {}
        }
        return AppResult::Continue;
    }

    if app.action_dialog.is_some() {
        return handle_action_dialog_key(app, key);
    }

    if app.current_action() == SessionAction::Rename && app.action_field == ActionField::RenameTitle
    {
        match key.code {
            KeyCode::Char(_) | KeyCode::Backspace => {
                app.edit_rename_input(key.code);
                return AppResult::Continue;
            }
            _ => {}
        }
    }

    if app.current_action() == SessionAction::Export && app.action_field == ActionField::ExportPath
    {
        match key.code {
            KeyCode::Char(_) | KeyCode::Backspace => {
                app.edit_export_output_prefix(key.code);
                return AppResult::Continue;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Up => {
            app.move_modal_field_previous();
            AppResult::Continue
        }
        KeyCode::Down => {
            app.move_modal_field_next();
            AppResult::Continue
        }
        KeyCode::Left => {
            app.cycle_modal_value(false);
            AppResult::Continue
        }
        KeyCode::Right => {
            app.cycle_modal_value(true);
            AppResult::Continue
        }
        KeyCode::Enter => {
            app.activate_modal_field();
            AppResult::Continue
        }
        KeyCode::Esc => {
            app.close_action_modal();
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn handle_action_dialog_key(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Up => {
            app.cycle_action_dialog_selection(false);
            AppResult::Continue
        }
        KeyCode::Down => {
            app.cycle_action_dialog_selection(true);
            AppResult::Continue
        }
        KeyCode::Enter => {
            app.confirm_action_dialog();
            AppResult::Continue
        }
        KeyCode::Esc => {
            app.close_action_dialog();
            AppResult::Continue
        }
        KeyCode::Backspace | KeyCode::Char(_)
            if matches!(app.action_dialog, Some(ActionDialog::TargetWorkspace)) =>
        {
            app.edit_workspace_input(key.code);
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn handle_search_key(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Up => {
            app.previous_search_match();
            AppResult::Continue
        }
        KeyCode::Down => {
            app.next_search_match();
            AppResult::Continue
        }
        KeyCode::Left => {
            app.cycle_search_scope(false);
            AppResult::Continue
        }
        KeyCode::Right => {
            app.cycle_search_scope(true);
            AppResult::Continue
        }
        KeyCode::Enter => {
            app.accept_search_selection();
            AppResult::Continue
        }
        KeyCode::Esc => {
            app.close_search_modal();
            AppResult::Continue
        }
        KeyCode::Backspace | KeyCode::Char(_) => {
            app.edit_search_query(key.code);
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn handle_detail_key(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Up => {
            app.detail_scroll_up();
            AppResult::Continue
        }
        KeyCode::Down => {
            app.detail_scroll_down();
            AppResult::Continue
        }
        KeyCode::Esc => {
            app.close_detail_modal();
            AppResult::Continue
        }
        _ => AppResult::Continue,
    }
}

fn highlighted_value_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.primary)
        .bg(theme.highlight)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

fn selected_row_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.primary)
        .bg(theme.background)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
}

fn action_footer_text(app: &App) -> &'static str {
    match app.action_field {
        ActionField::Action => "↑↓ Focus  ←→ Action  Enter Next  Esc Close",
        ActionField::TargetAgent => "↑↓ Focus  Enter Choose Target  Esc Close",
        ActionField::TargetWorkspace => "↑↓ Focus  Enter Choose Workspace  Esc Close",
        ActionField::ExportPath => "Type path  ↑↓ Focus  Enter Run  Esc Close",
        ActionField::RenameTitle => "Type title  ↑↓ Focus  Enter Run  Esc Close",
        ActionField::Execute => "↑↓ Focus  Enter Run  Esc Close",
    }
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

fn role_style(message: &MemorphMessage, theme: &Theme) -> Style {
    let color = match message.role.to_string().as_str() {
        "user" => theme.accent,
        "assistant" => theme.primary,
        "tool" => theme.secondary,
        _ => theme.text_dim,
    };

    Style::default()
        .fg(color)
        .bg(theme.highlight)
        .add_modifier(Modifier::BOLD)
}

fn content_preview(message: &MemorphMessage) -> String {
    for block in &message.content {
        match block {
            ContentBlock::Text { text } => return theme::truncate(text, 96),
            ContentBlock::Thinking { thinking, .. } => {
                return format!("Thinking: {}", theme::truncate(thinking, 84));
            }
            ContentBlock::ToolUse { name, .. } => return format!("Tool call: {}", name),
            ContentBlock::ToolResult { content, .. } => {
                return format!("Tool result: {}", theme::truncate(content, 80));
            }
            ContentBlock::File { path, .. } => return format!("File: {}", path),
            ContentBlock::Image { .. } => return "Image attachment".to_string(),
        }
    }

    "(empty message)".to_string()
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
