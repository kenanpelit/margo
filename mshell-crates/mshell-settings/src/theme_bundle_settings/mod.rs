//! Settings → Theme → Bundles: saved looks (colour scheme + matugen
//! settings + wallpaper set + preview) applied in one click, and their
//! editor.
//!
//! The bundles live in `theme.bundles`; applying one goes through
//! `mshell_cache::theme_bundle` (the same path as the Color Scheme menu's
//! bundle cards and `mshellctl theme bundle apply`). While a bundle is
//! active, wallpaper next/previous/random and the rotation step through its
//! wallpapers only.

mod editor;

use editor::{Draft, render_editor};
use mshell_cache::theme_bundle::{
    apply_theme_bundle, capture_path, clear_active_bundle, existing_wallpapers, load_thumbnail,
    preview_image,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, ThemeBundle, ThemeStoreFields};
use mshell_config::schema::themes::{MatugenMode, MatugenType, Themes};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::prelude::*;
use relm4::gtk::{gio, glib};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::PathBuf;
use std::time::Duration;

/// Seconds between "Capture desktop" and the capture, so Settings can be
/// closed first.
const CAPTURE_DELAY_SECS: u64 = 5;

pub(crate) struct ThemeBundleSettingsModel {
    bundles: Vec<ThemeBundle>,
    active: String,
    draft: Option<Draft>,
    status: String,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum ThemeBundleSettingsInput {
    Changed(Vec<ThemeBundle>, String),
    Apply(String),
    StopScoping,
    NewFromCurrent,
    Edit(String),
    Delete(String),
    DraftName(String),
    DraftTheme(u32),
    DraftScheme(u32),
    DraftMode(u32),
    AddWallpapers,
    WallpapersPicked(Vec<PathBuf>),
    AddCurrentWallpaper,
    RemoveWallpaper(usize),
    PickPreview,
    PreviewPicked(PathBuf),
    CapturePreview,
    ClearPreview,
    Save,
    Cancel,
}

#[derive(Debug)]
pub(crate) enum ThemeBundleSettingsOutput {}

pub(crate) struct ThemeBundleSettingsInit {}

#[derive(Debug)]
pub(crate) enum ThemeBundleSettingsCommandOutput {
    /// The desktop capture finished: the file, or why it failed.
    Captured(Result<PathBuf, String>),
}

#[relm4::component(pub)]
impl Component for ThemeBundleSettingsModel {
    type CommandOutput = ThemeBundleSettingsCommandOutput;
    type Input = ThemeBundleSettingsInput;
    type Output = ThemeBundleSettingsOutput;
    type Init = ThemeBundleSettingsInit;

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
                        set_icon_name: Some("palette-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Theme Bundles",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Save a colour scheme with its wallpapers and apply both in one click. While a bundle is active, next / previous / random wallpaper and the rotation use its wallpapers only.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,
                    gtk::Button {
                        add_css_class: "ok-button-primary",
                        set_label: "New bundle from current look",
                        #[watch]
                        set_sensitive: model.draft.is_none(),
                        connect_clicked => ThemeBundleSettingsInput::NewFromCurrent,
                    },
                    gtk::Button {
                        add_css_class: "ok-button-surface",
                        set_label: "Stop using the active bundle",
                        set_tooltip_text: Some("Wallpaper cycling goes back to the whole wallpaper folder; the scheme and wallpaper stay."),
                        #[watch]
                        set_visible: !model.active.is_empty(),
                        connect_clicked => ThemeBundleSettingsInput::StopScoping,
                    },
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    #[watch]
                    set_visible: !model.status.is_empty(),
                    #[watch]
                    set_label: &model.status,
                },

                #[name = "editor_box"]
                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 12,
                    #[watch]
                    set_visible: model.draft.is_some(),
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Saved bundles",
                    set_halign: gtk::Align::Start,
                },

