//! UFW Firewall menu widget — the content of `MenuType::Ufw` (and the
//! Control Center's Firewall page).
//!
//! * Header (§12 `PanelHeader`): status line, refresh, and the on/off
//!   switch (turning off asks first).
//! * Banner: what's wrong when the firewall is amber (unit failed / not
//!   running / not started at boot) or its files need root, with the
//!   one-click fix (Start ufw, Start at boot, Load rules).
//! * Settings rows: Start at boot (the `ufw.service` unit), the default
//!   policies and logging level.
//! * Rules: every rule ufw manages, read from `/etc/ufw/user*.rules`
//!   without root; each deletable rule has a delete button behind an
//!   inline confirm. The `+` opens the add form ([`super::form`]).
//!
//! State comes from the shared watcher
//! ([`crate::bars::bar_widgets::ufw::service`]) while the menu is open;
//! every change runs through the audited root helper (one polkit prompt
//! each) and the state is re-read right after. Failures are toasts (and
//! inline for the form).

use super::form::{self, FormUi, PROTOS, Verdict};
use super::view::{self, BannerAction};
use crate::bars::bar_widgets::ufw::service;
use crate::menus::menu_widgets::panel_header::PanelHeader;
use crate::menus::reveal_poll::{RevealGate, watch_stream};
use mshell_config::schema::menu_widgets::MenuWidget;
use mshell_privileged::ufw::rules::Rule;
use mshell_privileged::ufw::spec::{Action, PRESETS, Proto, RuleSpec};
use mshell_privileged::ufw::state::{self, Fix, Health, Snapshot};
use mshell_privileged::ufw::{Switch, UfwOp, UnitOp};
use relm4::gtk::glib;
use relm4::gtk::prelude::{
    BoxExt, ButtonExt, EditableExt, ListBoxRowExt, ObjectExt, OrientableExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::sync::Arc;

/// The rule list's height cap when no menu height is configured.
const LIST_CAP: i32 = 320;

/// What the inline confirm bar asks.
#[derive(Debug, Clone)]
enum Confirm {
    Delete(Rule, RuleSpec),
    TurnOff,
}

pub(crate) struct UfwMenuWidgetModel {
    snap: Arc<Snapshot>,
    health: Health,
    action: Action,
    proto: Proto,
    confirm: Option<Confirm>,
    /// A root action of this menu is running.
    working: bool,
    /// The last failed add, shown in the form until the next edit.
    form_error: Option<String>,
    started: bool,
    visible: RevealGate,
    /// What the rule list was last built from (snapshot, busy, editable):
    /// a keystroke in the form re-renders without rebuilding the list.
    list_key: std::cell::Cell<Option<(usize, bool, bool)>>,
    ui: Ui,
}

struct Ui {
    subtitle: gtk::Label,
    switch: gtk::Switch,
    switch_handler: glib::SignalHandlerId,
    banner: gtk::Box,
    banner_text: gtk::Label,
    banner_actions: gtk::Box,
    boot_row: gtk::Box,
    boot_switch: gtk::Switch,
    boot_handler: glib::SignalHandlerId,
    policy: gtk::Label,
    confirm: gtk::Box,
    confirm_text: gtk::Label,
    confirm_ok: gtk::Button,
    rules_title: gtk::Label,
    add_toggle: gtk::Button,
    form_reveal: gtk::Revealer,
    form: FormUi,
    list: gtk::ListBox,
}

impl std::fmt::Debug for UfwMenuWidgetModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UfwMenuWidgetModel")
            .field("health", &self.health)
            .field("working", &self.working)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum UfwMenuWidgetInput {
    /// Sent by the host menu (and the Control Center page) on show /
    /// hide: the watcher subscription lives only while shown.
    ParentRevealChanged(bool),
    Refresh,
    SetEnabled(bool),
    SetBoot(bool),
    Banner(BannerAction),
    ToggleForm,
    SetAction(usize),
    SetProto(usize),
    FormChanged,
    Preset(usize),
    Add,
    AskDelete(usize),
    ConfirmYes,
    ConfirmNo,
}

#[derive(Debug)]
pub(crate) enum UfwMenuWidgetOutput {}

pub(crate) struct UfwMenuWidgetInit {}

#[derive(Debug)]
pub(crate) enum UfwMenuWidgetCommandOutput {
    State(Arc<Snapshot>),
    Done {
        title: &'static str,
        result: Result<(), String>,
        added: bool,
    },
}

#[relm4::component(pub(crate))]
impl Component for UfwMenuWidgetModel {
    type CommandOutput = UfwMenuWidgetCommandOutput;
    type Input = UfwMenuWidgetInput;
    type Output = UfwMenuWidgetOutput;
    type Init = UfwMenuWidgetInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "ufw-menu-widget",
            set_hexpand: false,
            set_orientation: gtk::Orientation::Vertical,
            set_spacing: 10,
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let widgets = view_output!();
        let ui = build(&root, &sender);
        let snap = service::current();
        let model = UfwMenuWidgetModel {
            health: state::classify(&snap),
            snap,
            action: Action::Allow,
            proto: Proto::Any,
            confirm: None,
            working: false,
            form_error: None,
            started: false,
            visible: RevealGate::new(false),
            list_key: std::cell::Cell::new(None),
            ui,
        };
        model.render(&sender);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>, _root: &Self::Root) {
        use UfwMenuWidgetInput as In;
        match message {
            In::ParentRevealChanged(visible) => {
                self.visible.set(visible);
                if visible {
                    if !self.started {
                        self.started = true;
                        let gate = self.visible.clone();
                        sender.command(move |out, shutdown| async move {
                            let watch = gate.stream_while_revealed(
                                || watch_stream(service::subscribe()),
                                |snap| {
                                    let _ = out.send(UfwMenuWidgetCommandOutput::State(snap));
                                },
                            );
                            tokio::select! {
                                () = shutdown.wait() => {}
                                () = watch => {}
                            }
                        });
                    }
                    service::refresh();
                } else {
                    self.confirm = None;
                }
            }
            In::Refresh => service::refresh(),
            In::SetEnabled(on) => {
                if on {
                    self.run(
                        &sender,
                        "Couldn't turn the firewall on",
                        UfwOp::Switch(Switch::On),
                        false,
                    );
                } else {
                    self.confirm = Some(Confirm::TurnOff);
                }
            }
            In::SetBoot(on) => {
                let op = UfwOp::Unit(if on { UnitOp::Enable } else { UnitOp::Disable });
                self.run(&sender, "Couldn't change the boot setting", op, false);
            }
            In::Banner(BannerAction::Fix(Fix::StartUnit)) => {
                self.run(
                    &sender,
                    "Couldn't start ufw",
                    UfwOp::Unit(UnitOp::Start),
                    false,
                );
            }
            In::Banner(BannerAction::Fix(Fix::EnableAtBoot)) => {
                self.run(
                    &sender,
                    "Couldn't enable ufw at boot",
                    UfwOp::Unit(UnitOp::Enable),
                    false,
                );
            }
            In::Banner(BannerAction::LoadRules) if !self.working => {
                self.working = true;
                sender.oneshot_command(async {
                    UfwMenuWidgetCommandOutput::Done {
                        title: "Couldn't load the rules",
                        result: service::load_rules_as_root().await,
                        added: false,
                    }
                });
            }
            In::Banner(BannerAction::LoadRules) => {}
            In::ToggleForm => {
                let open = !self.ui.form_reveal.reveals_child();
                self.ui.form_reveal.set_reveal_child(open);
                if open {
                    self.ui.form.port.grab_focus();
                }
            }
            In::SetAction(i) => {
                self.action = Action::ALL.get(i).copied().unwrap_or(Action::Allow);
                self.ui.form.set_action(i);
                self.form_error = None;
            }
            In::SetProto(i) => {
                self.proto = PROTOS.get(i).copied().unwrap_or(Proto::Any);
                self.ui.form.set_proto(i);
                self.form_error = None;
            }
            In::FormChanged => self.form_error = None,
            In::Preset(i) => {
                if let Some(p) = PRESETS.get(i) {
                    self.ui.form.port.set_text(p.port);
                    self.ui.form.comment.set_text(p.comment);
                    self.proto = p.proto;
                    let idx = PROTOS.iter().position(|x| *x == p.proto).unwrap_or(2);
                    self.ui.form.set_proto(idx);
                    self.form_error = None;
                }
            }
            In::Add => {
                let listed = self.snap.rules.as_ref().ok();
                let form = self.ui.form.read(self.action, self.proto);
                if let Verdict::Ready(add, _) = form::verdict(&form, listed)
                    && self.health.rules_editable()
                    && !self.working
                {
                    self.run(&sender, "Couldn't add the rule", UfwOp::Add(add), true);
                }
            }
            In::AskDelete(i) => {
                if let Ok(set) = &self.snap.rules
                    && let Some(rule) = set.rules.get(i)
                    && let Some(spec) = rule.delete_spec(set.complete)
                {
                    self.confirm = Some(Confirm::Delete(rule.clone(), spec));
                }
            }
            In::ConfirmNo => self.confirm = None,
            In::ConfirmYes => match self.confirm.take() {
                Some(Confirm::TurnOff) => {
                    self.run(
                        &sender,
                        "Couldn't turn the firewall off",
                        UfwOp::Switch(Switch::Off),
                        false,
                    );
                }
                Some(Confirm::Delete(_, spec)) => {
                    self.run(
                        &sender,
                        "Couldn't remove the rule",
                        UfwOp::Delete(spec),
                        false,
                    );
                }
                None => {}
            },
        }
        self.render(&sender);
    }

    fn update_cmd(
        &mut self,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            UfwMenuWidgetCommandOutput::State(snap) => {
                self.health = state::classify(&snap);
                self.snap = snap;
            }
            UfwMenuWidgetCommandOutput::Done {
                title,
                result,
                added,
            } => {
                self.working = false;
                match result {
                    Ok(()) if added => self.ui.form.clear(),
                    Ok(()) => {}
                    Err(e) => {
                        if added {
                            self.form_error = Some(e.clone());
                        }
                        // A dismissed prompt is the user's own choice.
                        if e != "Authorization was cancelled" {
                            service::report(title, Err(e));
                        }
                    }
                }
            }
        }
        self.render(&sender);
    }
}

