use anyhow::Result;
use std::time::{Duration, Instant};

mod app;
mod components;
mod events;
mod screens;
pub mod terminal;
mod ui;

use app::App;
use events::{is_quit_key, EventHandler, TuiEvent};
use screens::ScreenAction;

use crate::config::{load_config, ConfigPaths};

pub fn run(paths: ConfigPaths, mut terminal: terminal::Tui) -> Result<()> {
    // Load config
    let config = load_config(&paths)?;

    // Create app state
    let mut app = App::new(paths, config)?;

    // Create event handler (250ms tick rate)
    let events = EventHandler::new(Duration::from_millis(250));

    // Main event loop
    loop {
        // Render UI
        terminal.draw(|frame| {
            if let Err(e) = ui::render(&mut app, frame) {
                eprintln!("Render error: {}", e);
            }
        })?;

        // Handle events
        match events.next()? {
            TuiEvent::Key(key) => {
                // Check for quit
                if is_quit_key(&key) {
                    break;
                }

                // Handle global keys first
                let handled = app.handle_global_key(key.code)?;

                // If not handled globally, pass to current screen
                if !handled {
                    if let Some(action) = app.current_screen.handle_key(key)? {
                        match action {
                            ScreenAction::Back => app.navigate_back(),
                            ScreenAction::Refresh => app.needs_refresh = true,
                            ScreenAction::None => {}
                        }
                    }
                }
            }
            TuiEvent::Mouse(_) => {
                // Mouse support (optional for MVP)
            }
            TuiEvent::Resize(_, _) => {
                // Terminal resized - ratatui handles this automatically
            }
            TuiEvent::Tick => {
                // Check if status message expired
                if let Some(msg) = &app.status_message {
                    if msg.expires_at < Instant::now() {
                        app.status_message = None;
                    }
                }
            }
        }

        // Check if we should quit
        if app.should_quit {
            break;
        }
    }

    Ok(())
}
