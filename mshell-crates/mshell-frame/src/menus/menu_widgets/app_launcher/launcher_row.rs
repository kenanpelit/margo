//! Generic row component for the unified launcher result list.
//!
//! Renders any [`DisplayItem`] uniformly: small numeric
//! quick-activate hint, 32 px icon, bold name, greyed description,
//! optional ★ pin marker, hover highlight, selection ring. The
//! activation callback baked into the item is invoked on click —
//! the row doesn't know whether it's launching an app, copying a
//! calculator result or jumping to a Settings tab.
//!
//! Apps get the matugen-filtered icon treatment so they look
//! visually consistent with everything else in mshell: the row
//! inspects the `LauncherItem.id` prefix, and for `apps:<entry>`
//! ids it resolves the desktop entry and calls `set_icon` (the
//! same helper the legacy app launcher used).

use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, IconsStoreFields, ThemeStoreFields};
use mshell_launcher::{DisplayItem, LauncherItem};
use mshell_utils::app_icon::app_icon::set_icon;
use reactive_graph::traits::GetUntracked;
use relm4::gtk::gio::DesktopAppInfo;
use relm4::gtk::prelude::*;
use relm4::gtk::{gio, pango};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// One row in the result list.
pub(crate) struct LauncherRowModel {
    item: LauncherItem,
    /// Pin flag stamped by the runtime — drives the ★ glyph.
    pinned: bool,
    /// `"1".."9"` quick-activate digit, or empty string for rows
    /// past the first nine.
    quick_key: String,
    is_selected: bool,
}

#[derive(Debug)]
pub(crate) enum LauncherRowInput {
    /// Sent by the parent on arrow-key nav. The row compares
    /// against `self.item.id` and toggles its highlight class.
    SelectionChanged(String),
    /// Sent by the parent when a pin / unpin happens so the ★ can
    /// repaint without rebuilding the whole row controller.
    PinChanged(bool),
    /// Fired by the inner GtkButton's `clicked` signal.
    Activate,
}

#[derive(Debug)]
pub(crate) enum LauncherRowOutput {
    /// Forwarded to the launcher widget so it can dispatch the
    /// item's `on_activate` closure (the runtime is the only side
    /// that holds the closure + the frecency store).
    Activated(String),
}

pub(crate) struct LauncherRowInit {
    pub display: DisplayItem,
}

#[relm4::component(pub(crate))]
impl Component for LauncherRowModel {
    type CommandOutput = ();
    type Input = LauncherRowInput;
    type Output = LauncherRowOutput;
    type Init = LauncherRowInit;

    view! {
        #[root]
        #[name = "button"]
        gtk::Button {
            #[watch]
            set_css_classes: if model.is_selected {
                &["ok-button-surface", "app-launcher-item", "selected"]
            } else {
                &["ok-button-surface", "app-launcher-item"]
            },
            set_vexpand: false,
            set_hexpand: true,
            set_can_focus: false,
            connect_clicked[sender] => move |_| {
                sender.input(LauncherRowInput::Activate);
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,

                #[name = "quick_key_label"]
                gtk::Label {
                    add_css_class: "app-launcher-quick-key",
                    set_label: &model.quick_key,
                    set_visible: !model.quick_key.is_empty(),
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_margin_end: 8,
                    set_width_chars: 1,
                },

                #[name = "image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_margin_end: 12,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    set_hexpand: true,

                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_label: &model.item.name,
                        set_halign: gtk::Align::Start,
                        set_ellipsize: pango::EllipsizeMode::End,
                    },

                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: &model.item.description,
                        set_visible: !model.item.description.is_empty(),
                        set_halign: gtk::Align::Start,
                        set_ellipsize: pango::EllipsizeMode::End,
                    },
                },

                // ★ pin marker — always present in the layout (so the
                // row width stays stable across pin/unpin) but only
                // visible when `pinned`. CSS class drives a subtle
                // accent tint so the star reads as a state badge.
                #[name = "pin_marker"]
                gtk::Label {
                    add_css_class: "app-launcher-pin-marker",
                    set_label: "\u{2605}",
                    #[watch]
                    set_visible: model.pinned,
                    set_halign: gtk::Align::End,
                    set_valign: gtk::Align::Center,
                    set_margin_start: 8,
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let DisplayItem { item, pinned, quick_key } = params.display;
        let model = LauncherRowModel {
            item,
            pinned,
            quick_key,
            is_selected: false,
        };

        let widgets = view_output!();

        apply_icon(&widgets.image, &model.item);
        // `root` is held by the view's `#[root]` reference; the
        // local binding is a small unused alias the macro
        // generates, which we silence here.
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            LauncherRowInput::SelectionChanged(selected_id) => {
                self.is_selected = selected_id == self.item.id;
            }
            LauncherRowInput::PinChanged(pinned) => {
                self.pinned = pinned;
            }
            LauncherRowInput::Activate => {
                let _ = sender.output(LauncherRowOutput::Activated(self.item.id.clone()));
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Resolve the right icon for a launcher item. App entries get the
/// full matugen-filtered treatment via `set_icon`; everything else
/// falls back to a plain `set_icon_name`.
fn apply_icon(image: &gtk::Image, item: &LauncherItem) {
    if let Some(app_id) = item.id.strip_prefix("apps:") {
        // The id matches a `.desktop` file; gio caches its lookup
        // so this is essentially free.
        if let Some(info) = DesktopAppInfo::new(app_id) {
            apply_app_icon(image, info);
            return;
        }
        if let Some(info) = lookup_app_by_id_suffix(app_id) {
            apply_app_icon(image, info);
            return;
        }
    }
    image.set_icon_name(Some(&item.icon));
}

/// Slow-path lookup: when `DesktopAppInfo::new("firefox.desktop")`
/// fails (some entries are exposed under different ids), walk the
/// full app list and pick the first whose id ends with our suffix.
fn lookup_app_by_id_suffix(suffix: &str) -> Option<DesktopAppInfo> {
    gio::AppInfo::all()
        .into_iter()
        .filter_map(|info| info.downcast::<DesktopAppInfo>().ok())
        .find(|info| {
            info.id()
                .map(|g| g.to_string() == suffix)
                .unwrap_or(false)
        })
}

fn apply_app_icon(image: &gtk::Image, info: DesktopAppInfo) {
    // The reactive store accessors take `self` by value, so we
    // can't cache an intermediate handle — every argument
    // re-walks the chain from `config_manager()`. Matches the
    // pattern used in the legacy `app_launcher_item.rs`.
    set_icon(
        &Some(info),
        &None,
        image,
        config_manager()
            .config()
            .theme()
            .icons()
            .app_icon_theme()
            .get_untracked(),
        &config_manager().config().theme().theme().get_untracked(),
        config_manager()
            .config()
            .theme()
            .icons()
            .apply_theme_filter()
            .get_untracked(),
        config_manager()
            .config()
            .theme()
            .icons()
            .filter_strength()
            .get_untracked()
            .get(),
        config_manager()
            .config()
            .theme()
            .icons()
            .monochrome_strength()
            .get_untracked()
            .get(),
        config_manager()
            .config()
            .theme()
            .icons()
            .contrast_strength()
            .get_untracked()
            .get(),
    );
}
