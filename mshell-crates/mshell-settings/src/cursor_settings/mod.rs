//! Settings → Theme → Cursor: pick a pointer theme (with its real cursor
//! images) and a size, and apply both everywhere a desktop reads the
//! cursor from in one step: margo (`cursor_theme` / `cursor_size` +
//! reload, which also reaches XWayland and apps margo starts), GNOME
//! settings, GTK 3/4, the X11 default theme, the session environment, and
//! KDE / mdots files when present. A warning lists the places that are out
//! of step.
//!
//! Importing Windows `.cur` / `.ani` packs is not offered: it needs a
//! Windows cursor decoder (the source plugin shells out to `win2xcur`, which
//! isn't packaged for Arch outside the AUR).

mod apply;
mod engine;
mod preview;

use crate::compositor_conf::{patch_conf, read_raw, reload};
use crate::theme_files::{Roots, Step, icon_dirs, run_command, valid_theme_id};
use apply::{apply_commands, apply_files, read_all, summarize};
use engine::{CursorTheme, Reading, discover, effective, out_of_step};
use preview::{CursorImage, load_preview};
use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

/// Preview images are decoded at this nominal size.
const PREVIEW_SIZE: u32 = 32;
const MIN_SIZE: u32 = 8;
const MAX_SIZE: u32 = 256;

/// One theme with its decoded preview, built off the GTK thread.
#[derive(Debug)]
pub(crate) struct LoadedTheme {
    theme: CursorTheme,
    images: Vec<CursorImage>,
    sizes: Vec<u32>,
}

pub(crate) struct CursorSettingsModel {
    themes: Vec<LoadedTheme>,
    readings: Vec<Reading>,
    selected: Option<String>,
    size: u32,
    status: String,
    loading: bool,
    cards: Vec<(String, gtk::Button)>,
}

#[derive(Debug)]
pub(crate) enum CursorSettingsInput {
    Select(String),
    Size(u32),
    Apply,
    Refresh,
}

#[derive(Debug)]
pub(crate) enum CursorSettingsOutput {}

pub(crate) struct CursorSettingsInit {}

#[derive(Debug)]
pub(crate) enum CursorSettingsCommandOutput {
    Loaded(Vec<LoadedTheme>, Vec<Reading>),
    Applied(Vec<String>, Vec<Reading>),
}

/// margo's `config.conf` cursor keys, raw.
fn margo_reading() -> (Option<String>, Option<String>) {
    (read_raw("cursor_theme"), read_raw("cursor_size"))
}

fn load_all() -> CursorSettingsCommandOutput {
    let roots = Roots::from_env();
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let dirs = icon_dirs(&roots.home, &margo_fs::xdg::data_home(), &data_dirs);
    let themes = discover(&dirs)
        .into_iter()
        .map(|theme| {
            let (images, sizes) = load_preview(&theme.dir, PREVIEW_SIZE);
            LoadedTheme {
                theme,
                images,
                sizes,
            }
        })
        .collect();
    let readings = read_all(&roots, margo_reading(), &run_command);
    CursorSettingsCommandOutput::Loaded(themes, readings)
}

/// The sentence for the out-of-step warning, or `None` when every place
/// agrees with `theme` / `size`.
pub(crate) fn warning(readings: &[Reading], theme: &str, size: u32) -> Option<String> {
    let places = out_of_step(readings, theme, size);
    if places.is_empty() {
        return None;
    }
    let details: Vec<String> = places
        .iter()
        .filter_map(|p| readings.iter().find(|r| r.place == *p))
        .map(|r| {
            let t = r.theme.as_deref().unwrap_or("—");
            match r.size {
                Some(s) => format!("{} says {t} at {s} px", r.place.label()),
                None => format!("{} says {t}", r.place.label()),
            }
        })
        .collect();
    Some(format!(
        "The cursor settings disagree. In use: {theme} at {size} px, but {}. Apply to bring them in line.",
        details.join("; ")
    ))
}

