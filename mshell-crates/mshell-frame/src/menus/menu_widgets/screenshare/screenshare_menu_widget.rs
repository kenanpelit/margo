use crate::common_widgets::big_button::BigButton;
use mshell_screenshot::{ScreenSelectAreaRequest, ScreenSelection, select_screen};
use relm4::gtk::prelude::*;
use relm4::gtk::{gdk, glib};
use relm4::{Component, ComponentParts, ComponentSender, RelmWidgetExt, gtk};

#[derive(Debug, Clone)]
pub(crate) struct ScreenShareWindow {
    pub window_id: String,
    pub window_program: String,
    pub instance_title: String,
}

#[derive(Debug, Clone)]
pub(crate) struct Program {
    pub name: String,
    pub windows: Vec<ScreenShareWindow>,
}

#[derive(Debug)]
pub(crate) struct ScreenshareMenuWidgetModel {
    pub programs: Vec<Program>,
    reply: Option<tokio::sync::oneshot::Sender<String>>,
    is_revealed: bool,
}

#[derive(Debug)]
pub(crate) enum ScreenshareMenuWidgetInput {
    ParentRevealChanged(bool),
    SetReply(tokio::sync::oneshot::Sender<String>, String),
    SendReply(String),
    MonitorClicked,
    AreaClicked,
}

#[derive(Debug)]
pub(crate) enum ScreenshareMenuWidgetOutput {
    CloseMenu,
}

pub(crate) struct ScreenshareMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum ScreenshareMenuWidgetCommandOutput {}

#[relm4::component(pub)]
impl Component for ScreenshareMenuWidgetModel {
    type CommandOutput = ScreenshareMenuWidgetCommandOutput;
    type Input = ScreenshareMenuWidgetInput;
    type Output = ScreenshareMenuWidgetOutput;
    type Init = ScreenshareMenuWidgetInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "screen-share-menu-widget",
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 12,

            // ── §12 panel header ──
            gtk::Box {
                add_css_class: "panel-header",
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 12,
                gtk::Image {
                    add_css_class: "panel-header-icon",
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("camera-video-symbolic"),
                },
                gtk::Label {
                    add_css_class: "panel-title",
                    set_label: "Choose what to share",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                    set_valign: gtk::Align::Center,
                },
            },

            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 32,
                set_focusable: true,
                set_align: gtk::Align::Center,

                #[template]
                #[name = "monitor_button"]
                BigButton {
                    connect_clicked[sender] => move |_| {
                        sender.input(ScreenshareMenuWidgetInput::MonitorClicked);
                    },

                    #[template_child]
                    icon {
                        set_icon_name: Some("screenshot-monitor-symbolic"),
                    },
                    #[template_child]
                    label {
                        set_label: "Monitor",
                    },
                },

