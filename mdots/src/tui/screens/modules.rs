use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use std::collections::HashSet;

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

/// Target enabled-state for a bulk toggle: enable everything unless the whole
/// marked set is already enabled, in which case disable it.
///
/// This makes a mixed selection converge on "all enabled" with one keystroke
/// (the common case — you mark the modules you want and press Enter) while
/// still letting a fully-enabled selection be turned off.
fn bulk_enable_target(marked_enabled_states: &[bool]) -> bool {
    marked_enabled_states.iter().any(|&enabled| !enabled)
}

#[derive(Default)]
pub struct ModulesScreenState {
    modules: Vec<ModuleEntry>,
    /// Indices into `modules` that are currently visible (after optional filter)
    visible: Vec<usize>,
    list_state: ListState,
    filter_query: String,
    filter_active: bool,
    loaded: bool,
    load_error: Option<String>,
    /// Names of modules marked with `space` for a bulk toggle. Keyed by name
    /// rather than index so marks survive a filter change or a reload.
    marked: HashSet<String>,
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

    /// Mark or unmark the highlighted module for a bulk toggle.
    fn toggle_mark(&mut self) {
        let Some(name) = self.selected_entry().map(|e| e.name.clone()) else {
            return;
        };
        if !self.marked.remove(&name) {
            self.marked.insert(name);
        }
    }

    /// The bulk action for the current marks, or `None` when nothing is
    /// marked (in which case the caller falls back to the single-row toggle).
    ///
    /// Marks for modules that have since disappeared from the list are
    /// ignored rather than dispatched blind.
    fn bulk_action(&self) -> Option<Action> {
        if self.marked.is_empty() {
            return None;
        }
        let mut names: Vec<String> = Vec::new();
        let mut states: Vec<bool> = Vec::new();
        for entry in self
            .modules
            .iter()
            .filter(|m| self.marked.contains(&m.name))
        {
            names.push(entry.name.clone());
            states.push(entry.enabled);
        }
        if names.is_empty() {
            return None;
        }
        names.sort();
        Some(Action::ToggleModules {
            enable: bulk_enable_target(&states),
            names,
        })
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
            // Esc clears a selection first — losing marks by accident is
            // more annoying than needing a second Esc to leave.
            KeyCode::Esc => {
                if self.marked.is_empty() {
                    return Ok(Some(ScreenAction::Back));
                }
                self.marked.clear();
            }
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
            // Space marks for a bulk toggle and steps down, so a run of
            // modules can be selected by holding it.
            KeyCode::Char(' ') => {
                self.toggle_mark();
                self.select_next();
            }
            // Enter applies to the whole marked set if there is one, else to
            // the highlighted row alone.
            KeyCode::Enter => {
                if let Some(action) = self.bulk_action() {
                    return Ok(Some(ScreenAction::Request(action)));
                }
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
        // A dispatched bulk toggle consumed the marks; leaving them set
        // would invite applying the same batch twice by accident.
        self.marked.clear();
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

        let marked_hint = if self.marked.is_empty() {
            String::new()
        } else {
            format!(" [{} marked — Enter applies]", self.marked.len())
        };

        let title = format!(
            " Modules ({} enabled / {} total){}{} ",
            enabled_count,
            self.modules.len(),
            filter_hint,
            marked_hint
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
                    crate::tui::theme::dim()
                };
                let marked = self.marked.contains(&entry.name);

                let line = Line::from(vec![
                    Span::styled(
                        if marked { "▸ " } else { "  " },
                        Style::default().fg(crate::tui::theme::accent()),
                    ),
                    Span::styled(
                        format!("{} ", status_char),
                        Style::default().fg(status_color),
                    ),
                    Span::styled(
                        format!("{:<30}", &entry.name),
                        Style::default().fg(crate::tui::theme::text()),
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
                    .border_style(Style::default().fg(crate::tui::theme::accent()))
                    .title(title),
            )
            .highlight_style(
                Style::default()
                    .bg(crate::tui::theme::dim())
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
                crate::tui::theme::dim()
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
                    Style::default().fg(crate::tui::theme::dim()),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Create modules in the 'modules/'",
                    Style::default().fg(crate::tui::theme::dim()),
                )),
                Line::from(Span::styled(
                    "directory of your config folder.",
                    Style::default().fg(crate::tui::theme::dim()),
                )),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No module selected.",
                    Style::default().fg(crate::tui::theme::dim()),
                )),
            ]
        };

        let para = Paragraph::new(content)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(crate::tui::theme::accent()))
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

    // ── bulk toggle ─────────────────────────────────────────────────────

    #[test]
    fn bulk_target_enables_when_any_marked_module_is_disabled() {
        assert!(bulk_enable_target(&[true, false, true]));
        assert!(bulk_enable_target(&[false]));
    }

    #[test]
    fn bulk_target_disables_only_when_every_marked_module_is_enabled() {
        assert!(!bulk_enable_target(&[true, true]));
    }

    /// A screen loaded with `sample_entries` and the given names marked.
    fn marked_screen(names: &[&str]) -> ModulesScreenState {
        let mut screen = ModulesScreenState {
            modules: sample_entries(),
            ..Default::default()
        };
        screen.rebuild_visible();
        screen.list_state.select(Some(0));
        screen.marked = names.iter().map(|n| n.to_string()).collect();
        screen
    }

    #[test]
    fn no_marks_means_no_bulk_action() {
        assert!(marked_screen(&[]).bulk_action().is_none());
    }

    #[test]
    fn bulk_action_lists_marked_names_sorted() {
        let action = marked_screen(&["desktop", "audio"]).bulk_action().unwrap();
        assert_eq!(
            action,
            Action::ToggleModules {
                names: vec!["audio".to_string(), "desktop".to_string()],
                // "desktop" is disabled in the fixture, so the batch enables.
                enable: true,
            }
        );
    }

    /// Marks are keyed by name, so one left over from a module that has since
    /// vanished must be ignored rather than dispatched at nothing.
    #[test]
    fn bulk_action_ignores_marks_for_modules_that_no_longer_exist() {
        let screen = marked_screen(&["ghost-module"]);
        assert!(screen.bulk_action().is_none());
    }

    #[test]
    fn marking_toggles_and_survives_a_filter_change() {
        let mut screen = marked_screen(&[]);
        screen.toggle_mark(); // marks "base" (row 0)
        assert!(screen.marked.contains("base"));

        // Filtering to a disjoint set must not drop the mark.
        screen.filter_query = "desktop".to_string();
        screen.rebuild_visible();
        assert!(screen.marked.contains("base"));

        // Unmark by name via a fresh screen positioned on the same row.
        let mut screen = marked_screen(&["base"]);
        screen.toggle_mark();
        assert!(!screen.marked.contains("base"));
    }

    #[test]
    fn refresh_clears_marks_so_a_batch_is_not_applied_twice() {
        let mut screen = marked_screen(&["base", "desktop"]);
        screen.refresh();
        assert!(screen.marked.is_empty());
        assert!(screen.bulk_action().is_none());
    }
}