impl UfwMenuWidgetModel {
    /// Run a root action off the GTK thread; [`UfwMenuWidgetCommandOutput::Done`] follows.
    fn run(&mut self, sender: &ComponentSender<Self>, title: &'static str, op: UfwOp, added: bool) {
        if self.working {
            return;
        }
        self.working = true;
        sender.oneshot_command(async move {
            UfwMenuWidgetCommandOutput::Done {
                title,
                result: service::run(op).await.map(|_| ()),
                added,
            }
        });
    }

    fn render(&self, sender: &ComponentSender<Self>) {
        let ui = &self.ui;
        let snap = &self.snap;
        let h = &self.health;
        let busy = self.working || service::busy();

        let line = if self.working {
            "Waiting for authentication…".to_string()
        } else {
            view::status_line(snap, h)
        };
        set_text(&ui.subtitle, &line);

        let on = h.enforcing();
        if ui.switch.state() != on || ui.switch.is_active() != on {
            ui.switch.block_signal(&ui.switch_handler);
            ui.switch.set_state(on);
            ui.switch.set_active(on);
            ui.switch.unblock_signal(&ui.switch_handler);
        }
        ui.switch
            .set_sensitive(!busy && snap.installed && snap.conf.is_ok());

        // Banner.
        match view::banner(snap, h) {
            Some(b) => {
                ui.banner.set_visible(true);
                for tone in ["warn", "muted"] {
                    set_class(&ui.banner, tone, tone == b.tone);
                }
                set_text(&ui.banner_text, &b.text);
                while let Some(c) = ui.banner_actions.first_child() {
                    ui.banner_actions.remove(&c);
                }
                for a in b.actions {
                    let btn = gtk::Button::with_label(a.label());
                    btn.set_css_classes(&["ok-button-surface", "ok-button-cell"]);
                    btn.set_sensitive(!busy);
                    let s = sender.clone();
                    btn.connect_clicked(move |_| s.input(UfwMenuWidgetInput::Banner(a)));
                    ui.banner_actions.append(&btn);
                }
            }
            None => ui.banner.set_visible(false),
        }

        // Start at boot.
        match snap.unit() {
            Some(unit) => {
                ui.boot_row.set_visible(true);
                let boot = unit.starts_at_boot();
                if ui.boot_switch.is_active() != boot {
                    ui.boot_switch.block_signal(&ui.boot_handler);
                    ui.boot_switch.set_state(boot);
                    ui.boot_switch.set_active(boot);
                    ui.boot_switch.unblock_signal(&ui.boot_handler);
                }
                // "static" units can't be switched.
                let switchable = matches!(unit.file.as_str(), "enabled" | "disabled");
                ui.boot_switch.set_sensitive(!busy && switchable);
            }
            None => ui.boot_row.set_visible(false),
        }
        set_text(&ui.policy, &view::policy_line(snap));

        // Confirm bar.
        match &self.confirm {
            Some(c) => {
                ui.confirm.set_visible(true);
                let (q, ok) = match c {
                    Confirm::TurnOff => (
                        "Turn the firewall off? No rule is enforced until you turn it on again, also after a reboot.".to_string(),
                        "Turn off",
                    ),
                    Confirm::Delete(rule, _) => (
                        view::delete_question(
                            rule,
                            snap.rules.as_ref().is_ok_and(|set| set.ipv6),
                        ),
                        "Remove",
                    ),
                };
                set_text(&ui.confirm_text, &q);
                ui.confirm_ok.set_label(ok);
                ui.confirm_ok.set_sensitive(!busy);
            }
            None => ui.confirm.set_visible(false),
        }

        // Form.
        let editable = h.rules_editable() && snap.installed;
        ui.form.set_sensitive(!busy);
        ui.add_toggle.set_sensitive(snap.installed);
        let listed = snap.rules.as_ref().ok();
        let verdict = match &self.form_error {
            Some(e) => Verdict::Problem(e.clone()),
            None if !editable => {
                Verdict::Problem("Start ufw first: rule changes need its service running".into())
            }
            None => form::verdict(&ui.form.read(self.action, self.proto), listed),
        };
        ui.form.show(&verdict, editable && !busy);

        // Rules.
        let title = match &snap.rules {
            Ok(set) => format!("Rules · {}", set.rules.len()),
            Err(_) => "Rules".to_string(),
        };
        set_text(&ui.rules_title, &title);
        self.render_rules(sender, busy);
    }

