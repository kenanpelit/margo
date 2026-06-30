use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};
use std::collections::{HashMap, HashSet};

use crate::config::{Config, ConfigPaths, PackageType};
use crate::package::{Package, PackageManager};
use crate::tui::screens::{ScreenAction, ScreenTrait};
use crate::tui::scroll::{clamp_scroll, scroll_hint};

// ── Pure helpers (unit-tested below) ─────────────────────────────────────────

/// Count how many declared native/flatpak packages are already satisfied.
pub fn already_installed_count(
    declared: &[Package],
    installed_native: &HashMap<String, String>,
    installed_flatpak: &HashSet<String>,
) -> usize {
    declared
        .iter()
        .filter(|p| match p.package_type {
            PackageType::Native => installed_native.contains_key(&p.name),
            PackageType::Flatpak => installed_flatpak.contains(&p.name),
            PackageType::Nix => false,
        })
        .count()
}

// ── Data types ────────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct SyncPlan {
    native_to_install: Vec<String>,
    flatpak_to_install: Vec<String>,
    native_to_remove: Vec<String>,
    flatpak_to_remove: Vec<String>,
    already_ok: usize,
    total_declared: usize,
    nix_count: usize,
    enabled_modules: Vec<String>,
}

impl SyncPlan {
    fn is_up_to_date(&self) -> bool {
        self.native_to_install.is_empty()
            && self.flatpak_to_install.is_empty()
            && self.native_to_remove.is_empty()
            && self.flatpak_to_remove.is_empty()
    }
}

// ── Screen state ──────────────────────────────────────────────────────────────

#[derive(Clone, Default)]
pub struct SyncScreenState {
    plan: SyncPlan,
    /// Vertical scroll offset for the install list
    scroll: usize,
    loaded: bool,
    load_error: Option<String>,
}

impl SyncScreenState {
    fn load_data(&mut self, paths: &ConfigPaths, config: &Config) -> Result<()> {
        let pm = PackageManager::new(paths.clone());
        let declared = pm.get_declared_packages(config)?;

        let installed_native: HashMap<String, String> =
            pm.get_installed_native_packages(config).unwrap_or_default();

        let installed_flatpak: HashSet<String> = pm
            .get_installed_flatpaks(config.flatpak_scope.as_arg())
            .unwrap_or_default()
            .into_iter()
            .collect();

        let (native_to_install, flatpak_to_install) = crate::commands::sync::compute_installable(
            &declared,
            &installed_native,
            &installed_flatpak,
        );
        let already_ok = already_installed_count(&declared, &installed_native, &installed_flatpak);
        let nix_count = declared
            .iter()
            .filter(|p| matches!(p.package_type, PackageType::Nix))
            .count();

        // Prune side: packages tracked in state but no longer declared and still
        // installed (protected system packages excluded). Reuses the command-side
        // computation so the read-only preview matches what a real `--prune` does.
        let declared_names: HashSet<String> = declared.iter().map(|p| p.name.clone()).collect();
        let (native_to_remove, flatpak_to_remove) = crate::commands::sync::compute_prune_preview(
            paths,
            &declared_names,
            &installed_native,
            &installed_flatpak,
        );

        self.plan = SyncPlan {
            native_to_install,
            flatpak_to_install,
            native_to_remove,
            flatpak_to_remove,
            already_ok,
            total_declared: declared.len(),
            nix_count,
            enabled_modules: config.enabled_modules.clone(),
        };
        self.scroll = 0;
        self.load_error = None;
        self.loaded = true;
        Ok(())
    }

    fn scroll_down(&mut self) {
        self.scroll = self.scroll.saturating_add(1);
    }

    fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }
}

