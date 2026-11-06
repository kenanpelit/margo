//! The AI agents menu's Plugins page, read-only: installed Claude Code
//! plugins grouped by marketplace (version, scope, skill count, off /
//! update chips; click opens the plugin's page), then the bare skill
//! folders of `~/.claude/skills` (click opens the SKILL.md). Installing,
//! updating and removing stay with `claude plugin` / `/plugin`.

use crate::agents::plugins::{Inventory, Plugin, Skill};
use crate::agents::sessions;
use relm4::gtk;
use relm4::gtk::prelude::{BoxExt, ButtonExt, Cast, WidgetExt};
use std::rc::Rc;

/// The chips under a plugin. `(text, extra class)`.
pub(crate) fn plugin_chips(p: &Plugin) -> Vec<(String, &'static str)> {
    let mut v = Vec::new();
    if !p.account.is_empty() {
        v.push((p.account.clone(), ""));
    }
    if !p.version.is_empty() {
        v.push((format!("v{}", p.version), ""));
    }
    match (p.scope.as_str(), p.project.as_deref()) {
        ("user", _) => {}
        (scope, Some(project)) => v.push((format!("{scope} · {}", sessions::tilde(project)), "")),
        (scope, None) => v.push((scope.to_string(), "")),
    }
    match p.skills {
        0 => {}
        1 => v.push(("1 skill".into(), "")),
        n => v.push((format!("{n} skills"), "")),
    }
    if p.enabled == Some(false) {
        v.push(("off".into(), "warn"));
    }
    if let Some(new) = &p.update {
        v.push((format!("update {new}"), "accent"));
    }
    v
}

/// Rebuild the page (`None` = still reading). `on_open` gets a URL or a
/// file path.
pub(crate) fn rebuild_plugins(
    container: &gtk::Box,
    inv: Option<&Inventory>,
    on_open: Rc<dyn Fn(String)>,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let Some(inv) = inv else {
        container.append(&small("Loading…"));
        return;
    };
    if inv.plugins.is_empty() && inv.skills.is_empty() {
        container.append(&small(
            "No plugins or skills yet — /plugin in Claude Code installs them",
        ));
        return;
    }
    let mut start = 0;
    while start < inv.plugins.len() {
        let market = &inv.plugins[start].marketplace;
        let end = inv.plugins[start..]
            .iter()
            .position(|p| &p.marketplace != market)
            .map_or(inv.plugins.len(), |n| start + n);
        container.append(&section(market));
        let list = boxed_list();
        for p in &inv.plugins[start..end] {
            list.append(&plugin_row(p, on_open.clone()));
        }
        container.append(&list);
        start = end;
    }
    if !inv.skills.is_empty() {
        container.append(&section("Skills"));
        let list = boxed_list();
        for s in &inv.skills {
            list.append(&skill_row(s, on_open.clone()));
        }
        container.append(&list);
    }
    container.append(&small(
        "Read-only. Install, update or remove with /plugin in Claude Code.",
    ));
}

fn section(title: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(title));
    l.add_css_class("section-label-menu-widget");
    l.set_halign(gtk::Align::Start);
    l
}

fn boxed_list() -> gtk::Box {
    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    list.set_css_classes(&["ok-boxed-list", "compact", "agents-session-list"]);
    list
}

fn small(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_css_classes(&["label-small", "dim-label"]);
    l.set_halign(gtk::Align::Start);
    l.set_wrap(true);
    l
}

/// Title, one dim line, optional chips, and a trailing icon; a button
/// only when there is something to open.
fn row(
    title: &str,
    sub: &str,
    chips: &[(String, &'static str)],
    target: Option<String>,
    icon: &str,
    tooltip: &str,
    on_open: Rc<dyn Fn(String)>,
) -> gtk::Widget {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let block = gtk::Box::new(gtk::Orientation::Vertical, 2);
    block.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.set_css_classes(&["label-medium"]);
    t.set_halign(gtk::Align::Start);
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    block.append(&t);
    if !sub.is_empty() {
        let s = gtk::Label::new(Some(sub));
        s.set_css_classes(&["label-small", "dim-label"]);
        s.set_halign(gtk::Align::Start);
        s.set_ellipsize(gtk::pango::EllipsizeMode::End);
        s.set_tooltip_text(Some(sub));
        block.append(&s);
    }
    if !chips.is_empty() {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        line.add_css_class("agents-session-chips");
        for (text, class) in chips {
            let l = gtk::Label::new(Some(text));
            l.add_css_class("agents-chip");
            if !class.is_empty() {
                l.add_css_class(class);
            }
            line.append(&l);
        }
        block.append(&line);
    }
    row.append(&block);
    let Some(target) = target else {
        row.add_css_class("agents-session-row");
        return row.upcast();
    };
    let image = gtk::Image::from_icon_name(icon);
    image.set_valign(gtk::Align::Center);
    image.add_css_class("agents-session-go");
    row.append(&image);
    let button = gtk::Button::new();
    button.set_css_classes(&["agents-session-row"]);
    button.set_child(Some(&row));
    button.set_tooltip_text(Some(tooltip));
    button.connect_clicked(move |_| on_open(target.clone()));
    button.upcast()
}

fn plugin_row(p: &Plugin, on_open: Rc<dyn Fn(String)>) -> gtk::Widget {
    row(
        &p.name,
        &p.description,
        &plugin_chips(p),
        p.url.clone(),
        "web-browser-symbolic",
        "Open the plugin's page",
        on_open,
    )
}

fn skill_row(s: &Skill, on_open: Rc<dyn Fn(String)>) -> gtk::Widget {
    let chips = if s.account.is_empty() {
        Vec::new()
    } else {
        vec![(s.account.clone(), "")]
    };
    let is_file = s.path.extension().is_some_and(|e| e == "md");
    row(
        &s.name,
        &s.description,
        &chips,
        Some(s.path.to_string_lossy().into_owned()),
        if is_file {
            "document-edit-symbolic"
        } else {
            "folder-open-symbolic"
        },
        if is_file {
            "Open SKILL.md in your editor"
        } else {
            "Open the folder"
        },
        on_open,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin() -> Plugin {
        Plugin {
            name: "superpowers".into(),
            marketplace: "claude-plugins-official".into(),
            account: String::new(),
            version: "7.0.0".into(),
            scope: "user".into(),
            project: None,
            enabled: Some(true),
            update: None,
            skills: 14,
            description: String::new(),
            url: None,
        }
    }

    #[test]
    fn chips_say_version_skills_and_only_unusual_state() {
        let texts = |p: &Plugin| {
            plugin_chips(p)
                .into_iter()
                .map(|(t, c)| format!("{t}/{c}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(texts(&plugin()), ["v7.0.0/", "14 skills/"]);
        let p = Plugin {
            account: "work".into(),
            version: String::new(),
            scope: "local".into(),
            project: Some("/srv/x".into()),
            enabled: Some(false),
            update: Some("7.1.0".into()),
            skills: 1,
            ..plugin()
        };
        assert_eq!(
            texts(&p),
            [
                "work/",
                "local · /srv/x/",
                "1 skill/",
                "off/warn",
                "update 7.1.0/accent"
            ]
        );
        // Unknown enabled state shows nothing rather than a guess.
        let p = Plugin {
            enabled: None,
            skills: 0,
            scope: "project".into(),
            ..plugin()
        };
        assert_eq!(texts(&p), ["v7.0.0/", "project/"]);
    }
}
