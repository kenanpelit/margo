//! Settings → Overview.
//!
//! Pick which overview the generic `toggle_overview` keybind opens — the
//! niri-style **Scroller** (recommended) or the classic **Grid** — and
//! tune each independently. These are compositor settings: reads parse
//! margo's `config.conf`, writes patch the `key = value` line in place,
//! then `mctl reload` applies them live.

use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, gio};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;

/// `~/.config/margo/config.conf` (XDG-aware) — the compositor config,
/// same file the Input / Animations pages patch.
fn conf_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("margo").join("config.conf")
}

fn read_config() -> margo_config::Config {
    margo_config::parse_config_with_defaults(Some(&conf_path())).unwrap_or_default()
}

/// Patch `key = value` lines in place (append if missing), preserving
/// comments and unrelated keys.
fn patch_conf(updates: &[(&str, String)]) -> std::io::Result<()> {
    let path = conf_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut out = String::with_capacity(existing.len() + 128);
    let mut seen = vec![false; updates.len()];
    for line in existing.lines() {
        let t = line.trim_start();
        let mut handled = false;
        for (i, (key, val)) in updates.iter().enumerate() {
            if let Some(rest) = t.strip_prefix(*key)
                && rest.trim_start().starts_with('=')
            {
                seen[i] = true;
                out.push_str(&format!("{key} = {val}\n"));
                handled = true;
                break;
            }
        }
        if !handled {
            out.push_str(line);
            out.push('\n');
        }
    }
    for (i, (key, val)) in updates.iter().enumerate() {
        if !seen[i] {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&format!("{key} = {val}\n"));
        }
    }
    std::fs::write(&path, out)
}

/// Reload the compositor live, reaping the child asynchronously.
fn reload() {
    match std::process::Command::new("mctl").args(["reload"]).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(error = %e, "overview: `mctl reload` failed to spawn"),
    }
}

/// Apply one config key and reload.
fn apply(key: &str, val: String) {
    if let Err(e) = patch_conf(&[(key, val)]) {
        tracing::warn!(error = %e, key, "overview: failed to patch config.conf");
        return;
    }
    reload();
}

pub struct OverviewSettingsInit {}

pub struct OverviewSettingsModel {
    style_is_scroller: bool,
    scroller_zoom: f32,
    scroller_gap: i32,
    scroller_loop: bool,
    grid_zoom: f32,
    grid_gap_inner: i32,
    grid_gap_outer: i32,
    grid_dim: f32,
    grid_transition: u32,
    tab_mode: bool,
    selected_border_mult: f64,
    cycle_order_idx: u32,
    /// Solid backdrop colour painted behind the scroller-overview cells.
    backdrop_color: gdk::RGBA,
    /// Backdrop image path (empty = none, solid colour used).
    backdrop_image: String,
    style_model: gtk::StringList,
    cycle_model: gtk::StringList,
    mru_thumb: f64,
    mru_scope_idx: u32,
    mru_filter_idx: u32,
    mru_labels: bool,
    mru_scope_model: gtk::StringList,
    mru_filter_model: gtk::StringList,
}

impl OverviewSettingsModel {
    /// Description line for the backdrop-image row: the current path, or
    /// a hint that the solid colour is in effect when unset.
    fn image_desc(&self) -> String {
        if self.backdrop_image.is_empty() {
            "No image set — the backdrop colour above is used.".to_string()
        } else {
            self.backdrop_image.clone()
        }
    }
}

/// Format a `gdk::RGBA` as margo's `0xRRGGBBAA` hex (what `parse_color`
/// in margo-config reads).
fn rgba_to_hex(c: gdk::RGBA) -> String {
    let q = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u32;
    format!(
        "0x{:02x}{:02x}{:02x}{:02x}",
        q(c.red()),
        q(c.green()),
        q(c.blue()),
        q(c.alpha())
    )
}

