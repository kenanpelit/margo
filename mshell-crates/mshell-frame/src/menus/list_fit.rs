//! How a menu's list scrollers size themselves against the menu's
//! configured `maximum_height` (Settings → Widgets → <menu> → height).
//!
//! Many menus are a header plus one long list (AI Agents, GitHub, Crashes,
//! USB, Discord voice, Podman, Notes, UFW, the CPU dashboard tabs, the
//! network list). The configured height is the height of the whole menu:
//! the menu's outer scroller is capped at it (`menu.rs`), and when the
//! content is taller GTK hands the list only the room the header, tabs and
//! footer leave. The outer scroller does not scroll; the list scrolls on its
//! own, so the header stays put.
//!
//! That works because a `GtkViewport` (the outer scroller's child wrapper)
//! allocates its child at `max(viewport, child minimum)` — its default
//! `GTK_SCROLL_MINIMUM` policy — and a list scroller's minimum is tiny
//! (`min_content_height: 0`), so the surrounding `gtk::Box` keeps the header
//! at its natural height and squeezes the list. The outer scroller only
//! scrolls when even the minimums don't fit: a height below a list's own
//! `min_content_height` floor (Notes: 220-240 px) plus its header. Checked
//! with a GTK 4.24 layout probe of this exact shape (header + list scroller
//! + footer inside a width-pinned, height-capped outer scroller): outer
//! never scrolls, header stays at the top, list scrolls inside.
//!
//! So the list's own cap only decides how tall the menu grows when NO height
//! is set: each list carries a built-in cap, and an unset height keeps it.
//! With a height set the list may use all of it — a cap below the setting
//! would stop the menu short of it (the 4.9.1 bug). The exception is a list
//! embedded in a bigger host (the network and Podman lists inside the
//! Control Center, anything in mdash): the host's height is for the host, so
//! an embedded list keeps its built-in cap there
//! ([`lists_follow_menu_height`]).
//!
//! A widget marks its scroller with [`mark_menu_list`] from its `init`,
//! which runs while the hosting menu builds its content inside
//! [`MenuLists::collect`]; the menu then re-caps exactly those scrollers,
//! and only when the height (or the follow decision) actually changed — no
//! widget-tree walk on open.

use mshell_config::schema::menu_widgets::MenuWidget;
use relm4::gtk;
use relm4::gtk::glib;
use relm4::gtk::prelude::ObjectExt;
use std::cell::RefCell;

/// The list's `max_content_height`: the menu's configured height when one is
/// set (0 = not set) and the list is the menu's own content, else the list's
/// built-in cap.
pub(crate) fn list_cap(menu_max_height: i32, default_cap: i32, follows_menu: bool) -> i32 {
    if follows_menu && menu_max_height > 0 {
        menu_max_height
    } else {
        default_cap
    }
}

/// Whether the lists in a menu built from `kinds` follow the menu's height.
/// True when every configured widget is a list widget (the Crashes menu is
/// `[Crashes]`, the network menu `[Network]`, …); false for hosts that embed
/// a list among other content (Control Center, mdash, a user's mixed menu).
pub(crate) fn lists_follow_menu_height(kinds: &[MenuWidget]) -> bool {
    !kinds.is_empty()
        && kinds.iter().all(|k| {
            matches!(
                k,
                MenuWidget::Agents
                    | MenuWidget::Github
                    | MenuWidget::HostMonitor
                    | MenuWidget::Crashes
                    | MenuWidget::Usb
                    | MenuWidget::DiscordVoice
                    | MenuWidget::Podman
                    | MenuWidget::Notes
                    | MenuWidget::Ufw
                    | MenuWidget::Network
                    | MenuWidget::CpuDashboard
            )
        })
}

/// What a fit was computed for; a fit with the same key changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FitKey {
    menu_max_height: i32,
    follows_menu: bool,
}

/// `true` when the lists must be re-capped: never fitted, or the height /
/// follow decision differs from the last fit.
fn needs_fit(last: Option<FitKey>, now: FitKey) -> bool {
    last != Some(now)
}

struct MarkedList {
    sw: glib::WeakRef<gtk::ScrolledWindow>,
    default_cap: i32,
}

thread_local! {
    /// The list registry of the menu whose content is being built right now
    /// ([`MenuLists::collect`]); `None` outside a build.
    static COLLECTING: RefCell<Option<Vec<MarkedList>>> = const { RefCell::new(None) };
}

/// Mark `sw` as a menu list with built-in cap `default_cap`. Call it from the
/// widget's `init`; the menu being built registers it.
pub(crate) fn mark_menu_list(sw: &gtk::ScrolledWindow, default_cap: i32) {
    sw.set_max_content_height(default_cap.max(sw.min_content_height()));
    let registered = COLLECTING.with(|c| match c.borrow_mut().as_mut() {
        Some(lists) => {
            lists.push(MarkedList {
                sw: sw.downgrade(),
                default_cap,
            });
            true
        }
        None => false,
    });
    if !registered {
        // Built outside a menu content build: the list keeps its own cap.
        tracing::debug!(default_cap, "menu list marked outside a menu build");
    }
}

