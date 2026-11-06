//! Shelf menu — the panel for `MenuType::Shelf`: files you dropped in to
//! hold for a moment, ready to be dragged out into another app (a browser
//! upload, a chat, a mail, a folder).
//!
//! Panel header (§12) with *Copy all* and *Clear*, a drop zone (the whole
//! panel takes drops), *Drag all*, then one boxed-list row per file: its
//! icon, name and folder; drag the row out, click it to open the file, or
//! use the row buttons — show in folder, copy path, remove from the shelf.
//! A file that has gone from disk stays listed, dimmed, until removed.
//! The shelf keeps references only: nothing here moves or deletes files.

use super::dnd;
use crate::menus::menu_widgets::panel_header::PanelHeader;
use crate::menus::reveal_poll::{RevealGate, watch_stream};
use crate::shelf::{self, ShelfCommand, ShelfItem, ShelfState};
use mshell_config::schema::menu_widgets::MenuWidget;
use relm4::gtk::gio;
use relm4::gtk::prelude::{BoxExt, ButtonExt, DisplayExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::path::{Path, PathBuf};

pub(crate) struct ShelfMenuWidgetModel {
    state: ShelfState,
    list: gtk::Box,
    subtitle: gtk::Label,
    /// What "Drag all" offers (refreshed on every render).
    drag_all_paths: std::rc::Rc<std::cell::RefCell<Vec<PathBuf>>>,
}

#[derive(Debug)]
pub(crate) enum ShelfMenuWidgetInput {
    Open(PathBuf),
    ShowInFolder(PathBuf),
    CopyPath(PathBuf),
    Remove(PathBuf),
    CopyAll,
    Clear,
}

#[derive(Debug)]
pub(crate) enum ShelfMenuWidgetOutput {}

pub(crate) struct ShelfMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum ShelfMenuWidgetCommandOutput {
    State(ShelfState),
}

#[relm4::component(pub(crate))]
impl Component for ShelfMenuWidgetModel {
    type CommandOutput = ShelfMenuWidgetCommandOutput;
    type Input = ShelfMenuWidgetInput;
    type Output = ShelfMenuWidgetOutput;
    type Init = ShelfMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "shelf-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            #[local_ref]
            header_root -> gtk::Box {},

            #[name = "drop_zone"]
            gtk::Box {
                add_css_class: "shelf-drop-zone",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,
                set_halign: gtk::Align::Fill,

                gtk::Image {
                    set_icon_name: Some("folder-download-symbolic"),
                },
                gtk::Label {
                    set_css_classes: &["label-small", "dim-label"],
                    set_label: "Drop files here to hold them. Drag them back out into any app.",
                    set_wrap: true,
                    set_xalign: 0.0,
                    set_hexpand: true,
                },
                #[name = "drag_all"]
                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_label: "Drag all",
                    set_valign: gtk::Align::Center,
                    set_tooltip_text: Some("Drag every file on the shelf into another app"),
                },
            },

            #[name = "broken_label"]
            gtk::Label {
                set_css_classes: &["label-small-error"],
                set_xalign: 0.0,
                set_wrap: true,
                set_visible: false,
            },

            #[name = "fill_list"]
            gtk::ScrolledWindow {
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_min_content_height: 0,
                set_propagate_natural_height: true,

                #[local_ref]
                list -> gtk::Box {
                    set_css_classes: &["ok-boxed-list", "compact", "shelf-list"],
                    set_orientation: gtk::Orientation::Vertical,
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let (s_copy, s_clear) = (sender.clone(), sender.clone());
        let header = PanelHeader::new("folder-download-symbolic", MenuWidget::Shelf.display_name())
            .with_subtitle()
            .action("edit-copy-symbolic", "Copy all files", move || {
                s_copy.input(ShelfMenuWidgetInput::CopyAll);
            })
            .action("edit-clear-all-symbolic", "Clear the shelf", move || {
                s_clear.input(ShelfMenuWidgetInput::Clear);
            });
        let header_root = header.root().clone();
        let subtitle = header.subtitle().cloned().unwrap_or_default();
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);

        let open = RevealGate::follow_mapped(&root);
        sender.command(|out, shutdown| async move {
            let watch = open.stream_while_revealed(
                || watch_stream(shelf::subscribe()),
                |state| {
                    let _ = out.send(ShelfMenuWidgetCommandOutput::State(state));
                },
            );
            tokio::select! {
                () = shutdown.wait() => {}
                () = watch => {}
            }
        });

        let model = ShelfMenuWidgetModel {
            state: ShelfState::default(),
            list: list.clone(),
            subtitle,
            drag_all_paths: Default::default(),
        };
        let widgets = view_output!();
        crate::menus::list_fit::mark_menu_list(&widgets.fill_list, 420);
        // Drops anywhere on the panel land on the shelf.
        dnd::attach_drop_target(&root);
        dnd::attach_drag_source_with(
            &widgets.drag_all,
            model.drag_all_paths.clone(),
            &widgets.drag_all,
        );
        model.render(&widgets, &sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, _sender: ComponentSender<Self>, root: &Self::Root) {
        match msg {
            ShelfMenuWidgetInput::Open(p) => open(&p),
            ShelfMenuWidgetInput::ShowInFolder(p) => show_in_folder(p),
            ShelfMenuWidgetInput::CopyPath(p) => {
                root.display().clipboard().set_text(&p.to_string_lossy());
            }
            ShelfMenuWidgetInput::Remove(p) => shelf::send(ShelfCommand::Remove(p)),
            ShelfMenuWidgetInput::CopyAll => {
                let paths: Vec<PathBuf> = existing(&self.state.items);
                if !paths.is_empty() {
                    root.display()
                        .clipboard()
                        .set_content(Some(&dnd::clipboard_content(&paths)))
                        .ok();
                }
            }
            ShelfMenuWidgetInput::Clear => shelf::send(ShelfCommand::Clear),
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        msg: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match msg {
            ShelfMenuWidgetCommandOutput::State(s) => {
                self.state = s;
                self.render(widgets, &sender);
            }
        }
    }
}

/// Paths of the items still on disk (a drag or copy can't offer the rest).
fn existing(items: &[ShelfItem]) -> Vec<PathBuf> {
    items
        .iter()
        .filter(|i| i.path.exists())
        .map(|i| i.path.clone())
        .collect()
}

fn open(p: &Path) {
    if let Err(e) = mshell_process::desktop::open(p) {
        tracing::warn!(error = %e, "shelf: could not open the file");
    }
}

/// Highlight the file in the file manager (`FileManager1.ShowItems`), or
/// open its folder when no file manager answers.
fn show_in_folder(p: PathBuf) {
    relm4::spawn(async move {
        let uri = shelf::file_uri(&p);
        let shown = async {
            let conn = zbus::Connection::session().await?;
            conn.call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItems",
                &(vec![uri.as_str()], ""),
            )
            .await
            .map(|_| ())
        }
        .await;
        if shown.is_err()
            && let Some(dir) = p.parent()
        {
            open(dir);
        }
    });
}

/// `n files` / `n files · 1 missing`.
fn summary(items: &[ShelfItem], missing: usize) -> String {
    let n = items.len();
    let mut s = match n {
        0 => "Empty".to_string(),
        1 => "1 file".to_string(),
        n => format!("{n} files"),
    };
    if missing > 0 {
        s.push_str(&format!(" · {missing} missing"));
    }
    s
}

impl ShelfMenuWidgetModel {
    fn render(&self, widgets: &ShelfMenuWidgetModelWidgets, sender: &ComponentSender<Self>) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        let mut missing = 0;
        for item in &self.state.items {
            let present = item.path.exists();
            missing += usize::from(!present);
            self.list.append(&row(item, present, sender));
        }
        self.list.set_visible(!self.state.items.is_empty());
        self.subtitle
            .set_label(&summary(&self.state.items, missing));
        widgets.broken_label.set_label(&match &self.state.broken {
            Some(e) => format!(
                "shelf.json can't be read or saved ({e}); changes stay until the shell restarts."
            ),
            None => String::new(),
        });
        widgets
            .broken_label
            .set_visible(self.state.broken.is_some());
        // "Drag all" offers the files present right now.
        let all = existing(&self.state.items);
        widgets.drag_all.set_sensitive(!all.is_empty());
        // The one drag source on "Drag all" reads the current list.
        *self.drag_all_paths.borrow_mut() = all;
    }
}

