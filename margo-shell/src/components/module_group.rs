use crate::{config::AppearanceStyle, theme::use_theme};
use iced::{Border, Color, Element, widget::container};

/// Wraps content with the appropriate bar style container.
///
/// - `Solid | Gradient` → pass through as-is
/// - `Islands` → wrap in a container with background color + rounded border.
///   `appearance.show_outline = true` ise primary renkten ince bir outline
///   eklenir (noctalia capsule görünümü).
pub fn module_group<'a, Msg: 'static>(content: Element<'a, Msg>) -> Element<'a, Msg> {
    let (bar_style, opacity, radius, show_outline) = use_theme(|theme| {
        (
            theme.bar_style,
            theme.opacity,
            theme.radius,
            theme.show_outline,
        )
    });

    match bar_style {
        AppearanceStyle::Solid | AppearanceStyle::Gradient => content,
        AppearanceStyle::Islands => container(content)
            .style(move |iced_theme: &iced::Theme| {
                let (border_width, border_color) = if show_outline {
                    (1.0, iced_theme.palette().primary.scale_alpha(0.45))
                } else {
                    (0.0, Color::TRANSPARENT)
                };
                container::Style {
                    background: Some(
                        iced_theme.palette().background.scale_alpha(opacity).into(),
                    ),
                    border: Border {
                        width: border_width,
                        radius: radius.md.into(),
                        color: border_color,
                    },
                    ..container::Style::default()
                }
            })
            .into(),
    }
}