impl ScreenTrait for SyncScreenState {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match key.code {
            KeyCode::Esc => return Ok(Some(ScreenAction::Back)),
            KeyCode::Char('r') => {
                self.loaded = false;
                self.load_error = None;
            }
            KeyCode::Down | KeyCode::Char('j') => self.scroll_down(),
            KeyCode::Up | KeyCode::Char('k') => self.scroll_up(),
            _ => {}
        }
        Ok(None)
    }

    fn render(
        &mut self,
        paths: &ConfigPaths,
        config: &Config,
        frame: &mut Frame,
        area: Rect,
    ) -> Result<()> {
        if !self.loaded {
            if let Err(e) = self.load_data(paths, config) {
                self.load_error = Some(format!("{e:#}"));
                self.loaded = true;
            }
        }

        let has_error = self.load_error.is_some();
        let outer_constraints = if has_error {
            vec![Constraint::Length(3), Constraint::Min(0)]
        } else {
            vec![Constraint::Min(0)]
        };

        let outer_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(outer_constraints)
            .split(area);

        let content_area = if has_error {
            let err_msg = self.load_error.as_deref().unwrap_or("Unknown error");
            let para = Paragraph::new(err_msg).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red))
                    .title(" Error "),
            );
            frame.render_widget(para, outer_chunks[0]);
            outer_chunks[1]
        } else {
            outer_chunks[0]
        };

        // Vertical split: summary header (top) | plan details (bottom)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(0)])
            .split(content_area);

        self.render_summary(frame, chunks[0]);

        // Horizontal split for modules list and install list
        let detail_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(chunks[1]);

        self.render_modules(frame, detail_chunks[0]);
        self.render_install_list(frame, detail_chunks[1]);

        Ok(())
    }
}

