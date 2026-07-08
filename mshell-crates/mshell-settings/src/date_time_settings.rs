//! Settings → Date & Time.
//!
//! Wraps `timedatectl`: automatic time (NTP), timezone, and the shell's
//! 24-hour clock toggle. The NTP / timezone writes go through
//! `timedatectl`, which authenticates via polkit (org.freedesktop.timedate1)
//! — margo's integrated polkit agent prompts for it. The 24-hour toggle is
//! the shell's own config and applies live.

use crate::row::Row;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GeneralStoreFields};
use reactive_graph::prelude::GetUntracked;
use relm4::gtk::glib::SignalHandlerId;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) struct DateTimeSettingsModel {
    ntp: bool,
    clock_24h: bool,
    timezone: String,
    timezones: gtk::StringList,
    tz_index: u32,
}

#[derive(Debug)]
pub(crate) enum DateTimeSettingsInput {
    SetNtp(bool),
    SetTimezone(u32),
    SetClock24h(bool),
    /// The `timedatectl` reads finished off-thread (see `init`): NTP state,
    /// current timezone, and the full timezone list for the dropdown.
    Loaded {
        ntp: bool,
        timezone: String,
        timezones: Vec<String>,
    },
}

#[derive(Debug)]
pub(crate) enum DateTimeSettingsOutput {}

pub(crate) struct DateTimeSettingsInit {}

#[derive(Debug)]
pub(crate) enum DateTimeSettingsCommandOutput {}

#[relm4::component(pub)]
impl Component for DateTimeSettingsModel {
    type CommandOutput = DateTimeSettingsCommandOutput;
    type Input = DateTimeSettingsInput;
    type Output = DateTimeSettingsOutput;
    type Init = DateTimeSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Box {
                    add_css_class: "settings-hero",
                    set_orientation: gtk::Orientation::Horizontal,
                    set_halign: gtk::Align::Start,
                    set_spacing: 16,
                    gtk::Image {
                        add_css_class: "settings-hero-icon",
                        set_icon_name: Some("preferences-system-time-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label {
                            add_css_class: "settings-hero-title",
                            set_label: "Date & Time",
                            set_halign: gtk::Align::Start,
                        },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Time sync, timezone, and the clock format. System changes prompt for authentication.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Clock",
                    set_halign: gtk::Align::Start,
                },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Automatic time (NTP)" },
                        #[template_child] desc { set_label: "Sync the clock from the network. Turn off to set the time manually (timedatectl)." },
                        #[name = "ntp_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(ntp_handler)]
                            set_active: model.ntp,
                            connect_active_notify[sender] => move |s| {
                                sender.input(DateTimeSettingsInput::SetNtp(s.is_active()));
                            } @ntp_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "Time zone" },
                        #[template_child] desc { set_label: "Type to search. Applied via timedatectl." },
                        #[name = "tz_dd"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            set_width_request: 260,
                            set_enable_search: true,
                            set_model: Some(&model.timezones),
                            #[watch]
                            #[block_signal(tz_handler)]
                            set_selected: model.tz_index,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(DateTimeSettingsInput::SetTimezone(d.selected()));
                            } @tz_handler,
                        },
                    },

                    #[template]
                    Row {
                        #[template_child] title { set_label: "24-hour clock" },
                        #[template_child] desc { set_label: "Show the shell clock in 24-hour format." },
                        #[name = "clock_switch"]
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[block_signal(clock_handler)]
                            set_active: model.clock_24h,
                            connect_active_notify[sender] => move |s| {
                                sender.input(DateTimeSettingsInput::SetClock24h(s.is_active()));
                            } @clock_handler,
                        },
                    },
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // `timedatectl show` + `list-timezones` are subprocesses; Settings
        // pages are built eagerly at login, so running them here would block
        // the GTK main thread at startup. Seed neutral defaults and load the
        // real values off-thread, splicing them in via `Loaded`. The 24-hour
        // toggle is the shell's own in-memory config, so it stays synchronous.
        let model = DateTimeSettingsModel {
            ntp: true,
            clock_24h: config_manager()
                .config()
                .general()
                .clock_format_24_h()
                .get_untracked(),
            timezone: "UTC".to_string(),
            timezones: gtk::StringList::new(&[]),
            tz_index: 0,
        };
        {
            let sender = sender.clone();
            std::thread::spawn(move || {
                let (ntp, timezone) = read_timedatectl();
                let timezones = list_timezones();
                sender.input(DateTimeSettingsInput::Loaded {
                    ntp,
                    timezone,
                    timezones,
                });
            });
        }
        let widgets = view_output!();
        let _ = root;
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
            DateTimeSettingsInput::Loaded {
                ntp,
                timezone,
                timezones,
            } => {
                self.ntp = ntp;
                self.tz_index = timezones.iter().position(|z| *z == timezone).unwrap_or(0) as u32;
                self.timezone = timezone;
                // Splice with the write handler blocked so the auto-select
                // notify on model growth doesn't bounce back as a spurious
                // set-timezone (a polkit prompt on page open). The correct
                // selection is re-applied by the #[watch] set_selected, also
                // with the handler blocked.
                block(&widgets.tz_dd, &widgets.tz_handler, || {
                    let refs: Vec<&str> = timezones.iter().map(|s| s.as_str()).collect();
                    self.timezones.splice(0, self.timezones.n_items(), &refs);
                });
            }
            DateTimeSettingsInput::SetNtp(v) => {
                self.ntp = v;
                run_timedatectl(&["set-ntp", if v { "true" } else { "false" }]);
            }
            DateTimeSettingsInput::SetTimezone(idx) => {
                if let Some(tz) = self.timezones.string(idx) {
                    let tz = tz.to_string();
                    self.timezone = tz.clone();
                    self.tz_index = idx;
                    run_timedatectl(&["set-timezone", &tz]);
                }
            }
            DateTimeSettingsInput::SetClock24h(v) => {
                self.clock_24h = v;
                config_manager().update_config(move |c| c.general.clock_format_24_h = v);
            }
        }
        self.update_view(widgets, sender);
    }
}

