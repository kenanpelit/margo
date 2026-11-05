//! The Wallpaper bar pill. Each mouse button runs a configurable
//! [`WallpaperPillAction`] (`bars.widgets.wallpaper`, Settings → Theme →
//! Wallpaper → Bar pill): open the picker menu, or step the wallpaper the
//! way `mshellctl wallpaper next|prev|random` does; scrolling can step it
//! too. The tooltip names the current wallpaper. The defaults keep the old
//! pill: a click opens the picker, the other buttons and scrolling do
//! nothing.

use mshell_cache::wallpaper::{
    CycleDirection, WallpaperStateStoreFields, current_wallpaper_path, cycle_wallpaper,
    wallpaper_store,
};
use mshell_common::scoped_effects::EffectScope;
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::{
    BarWidgetsStoreFields, BarsStoreFields, ConfigStoreFields, WallpaperBarWidget,
    WallpaperPillAction,
};
use reactive_graph::prelude::{Get, GetUntracked};
use relm4::gtk::Orientation;
use relm4::gtk::prelude::{ButtonExt, GestureSingleExt, WidgetExt};
use relm4::{ComponentParts, ComponentSender, SimpleComponent, gtk};
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub(crate) struct WallpaperModel {
    orientation: Orientation,
    actions: WallpaperBarWidget,
    /// File name of the current wallpaper, for the tooltip.
    current: Option<String>,
    scroll: ScrollStepper,
    /// Mirrors `actions.scroll` for the scroll controller, which lets the
    /// event through to the bar while scrolling is off.
    scroll_enabled: Rc<Cell<bool>>,
    _effects: EffectScope,
}

/// The shortest time between two scroll steps. Each step decodes a whole
/// wallpaper, and a touchpad flick sends dozens of scroll events.
const SCROLL_STEP_INTERVAL: Duration = Duration::from_millis(250);

/// Turns scroll deltas into wallpaper steps: one step per notch of a wheel
/// (`dy` = ±1), one per accumulated unit of smooth (touchpad) scrolling, at
/// most one per [`SCROLL_STEP_INTERVAL`]. Up = next, down = previous.
#[derive(Debug, Default)]
struct ScrollStepper {
    acc: f64,
    last_step: Option<Instant>,
}

impl ScrollStepper {
    fn feed(&mut self, dy: f64, now: Instant) -> Option<CycleDirection> {
        if !dy.is_finite() || dy == 0.0 {
            return None;
        }
        // A reversal drops what was gathered the other way.
        if self.acc != 0.0 && self.acc.signum() != dy.signum() {
            self.acc = 0.0;
        }
        self.acc += dy;
        if self.acc.abs() < 1.0 {
            return None;
        }
        let direction = if self.acc < 0.0 {
            CycleDirection::Next
        } else {
            CycleDirection::Previous
        };
        self.acc = 0.0;
        if self
            .last_step
            .is_some_and(|t| now.duration_since(t) < SCROLL_STEP_INTERVAL)
        {
            return None;
        }
        self.last_step = Some(now);
        Some(direction)
    }
}

/// Which mouse button was pressed on the pill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PillButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug)]
pub(crate) enum WallpaperInput {
    Pressed(PillButton),
    Scrolled(f64),
    ActionsEffect(WallpaperBarWidget),
    CurrentEffect(Option<String>),
}

#[derive(Debug)]
pub(crate) enum WallpaperOutput {
    Clicked,
}

pub(crate) struct WallpaperInit {
    pub(crate) orientation: Orientation,
}

/// What the pill does for one press, decided from the configured actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PillEffect {
    Nothing,
    OpenPicker,
    Cycle(CycleDirection),
}

fn action_for(actions: &WallpaperBarWidget, button: PillButton) -> WallpaperPillAction {
    match button {
        PillButton::Left => actions.left_click,
        PillButton::Middle => actions.middle_click,
        PillButton::Right => actions.right_click,
    }
}

fn effect_of(action: WallpaperPillAction) -> PillEffect {
    match action {
        WallpaperPillAction::None => PillEffect::Nothing,
        WallpaperPillAction::OpenPicker => PillEffect::OpenPicker,
        WallpaperPillAction::Next => PillEffect::Cycle(CycleDirection::Next),
        WallpaperPillAction::Previous => PillEffect::Cycle(CycleDirection::Previous),
        WallpaperPillAction::Random => PillEffect::Cycle(CycleDirection::Random),
    }
}

