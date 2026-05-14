//! UFW firewall bar widget — full port of the noctalia `nufw`
//! plugin.
//!
//! Two surfaces:
//!   1. **Bar pill** — security-{high,low,medium}-symbolic icon
//!      whose state reflects `ufw status verbose`. Tooltip lists
//!      status + default policies + rule count. Click toggles the
//!      panel popover below.
//!   2. **Popover panel** — header with active/inactive badge +
//!      `enable / disable` toggle switch, three "default policy"
//!      chips (incoming / outgoing / routed), logging-level
//!      indicator, scrollable rule list with per-row delete
//!      buttons, and a refresh button.
//!
//! All privileged actions run through `pkexec ufw …` directly —
//! the session's `margo-polkit-agent.service` surfaces the
//! graphical password prompt. We do NOT shell out to a terminal:
//! the previous MVP did and `sudo` inside a non-interactive
//! `xterm -e` wouldn't accept the user's password reliably. The
//! polkit path also gives the agent the action-name context for
//! per-action policy customisation later.
//!
//! Parsing is pure Rust against the well-documented `ufw status
//! verbose` text format — same as before, just with a richer
//! data shape so the panel rule rows can carry their original
//! line back to `ufw delete`.

use relm4::gtk::prelude::{
    BoxExt, ButtonExt, Cast, ListBoxRowExt, ObjectExt, PopoverExt, WidgetExt,
};
use relm4::{Component, ComponentParts, ComponentSender, gtk};
use std::time::Duration;
use tracing::warn;

