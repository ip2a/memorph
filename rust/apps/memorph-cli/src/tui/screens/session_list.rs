use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::{
    provider_label, ActionDialog, ActionField, ActionResult, AgentManagementFocus, App, AppResult,
    SearchScope, SessionAction, ACTION_OPTIONS,
};
use crate::tui::overlays::filter::{self, FilterScope, FilterState};
use crate::tui::overlays::{
    confirm::ConfirmAction, input::InputAction, picker::PickerAction, Overlay,
};
use crate::tui::theme::{self, Theme};
use memorph::core::{compression, SessionItem};
use memorph::session::{Block as EventBlock, Event, Role};

/// Draw session table page
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    if matches!(&app.overlay, Overlay::Detail) {
        draw_detail_modal(frame, app, area, theme);
        return;
    }

    let table_area = if matches!(&app.overlay, Overlay::Search) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);
        draw_inline_search(frame, app, chunks[0], theme);
        chunks[1]
    } else {
        area
    };

    let language = app.language();
    crate::tui::widgets::session_table::draw(
        frame,
        &app.session_groups,
        &mut app.table_state,
        language,
        table_area,
        theme,
    );

    if matches!(&app.overlay, Overlay::Action) {
        draw_action_modal(frame, app, area, theme);
    }
}

fn draw_inline_search(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let matches = app.search_matches();
    let state = FilterState {
        query: app.search_query.clone(),
        scope: match app.current_search_scope() {
            SearchScope::All => FilterScope::All,
            SearchScope::Title => FilterScope::Title,
            SearchScope::SessionId => FilterScope::SessionId,
            SearchScope::Workspace => FilterScope::Workspace,
        },
        match_count: matches.len(),
        current_match: app.search_match_index.min(matches.len().saturating_sub(1)),
    };
    filter::draw(frame, &state, app.language(), area, theme);
}

fn draw_chip_row<T: AsRef<str>>(
    frame: &mut Frame,
    title: &str,
    options: &[T],
    selected: usize,
    focused: bool,
    area: Rect,
    theme: &Theme,
) {
    let mut spans = Vec::new();
    let row_bg = if focused {
        theme.surface_active
    } else {
        theme.surface
    };

    for (index, option) in options.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }

        let style = if index == selected {
            highlighted_value_style(theme)
        } else {
            Style::default().fg(theme.text).bg(theme.surface)
        };
        spans.push(Span::styled(format!(" {} ", option.as_ref()), style));
    }

    let row = Paragraph::new(Line::from(spans))
        .block(chip_row_block(title, focused, row_bg, theme))
        .style(Style::default().fg(theme.text).bg(row_bg))
        .wrap(Wrap { trim: true });
    frame.render_widget(row, area);
}