/// The marked lists of one menu's content.
#[derive(Default)]
pub(crate) struct MenuLists {
    lists: Vec<MarkedList>,
    fitted: Option<FitKey>,
}

impl MenuLists {
    /// Run `build` (the menu's content build) and register every list it
    /// marks.
    pub(crate) fn collect(build: impl FnOnce()) -> Self {
        let outer = COLLECTING.with(|c| c.borrow_mut().replace(Vec::new()));
        build();
        let lists = COLLECTING.with(|c| {
            let mut slot = c.borrow_mut();
            let lists = slot.take().unwrap_or_default();
            *slot = outer;
            lists
        });
        Self {
            lists,
            fitted: None,
        }
    }

    /// Cap every registered list for `menu_max_height`. Returns how many
    /// scrollers were touched: 0 when nothing changed since the last fit.
    pub(crate) fn fit(&mut self, menu_max_height: i32, follows_menu: bool) -> usize {
        let key = FitKey {
            menu_max_height,
            follows_menu,
        };
        if !needs_fit(self.fitted, key) {
            return 0;
        }
        self.fitted = Some(key);
        let mut touched = 0;
        for list in &self.lists {
            if let Some(sw) = list.sw.upgrade() {
                // Never below the list's own minimum: GTK asserts min <= max.
                let max = list_cap(menu_max_height, list.default_cap, follows_menu)
                    .max(sw.min_content_height());
                sw.set_max_content_height(max);
                touched += 1;
            }
        }
        touched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_config::schema::config::Menus;

    #[test]
    fn without_a_configured_height_the_list_keeps_its_cap() {
        assert_eq!(list_cap(0, 560, true), 560);
        assert_eq!(list_cap(0, 560, false), 560);
    }

    /// A configured height larger than the built-in cap used to change
    /// nothing: the list stopped at 560 px and so did the menu.
    #[test]
    fn a_configured_height_is_the_lists_cap() {
        assert_eq!(list_cap(1000, 560, true), 1000);
        assert_eq!(list_cap(300, 560, true), 300);
    }

    /// The Control Center's 720 px is the Control Center's height; the
    /// network list inside it keeps its own 420 px cap, as before 4.9.2.
    #[test]
    fn a_host_menu_keeps_the_lists_built_in_cap() {
        assert_eq!(list_cap(720, 420, false), 420);
    }

    #[test]
    fn single_list_menus_follow_their_height() {
        let m = Menus::default();
        for kinds in [
            &m.agents_menu.widgets,
            &m.github_menu.widgets,
            &m.host_monitor_menu.widgets,
            &m.crashes_menu.widgets,
            &m.usb_menu.widgets,
            &m.discord_voice_menu.widgets,
            &m.podman_menu.widgets,
            &m.notes_menu.widgets,
            &m.ufw_menu.widgets,
            &m.network_menu.widgets,
            &m.cpu_dashboard_menu.widgets,
        ] {
            assert!(lists_follow_menu_height(kinds), "{kinds:?}");
        }
    }

    #[test]
    fn hosts_and_mixed_menus_do_not() {
        let m = Menus::default();
        assert!(!lists_follow_menu_height(&m.control_center_menu.widgets));
        assert!(!lists_follow_menu_height(&m.mdash_menu.widgets));
        assert!(!lists_follow_menu_height(&[]));
        assert!(!lists_follow_menu_height(&[
            MenuWidget::Network,
            MenuWidget::Bluetooth
        ]));
    }

    /// The fit used to walk the whole widget tree on every open. Now a
    /// repeated open (or a config re-notify with the same height) is free.
    #[test]
    fn an_unchanged_height_needs_no_refit() {
        let k = |menu_max_height, follows_menu| FitKey {
            menu_max_height,
            follows_menu,
        };
        assert!(needs_fit(None, k(0, true)));
        assert!(!needs_fit(Some(k(640, true)), k(640, true)));
        assert!(needs_fit(Some(k(640, true)), k(800, true)));
        assert!(needs_fit(Some(k(640, true)), k(640, false)));
    }

    #[test]
    fn a_second_fit_with_the_same_height_touches_nothing() {
        let mut lists = MenuLists::collect(|| {});
        assert_eq!(lists.fit(640, true), 0, "no lists registered");
        assert_eq!(
            lists.fitted,
            Some(FitKey {
                menu_max_height: 640,
                follows_menu: true
            })
        );
        assert_eq!(lists.fit(640, true), 0);
    }

    #[test]
    fn collect_restores_an_enclosing_collection() {
        COLLECTING.with(|c| assert!(c.borrow().is_none()));
        let _outer = MenuLists::collect(|| {
            let inner = MenuLists::collect(|| {});
            assert!(inner.lists.is_empty());
            COLLECTING.with(|c| assert!(c.borrow().is_some(), "outer collection lost"));
        });
        COLLECTING.with(|c| assert!(c.borrow().is_none()));
    }
}
