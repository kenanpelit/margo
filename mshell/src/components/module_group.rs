use crate::{config::AppearanceStyle, theme::use_theme};
use iced::{Border, Color, Element, Shadow, Vector, widget::container};

/// Wraps content with the appropriate bar style container.
///
/// - `Solid | Gradient` → pass through as-is
/// - `Islands` → wrap in a rounded capsule with subtle drop shadow.
///   `appearance.show_outline = true` overlays a thin accent border
///   (noctalia capsule look).
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
                    // Hairline soft border even without outline — gives
                    // the capsule a defined edge against the wallpaper
                    // without needing the accent treatment.
                    (
                        1.0,
                        iced_theme
                            .extended_palette()
                            .background
                            .weak
                            .color
                            .scale_alpha(0.55),
                    )
                };
                container::Style {
                    background: Some(
                        iced_theme.palette().background.scale_alpha(opacity).into(),
                    ),
                    border: Border {
                        width: border_width,
                        radius: radius.lg.into(),
                        color: border_color,
                    },
                    shadow: Shadow {
                        color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.22 },
                        offset: Vector::new(0.0, 2.0),
                        blur_radius: 10.0,
                    },
                    ..container::Style::default()
                }
            })
            .into(),
    }
}
