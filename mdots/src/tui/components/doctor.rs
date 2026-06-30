use anyhow::Result;
use ratatui::{
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

use super::centered_rect;
use crate::commands::doctor::{summarize, Check, Health};

/// Render the modal doctor health-check overlay (opened with `D`) on top of
/// everything. Groups the gathered [`Check`]s by area exactly like the CLI
/// `mdots doctor` report, colours each by [`Health`], and scrolls with the
/// given offset. The summary plus scroll/close hints live in the top border
/// so they stay visible no matter how far the body is scrolled.
pub fn render_doctor_overlay(
    checks: &[Check],
    scroll: u16,
    frame: &mut Frame,
    area: Rect,
) -> Result<()> {
    let popup_area = centered_rect(75, 80, area);
    frame.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::new();
    let mut current_area = "";
    for check in checks {
        if check.area != current_area {
            if !current_area.is_empty() {
                lines.push(Line::from(""));
            }
            current_area = check.area;
            lines.push(Line::from(Span::styled(
                check.area.to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )));
        }

        let (mark, color) = match check.status {
            Health::Ok => ("✓", Color::Green),
            Health::Warn => ("!", Color::Yellow),
            Health::Fail => ("✗", Color::Red),
        };
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{mark} "), Style::default().fg(color)),
            Span::styled(
                check.name.clone(),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]));
        if !check.detail.is_empty() {
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled(check.detail.clone(), Style::default().fg(Color::Gray)),
            ]));
        }
    }

    if checks.is_empty() {
        lines.push(Line::from(Span::styled(
            "No checks ran.",
            Style::default().fg(Color::DarkGray),
        )));
    }

    let (ok, warn, fail, _code) = summarize(checks);
    let border_color = if fail > 0 {
        Color::Red
    } else if warn > 0 {
        Color::Yellow
    } else {
        Color::Green
    };
    let title = format!(
        " Doctor — {ok} ok · {warn} warn · {fail} fail   [j/k ↑/↓] scroll  [Esc/D/q] close "
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .alignment(Alignment::Left)
        .wrap(Wrap { trim: true })
        .scroll((scroll, 0));

    frame.render_widget(paragraph, popup_area);
    Ok(())
}
