use anyhow::Result;
use crossterm::event::KeyCode;
use std::time::Instant;

use crate::config::{Config, ConfigPaths};
use crate::tui::screens::Screen;

/// Main TUI application state
pub struct App {
    /// Application configuration
    pub paths: ConfigPaths,
    pub config: Config,

    /// Current active screen
    pub current_screen: Screen,

    /// Navigation history (for back button)
    pub screen_history: Vec<Screen>,

    /// Sidebar state
    pub sidebar: SidebarState,

    /// Dialog state (if any dialog is open)
    pub dialog: Option<Dialog>,

    /// A confirmed-but-not-yet-dispatched [`Action`], stashed while its
    /// `Dialog::Confirm` is shown. Set by `tui::run` when a screen returns
    /// `ScreenAction::Request`; consumed (dispatched via suspend → the
    /// matching `commands::*` call → restore) on y/Enter, dropped on n/Esc.
    pub pending_action: Option<Action>,

    /// Whether the `?` keybinding help overlay is currently shown
    pub help_visible: bool,

    /// Global message/notification
    pub status_message: Option<StatusMessage>,

    /// Should the app quit?
    pub should_quit: bool,

    /// Force UI refresh flag
    pub needs_refresh: bool,
}

pub struct SidebarState {
    pub collapsed: bool,
    pub selected_index: usize,
    pub items: Vec<SidebarItem>,
}

pub struct SidebarItem {
    pub name: &'static str,
    pub icon: &'static str,
}

#[allow(dead_code)] // kept: complete TUI dialog-kind set; not every kind is constructed yet
pub enum Dialog {
    Confirm {
        title: String,
        message: String,
        confirmed: bool,
    },
    Error {
        title: String,
        message: String,
    },
    Info {
        title: String,
        message: String,
    },
}

pub struct StatusMessage {
    pub text: String,
    pub level: MessageLevel,
    pub expires_at: Instant,
}

/// A confirmed, system-mutating action a screen can request via
/// `ScreenAction::Request`. Screens only ever build and return one of
/// these — `tui::run` (which owns the `Terminal`) is the sole place that
/// turns a confirmed `Action` into a real `commands::*` call, via
/// suspend → run → restore (see `tui::terminal::with_suspended`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// Enable or disable a module — dispatches to
    /// `commands::module::{enable,disable}`.
    ToggleModule { name: String, enable: bool },
    /// Run a full package sync — dispatches to `commands::sync::run` with
    /// the standard (no extra flags) options, exactly like `mdots sync`.
    /// The plan counts are carried here (not re-derived at dispatch time)
    /// purely so the confirm message can be built from the `Action` alone.
    RunSync {
        native_install: usize,
        flatpak_install: usize,
        prune: usize,
    },
    /// Enable a service profile — dispatches to `commands::service::enable`.
    EnableService { name: String },
    /// Disable a service profile — dispatches to `commands::service::disable`.
    DisableService { name: String },
    /// Open a secret in `sops`/$EDITOR — dispatches to `commands::secrets::edit`.
    EditSecret { name: String },
    /// Decrypt declared secrets into place — dispatches to `commands::secrets::sync`.
    SyncSecrets,
    /// Run a single module hook — dispatches to `commands::hooks::run`. The
    /// `pre`/`disable` flags select which of the module's hooks to run; the
    /// `label` (e.g. `pre-install`) is carried only for the confirm/status
    /// text, so they can be built from the `Action` alone.
    RunHook {
        module: String,
        pre: bool,
        disable: bool,
        label: String,
    },
}

impl Action {
    /// Build the (title, message) pair the confirm dialog shows for this
    /// action. Pure — no I/O, no `App`/screen access — so it's directly
    /// unit-testable.
    pub fn confirm_text(&self) -> (String, String) {
        match self {
            Action::ToggleModule { name, enable } => toggle_module_confirm(name, *enable),
            Action::RunSync {
                native_install,
                flatpak_install,
                prune,
            } => run_sync_confirm(*native_install, *flatpak_install, *prune),
            Action::EnableService { name } => (
                "Enable service profile".to_string(),
                format!("Enable service profile `{name}`? This starts its services now."),
            ),
            Action::DisableService { name } => (
                "Disable service profile".to_string(),
                format!("Disable service profile `{name}`? This stops its services now."),
            ),
            Action::EditSecret { name } => (
                "Edit secret".to_string(),
                format!("Edit secret `{name}` with sops? This opens your editor."),
            ),
            Action::SyncSecrets => (
                "Sync secrets".to_string(),
                "Decrypt all declared secrets into place (0600)?".to_string(),
            ),
            Action::RunHook { module, label, .. } => (
                "Run hook".to_string(),
                format!("Run the `{label}` hook for module `{module}` now?"),
            ),
        }
    }
}

