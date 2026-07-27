use ratatui::{
    layout::Rect,
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::tui::app::App;
use crate::tui::theme::Theme;

#[allow(dead_code)]
/// Draw help page
pub fn draw(frame: &mut Frame, _app: &App, area: Rect, theme: &Theme) {
    let help_text = format!(
        r#"memorph TUI - Help

Keyboard Shortcuts:
  Navigation:
    ↑ / k         Move selection up
    ↓ / j         Move selection down
    Enter         Select / Confirm
    Esc / q       Back / Cancel
    Tab           Switch focus

  Actions:
    s             Switch session to another provider
    e             Export session
    d             Delete session
    r             Rename session
    /             Search / filter
    i             Import session

  General:
    ? / h         Show this help
    Ctrl+C        Quit

Press any key to close this help.
"#
    );

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .title(" Help ")
                .borders(Borders::ALL)
                .border_style(theme.border_focused),
        )
        .wrap(Wrap { trim: true });

    frame.render_widget(help, area);
}
