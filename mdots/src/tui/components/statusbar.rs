use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::{App, MessageLevel};

pub fn render_statusbar(app: &App, frame: &mut Frame, area: Rect) -> Result<()> {
    // Show status message if any, otherwise show keybindings
    let content = if let Some(msg) = &app.status_message {
        let color = match msg.level {
            MessageLevel::Info => Color::Cyan,
            MessageLevel::Success => Color::Green,
            MessageLevel::Warning => Color::Yellow,
            MessageLevel::Error => Color::Red,
        };

        Line::from(vec![Span::styled(
            &msg.text,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        )])
    } else {
        // Default keybindings
        Line::from(vec![
            Span::raw(" ["),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw("] Quit  ["),
            Span::styled("m", Style::default().fg(Color::Yellow)),
            Span::raw("] Toggle Menu  ["),
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw("] Navigate  ["),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw("] Back "),
        ])
    };

    let paragraph = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue)),
    );

    frame.render_widget(paragraph, area);
    Ok(())
}
