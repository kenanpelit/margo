//! The Wallpaper bar pill. Each mouse button runs a configurable
//! [`WallpaperPillAction`] (`bars.widgets.wallpaper`, Settings → Theme →
//! Wallpaper → Bar pill): open the picker menu, or step the wallpaper the
//! way `mshellctl wallpaper next|prev|random` does. The defaults keep the
//! old pill: a click opens the picker, the other buttons do nothing.

use mshell_cache::wallpaper::{CycleDirection, cycle_wallpaper};
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

#[derive(Debug)]
pub(crate) struct WallpaperModel {
    orientation: Orientation,
    actions: WallpaperBarWidget,
    _effects: EffectScope,
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
    ActionsEffect(WallpaperBarWidget),
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
    [
        ("Click", actions.left_click),
        ("Middle-click", actions.middle_click),
        ("Right-click", actions.right_click),
    ]
    .into_iter()
    .filter_map(|(button, action)| hint(action).map(|h| format!("{button}: {h}")))
    .collect::<Vec<_>>()
    .join("  ·  ")
}

/// The pill tooltip: a summary plus the interaction hints.
fn pill_tooltip(actions: &WallpaperBarWidget) -> String {
    let hints = hint_line(actions);
    if hints.is_empty() {
        "Wallpaper".to_string()
    } else {
        format!("Wallpaper  ·  {hints}")
    }
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
            set_tooltip_text: Some(&pill_tooltip(&model.actions)),

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

        let model = WallpaperModel {
            orientation: params.orientation,
            actions: config_manager()
                .config()
                .bars()
                .widgets()
                .wallpaper()
                .get_untracked(),
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
            WallpaperInput::ActionsEffect(actions) => self.actions = actions,
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
        assert_eq!(pill_tooltip(&a), "Wallpaper  ·  Click: open the picker");
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
            pill_tooltip(&actions(OpenPicker, Next, Random)),
            "Wallpaper  ·  Click: open the picker  ·  Middle-click: next wallpaper  ·  \
             Right-click: random wallpaper"
        );
        assert_eq!(
            pill_tooltip(&actions(None, None, Previous)),
            "Wallpaper  ·  Right-click: previous wallpaper"
        );
        assert_eq!(pill_tooltip(&actions(None, None, None)), "Wallpaper");
    }
}
