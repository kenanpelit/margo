//! Settings → Monitors. A list editor over margo's `monitorrule = …` lines in
//! `config.conf`. Existing rules are shown verbatim with a Remove button; the
//! Add form builds a rule from name + mode + position + scale + VRR.

use crate::compositor_conf::{read_block, write_block};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum MonitorsInput {
    SetName(String),
    SetWidth(f64),
    SetHeight(f64),
    SetRefresh(f64),
    SetX(f64),
    SetY(f64),
    SetScale(f64),
    SetVrr(bool),
    Add,
    Remove(usize),
}

#[derive(Debug)]
pub(crate) enum MonitorsOutput {}
#[derive(Debug)]
pub(crate) enum MonitorsCommandOutput {}
pub(crate) struct MonitorsInit {}

pub(crate) struct MonitorsModel {
    rules: Vec<String>,
    list_box: gtk::ListBox,
    f_name: String,
    f_width: i32,
    f_height: i32,
    f_refresh: i32,
    f_x: i32,
    f_y: i32,
    f_scale: f64,
    f_vrr: bool,
}

fn rebuild_list(model: &MonitorsModel, sender: &ComponentSender<MonitorsModel>) {
    while let Some(child) = model.list_box.first_child() {
        model.list_box.remove(&child);
    }
    if model.rules.is_empty() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let lbl = gtk::Label::new(Some(
            "No monitor rules yet — outputs use auto-detected modes.",
        ));
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
        lbl.set_selectable(true);
        lbl.add_css_class("label-medium");
        hbox.append(&lbl);
        let btn = gtk::Button::from_icon_name("user-trash-symbolic");
        btn.add_css_class("flat");
        btn.set_valign(gtk::Align::Center);
        let s = sender.clone();
        btn.connect_clicked(move |_| s.input(MonitorsInput::Remove(i)));
        hbox.append(&btn);
        row.set_child(Some(&hbox));
        model.list_box.append(&row);
    }
}

fn adj(value: f64, lo: f64, hi: f64, step: f64) -> gtk::Adjustment {
    gtk::Adjustment::new(value, lo, hi, step, step * 4.0, 0.0)
}

#[relm4::component(pub)]
impl Component for MonitorsModel {
    type CommandOutput = MonitorsCommandOutput;
    type Input = MonitorsInput;
    type Output = MonitorsOutput;
    type Init = MonitorsInit;

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
                        set_icon_name: Some("video-display-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Monitors", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Per-output mode, position, scale and VRR. Connector names come from `mctl outputs`. Applied live via mctl reload.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Add a monitor rule", set_halign: gtk::Align::Start },

                #[template] Row {
                    #[template_child] title { set_label: "Connector name" },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,
                        set_placeholder_text: Some("DP-1 / eDP-1 / HDMI-A-1"),
                        connect_changed[sender] => move |e| sender.input(MonitorsInput::SetName(e.text().to_string())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Width (px, 0 = auto)" },
                    gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(0.0, 0.0, 16384.0, 10.0),
                        connect_value_changed[sender] => move |s| sender.input(MonitorsInput::SetWidth(s.value())) } },
                #[template] Row {
                    #[template_child] title { set_label: "Height (px, 0 = auto)" },
                    gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(0.0, 0.0, 16384.0, 10.0),
                        connect_value_changed[sender] => move |s| sender.input(MonitorsInput::SetHeight(s.value())) } },
                #[template] Row {
                    #[template_child] title { set_label: "Refresh rate (Hz, 0 = auto)" },
                    gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(0.0, 0.0, 360.0, 1.0),
                        connect_value_changed[sender] => move |s| sender.input(MonitorsInput::SetRefresh(s.value())) } },
                #[template] Row {
                    #[template_child] title { set_label: "Position X" },
                    gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(0.0, 0.0, 32768.0, 10.0),
                        connect_value_changed[sender] => move |s| sender.input(MonitorsInput::SetX(s.value())) } },
                #[template] Row {
                    #[template_child] title { set_label: "Position Y" },
                    gtk::SpinButton { set_valign: gtk::Align::Center, set_adjustment: &adj(0.0, 0.0, 32768.0, 10.0),
                        connect_value_changed[sender] => move |s| sender.input(MonitorsInput::SetY(s.value())) } },
                #[template] Row {
                    #[template_child] title { set_label: "Scale" },
                    gtk::SpinButton { set_valign: gtk::Align::Center, set_digits: 2, set_adjustment: &adj(1.0, 0.5, 4.0, 0.05),
                        connect_value_changed[sender] => move |s| sender.input(MonitorsInput::SetScale(s.value())) } },
                #[template] Row {
                    #[template_child] title { set_label: "Variable refresh rate (VRR)" },
                    gtk::Switch { set_valign: gtk::Align::Center,
                        connect_active_notify[sender] => move |s| sender.input(MonitorsInput::SetVrr(s.is_active())) } },
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "suggested-action",
                    set_label: "Add monitor rule",
                    connect_clicked[sender] => move |_| sender.input(MonitorsInput::Add),
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Current monitor rules", set_halign: gtk::Align::Start, set_margin_top: 8 },

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
        let model = MonitorsModel {
            rules: read_block("monitorrule"),
            list_box: gtk::ListBox::new(),
            f_name: String::new(),
            f_width: 0,
            f_height: 0,
            f_refresh: 0,
            f_x: 0,
            f_y: 0,
            f_scale: 1.0,
            f_vrr: false,
        };
        let list_box = model.list_box.clone();
        let widgets = view_output!();
        rebuild_list(&model, &sender);
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            MonitorsInput::SetName(v) => self.f_name = v,
            MonitorsInput::SetWidth(v) => self.f_width = v as i32,
            MonitorsInput::SetHeight(v) => self.f_height = v as i32,
            MonitorsInput::SetRefresh(v) => self.f_refresh = v as i32,
            MonitorsInput::SetX(v) => self.f_x = v as i32,
            MonitorsInput::SetY(v) => self.f_y = v as i32,
            MonitorsInput::SetScale(v) => self.f_scale = v,
            MonitorsInput::SetVrr(v) => self.f_vrr = v,
            MonitorsInput::Add => {
                let name = self.f_name.trim();
                if name.is_empty() {
                    return;
                }
                let mut parts: Vec<String> = vec![format!("name:{name}")];
                if self.f_width > 0 && self.f_height > 0 {
                    parts.push(format!("width:{}", self.f_width));
                    parts.push(format!("height:{}", self.f_height));
                }
                if self.f_refresh > 0 {
                    parts.push(format!("refresh:{}", self.f_refresh));
                }
                parts.push(format!("x:{}", self.f_x));
                parts.push(format!("y:{}", self.f_y));
                parts.push(format!("scale:{:.2}", self.f_scale));
                if self.f_vrr {
                    parts.push("vrr:1".to_string());
                }
                self.rules.push(parts.join(", "));
                write_block("monitorrule", &self.rules);
                rebuild_list(self, &sender);
            }
            MonitorsInput::Remove(i) => {
                if i < self.rules.len() {
                    self.rules.remove(i);
                    write_block("monitorrule", &self.rules);
                    rebuild_list(self, &sender);
                }
            }
        }
    }
}
