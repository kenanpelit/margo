//! Settings → Theme → Icons: every installed icon theme shown with its own
//! folder, home, document, image, file-manager, browser, terminal and
//! settings icons (resolved per the Icon Theme spec, inherited themes
//! included), applied to GTK, GNOME settings, Qt (qt5ct/qt6ct,
//! `QT_ICON_THEME`) and KDE at once, with a warning when those disagree,
//! and installable from a local folder.
//!
//! Not included: downloading from gnome-look.org (it would need an
//! unauthenticated scraper for the OCS API plus archive unpacking of
//! untrusted tarballs; install from an extracted folder instead) and Papirus
//! folder recolouring.

mod apply;
mod engine;

use crate::theme_files::{Roots, icon_dirs, run_command, valid_theme_id};
use apply::{
    IconPlace, IconReading, apply_commands, apply_files, in_use, out_of_step, read_all, summarize,
};
use engine::{IconTheme, SAMPLES, discover, find_icon, install_from};
use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, gio, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Preview icon size in px.
const ICON_PX: u32 = 32;

/// A decoded preview icon (straight RGBA).
#[derive(Debug)]
pub(crate) struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// One theme with its decoded sample icons (`None`: the theme lacks it).
#[derive(Debug)]
pub(crate) struct LoadedIconTheme {
    id: String,
    name: String,
    comment: String,
    icons: Vec<Option<Pixels>>,
}

pub(crate) struct IconSettingsModel {
    themes: Vec<LoadedIconTheme>,
    readings: Vec<IconReading>,
    selected: Option<String>,
    status: String,
    loading: bool,
    cards: Vec<(String, gtk::Button)>,
}

#[derive(Debug)]
pub(crate) enum IconSettingsInput {
    Select(String),
    Apply,
    Refresh,
    InstallFolder,
    Install(PathBuf),
}

#[derive(Debug)]
pub(crate) enum IconSettingsOutput {}

pub(crate) struct IconSettingsInit {}

#[derive(Debug)]
pub(crate) enum IconSettingsCommandOutput {
    Loaded(Vec<LoadedIconTheme>, Vec<IconReading>),
    Applied(Vec<String>, Vec<IconReading>),
    Installed(Result<Vec<String>, String>),
}

/// Decode `path` (SVG or PNG) to fit `px`×`px`. Blocking.
fn decode_icon(path: &Path, px: u32) -> Option<Pixels> {
    let never = AtomicBool::new(false);
    let fit = move |w: u32, h: u32| {
        let long = w.max(h).max(1);
        (
            (u64::from(w) * u64::from(px) / u64::from(long)).max(1) as u32,
            (u64::from(h) * u64::from(px) / u64::from(long)).max(1) as u32,
        )
    };
    let img = mshell_image::lut::decode_rgba_scaled(path, fit, &never)?;
    let (width, height) = img.dimensions();
    Some(Pixels {
        width,
        height,
        rgba: img.into_raw(),
    })
}

fn search_roots() -> (Roots, Vec<PathBuf>) {
    let roots = Roots::from_env();
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let dirs = icon_dirs(&roots.home, &margo_fs::xdg::data_home(), &data_dirs);
    (roots, dirs)
}

fn load_all() -> IconSettingsCommandOutput {
    let (roots, dirs) = search_roots();
    let all: Vec<IconTheme> = discover(&dirs);
    let themes = all
        .iter()
        .map(|t| LoadedIconTheme {
            id: t.id.clone(),
            name: t.name().to_string(),
            comment: t.index.comment.clone(),
            icons: SAMPLES
                .iter()
                .map(|(_, names)| {
                    find_icon(&all, &t.id, names, ICON_PX).and_then(|p| decode_icon(&p, ICON_PX))
                })
                .collect(),
        })
        .collect();
    IconSettingsCommandOutput::Loaded(themes, read_all(&roots, &run_command))
}

/// The out-of-step sentence, or `None` when every place agrees.
pub(crate) fn warning(readings: &[IconReading]) -> Option<String> {
    let theme = in_use(readings)?;
    let places = out_of_step(readings, &theme);
    if places.is_empty() {
        return None;
    }
    let details: Vec<String> = readings
        .iter()
        .filter(|r| places.contains(&r.place))
        .map(|r| {
            format!(
                "{} says {}",
                r.place.label(),
                r.theme.as_deref().unwrap_or("—")
            )
        })
        .collect();
    Some(format!(
        "The icon theme settings disagree. In use: {theme}, but {}. Use a theme to bring them in line.",
        details.join("; ")
    ))
}