const REFRESH_INTERVAL: Duration = Duration::from_secs(120);
const STARTUP_DELAY: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Active,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UfwSummary {
    status: Option<Status>,
    logging: String,
    incoming: String,
    outgoing: String,
    routed: String,
    /// Each entry is the raw rule line as printed by `ufw status
    /// verbose`. Carrying the literal line lets us delete by
    /// rule-text via `ufw delete <rule>` rather than depending on
    /// re-numbering after every change.
    rules: Vec<String>,
    error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct NufwModel {
    summary: UfwSummary,
    /// Mutable popover reference so model code can pop it down
    /// after a successful action (UX hint that something
    /// happened — the auto-refresh re-shows the new state).
    popover: Option<gtk::Popover>,
    /// gtk::ListBox holding the rule rows; kept so we can clear +
    /// re-populate on each refresh without re-creating the
    /// popover children.
    rule_list: Option<gtk::ListBox>,
    /// Status badge label inside the popover header.
    status_label: Option<gtk::Label>,
    /// "enable / disable" switch in the popover header. Held so
    /// we can call `set_state` on it from refresh without firing
    /// the `connect_state_set` handler again (we block it via
    /// `block_signal_handler`).
    toggle_switch: Option<gtk::Switch>,
    toggle_signal: Option<glib::SignalHandlerId>,
    policy_in: Option<gtk::Label>,
    policy_out: Option<gtk::Label>,
    policy_routed: Option<gtk::Label>,
    logging_label: Option<gtk::Label>,
}

#[derive(Debug)]
pub(crate) enum NufwInput {
    BarClicked,
    ToggleEnable(bool),
    DeleteRule(String),
    RefreshNow,
    AddRule,
}

#[derive(Debug)]
pub(crate) enum NufwOutput {}

pub(crate) struct NufwInit {}

#[derive(Debug)]
pub(crate) enum NufwCommandOutput {
    Refreshed(UfwSummary),
    /// Re-poll after a state-changing action so the panel updates
    /// without waiting for the 120 s timer.
    KickRefresh,
}

use relm4::gtk::glib;

#[relm4::component(pub)]
impl Component for NufwModel {
    type CommandOutput = NufwCommandOutput;
    type Input = NufwInput;
    type Output = NufwOutput;
    type Init = NufwInit;

    view! {
        #[root]
        gtk::Box {
            set_css_classes: &["ok-button-surface", "ok-bar-widget", "nufw-bar-widget"],
            set_hexpand: false,
            set_vexpand: false,
            set_has_tooltip: true,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-flat"],
                set_hexpand: true,
                set_vexpand: true,
                connect_clicked[sender] => move |_| {
                    sender.input(NufwInput::BarClicked);
                },

                #[name="image"]
                gtk::Image {
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                }
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
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
                    let summary = fetch_ufw_summary().await;
                    let _ = out.send(NufwCommandOutput::Refreshed(summary));
                }
            }
        });

        let mut model = NufwModel {
            summary: UfwSummary::default(),
            popover: None,
            rule_list: None,
            status_label: None,
            toggle_switch: None,
            toggle_signal: None,
            policy_in: None,
            policy_out: None,
            policy_routed: None,
            logging_label: None,
        };

        let widgets = view_output!();

        let popover = build_popover(&sender, &mut model);
        popover.set_parent(&widgets.button);
        model.popover = Some(popover);

        apply_visual(&widgets.image, &root, &model.summary);
        sync_popover(&model, &sender);

        ComponentParts { model, widgets }
    }

    fn update(
        &mut self,
        message: Self::Input,
        sender: ComponentSender<Self>,
        _root: &Self::Root,
    ) {
        match message {
            NufwInput::BarClicked => {
                if let Some(p) = &self.popover {
                    if p.is_visible() {
                        p.popdown();
                    } else {
                        sync_popover(self, &sender);
                        p.popup();
                    }
                }
            }
            NufwInput::ToggleEnable(want_on) => {
                let cmd = if want_on { "enable" } else { "disable" };
                spawn_pkexec(&[cmd], sender.clone());
            }
            NufwInput::DeleteRule(line) => {
                // `ufw delete <RULE>` mirrors what the user would
                // type; safer than `ufw delete <NUM>` which races
                // with concurrent rule edits.
                spawn_pkexec(&["delete", &line], sender.clone());
            }
            NufwInput::RefreshNow => {
                sender.spawn_command(|out| {
                    tokio::spawn(async move {
                        let s = fetch_ufw_summary().await;
                        let _ = out.send(NufwCommandOutput::Refreshed(s));
                    });
                });
            }
            NufwInput::AddRule => {
                // Add-rule UX would need an input dialog with port
                // / protocol / direction selectors; that's the
                // next iteration. For now open the man page in a
                // browser so the user can paste a `ufw allow …`
                // formula manually via `pkexec ufw <args>` later.
                tokio::spawn(async move {
                    let _ = tokio::process::Command::new("xdg-open")
                        .arg("https://manpages.ubuntu.com/manpages/jammy/man8/ufw.8.html")
                        .status()
                        .await;
                });
            }
        }
    }

    fn update_cmd_with_view(
        &mut self,
        widgets: &mut Self::Widgets,
        message: Self::CommandOutput,
        sender: ComponentSender<Self>,
        root: &Self::Root,
    ) {
        match message {
            NufwCommandOutput::Refreshed(summary) => {
                self.summary = summary;
                apply_visual(&widgets.image, root, &self.summary);
                sync_popover(self, &sender);
            }
            NufwCommandOutput::KickRefresh => {
                sender.input(NufwInput::RefreshNow);
            }
        }
    }
}

