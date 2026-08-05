use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{draw_overlay_frame, Theme};
use memorph::{config::UiLanguage, i18n};

/// What kind of picker is open — determines data source and confirm behavior.
#[derive(Debug, Clone)]
pub enum PickerKind {
    Workspace,
    SwitchTarget,
    SwitchWorkspace,
    CompressTarget,
    AgentManagement,
    Settings,
}

/// Unified picker state: a list of labeled items with optional filter input.
#[derive(Debug, Clone)]
pub struct PickerState {
    pub kind: PickerKind,
    pub title: String,
    pub items: Vec<PickerItem>,
    pub selected: usize,
    pub filter: String,
    pub allow_filter: bool,
}

#[derive(Debug, Clone)]
pub struct PickerItem {
    pub id: String,
    pub label: String,
    pub description: String,
    pub enabled: Option<bool>,
}

impl PickerState {
    pub fn new(kind: PickerKind, title: impl Into<String>, items: Vec<PickerItem>) -> Self {
        Self {
            kind,
            title: title.into(),
            items,
            selected: 0,
            filter: String::new(),
            allow_filter: true,
        }
    }

    pub fn filtered_items(&self) -> Vec<(usize, &PickerItem)> {
        if self.filter.is_empty() {
            return self.items.iter().enumerate().collect();
        }
        let query = self.filter.to_lowercase();
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| {
                item.label.to_lowercase().contains(&query)
                    || item.description.to_lowercase().contains(&query)
            })
            .collect()
    }

    pub fn selected_item(&self) -> Option<&PickerItem> {
        let filtered = self.filtered_items();
        filtered
            .get(self.selected.min(filtered.len().saturating_sub(1)))
            .map(|(_, item)| *item)
    }

    pub fn move_up(&mut self) {
        let len = self.filtered_items().len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected + len - 1) % len;
    }

    pub fn move_down(&mut self) {
        let len = self.filtered_items().len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected + 1) % len;
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> PickerAction {
        match key.code {
            KeyCode::Esc => PickerAction::Cancel,
            KeyCode::Enter => PickerAction::Confirm,
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                PickerAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                PickerAction::Continue
            }
            KeyCode::Char(ch) if self.allow_filter && !ch.is_control() => {
                self.filter.push(ch);
                self.selected = 0;
                PickerAction::Continue
            }
            KeyCode::Backspace if self.allow_filter => {
                self.filter.pop();
                self.selected = 0;
                PickerAction::Continue
            }
            _ => PickerAction::Continue,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerAction {
    Continue,
    Confirm,
    Cancel,
}

/// Draw the picker overlay.
pub fn draw(
    frame: &mut Frame,
    state: &PickerState,
    language: UiLanguage,
    area: Rect,
    theme: &Theme,
) {
    let inner = draw_overlay_frame(frame, &state.title, 60, 60, area, theme);

    let constraints = if state.allow_filter {
        vec![
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ]
    } else {
        vec![
            Constraint::Length(0),
            Constraint::Length(0),
            Constraint::Min(1),
        ]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    // Filter input
    if state.allow_filter {
        let filter_line = Line::from(vec![
            Span::styled("/ ", Style::default().fg(theme.text_dim)),
            Span::styled(&state.filter, Style::default().fg(theme.text)),
            Span::styled("█", Style::default().fg(theme.primary)),
        ]);
        frame.render_widget(
            Paragraph::new(filter_line).style(Style::default().bg(theme.background)),
            chunks[0],
        );
    }

    // Item list
    let filtered = state.filtered_items();
    let visible_height = chunks[2].height as usize;
    let scroll_offset = if state.selected >= visible_height {
        state.selected - visible_height + 1
    } else {
        0
    };

    let lines: Vec<Line> = filtered
        .iter()
        .skip(scroll_offset)
        .take(visible_height)
        .enumerate()
        .map(|(view_idx, (_, item))| {
            let is_selected = view_idx + scroll_offset == state.selected;
            let prefix = if is_selected { "▸ " } else { "  " };
            let style = if is_selected {
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD)
                    .bg(theme.highlight)
            } else {
                Style::default().fg(theme.text).bg(theme.background)
            };

            let mut spans = vec![Span::styled(format!("{}{}", prefix, item.label), style)];
            if let Some(enabled) = item.enabled {
                let badge = format!(
                    " [{}]",
                    i18n::text(language, if enabled { "enabled" } else { "disabled" })
                );
                let badge_style = if enabled {
                    Style::default().fg(theme.success)
                } else {
                    Style::default().fg(theme.text_dim)
                };
                spans.push(Span::styled(badge, badge_style));
            }
            if !item.description.is_empty() {
                spans.push(Span::styled(
                    format!("  {}", item.description),
                    Style::default().fg(theme.text_dim),
                ));
            }
            Line::from(spans)
        })
        .collect();

    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme.background)),
        chunks[2],
    );
}
