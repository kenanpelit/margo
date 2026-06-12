//! Settings → Window Rules. A list editor over margo's `windowrule = …` lines
//! in `config.conf` (comma-separated `key:value` pairs). Existing rules are
//! shown verbatim with a Remove button; the Add form builds a new rule from
//! the common fields (match by app-id/title regex + a few actions). Exotic
//! fields on existing rules are preserved (rules are stored as raw payloads).

use crate::compositor_conf::{read_block, write_block};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum WindowRulesInput {
    SetAppid(String),
    SetTitle(String),
    SetMonitor(String),
    SetTags(u32),
    SetFloating(bool),
    SetFullscreen(bool),
    SetWidth(f64),
    SetHeight(f64),
    Add,
    Remove(usize),
}

#[derive(Debug)]
pub(crate) enum WindowRulesOutput {}
#[derive(Debug)]
pub(crate) enum WindowRulesCommandOutput {}
pub(crate) struct WindowRulesInit {}

pub(crate) struct WindowRulesModel {
    rules: Vec<String>,
    list_box: gtk::ListBox,
    tags_model: gtk::StringList,
    f_appid: String,
    f_title: String,
    f_monitor: String,
    f_tags: u32,
    f_floating: bool,
    f_fullscreen: bool,
    f_width: i32,
    f_height: i32,
}

fn rebuild_list(model: &WindowRulesModel, sender: &ComponentSender<WindowRulesModel>) {
    while let Some(child) = model.list_box.first_child() {
        model.list_box.remove(&child);
    }
    if model.rules.is_empty() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let lbl = gtk::Label::new(Some("No window rules yet."));
        lbl.add_css_class("label-small");
        lbl.set_halign(gtk::Align::Start);
        lbl.set_margin_top(8);
        lbl.set_margin_bottom(8);
        lbl.set_margin_start(8);
        row.set_child(Some(&lbl));
        model.list_box.append(&row);
        return;
    }
    for (i, payload) in model.rules.iter().enumerate() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        hbox.set_margin_top(6);
        hbox.set_margin_bottom(6);
        hbox.set_margin_start(8);
        hbox.set_margin_end(8);
        let lbl = gtk::Label::new(Some(payload));
        lbl.set_halign(gtk::Align::Start);
        lbl.set_hexpand(true);
        lbl.set_xalign(0.0);
        lbl.set_wrap(true);
        // A rule payload is mostly one long, space-free token (regex / app-id),
        // so plain word-wrap can't break it — its minimum width becomes the
        // whole token and forces the Settings panel wider than its configured
        // size on this page. Allow breaking mid-token and report the wrapped
        // (small) width as the natural one so the page fits the panel instead.
        lbl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        lbl.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
        lbl.set_selectable(true);
        lbl.add_css_class("label-medium");
        hbox.append(&lbl);
        let btn = gtk::Button::from_icon_name("user-trash-symbolic");
        btn.add_css_class("flat");
        btn.set_valign(gtk::Align::Center);
        let s = sender.clone();
        btn.connect_clicked(move |_| s.input(WindowRulesInput::Remove(i)));
        hbox.append(&btn);
        row.set_child(Some(&hbox));
        model.list_box.append(&row);
    }
}

#[relm4::component(pub)]
impl Component for WindowRulesModel {
    type CommandOutput = WindowRulesCommandOutput;
    type Input = WindowRulesInput;
    type Output = WindowRulesOutput;
    type Init = WindowRulesInit;

