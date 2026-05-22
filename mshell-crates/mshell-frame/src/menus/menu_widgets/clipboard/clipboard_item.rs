use mshell_clipboard::{ClipboardEntry, EntryPreview, clipboard_service};
use relm4::{
    Component, ComponentParts, ComponentSender, RelmWidgetExt,
    gtk::{self, glib, prelude::*},
};

#[derive(Debug, Clone)]
pub(crate) struct ClipboardItemModel {
    entry: ClipboardEntry,
}

#[derive(Debug)]
pub(crate) enum ClipboardItemInput {
    DeleteEntry,
    CopyEntry,
    TogglePin,
}

#[derive(Debug)]
pub(crate) enum ClipboardItemOutput {}

#[derive(Debug)]
pub(crate) enum ClipboardItemCommandOutput {}

#[relm4::component(pub)]
impl Component for ClipboardItemModel {
    type CommandOutput = ClipboardItemCommandOutput;
    type Input = ClipboardItemInput;
    type Output = ClipboardItemOutput;
    type Init = ClipboardEntry;

    view! {
        #[root]
        gtk::Overlay {
            // Action cluster — top-right, grouped so neither button
            // sits over the `#id` title (top-left). Pin first, then
            // trash. `.pinned` flips the pin chrome to the accent so
            // a favourite reads at a glance vs a dim outline star
            // (item is rebuilt on each refresh — init state is
            // always current).
            add_overlay = &gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_halign: gtk::Align::End,
                set_valign: gtk::Align::Start,
                set_margin_all: 8,

                gtk::Button {
                    set_css_classes: if model.entry.pinned {
                        &["ok-button-surface", "clipboard-pin-button", "pinned"]
                    } else {
                        &["ok-button-surface", "clipboard-pin-button"]
                    },
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardItemInput::TogglePin);
                    },

                    #[name="pin_image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some(if model.entry.pinned {
                            "starred-symbolic"
                        } else {
                            "non-starred-symbolic"
                        }),
                    },
                },

                gtk::Button {
                    add_css_class: "ok-button-surface",
                    add_css_class: "clipboard-trash-button",
                    connect_clicked[sender] => move |_| {
                        sender.input(ClipboardItemInput::DeleteEntry);
                    },

                    #[name="image"]
                    gtk::Image {
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some("trash-symbolic"),
                    },
                },
            },

            gtk::Button {
                add_css_class: "clipboard-copy-button",
                connect_clicked[sender] => move |_| {
                    sender.input(ClipboardItemInput::CopyEntry);
                },

                gtk::Box {
                    add_css_class: "clipboard-item",
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,

                    gtk::Label {
                        add_css_class: "clipboard-item-title",
                        set_label: relative_time(&model.entry).as_str(),
                        set_hexpand: true,
                        set_halign: gtk::Align::Start,
                    },

                    // Preview — conditionally built in init
                    #[name = "preview_box"]
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                    },
                },
            },
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = ClipboardItemModel { entry: params };
        let widgets = view_output!();

        // Build the preview content based on entry type.
        match &model.entry.preview {
            EntryPreview::Text(text) => {
                let label = gtk::Label::builder()
                    .label(text)
                    // Fill the row width (xalign keeps the text left) so a
                    // wider clipboard menu reflows the preview instead of
                    // leaving it hugging the left edge at a fixed width.
                    .halign(gtk::Align::Fill)
                    .hexpand(true)
                    .xalign(0.0)
                    .ellipsize(gtk::pango::EllipsizeMode::End)
                    .lines(2)
                    .wrap(true)
                    .wrap_mode(gtk::pango::WrapMode::WordChar)
                    .build();
                label.add_css_class("label-medium-bold");
                widgets.preview_box.append(&label);
            }
            EntryPreview::Image {
                rgba,
                width,
                height,
            } => {
                let bytes = glib::Bytes::from(rgba);
                let texture = gtk::gdk::MemoryTexture::new(
                    *width as i32,
                    *height as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &bytes,
                    (*width * 4) as usize,
                );
                let picture = gtk::Picture::for_paintable(&texture);
                picture.set_content_fit(gtk::ContentFit::Cover);
                picture.set_hexpand(true);

                let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
                frame.set_overflow(gtk::Overflow::Hidden);
                frame.set_height_request(200);
                frame.set_hexpand(true);
                frame.append(&picture);
                frame.add_css_class("clipboard-item-image");
                widgets.preview_box.append(&frame);
            }
            EntryPreview::Binary { mime_type, size } => {
                let label = gtk::Label::builder()
                    .label(format!("{mime_type}  ({})", format_size(*size)))
                    .halign(gtk::Align::Start)
                    .build();
                label.add_css_class("label-small-bold");
                widgets.preview_box.append(&label);
            }
        }

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
            ClipboardItemInput::DeleteEntry => {
                clipboard_service().delete_entry(self.entry.id);
            }
            ClipboardItemInput::CopyEntry => {
                clipboard_service().copy_entry(self.entry.id);
            }
            ClipboardItemInput::TogglePin => {
                clipboard_service().toggle_pin(self.entry.id);
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Relative "captured at" label for an entry's title line — "just
/// now", "5m ago", "3h ago", "yesterday", "4d ago". Computed from the
/// entry's unix timestamp vs. the wall clock, so it needs no `time`
/// crate import here.
fn relative_time(entry: &ClipboardEntry) -> String {
    let then = entry.timestamp.unix_timestamp();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(then);
    let diff = (now - then).max(0);
    if diff < 45 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{}m ago", (diff / 60).max(1))
    } else if diff < 86_400 {
        format!("{}h ago", diff / 3600)
    } else if diff < 172_800 {
        "yesterday".to_string()
    } else {
        format!("{}d ago", diff / 86_400)
    }
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}
