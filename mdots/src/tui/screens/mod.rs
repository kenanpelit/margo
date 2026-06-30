use anyhow::Result;
use crossterm::event::KeyEvent;
use ratatui::{layout::Rect, Frame};

use crate::config::{Config, ConfigPaths};

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
}

/// Actions that screens can request
#[allow(dead_code)] // kept: complete screen-action set; None/Refresh reserved for screens
pub enum ScreenAction {
    None,
    Back,
    Refresh,
}

/// Enum of all possible screens
#[derive(Clone)]
pub enum Screen {
    Overview(OverviewScreenState),
    Modules(ModulesScreenState),
    Packages(PackagesScreenState),
    Sync(SyncScreenState),
}

impl Screen {
    /// Delegate to the appropriate screen implementation
    pub fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match self {
            Screen::Overview(s) => s.handle_key(key),
            Screen::Modules(s) => s.handle_key(key),
            Screen::Packages(s) => s.handle_key(key),
            Screen::Sync(s) => s.handle_key(key),
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
        }
    }

    pub fn on_activate(&mut self, paths: &ConfigPaths, config: &Config) -> Result<()> {
        match self {
            Screen::Overview(s) => s.on_activate(paths, config),
            Screen::Modules(s) => s.on_activate(paths, config),
            Screen::Packages(s) => s.on_activate(paths, config),
            Screen::Sync(s) => s.on_activate(paths, config),
        }
    }

    /// Human-readable label for the active screen (used by the help overlay).
    pub fn name(&self) -> &'static str {
        match self {
            Screen::Overview(_) => "Overview",
            Screen::Modules(_) => "Modules",
            Screen::Packages(_) => "Packages",
            Screen::Sync(_) => "Sync",
        }
    }

    /// See [`ScreenTrait::is_filtering`].
    pub fn is_filtering(&self) -> bool {
        match self {
            Screen::Overview(s) => s.is_filtering(),
            Screen::Modules(s) => s.is_filtering(),
            Screen::Packages(s) => s.is_filtering(),
            Screen::Sync(s) => s.is_filtering(),
        }
    }
}

// Re-export screen states
pub use modules::ModulesScreenState;
pub use overview::OverviewScreenState;
pub use packages::PackagesScreenState;
pub use sync::SyncScreenState;

mod modules;
mod overview;
mod packages;
mod sync;