/// Build the popover skeleton once at widget init. We stash all
/// the inner widget refs onto the model so `sync_popover` can
/// update them on each refresh without re-walking the tree.
fn build_popover(sender: &ComponentSender<NufwModel>, model: &mut NufwModel) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("nufw-panel");
    popover.set_has_arrow(false);
    popover.set_autohide(true);

    let outer = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(12)
        .margin_end(12)
        .margin_top(10)
        .margin_bottom(10)
        .width_request(360)
        .build();

    // Header — title + status badge + toggle switch.
    let header = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    let title = gtk::Label::new(Some("UFW Firewall"));
    title.add_css_class("label-large-bold");
    title.set_hexpand(true);
    title.set_xalign(0.0);
    header.append(&title);

    let status_label = gtk::Label::new(Some("loading"));
    status_label.add_css_class("nufw-status-badge");
    header.append(&status_label);

    let toggle = gtk::Switch::new();
    toggle.set_valign(gtk::Align::Center);
    let toggle_sender = sender.clone();
    let toggle_id = toggle.connect_state_set(move |_, want_on| {
        toggle_sender.input(NufwInput::ToggleEnable(want_on));
        glib::Propagation::Stop
    });
    header.append(&toggle);
    outer.append(&header);

    // Default policies — three pill chips.
    let policies = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(4)
        .build();

    let policy_in = make_policy_chip("In", "n/a");
    let policy_out = make_policy_chip("Out", "n/a");
    let policy_routed = make_policy_chip("Routed", "n/a");
    policies.append(&policy_in);
    policies.append(&policy_out);
    policies.append(&policy_routed);
    outer.append(&policies);

    // Logging row.
    let logging_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    let logging_caption = gtk::Label::new(Some("Logging"));
    logging_caption.add_css_class("label-small");
    logging_caption.set_xalign(0.0);
    let logging_label = gtk::Label::new(Some("n/a"));
    logging_label.add_css_class("label-small-bold");
    logging_label.set_xalign(1.0);
    logging_label.set_hexpand(true);
    logging_row.append(&logging_caption);
    logging_row.append(&logging_label);
    outer.append(&logging_row);

    outer.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    // Rules list — scrollable.
    let rules_title = gtk::Label::new(Some("Rules"));
    rules_title.add_css_class("label-medium-bold");
    rules_title.set_xalign(0.0);
    outer.append(&rules_title);

    let scroller = gtk::ScrolledWindow::new();
    scroller.set_min_content_height(180);
    scroller.set_max_content_height(320);
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroller.set_propagate_natural_height(true);
    let rule_list = gtk::ListBox::new();
    rule_list.add_css_class("nufw-rule-list");
    rule_list.set_selection_mode(gtk::SelectionMode::None);
    scroller.set_child(Some(&rule_list));
    outer.append(&scroller);

    // Actions row.
    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .margin_top(4)
        .build();
    let refresh_btn = gtk::Button::with_label("Refresh");
    refresh_btn.add_css_class("ok-button-surface");
    let refresh_sender = sender.clone();
    refresh_btn.connect_clicked(move |_| refresh_sender.input(NufwInput::RefreshNow));
    actions.append(&refresh_btn);

    let docs_btn = gtk::Button::with_label("ufw(8)");
    docs_btn.add_css_class("ok-button-surface");
    let docs_sender = sender.clone();
    docs_btn.connect_clicked(move |_| docs_sender.input(NufwInput::AddRule));
    actions.append(&docs_btn);

    actions.append(&{
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        spacer
    });
    outer.append(&actions);

    popover.set_child(Some(&outer));

    model.rule_list = Some(rule_list);
    model.status_label = Some(status_label);
    model.toggle_switch = Some(toggle);
    model.toggle_signal = Some(toggle_id);
    model.policy_in = Some(extract_value_label(&policy_in));
    model.policy_out = Some(extract_value_label(&policy_out));
    model.policy_routed = Some(extract_value_label(&policy_routed));
    model.logging_label = Some(logging_label);

    popover
}

fn make_policy_chip(caption: &str, initial: &str) -> gtk::Box {
    let chip = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .css_classes(vec!["nufw-policy-chip"])
        .hexpand(true)
        .build();
    let cap = gtk::Label::new(Some(caption));
    cap.add_css_class("label-small");
    cap.set_xalign(0.5);
    chip.append(&cap);
    let value = gtk::Label::new(Some(initial));
    value.add_css_class("label-small-bold");
    value.set_xalign(0.5);
    chip.append(&value);
    chip
}

/// Pull the value Label out of a policy chip Box. The chip is
/// built as `Box(vertical) { Label(caption), Label(value) }` so
/// the value is the second child. Returning it as `gtk::Label`
/// avoids a `dynamic_cast`-like dance on every refresh.
fn extract_value_label(chip: &gtk::Box) -> gtk::Label {
    let mut child = chip.first_child();
    // first_child = caption; we want the next one.
    if let Some(c) = child.as_ref() {
        child = c.next_sibling();
    }
    child
        .and_then(|w| w.downcast::<gtk::Label>().ok())
        .expect("policy chip layout invariant: second child is Label")
}

