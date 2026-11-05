//! Control Center → Tiles settings section.
//!
//! A vertical list of all CC tiles — one row per tile — with:
//!   - display name
//!   - visibility `gtk::Switch` (bound to the per-tile config bool)
//!   - **↑** and **↓** buttons that move the tile in `tile_order`
//!
//! Rendered as a sub-section inside the generic
//! `WidgetMenuSettingsModel` page when
//! `kind == MenuKind::ControlCenter`.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::control_center_tiles::{self, CONTROL_CENTER_TILES};
use mshell_config::schema::config::{ConfigStoreFields, ControlCenterConfigStoreFields};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

// ── Tile metadata ─────────────────────────────────────────────────────────────

/// All known tile ids in the canonical default order (the shared
/// Control Center tile registry).
fn all_tile_ids() -> impl Iterator<Item = &'static str> {
    CONTROL_CENTER_TILES.iter().map(|t| t.id)
}

fn tile_display_name(id: &str) -> &'static str {
    control_center_tiles::tile(id).map_or("Unknown", |t| t.label)
}

/// Read a tile's current visibility from the config store.
fn tile_visible(id: &str) -> bool {
    control_center_tiles::tile(id).is_none_or(|t| (t.visible)())
}

/// Read whether a tile is currently in the `wide_tiles` config list.
fn tile_is_wide(id: &str) -> bool {
    config_manager()
        .config()
        .control_center()
        .wide_tiles()
        .get_untracked()
        .iter()
        .any(|s| s == id)
}

/// Add or remove a tile id from `wide_tiles` in the config.
fn set_tile_wide(id: &str, wide: bool) {
    let id = id.to_string();
    config_manager().update_config(move |c| {
        if wide {
            if !c.control_center.wide_tiles.contains(&id) {
                c.control_center.wide_tiles.push(id);
            }
        } else {
            c.control_center.wide_tiles.retain(|s| s != &id);
        }
    });
}

/// Write a tile's visibility to the config.
fn set_tile_visible(id: &str, visible: bool) {
    if let Some(t) = control_center_tiles::tile(id) {
        config_manager().update_config(move |c| (t.set_visible)(&mut c.control_center, visible));
    }
}

/// Compute the effective ordered list from a `tile_order` config vec.
/// Ids present in the config appear first (in config order); ids missing
/// from the config are appended in canonical order.
fn effective_order(tile_order: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut result = Vec::with_capacity(CONTROL_CENTER_TILES.len());
    for s in tile_order {
        if control_center_tiles::tile(s).is_some() && seen.insert(s.clone()) {
            result.push(s.clone());
        }
    }
    for id in all_tile_ids() {
        if seen.insert(id.to_string()) {
            result.push(id.to_string());
        }
    }
    result
}

// ── Row widget ────────────────────────────────────────────────────────────────

/// One row in the reorder list: name + visibility switch + wide switch + up/down buttons.
struct TileRow {
    container: gtk::Box,
    switch: gtk::Switch,
    wide_switch: gtk::Switch,
}

