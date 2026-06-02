use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{backend::Backend, Terminal};
use std::time::Duration;

use super::app::{App, AppResult};

/// TUI event loop
pub fn run<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    let mut last_tick = std::time::Instant::now();
    let tick_rate = Duration::from_millis(250);

    loop {
        terminal.draw(|f| super::ui::draw(f, app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));

        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match handle_key_event(key, app) {
                    AppResult::Continue => {}
                    AppResult::Quit => {
                        return Ok(());
                    }
                    AppResult::Error(e) => {
                        return Err(e);
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.on_tick();
            last_tick = std::time::Instant::now();
        }
    }
}

fn handle_key_event(key: KeyEvent, app: &mut App) -> AppResult {
    if !matches!(
        key.kind,
        event::KeyEventKind::Press | event::KeyEventKind::Repeat
    ) {
        return AppResult::Continue;
    }

    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return AppResult::Quit;
    }

    if key.code == KeyCode::Char('?')
        && !app.action_modal_open
        && !app.workspace_modal_open
        && !app.settings_modal_open
        && key.modifiers.is_empty()
    {
        app.toggle_help();
        return AppResult::Continue;
    }

    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Char('?') => {
                app.show_help = false;
            }
            _ => {}
        }
        return AppResult::Continue;
    }

    if app.is_loading() {
        return AppResult::Continue;
    }

    app.handle_key(key)
}
