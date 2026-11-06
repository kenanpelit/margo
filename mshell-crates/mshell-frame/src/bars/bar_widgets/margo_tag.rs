//! Per-tag pill button for the bar's MargoTags row.
//!
//! Each pill is a `gtk::Button` showing the tag's 1-indexed number
//! ("1".."9"), the configured glyph for its state
//! (`bars.widgets.margo_tags.glyph_*`), or — with `minimap` on and windows
//! on the tag — a tiny proportional map of those windows. The tag is in
//! exactly one [`TagState`] (focused > urgent > occupied > empty), carried
//! as CSS classes:
//!
//!   * `.tag-active`        — the focused tag: a filled capsule.
//!   * `.tag-urgent`        — a visible window on the tag wants attention:
//!     a filled capsule (unless focused).
//!   * `.tag-has-windows`   — at least one window: the digit is tinted.
//!   * `.tag-accent-<role>` — the theme role of the current state
//!     (`accent_*`; primary / error / primary by default), so the
//!     stylesheet picks the matugen var and the pill never hard-codes a
//!     colour.
//!
//! An empty, unfocused tag has no state class: a dim digit.
//!
//! The pill hides itself when the display mode (`display`) leaves its tag
//! out ([`super::margo_tags::shown_tags`]). Everything is reactive: the
//! tag's window count, urgency and minimap, the service's tag masks, and
//! the config store.
//!
//! Click sends `dispatch view <bitmask>` directly on margo's IPC
//! socket (`mshell_margo_client::ctl`).

use crate::bars::bar_widgets::margo_tags::{active_mask, shown_tags};
use futures::StreamExt;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, MargoTagsConfig,
};
use mshell_margo_client::indicators::TagMasks;
use mshell_margo_client::minimap::TagMinimap;
use mshell_margo_client::{Workspace, WorkspaceInfo};
use mshell_services::margo_service;
use mshell_utils::margo::get_active_workspaces;
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{ButtonExt, FixedExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;
use tracing::warn;

/// Height of a tag minimap in px; its width follows the tag's aspect.
const MINIMAP_HEIGHT: f32 = 14.0;
/// Narrowest / widest a minimap gets (a very wide scroller strip is
/// squeezed rather than stretching the row).
const MINIMAP_MIN_WIDTH: f32 = 14.0;
const MINIMAP_MAX_WIDTH: f32 = 44.0;

/// The one state a tag pill shows, by precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TagState {
    Focused,
    Urgent,
    Occupied,
    Empty,
}

pub(crate) fn tag_state(is_active: bool, urgent: bool, windows: u16) -> TagState {
    if is_active {
        TagState::Focused
    } else if urgent {
        TagState::Urgent
    } else if windows > 0 {
        TagState::Occupied
    } else {
        TagState::Empty
    }
}

/// The pill's text: the state's glyph, or the tag number when it has none.
pub(crate) fn tag_label(cfg: &MargoTagsConfig, state: TagState, id: i64) -> String {
    let glyph = match state {
        TagState::Focused => &cfg.glyph_focused,
        TagState::Urgent => &cfg.glyph_urgent,
        TagState::Occupied => &cfg.glyph_occupied,
        TagState::Empty => &cfg.glyph_empty,
    }
    .trim();
    if glyph.is_empty() {
        id.to_string()
    } else {
        glyph.to_string()
    }
}

/// Compose the CSS class list for the pill button. Each `Vec` slot is
/// **one** class name: GTK4's `set_css_classes(&[&str])` adds each entry
/// verbatim, so a space-joined `"margo-tag tag-active"` would become a
/// single bogus token and silently match nothing.
pub(crate) fn tag_classes(
    cfg: &MargoTagsConfig,
    state: TagState,
    windows: u16,
    minimap: bool,
) -> Vec<&'static str> {
    let mut classes = vec!["margo-tag"];
    let accent = match state {
        TagState::Focused => {
            classes.push("tag-active");
            Some(cfg.accent_focused)
        }
        TagState::Urgent => {
            classes.push("tag-urgent");
            Some(cfg.accent_urgent)
        }
        TagState::Occupied => Some(cfg.accent_occupied),
        TagState::Empty => None,
    };
    if windows > 0 {
        classes.push("tag-has-windows");
    }
    if let Some(accent) = accent {
        classes.push(accent.css_class());
    }
    if minimap {
        classes.push("tag-minimap");
    }
    classes
}

