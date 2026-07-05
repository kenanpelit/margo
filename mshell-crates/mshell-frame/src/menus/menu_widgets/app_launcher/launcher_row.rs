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
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, Sender, gtk};
use std::cell::RefCell;
use std::rc::Rc;

/// One row in the result list.
pub(crate) struct LauncherRowModel {
    item: LauncherItem,
    /// Char positions in `item.name` the query matched — rendered as an
    /// accent-coloured span per char (fzf-style). Empty → plain name.
    match_indices: Vec<u32>,
    /// Pin flag stamped by the runtime — drives the ★ glyph.
    pinned: bool,
    /// Hidden flag stamped by the runtime — flips the
    /// right-click context menu label between "Hide" / "Unhide".
    hidden: bool,
    /// Current persistence key for Pin/Hide actions. DynamicBox can keep
    /// this row controller alive while changing the displayed item, so
    /// right-click callbacks must not capture the init-time value.
    usage_key: Rc<RefCell<Option<String>>>,
    /// Context popover is intentionally built outside the declarative
    /// GtkButton child tree. GtkButton is single-child; making Popover a
    /// sibling inside it can leave GTK with an invalid popup parent and
    /// crash on right-click under Wayland.
    context_menu: gtk::Popover,
    pin_button: gtk::Button,
    hide_button: gtk::Button,
    /// `"1".."9"` quick-activate digit, or empty string for rows
    /// past the first nine.
    quick_key: String,
    is_selected: bool,
    /// Per-provider CSS modifier class (`row-apps`, `row-calc`, …) so
    /// SCSS can specialise the row layout/typography by source.
    variant: String,
    /// Short human label for the trailing source chip ("App", "Calc",
    /// "SSH", …) derived from `item.provider_name`. Empty → no chip.
    source_label: String,
    /// True when this is a synthetic in-list section header (the "All"
    /// view groups rows under Apps / Actions / Insert / … captions).
    /// Header rows render a dim non-interactive caption instead of the
    /// normal icon/title/badge row and are never selectable — the
    /// launcher keeps them out of its `results` vec so keyboard nav and
    /// the quick-key (1–9) numbering skip them automatically.
    is_header: bool,
}

/// Provider name the launcher stamps on a synthetic section-header
/// [`DisplayItem`]. Rows with this provider render as a section
/// caption rather than an interactive result.
pub(crate) const SECTION_HEADER_PROVIDER: &str = "__section_header__";

#[derive(Debug)]
pub(crate) enum LauncherRowInput {
    /// DynamicBox kept this row controller because the result id is stable,
    /// but the backing item changed (e.g. `websearch:g` for a new query).
    /// Refresh every displayed field and activation closure in place.
    DisplayChanged(DisplayItem),
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
    /// User picked "Pin" / "Unpin" from the row's context menu.
    /// Parent calls `runtime.toggle_pin(usage_key)`.
    TogglePin(String),
    /// User picked "Hide" / "Unhide" from the row's context menu.
    /// Parent calls `runtime.toggle_hidden(usage_key)`. Hidden
    /// items disappear from the empty-browse list on next query.
    ToggleHidden(String),
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
            set_css_classes: &{
                if model.is_header {
                    // Non-interactive section caption — none of the
                    // button-surface / result-row chrome.
                    vec!["app-launcher-section-header"]
                } else {
                    let mut v =
                        vec!["ok-button-surface", "app-launcher-item", model.variant.as_str()];
                    if model.is_selected {
                        v.push("selected");
                    }
                    v
                }
            },
            set_vexpand: false,
            set_hexpand: true,
            set_can_focus: false,
            // Headers must not steal pointer events (no hover wash, no
            // click) — they're pure labels sitting in the list flow.
            #[watch]
            set_can_target: !model.is_header,
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
                    // Section headers carry no icon.
                    #[watch]
                    set_visible: !model.is_header,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_valign: gtk::Align::Center,
                    set_hexpand: true,