impl SyncScreenState {
    fn render_summary(&self, frame: &mut Frame, area: Rect) {
        let status_text = if self.plan.is_up_to_date() {
            "System is up to date"
        } else {
            "Changes pending (install / prune)"
        };
        let status_color = if self.plan.is_up_to_date() {
            Color::Green
        } else {
            Color::Yellow
        };

        let lines = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("Status: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    status_text,
                    Style::default()
                        .fg(status_color)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("Declared packages: ", Style::default().fg(Color::Cyan)),
                Span::styled(
                    format!("{}", self.plan.total_declared),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("Already installed: ", Style::default().fg(Color::Green)),
                Span::styled(
                    format!("{}", self.plan.already_ok),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("To install (native): ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("{}", self.plan.native_to_install.len()),
                    Style::default()
                        .fg(if self.plan.native_to_install.is_empty() {
                            Color::Green
                        } else {
                            Color::Yellow
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("To install (flatpak): ", Style::default().fg(Color::Yellow)),
                Span::styled(
                    format!("{}", self.plan.flatpak_to_install.len()),
                    Style::default()
                        .fg(if self.plan.flatpak_to_install.is_empty() {
                            Color::Green
                        } else {
                            Color::Yellow
                        })
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![
                Span::styled("To prune (native): ", Style::default().fg(Color::Red)),
                Span::styled(
                    format!("{}", self.plan.native_to_remove.len()),
                    Style::default()
                        .fg(if self.plan.native_to_remove.is_empty() {
                            Color::Green
                        } else {
                            Color::Red
                        })
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled("To prune (flatpak): ", Style::default().fg(Color::Red)),
                Span::styled(
                    format!("{}", self.plan.flatpak_to_remove.len()),
                    Style::default()
                        .fg(if self.plan.flatpak_to_remove.is_empty() {
                            Color::Green
                        } else {
                            Color::Red
                        })
                        .add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![Span::styled(
                "[READ-ONLY PREVIEW — run 'mdots sync' to apply]",
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )]),
        ];

        let para = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(" Sync Plan Preview "),
        );
        frame.render_widget(para, area);
    }

    fn render_modules(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .plan
            .enabled_modules
            .iter()
            .map(|name| {
                ListItem::new(Line::from(vec![
                    Span::styled("● ", Style::default().fg(Color::Green)),
                    Span::raw(name.as_str()),
                ]))
            })
            .collect();

        let title = format!(" Enabled Modules ({}) ", self.plan.enabled_modules.len());

        let list = List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(title),
        );
        frame.render_widget(list, area);
    }

    fn render_install_list(&self, frame: &mut Frame, area: Rect) {
        let mut items: Vec<ListItem> = Vec::new();

        if self.plan.nix_count > 0 {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                format!(
                    "  {} nix package(s) managed by home-manager",
                    self.plan.nix_count
                ),
                Style::default().fg(Color::DarkGray),
            )])));
        }

        if self.plan.is_up_to_date() {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                "  All declared packages are installed.",
                Style::default().fg(Color::Green),
            )])));
        } else {
            if !self.plan.native_to_install.is_empty() {
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    "  Native packages to install:",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )])));
                for pkg in &self.plan.native_to_install {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled("  + ", Style::default().fg(Color::Yellow)),
                        Span::raw(pkg.as_str()),
                    ])));
                }
            }

            if !self.plan.flatpak_to_install.is_empty() {
                if !self.plan.native_to_install.is_empty() {
                    items.push(ListItem::new(Line::from("")));
                }
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    "  Flatpak packages to install:",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )])));
                for pkg in &self.plan.flatpak_to_install {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled("  + ", Style::default().fg(Color::Yellow)),
                        Span::raw(pkg.as_str()),
                    ])));
                }
            }

            let has_installs =
                !self.plan.native_to_install.is_empty() || !self.plan.flatpak_to_install.is_empty();

            if !self.plan.native_to_remove.is_empty() {
                if has_installs {
                    items.push(ListItem::new(Line::from("")));
                }
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    "  Native packages to prune:",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )])));
                for pkg in &self.plan.native_to_remove {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled("  - ", Style::default().fg(Color::Red)),
                        Span::raw(pkg.as_str()),
                    ])));
                }
            }

            if !self.plan.flatpak_to_remove.is_empty() {
                if has_installs || !self.plan.native_to_remove.is_empty() {
                    items.push(ListItem::new(Line::from("")));
                }
                items.push(ListItem::new(Line::from(vec![Span::styled(
                    "  Flatpak packages to prune:",
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                )])));
                for pkg in &self.plan.flatpak_to_remove {
                    items.push(ListItem::new(Line::from(vec![
                        Span::styled("  - ", Style::default().fg(Color::Red)),
                        Span::raw(pkg.as_str()),
                    ])));
                }
            }
        }

        let total_items = items.len();
        let visible_height = area.height.saturating_sub(2) as usize; // subtract borders
        let scroll = clamp_scroll(self.scroll, total_items, visible_height);

        // Apply scroll: drop the first `scroll` items
        let visible_items: Vec<ListItem> = items.into_iter().skip(scroll).collect();

        let title = format!(
            " Plan Details{} ",
            scroll_hint(scroll, total_items, visible_height)
        );

        let list = List::new(visible_items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(title),
        );

        frame.render_widget(list, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PackageType;

    fn pkg(name: &str, pkg_type: PackageType) -> Package {
        Package {
            name: name.to_string(),
            package_type: pkg_type,
        }
    }

    fn inst(names: &[&str]) -> HashMap<String, String> {
        names
            .iter()
            .map(|&n| (n.to_string(), "1.0".to_string()))
            .collect()
    }

    fn fpi(names: &[&str]) -> HashSet<String> {
        names.iter().map(|&n| n.to_string()).collect()
    }

    #[test]
    fn already_installed_count_correct() {
        let declared = vec![
            pkg("vim", PackageType::Native),
            pkg("htop", PackageType::Native),
            pkg("com.spotify.Client", PackageType::Flatpak),
        ];
        let native = inst(&["vim"]);
        let fp = fpi(&["com.spotify.Client"]);
        assert_eq!(already_installed_count(&declared, &native, &fp), 2);
    }
}
