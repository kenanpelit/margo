use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent};
use std::time::Duration;

/// Events that the TUI can handle
#[allow(dead_code)] // kept: complete terminal-event set; mouse/resize payloads not consumed yet
pub enum TuiEvent {
    /// Keyboard input
    Key(KeyEvent),

    /// Mouse input
    Mouse(MouseEvent),

    /// Terminal resize
    Resize(u16, u16),

    /// Timer tick (for animations, auto-refresh, etc.)
    Tick,
}

/// Event handler that polls for terminal events
pub struct EventHandler {
    tick_rate: Duration,
}

impl EventHandler {
    pub fn new(tick_rate: Duration) -> Self {
        Self { tick_rate }
    }

    /// Wait for the next event (blocking with timeout)
    pub fn next(&self) -> Result<TuiEvent> {
        // Poll with timeout for tick events
        if event::poll(self.tick_rate)? {
            match event::read()? {
                Event::Key(key) => Ok(TuiEvent::Key(key)),
                Event::Mouse(mouse) => Ok(TuiEvent::Mouse(mouse)),
                Event::Resize(w, h) => Ok(TuiEvent::Resize(w, h)),
                _ => Ok(TuiEvent::Tick),
            }
        } else {
            Ok(TuiEvent::Tick)
        }
    }
}

/// Helper to check if Ctrl+C was pressed
pub fn is_quit_key(key: &KeyEvent) -> bool {
    matches!(
        key,
        KeyEvent {
            code: KeyCode::Char('c'),
            modifiers: KeyModifiers::CONTROL,
            ..
        }
    )
}