/// Whether the pill draws its minimap instead of the label.
fn minimap_shown(cfg: &MargoTagsConfig, map: &TagMinimap) -> bool {
    cfg.minimap && !map.windows.is_empty()
}

/// Pixel size of a minimap with `aspect` (width / height).
pub(crate) fn minimap_size(aspect: f32) -> (i32, i32) {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let w = (MINIMAP_HEIGHT * aspect).clamp(MINIMAP_MIN_WIDTH, MINIMAP_MAX_WIDTH);
    (w.round() as i32, MINIMAP_HEIGHT as i32)
}

/// One rect of the map in px inside a `w × h` map: at least 1 px, with a
/// 1 px gap to its right / bottom neighbour so adjacent tiles read apart.
pub(crate) fn minimap_rect(
    (x, y, rw, rh): (f32, f32, f32, f32),
    (w, h): (i32, i32),
) -> (f64, f64, i32, i32) {
    let (w, h) = (w as f32, h as f32);
    let px = |v: f32, total: f32| (v * total).round();
    let left = px(x, w);
    let top = px(y, h);
    let width = (px(x + rw, w) - left - 1.0).max(1.0);
    let height = (px(y + rh, h) - top - 1.0).max(1.0);
    (f64::from(left), f64::from(top), width as i32, height as i32)
}

#[derive(Debug)]
pub(crate) struct MargoTagModel {
    workspace: Arc<Workspace>,
    is_active: bool,
    windows: u16,
    urgent: bool,
    minimap: TagMinimap,
    masks: TagMasks,
    /// Mask of the focused tag(s), for the display mode.
    active: u32,
    cfg: MargoTagsConfig,
    _effects: mshell_common::scoped_effects::EffectScope,
}

impl MargoTagModel {
    fn state(&self) -> TagState {
        tag_state(self.is_active, self.urgent, self.windows)
    }

    fn shown(&self) -> bool {
        let id = self.workspace.id.get();
        if !(1..=32).contains(&id) {
            return true;
        }
        let count = margo_service().workspaces.get().len() as u32;
        shown_tags(self.cfg.display, self.masks, self.active, count) & (1u32 << (id - 1)) != 0
    }

    fn label(&self) -> String {
        tag_label(&self.cfg, self.state(), self.workspace.id.get())
    }

    fn classes(&self) -> Vec<&'static str> {
        tag_classes(
            &self.cfg,
            self.state(),
            self.windows,
            minimap_shown(&self.cfg, &self.minimap),
        )
    }
}

#[derive(Debug)]
pub(crate) enum MargoTagInput {
    ActiveUpdate(Vec<WorkspaceInfo>),
    WorkspaceClicked,
    /// `bars.widgets.margo_tags` changed.
    Config(MargoTagsConfig),
}

#[derive(Debug)]
pub(crate) enum MargoTagOutput {}

#[relm4::component(pub)]
impl Component for MargoTagModel {
    type CommandOutput = MargoTagCommandOutput;
    type Input = MargoTagInput;
    type Output = MargoTagOutput;
    type Init = Arc<Workspace>;