#[derive(Debug)]
pub enum OverviewSettingsInput {
    /// 0 = Scroller, 1 = Grid.
    SetStyle(u32),
    SetScrollerZoom(f64),
    SetScrollerGap(i32),
    SetScrollerLoop(bool),
    SetGridZoom(f64),
    SetGridGapInner(i32),
    SetGridGapOuter(i32),
    SetGridDim(f64),
    SetGridTransition(i32),
    SetTabMode(bool),
    SetSelectedBorderMult(f64),
    /// 0 = MRU, 1 = Tag, 2 = Mixed.
    SetCycleOrder(u32),
    SetBackdropColor(gdk::RGBA),
    /// Open the image file chooser.
    OpenBackdropImage,
    /// A path was chosen — persist + display it.
    SetBackdropImage(String),
    /// Drop the image; fall back to the solid backdrop colour.
    ClearBackdropImage,
    // MRU window switcher.
    SetMruThumb(i32),
    /// 0 = All, 1 = Output, 2 = Workspace.
    SetMruScope(u32),
    /// 0 = All, 1 = Same app.
    SetMruFilter(u32),
    SetMruLabels(bool),
}

#[relm4::component(pub)]
impl Component for OverviewSettingsModel {
    type CommandOutput = ();
    type Input = OverviewSettingsInput;
    type Output = ();
    type Init = OverviewSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("view-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Overview",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Pick and tune the overview the toggle_overview key opens. Applied to the compositor live.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                // ════════ Style ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Style",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Preferred overview" },
                        #[template_child] desc {
                            set_label: "Which overview the toggle_overview keybind opens. The grid and scroller can also be bound directly (toggle_grid_overview / toggle_scroller_overview).",
                        },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.style_model),
                            set_selected: if model.style_is_scroller { 0 } else { 1 },
                            connect_selected_notify[sender] => move |d| {
                                sender.input(OverviewSettingsInput::SetStyle(d.selected()));
                            },
                        },
                    },
                },

                // ════════ Scroller overview ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Scroller overview",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Zoom" },
                        #[template_child] desc {
                            set_label: "Mini-desktop size = screen × zoom. Lower fits more tags on screen.",
                        },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.1, 1.0),
                            set_increments: (0.05, 0.1),
                            set_digits: 2,
                            set_value: model.scroller_zoom as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetScrollerZoom(s.value()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Gap" },
                        #[template_child] desc { set_label: "Vertical gap between tag cells, in pixels." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 300.0),
                            set_increments: (4.0, 20.0),
                            set_digits: 0,
                            set_value: model.scroller_gap as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetScrollerGap(s.value() as i32));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Loop" },
                        #[template_child] desc {
                            set_label: "Wrap around: scrolling past the last tag continues to the first (and back), instead of stopping at the ends.",
                        },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: model.scroller_loop,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(OverviewSettingsInput::SetScrollerLoop(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                },

                // ════════ Backdrop ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Backdrop",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Backdrop colour" },
                        #[template_child] desc {
                            set_label: "Solid colour painted behind the scroller-overview cells (used when no backdrop image is set).",
                        },
                        gtk::ColorDialogButton {
                            set_valign: gtk::Align::Center,
                            set_dialog: &gtk::ColorDialog::builder().with_alpha(true).build(),
                            set_rgba: &model.backdrop_color,
                            connect_rgba_notify[sender] => move |b| {
                                sender.input(OverviewSettingsInput::SetBackdropColor(b.rgba()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Backdrop image" },
                        #[template_child] desc {
                            #[watch]
                            set_label: &model.image_desc(),
                            set_wrap: true,
                            set_xalign: 0.0,
                        },
                        gtk::Box {
                            set_valign: gtk::Align::Center,
                            set_spacing: 6,
                            gtk::Button {
                                set_label: "Choose…",
                                connect_clicked => OverviewSettingsInput::OpenBackdropImage,
                            },
                            gtk::Button {
                                add_css_class: "destructive-action",
                                set_label: "Clear",
                                #[watch]
                                set_sensitive: !model.backdrop_image.is_empty(),
                                connect_clicked => OverviewSettingsInput::ClearBackdropImage,
                            },
                        },
                    },
                },

                // ════════ Grid overview ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Grid overview",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Zoom" },
                        #[template_child] desc { set_label: "Centered sub-rect the flattened grid arranges into." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.1, 1.0),
                            set_increments: (0.05, 0.1),
                            set_digits: 2,
                            set_value: model.grid_zoom as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetGridZoom(s.value()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Inner gap" },
                        #[template_child] desc { set_label: "Gap between grid thumbnails, in pixels." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 200.0),
                            set_increments: (1.0, 10.0),
                            set_digits: 0,
                            set_value: model.grid_gap_inner as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetGridGapInner(s.value() as i32));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Outer gap" },
                        #[template_child] desc { set_label: "Margin around the grid, in pixels." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 200.0),
                            set_increments: (1.0, 10.0),
                            set_digits: 0,
                            set_value: model.grid_gap_outer as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetGridGapOuter(s.value() as i32));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Dim" },
                        #[template_child] desc { set_label: "Opacity of non-selected thumbnails (1.0 = no dim)." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.1, 1.0),
                            set_increments: (0.05, 0.1),
                            set_digits: 2,
                            set_value: model.grid_dim as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetGridDim(s.value()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Transition" },
                        #[template_child] desc { set_label: "Open / close duration, in milliseconds." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (0.0, 2000.0),
                            set_increments: (10.0, 50.0),
                            set_digits: 0,
                            set_value: model.grid_transition as f64,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetGridTransition(s.value() as i32));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Selected thumbnail border" },
                        #[template_child] desc { set_label: "Border thickness multiplier for the hovered/selected thumbnail." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (1.0, 4.0),
                            set_increments: (0.1, 0.5),
                            set_digits: 1,
                            set_value: model.selected_border_mult,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetSelectedBorderMult(s.value()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Tab mode" },
                        #[template_child] desc { set_label: "Alternate overview tab layout." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: model.tab_mode,
                            connect_active_notify[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetTabMode(s.is_active()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Cycle order" },
                        #[template_child] desc { set_label: "Order alt+Tab walks the grid thumbnails." },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.cycle_model),
                            set_selected: model.cycle_order_idx,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(OverviewSettingsInput::SetCycleOrder(d.selected()));
                            },
                        },
                    },
                },

                // ════════ MRU window switcher (Super/Alt+Tab) ════════
                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Window switcher (Super/Alt+Tab)",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Thumbnail size" },
                        #[template_child] desc { set_label: "Height of each window thumbnail in the switcher, in pixels." },
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_range: (60.0, 600.0),
                            set_increments: (10.0, 40.0),
                            set_digits: 0,
                            set_value: model.mru_thumb,
                            connect_value_changed[sender] => move |s| {
                                sender.input(OverviewSettingsInput::SetMruThumb(s.value() as i32));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Scope" },
                        #[template_child] desc { set_label: "Which windows the switcher lists (default for binds that pass none)." },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.mru_scope_model),
                            set_selected: model.mru_scope_idx,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(OverviewSettingsInput::SetMruScope(d.selected()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Filter" },
                        #[template_child] desc { set_label: "All windows, or only those sharing the focused window's app." },
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_model: Some(&model.mru_filter_model),
                            set_selected: model.mru_filter_idx,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(OverviewSettingsInput::SetMruFilter(d.selected()));
                            },
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Show labels" },
                        #[template_child] desc { set_label: "Draw the app-id under each thumbnail." },
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            set_active: model.mru_labels,
                            connect_state_set[sender] => move |_, on| {
                                sender.input(OverviewSettingsInput::SetMruLabels(on));
                                gtk::glib::Propagation::Proceed
                            },
                        },
                    },
                },
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let cfg = read_config();
        let model = OverviewSettingsModel {
            style_is_scroller: matches!(cfg.overview_style, margo_config::OverviewStyle::Scroller),
            scroller_zoom: cfg.scroller_overview_zoom,
            scroller_gap: cfg.scroller_overview_gap,
            scroller_loop: cfg.scroller_overview_loop,
            grid_zoom: cfg.overview_zoom,
            grid_gap_inner: cfg.overview_gap_inner,
            grid_gap_outer: cfg.overview_gap_outer,
            grid_dim: cfg.overview_dim_alpha,
            grid_transition: cfg.overview_transition_ms,
            tab_mode: cfg.ov_tab_mode != 0,
            selected_border_mult: cfg.overview_selected_border_multiplier as f64,
            cycle_order_idx: match cfg.overview_cycle_order {
                margo_config::OverviewCycleOrder::Mru => 0,
                margo_config::OverviewCycleOrder::Tag => 1,
                margo_config::OverviewCycleOrder::Mixed => 2,
            },
            backdrop_color: {
                let c = cfg.overview_backdrop_color.0;
                gdk::RGBA::new(c[0], c[1], c[2], c[3])
            },
            backdrop_image: cfg.overview_backdrop_image.clone().unwrap_or_default(),
            style_model: gtk::StringList::new(&["Scroller (Recommended)", "Grid"]),
            cycle_model: gtk::StringList::new(&["Most recent (MRU)", "Tag order", "Mixed"]),
            mru_thumb: cfg.mru_thumb_height as f64,
            mru_scope_idx: match cfg.mru_scope.as_str() {
                "output" => 1,
                "workspace" => 2,
                _ => 0,
            },
            mru_filter_idx: if cfg.mru_filter == "appid" { 1 } else { 0 },
            mru_labels: cfg.mru_show_labels,
            mru_scope_model: gtk::StringList::new(&[
                "All windows",
                "This output",
                "This workspace",
            ]),
            mru_filter_model: gtk::StringList::new(&["All windows", "Same app"]),
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            OverviewSettingsInput::SetStyle(idx) => {
                let v = if idx == 0 { "scroller" } else { "grid" };
                apply("overview_style", v.to_string());
            }
            OverviewSettingsInput::SetScrollerZoom(v) => {
                apply("scroller_overview_zoom", format!("{v:.2}"));
            }
            OverviewSettingsInput::SetScrollerGap(v) => {
                apply("scroller_overview_gap", v.to_string());
            }
            OverviewSettingsInput::SetScrollerLoop(on) => {
                apply(
                    "scroller_overview_loop",
                    if on { "1" } else { "0" }.to_string(),
                );
            }
            OverviewSettingsInput::SetGridZoom(v) => {
                apply("overview_zoom", format!("{v:.2}"));
            }
            OverviewSettingsInput::SetGridGapInner(v) => {
                apply("overviewgappi", v.to_string());
            }
            OverviewSettingsInput::SetGridGapOuter(v) => {
                apply("overviewgappo", v.to_string());
            }
            OverviewSettingsInput::SetGridDim(v) => {
                apply("overview_dim_alpha", format!("{v:.2}"));
            }
            OverviewSettingsInput::SetGridTransition(v) => {
                apply("overview_transition_ms", v.to_string());
            }
            OverviewSettingsInput::SetTabMode(on) => {
                apply("ov_tab_mode", if on { "1" } else { "0" }.to_string());
            }
            OverviewSettingsInput::SetSelectedBorderMult(v) => {
                apply("overview_selected_border_multiplier", format!("{v:.1}"));
            }
            OverviewSettingsInput::SetCycleOrder(idx) => {
                let v = match idx {
                    1 => "tag",
                    2 => "mixed",
                    _ => "mru",
                };
                apply("overview_cycle_order", v.to_string());
            }
            OverviewSettingsInput::SetBackdropColor(rgba) => {
                self.backdrop_color = rgba;
                apply("overview_backdrop_color", rgba_to_hex(rgba));
            }
            OverviewSettingsInput::OpenBackdropImage => {
                let dialog = gtk::FileDialog::builder()
                    .title("Choose overview backdrop image")
                    .modal(true)
                    .build();
                // Only show image files in the picker.
                let filter = gtk::FileFilter::new();
                filter.set_name(Some("Images"));
                filter.add_mime_type("image/*");
                let filters = gio::ListStore::new::<gtk::FileFilter>();
                filters.append(&filter);
                dialog.set_filters(Some(&filters));
                dialog.set_default_filter(Some(&filter));

                let sender = sender.clone();
                dialog.open(gtk::Window::NONE, gio::Cancellable::NONE, move |result| {
                    if let Ok(file) = result
                        && let Some(path) = file.path()
                    {
                        sender.input(OverviewSettingsInput::SetBackdropImage(
                            path.to_string_lossy().to_string(),
                        ));
                    }
                });
            }
            OverviewSettingsInput::SetBackdropImage(path) => {
                self.backdrop_image = path.clone();
                apply("overview_backdrop_image", path);
            }
            OverviewSettingsInput::ClearBackdropImage => {
                self.backdrop_image.clear();
                apply("overview_backdrop_image", String::new());
            }
            OverviewSettingsInput::SetMruThumb(v) => {
                apply("mru_thumb_height", v.to_string());
            }
            OverviewSettingsInput::SetMruScope(idx) => {
                let v = match idx {
                    1 => "output",
                    2 => "workspace",
                    _ => "all",
                };
                apply("mru_scope", v.to_string());
            }
            OverviewSettingsInput::SetMruFilter(idx) => {
                apply(
                    "mru_filter",
                    if idx == 1 { "appid" } else { "all" }.to_string(),
                );
            }
            OverviewSettingsInput::SetMruLabels(on) => {
                apply("mru_show_labels", if on { "1" } else { "0" }.to_string());
            }
        }
    }
}
