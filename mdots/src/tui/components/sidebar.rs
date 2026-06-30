use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
    Frame,
};

use crate::tui::app::App;

pub fn render_sidebar(app: &App, frame: &mut Frame, area: Rect) -> Result<()> {
    let items: Vec<ListItem> = app
        .sidebar
        .items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if i == app.sidebar.selected_index {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let content = format!(" {} {}", item.icon, item.name);
            ListItem::new(Line::from(Span::styled(content, style)))
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Blue))
            .title(" Navigation "),
    );

    frame.render_widget(list, area);
    Ok(())
}