fn build_tile_row(tile_id: &str, sender: &ComponentSender<CcTilesSettingsModel>) -> TileRow {
    let container = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    container.set_hexpand(true);

    let name_label = gtk::Label::new(Some(tile_display_name(tile_id)));
    name_label.add_css_class("label-medium-bold");
    name_label.set_halign(gtk::Align::Start);
    name_label.set_hexpand(true);

    // Visibility switch
    let switch = gtk::Switch::new();
    switch.set_active(tile_visible(tile_id));
    switch.set_valign(gtk::Align::Center);
    switch.set_tooltip_text(Some("Show tile"));

    {
        let id = tile_id.to_string();
        switch.connect_active_notify(move |sw| {
            set_tile_visible(&id, sw.is_active());
        });
    }

    // Wide switch — label + switch pair
    let wide_label = gtk::Label::new(Some("Wide"));
    wide_label.add_css_class("label-small");
    wide_label.set_valign(gtk::Align::Center);

    let wide_switch = gtk::Switch::new();
    wide_switch.set_active(tile_is_wide(tile_id));
    wide_switch.set_valign(gtk::Align::Center);
    wide_switch.set_tooltip_text(Some("Span 2 columns"));

    {
        let id = tile_id.to_string();
        wide_switch.connect_active_notify(move |sw| {
            set_tile_wide(&id, sw.is_active());
        });
    }

    let up_btn = gtk::Button::new();
    up_btn.set_icon_name("go-up-symbolic");
    up_btn.add_css_class("flat");
    up_btn.set_valign(gtk::Align::Center);
    up_btn.set_tooltip_text(Some("Move up"));
    {
        let s = sender.clone();
        let id = tile_id.to_string();
        up_btn.connect_clicked(move |_| {
            s.input(CcTilesSettingsInput::MoveUp(id.clone()));
        });
    }

    let down_btn = gtk::Button::new();
    down_btn.set_icon_name("go-down-symbolic");
    down_btn.add_css_class("flat");
    down_btn.set_valign(gtk::Align::Center);
    down_btn.set_tooltip_text(Some("Move down"));
    {
        let s = sender.clone();
        let id = tile_id.to_string();
        down_btn.connect_clicked(move |_| {
            s.input(CcTilesSettingsInput::MoveDown(id.clone()));
        });
    }

    let grip = gtk::Image::from_icon_name("list-drag-handle-symbolic");
    grip.add_css_class("reorder-grip");
    grip.set_tooltip_text(Some("Drag to reorder"));

    container.append(&grip);
    container.append(&name_label);
    container.append(&switch);
    container.append(&wide_label);
    container.append(&wide_switch);
    container.append(&up_btn);
    container.append(&down_btn);

    // Drag-to-reorder via the grip handle.
    {
        let s = sender.clone();
        let id = tile_id.to_string();
        crate::reorder_dnd::attach_grip_drag(&grip, &container, move |delta| {
            s.input(CcTilesSettingsInput::ReorderByDelta(id.clone(), delta));
        });
    }

    TileRow {
        container,
        switch,
        wide_switch,
    }
}

// ── Model ─────────────────────────────────────────────────────────────────────

/// Build a labelled on/off row for a non-tile Control Center section.
/// `on_toggle` is a plain fn pointer (writes config) — no capture needed.
fn build_section_toggle(
    title: &str,
    subtitle: &str,
    initial: bool,
    on_toggle: fn(bool),
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
    col.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium");
    t.set_halign(gtk::Align::Start);
    let s = gtk::Label::new(Some(subtitle));
    s.add_css_class("label-small");
    s.set_halign(gtk::Align::Start);
    s.set_xalign(0.0);
    s.set_wrap(true);
    col.append(&t);
    col.append(&s);
    row.append(&col);
    let sw = gtk::Switch::new();
    sw.set_valign(gtk::Align::Center);
    sw.set_active(initial);
    sw.connect_active_notify(move |sw| on_toggle(sw.is_active()));
    row.append(&sw);
    row
}

pub(crate) struct CcTilesSettingsModel {
    /// Current effective ordered tile ids.
    tile_order: Vec<String>,
    _effects: EffectScope,
}

impl std::fmt::Debug for CcTilesSettingsModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CcTilesSettingsModel")
            .field("tile_order", &self.tile_order)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum CcTilesSettingsInput {
    MoveUp(String),
    MoveDown(String),
    /// Grip drag-to-reorder: move tile `id` by `delta` positions.
    ReorderByDelta(String, i32),
    /// Config changed externally — rebuild the list.
    OrderChanged(Vec<String>),
}

#[derive(Debug)]
pub(crate) enum CcTilesSettingsOutput {}

pub(crate) struct CcTilesSettingsInit {}

