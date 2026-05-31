//! Active-window bar pill.
//!
//! Render-only — shows the title of the globally focused window
//! next to a window glyph. The title rides in a width-capped scroller
//! that marquees long titles (scroll → dwell → snap back) instead of
//! ellipsizing; the app class rides along in the tooltip.
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
use relm4::gtk::glib;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// Marquee tick cadence + per-tick step + end dwell (in ticks).
const MARQUEE_INTERVAL: Duration = Duration::from_millis(30);
const MARQUEE_STEP_PX: f64 = 2.0;
const MARQUEE_PAUSE_TICKS: u32 = 50;

/// One-directional title-marquee phases: dwell at the start, scroll to the
/// end, dwell, snap back, repeat. (Mirrors the media-player track marquee.)
#[derive(Clone, Copy)]
enum ScrollState {
    PauseStart(u32),
    Scrolling,
    PauseEnd(u32),
}

pub(crate) struct ActiveWindowModel {
    watcher_token: WatcherToken,
    has_window: bool,
    class: String,
    title: String,
    /// Keeps the marquee timer alive for the widget's lifetime.
    #[allow(dead_code)]
    marquee_source: Option<glib::SourceId>,
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

                // The title rides in a capped scroller: short titles size to
                // fit, long ones cap at `max_content_width` and the tick
                // callback marquees the full text instead of ellipsizing.
                #[name = "marquee_scroller"]
                gtk::ScrolledWindow {
                    add_css_class: "active-window-marquee",
                    set_hscrollbar_policy: gtk::PolicyType::External,
                    set_vscrollbar_policy: gtk::PolicyType::Never,
                    set_propagate_natural_width: true,
                    set_propagate_natural_height: true,
                    set_min_content_width: 0,
                    // ~20 chars at the bar font — the old ellipsize cap.
                    set_max_content_width: 200,
                    set_valign: gtk::Align::Center,

                    #[name = "label"]
                    gtk::Label {
                        add_css_class: "active-window-bar-label",
                        set_halign: gtk::Align::Start,
                        set_valign: gtk::Align::Center,
                        set_single_line_mode: true,
                    },
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
        sender.command(|out, shutdown| async move {
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
        });

        let mut model = ActiveWindowModel {
            watcher_token: WatcherToken::new(),
            has_window: false,
            class: String::new(),
            title: String::new(),
            marquee_source: None,
        };

        subscribe_focused(&sender, &mut model.watcher_token);

        let widgets = view_output!();
        model.marquee_source = Some(start_scroll(&widgets.marquee_scroller));
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
    // The label is about to change — snap the marquee back to the start so
    // the new title reads from its beginning rather than mid-scroll.
    widgets.marquee_scroller.hadjustment().set_value(0.0);

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
    widgets
        .root
        .set_tooltip_text(Some(&if model.class.trim().is_empty() {
            title.to_string()
        } else {
            format!("{}  ·  {}", model.class.trim(), title)
        }));
}

/// Start the title marquee: a periodic timer that scrolls the capped
/// scroller from start → end with a dwell at each end, then snaps back.
/// Idle (a cheap early-return) while the title fits. Returns the source id
/// so the model can keep it alive for the widget's lifetime.
fn start_scroll(scrolled_window: &gtk::ScrolledWindow) -> glib::SourceId {
    let state = Rc::new(Cell::new(ScrollState::PauseStart(0)));
    let scroll = scrolled_window.clone();
    glib::timeout_add_local(MARQUEE_INTERVAL, move || {
        let adj = scroll.hadjustment();
        let max = adj.upper() - adj.page_size();
        if max <= 0.0 {
            return glib::ControlFlow::Continue;
        }
        match state.get() {
            ScrollState::PauseStart(n) => {
                if n >= MARQUEE_PAUSE_TICKS {
                    state.set(ScrollState::Scrolling);
                } else {
                    state.set(ScrollState::PauseStart(n + 1));
                }
            }
            ScrollState::Scrolling => {
                let current = adj.value();
                if current >= max {
                    state.set(ScrollState::PauseEnd(0));
                } else {
                    adj.set_value(current + MARQUEE_STEP_PX);
                }
            }
            ScrollState::PauseEnd(n) => {
                if n >= MARQUEE_PAUSE_TICKS {
                    adj.set_value(0.0);
                    state.set(ScrollState::PauseStart(0));
                } else {
                    state.set(ScrollState::PauseEnd(n + 1));
                }
            }
        }
        glib::ControlFlow::Continue
    })
}
