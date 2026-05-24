use std::process::{Command, Output};

use crossterm::event::KeyCode;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
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
    system_shell: String,
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
    pub fn new(
        power_config: PowerControlConfig,
        switcher_config: SwitcherConfig,
        system_shell: String,
    ) -> Self {
        Self {
            power_config,
            switcher_config,
            system_shell,
        }
    }

    fn switcher_toggle_style(&self) -> Style {
        let mut style = Style::default().fg(get_color(&self.switcher_config.toggle_hint_color));

        for modifier in get_modifiers(&self.switcher_config.toggle_hint_modifiers) {
            style = style.add_modifier(modifier);
        }

        style
    }

    /// Render the power controls as a single centred row of quiet "chips"
    /// at the bottom of the stack — mlock keeps these muted (accent is
    /// reserved for the card), so each chip is just `key hint` in the
    /// configured (muted) colour, the key underlined, with a gap between.
    pub fn render(&self, frame: &mut Frame<impl ratatui::backend::Backend>, area: Rect) {
        let mut items = Vec::new();

        for power_control in self
            .power_config
            .base_entries
            .0
            .iter()
            .chain(self.power_config.entries.0.iter())
        {
            if !items.is_empty() {
                // Separator between chips.
                items.push(Span::raw("   "));
            }
            items.push(Span::styled(
                power_control.key.as_str(),
                power_control.style().add_modifier(Modifier::UNDERLINED),
            ));
            items.push(Span::raw(" "));
            items.push(Span::styled(power_control.hint.as_str(), power_control.style()));
        }

        // The session-switcher toggle hint (when bound to an F-key) reads as
        // one more chip on the same centred row.
        if let SwitcherVisibility::Keybind(KeyCode::F(n)) = self.switcher_config.switcher_visibility
        {
            if !items.is_empty() {
                items.push(Span::raw("   "));
            }
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

    pub(crate) fn key_press(&self, key_code: KeyCode) -> Option<super::ErrorStatusMessage> {
        // TODO: Properly handle StdIn
        for power_control in self
            .power_config
            .base_entries
            .0
            .iter()
            .chain(self.power_config.entries.0.iter())
        {
            if key_code == get_key(&power_control.key) {
                let cmd_status = Command::new(&self.system_shell)
                    .arg("-c")
                    .arg(power_control.cmd.clone())
                    .output();

                match cmd_status {
                    Err(err) => {
                        log::error!("Failed to execute shutdown command: {:?}", err);
                        return Some(super::ErrorStatusMessage::FailedPowerControl(
                            power_control.hint.clone(),
                        ));
                    }
                    Ok(Output {
                        status,
                        stdout,
                        stderr,
                    }) if !status.success() => {
                        log::error!("Error while executing \"{}\"", power_control.hint);
                        log::error!("STDOUT:\n{:?}", stdout);
                        log::error!("STDERR:\n{:?}", stderr);

                        return Some(super::ErrorStatusMessage::FailedPowerControl(
                            power_control.hint.clone(),
                        ));
                    }
                    _ => {}
                }
            }
        }

        None
    }
}
