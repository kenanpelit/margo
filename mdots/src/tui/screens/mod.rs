use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{layout::Rect, Frame};

use crate::config::{Config, ConfigPaths};
use crate::tui::app::Action;

/// Trait that all screens must implement
pub trait ScreenTrait {
    /// Handle keyboard input - returns actions to be applied to App
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>>;

    /// Render the screen
    fn render(
        &mut self,
        paths: &ConfigPaths,
        config: &Config,
        frame: &mut Frame,
        area: Rect,
    ) -> Result<()>;

    /// Called when screen becomes active (for loading data)
    fn on_activate(&mut self, _paths: &ConfigPaths, _config: &Config) -> Result<()> {
        Ok(())
    }

    /// Whether the screen is currently capturing raw text input (e.g. a `/`
    /// filter field). Global single-key shortcuts like `?` must not fire
    /// while this is true, since the keystroke belongs to the text field.
    /// Defaults to false; screens with a text-input mode override it.
    fn is_filtering(&self) -> bool {
        false
    }

    /// Force the screen to reload its data on next render. Called by the
    /// main loop after a dispatched [`Action`] mutates the system state the
    /// screen was displaying (e.g. a module's enabled flag, or the sync
    /// plan). Defaults to no-op; screens with a `loaded` reload-gate
    /// override it (the same effect as their own `r` refresh key).
    fn refresh(&mut self) {}
}

/// Actions that screens can request
#[allow(dead_code)] // kept: complete screen-action set; None/Refresh reserved for screens
pub enum ScreenAction {
    None,
    Back,
    Refresh,
    /// Request a confirmed, system-mutating [`Action`]. The main loop
    /// (`tui::run`) shows a `Dialog::Confirm` built from
    /// `Action::confirm_text` and only dispatches on y/Enter; screens never
    /// touch the terminal or call `commands::*` directly.
    Request(Action),
}

/// Enum of all possible screens
#[derive(Clone)]
pub enum Screen {
    Overview(OverviewScreenState),
    Modules(ModulesScreenState),
    Packages(PackagesScreenState),
    Sync(SyncScreenState),
    Services(ServicesScreenState),
}

impl Screen {
    /// Delegate to the appropriate screen implementation
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match self {
            Screen::Overview(s) => s.handle_key(key),
            Screen::Modules(s) => s.handle_key(key),
            Screen::Packages(s) => s.handle_key(key),
            Screen::Sync(s) => s.handle_key(key),
            Screen::Services(s) => s.handle_key(key),
        }
    }

    pub fn render(
        &mut self,
        paths: &ConfigPaths,
        config: &Config,
        frame: &mut Frame,
        area: Rect,
    ) -> Result<()> {
        match self {
            Screen::Overview(s) => s.render(paths, config, frame, area),
            Screen::Modules(s) => s.render(paths, config, frame, area),
            Screen::Packages(s) => s.render(paths, config, frame, area),
            Screen::Sync(s) => s.render(paths, config, frame, area),
            Screen::Services(s) => s.render(paths, config, frame, area),
        }
    }

    pub fn on_activate(&mut self, paths: &ConfigPaths, config: &Config) -> Result<()> {
        match self {
            Screen::Overview(s) => s.on_activate(paths, config),
            Screen::Modules(s) => s.on_activate(paths, config),
            Screen::Packages(s) => s.on_activate(paths, config),
            Screen::Sync(s) => s.on_activate(paths, config),
            Screen::Services(s) => s.on_activate(paths, config),
        }
    }

    /// Human-readable label for the active screen (used by the help overlay).
    pub fn name(&self) -> &'static str {
        match self {
            Screen::Overview(_) => "Overview",
            Screen::Modules(_) => "Modules",
            Screen::Packages(_) => "Packages",
            Screen::Sync(_) => "Sync",
            Screen::Services(_) => "Services",
        }
    }

    /// See [`ScreenTrait::is_filtering`].
    pub fn is_filtering(&self) -> bool {
        match self {
            Screen::Overview(s) => s.is_filtering(),
            Screen::Modules(s) => s.is_filtering(),
            Screen::Packages(s) => s.is_filtering(),
            Screen::Sync(s) => s.is_filtering(),
            Screen::Services(s) => s.is_filtering(),
        }
    }

    /// See [`ScreenTrait::refresh`].
    pub fn refresh(&mut self) {
        match self {
            Screen::Overview(s) => s.refresh(),
            Screen::Modules(s) => s.refresh(),
            Screen::Packages(s) => s.refresh(),
            Screen::Sync(s) => s.refresh(),
            Screen::Services(s) => s.refresh(),
        }
    }
}

// Re-export screen states
pub use modules::ModulesScreenState;
pub use overview::OverviewScreenState;
pub use packages::PackagesScreenState;
pub use services::ServicesScreenState;
pub use sync::SyncScreenState;

mod modules;
mod overview;
mod packages;
mod services;
mod sync;
