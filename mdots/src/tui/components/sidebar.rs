use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::tui::app::App;

/// Draw the navigation sidebar into `area` and return the inner rect its
/// item rows occupy (the block border excluded). The caller records that
/// rect in [`crate::tui::layout::LayoutSnapshot`] so mouse hit-testing uses
/// the geometry that was actually drawn instead of a second copy of it.
pub fn render_sidebar(app: &App, frame: &mut Frame, area: Rect) -> Result<Rect> {
    let items: Vec<ListItem> = app
        .sidebar
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if i == app.sidebar.selected_index {
                Style::default()
                    .fg(crate::tui::theme::accent_fg())
                    .bg(crate::tui::theme::accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(crate::tui::theme::text())
            };

            let content = format!(" {} {}", item.icon, item.name);
            ListItem::new(Line::from(Span::styled(content, style)))
        })
        .collect();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(crate::tui::theme::accent()))
        .title(" Navigation ");
    let items_area = block.inner(area);

    frame.render_widget(List::new(items).block(block), area);
    Ok(items_area)
}