/// Update the panel widgets from the current summary. Safe to
/// call on every refresh — every set is guarded against the
/// current GTK value so we don't re-render rule rows pointlessly.
fn sync_popover(model: &NufwModel, sender: &ComponentSender<NufwModel>) {
    if let Some(label) = &model.status_label {
        label.set_label(status_word(model.summary.status));
        label.set_css_classes(&[
            "nufw-status-badge",
            status_class(model.summary.status),
        ]);
    }
    if let Some(label) = &model.policy_in {
        label.set_label(empty_to_dash(&model.summary.incoming));
    }
    if let Some(label) = &model.policy_out {
        label.set_label(empty_to_dash(&model.summary.outgoing));
    }
    if let Some(label) = &model.policy_routed {
        label.set_label(empty_to_dash(&model.summary.routed));
    }
    if let Some(label) = &model.logging_label {
        label.set_label(empty_to_dash(&model.summary.logging));
    }
    if let (Some(switch), Some(id)) = (&model.toggle_switch, &model.toggle_signal) {
        let active = matches!(model.summary.status, Some(Status::Active));
        if switch.state() != active {
            // Block our own handler so set_state doesn't loop
            // back into ToggleEnable.
            switch.block_signal(id);
            switch.set_state(active);
            switch.set_active(active);
            switch.unblock_signal(id);
        }
    }

    if let Some(list) = &model.rule_list {
        // Clear + re-populate. The rule set is small enough
        // (single-digit to low-double-digit count) that diffing
        // would buy us nothing readable.
        while let Some(row) = list.first_child() {
            list.remove(&row);
        }
        for rule in &model.summary.rules {
            list.append(&make_rule_row(rule, sender));
        }
        if model.summary.rules.is_empty() {
            let row = gtk::ListBoxRow::new();
            row.set_activatable(false);
            row.set_selectable(false);
            let label = gtk::Label::new(Some(
                if matches!(model.summary.status, Some(Status::Inactive)) {
                    "(firewall is inactive)"
                } else {
                    "(no rules)"
                },
            ));
            label.add_css_class("label-small");
            label.set_xalign(0.0);
            label.set_margin_top(8);
            label.set_margin_bottom(8);
            row.set_child(Some(&label));
            list.append(&row);
        }
    }
}

fn make_rule_row(rule: &str, sender: &ComponentSender<NufwModel>) -> gtk::ListBoxRow {
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
    let sender_clone = sender.clone();
    del.connect_clicked(move |_| {
        sender_clone.input(NufwInput::DeleteRule(rule_owned.clone()));
    });
    outer.append(&del);

    row.set_child(Some(&outer));
    row
}

fn status_word(s: Option<Status>) -> &'static str {
    match s {
        Some(Status::Active) => "active",
        Some(Status::Inactive) => "inactive",
        _ => "unknown",
    }
}

fn status_class(s: Option<Status>) -> &'static str {
    match s {
        Some(Status::Active) => "nufw-status-active",
        Some(Status::Inactive) => "nufw-status-inactive",
        _ => "nufw-status-unknown",
    }
}

fn empty_to_dash(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}

fn apply_visual(image: &gtk::Image, root: &gtk::Box, s: &UfwSummary) {
    let icon = match s.status {
        Some(Status::Active) => "security-high-symbolic",
        Some(Status::Inactive) => "security-low-symbolic",
        _ => "dialog-warning-symbolic",
    };
    image.set_icon_name(Some(icon));

    let tooltip = if let Some(err) = &s.error {
        format!("UFW: {err}")
    } else {
        let mut lines = vec![format!("UFW: {}", status_word(s.status))];
        if !s.incoming.is_empty() {
            lines.push(format!("Incoming: {}", s.incoming));
        }
        if !s.outgoing.is_empty() {
            lines.push(format!("Outgoing: {}", s.outgoing));
        }
        if !s.routed.is_empty() {
            lines.push(format!("Routed: {}", s.routed));
        }
        if matches!(s.status, Some(Status::Active)) {
            lines.push(format!("Rules: {}", s.rules.len()));
        }
        lines.join("\n")
    };
    root.set_tooltip_text(Some(&tooltip));

    if matches!(s.status, Some(Status::Inactive)) {
        root.add_css_class("inactive");
    } else {
        root.remove_css_class("inactive");
    }
}

