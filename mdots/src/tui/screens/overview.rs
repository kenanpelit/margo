use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use std::path::PathBuf;
use walkdir::WalkDir;

use crate::config::{Config, ConfigPaths};
use crate::tui::screens::{ScreenAction, ScreenTrait};
use crate::tui::scroll::{clamp_scroll, scroll_hint};

#[derive(Clone)]
pub struct OverviewScreenState {
    enabled_modules_count: usize,
    total_modules_count: usize,
    declared_packages_count: usize,
    installed_packages_count: usize,
    config_tree: Vec<String>,
    /// Vertical scroll offset for the config tree list
    scroll: usize,
    hostname: String,
    auto_prune: bool,
    flatpak_scope: String,
    backup_tool: String,
    loaded: bool,
}

impl Default for OverviewScreenState {
    fn default() -> Self {
        Self {
            enabled_modules_count: 0,
            total_modules_count: 0,
            declared_packages_count: 0,
            installed_packages_count: 0,
            config_tree: Vec::new(),
            scroll: 0,
            hostname: String::new(),
            auto_prune: false,
            flatpak_scope: String::from("user"),
            backup_tool: String::from("none"),
            loaded: false,
        }
    }
}

impl ScreenTrait for OverviewScreenState {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match key.code {
            KeyCode::Char('r') => {
                // Mark as not loaded so it will refresh on next render
                self.loaded = false;
                Ok(None)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.scroll = self.scroll.saturating_add(1);
                Ok(None)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.scroll = self.scroll.saturating_sub(1);
                Ok(None)
            }
            KeyCode::Esc => Ok(Some(ScreenAction::Back)),
            _ => Ok(None),
        }
    }

    fn render(
        &mut self,
        paths: &ConfigPaths,
        config: &Config,
        frame: &mut Frame,
        area: Rect,
    ) -> Result<()> {
        // Load data on first render if not already loaded
        if !self.loaded {
            if let Err(e) = self.load_data(paths, config) {
                eprintln!("Error loading overview data: {}", e);
            }
        }

        // Split into two sections: System Info (top) and Config Tree (bottom)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(15), // System info and quick stats
                Constraint::Min(0),     // Config tree
            ])
            .split(area);

        // Render system info and quick stats side by side
        let info_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(chunks[0]);

        self.render_system_info(frame, info_chunks[0])?;
        self.render_quick_stats(frame, info_chunks[1])?;

        // Render config tree
        self.render_config_tree(frame, chunks[1])?;

        Ok(())
    }
}

