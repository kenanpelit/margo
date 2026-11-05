//! Settings → Widgets → GitHub — what the bar pill counts. Composed with
//! the generic per-menu geometry page.

use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{ConfigStoreFields, GithubConfigStoreFields, GithubPillMetric};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::prelude::{BoxExt, OrientableExt, WidgetExt};
use relm4::{Component, ComponentParts, ComponentSender, gtk};

pub(crate) struct GithubSettingsModel {
    metric: u32,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum GithubSettingsInput {
    MetricChanged(u32),
    MetricEffect(GithubPillMetric),
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
        let model = GithubSettingsModel {
            metric: metric_index(
                config_manager()
                    .config()
                    .github()
                    .pill_metric()
                    .get_untracked(),
            ),
            _effects: effects,
        };
        let widgets = view_output!();
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
        }
    }
}