#[relm4::component(pub)]
impl Component for CursorSettingsModel {
    type CommandOutput = CursorSettingsCommandOutput;
    type Input = CursorSettingsInput;
    type Output = CursorSettingsOutput;
    type Init = CursorSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("input-mouse-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Cursor",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Pick a pointer theme and size. Apply sets them for margo (live, XWayland too), GTK, GNOME settings, the X11 default and the session environment at once. Running apps that load their own cursor pick it up when restarted.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Box {
                    add_css_class: "asset-picker-warning",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 12,
                    #[watch]
                    set_visible: model.current_warning().is_some(),
                    gtk::Image {
                        set_icon_name: Some("dialog-warning-symbolic"),
                        set_valign: gtk::Align::Start,
                    },
                    gtk::Label {
                        add_css_class: "label-small",
                        set_hexpand: true,
                        set_halign: gtk::Align::Start,
                        set_xalign: 0.0,
                        set_wrap: true,
                        #[watch]
                        set_label: &model.current_warning().unwrap_or_default(),
                    },
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] crate::row::Row {
                        set_widget_name: "setting:conf:cursor_theme",
                        #[template_child] title { set_label: "Selected theme" },
                        #[template_child] desc {
                            #[watch]
                            set_label: &model.in_use_text(),
                        },
                        gtk::Button {
                            add_css_class: "ok-button-primary",
                            set_label: "Apply",
                            #[watch]
                            set_sensitive: model.selected.is_some() && !model.loading,
                            connect_clicked => CursorSettingsInput::Apply,
                        },
                    },
                    #[template] crate::row::Row {
                        set_widget_name: "setting:conf:cursor_size_picker",
                        #[template_child] title { set_label: "Size (px)" },
                        #[template_child] desc {
                            #[watch]
                            set_label: &model.sizes_text(),
                        },
                        #[name = "size_spin"]
                        gtk::SpinButton {
                            set_valign: gtk::Align::Center,
                            set_adjustment: &gtk::Adjustment::new(24.0, MIN_SIZE as f64, MAX_SIZE as f64, 1.0, 8.0, 0.0),
                            connect_value_changed[sender] => move |s| sender.input(CursorSettingsInput::Size(s.value() as u32)),
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_selectable: true,
                    #[watch]
                    set_visible: !model.status.is_empty(),
                    #[watch]
                    set_label: &model.status,
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Label {
                        add_css_class: "label-large-bold",
                        set_label: "Installed themes",
                        set_halign: gtk::Align::Start,
                        set_hexpand: true,
                    },
                    gtk::Button {
                        add_css_class: "flat",
                        set_icon_name: "view-refresh-symbolic",
                        set_tooltip_text: Some("Look for themes again"),
                        #[watch]
                        set_sensitive: !model.loading,
                        connect_clicked => CursorSettingsInput::Refresh,
                    },
                },

                #[name = "grid"]
                gtk::FlowBox {
                    add_css_class: "asset-picker-grid",
                    set_selection_mode: gtk::SelectionMode::None,
                    set_homogeneous: true,
                    set_min_children_per_line: 2,
                    set_max_children_per_line: 4,
                    set_column_spacing: 8,
                    set_row_spacing: 8,
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = CursorSettingsModel {
            themes: Vec::new(),
            readings: Vec::new(),
            selected: None,
            size: 24,
            status: "Looking for cursor themes…".into(),
            loading: true,
            cards: Vec::new(),
        };
        let widgets = view_output!();
        mshell_common::a11y::name(&root, "Cursor settings");
        sender.spawn_oneshot_command(load_all);
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
            CursorSettingsInput::Select(id) => {
                self.selected = Some(id);
                self.sync_cards();
            }
            CursorSettingsInput::Size(size) => self.size = size.clamp(MIN_SIZE, MAX_SIZE),
            CursorSettingsInput::Refresh => {
                self.loading = true;
                self.status = "Looking for cursor themes…".into();
                sender.spawn_oneshot_command(load_all);
            }
            CursorSettingsInput::Apply => {
                let Some(theme) = self.selected.clone() else {
                    return;
                };
                if !valid_theme_id(&theme) {
                    self.status = format!("“{theme}” is not a usable theme name.");
                } else {
                    let size = self.size;
                    // margo first: the key + reload reach its own pointer,
                    // XWayland and the apps it starts.
                    patch_conf(&[
                        ("cursor_theme", theme.clone()),
                        ("cursor_size", size.to_string()),
                    ]);
                    reload();
                    self.loading = true;
                    self.status = format!("Applying {theme} at {size} px…");
                    sender.spawn_oneshot_command(move || {
                        let roots = Roots::from_env();
                        let mut steps = vec![(engine::Place::Margo, Step::Done)];
                        steps.extend(apply_files(&roots, &theme, size));
                        steps.extend(apply_commands(&theme, size, &run_command));
                        let readings = read_all(&roots, margo_reading(), &run_command);
                        CursorSettingsCommandOutput::Applied(summarize(&steps), readings)
                    });
                }
            }
        }
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
            CursorSettingsCommandOutput::Loaded(themes, readings) => {
                self.loading = false;
                let (theme, size) = effective(&readings);
                self.readings = readings;
                self.themes = themes;
                if self.selected.is_none() {
                    self.selected = Some(theme);
                    self.size = size;
                    widgets.size_spin.set_value(f64::from(size));
                }
                self.status = if self.themes.is_empty() {
                    "No cursor themes found in ~/.local/share/icons, ~/.icons or the system icon folders.".into()
                } else {
                    String::new()
                };
                self.render_cards(&widgets.grid, &sender);
            }
            CursorSettingsCommandOutput::Applied(problems, readings) => {
                self.loading = false;
                self.readings = readings;
                self.status = if problems.is_empty() {
                    "Applied everywhere.".into()
                } else {
                    format!("Applied, except:\n{}", problems.join("\n"))
                };
            }
        }
        self.update_view(widgets, sender);
    }
}

