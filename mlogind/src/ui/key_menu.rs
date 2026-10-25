use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::config::{
    PowerControl, PowerControlConfig, SwitcherConfig, SwitcherVisibility, get_color, get_modifiers,
    parse_key_spec,
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

        // Full chip = "[Ctrl+F1] Shutdown" (3 + key + hint), 4-space
        // separators. If that overflows the row, drop the hints and shorten
        // the chord to "[C-F1]", then to the bare "[F1]", so the F-keys can
        // never be clipped off a narrow terminal.
        let full_w: usize = entries
            .iter()
            .map(|e| 3 + e.key.len() + e.hint.len())
            .sum::<usize>()
            + 4 * entries.len().saturating_sub(1);
        let compact = full_w > usize::from(area.width);
        let keys: Vec<String> = if !compact {
            entries.iter().map(|e| e.key.clone()).collect()
        } else {
            let row = |keys: &[String]| {
                keys.iter().map(|k| 2 + k.len()).sum::<usize>() + 2 * keys.len().saturating_sub(1)
            };
            let short: Vec<String> = entries.iter().map(|e| short_key(&e.key)).collect();
            if row(&short) <= usize::from(area.width) {
                short
            } else {
                entries
                    .iter()
                    .map(|e| bare_key(&e.key).to_string())
                    .collect()
            }
        };

        let mut items = Vec::new();
        for (power_control, key) in entries.iter().zip(keys) {
            if !items.is_empty() {
                items.push(Span::raw(if compact { "  " } else { "    " }));
            }
            items.push(Span::styled("[", bracket));
            items.push(Span::styled(key, key_style));
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
        if !compact
            && let SwitcherVisibility::Keybind(KeyCode::F(n)) =
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
    ///
    /// Modifiers must match exactly: `Ctrl+Shift+F1` does not fire on a bare
    /// F1, and a bare `F1` entry does not fire inside Ctrl+Alt+F1 (the VT
    /// switch chord).
    pub(crate) fn power_index(&self, key_code: KeyCode, modifiers: KeyModifiers) -> Option<usize> {
        let held = modifiers
            & (KeyModifiers::CONTROL
                | KeyModifiers::SHIFT
                | KeyModifiers::ALT
                | KeyModifiers::SUPER);
        self.entries()
            .position(|control| parse_key_spec(&control.key) == Some((held, key_code)))
    }
}

/// The function key of a power-key spec: `Ctrl+Shift+F1` → `F1`.
fn bare_key(spec: &str) -> &str {
    spec.rsplit('+').next().unwrap_or(spec).trim()
}

/// Emacs-style short chord for a narrow footer: `Ctrl+Shift+F1` → `C-S-F1`.
fn short_key(spec: &str) -> String {
    let mut parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    let key = parts.pop().unwrap_or_default();
    let mut out = String::new();
    for m in parts {
        if let Some(c) = m.chars().next() {
            out.push(c.to_ascii_uppercase());
            out.push('-');
        }
    }
    out.push_str(key);
    out
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
        assert_eq!(m.power_index(KeyCode::F(1), KeyModifiers::NONE), Some(0));
        assert_eq!(m.power_index(KeyCode::F(2), KeyModifiers::NONE), Some(1));
        assert_eq!(m.power_index(KeyCode::F(3), KeyModifiers::NONE), Some(2));
    }

    #[test]
    fn an_unbound_key_triggers_nothing() {
        let m = menu(&["F1"], &[]);
        assert_eq!(m.power_index(KeyCode::F(9), KeyModifiers::NONE), None);
        assert_eq!(m.power_index(KeyCode::Enter, KeyModifiers::NONE), None);
    }

    #[test]
    fn a_key_bound_twice_resolves_to_the_first() {
        // The old loop ran BOTH commands. Shutting down twice is not better.
        let m = menu(&["F1"], &["F1"]);
        assert_eq!(m.power_index(KeyCode::F(1), KeyModifiers::NONE), Some(0));
    }

    #[test]
    fn modifiers_must_match_exactly() {
        let m = menu(&["Ctrl+Shift+F1", "F2"], &[]);
        let cs = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
        assert_eq!(m.power_index(KeyCode::F(1), cs), Some(0));
        assert_eq!(m.power_index(KeyCode::F(1), KeyModifiers::NONE), None);
        assert_eq!(m.power_index(KeyCode::F(1), KeyModifiers::CONTROL), None);
        // The VT-switch chord never reaches a bare-F2 power action.
        let ca = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(m.power_index(KeyCode::F(2), ca), None);
        assert_eq!(m.power_index(KeyCode::F(2), KeyModifiers::NONE), Some(1));
    }

    #[test]
    fn a_misspelt_modifier_binds_nothing() {
        let m = menu(&["Ctlr+F1"], &[]);
        assert_eq!(m.power_index(KeyCode::F(1), KeyModifiers::CONTROL), None);
        assert_eq!(m.power_index(KeyCode::F(1), KeyModifiers::NONE), None);
    }

    #[test]
    fn an_empty_config_binds_nothing() {
        let m = menu(&[], &[]);
        assert_eq!(m.power_index(KeyCode::F(1), KeyModifiers::NONE), None);
    }
}