fn icon_for(path: &Path) -> gtk::Image {
    if path.is_dir() {
        return gtk::Image::from_icon_name("folder-symbolic");
    }
    let (ct, _) = gio::content_type_guess(Some(path), None);
    gtk::Image::from_gicon(&gio::content_type_get_symbolic_icon(&ct))
}

fn row(
    item: &ShelfItem,
    present: bool,
    sender: &ComponentSender<ShelfMenuWidgetModel>,
) -> gtk::Box {
    let path = item.path.clone();
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    row.add_css_class("shelf-row");
    if !present {
        row.add_css_class("missing");
    }

    // The main part: open on click, drag out into another app.
    let main = gtk::Button::new();
    main.add_css_class("shelf-row-main");
    main.set_hexpand(true);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let icon = icon_for(&path);
    icon.set_valign(gtk::Align::Center);
    inner.append(&icon);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.set_hexpand(true);
    let name = gtk::Label::new(Some(
        &path
            .file_name()
            .map_or_else(|| path.to_string_lossy(), |n| n.to_string_lossy()),
    ));
    name.add_css_class("label-medium");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    text.append(&name);
    let folder = path
        .parent()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    let detail = gtk::Label::new(Some(&if present {
        folder
    } else {
        format!("Missing · {folder}")
    }));
    detail.set_css_classes(&["label-small", "dim-label"]);
    detail.set_xalign(0.0);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    text.append(&detail);
    inner.append(&text);
    main.set_child(Some(&inner));
    main.set_tooltip_text(Some(&format!(
        "{}\nClick to open · drag into another app",
        path.display()
    )));
    main.set_sensitive(present);
    {
        let (s, p) = (sender.clone(), path.clone());
        main.connect_clicked(move |_| s.input(ShelfMenuWidgetInput::Open(p.clone())));
    }
    if present {
        dnd::attach_drag_source(&main, vec![path.clone()], &icon);
    }
    row.append(&main);

    #[derive(Clone, Copy)]
    enum Action {
        Show,
        Copy,
        Remove,
    }
    for (icon_name, label, action, enabled) in [
        (
            "folder-open-symbolic",
            "Show in folder",
            Action::Show,
            present,
        ),
        ("edit-copy-symbolic", "Copy path", Action::Copy, true),
        (
            "window-close-symbolic",
            "Remove from the shelf",
            Action::Remove,
            true,
        ),
    ] {
        let b = gtk::Button::from_icon_name(icon_name);
        mshell_common::a11y::name(&b, label);
        b.set_css_classes(&["ok-button-flat", "shelf-row-action"]);
        b.set_valign(gtk::Align::Center);
        b.set_sensitive(enabled);
        let (s, p) = (sender.clone(), path.clone());
        b.connect_clicked(move |_| {
            s.input(match action {
                Action::Show => ShelfMenuWidgetInput::ShowInFolder(p.clone()),
                Action::Copy => ShelfMenuWidgetInput::CopyPath(p.clone()),
                Action::Remove => ShelfMenuWidgetInput::Remove(p.clone()),
            });
        });
        row.append(&b);
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(p: &str) -> ShelfItem {
        ShelfItem {
            path: PathBuf::from(p),
            added: 0,
        }
    }

    #[test]
    fn the_subtitle_counts_files_and_missing_ones() {
        assert_eq!(summary(&[], 0), "Empty");
        assert_eq!(summary(&[item("/a")], 0), "1 file");
        assert_eq!(summary(&[item("/a"), item("/b")], 1), "2 files · 1 missing");
    }

    #[test]
    fn only_present_files_are_offered() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "x").unwrap();
        let items = [
            ShelfItem {
                path: f.clone(),
                added: 0,
            },
            item("/nonexistent/margo-shelf-test"),
        ];
        assert_eq!(existing(&items), [f]);
    }
}
