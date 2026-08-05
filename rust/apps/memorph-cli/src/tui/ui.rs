use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::app::{
    provider_label, AgentManagementActionKind, AgentManagementFocus, App, SettingsField,
    SETTINGS_FIELDS,
};
use super::theme::Theme;
use memorph::config;

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
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(frame, app, chunks[0], &theme);
    draw_main(frame, app, chunks[1], &theme);
    draw_footer(frame, app, chunks[2], &theme);

    match &app.overlay {
        super::overlays::Overlay::Help => draw_help_overlay(frame, app, &theme),
        super::overlays::Overlay::Workspace => draw_workspace_modal(frame, app, &theme),
        super::overlays::Overlay::Agents => draw_agents_modal(frame, app, &theme),
        super::overlays::Overlay::Settings => draw_settings_modal(frame, app, &theme),
        super::overlays::Overlay::Input(state) => {
            super::overlays::input::draw(frame, state, app.language(), area, &theme)
        }
        super::overlays::Overlay::Confirm(state) => {
            super::overlays::confirm::draw(frame, state, app.language(), area, &theme)
        }
        super::overlays::Overlay::Picker(state) => {
            super::overlays::picker::draw(frame, state, app.language(), area, &theme)
        }
        _ => {}
    }
    if app.is_loading() {
        draw_loading_overlay(frame, app, &theme);
    }
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    super::widgets::header::draw(
        frame,
        app.workspace.as_deref(),
        app.session_count(),
        &app.provider_tabs(),
        app.selected_provider_tab,
        app.language(),
        area,
        theme,
    );
}

fn draw_main(frame: &mut Frame, app: &mut App, area: Rect, theme: &Theme) {
    super::screens::session_list::draw(frame, app, area, theme);
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    if let Some(message) = app.error_message.as_deref() {
        frame.render_widget(
            Paragraph::new(message)
                .style(Style::default().fg(theme.error).bg(theme.background))
                .alignment(Alignment::Center),
            area,
        );
    } else {
        super::widgets::hint_bar::draw(frame, app, area, theme);
    }
}

