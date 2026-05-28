use crate::scoped_effects::EffectScope;
use mshell_config::schema::config::{
    ConfigStoreFields, GeneralStoreFields, NotificationsStoreFields,
};
use reactive_graph::traits::{Get, GetUntracked};
use relm4::gtk::pango;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk, once_cell};
use std::sync::Arc;
use time::format_description::parse;
use time::{OffsetDateTime, UtcOffset};
use wayle_notification::core::notification::Notification;

static TIME_FORMAT_24: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| {
        parse("[hour repr:24 padding:zero]:[minute padding:zero]").unwrap()
    });

static TIME_FORMAT_12: once_cell::sync::Lazy<Vec<time::format_description::FormatItem<'static>>> =
    once_cell::sync::Lazy::new(|| {
        parse("[hour repr:12 padding:zero]:[minute padding:zero] [period case:lower]").unwrap()
    });

/// Pixel size for the body image / album-art thumbnail.
pub const BODY_IMAGE_SIZE: i32 = 72;
/// Pixel size for the per-app icon in the header.
pub const APP_ICON_SIZE: i32 = 16;

#[derive(Debug, Clone)]
pub struct NotificationModel {
    notification: Arc<Notification>,
    time: String,
    _effects: EffectScope,
}

#[derive(Debug)]
pub enum NotificationInput {
    CloseClicked,
    ChangeTimeFormat(bool),
}

#[derive(Debug)]
pub enum NotificationOutput {
    ActionActivated,
}

pub struct NotificationInit {
    pub notification: Arc<Notification>,
}

#[derive(Debug)]
pub enum NotificationCommandOutput {}

#[relm4::component(pub)]
impl Component for NotificationModel {
    type CommandOutput = NotificationCommandOutput;
    type Input = NotificationInput;
    type Output = NotificationOutput;
    type Init = NotificationInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "notification",
            set_orientation: gtk::Orientation::Vertical,
            set_hexpand: true,
            set_spacing: 8,

            #[name = "header"]
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                // The per-app icon is prepended here in init() when the
                // notification carries an `app_icon`.

                gtk::Label {
                    add_css_class: "label-small-bold-variant",
                    set_label: model.notification.app_name.get().unwrap_or("".to_string()).as_str(),
                    set_hexpand: true,
                    set_xalign: 0.0,
                },

                gtk::Label {
                    add_css_class: "label-small",
                    #[watch]
                    set_label: model.time.as_str(),
                },

