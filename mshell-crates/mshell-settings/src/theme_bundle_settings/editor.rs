//! The bundle editor: name, colour scheme, matugen settings, wallpapers and
//! preview of the bundle being created or edited. Nothing is written until
//! Save.

use super::{ThemeBundleSettingsInput as Input, ThemeBundleSettingsModel, thumbnail_picture};
use mshell_config::schema::config::ThemeBundle;
use mshell_config::schema::themes::{MatugenMode, MatugenType, Themes};
use relm4::ComponentSender;
use relm4::gtk;
use relm4::gtk::prelude::*;
use std::path::PathBuf;

/// The bundle being edited and the name it was saved under (`None` = new).
#[derive(Debug, Clone)]
pub(crate) struct Draft {
    pub original: Option<String>,
    pub bundle: ThemeBundle,
}

impl Draft {
    pub(crate) fn new(bundle: ThemeBundle) -> Self {
        Self {
            original: None,
            bundle,
        }
    }

    pub(crate) fn edit(bundle: ThemeBundle) -> Self {
        Self {
            original: Some(bundle.name.clone()),
            bundle,
        }
    }

    /// Append `paths`, skipping ones already in the bundle.
    pub(crate) fn add_wallpapers(&mut self, paths: impl IntoIterator<Item = String>) {
        for p in paths {
            if !p.trim().is_empty() && !self.bundle.wallpapers.contains(&p) {
                self.bundle.wallpapers.push(p);
            }
        }
    }
}

type Sender = ComponentSender<ThemeBundleSettingsModel>;

/// A boxed-list row: bold title + small description on the left, `control`
/// on the right (the `Row` template, built in code).
fn row(title: &str, desc: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::builder()
        .css_classes(["action-row"])
        .orientation(gtk::Orientation::Horizontal)
        .spacing(20)
        .build();
    let text = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .hexpand(true)
        .valign(gtk::Align::Center)
        .build();
    text.append(
        &gtk::Label::builder()
            .css_classes(["label-medium-bold"])
            .label(title)
            .halign(gtk::Align::Start)
            .build(),
    );
    if !desc.is_empty() {
        text.append(
            &gtk::Label::builder()
                .css_classes(["label-small"])
                .label(desc)
                .halign(gtk::Align::Start)
                .xalign(0.0)
                .wrap(true)
                .build(),
        );
    }
    row.append(&text);
    control.set_valign(gtk::Align::Center);
    row.append(control);
    row
}

fn dropdown(
    labels: &[&str],
    selected: usize,
    on_pick: impl Fn(u32) -> Input + 'static,
    sender: &Sender,
) -> gtk::DropDown {
    let dd = gtk::DropDown::from_strings(labels);
    dd.set_selected(selected as u32);
    let sender = sender.clone();
    dd.connect_selected_notify(move |dd| sender.input(on_pick(dd.selected())));
    dd
}

fn button(label: &str, input: impl Fn() -> Input + 'static, sender: &Sender) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("ok-button-surface");
    let sender = sender.clone();
    b.connect_clicked(move |_| sender.input(input()));
    b
}

