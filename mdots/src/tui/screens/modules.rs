use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::config::{Config, ConfigPaths};
use crate::module::ModuleManager;
use crate::tui::app::{module_toggle_enable, Action};
use crate::tui::screens::{ScreenAction, ScreenTrait};

/// A row of data about one module, kept separately from the heavy `ModuleInfo`.
#[derive(Clone)]
struct ModuleEntry {
    name: String,
    description: String,
    package_count: usize,
    enabled: bool,
}

/// Return the indices of entries whose name or description match `query`
/// (case-insensitive, substring).  An empty query returns all indices.
fn filter_module_entries(entries: &[ModuleEntry], query: &str) -> Vec<usize> {
    if query.is_empty() {
        return (0..entries.len()).collect();
    }
    let q = query.to_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            e.name.to_lowercase().contains(&q) || e.description.to_lowercase().contains(&q)
        })
        .map(|(i, _)| i)
        .collect()
}

#[derive(Clone, Default)]
pub struct ModulesScreenState {
    modules: Vec<ModuleEntry>,
    /// Indices into `modules` that are currently visible (after optional filter)
    visible: Vec<usize>,
    list_state: ListState,
    filter_query: String,
    filter_active: bool,
    loaded: bool,
    load_error: Option<String>,
}

impl ModulesScreenState {
    fn load_data(&mut self, paths: &ConfigPaths, config: &Config) -> Result<()> {
        let manager = ModuleManager::new(paths.clone());
        let raw = manager.list_modules()?;

        self.modules = raw
            .into_iter()
            .map(|m| {
                let enabled = config.enabled_modules.contains(&m.name);
                ModuleEntry {
                    name: m.name,
                    description: m.description,
                    package_count: m.package_count,
                    enabled,
                }
            })
            .collect();

        self.rebuild_visible();

        // Select first item if nothing is selected
        if !self.visible.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }

        self.load_error = None;
        self.loaded = true;
        Ok(())
    }

    fn rebuild_visible(&mut self) {
        self.visible = filter_module_entries(&self.modules, &self.filter_query);
        // Clamp selection to valid range
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

    fn selected_entry(&self) -> Option<&ModuleEntry> {
        let idx = self.list_state.selected()?;
        let module_idx = *self.visible.get(idx)?;
        self.modules.get(module_idx)
    }
}

impl ScreenTrait for ModulesScreenState {
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
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(entry) = self.selected_entry() {
                    let enable = module_toggle_enable(entry.enabled);
                    return Ok(Some(ScreenAction::Request(Action::ToggleModule {
                        name: entry.name.clone(),
                        enable,
                    })));
                }
            }
            _ => {}
        }
        Ok(None)
    }

    fn is_filtering(&self) -> bool {
        self.filter_active
    }

    fn refresh(&mut self) {
        self.loaded = false;
        self.load_error = None;
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
                self.loaded = true; // avoid re-trying every frame on hard errors
            }
        }

        // Layout: optional error banner, then list + detail pane
        let has_error = self.load_error.is_some();
        let constraints = if has_error {
            vec![Constraint::Length(3), Constraint::Min(0)]
        } else {
            vec![Constraint::Min(0)]
        };

        let chunks = Layout::default()
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
            frame.render_widget(para, chunks[0]);
            chunks[1]
        } else {
            chunks[0]
        };

        // Split content into list (left) and detail (right)
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(content_area);

        self.render_list(frame, panes[0]);
        self.render_detail(frame, panes[1]);

        Ok(())
    }
}

impl ModulesScreenState {
    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let enabled_count = self.modules.iter().filter(|m| m.enabled).count();

        let filter_hint = if self.filter_active {
            format!(" [filter: {}▌]", self.filter_query)
        } else if !self.filter_query.is_empty() {
            format!(" [/{}/]", self.filter_query)
        } else {
            String::new()
        };

        let title = format!(
            " Modules ({} enabled / {} total){} ",
            enabled_count,
            self.modules.len(),
            filter_hint
        );

        let items: Vec<ListItem> = self
            .visible
            .iter()
            .filter_map(|&idx| self.modules.get(idx))
            .map(|entry| {
                let status_char = if entry.enabled { "●" } else { "○" };
                let status_color = if entry.enabled {
                    Color::Green
                } else {
                    Color::DarkGray
                };

                let line = Line::from(vec![
                    Span::styled(
                        format!("{} ", status_char),
                        Style::default().fg(status_color),
                    ),
                    Span::styled(
                        format!("{:<30}", &entry.name),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!(" ({} pkgs)", entry.package_count),
                        Style::default().fg(Color::Cyan),
                    ),
                ]);
                ListItem::new(line)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue))
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

    fn render_detail(&self, frame: &mut Frame, area: Rect) {
        let content = if let Some(entry) = self.selected_entry() {
            let status = if entry.enabled { "Enabled" } else { "Disabled" };
            let status_color = if entry.enabled {
                Color::Green
            } else {
                Color::DarkGray
            };

            vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("Name: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        entry.name.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(Color::Cyan)),
                    Span::styled(status, Style::default().fg(status_color)),
                ]),
                Line::from(vec![
                    Span::styled("Packages: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!("{}", entry.package_count),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    if entry.description.is_empty() {
                        "(no description)"
                    } else {
                        &entry.description
                    },
                    Style::default().fg(Color::Gray),
                )),
            ]
        } else if self.modules.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No modules found.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Create modules in the 'modules/'",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "directory of your config folder.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No module selected.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        };

        let para = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Blue))
                    .title(" Detail "),
            )
            .wrap(ratatui::widgets::Wrap { trim: true });

        frame.render_widget(para, area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<ModuleEntry> {
        vec![
            ModuleEntry {
                name: "base".to_string(),
                description: "Core packages".to_string(),
                package_count: 10,
                enabled: true,
            },
            ModuleEntry {
                name: "desktop".to_string(),
                description: "Desktop environment".to_string(),
                package_count: 20,
                enabled: false,
            },
            ModuleEntry {
                name: "audio".to_string(),
                description: "PipeWire audio stack".to_string(),
                package_count: 5,
                enabled: true,
            },
        ]
    }

    #[test]
    fn filter_empty_query_returns_all() {
        let entries = sample_entries();
        let result = filter_module_entries(&entries, "");
        assert_eq!(result, vec![0, 1, 2]);
    }

    #[test]
    fn filter_matches_name_substring() {
        let entries = sample_entries();
        let result = filter_module_entries(&entries, "aud");
        assert_eq!(result, vec![2]);
    }

    #[test]
    fn filter_matches_description_substring() {
        let entries = sample_entries();
        let result = filter_module_entries(&entries, "core");
        assert_eq!(result, vec![0]);
    }

    #[test]
    fn filter_is_case_insensitive() {
        let entries = sample_entries();
        let result = filter_module_entries(&entries, "DESKTOP");
        assert_eq!(result, vec![1]);
    }

    #[test]
    fn filter_no_match_returns_empty() {
        let entries = sample_entries();
        let result = filter_module_entries(&entries, "xyzzy");
        assert!(result.is_empty());
    }
}
