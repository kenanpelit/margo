//! Bar-mdash widget.
//!
//! Twin of `clock.rs` — same per-second tick, same `[tempo]`
//! cycling format list, same chrono-strftime rendering. Only
//! difference: a left-click emits `MdashOutput::Clicked` so the
//! frame toggles the mdash menu (greeting hero + calendar +
//! weather + media player + QS tile stack + menu-shortcut grid)
//! instead of the plain clock menu. Right-click still cycles
//! through the configured `formats` list so the label feels
//! identical to the Clock pill — users who prefer mdash's richer
//! surface can swap pills without losing their date/time wording.
//!
//! The CSS hooks (`clock-bar-widget`, `clock-bar-label`) are
//! reused so the existing bar typography stays uniform — there's
//! no visual reason the mdash label should read differently from
//! the standalone clock pill.

use chrono::Local;
use mshell_common::scoped_effects::EffectScope;
use mshell_config::schema::config::*;
use reactive_graph::traits::{Get, GetUntracked};
use relm4::{
    ComponentParts, ComponentSender, SimpleComponent,
    gtk::{
        self, Orientation,
        glib::{self, SourceId},
        prelude::{ButtonExt, GestureSingleExt, WidgetExt},
    },
};
use std::cell::Cell;
use std::rc::Rc;

const FALLBACK_24H: &str = "%H:%M";
const FALLBACK_12H: &str = "%I:%M %p";
const FALLBACK_24H_VERTICAL: &str = "%H\n%M";
const FALLBACK_12H_VERTICAL: &str = "%I\n%M";

#[derive(Debug)]
pub(crate) struct MdashModel {
    orientation: Orientation,
    formats: Vec<String>,
    fallback_24h: bool,
    current_idx: Rc<Cell<usize>>,
    time_label: String,
    timer_id: Option<SourceId>,
    _effects: EffectScope,
}

#[derive(Debug)]
pub(crate) enum MdashInput {
    UpdateTime,
    CycleFormat,
    ReloadFormats {
        formats: Vec<String>,
        fallback_24h: bool,
    },
}

#[derive(Debug)]
pub(crate) enum MdashOutput {
    Clicked,
}

pub(crate) struct MdashInit {
    pub(crate) orientation: Orientation,
}

#[relm4::component(pub)]
impl SimpleComponent for MdashModel {
    type Input = MdashInput;
    type Output = MdashOutput;
    type Init = MdashInit;

    view! {
        #[root]
        gtk::Box {
            add_css_class: "clock-bar-widget",
            add_css_class: "mdash-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,

            #[name="button"]
            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                connect_clicked[sender] => move |_| {
                    sender.output(MdashOutput::Clicked).unwrap_or_default();
                },

                gtk::Label {
                    add_css_class: "clock-bar-label",
                    #[watch]
                    set_label: model.time_label.as_str(),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let base_config = mshell_config::config_manager::config_manager().config();

        // `input_sender().send()` not `sender_clone.input()` —
        // self-cancels the tick when the controller drops so a
        // bar-rebuild on widget reorder can't abort mshell with
        // "The runtime of the component was shutdown". See
        // sysstat.rs `schedule_poll` for the full story.
        let sender_clone = sender.clone();
        let id = glib::timeout_add_local(std::time::Duration::from_secs(1), move || {
            if sender_clone
                .input_sender()
                .send(MdashInput::UpdateTime)
                .is_err()
            {
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });

        let fallback_24h = base_config
            .clone()
            .general()
            .clock_format_24_h()
            .get_untracked();

        let formats = collect_formats(base_config.clone().tempo().get_untracked());
        let current_idx = Rc::new(Cell::new(0usize));

        let time_label = render_now(
            &formats,
            current_idx.get(),
            fallback_24h,
            params.orientation,
        );

        let mut effects = EffectScope::new();
        let sender_clone = sender.clone();
        let base_config_eff = base_config.clone();
        effects.push(move |_| {
            let fallback_24h = base_config_eff.clone().general().clock_format_24_h().get();
            let formats = collect_formats(base_config_eff.clone().tempo().get());
            sender_clone.input(MdashInput::ReloadFormats {
                formats,
                fallback_24h,
            });
        });

        let model = MdashModel {
            orientation: params.orientation,
            formats,
            fallback_24h,
            current_idx: current_idx.clone(),
            time_label,
            timer_id: Some(id),
            _effects: effects,
        };

        let widgets = view_output!();

        // Right-click double-press cycles the cached format index —
        // mirrors the Clock pill so the muscle memory is the same.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(gtk::gdk::BUTTON_SECONDARY);
        let sender_dbl = sender.clone();
        gesture.connect_pressed(move |_, n_press, _, _| {
            if n_press >= 2 {
                sender_dbl.input(MdashInput::CycleFormat);
            }
        });
        widgets.button.add_controller(gesture);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, _sender: ComponentSender<Self>) {
        match message {
            MdashInput::UpdateTime => {
                self.time_label = render_now(
                    &self.formats,
                    self.current_idx.get(),
                    self.fallback_24h,
                    self.orientation,
                );
            }
            MdashInput::CycleFormat => {
                if self.formats.len() > 1 {
                    let next = (self.current_idx.get() + 1) % self.formats.len();
                    self.current_idx.set(next);
                    self.time_label =
                        render_now(&self.formats, next, self.fallback_24h, self.orientation);
                }
            }
            MdashInput::ReloadFormats {
                formats,
                fallback_24h,
            } => {
                self.formats = formats;
                self.fallback_24h = fallback_24h;
                self.current_idx.set(0);
                self.time_label = render_now(&self.formats, 0, self.fallback_24h, self.orientation);
            }
        }
    }
}

impl Drop for MdashModel {
    fn drop(&mut self) {
        if let Some(id) = self.timer_id.take() {
            id.remove();
        }
    }
}

fn collect_formats(tempo: Tempo) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tempo.formats.len() + 1);
    let initial = tempo.clock_format.trim().to_string();
    if !initial.is_empty() {
        out.push(initial.clone());
    }
    for f in tempo.formats {
        let trimmed = f.trim();
        if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn render_now(
    formats: &[String],
    idx: usize,
    fallback_24h: bool,
    orientation: Orientation,
) -> String {
    let now = Local::now();
    if let Some(fmt) = formats.get(idx) {
        return now.format(fmt).to_string();
    }
    let fmt = match (orientation, fallback_24h) {
        (Orientation::Vertical, true) => FALLBACK_24H_VERTICAL,
        (Orientation::Vertical, false) => FALLBACK_12H_VERTICAL,
        (_, true) => FALLBACK_24H,
        (_, false) => FALLBACK_12H,
    };
    now.format(fmt).to_string()
}