fn draw_help_overlay(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let popup_area = centered_rect(64, 72, area);

    frame.render_widget(Clear, popup_area);

    let help_text = Text::from(vec![
        Line::from(Span::styled(
            app.t("keyboardShortcuts"),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![Span::styled(
            app.t("navigation"),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(app.t("tuiHelpSelectSession")),
        Line::from(app.t("tuiHelpSwitchProvider")),
        Line::from(app.t("tuiHelpOpenDetails")),
        Line::from(app.t("tuiHelpFilterSessions")),
        Line::from(""),
        Line::from(vec![Span::styled(
            app.t("sessionMoreActions"),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(app.t("tuiHelpSwitchAgent")),
        Line::from(app.t("tuiHelpCompressSession")),
        Line::from(app.t("tuiHelpExportJson")),
        Line::from(app.t("tuiHelpRenameSession")),
        Line::from(app.t("tuiHelpDeleteSession")),
        Line::from(""),
        Line::from(vec![Span::styled(
            app.t("manage"),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        )]),
        Line::from(app.t("tuiHelpWorkspace")),
        Line::from(app.t("tuiHelpAgents")),
        Line::from(app.t("tuiHelpSettings")),
        Line::from(app.t("tuiHelpToggleHelp")),
        Line::from(app.t("tuiHelpQuit")),
        Line::from(app.t("tuiHelpBackCancel")),
    ]);

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(format!(" {} ", app.t("help")))
                .borders(Borders::ALL)
                .style(Style::default().fg(theme.text).bg(theme.surface))
                .border_style(theme.border_focused),
        )
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });

    frame.render_widget(help, popup_area);
}

fn draw_loading_overlay(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let label = app.t("loading");
    let width = (label.chars().count() as u16 + 10)
        .max(18)
        .min(area.width.max(1));
    let height = 3.min(area.height.max(1));
    let popup_area = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    );
    let inner = popup_area.inner(ratatui::layout::Margin {
        horizontal: 1,
        vertical: 1,
    });

    frame.render_widget(Clear, popup_area);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .style(Style::default().fg(theme.text).bg(theme.surface))
            .border_style(theme.border_focused),
        popup_area,
    );

    let spinner = Paragraph::new(format!("{} {}", app.loading_spinner(), label))
        .style(
            Style::default()
                .fg(theme.text)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(Alignment::Center);
    frame.render_widget(spinner, inner);
}

fn draw_workspace_modal(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let popup_area = centered_rect(80, 68, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block(app.t("workspace"), theme), popup_area);

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
            app.t("typeLocalWorkspacePath"),
            Style::default().fg(theme.text),
        )),
    ]))
    .block(section_block(app.t("workspacePath"), true, theme))
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
            app.t("noSavedWorkspaceMatchesPath"),
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
        .block(section_block(app.t("savedWorkspaces"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(suggestions, chunks[1]);

    let footer = Paragraph::new(app.t("tuiFooterWorkspaceDialog"))
        .style(Style::default().fg(theme.text).bg(theme.surface));
    frame.render_widget(footer, chunks[2]);
}

fn draw_agents_modal(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let popup_area = centered_rect(82, 58, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block(app.t("agents"), theme), popup_area);

    let inner = popup_area.inner(ratatui::layout::Margin {
        horizontal: 2,
        vertical: 1,
    });
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(24), Constraint::Min(0)])
        .split(inner);

    let mut provider_lines = Vec::new();
    if app.agent_management_entries.is_empty() {
        provider_lines.push(Line::from(Span::styled(
            app.t("noProviders"),
            Style::default().fg(theme.warning),
        )));
    } else {
        for (index, entry) in app.agent_management_entries.iter().enumerate() {
            let selected = index == app.agent_management_index;
            let label = provider_label(&entry.provider_id);
            let status = if entry.environment.installed {
                app.t("installed")
            } else {
                app.t("notDetected")
            };
            let style = agent_management_item_style(
                selected,
                app.agent_management_focus == AgentManagementFocus::Providers,
                theme,
            );
            provider_lines.push(Line::from(Span::styled(
                format!(" {} [{}] ", label, status),
                style,
            )));
            if index + 1 < app.agent_management_entries.len() {
                provider_lines.push(Line::from(""));
            }
        }
    }
    let providers = Paragraph::new(Text::from(provider_lines))
        .block(section_block(
            app.t("agentManagementProvidersLabel"),
            app.agent_management_focus == AgentManagementFocus::Providers,
            theme,
        ))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(providers, chunks[0]);

    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Min(8),
        ])
        .split(chunks[1]);

    let selected = app.selected_agent_management_entry();
    let detail_lines = if let Some(entry) = selected {
        let mut lines = vec![
            Line::from(format!(
                "{}: {}",
                app.t("provider"),
                provider_label(&entry.provider_id)
            )),
            Line::from(format!(
                "{}: {}",
                app.t("workspace"),
                app.workspace.as_deref().unwrap_or(app.t("workspaceEmpty"))
            )),
            Line::from(format!(
                "{}: {}",
                app.t("agentInstallStatus"),
                if entry.environment.installed {
                    app.t("installed")
                } else {
                    app.t("notDetected")
                }
            )),
            Line::from(format!(
                "{}: {}",
                app.t("agentInstallMethod"),
                if entry.environment.install_method.trim().is_empty() {
                    app.t("unknown")
                } else {
                    entry.environment.install_method.as_str()
                }
            )),
            Line::from(format!(
                "{}: {}",
                app.t("agentConfigPath"),
                entry.environment.config_path
            )),
            Line::from(format!(
                "{}: {}",
                app.t("agentExecutableDir"),
                entry.environment.executable_dir.as_deref().unwrap_or("—")
            )),
            Line::from(format!(
                "{}: {}",
                app.t("agentExecutablePath"),
                entry.environment.executable_path.as_deref().unwrap_or("—")
            )),
            Line::from(format!(
                "{}: {}",
                app.t("hookStatusLabel"),
                entry
                    .capabilities
                    .hook_management
                    .as_ref()
                    .map(|hook| hook.status.as_str())
                    .unwrap_or("unsupported")
            )),
        ];
        if entry.provider_id == "opencode" {
            lines.push(Line::from(format!(
                "{}: {}",
                app.t("showSubagents"),
                if app.settings_show_opencode_subagents {
                    app.t("enabled")
                } else {
                    app.t("disabled")
                }
            )));
        }
        lines
    } else {
        vec![Line::from(app.t("noProviders"))]
    };
    let summary = Paragraph::new(detail_lines)
        .block(section_block(
            app.t("agentManagementEnvironment"),
            false,
            theme,
        ))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(summary, right[0]);

    let actions = app.current_agent_management_actions();
    let mut action_lines = Vec::new();
    if actions.is_empty() {
        action_lines.push(Line::from(Span::styled(
            app.t("noAgentManagementActionForProvider"),
            Style::default().fg(theme.warning),
        )));
    } else {
        for (index, action) in actions.iter().enumerate() {
            let selected = index == app.agent_management_action_index;
            let label = match action.kind {
                AgentManagementActionKind::Toggle => format!(
                    "{} [{}]",
                    action.label,
                    enabled_label(app.language(), action.enabled.unwrap_or(false))
                ),
                AgentManagementActionKind::Action | AgentManagementActionKind::Detect => {
                    action.label.clone()
                }
            };
            action_lines.push(Line::from(Span::styled(
                format!(" {} ", label),
                agent_management_item_style(
                    selected,
                    app.agent_management_focus == AgentManagementFocus::Actions,
                    theme,
                ),
            )));
            if index + 1 < actions.len() {
                action_lines.push(Line::from(""));
            }
        }
        if let Some(action) = app.selected_agent_management_action() {
            action_lines.push(Line::from(""));
            action_lines.push(Line::from(Span::styled(
                action.description,
                Style::default().fg(theme.text_dim).bg(theme.surface),
            )));
        }
    }
    let action_panel = Paragraph::new(Text::from(action_lines))
        .block(section_block(
            app.t("agentManagementControls"),
            app.agent_management_focus == AgentManagementFocus::Actions,
            theme,
        ))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(action_panel, right[1]);

    let result_lines = if let Some(result) = &app.agent_management_result {
        result.lines.clone()
    } else {
        vec![app.t("agentsIdleHint").to_string()]
    };
    let result = Paragraph::new(result_lines.join("\n"))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .block(section_block(
            app.t("agentManagementResultTitle"),
            false,
            theme,
        ))
        .wrap(Wrap { trim: false });
    frame.render_widget(result, right[2]);
}