    view! {
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
                        set_icon_name: Some("window-new-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Window Rules", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Match windows by app-id / title (PCRE2 regex) and place or style them. Applied live via mctl reload.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Add a rule", set_halign: gtk::Align::Start },

                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                #[template] Row {
                    #[template_child] title { set_label: "Match app-id (regex)" },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,
                        set_placeholder_text: Some("^(firefox|kitty)$"),
                        connect_changed[sender] => move |e| sender.input(WindowRulesInput::SetAppid(e.text().to_string())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Match title (regex)" },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,
                        set_placeholder_text: Some("optional"),
                        connect_changed[sender] => move |e| sender.input(WindowRulesInput::SetTitle(e.text().to_string())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Pin to tag" },
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.tags_model),
                        connect_selected_notify[sender] => move |d| sender.input(WindowRulesInput::SetTags(d.selected())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Floating" },
                    gtk::Switch { set_valign: gtk::Align::Center,
                        connect_active_notify[sender] => move |s| sender.input(WindowRulesInput::SetFloating(s.is_active())) } },
                #[template] Row {
                    #[template_child] title { set_label: "Fullscreen" },
                    gtk::Switch { set_valign: gtk::Align::Center,
                        connect_active_notify[sender] => move |s| sender.input(WindowRulesInput::SetFullscreen(s.is_active())) } },
                #[template] Row {
                    #[template_child] title { set_label: "Floating width (px, 0 = unset)" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &gtk::Adjustment::new(0.0, 0.0, 9999.0, 10.0, 100.0, 0.0),
                        connect_value_changed[sender] => move |s| sender.input(WindowRulesInput::SetWidth(s.value())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Floating height (px, 0 = unset)" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &gtk::Adjustment::new(0.0, 0.0, 9999.0, 10.0, 100.0, 0.0),
                        connect_value_changed[sender] => move |s| sender.input(WindowRulesInput::SetHeight(s.value())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Pin to monitor (connector name)" },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_placeholder_text: Some("e.g. DP-1 — optional"),
                        connect_changed[sender] => move |e| sender.input(WindowRulesInput::SetMonitor(e.text().to_string())),
                    },
                },
                },
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "ok-button-primary",
                    set_label: "Add rule",
                    connect_clicked[sender] => move |_| sender.input(WindowRulesInput::Add),
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Current rules", set_halign: gtk::Align::Start, set_margin_top: 8 },

                #[local_ref]
                list_box -> gtk::ListBox {
                    add_css_class: "boxed-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = WindowRulesModel {
            rules: read_block("windowrule"),
            list_box: gtk::ListBox::new(),
            tags_model: gtk::StringList::new(&[
                "None", "1", "2", "3", "4", "5", "6", "7", "8", "9",
            ]),
            f_appid: String::new(),
            f_title: String::new(),
            f_monitor: String::new(),
            f_tags: 0,
            f_floating: false,
            f_fullscreen: false,
            f_width: 0,
            f_height: 0,
        };
        let list_box = model.list_box.clone();
        let widgets = view_output!();
        rebuild_list(&model, &sender);
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            WindowRulesInput::SetAppid(v) => self.f_appid = v,
            WindowRulesInput::SetTitle(v) => self.f_title = v,
            WindowRulesInput::SetMonitor(v) => self.f_monitor = v,
            WindowRulesInput::SetTags(i) => self.f_tags = i,
            WindowRulesInput::SetFloating(v) => self.f_floating = v,
            WindowRulesInput::SetFullscreen(v) => self.f_fullscreen = v,
            WindowRulesInput::SetWidth(v) => self.f_width = v as i32,
            WindowRulesInput::SetHeight(v) => self.f_height = v as i32,
            WindowRulesInput::Add => {
                let appid = self.f_appid.trim();
                let title = self.f_title.trim();
                // A rule needs a matcher; without one it would match nothing.
                if appid.is_empty() && title.is_empty() {
                    return;
                }
                let mut parts: Vec<String> = Vec::new();
                if self.f_tags > 0 {
                    parts.push(format!("tags:{}", self.f_tags));
                }
                if self.f_floating {
                    parts.push("isfloating:1".to_string());
                }
                if self.f_fullscreen {
                    parts.push("isfullscreen:1".to_string());
                }
                if self.f_width > 0 {
                    parts.push(format!("width:{}", self.f_width));
                }
                if self.f_height > 0 {
                    parts.push(format!("height:{}", self.f_height));
                }
                if !self.f_monitor.trim().is_empty() {
                    parts.push(format!("monitor:{}", self.f_monitor.trim()));
                }
                if !appid.is_empty() {
                    parts.push(format!("appid:{appid}"));
                }
                if !title.is_empty() {
                    parts.push(format!("title:{title}"));
                }
                self.rules.push(parts.join(", "));
                write_block("windowrule", &self.rules);
                rebuild_list(self, &sender);
            }
            WindowRulesInput::Remove(i) => {
                if i < self.rules.len() {
                    self.rules.remove(i);
                    write_block("windowrule", &self.rules);
                    rebuild_list(self, &sender);
                }
            }
        }
    }
}