/// Pure confirm-text builder for [`Action::ToggleModule`].
pub fn toggle_module_confirm(name: &str, enable: bool) -> (String, String) {
    if enable {
        (
            "Enable module".to_string(),
            format!(
                "Enable module `{name}`? This may run a sync afterward to install its packages."
            ),
        )
    } else {
        (
            "Disable module".to_string(),
            format!(
                "Disable module `{name}`? Its packages stay installed until you run sync --prune."
            ),
        )
    }
}

/// Pure confirm-text builder for [`Action::RunSync`], summarizing the
/// previewed plan counts (matches the Sync screen's preview).
pub fn run_sync_confirm(
    native_install: usize,
    flatpak_install: usize,
    prune: usize,
) -> (String, String) {
    (
        "Run sync".to_string(),
        format!(
            "Run sync? +{native_install} native, +{flatpak_install} flatpak to install, \u{2212}{prune} to prune."
        ),
    )
}

/// Decide the new `enable` flag for a module-toggle request: the opposite
/// of its current enabled state.
pub fn module_toggle_enable(currently_enabled: bool) -> bool {
    !currently_enabled
}

#[allow(dead_code)] // kept: complete status-severity set; not every level is emitted yet
#[derive(Clone, Copy)]
pub enum MessageLevel {
    Info,
    Success,
    Warning,
    Error,
}

impl App {
    pub fn new(paths: ConfigPaths, config: Config) -> Result<Self> {
        Ok(Self {
            paths,
            config,
            current_screen: Screen::Overview(Default::default()),
            screen_history: Vec::new(),
            sidebar: SidebarState::new(),
            dialog: None,
            pending_action: None,
            help_visible: false,
            status_message: None,
            should_quit: false,
            needs_refresh: true,
        })
    }

    /// Navigate to a new screen
    pub fn navigate_to(&mut self, mut screen: Screen) {
        // Activate the new screen before switching
        if let Err(e) = screen.on_activate(&self.paths, &self.config) {
            eprintln!("Error activating screen: {}", e);
        }

        let current = std::mem::replace(&mut self.current_screen, screen);
        self.screen_history.push(current);

        // Auto-collapse sidebar so screen can receive key input
        self.sidebar.collapsed = true;
        self.needs_refresh = true;
    }

    /// Go back to previous screen
    pub fn navigate_back(&mut self) {
        if let Some(screen) = self.screen_history.pop() {
            self.current_screen = screen;
            self.needs_refresh = true;
        }
    }

    /// Show a status message for N seconds
    pub fn show_message(&mut self, text: String, level: MessageLevel, duration_secs: u64) {
        self.status_message = Some(StatusMessage {
            text,
            level,
            expires_at: Instant::now() + std::time::Duration::from_secs(duration_secs),
        });
    }

