//! GTK4 drag-and-drop glue shared by the Shelf pill and menu: a drop
//! target that takes files from any app, and the content a drag out (or
//! *Copy all*) offers to other apps.

use crate::shelf::{self, ShelfCommand};
use relm4::gtk;
use relm4::gtk::gdk;
use relm4::gtk::gio;
use relm4::gtk::glib;
use relm4::gtk::prelude::{Cast, FileExt, StaticType, ToValue, WidgetExt};
use std::path::PathBuf;

/// Accept files dropped on `widget` and put them on the shelf.
///
/// Only COPY and LINK are offered: a MOVE drop would let the source app
/// delete its file once we "took" it, and the shelf only keeps a
/// reference. (GTK turns a source's `text/uri-list` into a
/// [`gdk::FileList`] for us.) Files without a local path (a browser's
/// remote image) are skipped.
pub(crate) fn attach_drop_target(widget: &impl gtk::prelude::IsA<gtk::Widget>) {
    let target = gtk::DropTarget::new(
        gdk::FileList::static_type(),
        gdk::DragAction::COPY | gdk::DragAction::LINK,
    );
    target.set_preload(false);
    target.connect_drop(|_, value, _, _| {
        let Ok(list) = value.get::<gdk::FileList>() else {
            return false;
        };
        let paths: Vec<PathBuf> = list.files().iter().filter_map(FileExt::path).collect();
        if paths.is_empty() {
            return false;
        }
        shelf::send(ShelfCommand::Add(paths));
        true
    });
    widget.add_controller(target);
}

fn file_list(paths: &[PathBuf]) -> gdk::FileList {
    let files: Vec<gio::File> = paths.iter().map(gio::File::for_path).collect();
    gdk::FileList::from_array(&files)
}

/// What a drag out offers: the files (GTK serialises them as
/// `text/uri-list` and the portal file-transfer format, which browsers,
/// chat apps and file managers take).
pub(crate) fn drag_content(paths: &[PathBuf]) -> gdk::ContentProvider {
    gdk::ContentProvider::for_value(&file_list(paths).to_value())
}

/// What *Copy all* puts on the clipboard: the files, GNOME's copied-files
/// format (paste in a file manager copies them) and the paths as text.
pub(crate) fn clipboard_content(paths: &[PathBuf]) -> gdk::ContentProvider {
    let text = paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    gdk::ContentProvider::new_union(&[
        drag_content(paths),
        gdk::ContentProvider::for_bytes(
            "x-special/gnome-copied-files",
            &glib::Bytes::from_owned(shelf::gnome_copied_files(paths).into_bytes()),
        ),
        gdk::ContentProvider::for_bytes(
            "text/uri-list",
            &glib::Bytes::from_owned(shelf::uri_list(paths).into_bytes()),
        ),
        gdk::ContentProvider::for_value(&text.to_value()),
    ])
}

/// Make `widget` a drag source for `paths` (a copy, never a move), with
/// `icon_from` as the drag image.
pub(crate) fn attach_drag_source(
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
    paths: Vec<PathBuf>,
    icon_from: &impl gtk::prelude::IsA<gtk::Widget>,
) {
    let paths = std::rc::Rc::new(std::cell::RefCell::new(paths));
    attach_drag_source_with(widget, paths, icon_from);
}

/// [`attach_drag_source`] whose paths are read when the drag starts
/// (an empty list refuses the drag).
pub(crate) fn attach_drag_source_with(
    widget: &impl gtk::prelude::IsA<gtk::Widget>,
    paths: std::rc::Rc<std::cell::RefCell<Vec<PathBuf>>>,
    icon_from: &impl gtk::prelude::IsA<gtk::Widget>,
) {
    let source = gtk::DragSource::new();
    source.set_actions(gdk::DragAction::COPY);
    source.connect_prepare(move |_, _, _| {
        let paths = paths.borrow();
        (!paths.is_empty()).then(|| drag_content(&paths))
    });
    let icon = icon_from.clone().upcast::<gtk::Widget>();
    source.connect_drag_begin(move |src, _| {
        let paintable = gtk::WidgetPaintable::new(Some(&icon));
        src.set_icon(Some(&paintable), 0, 0);
    });
    widget.add_controller(source);
}
