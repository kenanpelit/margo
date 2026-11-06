//! Settings → Widgets → GitHub — what the bar pill counts (and whether it
//! hides at zero), and the PR tab's search rules + excluded repositories.
//! Composed with the generic per-menu geometry page.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GithubConfigStoreFields, GithubPillMetric};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::glib;
use relm4::gtk::prelude::{BoxExt, OrientableExt, TextBufferExt, TextViewExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct GithubSettingsModel {
    metric: u32,
    hide_when_zero: bool,
    rules: gtk::TextBuffer,
    excluded: gtk::TextBuffer,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum GithubSettingsInput {
    MetricChanged(u32),
    MetricEffect(GithubPillMetric),
    HideChanged(bool),
    HideEffect(bool),
    /// A list editor lost focus: write both lists.
    ListsCommitted,
    ListsEffect(Vec<String>, Vec<String>),
}

/// One entry per non-blank line, trimmed.
pub(crate) fn lines_to_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

fn buffer_text(b: &gtk::TextBuffer) -> String {
    b.text(&b.start_iter(), &b.end_iter(), false).to_string()
}

/// A titled multi-line editor row (one item per line), committed on
/// focus-out.
fn list_editor(
    title: &str,
    help: &str,
    buffer: &gtk::TextBuffer,
    sender: &ComponentSender<GithubSettingsModel>,
) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Vertical, 8);
    row.add_css_class("action-row");
    row.append(&row_text(title, help));
    let view = gtk::TextView::with_buffer(buffer);
    view.add_css_class("settings-textview");
    view.set_monospace(true);
    view.set_top_margin(6);
    view.set_bottom_margin(6);
    view.set_left_margin(8);
    view.set_right_margin(8);
    view.set_height_request(72);
    mshell_common::a11y::name(&view, title);
    let focus = gtk::EventControllerFocus::new();
    let s = sender.clone();
    focus.connect_leave(move |_| s.input(GithubSettingsInput::ListsCommitted));
    view.add_controller(focus);
    row.append(&view);
    row
}

#[derive(Debug)]
pub(crate) enum GithubSettingsOutput {}

pub(crate) struct GithubSettingsInit {}

pub(crate) fn row_text(title: &str, help: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.set_valign(gtk::Align::Center);
    b.set_hexpand(true);
    let t = gtk::Label::new(Some(title));
    t.add_css_class("label-medium-bold");
    t.set_halign(gtk::Align::Start);
    b.append(&t);
    let h = gtk::Label::new(Some(help));
    h.add_css_class("label-small");
    h.set_halign(gtk::Align::Start);
    h.set_xalign(0.0);
    h.set_wrap(true);
    h.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
    b.append(&h);
    b
}

fn metric_index(m: GithubPillMetric) -> u32 {
    GithubPillMetric::all()
        .iter()
        .position(|x| *x == m)
        .unwrap_or(0) as u32
}

#[relm4::component(pub)]
impl Component for GithubSettingsModel {
    type CommandOutput = ();
    type Input = GithubSettingsInput;
    type Output = GithubSettingsOutput;
    type Init = GithubSettingsInit;

