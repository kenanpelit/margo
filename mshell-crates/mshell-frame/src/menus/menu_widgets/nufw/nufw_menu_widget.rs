//! UFW Firewall menu widget — the right-pane content for
//! `MenuType::Nufw`.
//!
//! Mirrors the noctalia `nufw/Panel.qml` layout (status header
//! with on/off switch, default-policy chips, logging line,
//! scrollable rule list with per-row delete buttons, refresh
//! action) using the same primitives every other mshell menu
//! widget uses (`label-large-bold`, `ok-button-surface`,
//! `gtk::Switch`, `gtk::ListBox`). That keeps theming +
//! positioning consistent with clipboard / screenshot / quick-
//! settings menus and makes it scale with the
//! `menus.nufw_menu.minimum_width` config knob.
//!
//! All privileged actions go through `pkexec ufw <args>` — the
//! session's `margo-polkit-agent.service` surfaces the graphical
//! password prompt. No terminal-wrap, no sudo, no askpass dance.

use crate::bars::bar_widgets::nufw::{
    Status, UfwSummary, fetch_ufw_summary, fetch_ufw_summary_pkexec, status_word,
};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, ListBoxRowExt, ObjectExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(120);
const STARTUP_DELAY: Duration = Duration::from_millis(250);
const POST_ACTION_DELAY: Duration = Duration::from_millis(500);

pub(crate) struct NufwMenuWidgetModel {
    summary: UfwSummary,
    status_label: gtk::Label,
    toggle_switch: gtk::Switch,
    toggle_signal: glib::SignalHandlerId,
    policy_in: gtk::Label,
    policy_out: gtk::Label,
    policy_routed: gtk::Label,
    logging_label: gtk::Label,
    rule_list: gtk::ListBox,
}

impl std::fmt::Debug for NufwMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NufwMenuWidgetModel")
            .field("summary", &self.summary)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NufwMenuWidgetInput {
    ToggleEnable(bool),
    DeleteRule(String),
    RefreshNow,
}

#[derive(Debug)]
pub(crate) enum NufwMenuWidgetOutput {}

pub(crate) struct NufwMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum NufwMenuWidgetCommandOutput {
    Refreshed(UfwSummary),
}

#[relm4::component(pub(crate))]
impl Component for NufwMenuWidgetModel {
    type CommandOutput = NufwMenuWidgetCommandOutput;
    type Input = NufwMenuWidgetInput;
    type Output = NufwMenuWidgetOutput;
    type Init = NufwMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "nufw-menu-widget",
            set_hexpand: false,
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,