                #[template]
                BigButton {
                    connect_clicked[sender] => move |_| {
                        sender.input(ScreenshareMenuWidgetInput::AreaClicked);
                    },

                    #[template_child]
                    icon {
                        set_icon_name: Some("screenshot-area-symbolic"),
                    },
                    #[template_child]
                    label {
                        set_label: "Area",
                    },
                },
            },

            #[name = "programs_box"]
            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 12,
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let key_controller = gtk::EventControllerKey::new();
        let sender_clone = sender.clone();
        key_controller.connect_key_pressed(move |_, key, _, _| match key {
            gdk::Key::Escape => {
                sender_clone.input(ScreenshareMenuWidgetInput::SendReply(String::new()));
                let _ = sender_clone.output(ScreenshareMenuWidgetOutput::CloseMenu);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        });

        let model = ScreenshareMenuWidgetModel {
            programs: Vec::new(),
            reply: None,
            is_revealed: false,
        };

        let widgets = view_output!();

        widgets.root.add_controller(key_controller);

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
            ScreenshareMenuWidgetInput::ParentRevealChanged(revealed) => {
                // Keyboard mode is owned centrally by the frame's
                // `sync_keyboard_mode` (Exclusive iff a menu is genuinely
                // revealed). Do NOT flip it here: `ParentRevealChanged`
                // rides the menu ScrolledWindow's `map` signal, which fires
                // at startup even when the frame never revealed us, and
                // setting Exclusive there strands the invisible frame layer
                // in keyboard focus (the login focus-trap).
                if revealed && !self.is_revealed {
                    widgets.monitor_button.grab_focus();
                }
                self.is_revealed = revealed;
            }
            ScreenshareMenuWidgetInput::SetReply(reply, payload) => {
                self.reply = Some(reply);
                self.programs = group_by_window_program(parse_screen_share_string(&payload));
                rebuild_programs_list(&widgets.programs_box, &self.programs, &sender);
            }
            ScreenshareMenuWidgetInput::SendReply(reply_value) => {
                if let Some(reply) = self.reply.take() {
                    let _ = reply.send(reply_value);
                }
            }
            ScreenshareMenuWidgetInput::MonitorClicked => {
                let _ = sender.output(ScreenshareMenuWidgetOutput::CloseMenu);
                let sender_clone = sender.clone();
                select_screen(
                    ScreenSelectAreaRequest::SelectMonitor,
                    move |result| match result {
                        Ok(selection) => {
                            complete_selection(&selection, &sender_clone);
                        }
                        Err(e) => {
                            eprintln!("Selection failed: {e}");
                        }
                    },
                );
            }
            ScreenshareMenuWidgetInput::AreaClicked => {
                let _ = sender.output(ScreenshareMenuWidgetOutput::CloseMenu);
                let sender_clone = sender.clone();
                select_screen(
                    ScreenSelectAreaRequest::SelectRegion,
                    move |result| match result {
                        Ok(selection) => {
                            complete_selection(&selection, &sender_clone);
                        }
                        Err(e) => {
                            eprintln!("Selection failed: {e}");
                            sender.input(ScreenshareMenuWidgetInput::SendReply(String::new()));
                        }
                    },
                );
            }
        }
    }
}

fn complete_selection(
    selection: &ScreenSelection,
    sender: &ComponentSender<ScreenshareMenuWidgetModel>,
) {
    match selection {
        ScreenSelection::Monitor(name) => {
            sender.input(ScreenshareMenuWidgetInput::SendReply(format!(
                "[SELECTION]/screen:{name}"
            )));
        }
        ScreenSelection::Region(region) => {
            sender.input(ScreenshareMenuWidgetInput::SendReply(format!(
                "[SELECTION]/region:{}@{},{},{},{}",
                region.output, region.x, region.y, region.width, region.height
            )));
        }
    }
}