    /// Handle global keybindings (works across all screens)
    pub fn handle_global_key(&mut self, key: KeyCode) -> Result<bool> {
        // The help overlay is modal: while it's open it swallows every key
        // except the two that close it, so nothing scrolls/navigates
        // invisibly underneath it.
        if self.help_visible {
            if matches!(key, KeyCode::Char('?') | KeyCode::Esc) {
                self.help_visible = false;
            }
            return Ok(true);
        }

        match key {
            KeyCode::Char('q') if self.dialog.is_none() => {
                self.should_quit = true;
                return Ok(true);
            }
            // Don't let `?` hijack a keystroke meant for an active filter
            // text field (e.g. searching for a package literally named
            // "what?").
            KeyCode::Char('?') if self.dialog.is_none() && !self.current_screen.is_filtering() => {
                self.help_visible = true;
                return Ok(true);
            }
            KeyCode::Char('m') => {
                self.sidebar.toggle();
                return Ok(true);
            }
            KeyCode::Esc => {
                if self.dialog.is_some() {
                    self.dialog = None;
                    return Ok(true);
                } else if !self.sidebar.collapsed {
                    // If sidebar is open, collapse it instead of going back
                    self.sidebar.collapsed = true;
                    return Ok(true);
                }
                // Don't handle Esc globally - let screens handle it for their own back/cancel logic
                return Ok(false);
            }
            // Only handle Tab/BackTab/Arrow keys when sidebar is expanded (focused)
            KeyCode::Tab | KeyCode::Down if !self.sidebar.collapsed => {
                self.sidebar.select_next();
                return Ok(true);
            }
            KeyCode::BackTab | KeyCode::Up if !self.sidebar.collapsed => {
                self.sidebar.select_prev();
                return Ok(true);
            }
            // Only handle Enter when sidebar is expanded (for navigation)
            KeyCode::Enter if !self.sidebar.collapsed => {
                // Navigate to selected sidebar item
                let screen_index = self.sidebar.selected_index;
                let new_screen = match screen_index {
                    0 => Screen::Overview(Default::default()),
                    1 => Screen::Modules(Default::default()),
                    2 => Screen::Packages(Default::default()),
                    3 => Screen::Sync(Default::default()),
                    4 => Screen::Services(Default::default()),
                    5 => Screen::Secrets(Default::default()),
                    6 => Screen::Hooks(Default::default()),
                    _ => return Ok(false),
                };
                self.navigate_to(new_screen);
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }
}

impl SidebarState {
    pub fn new() -> Self {
        Self {
            collapsed: false,
            selected_index: 0,
            items: vec![
                SidebarItem {
                    name: "Overview",
                    icon: "📊",
                },
                SidebarItem {
                    name: "Modules",
                    icon: "📦",
                },
                SidebarItem {
                    name: "Packages",
                    icon: "🔍",
                },
                SidebarItem {
                    name: "Sync",
                    icon: "🔄",
                },
                SidebarItem {
                    name: "Services",
                    icon: "🧩",
                },
                SidebarItem {
                    name: "Secrets",
                    icon: "🔐",
                },
                SidebarItem {
                    name: "Hooks",
                    icon: "🪝",
                },
            ],
        }
    }

    pub fn toggle(&mut self) {
        self.collapsed = !self.collapsed;
    }

    pub fn select_next(&mut self) {
        self.selected_index = (self.selected_index + 1) % self.items.len();
    }

    pub fn select_prev(&mut self) {
        if self.selected_index == 0 {
            self.selected_index = self.items.len() - 1;
        } else {
            self.selected_index -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_toggle_enable_flips_disabled_to_enabled() {
        assert!(module_toggle_enable(false));
    }

    #[test]
    fn module_toggle_enable_flips_enabled_to_disabled() {
        assert!(!module_toggle_enable(true));
    }

    #[test]
    fn toggle_module_confirm_enable_names_module_and_mentions_sync() {
        let (title, message) = toggle_module_confirm("zsh", true);
        assert_eq!(title, "Enable module");
        assert!(message.contains("zsh"));
        assert!(message.to_lowercase().contains("sync"));
    }

    #[test]
    fn toggle_module_confirm_disable_names_module_and_mentions_prune() {
        let (title, message) = toggle_module_confirm("zsh", false);
        assert_eq!(title, "Disable module");
        assert!(message.contains("zsh"));
        assert!(message.contains("prune"));
    }

    #[test]
    fn run_sync_confirm_formats_plan_counts() {
        let (title, message) = run_sync_confirm(3, 1, 2);
        assert_eq!(title, "Run sync");
        assert!(message.contains("+3 native"));
        assert!(message.contains("+1 flatpak"));
        assert!(message.contains('2'));
    }

    #[test]
    fn run_sync_confirm_zero_counts_still_formats() {
        let (_, message) = run_sync_confirm(0, 0, 0);
        assert!(message.contains("+0 native"));
        assert!(message.contains("+0 flatpak"));
    }

    #[test]
    fn action_confirm_text_delegates_to_toggle_module_confirm() {
        let action = Action::ToggleModule {
            name: "audio".to_string(),
            enable: true,
        };
        assert_eq!(action.confirm_text(), toggle_module_confirm("audio", true));
    }

    #[test]
    fn action_confirm_text_delegates_to_run_sync_confirm() {
        let action = Action::RunSync {
            native_install: 5,
            flatpak_install: 0,
            prune: 1,
        };
        assert_eq!(action.confirm_text(), run_sync_confirm(5, 0, 1));
    }
}
