use anyhow::Result;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::centered_rect;
use crate::tui::keybindings::{self, KeyHint};
use crate::tui::screens::Screen;

/// Render the `?` keybinding help overlay on top of everything: global
/// keys, sidebar-navigation keys, and the active screen's context keys
/// (plus its filter-field keys, when relevant). All key lists are pulled
/// from `keybindings` — the same source the statusbar footer reads — so
/// this view can't drift from what's actually wired up.
pub fn render_help_overlay(screen: &Screen, frame: &mut Frame, area: Rect) -> Result<()> {
    let popup_area = centered_rect(70, 75, area);

    // Clear the area behind the overlay
    frame.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::new();

    lines.push(section_heading("Global"));
    lines.extend(hint_lines(keybindings::GLOBAL_HINTS));
    lines.push(Line::from(""));

    lines.push(section_heading("Sidebar (when expanded)"));
    lines.extend(hint_lines(keybindings::SIDEBAR_HINTS));
    lines.push(Line::from(""));

    lines.push(section_heading(&format!("This screen — {}", screen.name())));
    lines.extend(hint_lines(keybindings::screen_hints(screen)));

    if screen.is_filtering() {
        lines.push(Line::from(""));
        lines.push(section_heading("Filter field (active)"));
        lines.extend(hint_lines(keybindings::FILTER_HINTS));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Press ? or Esc to close",
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
    )));

    let block = Block::default()
        .title(" Keybindings ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true });

    frame.render_widget(paragraph, popup_area);
    Ok(())
}

fn section_heading(title: &str) -> Line<'static> {
    Line::from(Span::styled(
        title.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn hint_lines(hints: &[KeyHint]) -> Vec<Line<'static>> {
    hints
        .iter()
        .map(|hint| {
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("[{}]", hint.key),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}", hint.desc)),
            ])
        })
        .collect()
}
