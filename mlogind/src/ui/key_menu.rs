use crossterm::event::KeyCode;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::config::{
    get_color, get_key, get_modifiers, PowerControl, PowerControlConfig, SwitcherConfig,
    SwitcherVisibility,
};

#[derive(Clone)]
pub struct KeyMenuWidget {
    power_config: PowerControlConfig,
    switcher_config: SwitcherConfig,
}

impl PowerControl {
    fn style(&self) -> Style {
        let mut style = Style::default().fg(get_color(&self.hint_color));

        for modifier in get_modifiers(&self.hint_modifiers) {
            style = style.add_modifier(modifier);
        }

        style
    }
}

impl KeyMenuWidget {
    pub fn new(power_config: PowerControlConfig, switcher_config: SwitcherConfig) -> Self {
        Self {
            power_config,
            switcher_config,
        }
    }

    /// Every power action, base entries first — the order `Request::Power`
    /// indexes into, and the same order the runner rebuilds from its own copy of
    /// the config.
    fn entries(&self) -> impl Iterator<Item = &PowerControl> {
        self.power_config
            .base_entries
            .0
            .iter()
            .chain(self.power_config.entries.0.iter())
    }

    fn switcher_toggle_style(&self) -> Style {
        let mut style = Style::default().fg(get_color(&self.switcher_config.toggle_hint_color));

        for modifier in get_modifiers(&self.switcher_config.toggle_hint_modifiers) {
            style = style.add_modifier(modifier);
        }

        style
    }

    /// Render the power controls as a single centred row of chips at the
    /// bottom of the stack. The key is bracketed + accent-coloured so it's
    /// unmistakable (the user reported the bare keys reading as invisible on
    /// the VT); the hint stays in its configured (muted) colour.
    pub fn render(
        &self,
        frame: &mut Frame<impl ratatui::backend::Backend>,
        area: Rect,
        accent: Color,
    ) {
        let bracket = Style::default().fg(accent);
        let key_style = Style::default()
            .fg(accent)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);

        let entries: Vec<&PowerControl> = self.entries().collect();

        // Full chip = "[F1] Shutdown" (3 + key + hint), 4-space separators.
        // If that overflows the row, drop the hints to "[F1] [F2] [F3]" so
        // the F-keys can never be clipped off a narrow terminal.
        let full_w: usize = entries
            .iter()
            .map(|e| 3 + e.key.len() + e.hint.len())
            .sum::<usize>()
            + 4 * entries.len().saturating_sub(1);
        let compact = full_w > usize::from(area.width);

        let mut items = Vec::new();
        for power_control in &entries {
            if !items.is_empty() {
                items.push(Span::raw(if compact { "  " } else { "    " }));
            }
            items.push(Span::styled("[", bracket));
            items.push(Span::styled(power_control.key.as_str(), key_style));
            items.push(Span::styled("]", bracket));
            if !compact {
                items.push(Span::raw(" "));
                items.push(Span::styled(
                    power_control.hint.as_str(),
                    power_control.style(),
                ));
            }
        }

        // The session-switcher toggle hint (when bound to an F-key) reads as
        // one more chip on the row — only in the full form.
        if !compact {
            if let SwitcherVisibility::Keybind(KeyCode::F(n)) =
                self.switcher_config.switcher_visibility
            {
                items.push(Span::raw("    "));
                items.push(Span::styled(
                    self.switcher_config
                        .toggle_hint
                        .replace("%key%", &format!("F{n}")),
                    self.switcher_toggle_style(),
                ));
            }
        }

        let widget = Paragraph::new(Line::from(items)).alignment(Alignment::Center);
        frame.render_widget(widget, area);
    }

    /// Which power action `key_code` triggers, if any.
    ///
    /// The widget used to run the command itself, with `sh -c`. It cannot any
    /// more: under `cage` this same TUI is the unprivileged greeter. It returns
    /// an index instead, and the caller sends `Request::Power` to the root
    /// session runner, which resolves it against its own config.
    ///
    /// The first match wins, so a key bound twice runs the earlier action —
    /// which is what the old loop did, except it ran both.
    pub(crate) fn power_index(&self, key_code: KeyCode) -> Option<usize> {
        self.entries()
            .position(|control| key_code == get_key(&control.key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, PowerControlVec};

    fn control(key: &str) -> PowerControl {
        PowerControl {
            key: key.to_string(),
            hint: key.to_string(),
            ..PowerControl::default()
        }
    }

    /// The config structs are generated by `toml_config_struct!` and have no
    /// `Default`; the baked one is the only way to get a fully-populated pair.
    fn menu(base: &[&str], extra: &[&str]) -> KeyMenuWidget {
        let config = Config::default();
        let mut power = config.power_controls.clone();
        power.base_entries = PowerControlVec(base.iter().copied().map(control).collect());
        power.entries = PowerControlVec(extra.iter().copied().map(control).collect());
        KeyMenuWidget::new(power, config.environment_switcher.clone())
    }

    #[test]
    fn base_entries_come_before_the_extra_ones() {
        // This ordering is the wire format: the runner rebuilds the same list.
        let m = menu(&["F1", "F2"], &["F3"]);
        assert_eq!(m.power_index(KeyCode::F(1)), Some(0));
        assert_eq!(m.power_index(KeyCode::F(2)), Some(1));
        assert_eq!(m.power_index(KeyCode::F(3)), Some(2));
    }

    #[test]
    fn an_unbound_key_triggers_nothing() {
        let m = menu(&["F1"], &[]);
        assert_eq!(m.power_index(KeyCode::F(9)), None);
        assert_eq!(m.power_index(KeyCode::Enter), None);
    }

    #[test]
    fn a_key_bound_twice_resolves_to_the_first() {
        // The old loop ran BOTH commands. Shutting down twice is not better.
        let m = menu(&["F1"], &["F1"]);
        assert_eq!(m.power_index(KeyCode::F(1)), Some(0));
    }

    #[test]
    fn an_empty_config_binds_nothing() {
        let m = menu(&[], &[]);
        assert_eq!(m.power_index(KeyCode::F(1)), None);
    }
}