/// The interaction-hint half of the tooltip (§4.6): one `Button: action`
/// entry per button that does something, in left / middle / right order.
fn hint_line(actions: &WallpaperBarWidget) -> String {
    let hint = |action: WallpaperPillAction| match action {
        WallpaperPillAction::None => None,
        WallpaperPillAction::OpenPicker => Some("open the picker"),
        WallpaperPillAction::Next => Some("next wallpaper"),
        WallpaperPillAction::Previous => Some("previous wallpaper"),
        WallpaperPillAction::Random => Some("random wallpaper"),
    };
    let mut hints: Vec<String> = [
        ("Click", actions.left_click),
        ("Middle-click", actions.middle_click),
        ("Right-click", actions.right_click),
    ]
    .into_iter()
    .filter_map(|(button, action)| hint(action).map(|h| format!("{button}: {h}")))
    .collect();
    if actions.scroll {
        hints.push("Scroll: next / previous".to_string());
    }
    hints.join("  ·  ")
}

/// The pill tooltip: the current wallpaper's name on the first line, the
/// interaction hints on the second.
fn pill_tooltip(actions: &WallpaperBarWidget, current: Option<&str>) -> String {
    let summary = match current {
        Some(name) => format!("Wallpaper  ·  {name}"),
        None => "Wallpaper".to_string(),
    };
    let hints = hint_line(actions);
    if hints.is_empty() {
        summary
    } else {
        format!("{summary}\n{hints}")
    }
}

/// File name of the wallpaper that is set, if one was recorded.
fn current_wallpaper_name() -> Option<String> {
    current_wallpaper_path()?
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
}

#[relm4::component(pub)]
impl SimpleComponent for WallpaperModel {
    type Input = WallpaperInput;
    type Output = WallpaperOutput;
    type Init = WallpaperInit;

    view! {
        #[root]
        #[name = "root"]
        gtk::Box {
            add_css_class: "wallpaper-bar-widget",
            set_hexpand: model.orientation == Orientation::Vertical,
            set_vexpand: model.orientation == Orientation::Horizontal,
            set_halign: gtk::Align::Center,
            set_valign: gtk::Align::Center,
            // §4.5: pill tooltip = summary + interaction hint.
            #[watch]
            set_tooltip_text: Some(&pill_tooltip(&model.actions, model.current.as_deref())),

            gtk::Button {
                set_css_classes: &["ok-button-surface", "ok-bar-widget"],
                set_hexpand: false,
                set_vexpand: false,
                connect_clicked[sender] => move |_| {
                    sender.input(WallpaperInput::Pressed(PillButton::Left));
                },

                #[name="image"]
                gtk::Image {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_halign: gtk::Align::Center,
                    set_valign: gtk::Align::Center,
                    set_icon_name: Some("wallpaper-symbolic"),
                }
            }
        }
    }