                #[name = "close_button"]
                gtk::Button {
                    add_css_class: "ok-button-surface",
                    set_margin_start: 4,
                    set_hexpand: false,
                    set_vexpand: false,
                    connect_clicked[sender] => move |_| {
                        sender.input(NotificationInput::CloseClicked);
                    },

                    gtk::Image {
                        set_hexpand: true,
                        set_vexpand: true,
                        set_halign: gtk::Align::Center,
                        set_valign: gtk::Align::Center,
                        set_icon_name: Some("close-symbolic"),
                    },
                },
            },

            // Body row: optional left thumbnail (album art / image hint,
            // prepended in init) + the text column. Clicking this row
            // invokes the default action when the notification has one.
            #[name = "content"]
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 10,

                gtk::Box {
                    set_orientation: gtk::Orientation::Vertical,
                    set_spacing: 4,
                    set_hexpand: true,

                    gtk::Label {
                        add_css_class: "label-medium-bold",
                        set_label: model.notification.summary.get().as_str(),
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_wrap_mode: pango::WrapMode::WordChar,
                        set_width_chars: 20,
                        set_max_width_chars: 44,
                    },

                    #[name = "body_label"]
                    gtk::Label {
                        add_css_class: "label-small",
                        set_xalign: 0.0,
                        set_wrap: true,
                        set_wrap_mode: pango::WrapMode::WordChar,
                        set_width_chars: 20,
                        set_max_width_chars: 44,
                    },
                },
            },

            // Detected 2FA / OTP code → one-click copy (filled in init).
            #[name = "code_container"]
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
            },

            #[name = "actions_container"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 4,
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = mshell_config::config_manager::config_manager().config();

        let format_24_h = base_config
            .clone()
            .general()
            .clock_format_24_h()
            .get_untracked();

        let timestamp = params.notification.timestamp.get();

        let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);

        let odt = OffsetDateTime::from_unix_timestamp(timestamp.timestamp())
            .unwrap()
            .replace_nanosecond(timestamp.timestamp_subsec_nanos())
            .unwrap()
            .to_offset(local_offset);

        let time = if format_24_h {
            odt.format(&TIME_FORMAT_24).unwrap()
        } else {
            odt.format(&TIME_FORMAT_12).unwrap()
        };

        // Per-notification button visibility (read once; applies to
        // notifications created after a config change).
        let show_close_button = base_config
            .clone()
            .notifications()
            .show_close_button()
            .get_untracked();
        let show_action_buttons = base_config
            .clone()
            .notifications()
            .show_action_buttons()
            .get_untracked();

        let mut effects = EffectScope::new();

        let sender_clone = sender.clone();
        effects.push(move |_| {
            let config = base_config.clone();
            let format_24_h = config.general().clock_format_24_h().get();
            sender_clone.input(NotificationInput::ChangeTimeFormat(format_24_h));
        });

        let model = NotificationModel {
            notification: params.notification,
            time,
            _effects: effects,
        };

        let widgets = view_output!();

        // Header close (✕) button — configurable (notifications.show_close_button).
        widgets.close_button.set_visible(show_close_button);

        // ── Body text: render notification-spec markup when it's valid,
        // otherwise fall back to the literal string. Apps that don't
        // escape `&`/`<` would crash a blind set_markup, so validate
        // through pango first.
        let body = model.notification.body.get().unwrap_or_default();
        apply_body_text(&widgets.body_label, &body);

        // ── Per-app icon in the header (icon name or absolute path).
        if let Some(icon) = model.notification.app_icon.get() {
            let icon = icon.trim();
            if !icon.is_empty() {
                let img = build_image(icon, APP_ICON_SIZE);
                img.add_css_class("notification-app-icon");
                widgets.header.prepend(&img);
            }
        }

        // ── Body image / album art thumbnail. wayle resolves image-data
        // hints to a cached file path, but the `image-path` hint may
        // *also* be a bare freedesktop icon name (the spec allows both,
        // and libnotify's `notify-send -i <name>` routes the icon there,
        // leaving `app_icon` empty). Go through `build_image` so a name
        // resolves via the icon theme instead of failing `from_file` and
        // rendering a broken-image placeholder.
        if let Some(path) = model.notification.image_path.get() {
            let path = path.trim();
            if !path.is_empty() {
                let img = build_image(path, BODY_IMAGE_SIZE);
                img.set_valign(gtk::Align::Start);
                img.add_css_class("notification-image");
                widgets.content.prepend(&img);
            }
        }

        // ── Body-row gesture: one GestureDrag drives BOTH
        //   • tap → invoke the default action (open the app / chat), and
        //   • horizontal swipe past a threshold → dismiss the toast,
        // with the opacity fading during the drag for feedback. Scoped
        // to `content` so the close button + action buttons stay clear.
        // Unifying into one gesture avoids click-vs-drag conflicts.
        let default_key = model.notification.default_action.get().map(|a| a.id.clone());
        if default_key.is_some() {
            widgets.content.add_css_class("notification-clickable");
        }
        let drag = gtk::GestureDrag::new();
        let root_for_update = root.clone();
        drag.connect_drag_update(move |_, off_x, _| {
            let fade = (off_x.abs() / 320.0).min(0.6);
            root_for_update.set_opacity(1.0 - fade);
        });
        let notification = model.notification.clone();
        let sender_clone = sender.clone();
        let root_for_end = root.clone();
        drag.connect_drag_end(move |_, off_x, off_y| {
            if off_x.abs() > 64.0 && off_x.abs() > off_y.abs() {
                // Swipe → dismiss.
                notification.dismiss();
                let _ = sender_clone.output(NotificationOutput::ActionActivated);
            } else if off_x.abs() < 8.0 && off_y.abs() < 8.0 {
                // Tap → default action.
                root_for_end.set_opacity(1.0);
                if let Some(key) = default_key.clone() {
                    let notification = notification.clone();
                    let sender_clone = sender_clone.clone();
                    tokio::spawn(async move {
                        let _ = notification.invoke(&key).await;
                        let _ = sender_clone.output(NotificationOutput::ActionActivated);
                    });
                }
            } else {
                // Incomplete drag → snap back.
                root_for_end.set_opacity(1.0);
            }
        });
        widgets.content.add_controller(drag);

        // ── 2FA / OTP code detection → one-click copy. Scans summary +
        // body for a 4–8 digit run or a 3-3 grouped code.
        let haystack = format!("{} {}", model.notification.summary.get(), body);
        if let Some(code) = detect_code(&haystack) {
            let btn = gtk::Button::new();
            btn.add_css_class("notification-code-copy");
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let icon = gtk::Image::from_icon_name("edit-copy-symbolic");
            let label = gtk::Label::new(Some(&format!("Copy code  {code}")));
            content.append(&icon);
            content.append(&label);
            btn.set_child(Some(&content));
            let code_for_click = code.clone();
            btn.connect_clicked(move |b| {
                b.clipboard().set_text(&code_for_click);
            });
            widgets.code_container.append(&btn);
        }

        // ── Explicit action buttons. With the action-icons capability,
        // the action id is an icon name rather than a label.
        let action_icons = model.notification.action_icons.get();
        let actions = &model.notification.actions.get();
        if show_action_buttons && !actions.is_empty() {
            for action in actions {
                let btn = if action_icons {
                    let b = gtk::Button::new();
                    b.set_child(Some(&gtk::Image::from_icon_name(&action.id)));
                    b.set_tooltip_text(Some(&action.label));
                    b
                } else {
                    gtk::Button::with_label(&action.label)
                };
                btn.add_css_class("ok-button-primary");

                let notification = model.notification.clone();
                let key = action.id.clone();
                let sender_clone = sender.clone();
                btn.connect_clicked(move |_| {
                    let notification = notification.clone();
                    let key = key.clone();
                    let sender_clone = sender_clone.clone();
                    tokio::spawn(async move {
                        let _ = notification.invoke(&key).await;
                        let _ = sender_clone.output(NotificationOutput::ActionActivated);
                    });
                });

                widgets.actions_container.append(&btn);
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
            NotificationInput::CloseClicked => {
                let notification = self.notification.clone();
                notification.dismiss();
            }
            NotificationInput::ChangeTimeFormat(format_24_h) => {
                let timestamp = self.notification.timestamp.get();

                let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);

                let odt = OffsetDateTime::from_unix_timestamp(timestamp.timestamp())
                    .unwrap()
                    .replace_nanosecond(timestamp.timestamp_subsec_nanos())
                    .unwrap()
                    .to_offset(local_offset);

                if format_24_h {
                    self.time = odt.format(&TIME_FORMAT_24).unwrap();
                } else {
                    self.time = odt.format(&TIME_FORMAT_12).unwrap();
                }
            }
        }

        self.update_view(widgets, sender);
    }
}

