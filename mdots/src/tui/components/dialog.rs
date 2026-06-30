use anyhow::Result;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::centered_rect;
use crate::tui::app::Dialog;

pub fn render_dialog(dialog: &Dialog, frame: &mut Frame, area: Rect) -> Result<()> {
    // Create centered rectangle for dialog
    let dialog_area = centered_rect(60, 40, area);

    // Clear the area behind the dialog
    frame.render_widget(Clear, dialog_area);

    match dialog {
        Dialog::Confirm {
            title,
            message,
            confirmed: _,
        } => {
            let block = Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow));

            let text = vec![
                Line::from(""),
                Line::from(message.as_str()),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        "[Y]",
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" Yes  "),
                    Span::styled(
                        "[N]",
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" No"),
                ]),
            ];

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });

            frame.render_widget(paragraph, dialog_area);
        }
        Dialog::Error { title, message } => {
            let block = Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red));

            let text = vec![
                Line::from(""),
                Line::from(Span::styled(
                    message.as_str(),
                    Style::default().fg(Color::Red),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });

            frame.render_widget(paragraph, dialog_area);
        }
        Dialog::Info { title, message } => {
            let block = Block::default()
                .title(format!(" {} ", title))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan));

            let text = vec![
                Line::from(""),
                Line::from(message.as_str()),
                Line::from(""),
                Line::from(Span::styled(
                    "Press Esc to close",
                    Style::default().fg(Color::DarkGray),
                )),
            ];

            let paragraph = Paragraph::new(text)
                .block(block)
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true });

            frame.render_widget(paragraph, dialog_area);
        }
    }

    Ok(())
}
