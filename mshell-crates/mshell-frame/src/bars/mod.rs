pub(crate) mod bar;
pub(crate) mod bar_widgets;
pub(crate) mod pill_classes;

/// DESIGN.md §4 pill anatomy for the pills new code copies from: the pill
/// surface (`ok-button-surface ok-bar-widget`) sits on the root Box, the
/// inner click Button is `ok-button-flat`.
#[cfg(test)]
mod anatomy_tests {
    const PILLS: &[(&str, &str)] = &[
        ("keymode", include_str!("bar_widgets/keymode.rs")),
        ("usb", include_str!("bar_widgets/usb.rs")),
        (
            "discord_voice",
            include_str!("bar_widgets/discord_voice.rs"),
        ),
        (
            "keyboard_layout",
            include_str!("bar_widgets/keyboard_layout.rs"),
        ),
    ];

    #[test]
    fn pill_surface_is_on_the_root_and_the_button_is_flat() {
        for (name, src) in PILLS {
            let root = src.split("#[root]").nth(1).expect(name);
            let (root_props, rest) = root.split_once("gtk::Button {").expect(name);
            assert!(
                root_props.contains("\"ok-button-surface\"")
                    && root_props.contains("\"ok-bar-widget\""),
                "{name}: the root carries the pill surface"
            );
            let button_classes = rest
                .lines()
                .find(|l| l.contains("set_css_classes"))
                .expect(name);
            assert!(
                button_classes.contains("&[\"ok-button-flat\"]"),
                "{name}: the inner button is ok-button-flat, got {button_classes}"
            );
        }
    }
}