#[relm4::component(pub)]
impl Component for IconSettingsModel {
    type CommandOutput = IconSettingsCommandOutput;
    type Input = IconSettingsInput;
    type Output = IconSettingsOutput;
    type Init = IconSettingsInit;

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
                        set_icon_name: Some("folder-pictures-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Icons",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "The icon theme apps use. Each card shows the theme's own icons. Use sets it for GTK, GNOME settings, Qt (qt5ct / qt6ct, QT_ICON_THEME) and KDE at once; apps pick it up when restarted. The shell's own icons are set under Theme → Scheme.",
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
                    set_visible: warning(&model.readings).is_some(),
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
                        set_label: &warning(&model.readings).unwrap_or_default(),
                    },
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template] crate::row::Row {
                        set_widget_name: "setting:icons:theme",
                        #[template_child] title { set_label: "Selected theme" },
                        #[template_child] desc {
                            #[watch]
                            set_label: &model.selected_text(),
                        },
                        gtk::Button {
                            add_css_class: "ok-button-primary",
                            set_label: "Use this theme",
                            #[watch]
                            set_sensitive: model.selected.is_some() && !model.loading,
                            connect_clicked => IconSettingsInput::Apply,
                        },
                    },
                    #[template] crate::row::Row {
                        set_widget_name: "setting:icons:install",
                        #[template_child] title { set_label: "Install from a folder" },
                        #[template_child] desc { set_label: "A theme folder, or a folder holding several (extract a downloaded archive first). Copied to ~/.local/share/icons." },
                        gtk::Button {
                            add_css_class: "ok-button-surface",
                            set_label: "Choose folder…",
                            #[watch]
                            set_sensitive: !model.loading,
                            connect_clicked => IconSettingsInput::InstallFolder,
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
                        connect_clicked => IconSettingsInput::Refresh,
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
        let model = IconSettingsModel {
            themes: Vec::new(),
            readings: Vec::new(),
            selected: None,
            status: "Looking for icon themes…".into(),
            loading: true,
            cards: Vec::new(),
        };
        let widgets = view_output!();
        mshell_common::a11y::name(&root, "Icon theme settings");
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
            IconSettingsInput::Select(id) => {
                self.selected = Some(id);
                self.sync_cards();
            }
            IconSettingsInput::Refresh => {
                self.loading = true;
                self.status = "Looking for icon themes…".into();
                sender.spawn_oneshot_command(load_all);
            }
            IconSettingsInput::Apply => {
                let Some(theme) = self.selected.clone() else {
                    return;
                };
                if valid_theme_id(&theme) {
                    let session_sets_qt = self
                        .readings
                        .iter()
                        .any(|r| r.place == IconPlace::SessionEnv && r.theme.is_some());
                    self.loading = true;
                    self.status = format!("Using {theme}…");
                    sender.spawn_oneshot_command(move || {
                        let roots = Roots::from_env();
                        let mut steps = apply_files(&roots, &theme);
                        steps.extend(apply_commands(&theme, session_sets_qt, &run_command));
                        IconSettingsCommandOutput::Applied(
                            summarize(&steps),
                            read_all(&roots, &run_command),
                        )
                    });
                } else {
                    self.status = format!("“{theme}” is not a usable theme name.");
                }
            }
            IconSettingsInput::InstallFolder => {
                let dialog = gtk::FileDialog::builder()
                    .title("Choose an icon theme folder")
                    .build();
                let sender = sender.clone();
                dialog.select_folder(None::<&gtk::Window>, gio::Cancellable::NONE, move |res| {
                    if let Ok(folder) = res
                        && let Some(path) = folder.path()
                    {
                        sender.input(IconSettingsInput::Install(path));
                    }
                });
            }
            IconSettingsInput::Install(path) => {
                self.loading = true;
                self.status = format!("Installing from {}…", path.display());
                sender.spawn_oneshot_command(move || {
                    let dest = margo_fs::xdg::data_home().join("icons");
                    IconSettingsCommandOutput::Installed(install_from(&path, &dest))
                });
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
            IconSettingsCommandOutput::Loaded(themes, readings) => {
                self.loading = false;
                if self.selected.is_none() {
                    self.selected = in_use(&readings);
                }
                self.readings = readings;
                self.themes = themes;
                self.status = if self.themes.is_empty() {
                    "No icon themes found.".into()
                } else {
                    String::new()
                };
                self.render_cards(&widgets.grid, &sender);
            }
            IconSettingsCommandOutput::Applied(problems, readings) => {
                self.loading = false;
                self.readings = readings;
                self.status = if problems.is_empty() {
                    "Icon theme set everywhere. Restart apps to see it.".into()
                } else {
                    format!("Set, except:\n{}", problems.join("\n"))
                };
            }
            IconSettingsCommandOutput::Installed(result) => match result {
                Ok(ids) => {
                    self.selected = ids.first().cloned();
                    self.status = format!("Installed {}.", ids.join(", "));
                    sender.spawn_oneshot_command(load_all);
                }
                Err(e) => {
                    self.loading = false;
                    self.status = e;
                }
            },
        }
        self.update_view(widgets, sender);
    }
}