pub(crate) struct CcTilesSettingsWidgets {
    /// The rows list box, used to clear and rebuild on order change.
    rows_box: gtk::Box,
    /// Ordered rows — parallel to `model.tile_order`. Rebuilt on order change.
    rows: Vec<TileRow>,
}

impl std::fmt::Debug for CcTilesSettingsWidgets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CcTilesSettingsWidgets").finish()
    }
}

impl Component for CcTilesSettingsModel {
    type CommandOutput = ();
    type Input = CcTilesSettingsInput;
    type Output = CcTilesSettingsOutput;
    type Init = CcTilesSettingsInit;
    type Root = gtk::Box;
    type Widgets = CcTilesSettingsWidgets;

    fn init_root() -> Self::Root {
        let section = gtk::Box::new(gtk::Orientation::Vertical, 12);
        section.set_hexpand(true);
        section
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // ── Sections (non-tile control-center widgets) ──
        let sections_label = gtk::Label::new(Some("Sections"));
        sections_label.add_css_class("label-large-bold");
        sections_label.set_halign(gtk::Align::Start);
        root.append(&sections_label);

        root.append(&build_section_toggle(
            "Battery chip",
            "Show the battery icon + percentage in the header (auto-hidden on desktops).",
            config_manager()
                .config()
                .control_center()
                .show_battery_chip()
                .get_untracked(),
            |v| {
                config_manager().update_config(move |c| c.control_center.show_battery_chip = v);
            },
        ));
        root.append(&build_section_toggle(
            "Power profile",
            "Show the Saver / Balanced / Performance switcher above the sliders.",
            config_manager()
                .config()
                .control_center()
                .show_power_profile()
                .get_untracked(),
            |v| {
                config_manager().update_config(move |c| c.control_center.show_power_profile = v);
            },
        ));

        // Section header
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        root.append(&separator);

        let header_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let header_label = gtk::Label::new(Some("Tiles"));
        header_label.add_css_class("label-large-bold");
        header_label.set_halign(gtk::Align::Start);
        let header_desc = gtk::Label::new(Some(
            "Choose which tiles appear in the Control Center and reorder them by dragging a row or with the ↑ / ↓ buttons. Changes take effect immediately.",
        ));
        header_desc.add_css_class("label-small");
        header_desc.set_halign(gtk::Align::Start);
        header_desc.set_xalign(0.0);
        header_desc.set_wrap(true);
        header_box.append(&header_label);
        header_box.append(&header_desc);
        root.append(&header_box);

        // Rows container
        let rows_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
        rows_box.set_hexpand(true);
        root.append(&rows_box);

        // Read initial order
        let initial_order = effective_order(
            &config_manager()
                .config()
                .control_center()
                .tile_order()
                .get_untracked(),
        );

        // Build initial rows
        let rows = build_rows(&rows_box, &initial_order, &sender);

        // Reactive effect: track tile_order + wide_tiles + every tile's
        // visibility bool.
        let mut effects = EffectScope::new();
        {
            let s = sender.clone();
            effects.push(move |_| {
                let cm = config_manager();
                for t in CONTROL_CENTER_TILES {
                    let _ = (t.track_visible)();
                }
                let _ = cm.config().control_center().wide_tiles().get();
                let order = effective_order(&cm.config().control_center().tile_order().get());
                s.input(CcTilesSettingsInput::OrderChanged(order));
            });
        }

        let model = CcTilesSettingsModel {
            tile_order: initial_order,
            _effects: effects,
        };

        let widgets = CcTilesSettingsWidgets { rows_box, rows };

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
            CcTilesSettingsInput::MoveUp(id) => {
                let mut order = config_manager()
                    .config()
                    .control_center()
                    .tile_order()
                    .get_untracked();
                // Ensure all canonical ids are present
                for cid in all_tile_ids() {
                    if !order.contains(&cid.to_string()) {
                        order.push(cid.to_string());
                    }
                }
                if let Some(pos) = order.iter().position(|s| s == &id)
                    && pos > 0
                {
                    order.swap(pos, pos - 1);
                    config_manager().update_config(|c| {
                        c.control_center.tile_order = order;
                    });
                }
            }

            CcTilesSettingsInput::MoveDown(id) => {
                let mut order = config_manager()
                    .config()
                    .control_center()
                    .tile_order()
                    .get_untracked();
                for cid in all_tile_ids() {
                    if !order.contains(&cid.to_string()) {
                        order.push(cid.to_string());
                    }
                }
                if let Some(pos) = order.iter().position(|s| s == &id)
                    && pos + 1 < order.len()
                {
                    order.swap(pos, pos + 1);
                    config_manager().update_config(|c| {
                        c.control_center.tile_order = order;
                    });
                }
            }

            CcTilesSettingsInput::ReorderByDelta(id, delta) => {
                let mut order = config_manager()
                    .config()
                    .control_center()
                    .tile_order()
                    .get_untracked();
                for cid in all_tile_ids() {
                    if !order.contains(&cid.to_string()) {
                        order.push(cid.to_string());
                    }
                }
                if let Some(from) = order.iter().position(|s| s == &id) {
                    let to = (from as i32 + delta).clamp(0, order.len() as i32 - 1) as usize;
                    if to != from {
                        let item = order.remove(from);
                        order.insert(to, item);
                        config_manager().update_config(|c| {
                            c.control_center.tile_order = order;
                        });
                    }
                }
            }

            CcTilesSettingsInput::OrderChanged(new_order) => {
                if self.tile_order != new_order {
                    self.tile_order = new_order.clone();
                    rebuild_rows(widgets, &new_order, &sender);
                } else {
                    // Order is the same but visibility or wide state might have
                    // changed — update switch states without rebuilding rows.
                    for (row, id) in widgets.rows.iter().zip(self.tile_order.iter()) {
                        row.switch.set_active(tile_visible(id));
                        row.wide_switch.set_active(tile_is_wide(id));
                    }
                }
            }
        }
    }
}

