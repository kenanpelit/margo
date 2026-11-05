//! The VPN menu's error header: what is wrong with Mullvad in one plain
//! sentence, with a button per fix (`mvpn health --json`, diagnosed by
//! `mvpn_proto::health::diagnose`). Hidden while all is well.

use mshell_utils::mvpn::health::{Fix, Problem, Severity};
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, WidgetExt};
use std::rc::Rc;

/// The banner's widgets, refilled in place by [`ProblemBanner::show`].
pub(crate) struct ProblemBanner {
    root: gtk::Box,
    icon: gtk::Image,
    message: gtk::Label,
    fixes: gtk::Box,
    /// The problem on screen, so an unchanged one rebuilds nothing.
    shown: Option<Problem>,
}

impl ProblemBanner {
    pub(crate) fn new() -> Self {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root.add_css_class("vpn-problem");
        root.set_visible(false);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let icon = gtk::Image::new();
        icon.add_css_class("vpn-problem-icon");
        icon.set_valign(gtk::Align::Start);
        let message = gtk::Label::new(None);
        message.add_css_class("vpn-problem-message");
        message.set_xalign(0.0);
        message.set_hexpand(true);
        message.set_wrap(true);
        message.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        head.append(&icon);
        head.append(&message);
        let fixes = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        fixes.add_css_class("vpn-problem-fixes");
        root.append(&head);
        root.append(&fixes);
        Self {
            root,
            icon,
            message,
            fixes,
            shown: None,
        }
    }

    pub(crate) fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Show `problem` (or hide the banner for `None`); a fix click calls `on_fix`.
    pub(crate) fn show(&mut self, problem: Option<Problem>, on_fix: Rc<dyn Fn(Fix)>) {
        if self.shown == problem {
            return;
        }
        let Some(p) = &problem else {
            self.root.set_visible(false);
            self.shown = None;
            return;
        };
        for class in ["calm", "warn", "danger"] {
            self.root.remove_css_class(class);
        }
        self.root.add_css_class(severity_class(p.severity));
        self.icon.set_icon_name(Some(icon_for(p.severity)));
        self.message.set_label(&p.message);
        while let Some(c) = self.fixes.first_child() {
            self.fixes.remove(&c);
        }
        for &fix in &p.fixes {
            let b = gtk::Button::with_label(fix.label());
            b.set_css_classes(&["ok-button-surface", "dns-action"]);
            let on_fix = on_fix.clone();
            b.connect_clicked(move |_| on_fix(fix));
            self.fixes.append(&b);
        }
        self.fixes.set_visible(!p.fixes.is_empty());
        self.root.set_visible(true);
        self.shown = problem;
    }
}

/// The DESIGN.md §2 class for a severity.
pub(crate) fn severity_class(s: Severity) -> &'static str {
    match s {
        Severity::Calm => "calm",
        Severity::Warn => "warn",
        Severity::Danger => "danger",
    }
}

fn icon_for(s: Severity) -> &'static str {
    match s {
        Severity::Calm => "dialog-information-symbolic",
        Severity::Warn => "dialog-warning-symbolic",
        Severity::Danger => "dialog-error-symbolic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_maps_to_the_shared_ladder() {
        assert_eq!(severity_class(Severity::Calm), "calm");
        assert_eq!(severity_class(Severity::Warn), "warn");
        assert_eq!(severity_class(Severity::Danger), "danger");
    }
}
