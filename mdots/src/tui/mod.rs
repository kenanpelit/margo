use anyhow::Result;
use crossterm::event::KeyCode;
use std::time::{Duration, Instant};

mod app;
mod components;
mod events;
mod keybindings;
mod screens;
mod scroll;
pub mod terminal;
mod ui;

use app::{Action, App, Dialog, MessageLevel};
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

                // A confirm dialog tied to a pending mutating `Action`
                // takes priority over everything else: it must be
                // answered before any other key does anything, and the
                // answer is handled right here — not in
                // `App::handle_global_key` — because dispatching needs
                // `terminal`, which only this loop owns.
                if app.pending_action.is_some() {
                    match key.code {
                        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                            app.dialog = None;
                            if let Some(action) = app.pending_action.take() {
                                dispatch_action(&mut app, &mut terminal, action);
                            }
                        }
                        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                            app.dialog = None;
                            app.pending_action = None;
                        }
                        _ => {}
                    }
                    continue;
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
                            ScreenAction::Request(action) => {
                                let (title, message) = action.confirm_text();
                                app.dialog = Some(Dialog::Confirm {
                                    title,
                                    message,
                                    confirmed: false,
                                });
                                app.pending_action = Some(action);
                            }
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

/// Dispatch a confirmed [`Action`]: suspend the TUI, run the matching
/// CLI `commands::*` function verbatim against the real terminal, restore
/// the TUI (terminal-safe even on error — see
/// `terminal::with_suspended`), then surface the result as a status
/// message and force the current screen to reload so the change is
/// visible immediately.
fn dispatch_action(app: &mut App, terminal: &mut terminal::Tui, action: Action) {
    let paths = app.paths.clone();

    let result: Result<()> = match &action {
        Action::ToggleModule { name, enable } => {
            let name = name.clone();
            let enable = *enable;
            terminal::with_suspended(terminal, move || {
                if enable {
                    crate::commands::module::enable(&paths, &[name], false, false)
                } else {
                    crate::commands::module::disable(&paths, &name, false)
                }
            })
        }
        Action::RunSync { .. } => terminal::with_suspended(terminal, move || {
            crate::commands::sync::run(
                &paths,
                crate::commands::sync::SyncOptions {
                    dry_run: false,
                    prune: false,
                    force: false,
                    no_backup: false,
                    no_hooks: false,
                    force_dotfiles: false,
                    json: false,
                    auto_commit: false,
                },
            )
        }),
        Action::EnableService { name } => {
            let name = name.clone();
            terminal::with_suspended(terminal, move || {
                crate::commands::service::enable(&paths, &name, false)
            })
        }
        Action::DisableService { name } => {
            let name = name.clone();
            terminal::with_suspended(terminal, move || {
                crate::commands::service::disable(&paths, &name, false)
            })
        }
    };

    match result {
        Ok(()) => {
            let text = match &action {
                Action::ToggleModule { name, enable } if *enable => {
                    format!("Enabled module `{name}`.")
                }
                Action::ToggleModule { name, .. } => format!("Disabled module `{name}`."),
                Action::RunSync { .. } => "Sync complete.".to_string(),
                Action::EnableService { name } => format!("Enabled service profile `{name}`."),
                Action::DisableService { name } => format!("Disabled service profile `{name}`."),
            };
            app.show_message(text, MessageLevel::Success, 4);
        }
        Err(e) => {
            app.show_message(format!("{e:#}"), MessageLevel::Error, 6);
        }
    }

    app.current_screen.refresh();
}
