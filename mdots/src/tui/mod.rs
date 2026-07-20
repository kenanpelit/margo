use anyhow::Result;
use crossterm::event::KeyCode;
use std::time::{Duration, Instant};

mod app;
mod components;
mod events;
mod job;
mod keybindings;
mod layout;
mod palette;
#[cfg(test)]
mod render_tests;
mod screens;
mod scroll;
pub mod terminal;
mod theme;
pub(crate) mod ui;

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
        // Only repaint when something actually changed. Every input event
        // sets the flag below, so this is invisible in use — but it takes an
        // idle TUI from a full-screen redraw four times a second (the tick
        // rate) down to zero work.
        if app.needs_refresh {
            terminal.draw(|frame| {
                if let Err(e) = ui::render(&mut app, frame) {
                    eprintln!("Render error: {}", e);
                }
            })?;
            app.needs_refresh = false;
        }

        // Handle events
        match events.next()? {
            TuiEvent::Key(key) => {
                // Any keystroke can change what's on screen; the handlers
                // below don't each have to remember to say so.
                app.needs_refresh = true;

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
                let handled = app.handle_global_key(key)?;

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
            TuiEvent::Mouse(mouse) => {
                use crossterm::event::{MouseButton, MouseEventKind};
                app.needs_refresh = true;
                match mouse.kind {
                    MouseEventKind::ScrollDown => {
                        if let Err(e) = app.handle_scroll(true) {
                            app.show_message(format!("{e:#}"), MessageLevel::Error, 4);
                        }
                    }
                    MouseEventKind::ScrollUp => {
                        if let Err(e) = app.handle_scroll(false) {
                            app.show_message(format!("{e:#}"), MessageLevel::Error, 4);
                        }
                    }
                    MouseEventKind::Down(MouseButton::Left) => {
                        app.handle_left_click(mouse.column, mouse.row);
                    }
                    _ => {}
                }
            }
            TuiEvent::Resize(_, _) => {
                // ratatui re-derives the layout from the new size on the next
                // draw; we just have to ask for one.
                app.needs_refresh = true;
            }
            TuiEvent::Tick => {
                // Check if status message expired
                if let Some(msg) = &app.status_message {
                    if msg.expires_at < Instant::now() {
                        app.status_message = None;
                        app.needs_refresh = true;
                    }
                }
                // A screen waiting on a background probe has no input event
                // coming to wake it, so keep drawing while work is in flight
                // — that's what polls the worker and picks up its result.
                if app.current_screen.is_busy() {
                    app.needs_refresh = true;
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
        Action::ToggleModules { names, enable } => {
            let names = names.clone();
            let enable = *enable;
            terminal::with_suspended(terminal, move || {
                if enable {
                    // `module::enable` already takes a slice, so the whole
                    // batch is one call (and one sync afterwards).
                    crate::commands::module::enable(&paths, &names, false, false)
                } else {
                    // `module::disable` is single-module only. Keep going on
                    // failure so one bad module doesn't strand the rest, and
                    // report the first error at the end.
                    let mut first_error = None;
                    for name in &names {
                        if let Err(e) = crate::commands::module::disable(&paths, name, false) {
                            first_error.get_or_insert(e);
                        }
                    }
                    match first_error {
                        Some(e) => Err(e),
                        None => Ok(()),
                    }
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
        Action::EditSecret { name } => {
            let name = name.clone();
            terminal::with_suspended(terminal, move || {
                crate::commands::secrets::edit(&paths, &name)
            })
        }
        Action::SyncSecrets => terminal::with_suspended(terminal, move || {
            crate::commands::secrets::sync(&paths, false, false, false)
        }),
        Action::RunHook {
            module,
            pre,
            disable,
            ..
        } => {
            let module = module.clone();
            let pre = *pre;
            let disable = *disable;
            terminal::with_suspended(terminal, move || {
                crate::commands::hooks::run(&paths, &module, pre, disable)
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
                Action::ToggleModules { names, enable } => format!(
                    "{} {} modules.",
                    if *enable { "Enabled" } else { "Disabled" },
                    names.len()
                ),
                Action::RunSync { .. } => "Sync complete.".to_string(),
                Action::EnableService { name } => format!("Enabled service profile `{name}`."),
                Action::DisableService { name } => format!("Disabled service profile `{name}`."),
                Action::EditSecret { name } => format!("Edited secret `{name}`."),
                Action::SyncSecrets => "Secrets synced.".to_string(),
                Action::RunHook { module, label, .. } => {
                    format!("Ran `{label}` hook for `{module}`.")
                }
            };
            app.show_message(text, MessageLevel::Success, 4);
        }
        Err(e) => {
            app.show_message(format!("{e:#}"), MessageLevel::Error, 6);
        }
    }

    // `resume` cleared the terminal behind us, so the next iteration must
    // draw whatever the flag would otherwise have let it skip.
    app.current_screen.refresh();
    app.needs_refresh = true;
}
