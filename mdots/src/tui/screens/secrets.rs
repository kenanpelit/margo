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
use crate::secrets::{
    classify_secret_status, resolve_key_path, resolve_secret_target, secret_name, sops_available,
    SecretState,
};
use crate::tui::app::Action;
use crate::tui::screens::{ScreenAction, ScreenTrait};

/// One declared secret row, derived the same way `mdots secrets status` does.
#[derive(Clone)]
struct SecretRow {
    name: String,
    target: String,
    label: String,
    decrypted: bool,
}

#[derive(Clone, Default)]
pub struct SecretsScreenState {
    secrets: Vec<SecretRow>,
    list_state: ListState,
    loaded: bool,
    load_error: Option<String>,
}

impl SecretsScreenState {
    fn load_data(&mut self, paths: &ConfigPaths, config: &Config) -> Result<()> {
        let home = crate::commands::secrets::home_dir()?;
        let repo_root = &paths.config_dir;
        let key_path = resolve_key_path(config.sops_key_path.as_deref(), &home);
        let sops = sops_available();
        let key_available = key_path.as_ref().map(|k| k.exists()).unwrap_or(true);

        self.secrets = config
            .secrets
            .iter()
            .map(|entry| {
                let name = secret_name(entry);
                match resolve_secret_target(&entry.target, &home, repo_root) {
                    Err(e) => SecretRow {
                        name,
                        target: entry.target.clone(),
                        label: format!("invalid: {e}"),
                        decrypted: false,
                    },
                    Ok(target) => {
                        let source_exists = repo_root.join(&entry.source).exists();
                        let target_exists = target.exists();
                        let state = classify_secret_status(
                            sops,
                            source_exists,
                            key_available,
                            target_exists,
                        );
                        let (label, decrypted) = match state {
                            SecretState::SopsMissing => ("sops not installed".to_string(), false),
                            SecretState::SourceMissing => ("source not found".to_string(), false),
                            SecretState::KeyMissing => ("key missing".to_string(), false),
                            SecretState::Pending => ("pending".to_string(), false),
                            SecretState::Decrypted => ("decrypted".to_string(), true),
                        };
                        SecretRow {
                            name,
                            target: target.display().to_string(),
                            label,
                            decrypted,
                        }
                    }
                }
            })
            .collect();

        if !self.secrets.is_empty() && self.list_state.selected().is_none() {
            self.list_state.select(Some(0));
        }
        self.load_error = None;
        self.loaded = true;
        Ok(())
    }

    fn select_next(&mut self) {
        if self.secrets.is_empty() {
            return;
        }
        let next = match self.list_state.selected() {
            Some(i) => (i + 1).min(self.secrets.len().saturating_sub(1)),
            None => 0,
        };
        self.list_state.select(Some(next));
    }

    fn select_prev(&mut self) {
        if self.secrets.is_empty() {
            return;
        }
        let prev = match self.list_state.selected() {
            Some(0) | None => 0,
            Some(i) => i - 1,
        };
        self.list_state.select(Some(prev));
    }

    fn selected_entry(&self) -> Option<&SecretRow> {
        self.secrets.get(self.list_state.selected()?)
    }
}

impl ScreenTrait for SecretsScreenState {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match key.code {
            KeyCode::Esc => return Ok(Some(ScreenAction::Back)),
            KeyCode::Char('r') => {
                self.loaded = false;
                self.load_error = None;
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            KeyCode::Char('e') | KeyCode::Enter => {
                if let Some(entry) = self.selected_entry() {
                    return Ok(Some(ScreenAction::Request(Action::EditSecret {
                        name: entry.name.clone(),
                    })));
                }
            }
            KeyCode::Char('s') => {
                return Ok(Some(ScreenAction::Request(Action::SyncSecrets)));
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
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(content_area);

        self.render_list(frame, panes[0]);
        self.render_detail(frame, panes[1]);
        Ok(())
    }
}

impl SecretsScreenState {
    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let decrypted = self.secrets.iter().filter(|s| s.decrypted).count();
        let title = format!(
            " Secrets ({} decrypted / {} declared) ",
            decrypted,
            self.secrets.len()
        );

        let items: Vec<ListItem> = self
            .secrets
            .iter()
            .map(|entry| {
                let (mark, color) = if entry.decrypted {
                    ("✓", Color::Green)
                } else if entry.label == "pending" {
                    ("•", Color::Blue)
                } else {
                    ("✗", Color::Red)
                };
                let line = Line::from(vec![
                    Span::styled(format!("{mark} "), Style::default().fg(color)),
                    Span::styled(
                        format!("{:<20}", &entry.name),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(
                        format!(" {}", &entry.label),
                        Style::default().fg(Color::Gray),
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
            let color = if entry.decrypted {
                Color::Green
            } else {
                Color::Yellow
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
                    Span::styled(entry.label.clone(), Style::default().fg(color)),
                ]),
                Line::from(vec![Span::styled(
                    "Target: ",
                    Style::default().fg(Color::Cyan),
                )]),
                Line::from(Span::styled(
                    entry.target.clone(),
                    Style::default().fg(Color::Gray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "[e/Enter] edit with sops   [s] sync secrets",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        } else if self.secrets.is_empty() {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No secrets declared.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Declare secrets in your host config",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(Span::styled(
                    "under the `secrets:` key.",
                    Style::default().fg(Color::DarkGray),
                )),
            ]
        } else {
            vec![
                Line::from(""),
                Line::from(Span::styled(
                    "No secret selected.",
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