impl CursorSettingsModel {
    /// The theme and size the warning compares against: margo's (or the
    /// first place that sets them).
    fn current_warning(&self) -> Option<String> {
        if self.readings.is_empty() {
            return None;
        }
        let (theme, size) = effective(&self.readings);
        warning(&self.readings, &theme, size)
    }

    fn in_use_text(&self) -> String {
        let selected = self.selected.as_deref().unwrap_or("none");
        if self.readings.is_empty() {
            return selected.to_string();
        }
        let (theme, size) = effective(&self.readings);
        format!("{selected} · in use now: {theme} at {size} px")
    }

    fn sizes_text(&self) -> String {
        let sizes = self
            .selected
            .as_deref()
            .and_then(|id| self.themes.iter().find(|t| t.theme.id == id))
            .map(|t| &t.sizes)
            .filter(|s| !s.is_empty());
        match sizes {
            Some(s) => format!(
                "This theme ships {} px; other sizes are scaled.",
                s.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
            ),
            None => "Logical pixels, 8–256.".into(),
        }
    }

    fn sync_cards(&self) {
        for (id, card) in &self.cards {
            if self.selected.as_deref() == Some(id.as_str()) {
                card.add_css_class("active");
            } else {
                card.remove_css_class("active");
            }
        }
    }

    fn render_cards(&mut self, grid: &gtk::FlowBox, sender: &ComponentSender<Self>) {
        grid.remove_all();
        self.cards.clear();
        for loaded in &self.themes {
            let card = cursor_card(loaded);
            let id = loaded.theme.id.clone();
            let s = sender.clone();
            let click_id = id.clone();
            card.connect_clicked(move |_| s.input(CursorSettingsInput::Select(click_id.clone())));
            grid.insert(&card, -1);
            self.cards.push((id, card));
        }
        self.sync_cards();
    }
}

/// A theme card: its cursor images in a row, then its name.
fn cursor_card(loaded: &LoadedTheme) -> gtk::Button {
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let strip = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    strip.add_css_class("asset-picker-preview");
    strip.set_halign(gtk::Align::Center);
    strip.set_height_request(PREVIEW_SIZE as i32);
    for image in loaded.images.iter().take(6) {
        let bytes = glib::Bytes::from(&image.pixels[..]);
        let texture = gdk::MemoryTexture::new(
            image.width as i32,
            image.height as i32,
            gdk::MemoryFormat::B8g8r8a8Premultiplied,
            &bytes,
            image.width as usize * 4,
        );
        let picture = gtk::Picture::for_paintable(&texture);
        picture.set_can_shrink(false);
        picture.set_valign(gtk::Align::Center);
        strip.append(&picture);
    }
    if loaded.images.is_empty() {
        let none = gtk::Label::new(Some("no preview"));
        none.add_css_class("label-small");
        strip.append(&none);
    }
    inner.append(&strip);
    let name = gtk::Label::new(Some(&loaded.theme.name));
    name.add_css_class("label-medium-bold");
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    inner.append(&name);
    if loaded.theme.name != loaded.theme.id {
        let id = gtk::Label::new(Some(&loaded.theme.id));
        id.add_css_class("label-small");
        id.set_ellipsize(gtk::pango::EllipsizeMode::End);
        inner.append(&id);
    }
    let button = gtk::Button::builder()
        .child(&inner)
        .css_classes(["ok-button-surface", "asset-picker-card"])
        .tooltip_text(loaded.theme.dir.display().to_string())
        .build();
    mshell_common::a11y::name(&button, &format!("Cursor theme {}", loaded.theme.name));
    button
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::Place;

    fn r(place: Place, theme: Option<&str>, size: Option<u32>) -> Reading {
        Reading {
            place,
            theme: theme.map(str::to_string),
            size,
        }
    }

    #[test]
    fn warning_names_each_place_that_disagrees() {
        let readings = [
            r(Place::Margo, Some("Bibata"), Some(32)),
            r(Place::Gsettings, Some("capitaine-cursors"), Some(24)),
            r(Place::XDefault, Some("Bibata"), None),
            r(Place::Gtk3, None, None),
        ];
        let w = warning(&readings, "Bibata", 32).expect("warning");
        assert!(w.contains("In use: Bibata at 32 px"), "{w}");
        assert!(
            w.contains("GNOME settings (gsettings) says capitaine-cursors at 24 px"),
            "{w}"
        );
        assert!(!w.contains("X11"), "{w}");
        assert_eq!(warning(&readings[2..], "Bibata", 32), None);
    }
}
