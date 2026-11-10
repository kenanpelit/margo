//! The UFW menu's add-rule form: action and protocol pickers (the shared
//! segmented control), port / source / comment entries, preset buttons
//! and the Add button. Validation is the helper's own
//! ([`AddForm::validate`]), run on every keystroke, so the form never
//! offers a prompt root would refuse; the problem shows inline under the
//! form, naming the field.

use crate::common_widgets::segmented::{Segmented, SegmentedStyle};
use mshell_privileged::ufw::rules::RuleSet;
use mshell_privileged::ufw::spec::{
    Action, AddForm, AddRule, Addr, FormError, PRESETS, Ports, Proto, RuleSpec,
};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, Cast, EditableExt, EntryExt, WidgetExt};
use std::rc::Rc;

/// The protocol segments, in order.
pub(super) const PROTOS: [Proto; 3] = [Proto::Tcp, Proto::Udp, Proto::Any];
const PROTO_LABELS: [&str; 3] = ["TCP", "UDP", "Both"];

/// What the form's message line says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Verdict {
    /// Nothing typed yet: no message, Add off.
    Empty,
    /// Ready: a dim preview of what Add does.
    Ready(AddRule, String),
    /// A problem: shown in the danger tone, Add off.
    Problem(String),
}

/// `0.0.0.0/0` / `::/0` read as anywhere (a one-family rule's delete
/// spec names them; an add says `any`).
fn loose(a: Addr) -> Addr {
    match a {
        Addr::Net(_, 0) => Addr::Any,
        a => a,
    }
}

fn same(a: &RuleSpec, b: &RuleSpec) -> bool {
    a.action == b.action
        && a.direction == b.direction
        && a.iface == b.iface
        && a.proto == b.proto
        && loose(a.from) == loose(b.from)
        && loose(a.to) == loose(b.to)
        && a.ports == b.ports
}

/// Whether every rule `add` would create is already listed (a "Both"
/// range is two rules: it exists when both do).
pub(super) fn already_listed(set: &RuleSet, add: &AddRule) -> bool {
    let multi = add.spec.ports.as_ref().is_some_and(Ports::is_multi);
    let wanted: Vec<RuleSpec> = if add.spec.proto == Proto::Any && multi {
        [Proto::Tcp, Proto::Udp]
            .into_iter()
            .map(|p| RuleSpec {
                proto: p,
                ..add.spec.clone()
            })
            .collect()
    } else {
        vec![add.spec.clone()]
    };
    let have: Vec<RuleSpec> = set
        .rules
        .iter()
        .filter_map(|r| r.delete_spec(true))
        .collect();
    wanted.iter().all(|w| have.iter().any(|h| same(h, w)))
}

/// `Adds: allow 22/tcp from anywhere`, `… (a TCP and a UDP rule)`.
pub(super) fn preview(add: &AddRule) -> String {
    let s = &add.spec;
    let from = match s.from {
        Addr::Any => "anywhere".to_string(),
        a => a.to_string(),
    };
    let two = s.proto == Proto::Any && s.ports.as_ref().is_some_and(Ports::is_multi);
    let target = if two {
        format!(
            "{}/tcp + udp",
            s.ports.as_ref().map_or_else(String::new, Ports::to_string)
        )
    } else {
        s.port_label()
    };
    format!("Adds: {} incoming {target} from {from}", s.action.keyword())
}

/// The verdict for the typed form against the listed rules.
pub(super) fn verdict(form: &AddForm, listed: Option<&RuleSet>) -> Verdict {
    if form.port.trim().is_empty() && form.from.trim().is_empty() && form.comment.trim().is_empty()
    {
        return Verdict::Empty;
    }
    match form.validate() {
        Err(FormError { field, message }) => {
            let what = match field {
                mshell_privileged::ufw::spec::Field::Port => "Port",
                mshell_privileged::ufw::spec::Field::From => "From",
                mshell_privileged::ufw::spec::Field::Comment => "Comment",
            };
            Verdict::Problem(format!("{what}: {message}"))
        }
        Ok(add) if listed.is_some_and(|set| already_listed(set, &add)) => {
            Verdict::Problem("That rule is already in the list".into())
        }
        Ok(add) => {
            let p = preview(&add);
            Verdict::Ready(add, p)
        }
    }
}

