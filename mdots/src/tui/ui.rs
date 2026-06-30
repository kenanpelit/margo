use anyhow::Result;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

use super::app::App;
use super::components::{
    render_dialog, render_help_overlay, render_sidebar, render_statusbar, render_titlebar,
};

/// Main UI rendering function
pub fn render(app: &mut App, frame: &mut Frame) -> Result<()> {
    let size = frame.area();

    // Top-level layout: [titlebar, content, statusbar]
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Title bar
            Constraint::Min(0),    // Content area
            Constraint::Length(3), // Status bar
        ])
        .split(size);

    // Render title bar
    render_titlebar(app, frame, chunks[0])?;

    // Content area layout: [sidebar, main content]
    let sidebar_width = if app.sidebar.collapsed { 0 } else { 20 };
    let content_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_width), Constraint::Min(0)])
        .split(chunks[1]);

    // Render sidebar (if not collapsed)
    if !app.sidebar.collapsed {
        render_sidebar(app, frame, content_chunks[0])?;
    }

    // Render current screen
    let content_area = if app.sidebar.collapsed {
        chunks[1]
    } else {
        content_chunks[1]
    };

    // Extract references before mutable borrow to avoid borrow checker issues
    let paths = &app.paths;
    let config = &app.config;
    app.current_screen
        .render(paths, config, frame, content_area)?;

    // Render status bar
    render_statusbar(app, frame, chunks[2])?;

    // Render dialog (if any) - on top of everything
    if let Some(dialog) = &app.dialog {
        render_dialog(dialog, frame, size)?;
    }

    // Render the keybinding help overlay (if toggled) - on top of everything,
    // including any dialog (the global key handler only allows opening it
    // while no dialog is active, but render order stays defensive).
    if app.help_visible {
        render_help_overlay(&app.current_screen, frame, size)?;
    }

    Ok(())
}
