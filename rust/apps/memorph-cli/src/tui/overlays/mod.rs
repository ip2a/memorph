pub mod confirm;
pub mod detail;
pub mod filter;
pub mod input;
pub mod picker;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Clear},
    Frame,
};

use super::theme::Theme;

/// Unified overlay state machine — at most one overlay is active at a time.
#[derive(Debug, Clone)]
pub enum Overlay {
    None,
    Workspace,
    Agents,
    Settings,
    Search,
    Detail,
    Action,
    Picker(picker::PickerState),
    Input(input::InputState),
    Confirm(confirm::ConfirmState),
    Help,
}

impl Default for Overlay {
    fn default() -> Self {
        Self::None
    }
}

impl Overlay {
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    pub fn is_some(&self) -> bool {
        !self.is_none()
    }
}

/// Draw a centered popup block with a title.
pub fn draw_overlay_frame(
    frame: &mut Frame,
    title: &str,
    width_pct: u16,
    height_pct: u16,
    area: Rect,
    theme: &Theme,
) -> Rect {
    let popup = centered_rect(width_pct, height_pct, area);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme.border_focused))
            .style(Style::default().bg(theme.background))
            .title(Span::styled(
                format!(" {} ", title),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
        popup,
    );
    popup.inner(Margin {
        horizontal: 2,
        vertical: 1,
    })
}

/// Calculate a centered rect within an area.
pub fn centered_rect(width_pct: u16, height_pct: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_pct) / 2),
            Constraint::Percentage(height_pct),
            Constraint::Percentage((100 - height_pct) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_pct) / 2),
            Constraint::Percentage(width_pct),
            Constraint::Percentage((100 - width_pct) / 2),
        ])
        .split(vertical[1])[1]
}
