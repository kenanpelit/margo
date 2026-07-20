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

use crate::commands::diff::{compute_drift, Drift};
use crate::config::{Config, ConfigPaths};
use crate::dotfiles::{compute_dotfile_drift, DotfileAction, DotfileDrift};
use crate::package::PackageManager;
use crate::tui::app::Action;
use crate::tui::job::Job;
use crate::tui::screens::{ScreenAction, ScreenTrait};

/// Everything the screen shows, gathered in one background probe.
struct DiffData {
    packages: Drift,
    dotfiles: Vec<DotfileDrift>,
}

impl DiffData {
    /// Dotfiles that a sync would actually touch.
    fn changed_dotfiles(&self) -> impl Iterator<Item = &DotfileDrift> {
        self.dotfiles.iter().filter(|d| !d.action.is_in_sync())
    }

    fn changed_dotfile_count(&self) -> usize {
        self.changed_dotfiles().count()
    }

    fn is_in_sync(&self) -> bool {
        self.packages.is_in_sync() && self.changed_dotfile_count() == 0
    }
}

/// Gather package drift and dotfile drift together. Runs on a worker thread
/// (see [`Job`]) — between `pacman`, `flatpak` and a stat of every declared
/// dotfile this is by far the heaviest probe in the TUI.
fn load_diff(paths: &ConfigPaths, config: &Config) -> Result<DiffData> {
    let pm = PackageManager::new(paths.clone());
    let declared = pm.get_declared_packages(config)?;
    let installed_native = pm.get_installed_native_packages(config).unwrap_or_default();
    let installed_flatpak: HashSet<String> = pm
        .get_installed_flatpaks(config.flatpak_scope.as_arg())
        .unwrap_or_default()
        .into_iter()
        .collect();

    Ok(DiffData {
        packages: compute_drift(paths, &declared, &installed_native, &installed_flatpak),
        dotfiles: compute_dotfile_drift(paths, config)?,
    })
}

/// One line in the flattened diff list.
///
/// Sections are rendered but never selectable — [`next_selectable`] skips
/// them — so the highlight only ever lands on something actionable.
enum DiffRow {
    Section(String),
    Install { name: String, kind: &'static str },
    Remove { name: String, kind: &'static str },
    Dotfile(DotfileDrift),
}

impl DiffRow {
    fn is_selectable(&self) -> bool {
        !matches!(self, DiffRow::Section(_))
    }
}

/// Flatten a [`DiffData`] into the rows shown in the list, in a fixed order:
/// what gets added, what gets removed, then dotfile work. Sections with no
/// entries are omitted entirely rather than rendered empty.
fn build_rows(data: &DiffData) -> Vec<DiffRow> {
    let mut rows = Vec::new();

    let mut push_group = |title: String, names: &[String], kind: &'static str, install: bool| {
        if names.is_empty() {
            return;
        }
        rows.push(DiffRow::Section(title));
        let mut sorted = names.to_vec();
        sorted.sort();
        for name in sorted {
            rows.push(if install {
                DiffRow::Install { name, kind }
            } else {
                DiffRow::Remove { name, kind }
            });
        }
    };

    let p = &data.packages;
    push_group(
        format!("Packages to install ({})", p.install_count()),
        &p.native_to_install,
        "native",
        true,
    );
    push_group(String::new(), &p.flatpak_to_install, "flatpak", true);
    push_group(
        format!("Packages to prune ({})", p.remove_count()),
        &p.native_to_remove,
        "native",
        false,
    );
    push_group(String::new(), &p.flatpak_to_remove, "flatpak", false);

    // The flatpak groups above reuse the native group's heading when the
    // native list was empty, which would leave them with a blank title —
    // drop those placeholder sections.
    rows.retain(|r| !matches!(r, DiffRow::Section(t) if t.is_empty()));

    let changed: Vec<&DotfileDrift> = data.changed_dotfiles().collect();
    if !changed.is_empty() {
        rows.push(DiffRow::Section(format!("Dotfiles ({})", changed.len())));
        for d in changed {
            rows.push(DiffRow::Dotfile(d.clone()));
        }
    }

    rows
}

/// Next selectable index at or after `from`, searching in `step` direction.
/// Returns `None` when there is nothing selectable that way.
fn next_selectable(rows: &[DiffRow], from: usize, forward: bool) -> Option<usize> {
    let mut i = from;
    loop {
        if rows.get(i)?.is_selectable() {
            return Some(i);
        }
        if forward {
            i = i.checked_add(1)?;
        } else {
            i = i.checked_sub(1)?;
        }
    }
}

#[derive(Default)]
pub struct DiffScreenState {
    data: Option<DiffData>,
    rows: Vec<DiffRow>,
    list_state: ListState,
    loaded: bool,
    load_error: Option<String>,
    load_job: Job<DiffData>,
}

impl DiffScreenState {
    fn start_load(&mut self, paths: &ConfigPaths, config: &Config) {
        let paths = paths.clone();
        let config = config.clone();
        self.load_job.spawn(move || load_diff(&paths, &config));
        self.loaded = false;
        self.load_error = None;
    }

