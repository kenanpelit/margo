use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::collections::HashMap;

use crate::config::{Config, ConfigPaths, PackageType};
use crate::package::PackageManager;
use crate::tui::screens::{ScreenAction, ScreenTrait};

/// A single row in the packages list.
#[derive(Clone)]
struct PackageRow {
    name: String,
    pkg_type: PackageType,
    installed: bool,
}

impl PackageRow {
    fn type_label(&self) -> &'static str {
        match self.pkg_type {
            PackageType::Native => "native",
            PackageType::Flatpak => "flatpak",
            PackageType::Nix => "nix",
        }
    }

    fn status_char(&self) -> &'static str {
        if self.installed {
            "✓"
        } else {
            "✗"
        }
    }

    fn status_color(&self) -> Color {
        if self.installed {
            Color::Green
        } else {
            Color::Yellow
        }
    }
}

/// Returns the indices of `packages` whose name or type label match `query`
/// (case-insensitive, substring). An empty query returns all indices.
fn filter_package_rows(packages: &[PackageRow], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..packages.len()).collect();
    }
    let q = query.to_lowercase();
    packages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.name.to_lowercase().contains(&q) || p.type_label().contains(q.as_str()))
        .map(|(i, _)| i)
        .collect()
}

#[derive(Clone, Default)]
pub struct PackagesScreenState {
    all_packages: Vec<PackageRow>,
    /// Indices into `all_packages` that are visible after the current filter.
    visible: Vec<usize>,
    list_state: ListState,
    filter_query: String,
    filter_active: bool,
    loaded: bool,
    load_error: Option<String>,
}

impl PackagesScreenState {
    fn load_data(&mut self, paths: &ConfigPaths, config: &Config) -> Result<()> {
        let pm = PackageManager::new(paths.clone());
        let declared = pm.get_declared_packages(config)?;

        // Get installed native packages — tolerate failure (e.g. no pacman)
        let installed_native: HashMap<String, String> =
            pm.get_installed_native_packages(config).unwrap_or_default();

        // Get installed flatpaks — tolerate failure
        let installed_flatpak: std::collections::HashSet<String> = pm
            .get_installed_flatpaks(config.flatpak_scope.as_arg())
            .unwrap_or_default()
            .into_iter()
            .collect();

        self.all_packages = declared
            .into_iter()
            .map(|pkg| {
                let installed = match pkg.package_type {
                    PackageType::Native => installed_native.contains_key(&pkg.name),
                    PackageType::Flatpak => installed_flatpak.contains(&pkg.name),
                    PackageType::Nix => false, // nix managed externally
                };
                PackageRow {
                    name: pkg.name,
                    pkg_type: pkg.package_type,
                    installed,
                }
            })
            .collect();

        // Sort: uninstalled first (false < true), then by name
        self.all_packages.sort_by(|a, b| {
            a.installed
                .cmp(&b.installed)
                .then_with(|| a.name.cmp(&b.name))
        });

        self.rebuild_visible();

        if !self.visible.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }

        self.load_error = None;
        self.loaded = true;
        Ok(())
    }

    fn rebuild_visible(&mut self) {
        self.visible = filter_package_rows(&self.all_packages, &self.filter_query);
        if let Some(sel) = self.list_state.selected() {
            if sel >= self.visible.len() {
                self.list_state.select(self.visible.len().checked_sub(1));
            }
        }
    }

    fn select_next(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let next = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.visible.len().saturating_sub(1)),
            None => 0,
        };
        self.list_state.select(Some(next));
    }

    fn select_prev(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let prev = match self.list_state.selected() {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(prev));
    }

    fn stats(&self) -> (usize, usize, usize) {
        let total = self.all_packages.len();
        let installed = self.all_packages.iter().filter(|p| p.installed).count();
        let missing = total - installed;
        (total, installed, missing)
    }
}