    fn render_rules(&self, sender: &ComponentSender<Self>, busy: bool) {
        let editable = self.health.rules_editable();
        let key = Some((Arc::as_ptr(&self.snap) as usize, busy, editable));
        if self.list_key.replace(key) == key {
            return;
        }
        let list = &self.ui.list;
        while let Some(row) = list.first_child() {
            list.remove(&row);
        }
        let snap = &self.snap;
        let placeholder = |text: &str| {
            let l = gtk::Label::new(Some(text));
            l.set_css_classes(&["label-small", "dim-label"]);
            l.set_xalign(0.0);
            l.set_wrap(true);
            list.append(&plain_row(&l));
        };
        if !snap.installed {
            placeholder("UFW isn't installed (package ufw).");
            return;
        }
        let set = match &snap.rules {
            Ok(set) => set,
            Err(state::ReadError::Denied) => {
                placeholder("The rule files can't be read without root. Load rules shows them.");
                return;
            }
            Err(state::ReadError::Missing) => {
                placeholder("No rule file yet (/etc/ufw/user.rules).");
                return;
            }
            Err(state::ReadError::Other(_)) if **snap == Snapshot::loading() => {
                placeholder("Reading the rules…");
                return;
            }
            Err(state::ReadError::Other(e)) => {
                placeholder(&format!("The rules can't be read: {e}"));
                return;
            }
        };
        if set.rules.is_empty() {
            placeholder("No rules yet. Use + to allow a port.");
            return;
        }
        for (i, rule) in set.rules.iter().enumerate() {
            let (title, sub) = view::rule_texts(rule, set.ipv6);
            let body = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            body.append(&title_block(&title, &sub));
            if rule.delete_spec(set.complete).is_some() {
                let del = gtk::Button::from_icon_name("user-trash-symbolic");
                del.add_css_class("ok-button-flat");
                del.set_valign(gtk::Align::Center);
                del.set_sensitive(editable && !busy);
                mshell_common::a11y::name(&del, &format!("Remove the {title} rule"));
                let s = sender.clone();
                del.connect_clicked(move |_| s.input(UfwMenuWidgetInput::AskDelete(i)));
                body.append(&del);
            } else {
                body.set_tooltip_text(Some(view::NOT_HERE));
            }
            list.append(&plain_row(&body));
        }
    }
}

