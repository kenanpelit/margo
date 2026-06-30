use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::config::{Config, ConfigPaths};
use crate::tui::screens::{ScreenAction, ScreenTrait};

#[derive(Clone, Default)]
pub struct SyncScreenState {
    // Sync preview and execution state will go here
}

impl ScreenTrait for SyncScreenState {
    fn handle_key(&mut self, key: KeyEvent) -> Result<Option<ScreenAction>> {
        match key.code {
            KeyCode::Esc => Ok(Some(ScreenAction::Back)),
            _ => Ok(None),
        }
    }

    fn render(
        &mut self,
        _paths: &ConfigPaths,
        _config: &Config,
        frame: &mut Frame,
        area: Rect,
    ) -> Result<()> {
        let placeholder = Paragraph::new(vec![
            Line::from("Sync Screen"),
            Line::from(""),
            Line::from("Coming soon..."),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Blue))
                .title(" Sync "),
        );

        frame.render_widget(placeholder, area);
        Ok(())
    }
}
