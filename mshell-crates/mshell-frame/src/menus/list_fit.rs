//! How a menu's list scrollers size themselves against the menu's
//! configured `maximum_height` (Settings → Widgets → <menu> → height).
//!
//! Many menus are a header plus one long list (AI Agents, GitHub, Crashes,
//! USB, Discord voice, Podman, Notes, UFW, the CPU dashboard tabs, the
//! network list). The list scrolls on its own so the header stays put, and
//! grows to its content up to a cap. That cap used to be a constant, so a
//! configured height above it changed nothing: the list — and with it the
//! menu — stopped at the constant.
//!
//! The clipboard and notification lists already did this right: their
//! scroller's cap IS the menu's configured height, re-read every time the
//! menu opens. Every other list now follows them. A widget marks its
//! scroller with [`mark_menu_list`] (its built-in cap rides along for menus
//! with no height set); the hosting menu calls [`fit_menu_lists`] whenever
//! it opens or its height setting changes.

use relm4::gtk;
use relm4::gtk::prelude::{Cast, ObjectExt, WidgetExt};

/// CSS class of a marked list scroller.
const LIST_CLASS: &str = "menu-fill-list";
/// Object-data key holding a marked scroller's built-in cap.
const CAP_KEY: &str = "mshell-menu-list-cap";

/// The list's `max_content_height`: the menu's configured height when one is
/// set (0 = not set), else the list's built-in cap.
pub(crate) fn list_cap(menu_max_height: i32, default_cap: i32) -> i32 {
    if menu_max_height > 0 {
        menu_max_height
    } else {
        default_cap
    }
}

/// Mark `sw` as a menu list with built-in cap `default_cap`.
pub(crate) fn mark_menu_list(sw: &gtk::ScrolledWindow, default_cap: i32) {
    sw.add_css_class(LIST_CLASS);
    // SAFETY: the key is only ever read back as `i32` (`cap_of`).
    unsafe { sw.set_data::<i32>(CAP_KEY, default_cap) };
    sw.set_max_content_height(default_cap);
}

fn cap_of(sw: &gtk::ScrolledWindow) -> Option<i32> {
    // SAFETY: `CAP_KEY` is only ever stored as `i32` (`mark_menu_list`).
    unsafe { sw.data::<i32>(CAP_KEY).map(|p| *p.as_ref()) }
}

/// Cap every marked list under `root` at `menu_max_height`.
pub(crate) fn fit_menu_lists(root: &gtk::Widget, menu_max_height: i32) {
    let mut stack = vec![root.clone()];
    while let Some(w) = stack.pop() {
        if w.has_css_class(LIST_CLASS)
            && let Some(sw) = w.downcast_ref::<gtk::ScrolledWindow>()
            && let Some(cap) = cap_of(sw)
        {
            // Never below the list's own minimum: GTK asserts min <= max.
            let max = list_cap(menu_max_height, cap).max(sw.min_content_height());
            sw.set_max_content_height(max);
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            stack.push(c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_configured_height_the_list_keeps_its_cap() {
        assert_eq!(list_cap(0, 560), 560);
    }

    /// A configured height larger than the built-in cap used to change
    /// nothing: the list stopped at 560 px and so did the menu.
    #[test]
    fn a_configured_height_is_the_lists_cap() {
        assert_eq!(list_cap(1000, 560), 1000);
        assert_eq!(list_cap(300, 560), 300);
    }
}
