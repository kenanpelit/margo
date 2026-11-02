//! Margo layout switcher bar pill.
//!
//! Pure trigger button — clicking emits `MargoLayoutOutput::Clicked`
//! which the bar forwards through `BarOutput::MargoLayoutClicked` →
//! `FrameInput::ToggleMenu(MenuId::MargoLayout)`, opening the new in-frame
//! Margo Layout menu (see
//! `menus/menu_widgets/margo_layout/margo_layout_menu_widget.rs`).
//!
//! Replaces the legacy `gtk::PopoverMenu` implementation that
//! spawned an `xdg_popup` window detached from the bar. The new
//! menu lives in the frame's regular menu stack so it slides out
//! contiguous with every other menu surface.
//!
//! The icon reflects whichever layout is currently active on the
//! focused output (pushed by margo's `watch state` stream through
//! `margo_service().active_layout`) so the pill is glanceable:
//! at-a-cursor you can read the layout without opening the menu.

use futures::StreamExt;
use mshell_margo_client::ActiveLayout;
use mshell_services::margo_service;
use relm4::{Component, ComponentParts, ComponentSender, gtk, gtk::Orientation, gtk::prelude::*};

pub(crate) struct MargoLayoutModel {
    /// Last-seen active layout. Skips GTK calls when nothing changed.
    last_active: Option<ActiveLayout>,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutCommandOutput {
    /// The focused output's layout changed.
    LayoutChanged(Option<ActiveLayout>),
}

#[derive(Debug)]
pub(crate) enum MargoLayoutInput {
    /// User clicked the pill — propagate via output to the bar +
    /// frame, which opens / closes the layout menu.
    Clicked,
}

#[derive(Debug)]
pub(crate) enum MargoLayoutOutput {
    /// Bar listens for this and emits
    /// `BarOutput::MargoLayoutClicked`. Frame then toggles the
    /// in-stack `MARGO_LAYOUT_MENU` surface.
    Clicked,
}

pub(crate) struct MargoLayoutInit {
    /// Kept on the init for bar-side API parity (every bar widget
    /// receives the bar's orientation). The pill itself is a
    /// single icon button so orientation doesn't affect layout
    /// here.
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub(crate))]
impl Component for MargoLayoutModel {
    type CommandOutput = MargoLayoutCommandOutput;
    type Input = MargoLayoutInput;
    type Output = MargoLayoutOutput;
    type Init = MargoLayoutInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "margo-layout-bar-widget",
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            #[name = "button"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                set_icon_name: "view-list-symbolic",
                connect_clicked[sender] => move |_| {
                    sender.input(MargoLayoutInput::Clicked);
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();

        // Seed the icon from the service's current snapshot so the pill
        // never flashes the generic placeholder when the layout is known.
        let initial = margo_service().active_layout.get();
        if let Some(layout) = &initial {
            widgets.button.set_icon_name(icon_for_layout(&layout.name));
        }

        // Event-driven: margo pushes a state frame on every change and the
        // service mirrors the focused output's layout. The watch is
        // change-only, so subscribe first, then re-prime once to close the
        // startup race. Ends with the component (relm4 shutdown).
        sender.command(|out, shutdown| async move {
            let mut stream = margo_service().active_layout.watch();
            let _ = out.send(MargoLayoutCommandOutput::LayoutChanged(
                margo_service().active_layout.get(),
            ));
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                tokio::select! {
                    () = &mut shutdown_fut => break,
                    next = stream.next() => match next {
                        Some(layout) => {
                            let _ = out.send(MargoLayoutCommandOutput::LayoutChanged(layout));
                        }
                        None => break,
                    },
                }
            }
        });

        // `params.orientation` is part of the bar-widget init
        // ABI but isn't used here — the pill is a single icon.
        let _ = params.orientation;
        let model = MargoLayoutModel {
            last_active: initial,
        };

        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoLayoutInput::Clicked => {
                let _ = sender.output(MargoLayoutOutput::Clicked);
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        _sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoLayoutCommandOutput::LayoutChanged(next) => {
                // `None` is transient (mid-reload): keep the last icon.
                if next.is_some() && self.last_active != next {
                    if let Some(layout) = &next {
                        widgets.button.set_icon_name(icon_for_layout(&layout.name));
                    }
                    self.last_active = next;
                }
            }
        }
    }
}

fn icon_for_layout(id: &str) -> &'static str {
    match id {
        "tile" => "layout-tile-symbolic",
        "scroller" => "layout-scrolling-symbolic",
        "grid" => "layout-grid-symbolic",
        "monocle" => "layout-monocle-symbolic",
        "deck" => "layout-deck-symbolic",
        "center_tile" => "layout-center-symbolic",
        "right_tile" => "layout-right-symbolic",
        "tgmix" => "layout-mix-symbolic",
        "dwindle" => "layout-dwindle-symbolic",
        // No dedicated icon packaged for "floating" or "mosaic" yet —
        // same generic fallback both already relied on.
        _ => "view-list-symbolic",
    }
}