    fn apply_load(&mut self, result: Result<DiffData>) {
        match result {
            Ok(data) => {
                self.rows = build_rows(&data);
                self.data = Some(data);
                self.list_state.select(next_selectable(&self.rows, 0, true));
                self.load_error = None;
            }
            Err(e) => self.load_error = Some(format!("{e:#}")),
        }
        self.loaded = true;
    }

    fn select_next(&mut self) {
        let Some(current) = self.list_state.selected() else {
            self.list_state.select(next_selectable(&self.rows, 0, true));
            return;
        };
        if let Some(next) = next_selectable(&self.rows, current + 1, true) {
            self.list_state.select(Some(next));
        }
    }

    fn select_prev(&mut self) {
        let Some(current) = self.list_state.selected() else {
            return;
        };
        if let Some(prev) = current
            .checked_sub(1)
            .and_then(|i| next_selectable(&self.rows, i, false))
        {
            self.list_state.select(Some(prev));
        }
    }

    /// The sync this diff would resolve, or `None` when nothing is pending.
    fn sync_action(&self) -> Option<Action> {
        let data = self.data.as_ref()?;
        if data.packages.is_in_sync() {
            return None;
        }
        Some(Action::RunSync {
            native_install: data.packages.native_to_install.len(),
            flatpak_install: data.packages.flatpak_to_install.len(),
            prune: data.packages.remove_count(),
        })
    }
}

impl ScreenTrait for DiffScreenState {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match key.code {
            KeyCode::Esc => return Ok(Some(ScreenAction::Back)),
            KeyCode::Char('r') => self.loaded = false,
            KeyCode::Char('s') => {
                if let Some(action) = self.sync_action() {
                    return Ok(Some(ScreenAction::Request(action)));
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
            _ => {}
        }
        Ok(None)
    }

    fn on_activate(&mut self, paths: &ConfigPaths, config: &Config) -> Result<()> {
        self.start_load(paths, config);
        Ok(())
    }

    fn refresh(&mut self) {
        self.loaded = false;
    }

    fn is_busy(&self) -> bool {
        self.load_job.is_running()
    }

    fn render(
        &mut self,
        paths: &ConfigPaths,
        config: &Config,
        frame: &mut Frame,
        area: Rect,
    ) -> Result<()> {
        if let Some(result) = self.load_job.take() {
            self.apply_load(result);
        } else if !self.loaded && !self.load_job.is_running() {
            self.start_load(paths, config);
        }

        if !self.loaded && self.data.is_none() && self.load_error.is_none() {
            let para = Paragraph::new("Computing drift…")
                .block(bordered(" Diff "))
                .style(Style::default().fg(crate::tui::theme::dim()));
            frame.render_widget(para, area);
            return Ok(());
        }

        if let Some(err) = &self.load_error {
            let para = Paragraph::new(err.as_str()).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red))
                    .title(" Diff — error "),
            );
            frame.render_widget(para, area);
            return Ok(());
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(3), Constraint::Min(0)])
            .split(area);

        self.render_summary(frame, chunks[0]);
        self.render_list(frame, chunks[1]);
        Ok(())
    }
}

/// The screen's standard bordered block, accent-coloured like every other.
fn bordered(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::tui::theme::accent()))
        .title(title.to_string())
}

