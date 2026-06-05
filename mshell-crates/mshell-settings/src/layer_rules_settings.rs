//! Settings → Layer Rules. List editor over margo's `layerrule = …` lines —
//! tweak layer-shell surfaces (bar / menus / notifications / osd) by namespace.

use crate::compositor_conf::{read_block, write_block};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum LayerRulesInput {
    SetName(String),
    SetNoAnim(bool),
    SetNoBlur(bool),
    SetNoShadow(bool),
    Add,
    Remove(usize),
}

#[derive(Debug)]
pub(crate) enum LayerRulesOutput {}
#[derive(Debug)]
pub(crate) enum LayerRulesCommandOutput {}
pub(crate) struct LayerRulesInit {}

pub(crate) struct LayerRulesModel {
    rules: Vec<String>,
    list_box: gtk::ListBox,
    f_name: String,
    f_noanim: bool,
    f_noblur: bool,
    f_noshadow: bool,
}

fn rebuild_list(model: &LayerRulesModel, sender: &ComponentSender<LayerRulesModel>) {
    while let Some(child) = model.list_box.first_child() {
        model.list_box.remove(&child);
    }
    if model.rules.is_empty() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let lbl = gtk::Label::new(Some("No layer rules yet."));
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
        btn.connect_clicked(move |_| s.input(LayerRulesInput::Remove(i)));
        hbox.append(&btn);
        row.set_child(Some(&hbox));
        model.list_box.append(&row);
    }
}

#[relm4::component(pub)]
impl Component for LayerRulesModel {
    type CommandOutput = LayerRulesCommandOutput;
    type Input = LayerRulesInput;
    type Output = LayerRulesOutput;
    type Init = LayerRulesInit;

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
                        set_icon_name: Some("view-paged-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Layer Rules", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Disable animation / blur / shadow on layer-shell surfaces by namespace (bar, notifications, osd…). Applied live via mctl reload.",
                            set_halign: gtk::Align::Start,
                            set_xalign: 0.0,
                            set_wrap: true,
                        },
                    },
                },

                gtk::Label { add_css_class: "label-large-bold", set_label: "Add a rule", set_halign: gtk::Align::Start },

                #[template] Row {
                    #[template_child] title { set_label: "Match namespace (regex)" },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,
                        set_placeholder_text: Some("^(notifications|toast|osd).*"),
                        connect_changed[sender] => move |e| sender.input(LayerRulesInput::SetName(e.text().to_string())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "No animation" },
                    gtk::Switch { set_valign: gtk::Align::Center,
                        connect_active_notify[sender] => move |s| sender.input(LayerRulesInput::SetNoAnim(s.is_active())) } },
                #[template] Row {
                    #[template_child] title { set_label: "No blur" },
                    gtk::Switch { set_valign: gtk::Align::Center,
                        connect_active_notify[sender] => move |s| sender.input(LayerRulesInput::SetNoBlur(s.is_active())) } },
                #[template] Row {
                    #[template_child] title { set_label: "No shadow" },
                    gtk::Switch { set_valign: gtk::Align::Center,
                        connect_active_notify[sender] => move |s| sender.input(LayerRulesInput::SetNoShadow(s.is_active())) } },
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "suggested-action",
                    set_label: "Add rule",
                    connect_clicked[sender] => move |_| sender.input(LayerRulesInput::Add),
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
        let model = LayerRulesModel {
            rules: read_block("layerrule"),
            list_box: gtk::ListBox::new(),
            f_name: String::new(),
            f_noanim: false,
            f_noblur: false,
            f_noshadow: false,
        };
        let list_box = model.list_box.clone();
        let widgets = view_output!();
        rebuild_list(&model, &sender);
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            LayerRulesInput::SetName(v) => self.f_name = v,
            LayerRulesInput::SetNoAnim(v) => self.f_noanim = v,
            LayerRulesInput::SetNoBlur(v) => self.f_noblur = v,
            LayerRulesInput::SetNoShadow(v) => self.f_noshadow = v,
            LayerRulesInput::Add => {
                let name = self.f_name.trim();
                if name.is_empty() {
                    return;
                }
                let mut parts: Vec<String> = Vec::new();
                if self.f_noanim {
                    parts.push("noanim:1".to_string());
                }
                if self.f_noblur {
                    parts.push("noblur:1".to_string());
                }
                if self.f_noshadow {
                    parts.push("noshadow:1".to_string());
                }
                if parts.is_empty() {
                    return; // a rule with no action does nothing
                }
                parts.push(format!("layer_name:{name}"));
                self.rules.push(parts.join(", "));
                write_block("layerrule", &self.rules);
                rebuild_list(self, &sender);
            }
            LayerRulesInput::Remove(i) => {
                if i < self.rules.len() {
                    self.rules.remove(i);
                    write_block("layerrule", &self.rules);
                    rebuild_list(self, &sender);
                }
            }
        }
    }
}