impl IconSettingsModel {
    fn selected_text(&self) -> String {
        let selected = self.selected.as_deref().unwrap_or("none");
        match in_use(&self.readings) {
            Some(theme) => format!("{selected} · in use now: {theme}"),
            None => selected.to_string(),
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
        for theme in &self.themes {
            let card = icon_card(theme);
            let s = sender.clone();
            let id = theme.id.clone();
            card.connect_clicked(move |_| s.input(IconSettingsInput::Select(id.clone())));
            grid.insert(&card, -1);
            self.cards.push((theme.id.clone(), card));
        }
        self.sync_cards();
    }
}

fn texture(p: &Pixels) -> gdk::Texture {
    let bytes = glib::Bytes::from(&p.rgba[..]);
    gdk::MemoryTexture::new(
        p.width as i32,
        p.height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        p.width as usize * 4,
    )
    .upcast()
}

/// A theme card: its sample icons in two rows of four, then its name.
fn icon_card(theme: &LoadedIconTheme) -> gtk::Button {
    let inner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let icons = gtk::Grid::builder()
        .column_spacing(8)
        .row_spacing(8)
        .halign(gtk::Align::Center)
        .css_classes(["asset-picker-preview"])
        .build();
    for (i, (icon, (label, _))) in theme.icons.iter().zip(SAMPLES).enumerate() {
        let cell: gtk::Widget = match icon {
            Some(p) => {
                let picture = gtk::Picture::for_paintable(&texture(p));
                picture.set_can_shrink(false);
                picture.upcast()
            }
            None => {
                let missing = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                missing.set_size_request(ICON_PX as i32, ICON_PX as i32);
                missing.upcast()
            }
        };
        cell.set_tooltip_text(Some(label));
        icons.attach(&cell, (i % 4) as i32, (i / 4) as i32, 1, 1);
    }
    inner.append(&icons);
    let name = gtk::Label::new(Some(&theme.name));
    name.add_css_class("label-medium-bold");
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    inner.append(&name);
    if theme.name != theme.id {
        let id = gtk::Label::new(Some(&theme.id));
        id.add_css_class("label-small");
        id.set_ellipsize(gtk::pango::EllipsizeMode::End);
        inner.append(&id);
    }
    let button = gtk::Button::builder()
        .child(&inner)
        .css_classes(["ok-button-surface", "asset-picker-card"])
        .build();
    if !theme.comment.is_empty() {
        button.set_tooltip_text(Some(&theme.comment));
    }
    mshell_common::a11y::name(&button, &format!("Icon theme {}", theme.name));
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warning_lists_only_the_disagreeing_places() {
        let r = |place, theme: Option<&str>| IconReading {
            place,
            theme: theme.map(str::to_string),
        };
        let readings = [
            r(IconPlace::Gsettings, Some("kora")),
            r(IconPlace::Gtk3, Some("kora")),
            r(IconPlace::Qt6ct, Some("breeze")),
            r(IconPlace::Kde, None),
        ];
        let w = warning(&readings).expect("warning");
        assert!(w.contains("In use: kora"), "{w}");
        assert!(w.contains("Qt 6 (qt6ct) says breeze"), "{w}");
        assert!(!w.contains("GTK 3"), "{w}");
        assert_eq!(warning(&readings[..2]), None);
        assert_eq!(warning(&[]), None);
    }

    /// Resolves and decodes this machine's icon themes (run with
    /// `--ignored`): every installed theme previews at least one icon.
    #[test]
    #[ignore = "reads the installed icon themes"]
    fn installed_themes_preview_real_icons() {
        let IconSettingsCommandOutput::Loaded(themes, _) = load_all() else {
            panic!("load_all returns Loaded");
        };
        assert!(!themes.is_empty());
        for t in &themes {
            let n = t.icons.iter().filter(|i| i.is_some()).count();
            eprintln!("{:<28} {n}/{}", t.id, SAMPLES.len());
        }
        assert!(themes.iter().any(|t| t.icons.iter().all(Option::is_some)));
    }
}