/// Run `f` with `widget`'s `handler` blocked, so a programmatic property set
/// doesn't bounce back through its `connect_*` closure.
fn block<W: IsA<gtk::glib::Object>>(widget: &W, handler: &SignalHandlerId, f: impl FnOnce()) {
    widget.block_signal(handler);
    f();
    widget.unblock_signal(handler);
}

/// `(ntp_enabled, timezone)` from `timedatectl show`. Falls back to
/// `(true, "UTC")` if timedatectl is unavailable.
fn read_timedatectl() -> (bool, String) {
    let out = std::process::Command::new("timedatectl")
        .args(["show", "-p", "NTP", "-p", "Timezone"])
        .output();
    let (mut ntp, mut tz) = (true, "UTC".to_string());
    if let Ok(out) = out {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(v) = line.strip_prefix("NTP=") {
                ntp = v.trim() == "yes";
            } else if let Some(v) = line.strip_prefix("Timezone=")
                && !v.trim().is_empty()
            {
                tz = v.trim().to_string();
            }
        }
    }
    (ntp, tz)
}

/// All timezones from `timedatectl list-timezones`; falls back to a small
/// built-in set so the dropdown is never empty.
fn list_timezones() -> Vec<String> {
    let out = std::process::Command::new("timedatectl")
        .arg("list-timezones")
        .output();
    if let Ok(out) = out
        && out.status.success()
    {
        let zones: Vec<String> = String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !zones.is_empty() {
            return zones;
        }
    }
    [
        "UTC",
        "Europe/Istanbul",
        "Europe/London",
        "America/New_York",
        "Asia/Tokyo",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// Run `timedatectl <args>` (polkit-authenticated), reaping asynchronously.
fn run_timedatectl(args: &[&str]) {
    match std::process::Command::new("timedatectl").args(args).spawn() {
        Ok(mut child) => {
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => tracing::warn!(error = %e, ?args, "date-time: timedatectl failed to spawn"),
    }
}