fn set_text(label: &gtk::Label, text: &str) {
    if label.label() != text {
        label.set_label(text);
    }
}

fn set_class(w: &impl WidgetExt, class: &str, on: bool) {
    if w.has_css_class(class) != on {
        if on {
            w.add_css_class(class);
        } else {
            w.remove_css_class(class);
        }
    }
}

fn plain_row(child: &impl gtk::prelude::IsA<gtk::Widget>) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_selectable(false);
    row.set_child(Some(child));
    row
}

/// Title + dim subtitle (the Adwaita ActionRow text block).
fn title_block(title: &str, subtitle: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_hexpand(true);
    b.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_xalign(0.0);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&t);
    if !subtitle.is_empty() {
        let s = gtk::Label::new(Some(subtitle));
        s.set_css_classes(&["label-small", "dim-label"]);
        s.set_xalign(0.0);
        s.set_wrap(true);
        s.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        b.append(&s);
    }
    b
}

/// A settings row: title + subtitle, then `trailing`.
fn setting_row(
    title: &str,
    subtitle: &gtk::Label,
    trailing: &impl gtk::prelude::IsA<gtk::Widget>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
    texts.set_hexpand(true);
    texts.set_valign(gtk::Align::Center);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_xalign(0.0);
    texts.append(&t);
    subtitle.set_css_classes(&["label-small", "dim-label"]);
    subtitle.set_xalign(0.0);
    subtitle.set_wrap(true);
    texts.append(subtitle);
    row.append(&texts);
    trailing.set_valign(gtk::Align::Center);
    row.append(trailing);
    row
}