            // ── Header row ──────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 8,

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "UFW Firewall",
                    set_halign: gtk::Align::Start,
                    set_hexpand: true,
                },

                #[local_ref]
                status_label_widget -> gtk::Label {
                    add_css_class: "nufw-status-badge",
                    set_valign: gtk::Align::Center,
                },

                #[local_ref]
                toggle_switch_widget -> gtk::Switch {
                    set_valign: gtk::Align::Center,
                },
            },

            // ── Default policy chips ────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_homogeneous: true,

                gtk::Box {
                    add_css_class: "nufw-policy-chip",
                    set_orientation: gtk::Orientation::Vertical,
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "In",
                        set_xalign: 0.5,
                    },
                    #[local_ref]
                    policy_in_widget -> gtk::Label {
                        add_css_class: "label-small-bold",
                        set_xalign: 0.5,
                    },
                },
                gtk::Box {
                    add_css_class: "nufw-policy-chip",
                    set_orientation: gtk::Orientation::Vertical,
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Out",
                        set_xalign: 0.5,
                    },
                    #[local_ref]
                    policy_out_widget -> gtk::Label {
                        add_css_class: "label-small-bold",
                        set_xalign: 0.5,
                    },
                },
                gtk::Box {
                    add_css_class: "nufw-policy-chip",
                    set_orientation: gtk::Orientation::Vertical,
                    gtk::Label {
                        add_css_class: "label-small",
                        set_label: "Routed",
                        set_xalign: 0.5,
                    },
                    #[local_ref]
                    policy_routed_widget -> gtk::Label {
                        add_css_class: "label-small-bold",
                        set_xalign: 0.5,
                    },
                },
            },

            // ── Logging row ─────────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                gtk::Label {
                    add_css_class: "label-small",
                    set_label: "Logging",
                    set_xalign: 0.0,
                },
                #[local_ref]
                logging_label_widget -> gtk::Label {
                    add_css_class: "label-small-bold",
                    set_xalign: 1.0,
                    set_hexpand: true,
                },
            },

            gtk::Separator { set_orientation: gtk::Orientation::Horizontal },

            gtk::Label {
                add_css_class: "label-medium-bold",
                set_label: "Rules",
                set_xalign: 0.0,
            },

            // ── Scrollable rule list ────────────────────────────
            gtk::ScrolledWindow {
                set_min_content_height: 180,
                set_max_content_height: 320,
                set_hscrollbar_policy: gtk::PolicyType::Never,
                set_propagate_natural_height: true,

                #[local_ref]
                rule_list_widget -> gtk::ListBox {
                    add_css_class: "nufw-rule-list",
                    set_selection_mode: gtk::SelectionMode::None,
                },
            },

            // ── Footer actions ──────────────────────────────────
            gtk::Box {
                set_orientation: gtk::Orientation::Horizontal,
                set_spacing: 6,
                set_margin_top: 4,

                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "Refresh",
                    connect_clicked[sender] => move |_| {
                        sender.input(NufwMenuWidgetInput::RefreshNow);
                    },
                },
                gtk::Button {
                    set_css_classes: &["ok-button-surface"],
                    set_label: "ufw(8)",
                    connect_clicked => |_| {
                        tokio::spawn(async {
                            let _ = tokio::process::Command::new("xdg-open")
                                .arg("https://manpages.ubuntu.com/manpages/jammy/man8/ufw.8.html")
                                .status()
                                .await;
                        });
                    },
                },
            },
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        // Pre-build all the widget refs the view will splice in via
        // `#[local_ref]` so we can hold them on the model too — that
        // way refresh / action paths can mutate them without
        // re-walking the view tree.
        let status_label_widget = gtk::Label::new(Some("loading"));
        let toggle_switch_widget = gtk::Switch::new();
        let policy_in_widget = gtk::Label::new(Some("—"));
        let policy_out_widget = gtk::Label::new(Some("—"));
        let policy_routed_widget = gtk::Label::new(Some("—"));
        let logging_label_widget = gtk::Label::new(Some("—"));
        let rule_list_widget = gtk::ListBox::new();

        let toggle_sender = sender.clone();
        let toggle_signal = toggle_switch_widget.connect_state_set(move |_, want_on| {
            toggle_sender.input(NufwMenuWidgetInput::ToggleEnable(want_on));
            glib::Propagation::Stop
        });

        // Periodic ufw poll. Tied to the menu widget lifetime so a
        // config reload (which rebuilds menu widgets) gets a fresh
        // task; the old one shuts down via the shutdown channel.
        sender.command(|out, shutdown| {
            async move {
                let shutdown_fut = shutdown.wait();
                tokio::pin!(shutdown_fut);
                let mut first = true;
                loop {
                    let delay = if first { STARTUP_DELAY } else { REFRESH_INTERVAL };
                    first = false;
                    tokio::select! {
                        () = &mut shutdown_fut => break,
                        _ = tokio::time::sleep(delay) => {}
                    }
                    let s = fetch_ufw_summary().await;
                    let _ = out.send(NufwMenuWidgetCommandOutput::Refreshed(s));
                }
            }
        });

        let model = NufwMenuWidgetModel {
            summary: UfwSummary::default(),
            status_label: status_label_widget.clone(),
            toggle_switch: toggle_switch_widget.clone(),
            toggle_signal,
            policy_in: policy_in_widget.clone(),
            policy_out: policy_out_widget.clone(),
            policy_routed: policy_routed_widget.clone(),
            logging_label: logging_label_widget.clone(),
            rule_list: rule_list_widget.clone(),
        };

        let widgets = view_output!();
        sync_view(&model, &sender);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NufwMenuWidgetInput::ToggleEnable(want_on) => {
                let cmd = if want_on { "enable" } else { "disable" };
                spawn_pkexec(&[cmd], sender.clone());
            }
            NufwMenuWidgetInput::DeleteRule(line) => {
                // `ufw delete <RULE>` mirrors what the user would
                // type; safer than `ufw delete <NUM>` which races
                // with concurrent rule edits.
                spawn_pkexec(&["delete", &line], sender.clone());
            }
            NufwMenuWidgetInput::RefreshNow => {
                // Use the pkexec-privileged probe so the panel
                // actually populates with rules + default policies.
                // `ufw status` requires root; the unprivileged
                // background poll can only get the active/inactive
                // bit from `systemctl is-active ufw.service`.
                // Polkit caches credentials for ~5 min, so after
                // the first password prompt the menu refreshes
                // silently within the session.
                sender.command(|out, _shutdown| async move {
                    let s = fetch_ufw_summary_pkexec().await;
                    let _ = out.send(NufwMenuWidgetCommandOutput::Refreshed(s));
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        _widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NufwMenuWidgetCommandOutput::Refreshed(s) => {
                self.summary = s;
                sync_view(self, &sender);
            }
        }
    }
}