/// Format a notification's timestamp as `HH:MM` (24-h) or `hh:mm am/pm`
/// (12-h), in local time. Shared by the popup component and the
/// virtualized history list so both render times identically.
pub fn format_notification_time(notification: &Notification, format_24_h: bool) -> String {
    let timestamp = notification.timestamp.get();
    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    let odt = OffsetDateTime::from_unix_timestamp(timestamp.timestamp())
        .unwrap()
        .replace_nanosecond(timestamp.timestamp_subsec_nanos())
        .unwrap()
        .to_offset(local_offset);
    if format_24_h {
        odt.format(&TIME_FORMAT_24).unwrap()
    } else {
        odt.format(&TIME_FORMAT_12).unwrap()
    }
}

/// Set a label to notification-spec markup when the body parses as
/// valid pango markup, else to the raw text. Apps frequently send
/// unescaped `&`/`<`, which would otherwise be dropped by set_markup.
pub fn apply_body_text(label: &gtk::Label, body: &str) {
    if body.contains('<') && pango::parse_markup(body, '\u{0}').is_ok() {
        label.set_markup(body);
    } else {
        label.set_label(body);
    }
}

/// Build a header/app image from either an icon name or a file path.
pub fn build_image(icon: &str, pixel_size: i32) -> gtk::Image {
    let img = if icon.starts_with('/') || icon.starts_with("file://") {
        let path = icon.strip_prefix("file://").unwrap_or(icon);
        gtk::Image::from_file(path)
    } else {
        gtk::Image::from_icon_name(&resolve_icon_name(icon))
    };
    img.set_pixel_size(pixel_size);
    img
}