impl ScreenTrait for PackagesScreenState {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        if self.filter_active {
            match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.filter_active = false;
                }
                KeyCode::Backspace => {
                    self.filter_query.pop();
                    self.rebuild_visible();
                }
                KeyCode::Char(c) => {
                    self.filter_query.push(c);
                    self.rebuild_visible();
                    if !self.visible.is_empty() {
                        self.list_state.select(Some(0));
                    }
                }
                _ => {}
            }
            return Ok(None);
        }

        match key.code {
            KeyCode::Esc => return Ok(Some(ScreenAction::Back)),
            KeyCode::Char('r') => {
                self.loaded = false;
                self.load_error = None;
            }
            KeyCode::Char('/') => {
                self.filter_active = true;
                self.filter_query.clear();
                self.rebuild_visible();
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            _ => {}
        }
        Ok(None)
    }

    fn is_filtering(&self) -> bool {
        self.filter_active
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
        let constraints = if has_error {
            vec![Constraint::Length(3), Constraint::Min(0)]
        } else {
            vec![Constraint::Min(0)]
        };

        let outer_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(constraints)
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

        // Stats bar (top) + list (below)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(content_area);

        self.render_stats(frame, chunks[0]);
        self.render_list(frame, chunks[1]);

        Ok(())
    }
}

impl PackagesScreenState {
    fn render_stats(&self, frame: &mut Frame, area: Rect) {
        let (total, installed, missing) = self.stats();

        let line = Line::from(vec![
            Span::styled("Total: ", Style::default().fg(Color::Cyan)),
            Span::styled(
                format!("{}", total),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Installed: ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("{}", installed),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("Missing: ", Style::default().fg(Color::Yellow)),
            Span::styled(
                format!("{}", missing),
                Style::default()
                    .fg(if missing > 0 {
                        Color::Yellow
                    } else {
                        Color::Green
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]);

        let para = Paragraph::new(vec![Line::from(""), line]).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(crate::tui::theme::accent()))
                .title(" Package Summary "),
        );

        frame.render_widget(para, area);
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let filter_hint = if self.filter_active {
            format!(" [filter: {}▌]", self.filter_query)
        } else if !self.filter_query.is_empty() {
            format!(" [/{}/  {} shown]", self.filter_query, self.visible.len())
        } else {
            String::new()
        };

        let title = format!(" Packages{} ", filter_hint);

        let items: Vec<ListItem> = self
            .visible
            .iter()
            .filter_map(|&idx| self.all_packages.get(idx))
            .map(|row| {
                let line = Line::from(vec![
                    Span::styled(
                        format!("{} ", row.status_char()),
                        Style::default().fg(row.status_color()),
                    ),
                    Span::styled(
                        format!("{:<35}", &row.name),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!(" [{}]", row.type_label()),
                        Style::default().fg(Color::DarkGray),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(crate::tui::theme::accent()))
                    .title(title),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::PackageType;

    fn make_row(name: &str, pkg_type: PackageType, installed: bool) -> PackageRow {
        PackageRow {
            name: name.to_string(),
            pkg_type,
            installed,
        }
    }

    #[test]
    fn filter_empty_query_returns_all() {
        let rows = vec![
            make_row("vim", PackageType::Native, true),
            make_row("flatpak-app", PackageType::Flatpak, false),
        ];
        let result = filter_package_rows(&rows, "");
        assert_eq!(result, vec![0, 1]);
    }

    #[test]
    fn filter_matches_name_substring() {
        let rows = vec![
            make_row("vim", PackageType::Native, true),
            make_row("neovim", PackageType::Native, false),
            make_row("emacs", PackageType::Native, true),
        ];
        let result = filter_package_rows(&rows, "vim");
        assert_eq!(result, vec![0, 1]);
    }

    #[test]
    fn filter_matches_type_label() {
        let rows = vec![
            make_row("pkg1", PackageType::Native, true),
            make_row("com.example.App", PackageType::Flatpak, false),
        ];
        let result = filter_package_rows(&rows, "flatpak");
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn filter_case_insensitive() {
        let rows = vec![make_row("Firefox", PackageType::Native, true)];
        let result = filter_package_rows(&rows, "firefox");
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn filter_no_match_returns_empty() {
        let rows = vec![make_row("vim", PackageType::Native, true)];
        let result = filter_package_rows(&rows, "zzz");
        assert!(result.is_empty());
    }
}