impl DiffScreenState {
    fn render_summary(&self, frame: &mut Frame, area: Rect) {
        let Some(data) = &self.data else { return };

        let line = if data.is_in_sync() {
            Line::from(Span::styled(
                "✓ in sync — nothing to do",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ))
        } else {
            Line::from(vec![
                Span::styled("+", Style::default().fg(Color::Green)),
                Span::styled(
                    format!("{} to install", data.packages.install_count()),
                    Style::default().fg(Color::Green),
                ),
                Span::raw("   "),
                Span::styled("−", Style::default().fg(Color::Red)),
                Span::styled(
                    format!("{} prunable", data.packages.remove_count()),
                    Style::default().fg(Color::Red),
                ),
                Span::raw("   "),
                Span::styled(
                    format!("{} dotfiles", data.changed_dotfile_count()),
                    Style::default().fg(Color::Yellow),
                ),
                Span::raw("   "),
                Span::styled(
                    "[s] sync  [r] refresh",
                    Style::default().fg(crate::tui::theme::dim()),
                ),
            ])
        };

        frame.render_widget(
            Paragraph::new(vec![Line::from(""), line]).block(bordered(" Drift ")),
            area,
        );
    }

    fn render_list(&mut self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.rows.iter().map(render_row).collect();

        let list = List::new(items)
            .block(bordered(" Changes "))
            .highlight_style(
                Style::default()
                    .bg(crate::tui::theme::dim())
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("> ");

        frame.render_stateful_widget(list, area, &mut self.list_state);
    }
}

/// Render one row. Kept a free function so the row → line mapping can be
/// exercised without building a whole screen.
fn render_row(row: &DiffRow) -> ListItem<'_> {
    match row {
        DiffRow::Section(title) => ListItem::new(Line::from(Span::styled(
            title.clone(),
            Style::default()
                .fg(crate::tui::theme::accent())
                .add_modifier(Modifier::BOLD),
        ))),
        DiffRow::Install { name, kind } => ListItem::new(Line::from(vec![
            Span::styled("  + ", Style::default().fg(Color::Green)),
            Span::styled(
                format!("{name:<38}"),
                Style::default().fg(crate::tui::theme::text()),
            ),
            Span::styled(
                format!(" [{kind}]"),
                Style::default().fg(crate::tui::theme::dim()),
            ),
        ])),
        DiffRow::Remove { name, kind } => ListItem::new(Line::from(vec![
            Span::styled("  − ", Style::default().fg(Color::Red)),
            Span::styled(
                format!("{name:<38}"),
                Style::default().fg(crate::tui::theme::text()),
            ),
            Span::styled(
                format!(" [{kind}]"),
                Style::default().fg(crate::tui::theme::dim()),
            ),
        ])),
        DiffRow::Dotfile(d) => {
            let (marker, colour) = match d.action {
                DotfileAction::Create => ("  + ", Color::Green),
                DotfileAction::Relink { .. } => ("  ~ ", Color::Yellow),
                DotfileAction::Replace { .. } => ("  ! ", Color::Yellow),
                DotfileAction::MissingSource => ("  ✗ ", Color::Red),
                DotfileAction::InSync => ("  = ", crate::tui::theme::dim()),
            };
            ListItem::new(Line::from(vec![
                Span::styled(marker, Style::default().fg(colour)),
                Span::styled(
                    format!("{:<38}", d.target.display().to_string()),
                    Style::default().fg(crate::tui::theme::text()),
                ),
                Span::styled(
                    format!(" {} · {}", d.action.label(), d.module),
                    Style::default().fg(crate::tui::theme::dim()),
                ),
            ]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn drift(native_in: &[&str], flatpak_in: &[&str], native_rm: &[&str]) -> Drift {
        Drift {
            native_to_install: native_in.iter().map(|s| s.to_string()).collect(),
            flatpak_to_install: flatpak_in.iter().map(|s| s.to_string()).collect(),
            native_to_remove: native_rm.iter().map(|s| s.to_string()).collect(),
            flatpak_to_remove: Vec::new(),
        }
    }

    fn dotfile(target: &str, action: DotfileAction) -> DotfileDrift {
        DotfileDrift {
            module: "zsh".to_string(),
            source: PathBuf::from("/modules/zsh/zshrc"),
            target: PathBuf::from(target),
            action,
        }
    }

    #[test]
    fn in_sync_data_produces_no_rows() {
        let data = DiffData {
            packages: drift(&[], &[], &[]),
            dotfiles: vec![dotfile("/home/u/.zshrc", DotfileAction::InSync)],
        };
        assert!(data.is_in_sync());
        assert!(build_rows(&data).is_empty());
    }

    #[test]
    fn install_rows_are_sorted_and_headed_by_a_section() {
        let data = DiffData {
            packages: drift(&["vim", "git"], &[], &[]),
            dotfiles: vec![],
        };
        let rows = build_rows(&data);
        assert!(matches!(&rows[0], DiffRow::Section(t) if t.contains('2')));
        assert!(matches!(&rows[1], DiffRow::Install { name, .. } if name == "git"));
        assert!(matches!(&rows[2], DiffRow::Install { name, .. } if name == "vim"));
    }

    #[test]
    fn in_sync_dotfiles_are_omitted_from_the_list() {
        let data = DiffData {
            packages: drift(&[], &[], &[]),
            dotfiles: vec![
                dotfile("/home/u/.zshrc", DotfileAction::InSync),
                dotfile("/home/u/.vimrc", DotfileAction::Create),
            ],
        };
        let rows = build_rows(&data);
        assert_eq!(data.changed_dotfile_count(), 1);
        let dotfile_rows = rows
            .iter()
            .filter(|r| matches!(r, DiffRow::Dotfile(_)))
            .count();
        assert_eq!(dotfile_rows, 1);
    }

    /// A flatpak-only install list must still get a real heading, not the
    /// blank placeholder the group builder emits for the native group.
    #[test]
    fn flatpak_only_install_list_has_no_blank_section() {
        let data = DiffData {
            packages: drift(&[], &["com.example.App"], &[]),
            dotfiles: vec![],
        };
        let rows = build_rows(&data);
        assert!(!rows
            .iter()
            .any(|r| matches!(r, DiffRow::Section(t) if t.is_empty())));
        assert_eq!(rows.iter().filter(|r| r.is_selectable()).count(), 1);
    }

    #[test]
    fn navigation_skips_section_headers() {
        let data = DiffData {
            packages: drift(&["vim"], &[], &["old"]),
            dotfiles: vec![],
        };
        let rows = build_rows(&data);
        // [Section, Install, Section, Remove]
        assert_eq!(next_selectable(&rows, 0, true), Some(1));
        assert_eq!(next_selectable(&rows, 2, true), Some(3));
        assert_eq!(next_selectable(&rows, 2, false), Some(1));
    }

    #[test]
    fn next_selectable_past_the_end_is_none() {
        let rows = build_rows(&DiffData {
            packages: drift(&["vim"], &[], &[]),
            dotfiles: vec![],
        });
        assert_eq!(next_selectable(&rows, rows.len(), true), None);
    }

    #[test]
    fn sync_action_carries_the_previewed_counts() {
        let mut screen = DiffScreenState::default();
        screen.apply_load(Ok(DiffData {
            packages: drift(&["vim", "git"], &["com.example.App"], &["old"]),
            dotfiles: vec![],
        }));
        assert_eq!(
            screen.sync_action(),
            Some(Action::RunSync {
                native_install: 2,
                flatpak_install: 1,
                prune: 1,
            })
        );
    }

    #[test]
    fn sync_action_is_none_when_packages_are_in_sync() {
        let mut screen = DiffScreenState::default();
        screen.apply_load(Ok(DiffData {
            packages: drift(&[], &[], &[]),
            dotfiles: vec![dotfile("/home/u/.vimrc", DotfileAction::Create)],
        }));
        assert_eq!(screen.sync_action(), None);
    }

    #[test]
    fn load_error_is_kept_for_display() {
        let mut screen = DiffScreenState::default();
        screen.apply_load(Err(anyhow::anyhow!("pacman unavailable")));
        assert!(screen.load_error.as_deref().unwrap().contains("pacman"));
        assert!(screen.loaded);
    }

    #[test]
    fn selection_lands_on_the_first_selectable_row_after_load() {
        let mut screen = DiffScreenState::default();
        screen.apply_load(Ok(DiffData {
            packages: drift(&["vim"], &[], &[]),
            dotfiles: vec![],
        }));
        // Row 0 is the section heading, so the highlight starts at 1.
        assert_eq!(screen.list_state.selected(), Some(1));
    }
}
