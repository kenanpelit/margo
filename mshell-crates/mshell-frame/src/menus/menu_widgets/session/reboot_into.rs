//! The session menu's "Reboot into…" section: the UEFI boot entries
//! (efibootmgr), with the running one and a pending `BootNext` marked.
//! Clicking an entry sets `BootNext` through the root helper (one polkit
//! prompt) and then runs the menu's Reboot; "Clear" drops a pending
//! `BootNext` without rebooting. Shown only with `session.reboot_into`.
//!
//! The section is a header button that reveals the list; the list is read
//! each time it opens (never while the menu is closed).

use mshell_utils::boot_next::{self, BootEntry, BootList, BootTarget, ListError, efiboot};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use std::rc::Rc;

/// What the parent should do with a click inside the section.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RebootIntoAction {
    Toggle,
    Pick(u16),
    Clear,
}

/// The section's widgets and state, owned by the session menu model.
pub(crate) struct RebootInto {
    root: gtk::Box,
    header: gtk::Button,
    chevron: gtk::Image,
    header_meta: gtk::Label,
    revealer: gtk::Revealer,
    list: gtk::Box,
    status: gtk::Label,
    clear: gtk::Button,
    /// Entry buttons currently shown (for the keyboard focus walk).
    rows: Vec<gtk::Button>,
    /// `None` until the first read after opening.
    data: Option<Result<BootList, ListError>>,
    /// A polkit prompt is open / efibootmgr is running.
    busy: bool,
    /// Where clicks go (the session menu's input).
    on: Rc<dyn Fn(RebootIntoAction)>,
}

