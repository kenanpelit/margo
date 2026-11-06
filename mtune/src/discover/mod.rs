// SPDX-License-Identifier: GPL-3.0-or-later

//! The "Radio & Podcasts" dialog (main menu → Radio & Podcasts…, Ctrl+R):
//! a view switcher over the station directory, the user's own stations and
//! podcasts. Everything it plays goes through the normal queue, so MPRIS,
//! the tray and the shell's Tune pill follow it like any track.

mod art;
mod episodes_page;
pub(crate) mod play;
mod podcasts_page;
mod radio_page;
mod rows;
pub(crate) mod shared;
mod stations_page;
pub mod tracker;

use std::{cell::RefCell, rc::Rc};

use adw::prelude::*;
use gtk::glib;

use crate::{i18n::i18n, window::Window};

pub use shared::flush;

thread_local! {
    /// The open dialog's toast overlay: the window's own toasts sit under
    /// the dialog, so they are shown here while it is up.
    static OPEN: RefCell<Option<glib::WeakRef<adw::ToastOverlay>>> = const { RefCell::new(None) };
}

/// Where a window toast should go while the dialog is open.
pub fn open_toasts() -> Option<adw::ToastOverlay> {
    OPEN.with(|o| o.borrow().as_ref().and_then(glib::WeakRef::upgrade))
}

/// What every page needs: the window to queue into, a toast target, and
/// the listener ids to drop when the dialog closes.
pub struct Ctx {
    win: glib::WeakRef<Window>,
    toasts: adw::ToastOverlay,
    ids: RefCell<Vec<u64>>,
}

impl Ctx {
    pub fn window(&self) -> Option<Window> {
        self.win.upgrade()
    }

    pub fn toast(&self, msg: &str) {
        // Plain text: messages carry station / episode names.
        let toast = adw::Toast::new(msg);
        toast.set_use_markup(false);
        self.toasts.add_toast(toast);
    }

    /// Refresh on library changes for as long as the dialog is open.
    pub fn listen(&self, f: impl Fn(shared::Change) + 'static) {
        self.ids.borrow_mut().push(shared::listen(f));
    }

    /// Toast once when a library file could not be read (it is then never
    /// saved over, so changes would silently vanish).
    fn warn_unwritable(&self) {
        let file = if !shared::radio().writable {
            Some(crate::radio::store::RadioLibrary::path())
        } else if !shared::podcasts().writable {
            Some(crate::podcasts::store::PodcastLibrary::path())
        } else {
            None
        };
        if let Some(path) = file {
            self.toast(&crate::i18n::i18n_k(
                // Translators: `{file}` is a path; leave it untranslated.
                "{file} does not parse; changes will not be saved until it is fixed",
                &[("file", &path.display().to_string())],
            ));
        }
    }
}

/// Open the dialog over `win`.
pub fn present(win: &Window) {
    let toasts = adw::ToastOverlay::new();
    let ctx = Rc::new(Ctx {
        win: win.downgrade(),
        toasts: toasts.clone(),
        ids: RefCell::new(Vec::new()),
    });

    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(
        &radio_page::build(&ctx),
        Some("radio"),
        &i18n("Radio"),
        "network-wireless-symbolic",
    );
    stack.add_titled_with_icon(
        &stations_page::build(&ctx),
        Some("stations"),
        &i18n("My Stations"),
        "starred-symbolic",
    );
    stack.add_titled_with_icon(
        &podcasts_page::build(&ctx),
        Some("podcasts"),
        &i18n("Podcasts"),
        "application-rss+xml-symbolic",
    );

    let header = adw::HeaderBar::new();
    header.set_title_widget(Some(
        &adw::ViewSwitcher::builder()
            .stack(&stack)
            .policy(adw::ViewSwitcherPolicy::Wide)
            .build(),
    ));

    toasts.set_child(Some(&stack));
    let view = adw::ToolbarView::new();
    view.add_top_bar(&header);
    view.set_content(Some(&toasts));

    let dialog = adw::Dialog::builder()
        .title(i18n("Radio & Podcasts"))
        .content_width(620)
        .content_height(700)
        .child(&view)
        .build();
    dialog.add_css_class("discover");
    OPEN.with(|o| o.replace(Some(toasts.downgrade())));
    let c = ctx.clone();
    dialog.connect_closed(move |_| {
        OPEN.with(|o| o.replace(None));
        for id in c.ids.take() {
            shared::unlisten(id);
        }
    });
    dialog.present(Some(win));
    ctx.warn_unwritable();
}