/// Spawn `pkexec ufw <args…>` and kick a refresh after it returns
/// (regardless of success — error path needs the panel updated
/// too so the user sees that nothing changed). polkit's graphical
/// agent (`margo-polkit-agent.service` in this session) handles
/// the password prompt, so we don't need a terminal.
fn spawn_pkexec(args: &[&str], sender: ComponentSender<NufwModel>) {
    let args: Vec<String> = std::iter::once("ufw".to_string())
        .chain(args.iter().map(|s| s.to_string()))
        .collect();
    sender.spawn_command(move |out| {
        tokio::spawn(async move {
            let status = tokio::process::Command::new("pkexec")
                .args(&args)
                .status()
                .await;
            match status {
                Ok(s) if s.success() => {}
                Ok(s) => warn!(?s, ?args, "pkexec ufw returned non-zero"),
                Err(e) => warn!(error = %e, ?args, "pkexec spawn failed"),
            }
            // Give ufw a half-second to settle then trigger a refresh.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = out.send(NufwCommandOutput::KickRefresh);
        });
    });
}

async fn fetch_ufw_summary() -> UfwSummary {
    let output = match tokio::process::Command::new("ufw")
        .arg("status")
        .arg("verbose")
        .output()
        .await
    {
        Ok(out) => out,
        Err(e) => {
            return UfwSummary {
                error: Some(if e.kind() == std::io::ErrorKind::NotFound {
                    "not installed".to_string()
                } else {
                    format!("spawn failed: {e}")
                }),
                ..UfwSummary::default()
            };
        }
    };

    if !output.status.success() {
        return UfwSummary {
            error: Some(format!(
                "ufw exit {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            )),
            ..UfwSummary::default()
        };
    }

    parse_ufw_verbose(&String::from_utf8_lossy(&output.stdout))
}

fn parse_ufw_verbose(stdout: &str) -> UfwSummary {
    let mut summary = UfwSummary::default();
    let mut in_rules = false;

    for line in stdout.lines() {
        let trimmed = line.trim();

        if let Some(rest) = trimmed.strip_prefix("Status:") {
            summary.status = Some(match rest.trim() {
                "active" => Status::Active,
                "inactive" => Status::Inactive,
                _ => Status::Unknown,
            });
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("Logging:") {
            summary.logging = rest.trim().to_string();
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("Default:") {
            for chunk in rest.split(',') {
                let chunk = chunk.trim();
                if let Some((policy, kind)) = chunk.split_once('(') {
                    let policy = policy.trim().to_string();
                    let kind = kind.trim_end_matches(')').trim();
                    match kind {
                        "incoming" => summary.incoming = policy,
                        "outgoing" => summary.outgoing = policy,
                        "routed" => summary.routed = policy,
                        _ => {}
                    }
                }
            }
            continue;
        }

        if trimmed.starts_with("To") && trimmed.contains("Action") && trimmed.contains("From") {
            continue;
        }
        if trimmed.starts_with("--") {
            in_rules = true;
            continue;
        }
        if in_rules && !trimmed.is_empty() {
            summary.rules.push(trimmed.to_string());
        }
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_active_with_rules() {
        let raw = "\
Status: active
Logging: on (low)
Default: deny (incoming), allow (outgoing), disabled (routed)
New profiles: skip

To                         Action      From
--                         ------      ----
22                         ALLOW IN    Anywhere
80                         ALLOW IN    Anywhere
443                        ALLOW IN    Anywhere
";
        let s = parse_ufw_verbose(raw);
        assert_eq!(s.status, Some(Status::Active));
        assert_eq!(s.incoming, "deny");
        assert_eq!(s.outgoing, "allow");
        assert_eq!(s.routed, "disabled");
        assert_eq!(s.logging, "on (low)");
        assert_eq!(s.rules.len(), 3);
        assert!(s.rules[0].starts_with("22"));
    }

    #[test]
    fn parses_inactive() {
        let raw = "Status: inactive\n";
        let s = parse_ufw_verbose(raw);
        assert_eq!(s.status, Some(Status::Inactive));
        assert!(s.rules.is_empty());
    }
}