                    #[name = "title_label"]
                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        add_css_class: "app-launcher-item-title",
                        set_label: &model.item.name,
                        set_halign: gtk::Align::Start,
                        set_ellipsize: pango::EllipsizeMode::End,
                    },

                    #[name = "subtitle_label"]
                    gtk::Label {
                        add_css_class: "label-small",
                        add_css_class: "app-launcher-item-sub",
                        set_label: &model.item.description,
                        set_visible: !model.item.description.is_empty(),
                        set_halign: gtk::Align::Start,
                        set_ellipsize: pango::EllipsizeMode::End,
                    },
                },

                // Trailing source chip — a dim metadata pill naming the
                // provider this row came from ("App", "Calc", "SSH", …).
                // Ranks below the title: it aids scanning a mixed result
                // list without competing with the row's name.
                #[name = "source_badge"]
                gtk::Label {
                    add_css_class: "app-launcher-source-badge",
                    #[watch]
                    set_label: &model.source_label,
                    #[watch]
                    set_visible: !model.is_header && !model.source_label.is_empty(),
                    set_halign: gtk::Align::End,
                    set_valign: gtk::Align::Center,
                    set_margin_start: 8,
                },

                // ★ pin marker — always present in the layout (the
                // glyph reserves the same physical space whether
                // pinned or not so the row's width is stable
                // across pin/unpin transitions). Opacity flips the
                // glyph between visible (pinned) and invisible
                // (unpinned) — using `set_visible: false` would
                // collapse the label and shift the rest of the
                // row right, which the user perceives as the
                // panel "jittering" during keyboard navigation.
                #[name = "pin_marker"]
                gtk::Label {
                    add_css_class: "app-launcher-pin-marker",
                    set_label: "\u{2605}",
                    #[watch]
                    set_opacity: if model.pinned { 1.0 } else { 0.0 },
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
        let DisplayItem {
            item,
            pinned,
            quick_key,
            hidden,
            match_indices,
        } = params.display;
        // Hide the context menu entirely for rows without a
        // usage_key — those are synthetic (commands palette, etc.)
        // and the Pin/Hide actions would have nothing to persist
        // against. Keep it in shared state because DynamicBox can
        // reuse this controller for updated display items.
        let usage_key = Rc::new(RefCell::new(item.usage_key.clone()));
        let (context_menu, pin_button, hide_button) = build_context_menu(pinned, hidden);
        // Per-provider styling hook: `.row-apps`, `.row-calc`, … —
        // lowercased + sanitised so the class is a valid CSS ident.
        let variant = row_variant(&item.provider_name);
        let is_header = item.provider_name == SECTION_HEADER_PROVIDER;
        let source_label = if is_header {
            String::new()
        } else {
            source_badge_label(&item.provider_name)
        };
        let model = LauncherRowModel {
            item,
            match_indices,
            pinned,
            hidden,
            usage_key,
            context_menu,
            pin_button,
            hide_button,
            quick_key,
            is_selected: false,
            variant,
            source_label,
            is_header,
        };

        let widgets = view_output!();

        apply_icon(&widgets.image, &model.item);
        apply_name(&widgets.title_label, &model.item.name, &model.match_indices);
        model.context_menu.set_parent(&widgets.button);

        // Right-click → open the context popover anchored on the
        // button. SECONDARY gesture so it doesn't collide with
        // the left-click Activate signal on the same button.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let popover = model.context_menu.clone();
        let usage_key_for_menu = model.usage_key.clone();
        gesture.connect_pressed(move |_, _, x, y| {
            if usage_key_for_menu.borrow().is_some() {
                let rect = gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1);
                popover.set_pointing_to(Some(&rect));
                popover.popup();
            }
        });
        widgets.button.add_controller(gesture);

        // Wire the popover buttons here (rather than in the
        // declarative view) so we capture the usage_key once and
        // dodge the partial-move-of-model issue inside view! closures.
        let popdown = model.context_menu.clone();
        let sender_pin = sender.clone();
        let key_pin = model.usage_key.clone();
        model.pin_button.connect_clicked(move |_| {
            popdown.popdown();
            if let Some(key) = key_pin.borrow().clone() {
                let _ = sender_pin.output(LauncherRowOutput::TogglePin(key));
            }
        });
        let popdown = model.context_menu.clone();
        let sender_hide = sender.clone();
        let key_hide = model.usage_key.clone();
        model.hide_button.connect_clicked(move |_| {
            popdown.popdown();
            if let Some(key) = key_hide.borrow().clone() {
                let _ = sender_hide.output(LauncherRowOutput::ToggleHidden(key));
            }
        });

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
            LauncherRowInput::DisplayChanged(display) => {
                let DisplayItem {
                    item,
                    pinned,
                    quick_key,
                    hidden,
                    match_indices,
                } = display;
                self.variant = row_variant(&item.provider_name);
                self.is_header = item.provider_name == SECTION_HEADER_PROVIDER;
                self.source_label = if self.is_header {
                    String::new()
                } else {
                    source_badge_label(&item.provider_name)
                };
                self.item = item;
                self.match_indices = match_indices;
                self.pinned = pinned;
                self.quick_key = quick_key;
                self.hidden = hidden;
                *self.usage_key.borrow_mut() = self.item.usage_key.clone();

                apply_name(&widgets.title_label, &self.item.name, &self.match_indices);
                widgets.subtitle_label.set_label(&self.item.description);
                widgets
                    .subtitle_label
                    .set_visible(!self.item.description.is_empty());
                widgets.quick_key_label.set_label(&self.quick_key);
                widgets
                    .quick_key_label
                    .set_visible(!self.quick_key.is_empty());
                apply_icon(&widgets.image, &self.item);
            }
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
        self.update_context_menu_labels();
        self.update_view(widgets, sender);
    }

    fn shutdown(&mut self, _widgets: &mut Self::Widgets, _output: Sender<Self::Output>) {
        self.context_menu.popdown();
        if self.context_menu.parent().is_some() {
            self.context_menu.unparent();
        }
    }
}

