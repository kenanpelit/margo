use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::commands::hooks::gather_hooks;
use crate::config::{Config, ConfigPaths};
use crate::tui::app::Action;
use crate::tui::screens::{ScreenAction, ScreenTrait};

/// One hook row for display, derived from `commands::hooks::HookStatus`.
#[derive(Clone)]
struct HookRow {
    module: String,
    hook_type: String,
    status: String,
    script: Option<String>,
    /// Flags to pass to `commands::hooks::run`; `None` for global update
    /// hooks, which are not runnable per-module from here.
    run_flags: Option<(bool, bool)>, // (pre, disable)
}

/// Map a hook's `hook_type` to the `(pre, disable)` flags `hooks::run` wants,
/// or `None` for the global update hooks (run via `mdots update`).
fn run_flags_for(hook_type: &str) -> Option<(bool, bool)> {
    match hook_type {
        "pre-install" => Some((true, false)),
        "post-install" => Some((false, false)),
        "disable" => Some((false, true)),
        _ => None, // pre-update / post-update are global
    }
}

#[derive(Clone, Default)]
pub struct HooksScreenState {
    hooks: Vec<HookRow>,
    list_state: ListState,
    loaded: bool,
    load_error: Option<String>,
}

impl HooksScreenState {
    fn load_data(&mut self, paths: &ConfigPaths, _config: &Config) -> Result<()> {
        self.hooks = gather_hooks(paths)?
            .into_iter()
            .map(|h| HookRow {
                run_flags: run_flags_for(&h.hook_type),
                module: h.module,
                hook_type: h.hook_type,
                status: h.status,
                script: h.script,
            })
            .collect();

        if !self.hooks.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
        self.load_error = None;
        self.loaded = true;
        Ok(())
    }

    fn select_next(&mut self) {
        if self.hooks.is_empty() {
            return;
        }
        let next = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.hooks.len().saturating_sub(1)),
            None => 0,
        };
        self.list_state.select(Some(next));
    }

    fn select_prev(&mut self) {
        if self.hooks.is_empty() {
            return;
        }
        let prev = match self.list_state.selected() {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(prev));
    }

    fn selected_entry(&self) -> Option<&HookRow> {
        self.hooks.get(self.list_state.selected()?)
    }
}

impl ScreenTrait for HooksScreenState {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match key.code {
            KeyCode::Esc => return Ok(Some(ScreenAction::Back)),
            KeyCode::Char('r') => {
                self.loaded = false;
                self.load_error = None;
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::Enter => {
                if let Some(entry) = self.selected_entry() {
                    if let Some((pre, disable)) = entry.run_flags {
                        return Ok(Some(ScreenAction::Request(Action::RunHook {
                            module: entry.module.clone(),
                            pre,
                            disable,
                            label: entry.hook_type.clone(),
                        })));
                    }
                }
            }
            _ => {}
        }
        Ok(None)
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
                self.loaded = true;
            }
        }

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
            let err = self.load_error.as_deref().unwrap_or("Unknown error");
            let para = Paragraph::new(err).block(
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

        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
            .split(content_area);

        self.render_list(frame, panes[0]);
        self.render_detail(frame, panes[1]);
        Ok(())
    }
}

impl HooksScreenState {
    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let title = format!(" Hooks ({}) ", self.hooks.len());

        let items: Vec<ListItem> = self
            .hooks
            .iter()
            .map(|entry| {
                let (mark, color) = match entry.status.as_str() {
                    "executed" => ("✓", Color::Green),
                    "skipped" => ("⊘", Color::Yellow),
                    "not_run" => ("○", crate::tui::theme::dim()),
                    _ => ("?", Color::Red),
                };
                let line = Line::from(vec![
                    Span::styled(format!("{mark} "), Style::default().fg(color)),
                    Span::styled(
                        format!("{:<6}", short_type(&entry.hook_type)),
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        entry.module.clone(),
                        Style::default().fg(crate::tui::theme::text()),
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
            let mut lines = vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("Module: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        entry.module.clone(),
                        Style::default().add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Type: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        entry.hook_type.clone(),
                        Style::default().fg(crate::tui::theme::text()),
                    ),
                ]),
                Line::from(vec![
                    Span::styled("Status: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        entry.status.clone(),
                        Style::default().fg(crate::tui::theme::text()),
                    ),
                ]),
            ];
            if let Some(script) = &entry.script {
                lines.push(Line::from(vec![Span::styled(
                    "Script: ",
                    Style::default().fg(Color::Cyan),
                )]));
                lines.push(Line::from(Span::styled(
                    script.clone(),
                    Style::default().fg(Color::Gray),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                if entry.run_flags.is_some() {
                    "[Enter] run this hook"
                } else {
                    "(global update hook — run via `mdots update`)"
                },
                Style::default().fg(crate::tui::theme::dim()),
            )));
            lines
        } else if self.hooks.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No hooks found in enabled modules.",
                    Style::default().fg(crate::tui::theme::dim()),
                )),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No hook selected.",
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

/// A short fixed-width label for a hook type.
fn short_type(hook_type: &str) -> &'static str {
    match hook_type {
        "pre-install" => "pre",
        "post-install" => "post",
        "disable" => "dis",
        "pre-update" => "preU",
        "post-update" => "postU",
        _ => "?",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_flags_module_hooks_are_runnable() {
        assert_eq!(run_flags_for("pre-install"), Some((true, false)));
        assert_eq!(run_flags_for("post-install"), Some((false, false)));
        assert_eq!(run_flags_for("disable"), Some((false, true)));
    }

    #[test]
    fn run_flags_update_hooks_are_not_runnable() {
        assert_eq!(run_flags_for("pre-update"), None);
        assert_eq!(run_flags_for("post-update"), None);
    }
}
