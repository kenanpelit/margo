//! Active-window bar pill.
//!
//! Render-only — shows the title of the globally focused window
//! next to a window glyph, ellipsized so the pill keeps a sane
//! width; the app class rides along in the tooltip.
//!
//! Focus is read from `margo_service().focused_client`, the
//! authoritative focus signal (resolved from `state.json`'s
//! `focused_idx`). The focused client's `title` / `class` can
//! also change without a focus change — typing in a browser, a
//! tab switch — so those reactives are watched under a
//! `WatcherToken` that's reset whenever focus moves.

use futures::StreamExt;
use mshell_common::{WatcherToken, watch_cancellable};
use mshell_margo_client::Client;
use mshell_services::margo_service;
use relm4::gtk::pango;
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;

pub(crate) struct ActiveWindowModel {
    watcher_token: WatcherToken,
    has_window: bool,
    class: String,
    title: String,
}

#[derive(Debug)]
pub(crate) enum ActiveWindowInput {}

#[derive(Debug)]
pub(crate) enum ActiveWindowOutput {}

pub(crate) struct ActiveWindowInit {}

#[derive(Debug)]
pub(crate) enum ActiveWindowCommandOutput {
    /// The focused client changed — re-subscribe + re-read.
    FocusChanged,
    /// The focused client's title / class changed — re-read.
    WindowMetaChanged,
}

#[relm4::component(pub)]
impl Component for ActiveWindowModel {
    type CommandOutput = ActiveWindowCommandOutput;
    type Input = ActiveWindowInput;
    type Output = ActiveWindowOutput;
    type Init = ActiveWindowInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            set_css_classes: &["active-window-bar-widget", "ok-button-surface", "ok-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 5,
                set_halign: gtk::Align::Center,
                set_valign: gtk::Align::Center,
                set_hexpand: true,
                set_vexpand: true,

                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("screenshot-window-symbolic"),
                },

                #[name = "label"]
                gtk::Label {
                    add_css_class: "active-window-bar-label",
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_ellipsize: pango::EllipsizeMode::End,
                    // Half the media pill's cap (40) — a glanceable
                    // hint, kept from competing with the media pill.
                    set_max_width_chars: 20,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Watch the authoritative `focused_client` reactive. It's
        // change-only (no replay), so subscribe *first* then prime
        // from the current snapshot — whichever side a startup
        // focus update lands on, the pill fills in.
        sender.command(|out, shutdown| {
            async move {
                let mut stream = margo_service().focused_client.watch();
                let _ = out.send(ActiveWindowCommandOutput::FocusChanged);
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                loop {
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        next = stream.next() => match next {
                            Some(_) => {
                                let _ = out.send(ActiveWindowCommandOutput::FocusChanged);
                            }
                            None => break,
                        },
                    }
                }
            }
        });

        let mut model = ActiveWindowModel {
            watcher_token: WatcherToken::new(),
            has_window: false,
            class: String::new(),
            title: String::new(),
        };

        subscribe_focused(&sender, &mut model.watcher_token);

        let widgets = view_output!();
        read_focused(&mut model);
        apply_visual(&widgets, &model);

        ComponentParts { model, widgets }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ActiveWindowCommandOutput::FocusChanged => {
                subscribe_focused(&sender, &mut self.watcher_token);
                read_focused(self);
            }
            ActiveWindowCommandOutput::WindowMetaChanged => {
                read_focused(self);
            }
        }
        apply_visual(widgets, self);
    }
}

fn focused_client() -> Option<Arc<Client>> {
    margo_service().focused_client.get()
}

/// Watch the focused client's `title` + `class` under a fresh
/// `WatcherToken` so live title edits (typing, tab switches)
/// refresh the pill without a focus change.
fn subscribe_focused(
    sender: &ComponentSender<ActiveWindowModel>,
    watcher_token: &mut WatcherToken,
) {
    let token = watcher_token.reset();
    let Some(client) = focused_client() else {
        return;
    };
    let title = client.title.clone();
    let class = client.class.clone();
    watch_cancellable!(sender, token, [title.watch(), class.watch()], |out| {
        let _ = out.send(ActiveWindowCommandOutput::WindowMetaChanged);
    });
}

fn read_focused(model: &mut ActiveWindowModel) {
    match focused_client() {
        Some(client) => {
            model.has_window = true;
            model.class = client.class.get();
            model.title = client.title.get();
        }
        None => {
            model.has_window = false;
            model.class.clear();
            model.title.clear();
        }
    }
}

fn apply_visual(widgets: &ActiveWindowModelWidgets, model: &ActiveWindowModel) {
    if !model.has_window {
        widgets.label.set_label("Desktop");
        widgets.root.set_tooltip_text(Some("No focused window"));
        return;
    }

    let title = if model.title.trim().is_empty() {
        model.class.trim()
    } else {
        model.title.trim()
    };
    let title = if title.is_empty() { "Window" } else { title };

    widgets.label.set_label(title);
    widgets.root.set_tooltip_text(Some(&if model.class.trim().is_empty() {
        title.to_string()
    } else {
        format!("{}  ·  {}", model.class.trim(), title)
    }));
}