impl LauncherRowModel {
    fn update_context_menu_labels(&self) {
        self.pin_button
            .set_label(if self.pinned { "Unpin" } else { "Pin" });
        self.hide_button
            .set_label(if self.hidden { "Unhide" } else { "Hide" });
    }
}

/// Short, human-readable source label for the trailing row chip. Maps
/// each provider's full name to a compact tag so the badge stays
/// metadata-rank (a hint, not a second title). Unknown providers fall
/// back to their own name so a future source still gets a badge.
fn source_badge_label(provider_name: &str) -> String {
    match provider_name {
        "Apps" => "App",
        "Windows" => "Window",
        "Tags" => "Tag",
        "Calculator" => "Calc",
        "Session" => "Session",
        "Margo" => "Margo",
        "Settings" => "Settings",
        "Clipboard" => "Clip",
        "Scripts" => "Script",
        "Symbols" => "Symbol",
        "Emoji" => "Emoji",
        "Web search" => "Web",
        "Providers" => "Help",
        "Player" => "Media",
        "Arch packages" => "Pkg",
        "Audio" => "Audio",
        "Bluetooth" => "BT",
        "SSH" => "SSH",
        "Pass" => "Pass",
        "Command" => "Run",
        other => other,
    }
    .to_string()
}

fn row_variant(provider_name: &str) -> String {
    format!(
        "row-{}",
        provider_name
            .to_ascii_lowercase()
            .replace(|c: char| !c.is_ascii_alphanumeric(), "-")
    )
}

thread_local! {
    /// Accent colour (matugen `--primary`) rows use to highlight matched
    /// query chars. Resolved once per launcher open from a realized widget by
    /// the parent (see [`set_match_accent`]); a row can't resolve it reliably
    /// at build time — its style context carries no CSS until it's parented.
    static MATCH_ACCENT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Stash the accent colour the launcher resolved from `--primary`. `None`
/// disables highlighting (rows fall back to a plain name).
pub(crate) fn set_match_accent(hex: Option<String>) {
    MATCH_ACCENT.with(|a| *a.borrow_mut() = hex);
}

pub(crate) fn match_accent_value() -> Option<String> {
    MATCH_ACCENT.with(|a| a.borrow().clone())
}

/// Read the matugen `--primary` value out of `widget`'s resolved style — the
/// only way to get a matugen CSS var as a literal (GTK exposes them as
/// `--vars`, not `@define-color`, so `lookup_color` can't see them). `widget`
/// must be realized/parented for the var to be present.
pub(crate) fn resolve_primary_var(widget: &impl IsA<gtk::Widget>) -> Option<String> {
    #[allow(deprecated)]
    let css = widget
        .style_context()
        .to_string(gtk::StyleContextPrintFlags::SHOW_STYLE);
    for line in css.lines() {
        if let Some(rest) = line.trim().strip_prefix("--primary:") {
            let value = rest.trim().trim_end_matches(';').trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Render `name` into `label`, accent-colouring the chars at `indices`
/// (fzf-style). Falls back to a plain label when there's nothing to highlight
/// or no accent has been resolved yet.
fn apply_name(label: &gtk::Label, name: &str, indices: &[u32]) {
    let Some(accent) = match_accent_value().filter(|_| !indices.is_empty()) else {
        label.set_label(name);
        return;
    };
    let matched: std::collections::HashSet<u32> = indices.iter().copied().collect();
    let mut markup = String::new();
    for (i, ch) in name.chars().enumerate() {
        let esc = gtk::glib::markup_escape_text(&ch.to_string());
        if matched.contains(&(i as u32)) {
            markup.push_str("<span foreground=\"");
            markup.push_str(&accent);
            markup.push_str("\">");
            markup.push_str(esc.as_str());
            markup.push_str("</span>");
        } else {
            markup.push_str(esc.as_str());
        }
    }
    label.set_markup(&markup);
}

fn build_context_menu(pinned: bool, hidden: bool) -> (gtk::Popover, gtk::Button, gtk::Button) {
    let popover = gtk::Popover::new();
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_has_arrow(false);
    popover.set_autohide(true);

    let container = gtk::Box::new(gtk::Orientation::Vertical, 2);
    container.set_margin_all(4);

    let pin_button = gtk::Button::with_label(if pinned { "Unpin" } else { "Pin" });
    pin_button.add_css_class("flat");
    pin_button.set_halign(gtk::Align::Fill);

    let hide_button = gtk::Button::with_label(if hidden { "Unhide" } else { "Hide" });
    hide_button.add_css_class("flat");
    hide_button.set_halign(gtk::Align::Fill);

    container.append(&pin_button);
    container.append(&hide_button);
    popover.set_child(Some(&container));

    (popover, pin_button, hide_button)
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
        .find(|info| info.id().map(|g| g == suffix).unwrap_or(false))
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
