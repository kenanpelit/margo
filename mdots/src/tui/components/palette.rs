use anyhow::Result;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

use crate::tui::palette::PaletteState;

use super::centered_rect;

/// Draw the command palette over whatever is on screen.
pub fn render_palette(palette: &PaletteState, frame: &mut Frame, area: Rect) -> Result<()> {
    let area = centered_rect(60, 60, area);
    frame.render_widget(Clear, area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::tui::theme::accent()))
        .title(" Command palette ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(inner);

    // Query line, with a block cursor so it reads as a live text field.
    let query = Paragraph::new(Line::from(vec![
        Span::styled("> ", Style::default().fg(crate::tui::theme::accent())),
        Span::styled(
            palette.query.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled("▌", Style::default().fg(crate::tui::theme::accent())),
    ]));
    frame.render_widget(query, chunks[0]);

    if palette.matches.is_empty() {
        let empty =
            Paragraph::new("no matching command").style(Style::default().fg(Color::DarkGray));
        frame.render_widget(empty, chunks[1]);
        return Ok(());
    }

    let width = chunks[1].width as usize;
    let items: Vec<ListItem> = palette
        .visible_entries()
        .map(|(entry, selected)| {
            let (marker, label_style) = if selected {
                (
                    "> ",
                    Style::default()
                        .fg(crate::tui::theme::accent())
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                ("  ", Style::default().fg(Color::White))
            };
            // Right-align the hint against the pane's inner width, clamping
            // so a narrow terminal degrades to "no padding" rather than
            // panicking on a negative width.
            let used = marker.len() + entry.label.chars().count() + entry.hint.len();
            let pad = width.saturating_sub(used).max(1);
            ListItem::new(Line::from(vec![
                Span::styled(marker, label_style),
                Span::styled(entry.label.clone(), label_style),
                Span::raw(" ".repeat(pad)),
                Span::styled(entry.hint, Style::default().fg(Color::DarkGray)),
            ]))
        })
        .collect();

    frame.render_widget(List::new(items), chunks[1]);
    Ok(())
}
