//! The AI agents menu's Memory page: every CLAUDE.md Claude Code reads —
//! each account's global one, then the project ones of projects with
//! sessions. Click opens a file in the default app (your editor for
//! Markdown).

use crate::agents::memory::MemoryFile;
use crate::agents::sessions;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use std::rc::Rc;

/// `2048` → `2.0 KB`.
pub(crate) fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    }
}

/// Rebuild the page from `files` (`None` = still reading).
pub(crate) fn rebuild_memory(
    container: &gtk::Box,
    files: Option<&[MemoryFile]>,
    on_open: Rc<dyn Fn(String)>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let Some(files) = files else {
        container.append(&small("Loading…"));
        return;
    };
    if files.is_empty() {
        container.append(&small(
            "No CLAUDE.md yet — Claude Code's /init writes one for a project",
        ));
        return;
    }
    for (title, global) in [("Global", true), ("Projects", false)] {
        let group: Vec<&MemoryFile> = files.iter().filter(|f| f.global == global).collect();
        if group.is_empty() {
            continue;
        }
        let l = gtk::Label::new(Some(title));
        l.add_css_class("section-label-menu-widget");
        l.set_halign(gtk::Align::Start);
        container.append(&l);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.set_css_classes(&["ok-boxed-list", "compact", "agents-session-list"]);
        for f in group {
            list.append(&row(f, on_open.clone()));
        }
        container.append(&list);
    }
}

fn small(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(&["label-small", "dim-label"]);
    l.set_halign(gtk::Align::Start);
    l.set_wrap(true);
    l
}

fn row(f: &MemoryFile, on_open: Rc<dyn Fn(String)>) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_css_classes(&["agents-session-row"]);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let block = gtk::Box::new(gtk::Orientation::Vertical, 2);
    block.set_hexpand(true);
    let path = f.path.to_string_lossy().into_owned();
    let t = gtk::Label::new(Some(&sessions::tilde(&path)));
    t.set_css_classes(&["label-medium"]);
    t.set_halign(gtk::Align::Start);
    t.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    block.append(&t);
    let sub = if f.global {
        format!("{} · {}", f.label, size_text(f.bytes))
    } else {
        size_text(f.bytes)
    };
    let s = gtk::Label::new(Some(&sub));
    s.set_css_classes(&["label-small", "dim-label"]);
    s.set_halign(gtk::Align::Start);
    block.append(&s);
    row.append(&block);
    let icon = gtk::Image::from_icon_name("document-edit-symbolic");
    icon.set_valign(gtk::Align::Center);
    icon.add_css_class("agents-session-go");
    row.append(&icon);
    button.set_child(Some(&row));
    button.set_tooltip_text(Some("Open in your editor"));
    button.connect_clicked(move |_| on_open(path.clone()));
    button
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size_text(228), "228 B");
        assert_eq!(size_text(2048), "2.0 KB");
    }
}