/// The form's widgets.
pub(super) struct FormUi {
    pub root: gtk::Box,
    action: Segmented,
    proto: Segmented,
    pub port: gtk::Entry,
    pub from: gtk::Entry,
    pub comment: gtk::Entry,
    message: gtk::Label,
    pub add: gtk::Button,
}

fn entry(placeholder: &str, name: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.add_css_class("menu-search");
    e.set_placeholder_text(Some(placeholder));
    e.set_hexpand(true);
    mshell_common::a11y::name(&e, name);
    e
}

fn caption(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(&["label-small", "dim-label"]);
    l.set_xalign(0.0);
    l
}

impl FormUi {
    /// `on_action(i)` / `on_proto(i)` fire on a segment click,
    /// `on_change` on every edit, `on_preset(i)` on a preset button,
    /// `on_add` on Add or Enter in an entry.
    pub(super) fn new(
        on_action: impl Fn(usize) + 'static,
        on_proto: impl Fn(usize) + 'static,
        on_change: impl Fn() + 'static,
        on_preset: impl Fn(usize) + 'static,
        on_add: impl Fn() + 'static,
    ) -> FormUi {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("ufw-add-form");

        let labels: Vec<&str> = Action::ALL.iter().map(|a| a.label()).collect();
        let action = Segmented::new(&labels, 0, SegmentedStyle::ButtonRow, on_action);
        root.append(&caption("Incoming traffic"));
        root.append(action.root());

        let port = entry("Port: 22, 8000:8100 or 80,443", "Port");
        let proto = Segmented::new(&PROTO_LABELS, 2, SegmentedStyle::ButtonRow, on_proto);
        let port_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        port_row.append(&port);
        proto.root().set_hexpand(false);
        port_row.append(proto.root());
        root.append(&port_row);

        let from = entry("From: anywhere, or 192.168.1.0/24", "From address");
        root.append(&from);
        let comment = entry("Comment (optional)", "Comment");
        root.append(&comment);

        let presets = gtk::FlowBox::new();
        presets.add_css_class("ufw-presets");
        presets.set_selection_mode(gtk::SelectionMode::None);
        presets.set_max_children_per_line(6);
        presets.set_column_spacing(4);
        presets.set_row_spacing(4);
        let on_preset = Rc::new(on_preset);
        for (i, p) in PRESETS.iter().enumerate() {
            let b = gtk::Button::with_label(p.name);
            b.set_css_classes(&["ok-button-surface", "label-small"]);
            b.set_tooltip_text(Some(&format!(
                "Fill in {} ({}{})",
                p.name,
                p.port,
                match p.proto {
                    Proto::Any => String::new(),
                    pr => format!("/{}", pr.keyword()),
                }
            )));
            let f = on_preset.clone();
            b.connect_clicked(move |_| f(i));
            presets.insert(&b, -1);
        }
        root.append(&caption("Presets"));
        root.append(&presets);

        let foot = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let message = gtk::Label::new(None);
        message.set_xalign(0.0);
        message.set_hexpand(true);
        message.set_wrap(true);
        message.set_css_classes(&["label-small", "dim-label"]);
        foot.append(&message);
        let add = gtk::Button::with_label("Add");
        add.set_css_classes(&["ok-button-primary", "ok-button-cell"]);
        add.set_valign(gtk::Align::Center);
        add.set_sensitive(false);
        foot.append(&add);
        root.append(&foot);

        let on_change = Rc::new(on_change);
        for e in [&port, &from, &comment] {
            let f = on_change.clone();
            e.connect_changed(move |_| f());
        }
        let on_add = Rc::new(on_add);
        for e in [&port, &from, &comment] {
            let f = on_add.clone();
            e.connect_activate(move |_| f());
        }
        add.connect_clicked(move |_| on_add());

        FormUi {
            root,
            action,
            proto,
            port,
            from,
            comment,
            message,
            add,
        }
    }

    pub(super) fn read(&self, action: Action, proto: Proto) -> AddForm {
        AddForm {
            action,
            port: self.port.text().to_string(),
            proto,
            from: self.from.text().to_string(),
            comment: self.comment.text().to_string(),
        }
    }

    pub(super) fn set_action(&self, i: usize) {
        self.action.set_active(i);
    }

    pub(super) fn set_proto(&self, i: usize) {
        self.proto.set_active(i);
    }