fn draw_action_modal(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(84, 82, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block(app.t("sessionMoreActions"), theme), popup_area);

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
    draw_session_summary(frame, app, selected, chunks[0], theme);
    draw_action_tabs(frame, app, chunks[1], theme);

    if let Some(result) = &app.action_result {
        draw_action_result(frame, app, result, chunks[2], theme);
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
    let labels: Vec<&str> = ACTION_OPTIONS
        .iter()
        .map(|action| action.label(app.language()))
        .collect();
    draw_chip_row(
        frame,
        app.t("actions"),
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
        SessionAction::Compress => draw_compress_panel(frame, app, area, theme),
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
        app.t("targetAgent"),
        app.selected_target_provider()
            .map(provider_label)
            .unwrap_or("-"),
        app.t("pressEnterChooseTargetAgent"),
        app.action_field == ActionField::TargetAgent,
        chunks[0],
        theme,
    );
    draw_picker_block(
        frame,
        app.t("workspace"),
        app.selected_target_workspace()
            .as_deref()
            .unwrap_or(app.t("workspaceEmpty")),
        app.t("pressEnterChooseWorkspace"),
        app.action_field == ActionField::TargetWorkspace,
        chunks[1],
        theme,
    );
    draw_execute_block(
        frame,
        app.t("runSwitch"),
        app.action_field == ActionField::Execute,
        chunks[2],
        theme,
    );

    let note = Paragraph::new(app.t("switchPanelHint"))
        .block(section_block(app.t("whatHappens"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(note, chunks[3]);
}

fn draw_compress_panel(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(9),
            Constraint::Length(4),
            Constraint::Length(5),
        ])
        .split(area);

    draw_picker_block(
        frame,
        app.t("targetAgent"),
        app.selected_target_provider()
            .map(provider_label)
            .unwrap_or("-"),
        app.t("tuiCompressionChooseTargetHint"),
        app.action_field == ActionField::TargetAgent,
        chunks[0],
        theme,
    );

    draw_compression_candidates(frame, app, chunks[1], theme);

    draw_execute_block(
        frame,
        app.t("tuiRunCompression"),
        app.action_field == ActionField::Execute,
        chunks[2],
        theme,
    );

    let note = Paragraph::new(app.t("tuiCompressionPanelHint"))
        .block(section_block(app.t("whatHappens"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(note, chunks[3]);
}

fn draw_compression_candidates(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    if let Some(error) = &app.compression_plan_error {
        let body = Paragraph::new(error.clone())
            .block(section_block(
                app.t("tuiCompressionCandidates"),
                app.action_field == ActionField::CompressionCandidates,
                theme,
            ))
            .style(Style::default().fg(theme.error).bg(theme.surface))
            .wrap(Wrap { trim: true });
        frame.render_widget(body, area);
        return;
    }

    let Some(report) = &app.compression_plan else {
        let body = Paragraph::new(app.t("tuiCompressionChooseTargetPlan"))
            .block(section_block(
                app.t("tuiCompressionCandidates"),
                app.action_field == ActionField::CompressionCandidates,
                theme,
            ))
            .style(Style::default().fg(theme.text).bg(theme.surface))
            .wrap(Wrap { trim: true });
        frame.render_widget(body, area);
        return;
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                app.t("tuiCompressionCandidates"),
                Style::default().fg(theme.text_dim),
            ),
            Span::raw(format!("  {}", report.candidates.len())),
            Span::raw("    "),
            Span::styled(app.t("selected"), Style::default().fg(theme.text_dim)),
            Span::raw(format!(
                "  {}",
                app.compression_selected_candidate_ids.len()
            )),
        ]),
        Line::from(vec![
            Span::styled(
                app.t("tuiCompressionEstimatedSaved"),
                Style::default().fg(theme.text_dim),
            ),
            Span::raw(format!(
                "  {} {} / {} {}",
                report.estimated_bytes_saved,
                app.t("bytes"),
                report.estimated_tokens_saved,
                app.t("tokens")
            )),
        ]),
    ];

    if report.candidates.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            app.t("tuiNoCompressionCandidates"),
            Style::default().fg(theme.warning),
        )));
    } else {
        let selected_index = app
            .compression_candidate_index
            .min(report.candidates.len().saturating_sub(1));
        let start = selected_index.saturating_sub(2);
        let end = (start + 5).min(report.candidates.len());
        for (index, candidate) in report.candidates.iter().enumerate().take(end).skip(start) {
            let active = index == selected_index;
            let checked = if app.compression_candidate_selected(&candidate.id) {
                "[x]"
            } else {
                "[ ]"
            };
            let style = if active {
                highlighted_value_style(theme)
            } else {
                Style::default().fg(theme.text).bg(theme.surface)
            };
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "{} {} {:?} {}={}B {}={:?}",
                    checked,
                    candidate.id,
                    candidate.kind,
                    app.t("saved"),
                    candidate.estimated_bytes_saved,
                    app.t("risk"),
                    candidate.risk
                ),
                style,
            )));
            lines.push(Line::from(Span::styled(
                format!("{}: {}", app.t("events"), candidate.event_ids.join(", ")),
                Style::default().fg(theme.text_dim),
            )));
        }
    }

    if !report.skipped.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            app.tf(
                "tuiCompressionSkipped",
                &[("count", &report.skipped.len().to_string())],
            ),
            Style::default().fg(theme.text_dim),
        )));
    }

    let body = Paragraph::new(Text::from(lines))
        .block(section_block(
            app.t("tuiCompressionCandidates"),
            app.action_field == ActionField::CompressionCandidates,
            theme,
        ))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(body, area);
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
        app.t("newTitle"),
        if app.rename_input.is_empty() {
            app.t("empty")
        } else {
            &app.rename_input
        },
        app.t("typeDirectlyInField"),
        app.action_field == ActionField::RenameTitle,
        chunks[0],
        theme,
    );
    draw_execute_block(
        frame,
        app.t("runRename"),
        app.action_field == ActionField::Execute,
        chunks[1],
        theme,
    );

    let note = Paragraph::new(app.t("renamePanelHint"))
        .block(section_block(app.t("howTo"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(note, chunks[2]);
}

fn draw_delete_panel(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(4)])
        .split(area);

    let warning = Paragraph::new(app.t("deletePanelHint"))
        .block(section_block(app.t("warning"), false, theme))
        .style(Style::default().fg(theme.warning).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(warning, chunks[0]);

    draw_execute_block(
        frame,
        app.t("runDelete"),
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
        app.t("outputPrefix"),
        if app.export_output_prefix.is_empty() {
            app.t("empty")
        } else {
            &app.export_output_prefix
        },
        app.t("tuiExportPathHint"),
        app.action_field == ActionField::ExportPath,
        chunks[0],
        theme,
    );

    draw_execute_block(
        frame,
        app.t("runExport"),
        app.action_field == ActionField::Execute,
        chunks[1],
        theme,
    );

    let info = Paragraph::new(app.t("exportPanelHint"))
        .block(section_block(app.t("export"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(info, chunks[2]);
}

fn draw_details_panel(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(4)])
        .split(area);

    let info = Paragraph::new(app.t("detailsPanelHint"))
        .block(section_block(app.t("details"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(info, chunks[0]);

    draw_execute_block(
        frame,
        app.t("openDetail"),
        app.action_field == ActionField::Execute,
        chunks[1],
        theme,
    );
}

fn draw_detail_modal(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let popup_area = area;
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block(app.t("sessionDetails"), theme), popup_area);

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
    draw_session_summary(frame, app, selected, chunks[0], theme);
    draw_detail_metadata(frame, app, chunks[1], theme);
    draw_detail_messages(frame, app, chunks[2], theme);

    let footer = Paragraph::new(app.t("tuiFooterDetailModal"))
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
    frame.render_widget(modal_block(app.t("targetAgent"), theme), popup_area);

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
            app.t("noAgentAvailableForSwitching"),
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
        .block(section_block(app.t("agents"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(body, chunks[0]);

    let footer = Paragraph::new(app.t("tuiFooterDialogSelectSave"))
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[1]);
}

fn draw_workspace_dialog(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let popup_area = centered_rect(80, 68, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block(app.t("workspace"), theme), popup_area);

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
            app.t("typeEditPathDirectly"),
            Style::default().fg(theme.text),
        )),
    ]))
    .block(section_block(app.t("workspacePath"), false, theme))
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
            app.t("noMatchingSavedWorkspace"),
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
        .block(section_block(app.t("suggestions"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(suggestions, chunks[1]);

    let footer = Paragraph::new(app.t("tuiFooterWorkspaceDialog"))
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[2]);
}

fn draw_session_summary(
    frame: &mut Frame,
    app: &App,
    selected: Option<&SessionItem>,
    area: Rect,
    theme: &Theme,
) {
    let text = if let Some(session) = selected {
        let lines = vec![
            Line::from(vec![
                Span::styled(
                    provider_label(&session.provider_id),
                    Style::default()
                        .fg(theme.provider_color(&session.provider_id))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(
                    session.title.as_deref().unwrap_or(app.t("untitled")),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled(app.t("session"), Style::default().fg(theme.text_dim)),
                Span::raw(format!("  {}", session.session_id)),
            ]),
            Line::from(vec![
                Span::styled(app.t("workspace"), Style::default().fg(theme.text_dim)),
                Span::raw(format!(
                    "  {}",
                    session.project_dir.as_deref().unwrap_or(app.t("noDir"))
                )),
            ]),
        ];

        Text::from(lines)
    } else {
        Text::from(Line::from(app.t("noSessionSelected")))
    };

    let summary = Paragraph::new(text)
        .block(section_block(app.t("session"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(summary, area);
}

fn draw_detail_metadata(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let Some(session) = &app.loaded_session else {
        let placeholder = Paragraph::new(app.t("sessionMetadataUnavailable"))
            .block(section_block(app.t("metadata"), false, theme))
            .style(Style::default().fg(theme.text_dim).bg(theme.surface));
        frame.render_widget(placeholder, area);
        return;
    };

    let created = session
        .created_at
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string());
    let active = session
        .last_active_at
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "-".to_string());

    let lines = vec![
        Line::from(vec![
            Span::styled(app.t("createdAt"), Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", created)),
            Span::raw("    "),
            Span::styled(app.t("lastActiveAt"), Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", active)),
        ]),
        Line::from(vec![
            Span::styled(app.t("messages"), Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", session.message_count)),
            Span::raw("    "),
            Span::styled(app.t("source"), Style::default().fg(theme.text_dim)),
            Span::raw(format!("  {}", session.provider_name)),
        ]),
    ];

    let text = Text::from(lines);

    let block = Paragraph::new(text)
        .block(section_block(app.t("metadata"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(block, area);
}

fn draw_detail_messages(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let Some(session) = &app.loaded_session else {
        let placeholder = Paragraph::new(app.t("sessionMessagesUnavailable"))
            .block(section_block(app.t("messagePreview"), false, theme))
            .style(Style::default().fg(theme.text_dim).bg(theme.surface));
        frame.render_widget(placeholder, area);
        return;
    };

    let total = session.events.len();
    if total == 0 {
        let empty = Paragraph::new(app.t("thisSessionHasNoMessages"))
            .block(section_block(app.t("messagePreview"), false, theme))
            .style(Style::default().fg(theme.text_dim).bg(theme.surface));
        frame.render_widget(empty, area);
        return;
    }

    let start = app.detail_scroll.min(total.saturating_sub(1));
    let end = (start + 5).min(total);
    let mut lines = vec![Line::from(Span::styled(
        app.tf(
            "showingRange",
            &[
                ("start", &(start + 1).to_string()),
                ("end", &end.to_string()),
                ("total", &total.to_string()),
            ],
        ),
        Style::default().fg(theme.text_dim),
    ))];

    for event in session.events.iter().skip(start).take(5) {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} ", role_name(event.role).to_uppercase()),
                role_style(event, theme),
            ),
            Span::styled(
                format!(" {}", event.timestamp.format("%H:%M:%S")),
                Style::default().fg(theme.text_dim),
            ),
        ]));
        lines.push(Line::from(Span::raw(content_preview(
            event,
            app.language(),
        ))));
    }

    let block = Paragraph::new(Text::from(lines))
        .block(section_block(app.t("messagePreview"), false, theme))
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

fn draw_action_result(
    frame: &mut Frame,
    app: &App,
    result: &ActionResult,
    area: Rect,
    theme: &Theme,
) {
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
        app.t("enterOrEscClosesResult"),
        Style::default().fg(theme.text),
    )));

    let result = Paragraph::new(Text::from(lines))
        .block(section_block(app.t("result"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(result, area);
}

/// Handle session table page key events
pub fn handle_key(app: &mut App, key: KeyEvent) -> AppResult {
    if app.overlay.is_some() {
        return handle_overlay_key(app, key);
    }
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            app.select_previous();
            AppResult::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.select_next();
            AppResult::Continue
        }
        KeyCode::Left | KeyCode::Char('h') => {
            app.previous_provider_tab();
            AppResult::Continue
        }
        KeyCode::Right | KeyCode::Char('l') => {
            app.next_provider_tab();
            AppResult::Continue
        }
        KeyCode::Enter => {
            app.open_session_action(SessionAction::Details);
            app.execute_modal_action();
            AppResult::Continue
        }
        KeyCode::Char('/') => {
            app.open_search_modal();
            AppResult::Continue
        }
        KeyCode::Char('f')
            if key.modifiers.is_empty() || key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            app.open_search_modal();
            AppResult::Continue
        }
        KeyCode::Char('d') if key.modifiers.is_empty() => {
            app.open_delete_confirmation();
            AppResult::Continue
        }
        KeyCode::Char('r') if key.modifiers.is_empty() => {
            app.open_rename_input_overlay();
            AppResult::Continue
        }
        KeyCode::Char('e') if key.modifiers.is_empty() => {
            app.open_export_input_overlay();
            AppResult::Continue
        }
        KeyCode::Char('s') if key.modifiers.is_empty() => {
            app.open_transfer_target_picker(SessionAction::Switch);
            AppResult::Continue
        }
        KeyCode::Char('c') if key.modifiers.is_empty() => {
            app.open_transfer_target_picker(SessionAction::Compress);
            AppResult::Continue
        }
        KeyCode::Char('w') if key.modifiers.is_empty() => {
            app.open_workspace_modal();
            AppResult::Continue
        }
        KeyCode::Char('a') if key.modifiers.is_empty() => {
            app.open_agents_modal();
            AppResult::Continue
        }
        KeyCode::Char(',') if key.modifiers.is_empty() => {
            app.open_settings_modal();
            AppResult::Continue
        }
        KeyCode::Char('q') if key.modifiers.is_empty() => AppResult::Quit,
        _ => AppResult::Continue,
    }
}

fn handle_overlay_key(app: &mut App, key: KeyEvent) -> AppResult {
    match &mut app.overlay {
        Overlay::Input(state) => match state.handle_key(key) {
            InputAction::Continue => {}
            InputAction::Cancel => app.overlay = Overlay::None,
            InputAction::Confirm => {
                let kind = state.kind;
                let value = state.value.clone();
                app.submit_input_overlay(kind, value);
            }
        },
        Overlay::Confirm(state) => match state.handle_key(key) {
            ConfirmAction::Continue => {}
            ConfirmAction::Cancel => app.overlay = Overlay::None,
            ConfirmAction::Confirm => app.confirm_delete_overlay(),
        },
        Overlay::Picker(state) => match state.handle_key(key) {
            PickerAction::Continue => {}
            PickerAction::Cancel => {
                app.overlay = Overlay::None;
                app.close_action_modal();
            }
            PickerAction::Confirm => {
                let kind = state.kind.clone();
                if let Some(item) = state.selected_item() {
                    let id = item.id.clone();
                    app.submit_picker_overlay(kind, id);
                }
            }
        },
        Overlay::Workspace => return handle_workspace_modal_key(app, key),
        Overlay::Agents => return handle_agents_modal_key(app, key),
        Overlay::Settings => return handle_settings_modal_key(app, key),
        Overlay::Search => return handle_search_key(app, key),
        Overlay::Detail => return handle_detail_key(app, key),
        Overlay::Action => return handle_modal_key(app, key),
        Overlay::Help | Overlay::None => app.overlay = Overlay::None,
    }
    AppResult::Continue
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

fn handle_agents_modal_key(app: &mut App, key: KeyEvent) -> AppResult {
    match key.code {
        KeyCode::Left => {
            app.agent_management_focus = AgentManagementFocus::Providers;
            AppResult::Continue
        }
        KeyCode::Right => {
            app.agent_management_focus = AgentManagementFocus::Actions;
            AppResult::Continue
        }
        KeyCode::Up => {
            match app.agent_management_focus {
                AgentManagementFocus::Providers => app.step_agent_management_selection(false),
                AgentManagementFocus::Actions => app.step_agent_management_action(false),
            }
            AppResult::Continue
        }
        KeyCode::Down => {
            match app.agent_management_focus {
                AgentManagementFocus::Providers => app.step_agent_management_selection(true),
                AgentManagementFocus::Actions => app.step_agent_management_action(true),
            }
            AppResult::Continue
        }
        KeyCode::Enter => {
            match app.agent_management_focus {
                AgentManagementFocus::Providers => {
                    app.agent_management_focus = AgentManagementFocus::Actions;
                }
                AgentManagementFocus::Actions => app.run_primary_agent_management_action(),
            }
            AppResult::Continue
        }
        KeyCode::Esc => {
            app.close_agents_modal();
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
        KeyCode::Up | KeyCode::Char('k') => {
            app.detail_scroll_up();
            AppResult::Continue
        }
        KeyCode::Down | KeyCode::Char('j') => {
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

fn action_footer_text(app: &App) -> &'static str {
    match app.action_field {
        ActionField::Action => app.t("tuiFooterActionSelect"),
        ActionField::TargetAgent => app.t("tuiFooterChooseTarget"),
        ActionField::TargetWorkspace => app.t("tuiFooterChooseWorkspace"),
        ActionField::CompressionCandidates => app.t("tuiFooterCompressionCandidates"),
        ActionField::ExportPath => app.t("tuiFooterTypePathRun"),
        ActionField::RenameTitle => app.t("tuiFooterTypeTitleRun"),
        ActionField::Execute => app.t("tuiFooterExecute"),
    }
}

fn modal_block(title: &str, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .border_style(theme.border_focused)
}

fn chip_row_block(
    title: &str,
    focused: bool,
    background: ratatui::style::Color,
    theme: &Theme,
) -> Block<'static> {
    Block::default()
        .title(format!(" {} ", title))
        .borders(Borders::ALL)
        .style(Style::default().fg(theme.text).bg(background))
        .border_style(if focused {
            theme.border_focused
        } else {
            theme.border
        })
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

fn role_style(event: &Event, theme: &Theme) -> Style {
    let color = match event.role {
        Role::User => theme.accent,
        Role::Assistant => theme.primary,
        Role::Tool => theme.secondary,
        Role::Other => theme.warning,
        _ => theme.text_dim,
    };

    Style::default()
        .fg(color)
        .bg(theme.highlight)
        .add_modifier(Modifier::BOLD)
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
        Role::System => "system",
        Role::Developer => "developer",
        _ => "unknown",
    }
}

fn content_preview(event: &Event, language: memorph::config::UiLanguage) -> String {
    if let Some(segment) = compression::compressed_segment(event) {
        return format!(
            "{}: {}",
            memorph::i18n::text(language, "compressed"),
            theme::truncate(&segment.summary, 80)
        );
    }

    if let Some(block) = event.blocks.first() {
        match block {
            EventBlock::Text { text } => return theme::truncate(text, 96),
            EventBlock::Thinking { text, .. } => {
                return format!(
                    "{}: {}",
                    memorph::i18n::text(language, "thinking"),
                    theme::truncate(text, 84)
                );
            }
            EventBlock::ToolCall { name, .. } => {
                return format!("{}: {}", memorph::i18n::text(language, "toolCall"), name)
            }
            EventBlock::ToolResult { content, .. } => {
                return format!(
                    "{}: {}",
                    memorph::i18n::text(language, "toolResultLabel"),
                    theme::truncate(content, 80)
                );
            }
            EventBlock::Patch { files, .. } => {
                return if files.is_empty() {
                    memorph::i18n::text(language, "patch").to_string()
                } else {
                    format!(
                        "{}: {}",
                        memorph::i18n::text(language, "patch"),
                        theme::truncate(&files.join(", "), 80)
                    )
                };
            }
            EventBlock::Command { command, .. } => {
                return format!("{}: {}", memorph::i18n::text(language, "command"), command)
            }
            EventBlock::CommandResult { stdout, .. } => {
                return format!(
                    "{}: {}",
                    memorph::i18n::text(language, "commandResult"),
                    theme::truncate(
                        stdout
                            .as_deref()
                            .unwrap_or(memorph::i18n::text(language, "noOutput")),
                        76
                    )
                );
            }
            EventBlock::File { path, .. } => {
                return format!("{}: {}", memorph::i18n::text(language, "file"), path)
            }
            EventBlock::Image { .. } => {
                return memorph::i18n::text(language, "imageAttachment").to_string()
            }
            EventBlock::Compressed { .. } => {
                return memorph::i18n::text(language, "compressedContext").to_string()
            }
            EventBlock::Other { raw } => {
                return raw
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .map(|kind| format!("{}: {kind}", memorph::i18n::text(language, "payload")))
                    .unwrap_or_else(|| memorph::i18n::text(language, "otherPayload").to_string());
            }
        }
    }

    memorph::i18n::text(language, "emptyEvent").to_string()
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