    view! {
        #[root]
        gtk::Box {
            set_hexpand: false,
            set_vexpand: false,
            #[watch]
            set_visible: model.shown(),

            #[name = "button"]
            gtk::Button {
                #[watch]
                set_css_classes: &model.classes(),
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(MargoTagInput::WorkspaceClicked);
                },

                gtk::Box {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,

                    gtk::Label {
                        add_css_class: "margo-tag-label",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        #[watch]
                        set_visible: !minimap_shown(&model.cfg, &model.minimap),
                        #[watch]
                        set_label: &model.label(),
                    },

                    #[name = "minimap"]
                    gtk::Fixed {
                        add_css_class: "margo-tag-minimap",
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_can_target: false,
                        #[watch]
                        set_visible: minimap_shown(&model.cfg, &model.minimap),
                    },
                },
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // The tag's own reactive fields, and the service-wide tag masks the
        // display mode needs (the "next empty" tag depends on every tag).
        // Each stream yields its current value first. Without these the
        // pill would only refresh on the coarse WorkspacesChanged cascade
        // (workspace add/remove), missing a window opening on its tag.
        let ws = params.clone();
        sender.command(move |out, shutdown| async move {
            let mut windows = ws.windows.watch();
            let mut urgent = ws.urgent.watch();
            let mut minimap = ws.minimap.watch();
            let mut masks = margo_service().tag_masks.watch();
            let shutdown_fut = shutdown.wait();
            tokio::pin!(shutdown_fut);
            loop {
                let msg = tokio::select! {
                    () = &mut shutdown_fut => break,
                    Some(v) = windows.next() => MargoTagCommandOutput::WindowsChanged(v),
                    Some(v) = urgent.next() => MargoTagCommandOutput::UrgentChanged(v),
                    Some(v) = minimap.next() => MargoTagCommandOutput::MinimapChanged(v),
                    Some(v) = masks.next() => MargoTagCommandOutput::MasksChanged(v),
                    else => break,
                };
                let _ = out.send(msg);
            }
        });

        // Settings edits apply live, without rebuilding the bar.
        let mut effects = mshell_common::scoped_effects::EffectScope::new();
        {
            let s = sender.clone();
            effects.push(move |_| {
                let cfg = config_manager()
                    .config()
                    .bars()
                    .widgets()
                    .margo_tags()
                    .get();
                s.input(MargoTagInput::Config(cfg));
            });
        }

        let active_infos = get_active_workspaces();
        let id = params.id.get();
        let model = MargoTagModel {
            is_active: active_infos.iter().any(|p| p.id == id),
            active: active_mask(&active_infos),
            windows: params.windows.get(),
            urgent: params.urgent.get(),
            minimap: params.minimap.get(),
            masks: margo_service().tag_masks.get(),
            cfg: config_manager()
                .config()
                .bars()
                .widgets()
                .margo_tags()
                .get_untracked(),
            workspace: params,
            _effects: effects,
        };

        let widgets = view_output!();
        render_minimap(&widgets.minimap, &model.minimap);

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
            MargoTagInput::ActiveUpdate(workspace_infos) => {
                let id = self.workspace.id.get();
                self.is_active = workspace_infos.iter().any(|p| p.id == id);
                self.active = active_mask(&workspace_infos);
            }
            MargoTagInput::Config(cfg) => self.cfg = cfg,
            MargoTagInput::WorkspaceClicked => {
                // `dispatch view <bitmask>` on margo's socket. Tag id
                // 1..=9 maps directly to bit (id-1). Fire-and-forget: the
                // visible result is observed via the next state push.
                let id = self.workspace.id.get();
                if !(1..=32).contains(&id) {
                    warn!(id, "MargoTag: out-of-range workspace id, skipping dispatch");
                    return;
                }
                let mask = 1u32 << (id - 1) as u32;
                mshell_margo_client::ctl::dispatch_detached("view", &[&mask.to_string()]);
            }
        }
        // Overriding `update_with_view` skips relm4's automatic
        // `update_view`: call it, or the `#[watch]` classes go stale (an
        // earlier version left the active pill stuck on the first tag).
        self.update_view(widgets, sender);
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            MargoTagCommandOutput::WindowsChanged(count) => self.windows = count,
            MargoTagCommandOutput::UrgentChanged(urgent) => self.urgent = urgent,
            MargoTagCommandOutput::MasksChanged(masks) => self.masks = masks,
            MargoTagCommandOutput::MinimapChanged(map) => {
                if map != self.minimap {
                    self.minimap = map;
                    render_minimap(&widgets.minimap, &self.minimap);
                }
            }
        }
        self.update_view(widgets, sender);
    }
}

#[derive(Debug)]
pub(crate) enum MargoTagCommandOutput {
    WindowsChanged(u16),
    UrgentChanged(bool),
    MinimapChanged(TagMinimap),
    MasksChanged(TagMasks),
}