    /// Show `v`; Add is on only for a ready form while `allowed`.
    pub(super) fn show(&self, v: &Verdict, allowed: bool) {
        let (text, problem) = match v {
            Verdict::Empty => ("", false),
            Verdict::Ready(_, p) => (p.as_str(), false),
            Verdict::Problem(p) => (p.as_str(), true),
        };
        if self.message.label() != text {
            self.message.set_label(text);
        }
        self.message.set_css_classes(if problem {
            &["label-small-error"]
        } else {
            &["label-small", "dim-label"]
        });
        self.add
            .set_sensitive(allowed && matches!(v, Verdict::Ready(..)));
    }

    pub(super) fn clear(&self) {
        self.port.set_text("");
        self.from.set_text("");
        self.comment.set_text("");
    }

    pub(super) fn set_sensitive(&self, on: bool) {
        for w in [
            self.action.root().upcast_ref::<gtk::Widget>(),
            self.proto.root().upcast_ref(),
            self.port.upcast_ref(),
            self.from.upcast_ref(),
            self.comment.upcast_ref(),
        ] {
            w.set_sensitive(on);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mshell_privileged::ufw::rules::parse_rules;

    const REAL_V4: &str =
        include_str!("../../../../../mshell-privileged/tests/data/ufw/user.rules");
    const REAL_V6: &str =
        include_str!("../../../../../mshell-privileged/tests/data/ufw/user6.rules");

    fn form(port: &str, proto: Proto, from: &str) -> AddForm {
        AddForm {
            action: Action::Allow,
            port: port.into(),
            proto,
            from: from.into(),
            comment: String::new(),
        }
    }

    #[test]
    fn listed_rules_are_caught_before_a_prompt() {
        let set = parse_rules(REAL_V4, Some(REAL_V6), true);
        let dup = |f: &AddForm| already_listed(&set, &f.validate().unwrap());
        assert!(dup(&form("22", Proto::Tcp, "")), "both-family 22/tcp");
        assert!(
            dup(&form("1714:1764", Proto::Any, "192.168.0.0/24")),
            "tcp + udp"
        );
        assert!(dup(&form("1714:1764", Proto::Udp, "192.168.0.0/24")));
        assert!(
            !dup(&form("1714:1764", Proto::Any, "")),
            "a different source"
        );
        assert!(!dup(&form("22", Proto::Udp, "")));
        assert!(!dup(&form("22", Proto::Any, "")), "any is its own rule");
        let mut deny = form("22", Proto::Tcp, "");
        deny.action = Action::Deny;
        assert!(!dup(&deny));
    }

    #[test]
    fn the_verdict_names_the_field_and_previews_the_rule() {
        let set = parse_rules(REAL_V4, Some(REAL_V6), true);
        assert_eq!(
            verdict(&form("", Proto::Tcp, ""), Some(&set)),
            Verdict::Empty
        );
        assert_eq!(
            verdict(&form("70000", Proto::Tcp, ""), Some(&set)),
            Verdict::Problem("Port: 70000 is out of range; ports are 1-65535".into())
        );
        assert!(matches!(
            verdict(&form("", Proto::Tcp, "lan"), Some(&set)),
            Verdict::Problem(p) if p.starts_with("Port: ")
        ));
        assert!(matches!(
            verdict(&form("80", Proto::Tcp, "lan"), Some(&set)),
            Verdict::Problem(p) if p.starts_with("From: ")
        ));
        assert_eq!(
            verdict(&form("22", Proto::Tcp, ""), Some(&set)),
            Verdict::Problem("That rule is already in the list".into())
        );
        match verdict(&form("8000-8100", Proto::Any, "10.0.0.0/8"), Some(&set)) {
            Verdict::Ready(add, p) => {
                assert_eq!(
                    p,
                    "Adds: allow incoming 8000:8100/tcp + udp from 10.0.0.0/8"
                );
                assert_eq!(add.spec.ports.unwrap().to_string(), "8000:8100");
            }
            v => panic!("{v:?}"),
        }
        match verdict(&form("443", Proto::Tcp, ""), None) {
            Verdict::Ready(_, p) => assert_eq!(p, "Adds: allow incoming 443/tcp from anywhere"),
            v => panic!("{v:?}"),
        }
    }
}