/// mshell ships a symbolic-only icon theme (MargoMaterial → Adwaita),
/// but most apps send `notify-send -i` *plain* freedesktop names
/// (`audio-volume-high`, `dialog-error`, …) which then resolve to
/// nothing and render a blank header. When the plain name isn't in the
/// active theme but its `-symbolic` sibling is, use that instead.
fn resolve_icon_name(name: &str) -> String {
    if name.ends_with("-symbolic") {
        return name.to_string();
    }
    if let Some(display) = gtk::gdk::Display::default() {
        let theme = gtk::IconTheme::for_display(&display);
        if !theme.has_icon(name) {
            let symbolic = format!("{name}-symbolic");
            if theme.has_icon(&symbolic) {
                return symbolic;
            }
        }
    }
    name.to_string()
}

/// Detect a 2FA / OTP code in notification text: a standalone run of
/// 4–8 digits, or a `123-456` / `123 456` grouped code. Returns the
/// first match (digits only, separators stripped). Hand-rolled to
/// avoid a regex dependency.
pub fn detect_code(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut i = 0;
    while i < n {
        if !bytes[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        // Must start on a token boundary (not mid-number/word).
        if i > 0 && (bytes[i - 1].is_ascii_digit() || bytes[i - 1].is_ascii_alphabetic()) {
            while i < n && bytes[i].is_ascii_digit() {
                i += 1;
            }
            continue;
        }
        let start = i;
        while i < n && bytes[i].is_ascii_digit() {
            i += 1;
        }
        let first_len = i - start;

        // `123-456` / `123 456` grouped form.
        if first_len == 3
            && i + 4 <= n
            && (bytes[i] == b'-' || bytes[i] == b' ')
            && bytes[i + 1..i + 4].iter().all(|b| b.is_ascii_digit())
            && (i + 4 == n || !bytes[i + 4].is_ascii_digit())
        {
            let mut code = String::with_capacity(6);
            code.push_str(&text[start..start + 3]);
            code.push_str(&text[i + 1..i + 4]);
            return Some(code);
        }

        // Plain 4–8 digit run on a token boundary.
        if (4..=8).contains(&first_len)
            && (i == n || !bytes[i].is_ascii_alphabetic())
        {
            return Some(text[start..i].to_string());
        }
    }
    None
}

#[cfg(test)]
mod detect_code_tests {
    use super::detect_code;

    #[test]
    fn plain_codes_4_to_8_digits() {
        assert_eq!(detect_code("Your code is 482913").as_deref(), Some("482913"));
        assert_eq!(detect_code("OTP: 1234").as_deref(), Some("1234"));
        assert_eq!(detect_code("PIN 12345678 now").as_deref(), Some("12345678"));
    }

    #[test]
    fn grouped_3_3_form_strips_separator() {
        assert_eq!(detect_code("Code 123 456").as_deref(), Some("123456"));
        assert_eq!(detect_code("123-456").as_deref(), Some("123456"));
        assert_eq!(detect_code("G-839201").as_deref(), Some("839201"));
    }

    #[test]
    fn rejects_runs_outside_the_length_window() {
        // 3 digits alone, not grouped → no.
        assert_eq!(detect_code("Only 123 here"), None);
        // 9+ digits → no.
        assert_eq!(detect_code("id 123456789"), None);
        // Two short groups that aren't a 3-3 code.
        assert_eq!(detect_code("12 34 56"), None);
    }

    #[test]
    fn requires_a_token_boundary() {
        // Mid-word digits (preceded by a letter) are skipped.
        assert_eq!(detect_code("abc1234"), None);
        // ...but a clean boundary after punctuation works.
        assert_eq!(detect_code("ref#4821").as_deref(), Some("4821"));
        // Trailing letter disqualifies the plain run (e.g. a hex-ish token).
        assert_eq!(detect_code("1234abc"), None);
    }

    #[test]
    fn no_digits_is_none() {
        assert_eq!(detect_code(""), None);
        assert_eq!(detect_code("no code in here"), None);
    }

    #[test]
    fn returns_first_match() {
        assert_eq!(detect_code("first 4821 then 9999").as_deref(), Some("4821"));
    }
}