/// Rebuild the minimap: one box per window (tiled first, floating on top),
/// then the screen outline when windows run past it. Colours come from the
/// `.margo-tag-mini-*` classes in `_margo_tag.scss`.
fn render_minimap(fixed: &gtk::Fixed, map: &TagMinimap) {
    while let Some(child) = fixed.first_child() {
        fixed.remove(&child);
    }
    let size = minimap_size(map.aspect);
    fixed.set_size_request(size.0, size.1);
    for w in &map.windows {
        let (x, y, width, height) = minimap_rect((w.x, w.y, w.w, w.h), size);
        let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        tile.add_css_class("margo-tag-mini-win");
        if w.floating {
            tile.add_css_class("floating");
        }
        if w.urgent {
            tile.add_css_class("urgent");
        }
        if w.focused {
            tile.add_css_class("focused");
        }
        tile.set_size_request(width, height);
        fixed.put(&tile, x, y);
    }
    if let Some(viewport) = map.viewport {
        let (x, y, width, height) = minimap_rect(viewport, size);
        let outline = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        outline.add_css_class("margo-tag-mini-viewport");
        outline.set_size_request(width + 1, height + 1);
        fixed.put(&outline, x, y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_config::schema::config::TagAccent;

    #[test]
    fn state_precedence_is_focused_urgent_occupied_empty() {
        assert_eq!(tag_state(true, true, 3), TagState::Focused);
        assert_eq!(tag_state(false, true, 3), TagState::Urgent);
        assert_eq!(tag_state(false, false, 3), TagState::Occupied);
        assert_eq!(tag_state(false, false, 0), TagState::Empty);
    }

    #[test]
    fn default_config_keeps_todays_classes_and_digits() {
        let cfg = MargoTagsConfig::default();
        assert_eq!(
            tag_classes(&cfg, TagState::Focused, 2, false),
            [
                "margo-tag",
                "tag-active",
                "tag-has-windows",
                "tag-accent-primary"
            ]
        );
        assert_eq!(
            tag_classes(&cfg, TagState::Occupied, 1, false),
            ["margo-tag", "tag-has-windows", "tag-accent-primary"]
        );
        assert_eq!(tag_classes(&cfg, TagState::Empty, 0, false), ["margo-tag"]);
        assert_eq!(
            tag_classes(&cfg, TagState::Urgent, 1, false),
            [
                "margo-tag",
                "tag-urgent",
                "tag-has-windows",
                "tag-accent-error"
            ]
        );
        for state in [
            TagState::Focused,
            TagState::Urgent,
            TagState::Occupied,
            TagState::Empty,
        ] {
            assert_eq!(tag_label(&cfg, state, 4), "4");
        }
    }

    #[test]
    fn per_state_glyphs_and_accents() {
        let cfg = MargoTagsConfig {
            glyph_focused: "🌕".into(),
            glyph_urgent: "🌟".into(),
            glyph_occupied: "🌗".into(),
            glyph_empty: " ".into(),
            accent_occupied: TagAccent::Tertiary,
            accent_focused: TagAccent::Secondary,
            ..Default::default()
        };
        assert_eq!(tag_label(&cfg, TagState::Focused, 1), "🌕");
        assert_eq!(tag_label(&cfg, TagState::Urgent, 1), "🌟");
        assert_eq!(tag_label(&cfg, TagState::Occupied, 1), "🌗");
        assert_eq!(tag_label(&cfg, TagState::Empty, 7), "7", "blank = number");
        assert!(tag_classes(&cfg, TagState::Occupied, 1, false).contains(&"tag-accent-tertiary"));
        let focused = tag_classes(&cfg, TagState::Focused, 1, true);
        assert!(focused.contains(&"tag-accent-secondary"));
        assert!(focused.contains(&"tag-minimap"));
    }

    #[test]
    fn minimap_sizes_follow_the_aspect_within_bounds() {
        assert_eq!(minimap_size(16.0 / 9.0), (25, 14));
        assert_eq!(minimap_size(0.2), (14, 14));
        assert_eq!(minimap_size(10.0), (44, 14));
        assert_eq!(minimap_size(f32::NAN), (25, 14));
    }

    #[test]
    fn minimap_rects_keep_a_gap_and_a_pixel() {
        // Two half-width tiles in a 24 px map: 0..11 and 12..23.
        assert_eq!(
            minimap_rect((0.0, 0.0, 0.5, 1.0), (24, 14)),
            (0.0, 0.0, 11, 13)
        );
        assert_eq!(
            minimap_rect((0.5, 0.0, 0.5, 1.0), (24, 14)),
            (12.0, 0.0, 11, 13)
        );
        // A sliver still shows.
        assert_eq!(minimap_rect((0.9, 0.0, 0.001, 0.01), (24, 14)).2, 1);
    }
}