                #[name = "list_box"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    set_halign: gtk::Align::Start,
                    set_xalign: 0.0,
                    set_wrap: true,
                    set_selectable: true,
                    set_label: "From a terminal or a keybind: mshellctl theme bundle list · mshellctl theme bundle apply <name> · mshellctl wallpaper next|random (scoped to the active bundle).",
                },
            },
        }
    }

    fn init(
        _init: Self::Init,
        _root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();
        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = config_manager().config();
            let bundles = config.clone().theme().bundles().get();
            let active = config.theme().active_bundle().get();
            sender_clone.input(ThemeBundleSettingsInput::Changed(bundles, active));
        });
        let model = ThemeBundleSettingsModel {
            bundles: Vec::new(),
            active: String::new(),
            draft: None,
            status: String::new(),
            _effects: effects,
        };
        let widgets = view_output!();
        ComponentParts { model, widgets }
    }

    fn update_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        // Whether the editor's widgets must be rebuilt (structure changed);
        // typing and dropdowns only update the draft.
        let mut rerender_editor = false;
        match message {
            ThemeBundleSettingsInput::Changed(bundles, active) => {
                self.bundles = bundles;
                self.active = active;
                render_list(&widgets.list_box, &self.bundles, &self.active, &sender);
            }
            ThemeBundleSettingsInput::Apply(name) => {
                self.status = match apply_theme_bundle(&name) {
                    Ok(()) => format!("Applied “{name}”."),
                    Err(e) => e.to_string(),
                };
            }
            ThemeBundleSettingsInput::StopScoping => {
                clear_active_bundle();
                self.status = "Wallpaper cycling uses the whole wallpaper folder again.".into();
            }
            ThemeBundleSettingsInput::NewFromCurrent => {
                let theme = config_manager().config().theme().get_untracked();
                let name = theme.unique_bundle_name("My look");
                let current = mshell_cache::wallpaper::current_wallpaper_path()
                    .map(|p| p.display().to_string());
                self.draft = Some(Draft::new(ThemeBundle::from_current(
                    &name,
                    &theme,
                    current.as_deref(),
                )));
                self.status.clear();
                rerender_editor = true;
            }
            ThemeBundleSettingsInput::Edit(name) => {
                if let Some(bundle) = self.bundles.iter().find(|b| b.name == name) {
                    self.draft = Some(Draft::edit(bundle.clone()));
                    self.status.clear();
                    rerender_editor = true;
                }
            }
            ThemeBundleSettingsInput::Delete(name) => {
                config_manager().update_config(|c| c.theme.delete_bundle(&name));
                self.status = format!("Deleted “{name}”.");
            }
            ThemeBundleSettingsInput::DraftName(name) => {
                if let Some(d) = &mut self.draft {
                    d.bundle.name = name;
                }
            }
            ThemeBundleSettingsInput::DraftTheme(i) => {
                if let (Some(d), Some(t)) = (&mut self.draft, Themes::all().get(i as usize)) {
                    d.bundle.theme = *t;
                }
            }
            ThemeBundleSettingsInput::DraftScheme(i) => {
                if let (Some(d), Some(t)) = (&mut self.draft, MatugenType::all().get(i as usize)) {
                    d.bundle.scheme_type = *t;
                }
            }
            ThemeBundleSettingsInput::DraftMode(i) => {
                if let (Some(d), Some(m)) = (&mut self.draft, MatugenMode::all().get(i as usize)) {
                    d.bundle.mode = *m;
                }
            }
            ThemeBundleSettingsInput::AddWallpapers => {
                let sender = sender.clone();
                image_dialog("Add wallpapers").open_multiple(
                    None::<&gtk::Window>,
                    gio::Cancellable::NONE,
                    move |res| {
                        let Ok(files) = res else { return };
                        let paths: Vec<PathBuf> = (0..files.n_items())
                            .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                            .filter_map(|f| f.path())
                            .collect();
                        sender.input(ThemeBundleSettingsInput::WallpapersPicked(paths));
                    },
                );
            }
            ThemeBundleSettingsInput::WallpapersPicked(paths) => {
                if let Some(d) = &mut self.draft {
                    d.add_wallpapers(paths.iter().map(|p| p.display().to_string()));
                    rerender_editor = true;
                }
            }
            ThemeBundleSettingsInput::AddCurrentWallpaper => {
                match mshell_cache::wallpaper::current_wallpaper_path() {
                    Some(p) => {
                        if let Some(d) = &mut self.draft {
                            d.add_wallpapers([p.display().to_string()]);
                            rerender_editor = true;
                        }
                    }
                    None => self.status = "No wallpaper is set right now.".into(),
                }
            }
            ThemeBundleSettingsInput::RemoveWallpaper(i) => {
                if let Some(d) = &mut self.draft
                    && i < d.bundle.wallpapers.len()
                {
                    d.bundle.wallpapers.remove(i);
                    rerender_editor = true;
                }
            }
            ThemeBundleSettingsInput::PickPreview => {
                let sender = sender.clone();
                image_dialog("Choose a preview image").open(
                    None::<&gtk::Window>,
                    gio::Cancellable::NONE,
                    move |res| {
                        if let Ok(file) = res
                            && let Some(path) = file.path()
                        {
                            sender.input(ThemeBundleSettingsInput::PreviewPicked(path));
                        }
                    },
                );
            }
            ThemeBundleSettingsInput::PreviewPicked(path) => {
                if let Some(d) = &mut self.draft {
                    d.bundle.preview = path.display().to_string();
                    rerender_editor = true;
                }
            }
            ThemeBundleSettingsInput::CapturePreview => {
                if let Some(d) = &self.draft {
                    if margo_fs::path::on_path("grim") {
                        let target = capture_path(&d.bundle);
                        sender.spawn_oneshot_command(move || {
                            std::thread::sleep(Duration::from_secs(CAPTURE_DELAY_SECS));
                            ThemeBundleSettingsCommandOutput::Captured(capture_desktop(&target))
                        });
                        self.status = format!(
                            "Capturing the desktop in {CAPTURE_DELAY_SECS} s: close Settings (Esc) to keep it out of the picture."
                        );
                    } else {
                        self.status =
                            "Desktop capture needs grim. Install it, or choose an image instead."
                                .into();
                    }
                }
            }
            ThemeBundleSettingsInput::ClearPreview => {
                if let Some(d) = &mut self.draft {
                    d.bundle.preview.clear();
                    rerender_editor = true;
                }
            }
            ThemeBundleSettingsInput::Save => {
                if let Some(d) = self.draft.take() {
                    let mut result = Ok(());
                    let original = d.original.clone();
                    let bundle = d.bundle.clone();
                    config_manager().update_config(|c| {
                        result = c.theme.save_bundle(original.as_deref(), bundle);
                    });
                    match result {
                        Ok(()) => self.status = format!("Saved “{}”.", d.bundle.name.trim()),
                        Err(e) => {
                            self.status = e.to_string();
                            self.draft = Some(d);
                        }
                    }
                }
            }
            ThemeBundleSettingsInput::Cancel => {
                self.draft = None;
                self.status.clear();
            }
        }
        if rerender_editor && let Some(d) = &self.draft {
            render_editor(&widgets.editor_box, d, &sender);
        }
        self.update_view(widgets, sender);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            ThemeBundleSettingsCommandOutput::Captured(Ok(path)) => {
                sender.input(ThemeBundleSettingsInput::PreviewPicked(path));
                self.status = "Desktop captured as the preview. Save to keep it.".into();
            }
            ThemeBundleSettingsCommandOutput::Captured(Err(e)) => self.status = e,
        }
    }
}

