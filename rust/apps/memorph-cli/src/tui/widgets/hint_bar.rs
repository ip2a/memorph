use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::super::app::App;
use super::super::overlays::Overlay;
use super::super::theme::Theme;

struct Hint {
    key: &'static str,
    label_key: &'static str,
}

pub fn draw(frame: &mut Frame, app: &App, area: Rect, theme: &Theme) {
    let hints = if matches!(&app.overlay, Overlay::Picker(_)) {
        vec![
            Hint {
                key: "↑↓/jk",
                label_key: "select",
            },
            Hint {
                key: "Enter",
                label_key: "confirm",
            },
            Hint {
                key: "Esc",
                label_key: "cancel",
            },
        ]
    } else if matches!(&app.overlay, Overlay::Input(_)) {
        vec![
            Hint {
                key: "Enter",
                label_key: "confirm",
            },
            Hint {
                key: "Esc",
                label_key: "cancel",
            },
        ]
    } else if matches!(&app.overlay, Overlay::Confirm(_)) {
        vec![
            Hint {
                key: "y/Enter",
                label_key: "confirm",
            },
            Hint {
                key: "n/Esc",
                label_key: "cancel",
            },
        ]
    } else if matches!(&app.overlay, Overlay::Help) {
        vec![Hint {
            key: "Esc/?",
            label_key: "close",
        }]
    } else if matches!(&app.overlay, Overlay::Detail) {
        vec![
            Hint {
                key: "↑↓/jk",
                label_key: "scroll",
            },
            Hint {
                key: "Esc",
                label_key: "back",
            },
        ]
    } else if matches!(&app.overlay, Overlay::Search) {
        vec![
            Hint {
                key: "↑↓",
                label_key: "matchesTitle",
            },
            Hint {
                key: "←→",
                label_key: "scope",
            },
            Hint {
                key: "Enter",
                label_key: "apply",
            },
            Hint {
                key: "Esc",
                label_key: "cancel",
            },
        ]
    } else if matches!(&app.overlay, Overlay::Action) {
        vec![
            Hint {
                key: "↑↓",
                label_key: "type",
            },
            Hint {
                key: "←→",
                label_key: "visible",
            },
            Hint {
                key: "Enter",
                label_key: "confirm",
            },
            Hint {
                key: "Esc",
                label_key: "cancel",
            },
        ]
    } else if matches!(
        &app.overlay,
        Overlay::Workspace | Overlay::Agents | Overlay::Settings
    ) {
        vec![
            Hint {
                key: "↑↓",
                label_key: "selectAll",
            },
            Hint {
                key: "Enter",
                label_key: "confirm",
            },
            Hint {
                key: "Esc",
                label_key: "cancel",
            },
        ]
    } else {
        vec![
            Hint {
                key: "↑↓/jk",
                label_key: "navigation",
            },
            Hint {
                key: "←→/hl",
                label_key: "providers",
            },
            Hint {
                key: "Enter",
                label_key: "details",
            },
            Hint {
                key: "/",
                label_key: "filters",
            },
            Hint {
                key: "d/r/e",
                label_key: "edit",
            },
            Hint {
                key: "s/c",
                label_key: "switch",
            },
            Hint {
                key: "w/a/,",
                label_key: "manage",
            },
            Hint {
                key: "?",
                label_key: "help",
            },
            Hint {
                key: "q",
                label_key: "quit",
            },
        ]
    };

    let mut spans = Vec::new();
    for (index, hint) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("[{}]", hint.key),
            Style::default().fg(theme.primary),
        ));
        spans.push(Span::styled(
            app.t(hint.label_key),
            Style::default().fg(theme.text_dim),
        ));
    }

    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(theme.background)),
        area,
    );
}
