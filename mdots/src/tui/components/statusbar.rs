use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::tui::app::{App, MessageLevel};
use crate::tui::keybindings::footer_hints;

pub fn render_statusbar(app: &App, frame: &mut Frame, area: Rect) -> Result<()> {
    // Show status message if any, otherwise show the active screen's
    // context keybindings (sourced from `keybindings`, the same place the
    // `?` help overlay reads from, so the two can't drift apart).
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
        let mut spans = vec![Span::raw(" ")];
        for (i, hint) in footer_hints(&app.current_screen).iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                format!("[{}]", hint.key),
                Style::default().fg(Color::Yellow),
            ));
            spans.push(Span::styled(
                format!(" {}", hint.desc),
                Style::default()
                    .fg(crate::tui::theme::dim())
                    .add_modifier(Modifier::DIM),
            ));
        }
        Line::from(spans)
    };

    let paragraph = Paragraph::new(content).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(crate::tui::theme::accent())),
    );

    frame.render_widget(paragraph, area);
    Ok(())
}