/// Capture every output into `target` with grim (blocking).
fn capture_desktop(target: &std::path::Path) -> Result<PathBuf, String> {
    if let Some(dir) = target.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    }
    let out = std::process::Command::new("grim")
        .arg("-t")
        .arg("png")
        .arg(target)
        .output()
        .map_err(|e| format!("Couldn't run grim: {e}"))?;
    if out.status.success() {
        Ok(target.to_path_buf())
    } else {
        Err(format!(
            "grim failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// An image file chooser.
fn image_dialog(title: &str) -> gtk::FileDialog {
    let dialog = gtk::FileDialog::builder().title(title).build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Images"));
    for ext in ["png", "jpg", "jpeg", "webp", "bmp", "gif"] {
        filter.add_suffix(ext);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog
}

/// One line under a bundle's name: scheme, mode and wallpaper count.
pub(super) fn bundle_summary(bundle: &ThemeBundle, wallpapers_on_disk: usize) -> String {
    let mut parts = vec![bundle.theme.label().to_string()];
    if bundle.theme == Themes::Wallpaper {
        parts.push(format!(
            "{} · {}",
            bundle.scheme_type.label(),
            bundle.mode.label()
        ));
    }
    let listed = bundle.wallpapers.len();
    parts.push(match (listed, wallpapers_on_disk) {
        (0, _) => "no wallpapers".to_string(),
        (n, m) if m == n => format!("{n} wallpaper{}", if n == 1 { "" } else { "s" }),
        (n, m) => format!("{m} of {n} wallpapers found"),
    });
    parts.join(" · ")
}

/// A preview picture that fills itself in off the GTK thread.
pub(super) fn thumbnail_picture(path: Option<PathBuf>, width: i32, height: i32) -> gtk::Picture {
    let picture = gtk::Picture::new();
    picture.set_size_request(width, height);
    picture.set_content_fit(gtk::ContentFit::Cover);
    picture.set_can_shrink(true);
    picture.add_css_class("theme-bundle-preview");
    if let Some(path) = path {
        let weak = picture.downgrade();
        let task = gio::spawn_blocking(move || load_thumbnail(&path, height as u32 * 2));
        glib::spawn_future_local(async move {
            if let Ok(Some(thumb)) = task.await
                && let Some(picture) = weak.upgrade()
            {
                picture.set_paintable(Some(thumb.texture().upcast_ref::<gtk::gdk::Paintable>()));
            }
        });
    }
    picture
}

fn render_list(
    container: &gtk::Box,
    bundles: &[ThemeBundle],
    active: &str,
    sender: &ComponentSender<ThemeBundleSettingsModel>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    if bundles.is_empty() {
        let row = gtk::Box::builder().css_classes(["action-row"]).build();
        row.append(
            &gtk::Label::builder()
                .css_classes(["label-small"])
                .label("No bundles yet. Set up a look, then “New bundle from current look”.")
                .halign(gtk::Align::Start)
                .xalign(0.0)
                .wrap(true)
                .build(),
        );
        container.append(&row);
        return;
    }
    for bundle in bundles {
        let row = gtk::Box::builder()
            .css_classes(["action-row"])
            .orientation(gtk::Orientation::Horizontal)
            .spacing(16)
            .build();
        row.append(&thumbnail_picture(preview_image(bundle), 96, 54));

        let text = gtk::Box::builder()
            .orientation(gtk::Orientation::Vertical)
            .valign(gtk::Align::Center)
            .hexpand(true)
            .build();
        let title = if bundle.name == active {
            format!("{} (active)", bundle.name)
        } else {
            bundle.name.clone()
        };
        text.append(
            &gtk::Label::builder()
                .css_classes(["label-medium-bold"])
                .label(title)
                .halign(gtk::Align::Start)
                .xalign(0.0)
                .ellipsize(gtk::pango::EllipsizeMode::End)
                .build(),
        );
        text.append(
            &gtk::Label::builder()
                .css_classes(["label-small"])
                .label(bundle_summary(bundle, existing_wallpapers(bundle).len()))
                .halign(gtk::Align::Start)
                .xalign(0.0)
                .wrap(true)
                .build(),
        );
        row.append(&text);

        let apply = gtk::Button::with_label("Apply");
        apply.add_css_class("ok-button-surface");
        apply.set_valign(gtk::Align::Center);
        let edit = gtk::Button::from_icon_name("document-edit-symbolic");
        edit.add_css_class("flat");
        edit.set_valign(gtk::Align::Center);
        edit.set_tooltip_text(Some("Edit this bundle"));
        mshell_common::a11y::name(&edit, &format!("Edit {}", bundle.name));
        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.add_css_class("flat");
        delete.set_valign(gtk::Align::Center);
        delete.set_tooltip_text(Some("Delete this bundle"));
        mshell_common::a11y::name(&delete, &format!("Delete {}", bundle.name));
        for (button, make) in [
            (&apply, ThemeBundleSettingsInput::Apply as fn(String) -> _),
            (&edit, ThemeBundleSettingsInput::Edit),
            (&delete, ThemeBundleSettingsInput::Delete),
        ] {
            let sender = sender.clone();
            let name = bundle.name.clone();
            button.connect_clicked(move |_| sender.input(make(name.clone())));
            row.append(button);
        }
        container.append(&row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_names_scheme_mode_and_missing_wallpapers() {
        let mut b = ThemeBundle {
            name: "x".into(),
            theme: Themes::Wallpaper,
            scheme_type: MatugenType::Vibrant,
            mode: MatugenMode::Light,
            wallpapers: vec!["/a".into(), "/b".into()],
            preview: String::new(),
        };
        assert_eq!(
            bundle_summary(&b, 2),
            "Wallpaper · Vibrant · Light · 2 wallpapers"
        );
        assert_eq!(
            bundle_summary(&b, 1),
            "Wallpaper · Vibrant · Light · 1 of 2 wallpapers found"
        );
        b.theme = Themes::TokyoNight;
        b.wallpapers.truncate(1);
        assert_eq!(bundle_summary(&b, 1), "Tokyo Night · 1 wallpaper");
        b.wallpapers.clear();
        assert_eq!(bundle_summary(&b, 0), "Tokyo Night · no wallpapers");
    }
}