fn draw_settings_modal(frame: &mut Frame, app: &App, theme: &Theme) {
    let area = frame.area();
    let popup_area = centered_rect(78, 76, area);
    frame.render_widget(Clear, popup_area);
    frame.render_widget(modal_block(app.t("settings"), theme), popup_area);

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
        .block(section_block(app.t("editableSettings"), false, theme))
        .style(Style::default().fg(theme.text).bg(theme.surface))
        .wrap(Wrap { trim: true });
    frame.render_widget(settings, chunks[0]);

    let provider = app
        .provider_tabs()
        .get(app.selected_provider_tab)
        .cloned()
        .unwrap_or_else(|| app.t("all").to_string());
    let info = Paragraph::new(Text::from(vec![
        Line::from(vec![
            Span::styled(
                app.t("version"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}", env!("CARGO_PKG_VERSION"))),
        ]),
        Line::from(vec![
            Span::styled(
                app.t("providerFilter"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}", provider)),
        ]),
        Line::from(vec![
            Span::styled(
                app.t("configPath"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}", config_path)),
        ]),
    ]))
    .block(section_block(app.t("configPath"), false, theme))
    .style(Style::default().fg(theme.text).bg(theme.surface))
    .wrap(Wrap { trim: true });
    frame.render_widget(info, chunks[1]);

    let footer = Paragraph::new(app.t("tuiFooterClose"))
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
            config::UiLanguage::Zh => app.t("languageNativeZh"),
            config::UiLanguage::En => app.t("languageNativeEn"),
        }
        .to_string(),
        SettingsField::SessionsPerProvider => app.settings_sessions_per_provider.to_string(),
        SettingsField::SortProvidersBySessionCount => {
            enabled_label(app.language(), app.settings_sort_providers_by_session_count)
        }
        SettingsField::PrimaryAgents => settings_agent_label(app),
        SettingsField::Save => app.t("writeConfig").to_string(),
    };

    Line::from(vec![
        Span::styled(format!(" {} ", field.label(app.language())), label_style),
        Span::raw("  "),
        Span::styled(value, value_style),
    ])
}

fn settings_agent_label(app: &App) -> String {
    let Some(agent) = app.settings_agent_order.get(app.settings_agent_index) else {
        return app.t("allVisible").to_string();
    };
    let state = if app
        .settings_primary_agents
        .iter()
        .any(|provider| provider == agent)
    {
        app.t("visibleState")
    } else {
        app.t("foldedState")
    };
    format!("{}: {}", provider_label(agent), state)
}

fn enabled_label(language: config::UiLanguage, enabled: bool) -> String {
    if enabled {
        memorph::i18n::text(language, "enabled").to_string()
    } else {
        memorph::i18n::text(language, "disabled").to_string()
    }
}

fn agent_management_item_style(selected: bool, focused: bool, theme: &Theme) -> Style {
    if selected {
        if focused {
            highlighted_value_style(theme)
        } else {
            Style::default()
                .fg(theme.primary)
                .bg(theme.surface)
                .add_modifier(Modifier::BOLD)
        }
    } else {
        Style::default().fg(theme.text).bg(theme.surface)
    }
}

fn highlighted_value_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.primary)
        .bg(theme.highlight)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
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

#[cfg(test)]
mod tests {
    use ratatui::{backend::TestBackend, Terminal};

    use super::*;

    #[test]
    fn renders_compact_main_view_at_standard_terminal_size() {
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new().unwrap();

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }

    #[test]
    fn renders_main_view_and_overlay_at_narrow_terminal_size() {
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new().unwrap();

        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
        app.open_settings_modal();
        terminal.draw(|frame| draw(frame, &mut app)).unwrap();
    }
}