    fn init(
        params: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let mut effects = EffectScope::new();
        let effect_sender = sender.clone();
        effects.push(move |_| {
            let actions = config_manager().config().bars().widgets().wallpaper().get();
            effect_sender.input(WallpaperInput::ActionsEffect(actions));
        });
        // Every applied wallpaper bumps the revision; re-read the name then.
        let effect_sender = sender.clone();
        effects.push(move |_| {
            let _revision = wallpaper_store().revision().get();
            effect_sender.input(WallpaperInput::CurrentEffect(current_wallpaper_name()));
        });

        let actions = config_manager()
            .config()
            .bars()
            .widgets()
            .wallpaper()
            .get_untracked();
        let scroll_enabled = Rc::new(Cell::new(actions.scroll));
        let model = WallpaperModel {
            orientation: params.orientation,
            actions,
            current: current_wallpaper_name(),
            scroll: ScrollStepper::default(),
            scroll_enabled: scroll_enabled.clone(),
            _effects: effects,
        };

        let widgets = view_output!();

        // Middle / right buttons. The inner button already owns the
        // primary click (`connect_clicked`), so this gesture skips it.
        let gesture = gtk::GestureClick::new();
        gesture.set_button(0);
        let click_sender = sender.clone();
        gesture.connect_pressed(move |g, _, _, _| {
            let button = match g.current_button() {
                gtk::gdk::BUTTON_MIDDLE => PillButton::Middle,
                gtk::gdk::BUTTON_SECONDARY => PillButton::Right,
                _ => return,
            };
            click_sender.input(WallpaperInput::Pressed(button));
        });
        widgets.root.add_controller(gesture);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let scroll_sender = sender.clone();
        scroll.connect_scroll(move |_, _dx, dy| {
            if !scroll_enabled.get() {
                return gtk::glib::Propagation::Proceed;
            }
            scroll_sender.input(WallpaperInput::Scrolled(dy));
            gtk::glib::Propagation::Stop
        });
        widgets.root.add_controller(scroll);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, message: Self::Input, sender: ComponentSender<Self>) {
        match message {
            WallpaperInput::Pressed(button) => {
                match effect_of(action_for(&self.actions, button)) {
                    PillEffect::Nothing => {}
                    PillEffect::OpenPicker => {
                        sender.output(WallpaperOutput::Clicked).unwrap_or_default();
                    }
                    // Main thread: `cycle_wallpaper` kicks glib work.
                    PillEffect::Cycle(direction) => cycle_wallpaper(direction),
                }
            }
            WallpaperInput::Scrolled(dy) => {
                if self.actions.scroll
                    && let Some(direction) = self.scroll.feed(dy, Instant::now())
                {
                    cycle_wallpaper(direction);
                }
            }
            WallpaperInput::ActionsEffect(actions) => {
                self.scroll_enabled.set(actions.scroll);
                self.actions = actions;
            }
            WallpaperInput::CurrentEffect(current) => self.current = current,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn actions(
        left: WallpaperPillAction,
        middle: WallpaperPillAction,
        right: WallpaperPillAction,
    ) -> WallpaperBarWidget {
        WallpaperBarWidget {
            left_click: left,
            middle_click: middle,
            right_click: right,
            scroll: false,
        }
    }

    #[test]
    fn default_pill_opens_the_picker_and_ignores_other_buttons() {
        let a = WallpaperBarWidget::default();
        assert_eq!(
            effect_of(action_for(&a, PillButton::Left)),
            PillEffect::OpenPicker
        );
        assert_eq!(
            effect_of(action_for(&a, PillButton::Middle)),
            PillEffect::Nothing
        );
        assert_eq!(
            effect_of(action_for(&a, PillButton::Right)),
            PillEffect::Nothing
        );
        assert_eq!(pill_tooltip(&a, None), "Wallpaper\nClick: open the picker");
    }

    #[test]
    fn each_button_maps_to_its_own_action() {
        use WallpaperPillAction::*;
        let a = actions(Next, Previous, Random);
        assert_eq!(
            effect_of(action_for(&a, PillButton::Left)),
            PillEffect::Cycle(CycleDirection::Next)
        );
        assert_eq!(
            effect_of(action_for(&a, PillButton::Middle)),
            PillEffect::Cycle(CycleDirection::Previous)
        );
        assert_eq!(
            effect_of(action_for(&a, PillButton::Right)),
            PillEffect::Cycle(CycleDirection::Random)
        );
    }

    #[test]
    fn tooltip_lists_only_buttons_that_do_something() {
        use WallpaperPillAction::*;
        assert_eq!(
            pill_tooltip(&actions(OpenPicker, Next, Random), Option::None),
            "Wallpaper\nClick: open the picker  ·  Middle-click: next wallpaper  ·  \
             Right-click: random wallpaper"
        );
        assert_eq!(
            pill_tooltip(&actions(None, None, Previous), Option::None),
            "Wallpaper\nRight-click: previous wallpaper"
        );
        assert_eq!(
            pill_tooltip(&actions(None, None, None), Option::None),
            "Wallpaper"
        );
    }

    #[test]
    fn tooltip_names_the_current_wallpaper_and_the_scroll() {
        let mut a = WallpaperBarWidget::default();
        assert_eq!(
            pill_tooltip(&a, Some("lake.jpg")),
            "Wallpaper  ·  lake.jpg\nClick: open the picker"
        );
        a.scroll = true;
        a.left_click = WallpaperPillAction::None;
        assert_eq!(
            pill_tooltip(&a, Some("lake.jpg")),
            "Wallpaper  ·  lake.jpg\nScroll: next / previous"
        );
    }

    #[test]
    fn wheel_notches_step_up_next_and_down_previous() {
        let mut s = ScrollStepper::default();
        let t0 = Instant::now();
        assert_eq!(s.feed(-1.0, t0), Some(CycleDirection::Next));
        assert_eq!(
            s.feed(1.0, t0 + Duration::from_millis(300)),
            Some(CycleDirection::Previous)
        );
        assert_eq!(s.feed(0.0, t0 + Duration::from_secs(5)), None);
        assert_eq!(s.feed(f64::NAN, t0 + Duration::from_secs(5)), None);
    }

    #[test]
    fn smooth_scroll_accumulates_to_one_step() {
        let mut s = ScrollStepper::default();
        let t0 = Instant::now();
        assert_eq!(s.feed(-0.4, t0), None);
        assert_eq!(s.feed(-0.4, t0), None);
        assert_eq!(s.feed(-0.4, t0), Some(CycleDirection::Next));
        // A reversal starts counting again from zero.
        let t1 = t0 + Duration::from_secs(1);
        assert_eq!(s.feed(-0.6, t1), None);
        assert_eq!(s.feed(0.6, t1), None);
        assert_eq!(s.feed(0.6, t1), Some(CycleDirection::Previous));
    }

    #[test]
    fn a_flick_steps_at_most_once_per_interval() {
        let mut s = ScrollStepper::default();
        let t0 = Instant::now();
        assert_eq!(s.feed(-1.0, t0), Some(CycleDirection::Next));
        for ms in [10, 50, 120, 240] {
            assert_eq!(
                s.feed(-1.0, t0 + Duration::from_millis(ms)),
                None,
                "{ms} ms"
            );
        }
        assert_eq!(
            s.feed(-1.0, t0 + Duration::from_millis(260)),
            Some(CycleDirection::Next)
        );
    }
}
