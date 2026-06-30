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
    #[allow(dead_code)]
    pub fn show_message(&mut self, text: String, level: MessageLevel, duration_secs: u64) {
        self.status_message = Some(StatusMessage {
            text,
            level,
            expires_at: Instant::now() + std::time::Duration::from_secs(duration_secs),
        });
    }

    /// Handle global keybindings (works across all screens)
    pub fn handle_global_key(&mut self, key: KeyCode) -> Result<bool> {
        match key {
            KeyCode::Char('q') if self.dialog.is_none() => {
                self.should_quit = true;
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
