//! Settings → Tag Rules. List editor over margo's `tagrule = …` lines — pin a
//! tag to a monitor and/or give it a layout, master factor or master count.

use crate::compositor_conf::{read_block, write_block};
use crate::row::Row;
use relm4::gtk::prelude::*;
use relm4::{Component, ComponentParts, ComponentSender, gtk};

#[derive(Debug)]
pub(crate) enum TagRulesInput {
    SetTag(u32),
    SetMonitor(String),
    SetLayout(String),
    SetMfact(f64),
    SetNmaster(f64),
    Add,
    Remove(usize),
}

#[derive(Debug)]
pub(crate) enum TagRulesOutput {}
#[derive(Debug)]
pub(crate) enum TagRulesCommandOutput {}
pub(crate) struct TagRulesInit {}

pub(crate) struct TagRulesModel {
    rules: Vec<String>,
    list_box: gtk::ListBox,
    tags_model: gtk::StringList,
    f_tag: u32,
    f_monitor: String,
    f_layout: String,
    f_mfact: f64,
    f_nmaster: i32,
}

fn rebuild_list(model: &TagRulesModel, sender: &ComponentSender<TagRulesModel>) {
    while let Some(child) = model.list_box.first_child() {
        model.list_box.remove(&child);
    }
    if model.rules.is_empty() {
        let row = gtk::ListBoxRow::new();
        row.set_selectable(false);
        let lbl = gtk::Label::new(Some("No tag rules yet."));
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
        // Break mid-token (regex / paths have no spaces) + report the
        // wrapped width as natural, so a long payload doesn't force the
        // Settings panel wider than its configured size on this page.
        lbl.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        lbl.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
        lbl.set_selectable(true);
        lbl.add_css_class("label-medium");
        hbox.append(&lbl);
        let btn = gtk::Button::from_icon_name("user-trash-symbolic");
        btn.add_css_class("flat");
        btn.set_valign(gtk::Align::Center);
        let s = sender.clone();
        btn.connect_clicked(move |_| s.input(TagRulesInput::Remove(i)));
        hbox.append(&btn);
        row.set_child(Some(&hbox));
        model.list_box.append(&row);
    }
}

#[relm4::component(pub)]
impl Component for TagRulesModel {
    type CommandOutput = TagRulesCommandOutput;
    type Input = TagRulesInput;
    type Output = TagRulesOutput;
    type Init = TagRulesInit;

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
                        set_icon_name: Some("view-grid-symbolic"),
                        set_valign: gtk::Align::Center,
                    },
                    gtk::Box {
                        set_orientation: gtk::Orientation::Vertical,
                        set_valign: gtk::Align::Center,
                        gtk::Label { add_css_class: "settings-hero-title", set_label: "Tag Rules", set_halign: gtk::Align::Start },
                        gtk::Label {
                            add_css_class: "settings-hero-subtitle",
                            set_label: "Pin a tag to a monitor and/or give it a layout, master factor or master count. Applied live via mctl reload.",
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
                    #[template_child] title { set_label: "Tag" },
                    gtk::DropDown {
                        set_valign: gtk::Align::Center,
                        set_model: Some(&model.tags_model),
                        connect_selected_notify[sender] => move |d| sender.input(TagRulesInput::SetTag(d.selected())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Home monitor (connector name)" },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_hexpand: true,
                        set_placeholder_text: Some("e.g. DP-1 — optional"),
                        connect_changed[sender] => move |e| sender.input(TagRulesInput::SetMonitor(e.text().to_string())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Layout name" },
                    #[template_child] desc { set_label: "e.g. tile, scroller, monocle, grid — optional." },
                    gtk::Entry {
                        set_valign: gtk::Align::Center,
                        set_placeholder_text: Some("optional"),
                        connect_changed[sender] => move |e| sender.input(TagRulesInput::SetLayout(e.text().to_string())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Master factor (0 = leave default)" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_digits: 2,
                        set_adjustment: &gtk::Adjustment::new(0.0, 0.0, 0.95, 0.05, 0.1, 0.0),
                        connect_value_changed[sender] => move |s| sender.input(TagRulesInput::SetMfact(s.value())),
                    },
                },
                #[template] Row {
                    #[template_child] title { set_label: "Master count (0 = leave default)" },
                    gtk::SpinButton {
                        set_valign: gtk::Align::Center,
                        set_adjustment: &gtk::Adjustment::new(0.0, 0.0, 9.0, 1.0, 1.0, 0.0),
                        connect_value_changed[sender] => move |s| sender.input(TagRulesInput::SetNmaster(s.value())),
                    },
                },
                },
                gtk::Button {
                    set_halign: gtk::Align::Start,
                    add_css_class: "ok-button-primary",
                    set_label: "Add rule",
                    connect_clicked[sender] => move |_| sender.input(TagRulesInput::Add),
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
        let model = TagRulesModel {
            rules: read_block("tagrule"),
            list_box: gtk::ListBox::new(),
            tags_model: gtk::StringList::new(&["1", "2", "3", "4", "5", "6", "7", "8", "9"]),
            f_tag: 1,
            f_monitor: String::new(),
            f_layout: String::new(),
            f_mfact: 0.0,
            f_nmaster: 0,
        };
        let list_box = model.list_box.clone();
        let widgets = view_output!();
        rebuild_list(&model, &sender);
        let _ = root;
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            TagRulesInput::SetTag(i) => self.f_tag = i + 1,
            TagRulesInput::SetMonitor(v) => self.f_monitor = v,
            TagRulesInput::SetLayout(v) => self.f_layout = v,
            TagRulesInput::SetMfact(v) => self.f_mfact = v,
            TagRulesInput::SetNmaster(v) => self.f_nmaster = v as i32,
            TagRulesInput::Add => {
                let monitor = self.f_monitor.trim();
                let layout = self.f_layout.trim();
                // Needs at least one action beyond the tag id.
                if monitor.is_empty()
                    && layout.is_empty()
                    && self.f_mfact <= 0.0
                    && self.f_nmaster <= 0
                {
                    return;
                }
                let mut parts: Vec<String> = vec![format!("id:{}", self.f_tag)];
                if !monitor.is_empty() {
                    parts.push(format!("monitor_name:{monitor}"));
                }
                if !layout.is_empty() {
                    parts.push(format!("layout_name:{layout}"));
                }
                if self.f_mfact > 0.0 {
                    parts.push(format!("mfact:{:.2}", self.f_mfact));
                }
                if self.f_nmaster > 0 {
                    parts.push(format!("nmaster:{}", self.f_nmaster));
                }
                self.rules.push(parts.join(", "));
                write_block("tagrule", &self.rules);
                rebuild_list(self, &sender);
            }
            TagRulesInput::Remove(i) => {
                if i < self.rules.len() {
                    self.rules.remove(i);
                    write_block("tagrule", &self.rules);
                    rebuild_list(self, &sender);
                }
            }
        }
    }
}