fn file_name(path: &str) -> String {
    PathBuf::from(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// Rebuild the editor for `draft` into `container`.
pub(crate) fn render_editor(container: &gtk::Box, draft: &Draft, sender: &Sender) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let b = &draft.bundle;
    let heading = if draft.original.is_some() {
        format!("Edit “{}”", b.name)
    } else {
        "New bundle".to_string()
    };
    container.append(
        &gtk::Label::builder()
            .css_classes(["label-large-bold"])
            .label(heading)
            .halign(gtk::Align::Start)
            .build(),
    );

    // ── Look ──
    let look = gtk::Box::builder()
        .css_classes(["boxed-list"])
        .orientation(gtk::Orientation::Vertical)
        .build();
    let name = gtk::Entry::builder().text(&b.name).width_chars(24).build();
    {
        let sender = sender.clone();
        name.connect_changed(move |e| sender.input(Input::DraftName(e.text().to_string())));
    }
    look.append(&row(
        "Title",
        "Shown on the card; unique among bundles.",
        &name,
    ));
    let themes: Vec<&str> = Themes::all().iter().map(|t| t.label()).collect();
    let theme_idx = Themes::all()
        .iter()
        .position(|t| *t == b.theme)
        .unwrap_or(0);
    look.append(&row(
        "Colour scheme",
        "A built-in palette, or Wallpaper to generate one from the bundle's wallpaper.",
        &dropdown(&themes, theme_idx, Input::DraftTheme, sender),
    ));
    let schemes: Vec<&str> = MatugenType::all().iter().map(|t| t.label()).collect();
    let scheme_idx = MatugenType::all()
        .iter()
        .position(|t| *t == b.scheme_type)
        .unwrap_or(0);
    look.append(&row(
        "Generator scheme",
        "How the Wallpaper scheme builds its palette.",
        &dropdown(&schemes, scheme_idx, Input::DraftScheme, sender),
    ));
    let modes: Vec<&str> = MatugenMode::all().iter().map(|m| m.label()).collect();
    let mode_idx = MatugenMode::all()
        .iter()
        .position(|m| *m == b.mode)
        .unwrap_or(0);
    look.append(&row(
        "Light or dark",
        "Polarity of the Wallpaper scheme.",
        &dropdown(&modes, mode_idx, Input::DraftMode, sender),
    ));
    container.append(&look);

    // ── Wallpapers ──
    container.append(
        &gtk::Label::builder()
            .css_classes(["label-medium-bold"])
            .label("Wallpapers")
            .halign(gtk::Align::Start)
            .build(),
    );
    let walls = gtk::Box::builder()
        .css_classes(["boxed-list"])
        .orientation(gtk::Orientation::Vertical)
        .build();
    if b.wallpapers.is_empty() {
        let empty = gtk::Box::builder().css_classes(["action-row"]).build();
        empty.append(
            &gtk::Label::builder()
                .css_classes(["label-small"])
                .label("No wallpapers: applying keeps the current one.")
                .halign(gtk::Align::Start)
                .build(),
        );
        walls.append(&empty);
    }
    for (i, path) in b.wallpapers.iter().enumerate() {
        let r = gtk::Box::builder()
            .css_classes(["action-row"])
            .orientation(gtk::Orientation::Horizontal)
            .spacing(16)
            .build();
        r.append(&thumbnail_picture(Some(PathBuf::from(path)), 64, 36));
        let label = gtk::Label::builder()
            .css_classes(["label-small"])
            .label(file_name(path))
            .tooltip_text(path)
            .halign(gtk::Align::Start)
            .hexpand(true)
            .xalign(0.0)
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .build();
        r.append(&label);
        let remove = gtk::Button::from_icon_name("user-trash-symbolic");
        remove.add_css_class("flat");
        remove.set_valign(gtk::Align::Center);
        remove.set_tooltip_text(Some("Remove from the bundle"));
        mshell_common::a11y::name(&remove, &format!("Remove {}", file_name(path)));
        let s = sender.clone();
        remove.connect_clicked(move |_| s.input(Input::RemoveWallpaper(i)));
        r.append(&remove);
        walls.append(&r);
    }
    container.append(&walls);
    let wall_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    wall_buttons.append(&button("Add images…", || Input::AddWallpapers, sender));
    wall_buttons.append(&button(
        "Add current wallpaper",
        || Input::AddCurrentWallpaper,
        sender,
    ));
    container.append(&wall_buttons);

    // ── Preview ──
    container.append(
        &gtk::Label::builder()
            .css_classes(["label-medium-bold"])
            .label("Preview")
            .halign(gtk::Align::Start)
            .build(),
    );
    let preview_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    let preview = if b.preview.trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(b.preview.trim()))
    };
    let has_preview = preview.is_some();
    preview_row.append(&thumbnail_picture(
        preview.or_else(|| b.wallpapers.first().map(PathBuf::from)),
        160,
        90,
    ));
    let preview_buttons = gtk::Box::new(gtk::Orientation::Vertical, 8);
    preview_buttons.set_valign(gtk::Align::Center);
    preview_buttons.append(&button("Capture desktop", || Input::CapturePreview, sender));
    preview_buttons.append(&button("Choose image…", || Input::PickPreview, sender));
    if has_preview {
        preview_buttons.append(&button(
            "Use the first wallpaper",
            || Input::ClearPreview,
            sender,
        ));
    }
    preview_row.append(&preview_buttons);
    container.append(&preview_row);

    // ── Save / Cancel ──
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let save = gtk::Button::with_label("Save bundle");
    save.add_css_class("ok-button-primary");
    {
        let s = sender.clone();
        save.connect_clicked(move |_| s.input(Input::Save));
    }
    actions.append(&save);
    actions.append(&button("Cancel", || Input::Cancel, sender));
    container.append(&actions);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_wallpapers_skips_repeats_and_blanks() {
        let mut d = Draft::new(ThemeBundle {
            wallpapers: vec!["/w/a.png".into()],
            ..ThemeBundle::default()
        });
        d.add_wallpapers(["/w/a.png", "/w/b.png", " ", "/w/b.png"].map(String::from));
        assert_eq!(d.bundle.wallpapers, ["/w/a.png", "/w/b.png"]);
        assert!(d.original.is_none());
        let e = Draft::edit(ThemeBundle {
            name: "Night".into(),
            ..ThemeBundle::default()
        });
        assert_eq!(e.original.as_deref(), Some("Night"));
    }

    #[test]
    fn file_names_fall_back_to_the_path() {
        assert_eq!(file_name("/w/sea.jpg"), "sea.jpg");
        assert_eq!(file_name("/"), "/");
    }
}