    view! {
        #[root]
        gtk::ScrolledWindow {
            set_vscrollbar_policy: gtk::PolicyType::Automatic,
            set_hscrollbar_policy: gtk::PolicyType::Never,
            set_propagate_natural_height: false,
            set_propagate_natural_width: false,
            set_hexpand: true,
            set_vexpand: true,

            #[name = "page"]
            gtk::Box {
                add_css_class: "settings-page",
                set_orientation: gtk::Orientation::Vertical,
                set_hexpand: true,
                set_spacing: 16,

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Bar pill",
                    set_halign: gtk::Align::Start,
                },

                #[name = "list"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "Pill shows",
                            "Unread notifications (the default), or a figure from your contribution calendar. The calendar is fetched with `gh api graphql` every 30 minutes, only while it is shown.",
                        ),
                        #[name = "metric_drop"]
                        gtk::DropDown {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(metric_handler)]
                            set_selected: model.metric,
                            connect_selected_notify[sender] => move |d| {
                                sender.input(GithubSettingsInput::MetricChanged(d.selected()));
                            } @metric_handler,
                        },
                    },

                    gtk::Box {
                        add_css_class: "action-row",
                        set_orientation: gtk::Orientation::Horizontal,
                        set_spacing: 20,
                        append: &row_text(
                            "Hide when zero",
                            "Hide the pill while its count is 0 (never while it is loading or gh reports an error).",
                        ),
                        gtk::Switch {
                            set_valign: gtk::Align::Center,
                            #[watch]
                            #[block_signal(hide_handler)]
                            set_active: model.hide_when_zero,
                            connect_state_set[sender] => move |_, v| {
                                sender.input(GithubSettingsInput::HideChanged(v));
                                glib::Propagation::Proceed
                            } @hide_handler,
                        },
                    },
                },

                gtk::Label {
                    add_css_class: "label-large-bold",
                    set_label: "Pull requests",
                    set_halign: gtk::Align::Start,
                },

                #[name = "pr_list"]
                gtk::Box {
                    add_css_class: "boxed-list",
                    set_orientation: gtk::Orientation::Vertical,
                },
            }
        }
    }

    fn init(
        _params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();
        let s = sender.clone();
        effects.push(move |_| {
            s.input(GithubSettingsInput::MetricEffect(
                config_manager().config().github().pill_metric().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            s.input(GithubSettingsInput::HideEffect(
                config_manager().config().github().hide_when_zero().get(),
            ));
        });
        let s = sender.clone();
        effects.push(move |_| {
            let gh = config_manager().config().github();
            s.input(GithubSettingsInput::ListsEffect(
                gh.clone().pr_rules().get(),
                gh.excluded_repos().get(),
            ));
        });
        let gh = config_manager().config().github().get_untracked();
        let rules = gtk::TextBuffer::new(None);
        rules.set_text(&gh.pr_rules.join("\n"));
        let excluded = gtk::TextBuffer::new(None);
        excluded.set_text(&gh.excluded_repos.join("\n"));
        let model = GithubSettingsModel {
            metric: metric_index(gh.pill_metric),
            hide_when_zero: gh.hide_when_zero,
            rules: rules.clone(),
            excluded: excluded.clone(),
            _effects: effects,
        };
        let widgets = view_output!();
        widgets.pr_list.append(&list_editor(
            "Search rules",
            "One GitHub search per line, each run as `is:pr is:open <rule>` (plus `archived:false`) and merged — e.g. `author:@me`, `review-requested:@me`, `org:acme label:bug`. Empty = `author:@me`. Up to 8 rules; applied when you leave the box.",
            &rules,
            &sender,
        ));
        widgets.pr_list.append(&list_editor(
            "Excluded repositories",
            "One per line: `owner/name`, or a bare `name` for any owner; `*` matches anything (`acme/legacy-*`). Case-insensitive.",
            &excluded,
            &sender,
        ));
        let labels: Vec<&str> = GithubPillMetric::all().iter().map(|m| m.label()).collect();
        widgets
            .metric_drop
            .set_model(Some(&gtk::StringList::new(&labels)));
        widgets.metric_drop.set_selected(model.metric);
        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>, _root: &Self::Root) {
        match message {
            GithubSettingsInput::MetricChanged(i) => {
                if let Some(m) = GithubPillMetric::all().get(i as usize).copied()
                    && i != self.metric
                {
                    self.metric = i;
                    config_manager().update_config(|c| c.github.pill_metric = m);
                }
            }
            GithubSettingsInput::MetricEffect(m) => self.metric = metric_index(m),
            GithubSettingsInput::HideChanged(v) => {
                config_manager().update_config(|c| c.github.hide_when_zero = v);
            }
            GithubSettingsInput::HideEffect(v) => self.hide_when_zero = v,
            GithubSettingsInput::ListsCommitted => {
                let rules = lines_to_list(&buffer_text(&self.rules));
                let excluded = lines_to_list(&buffer_text(&self.excluded));
                let cur = config_manager().config().github().get_untracked();
                if cur.pr_rules != rules || cur.excluded_repos != excluded {
                    config_manager().update_config(|c| {
                        c.github.pr_rules = rules;
                        c.github.excluded_repos = excluded;
                    });
                }
            }
            GithubSettingsInput::ListsEffect(rules, excluded) => {
                // Don't clobber text being edited with the same value.
                if lines_to_list(&buffer_text(&self.rules)) != rules {
                    self.rules.set_text(&rules.join("\n"));
                }
                if lines_to_list(&buffer_text(&self.excluded)) != excluded {
                    self.excluded.set_text(&excluded.join("\n"));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_lines_are_trimmed_and_blank_free() {
        assert_eq!(
            lines_to_list(" author:@me \n\n  review-requested:@me\n   "),
            ["author:@me", "review-requested:@me"]
        );
        assert!(lines_to_list("").is_empty());
    }
}
