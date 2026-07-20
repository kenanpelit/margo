pub mod dialog;
pub mod doctor;
pub mod help;
pub mod palette;
pub mod sidebar;
pub mod statusbar;
pub mod titlebar;

pub use dialog::render_dialog;
pub use doctor::render_doctor_overlay;
pub use help::render_help_overlay;
pub use palette::render_palette;
pub use sidebar::render_sidebar;
pub use statusbar::render_statusbar;
pub use titlebar::render_titlebar;

/// Compute a rectangle centered inside `r`, sized as a percentage of its
/// width/height. Shared by the dialog and help overlays so every popup in
/// this TUI is centered the same way.
pub(crate) fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    r: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    use ratatui::layout::{Constraint, Direction, Layout};

    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