impl OverviewScreenState {
    fn load_data(&mut self, paths: &ConfigPaths, config: &crate::config::Config) -> Result<()> {
        // Load hostname
        self.hostname = config.host.clone();

        // Load module counts
        self.enabled_modules_count = config.enabled_modules.len();

        // Count total modules
        let modules_dir = paths.config_dir.join("modules");
        if modules_dir.exists() {
            self.total_modules_count = std::fs::read_dir(&modules_dir)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    let path = e.path();
                    path.is_file() && path.extension().is_some_and(|ext| ext == "yaml")
                        || path.is_dir()
                })
                .count();
        }

        // Load package counts
        self.declared_packages_count = config.packages.len();

        // Try to get installed package count (this might fail in some environments)
        self.installed_packages_count = self.get_installed_package_count().unwrap_or(0);

        // Load config settings
        self.auto_prune = config.auto_prune;
        self.flatpak_scope = match config.flatpak_scope {
            crate::config::FlatpakScope::User => "user".to_string(),
            crate::config::FlatpakScope::System => "system".to_string(),
        };

        // Determine backup tool
        self.backup_tool = config
            .system_backups
            .tool
            .clone()
            .unwrap_or_else(|| "none".to_string());

        // Build config tree
        self.config_tree = self.build_config_tree(&paths.config_dir)?;
        self.scroll = 0;

        // Mark as loaded
        self.loaded = true;

        Ok(())
    }

    fn get_installed_package_count(&self) -> Result<usize> {
        // Try to count installed native packages via backend
        if let Ok(config) = crate::config::load_config(&crate::config::ConfigPaths::new()?) {
            if let Ok(backend) = crate::backend::create_backend(&config) {
                if let Ok(installed) = backend.get_installed_packages() {
                    return Ok(installed.len());
                }
            }
        }
        Ok(0)
    }

    fn build_config_tree(&self, config_dir: &PathBuf) -> Result<Vec<String>> {
        let mut tree = Vec::new();
        let dir_name = config_dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("mdots");
        tree.push(format!("📁 {}/", dir_name));

        // Walk the directory structure
        let mut entries: Vec<_> = WalkDir::new(config_dir)
            .max_depth(3)
            .into_iter()
            .filter_map(|e| e.ok())
            .collect();

        entries.sort_by(|a, b| a.path().cmp(b.path()));

        for entry in entries {
            if entry.path() == config_dir {
                continue;
            }

            let depth = entry.depth();
            let indent = "  ".repeat(depth);
            let name = entry.file_name().to_string_lossy();

            if entry.file_type().is_dir() {
                tree.push(format!("{}📂 {}/", indent, name));
            } else {
                let icon = if name.ends_with(".yaml") || name.ends_with(".yml") {
                    "📄"
                } else if name.ends_with(".sh") {
                    "📜"
                } else {
                    "📋"
                };
                tree.push(format!("{}{} {}", indent, icon, name));
            }
        }

        Ok(tree)
    }

    fn render_system_info(&self, frame: &mut Frame, area: Rect) -> Result<()> {
        let info_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Hostname: ", Style::default().fg(Color::Cyan)),
                Span::raw(&self.hostname),
            ]),
            Line::from(vec![
                Span::styled("Auto Prune: ", Style::default().fg(Color::Cyan)),
                Span::raw(if self.auto_prune {
                    "Enabled"
                } else {
                    "Disabled"
                }),
            ]),
            Line::from(vec![
                Span::styled("Flatpak Scope: ", Style::default().fg(Color::Cyan)),
                Span::raw(&self.flatpak_scope),
            ]),
            Line::from(vec![
                Span::styled("Backup Tool: ", Style::default().fg(Color::Cyan)),
                Span::raw(&self.backup_tool),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    "[r]",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" Refresh data"),
            ]),
        ];

        let paragraph = Paragraph::new(info_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(" System Information "),
        );

        frame.render_widget(paragraph, area);
        Ok(())
    }

    fn render_quick_stats(&self, frame: &mut Frame, area: Rect) -> Result<()> {
        let stats_lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Enabled Modules: ", Style::default().fg(Color::Green)),
                Span::styled(
                    format!(
                        "{}/{}",
                        self.enabled_modules_count, self.total_modules_count
                    ),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Declared Packages: ", Style::default().fg(Color::Green)),
                Span::styled(
                    format!("{}", self.declared_packages_count),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Installed Packages: ", Style::default().fg(Color::Green)),
                Span::styled(
                    format!("{}", self.installed_packages_count),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]),
        ];

        let paragraph = Paragraph::new(stats_lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(" Quick Stats "),
        );

        frame.render_widget(paragraph, area);
        Ok(())
    }

    fn render_config_tree(&self, frame: &mut Frame, area: Rect) -> Result<()> {
        let total = self.config_tree.len();
        let visible_height = area.height.saturating_sub(2) as usize; // subtract borders
        let scroll = clamp_scroll(self.scroll, total, visible_height);

        let items: Vec<ListItem> = self
            .config_tree
            .iter()
            .skip(scroll)
            .map(|line| ListItem::new(Line::from(line.as_str())))
            .collect();

        let title = format!(
            " Configuration Structure{} ",
            scroll_hint(scroll, total, visible_height)
        );

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(title),
        );

        frame.render_widget(list, area);
        Ok(())
    }
}