fn sync_view(model: &NufwMenuWidgetModel, sender: &ComponentSender<NufwMenuWidgetModel>) {
    let s = &model.summary;

    // Status badge.
    model.status_label.set_label(status_word(s.status));
    let class = match s.status {
        Some(Status::Active) => "nufw-status-active",
        Some(Status::Inactive) => "nufw-status-inactive",
        _ => "nufw-status-unknown",
    };
    model.status_label.set_css_classes(&["nufw-status-badge", class]);

    // Toggle switch — block our own signal so the set_state call
    // doesn't loop back into ToggleEnable.
    let active = matches!(s.status, Some(Status::Active));
    if model.toggle_switch.state() != active {
        model.toggle_switch.block_signal(&model.toggle_signal);
        model.toggle_switch.set_state(active);
        model.toggle_switch.set_active(active);
        model.toggle_switch.unblock_signal(&model.toggle_signal);
    }

    // Policy chips + logging.
    model.policy_in.set_label(empty_to_dash(&s.incoming));
    model.policy_out.set_label(empty_to_dash(&s.outgoing));
    model.policy_routed.set_label(empty_to_dash(&s.routed));
    model.logging_label.set_label(empty_to_dash(&s.logging));

    // Rule list — clear + rebuild. Small enough that diffing buys
    // nothing readable.
    while let Some(row) = model.rule_list.first_child() {
        model.rule_list.remove(&row);
    }
    if s.rules.is_empty() {
        let row = gtk::ListBoxRow::new();
        row.set_activatable(false);
        row.set_selectable(false);
        let label = gtk::Label::new(Some(if matches!(s.status, Some(Status::Inactive)) {
            "(firewall is inactive)"
        } else {
            "(no rules)"
        }));
        label.add_css_class("label-small");
        label.set_xalign(0.0);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        row.set_child(Some(&label));
        model.rule_list.append(&row);
    } else {
        for rule in &s.rules {
            model.rule_list.append(&make_rule_row(rule, sender));
        }
    }
}

fn make_rule_row(rule: &str, sender: &ComponentSender<NufwMenuWidgetModel>) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(4)
        .margin_bottom(4)
        .build();
    let label = gtk::Label::new(Some(rule));
    label.add_css_class("nufw-rule-text");
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_wrap(true);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    outer.append(&label);

    let del = gtk::Button::from_icon_name("user-trash-symbolic");
    del.add_css_class("ok-button-flat");
    del.set_tooltip_text(Some("Delete rule (requires authentication)"));
    let rule_owned = rule.to_string();
    let s = sender.clone();
    del.connect_clicked(move |_| {
        s.input(NufwMenuWidgetInput::DeleteRule(rule_owned.clone()));
    });
    outer.append(&del);

    row.set_child(Some(&outer));
    row
}

fn empty_to_dash(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}

/// Spawn `pkexec ufw <args…>` and kick a refresh after it returns
/// (regardless of success — error path needs the panel updated
/// too so the user sees that nothing changed). polkit's graphical
/// agent (`margo-polkit-agent.service` in this session) handles
/// the password prompt, so we don't need a terminal.
fn spawn_pkexec(args: &[&str], sender: ComponentSender<NufwMenuWidgetModel>) {
    let args: Vec<String> = std::iter::once("ufw".to_string())
        .chain(args.iter().map(|s| s.to_string()))
        .collect();
    // sender.command runs on relm4's tokio executor — both the
    // pkexec await and the post-action sleep need that runtime.
    sender.command(move |out, _shutdown| async move {
        let status = tokio::process::Command::new("pkexec")
            .args(&args)
            .status()
            .await;
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => warn!(?s, ?args, "pkexec ufw returned non-zero"),
            Err(e) => warn!(error = %e, ?args, "pkexec spawn failed"),
        }
        tokio::time::sleep(POST_ACTION_DELAY).await;
        // Re-fetch via pkexec right away — credentials are still
        // cached in polkit's window, so this won't re-prompt.
        let s = fetch_ufw_summary_pkexec().await;
        let _ = out.send(NufwMenuWidgetCommandOutput::Refreshed(s));
    });
}