impl RebootInto {
    pub(crate) fn new(on: impl Fn(RebootIntoAction) + 'static) -> Self {
        let on: Rc<dyn Fn(RebootIntoAction)> = Rc::new(on);
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("session-reboot-into");
        root.set_visible(false);

        let header = gtk::Button::new();
        header.set_css_classes(&["ok-button-surface", "session-reboot-into-header"]);
        let inner = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let icon = gtk::Image::from_icon_name("system-reboot-symbolic");
        inner.append(&icon);
        let title = gtk::Label::new(Some("Reboot into…"));
        title.add_css_class("session-reboot-into-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        inner.append(&title);
        let header_meta = gtk::Label::new(None);
        header_meta.add_css_class("session-reboot-into-meta");
        header_meta.set_ellipsize(gtk::pango::EllipsizeMode::End);
        inner.append(&header_meta);
        let chevron = gtk::Image::from_icon_name("go-next-symbolic");
        inner.append(&chevron);
        header.set_child(Some(&inner));
        header.set_tooltip_text(Some(
            "Pick the boot entry the next boot starts (this boot only), then reboot",
        ));
        {
            let on = on.clone();
            header.connect_clicked(move |_| on(RebootIntoAction::Toggle));
        }
        root.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.append(&list);
        let status = gtk::Label::new(None);
        status.add_css_class("session-status");
        status.set_xalign(0.0);
        status.set_wrap(true);
        status.set_visible(false);
        body.append(&status);
        let clear = gtk::Button::with_label("Clear next boot");
        clear.set_css_classes(&["ok-button-surface", "session-reboot-into-clear"]);
        clear.set_halign(gtk::Align::Start);
        clear.set_tooltip_text(Some(
            "Drop the pending one-shot entry; the next boot follows the boot order",
        ));
        clear.set_visible(false);
        {
            let on = on.clone();
            clear.connect_clicked(move |_| on(RebootIntoAction::Clear));
        }
        body.append(&clear);

        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_child(Some(&body));
        root.append(&revealer);

        Self {
            root,
            header,
            chevron,
            header_meta,
            revealer,
            list,
            status,
            clear,
            rows: Vec::new(),
            data: None,
            busy: false,
            on,
        }
    }

    pub(crate) fn root(&self) -> &gtk::Box {
        &self.root
    }

    pub(crate) fn is_open(&self) -> bool {
        self.revealer.reveals_child()
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    /// The menu opened (or closed): show the section per config, collapsed.
    pub(crate) fn reset(&mut self, enabled: bool) {
        self.root.set_visible(enabled);
        self.revealer.set_reveal_child(false);
        self.chevron.set_icon_name(Some("go-next-symbolic"));
        self.data = None;
        self.busy = false;
        self.header_meta.set_label("");
    }

    /// Expand or collapse; returns whether the caller should (re)read.
    pub(crate) fn toggle(&mut self) -> bool {
        let open = !self.is_open();
        self.revealer.set_reveal_child(open);
        self.chevron.set_icon_name(Some(if open {
            "go-down-symbolic"
        } else {
            "go-next-symbolic"
        }));
        if open {
            self.data = None;
            self.render(&[], false);
        }
        open
    }

    pub(crate) fn loaded(
        &mut self,
        data: Result<BootList, ListError>,
        hidden: &[String],
        all: bool,
    ) {
        self.data = Some(data);
        self.busy = false;
        self.render(hidden, all);
    }

    /// A prompt is open for `target`: freeze the rows and say so.
    pub(crate) fn waiting(&mut self) {
        self.busy = true;
        for r in &self.rows {
            r.set_sensitive(false);
        }
        self.clear.set_sensitive(false);
        self.show_status("Waiting for authorization…", false);
    }

    /// A set/clear failed: re-enable and show why (§17 error).
    pub(crate) fn failed(&mut self, msg: &str) {
        self.busy = false;
        for r in &self.rows {
            r.set_sensitive(true);
        }
        self.clear.set_sensitive(true);
        self.show_status(msg, true);
    }

    /// The header plus, when open, the entry rows and Clear (focus order).
    pub(crate) fn focusables(&self) -> Vec<gtk::Widget> {
        let mut v: Vec<gtk::Widget> = Vec::new();
        if !self.root.is_visible() {
            return v;
        }
        v.push(self.header.clone().into());
        if self.is_open() {
            v.extend(self.rows.iter().map(|b| b.clone().into()));
            if self.clear.is_visible() {
                v.push(self.clear.clone().into());
            }
        }
        v
    }

    fn show_status(&self, text: &str, danger: bool) {
        self.status.set_label(text);
        self.status.set_visible(!text.is_empty());
        if danger {
            self.status.add_css_class("danger");
        } else {
            self.status.remove_css_class("danger");
        }
    }

    fn render(&mut self, hidden: &[String], all: bool) {
        while let Some(child) = self.list.first_child() {
            self.list.remove(&child);
        }
        self.list.set_css_classes(&[]);
        self.rows.clear();
        self.clear.set_visible(false);
        self.clear.set_sensitive(true);
        match &self.data {
            None => {
                self.header_meta.set_label("");
                self.show_status("Reading boot entries…", false);
            }
            Some(Err(ListError::Missing)) => {
                self.show_status(&ListError::Missing.message(), false);
            }
            Some(Err(e)) => {
                self.show_status(
                    &format!("Couldn't read the boot entries: {}", e.message()),
                    true,
                );
            }
            Some(Ok(list)) => {
                let offered = efiboot::offered(list, hidden, all);
                self.header_meta.set_label(&next_meta(list));
                self.clear.set_visible(list.next.is_some());
                if offered.is_empty() {
                    self.show_status("No boot entries to offer", false);
                    return;
                }
                self.show_status("", false);
                self.list.set_css_classes(&[
                    "ok-boxed-list",
                    "compact",
                    "session-reboot-into-list",
                ]);
                for e in offered {
                    let (row, main) = entry_row(list, e);
                    let (on, num) = (self.on.clone(), e.num);
                    main.connect_clicked(move |_| on(RebootIntoAction::Pick(num)));
                    self.list.append(&row);
                    self.rows.push(main);
                }
            }
        }
    }
}

/// `Next boot: cachyos-nvme0` while a `BootNext` is pending.
fn next_meta(list: &BootList) -> String {
    match list.next {
        Some(n) => match list.entry(n) {
            Some(e) => format!("Next boot: {}", e.label),
            None => format!("Next boot: Boot{n:04X}"),
        },
        None => String::new(),
    }
}

/// The subtitle under an entry: `Boot0003 · this boot · next boot`.
pub(crate) fn entry_subtitle(list: &BootList, e: &BootEntry) -> String {
    let mut parts = vec![e.id()];
    if list.current == Some(e.num) {
        parts.push("this boot".into());
    }
    if list.next == Some(e.num) {
        parts.push("next boot".into());
    }
    parts.join(" · ")
}

/// One entry: a boxed-list row whose flat main button carries the click
/// (the GitHub menu's row shape); the pending `BootNext` is selected.
fn entry_row(list: &BootList, e: &BootEntry) -> (gtk::Box, gtk::Button) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("session-reboot-into-row");
    let main = gtk::Button::new();
    main.set_css_classes(&["session-reboot-into-row-main"]);
    main.set_hexpand(true);
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    col.set_hexpand(true);
    col.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(&e.label));
    t.add_css_class("session-reboot-into-row-title");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    col.append(&t);
    let s = gtk::Label::new(Some(&entry_subtitle(list, e)));
    s.add_css_class("session-reboot-into-row-subtitle");
    s.set_xalign(0.0);
    col.append(&s);
    inner.append(&col);
    if list.next == Some(e.num) {
        let check = gtk::Image::from_icon_name("object-select-symbolic");
        check.add_css_class("session-reboot-into-check");
        check.set_valign(gtk::Align::Center);
        inner.append(&check);
        row.add_css_class("selected");
    }
    main.set_child(Some(&inner));
    main.set_tooltip_text(Some(&format!("Reboot into {} (this boot only)", e.label)));
    row.append(&main);
    (row, main)
}

/// Read the listing (for `oneshot_command`).
pub(crate) async fn read() -> Result<BootList, ListError> {
    boot_next::read_list().await
}

/// Set / clear `BootNext` (for `oneshot_command`).
pub(crate) async fn apply(target: BootTarget) -> Result<(), String> {
    boot_next::set(target).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subtitles_and_meta_mark_current_and_next() {
        let l = efiboot::parse(
            "BootCurrent: 0001\nBootNext: 0003\nBoot0001* cachyos-nvme1\tHD()\nBoot0003* cachyos-nvme0\tHD()\n",
        );
        assert_eq!(entry_subtitle(&l, &l.entries[0]), "Boot0001 · this boot");
        assert_eq!(entry_subtitle(&l, &l.entries[1]), "Boot0003 · next boot");
        assert_eq!(next_meta(&l), "Next boot: cachyos-nvme0");
        let gone = efiboot::parse("BootNext: 0009\nBoot0001* a\n");
        assert_eq!(next_meta(&gone), "Next boot: Boot0009");
        assert_eq!(next_meta(&efiboot::parse("Boot0001* a\n")), "");
    }
}
