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
use crate::service_profile::ServiceProfileManager;
use crate::tui::app::Action;
use crate::tui::screens::{ScreenAction, ScreenTrait};

/// One service profile row, derived from `ServiceProfileInfo`.
#[derive(Clone)]
struct ServiceEntry {
    name: String,
    description: String,
    enabled: bool,
    enabled_count: usize,
    disabled_count: usize,
}

#[derive(Clone, Default)]
pub struct ServicesScreenState {
    services: Vec<ServiceEntry>,
    list_state: ListState,
    loaded: bool,
    load_error: Option<String>,
}

impl ServicesScreenState {
    fn load_data(&mut self, paths: &ConfigPaths, config: &Config) -> Result<()> {
        let manager = ServiceProfileManager::new(paths.clone());
        let profiles = manager.list_profiles(&config.enabled_service_profiles)?;

        self.services = profiles
            .into_iter()
            .map(|p| ServiceEntry {
                name: p.name,
                description: p.description,
                enabled: p.is_enabled,
                enabled_count: p.enabled_services.len(),
                disabled_count: p.disabled_services.len(),
            })
            .collect();

        if !self.services.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
        self.load_error = None;
        self.loaded = true;
        Ok(())
    }

    fn select_next(&mut self) {
        if self.services.is_empty() {
            return;
        }
        let next = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.services.len().saturating_sub(1)),
            None => 0,
        };
        self.list_state.select(Some(next));
    }

    fn select_prev(&mut self) {
        if self.services.is_empty() {
            return;
        }
        let prev = match self.list_state.selected() {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(prev));
    }

    fn selected_entry(&self) -> Option<&ServiceEntry> {
        self.services.get(self.list_state.selected()?)
    }
}

impl ScreenTrait for ServicesScreenState {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match key.code {
            KeyCode::Esc => return Ok(Some(ScreenAction::Back)),
            KeyCode::Char('r') => {
                self.loaded = false;
                self.load_error = None;
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::Char(' ') | KeyCode::Enter => {
                if let Some(entry) = self.selected_entry() {
                    let action = if entry.enabled {
                        Action::DisableService {
                            name: entry.name.clone(),
                        }
                    } else {
                        Action::EnableService {
                            name: entry.name.clone(),
                        }
                    };
                    return Ok(Some(ScreenAction::Request(action)));
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

impl ServicesScreenState {
    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let enabled_count = self.services.iter().filter(|s| s.enabled).count();
        let title = format!(
            " Service profiles ({} enabled / {} total) ",
            enabled_count,
            self.services.len()
        );

        let items: Vec<ListItem> = self
            .services
            .iter()
            .map(|entry| {
                let (status_char, status_color) = if entry.enabled {
                    ("●", Color::Green)
                } else {
                    ("○", Color::DarkGray)
                };
                let line = Line::from(vec![
                    Span::styled(format!("{status_char} "), Style::default().fg(status_color)),
                    Span::styled(
                        format!("{:<24}", &entry.name),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!(" ({} svc)", entry.enabled_count),
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
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }

    fn render_detail(&self, frame: &mut Frame, area: Rect) {
        let content = if let Some(entry) = self.selected_entry() {
            let (status, status_color) = if entry.enabled {
                ("Enabled", Color::Green)
            } else {
                ("Disabled", Color::DarkGray)
            };
            vec![
                Line::from(""),
                Line::from(vec![
                    Span::styled("Profile: ", Style::default().fg(Color::Cyan)),
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
                    Span::styled("Services: ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!(
                            "{} enabled, {} disabled",
                            entry.enabled_count, entry.disabled_count
                        ),
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
        } else if self.services.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No service profiles found.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Create profiles in the 'services/'",
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
                    "No profile selected.",
                    Style::default().fg(Color::DarkGray),
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