// ── Row helpers ───────────────────────────────────────────────────────────────

fn build_rows(
    rows_box: &gtk::Box,
    order: &[String],
    sender: &ComponentSender<CcTilesSettingsModel>,
) -> Vec<TileRow> {
    let rows: Vec<TileRow> = order.iter().map(|id| build_tile_row(id, sender)).collect();
    for row in &rows {
        rows_box.append(&row.container);
    }
    rows
}

fn rebuild_rows(
    widgets: &mut CcTilesSettingsWidgets,
    order: &[String],
    sender: &ComponentSender<CcTilesSettingsModel>,
) {
    // Remove old rows
    for row in &widgets.rows {
        widgets.rows_box.remove(&row.container);
    }
    // Build new rows in new order
    widgets.rows = build_rows(&widgets.rows_box, order, sender);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Settings list offers every Control Center tile (Noise
    /// Suppression was missing), in the grid's canonical order, and an
    /// order that skips a tile gets it appended instead of dropped.
    #[test]
    fn settings_lists_every_control_center_tile() {
        let ids: Vec<&str> = all_tile_ids().collect();
        assert_eq!(
            ids,
            [
                "wifi",
                "bluetooth",
                "audio_out",
                "mic",
                "vpn",
                "valent",
                "battery",
                "keep_awake",
                "dnd",
                "airplane_mode",
                "dark_mode",
                "night_light",
                "color_picker",
                "disk",
                "ufw",
                "podman",
                "mirror",
                "mic_denoise",
                "hotspot",
                "thinkpad_led",
            ]
        );
        let order = effective_order(&["mic_denoise".to_string(), "bogus".to_string()]);
        assert_eq!(order.first().map(String::as_str), Some("mic_denoise"));
        assert_eq!(order.len(), ids.len());
        assert_eq!(tile_display_name("mic_denoise"), "Noise Suppression");
        assert_eq!(tile_display_name("bogus"), "Unknown");
    }
}
