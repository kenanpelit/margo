//! How a menu's own list scroller sizes itself against the menu's
//! configured `maximum_height` (Settings → Widgets → <menu> → height).
//!
//! Menus whose content is a header + one long list (AI Agents, GitHub,
//! Crashes) keep the header fixed and scroll only the list. With no height
//! configured the list grows to its content up to a built-in cap. With a
//! height configured the menu is pinned to that height (`fixed_height` in
//! `menu.rs`) and the list fills whatever the header leaves — a built-in cap
//! would leave the menu short of the configured size or empty below the list.

/// Scroller settings for one list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListFit {
    /// `set_max_content_height`; `-1` = no cap.
    pub(crate) max_content_height: i32,
    /// `set_propagate_natural_height`: report the content's height upward
    /// (grow to fit) instead of taking the space the menu gives.
    pub(crate) propagate_natural_height: bool,
    /// `set_vexpand`: take the space left below the header.
    pub(crate) vexpand: bool,
}

/// The fit for a list whose built-in cap is `default_cap`, in a menu whose
/// configured `maximum_height` is `menu_max_height` (0 = not set).
pub(crate) fn list_fit(menu_max_height: i32, default_cap: i32) -> ListFit {
    if menu_max_height > 0 {
        ListFit {
            max_content_height: -1,
            propagate_natural_height: false,
            vexpand: true,
        }
    } else {
        ListFit {
            max_content_height: default_cap,
            propagate_natural_height: true,
            vexpand: false,
        }
    }
}

/// Apply [`list_fit`] to `sw`.
pub(crate) fn fit_scroller(
    sw: &relm4::gtk::ScrolledWindow,
    menu_max_height: i32,
    default_cap: i32,
) {
    use relm4::gtk::prelude::WidgetExt;
    let fit = list_fit(menu_max_height, default_cap);
    // min_content_height stays 0, so any max (or -1) keeps min <= max.
    sw.set_max_content_height(fit.max_content_height);
    sw.set_propagate_natural_height(fit.propagate_natural_height);
    sw.set_vexpand(fit.vexpand);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_configured_height_the_list_grows_to_its_cap() {
        let fit = list_fit(0, 560);
        assert_eq!(fit.max_content_height, 560);
        assert!(fit.propagate_natural_height);
        assert!(!fit.vexpand);
    }

    /// A configured height larger than the built-in cap used to change
    /// nothing: the list stopped at 560 px and so did the menu.
    #[test]
    fn a_configured_height_lets_the_list_fill_the_menu() {
        for h in [300, 560, 1000] {
            let fit = list_fit(h, 560);
            assert_eq!(fit.max_content_height, -1, "height {h}");
            assert!(!fit.propagate_natural_height);
            assert!(fit.vexpand);
        }
    }
}