fn section_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("ufw-section-label");
    l.set_halign(gtk::Align::Start);
    l.set_hexpand(true);
    l
}

/// Build the menu's widgets into `root`.
fn build(root: &gtk::Box, sender: &ComponentSender<UfwMenuWidgetModel>) -> Ui {
    use UfwMenuWidgetInput as In;
    let s = sender.clone();
    let header = PanelHeader::new("firewall-symbolic", MenuWidget::Ufw.display_name())
        .with_subtitle()
        .action("view-refresh-symbolic", "Refresh", move || {
            s.input(In::Refresh)
        });
    let subtitle = header.subtitle().cloned().unwrap_or_default();
    let switch = gtk::Switch::new();
    switch.set_valign(gtk::Align::Center);
    mshell_common::a11y::name(&switch, "Firewall on");
    let s = sender.clone();
    let switch_handler = switch.connect_state_set(move |_, on| {
        s.input(In::SetEnabled(on));
        glib::Propagation::Stop
    });
    header.root().append(&switch);
    root.append(header.root());

    // Banner.
    let banner = gtk::Box::new(gtk::Orientation::Vertical, 8);
    banner.add_css_class("ufw-banner");
    banner.set_visible(false);
    let banner_text = gtk::Label::new(None);
    banner_text.set_css_classes(&["label-small"]);
    banner_text.set_xalign(0.0);
    banner_text.set_wrap(true);
    banner.append(&banner_text);
    let banner_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    banner.append(&banner_actions);
    root.append(&banner);

    // Settings rows.
    let settings = gtk::Box::new(gtk::Orientation::Vertical, 0);
    settings.set_css_classes(&["ok-boxed-list", "compact"]);
    let boot_sub = gtk::Label::new(Some(
        "Load the rules when the computer starts (ufw.service)",
    ));
    let boot_switch = gtk::Switch::new();
    mshell_common::a11y::name(&boot_switch, "Start at boot");
    let s = sender.clone();
    let boot_handler = boot_switch.connect_state_set(move |_, on| {
        s.input(In::SetBoot(on));
        glib::Propagation::Stop
    });
    let boot_row = setting_row("Start at boot", &boot_sub, &boot_switch);
    settings.append(&boot_row);
    let policy = gtk::Label::new(Some("—"));
    let policy_row = setting_row(
        "Default policy",
        &policy,
        &gtk::Box::new(gtk::Orientation::Horizontal, 0),
    );
    settings.append(&policy_row);
    root.append(&settings);

    // Inline confirm.
    let confirm = gtk::Box::new(gtk::Orientation::Vertical, 8);
    confirm.add_css_class("ufw-confirm");
    confirm.set_visible(false);
    let confirm_text = gtk::Label::new(None);
    confirm_text.add_css_class("label-medium-bold");
    confirm_text.set_xalign(0.0);
    confirm_text.set_wrap(true);
    confirm.append(&confirm_text);
    let confirm_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    confirm_row.set_homogeneous(true);
    let cancel = gtk::Button::with_label("Cancel");
    cancel.set_css_classes(&["ok-button-surface", "ok-button-cell"]);
    let s = sender.clone();
    cancel.connect_clicked(move |_| s.input(In::ConfirmNo));
    let confirm_ok = gtk::Button::with_label("Remove");
    confirm_ok.set_css_classes(&["ok-button-error", "ok-button-cell"]);
    let s = sender.clone();
    confirm_ok.connect_clicked(move |_| s.input(In::ConfirmYes));
    confirm_row.append(&cancel);
    confirm_row.append(&confirm_ok);
    confirm.append(&confirm_row);
    root.append(&confirm);

    // Rules header: title + "+".
    let rules_head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let rules_title = section_label("Rules");
    rules_head.append(&rules_title);
    let add_toggle = gtk::Button::from_icon_name("list-add-symbolic");
    add_toggle.add_css_class("ok-button-flat");
    mshell_common::a11y::name(&add_toggle, "Add a rule");
    let s = sender.clone();
    add_toggle.connect_clicked(move |_| s.input(In::ToggleForm));
    rules_head.append(&add_toggle);
    root.append(&rules_head);

    // Add form (revealed by "+").
    let (s1, s2, s3, s4, s5) = (
        sender.clone(),
        sender.clone(),
        sender.clone(),
        sender.clone(),
        sender.clone(),
    );
    let form = FormUi::new(
        move |i| s1.input(In::SetAction(i)),
        move |i| s2.input(In::SetProto(i)),
        move || s3.input(In::FormChanged),
        move |i| s4.input(In::Preset(i)),
        move || s5.input(In::Add),
    );
    let form_reveal = gtk::Revealer::new();
    form_reveal.set_transition_type(gtk::RevealerTransitionType::SlideDown);
    form_reveal.set_child(Some(&form.root));
    root.append(&form_reveal);

    // Rule list.
    let list = gtk::ListBox::new();
    list.set_css_classes(&["ufw-rule-list", "ok-boxed-list", "compact"]);
    list.set_selection_mode(gtk::SelectionMode::None);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroller.set_min_content_height(0);
    scroller.set_propagate_natural_height(true);
    scroller.set_child(Some(&list));
    crate::menus::list_fit::mark_menu_list(&scroller, LIST_CAP);
    root.append(&scroller);

    // Footer.
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    footer.add_css_class("ufw-footer-row");
    footer.set_margin_top(16);
    footer.set_homogeneous(true);
    let refresh = gtk::Button::with_label("Refresh");
    refresh.set_css_classes(&["ok-button-surface", "ok-button-cell"]);
    let s = sender.clone();
    refresh.connect_clicked(move |_| s.input(In::Refresh));
    footer.append(&refresh);
    let man = gtk::Button::with_label("ufw(8)");
    man.set_css_classes(&["ok-button-surface", "ok-button-cell"]);
    man.connect_clicked(|_| {
        let url = "https://manpages.ubuntu.com/manpages/noble/man8/ufw.8.html";
        if let Err(e) = mshell_process::desktop::open(url) {
            tracing::warn!(error = %e, url, "ufw: open failed");
        }
    });
    footer.append(&man);
    root.append(&footer);

    Ui {
        subtitle,
        switch,
        switch_handler,
        banner,
        banner_text,
        banner_actions,
        boot_row,
        boot_switch,
        boot_handler,
        policy,
        confirm,
        confirm_text,
        confirm_ok,
        rules_title,
        add_toggle,
        form_reveal,
        form,
        list,
    }
}