fn rebuild_programs_list(
    programs_box: &gtk::Box,
    programs: &[Program],
    sender: &ComponentSender<ScreenshareMenuWidgetModel>,
) {
    while let Some(child) = programs_box.first_child() {
        programs_box.remove(&child);
    }

    for program in programs {
        let program_box = gtk::Box::new(gtk::Orientation::Vertical, 6);

        // Header label
        let label = gtk::Label::new(Some(&truncate(&program.name, 30)));
        label.add_css_class("label-large-bold");
        label.set_halign(gtk::Align::Center);
        program_box.append(&label);

        // Window buttons
        for window in &program.windows {
            let button = gtk::Button::new();
            let button_label = gtk::Label::new(Some(&window.instance_title));
            button_label.add_css_class("label-small");
            button_label.set_halign(gtk::Align::Fill);
            button_label.set_xalign(0.0);
            button_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            button_label.set_hexpand(true);
            button.add_css_class("ok-button-primary");
            button.set_hexpand(true);
            button.set_child(Some(&button_label));

            let window_id = window.window_id.clone();
            let sender_clone = sender.clone();
            button.connect_clicked(move |_| {
                let _ = sender_clone.output(ScreenshareMenuWidgetOutput::CloseMenu);
                sender_clone.input(ScreenshareMenuWidgetInput::SendReply(format!(
                    "[SELECTION]/window:{window_id}"
                )));
            });

            program_box.append(&button);
        }

        programs_box.append(&program_box);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() > max {
        s.chars().take(max).collect::<String>() + "…"
    } else {
        s.to_string()
    }
}

fn parse_screen_share_string(input: &str) -> Vec<ScreenShareWindow> {
    input
        .split("[HA>]")
        .filter(|part| !part.trim().is_empty() && part.contains("[HC>]") && part.contains("[HT>]"))
        .filter_map(|part| {
            let (window_id, rest) = part.split_once("[HC>]")?;
            let (window_program, instance_title) = rest.split_once("[HT>]")?;
            let instance_title = instance_title
                .split("[HE>]")
                .next()
                .unwrap_or(instance_title);
            Some(ScreenShareWindow {
                window_id: window_id.trim().to_string(),
                window_program: window_program.trim().to_string(),
                instance_title: instance_title.trim().to_string(),
            })
        })
        .collect()
}

fn group_by_window_program(windows: Vec<ScreenShareWindow>) -> Vec<Program> {
    let mut grouped: Vec<Program> = Vec::new();

    for window in windows {
        if let Some(group) = grouped.iter_mut().find(|g| g.name == window.window_program) {
            group.windows.push(window);
        } else {
            grouped.push(Program {
                name: window.window_program.clone(),
                windows: vec![window],
            });
        }
    }

    grouped.sort_by(|a, b| a.name.cmp(&b.name));

    for program in &mut grouped {
        program
            .windows
            .sort_by(|a, b| a.instance_title.cmp(&b.instance_title));
    }

    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triples(ws: &[ScreenShareWindow]) -> Vec<(&str, &str, &str)> {
        ws.iter()
            .map(|w| {
                (
                    w.window_id.as_str(),
                    w.window_program.as_str(),
                    w.instance_title.as_str(),
                )
            })
            .collect()
    }

    #[test]
    fn parses_the_margo_portal_payload() {
        // margo-portal: `<id>[HC>]<class>[HT>]<title>[HE>]` joined by `[HA>]`.
        let payload = "12[HC>]firefox[HT>]Mozilla Firefox[HE>][HA>]7[HC>]foot[HT>]~/src[HE>]";
        assert_eq!(
            triples(&parse_screen_share_string(payload)),
            [("12", "firefox", "Mozilla Firefox"), ("7", "foot", "~/src")]
        );
    }

    #[test]
    fn tolerates_xdph_trailing_fields_and_separators() {
        // xdph appends the window address after `[HE>]` and may end with
        // a trailing `[HA>]`.
        let payload = "1[HC>]kitty[HT>]vim[HE>]0x55d0[HA>]";
        assert_eq!(
            triples(&parse_screen_share_string(payload)),
            [("1", "kitty", "vim")]
        );
        // No `[HE>]` at all: the whole remainder is the title.
        assert_eq!(
            triples(&parse_screen_share_string("2[HC>]a[HT>]b")),
            [("2", "a", "b")]
        );
    }

    #[test]
    fn trims_fields_and_keeps_empty_titles() {
        let payload = " 3 [HC>] code [HT>]  [HE>]";
        assert_eq!(
            triples(&parse_screen_share_string(payload)),
            [("3", "code", "")]
        );
    }

    #[test]
    fn titles_may_contain_markup_lookalikes() {
        let payload = "4[HC>]term[HT>]echo [HC>] > x[HE>]";
        assert_eq!(
            triples(&parse_screen_share_string(payload)),
            [("4", "term", "echo [HC>] > x")]
        );
    }

    #[test]
    fn malformed_entries_are_dropped() {
        assert!(parse_screen_share_string("").is_empty());
        assert!(parse_screen_share_string("   ").is_empty());
        assert!(parse_screen_share_string("garbage").is_empty());
        let payload = "5[HC>]no-title-marker[HA>]6[HC>]ok[HT>]fine[HE>][HA>][HT>]no-class";
        assert_eq!(
            triples(&parse_screen_share_string(payload)),
            [("6", "ok", "fine")]
        );
    }

    #[test]
    fn groups_by_program_sorted_by_name_then_title() {
        let ws = parse_screen_share_string(
            "1[HC>]foot[HT>]b[HE>][HA>]2[HC>]firefox[HT>]z[HE>][HA>]3[HC>]foot[HT>]a[HE>]",
        );
        let grouped = group_by_window_program(ws);
        let names: Vec<_> = grouped.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["firefox", "foot"]);
        let foot: Vec<_> = grouped[1]
            .windows
            .iter()
            .map(|w| (w.window_id.as_str(), w.instance_title.as_str()))
            .collect();
        assert_eq!(foot, [("3", "a"), ("1", "b")]);
        assert!(group_by_window_program(Vec::new()).is_empty());
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        assert_eq!(truncate("short", 30), "short");
        assert_eq!(truncate("abcdef", 3), "abc…");
        assert_eq!(truncate("çğıöşü", 6), "çğıöşü");
        assert_eq!(truncate("çğıöşü", 2), "çğ…");
    }
}
