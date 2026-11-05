//! The user's own Control Center tile (`custom_shortcut`): a label, an
//! icon and a command run on click (`sh -c`), like the bar's custom
//! pills. An optional state command makes it a toggle: exit status 0 =
//! active (accent fill), its first output line = the subtitle. Configured
//! under Settings → Control Center → Custom tile
//! (`control_center.custom_shortcut_tile`).

use super::{ControlCenterTilesCommandOutput, ControlCenterTilesModel};
use mshell_config::config_manager::config_manager;
use mshell_config::schema::config::CustomShortcutTile;
use reactive_graph::traits::GetUntracked;
use relm4::ComponentSender;
use std::time::Duration;

/// How long a state command may run.
const STATE_TIMEOUT: Duration = Duration::from_secs(5);
/// Let the click's command take effect before the state is re-read.
const SETTLE: Duration = Duration::from_millis(700);
/// Subtitles are cut to this many characters.
const MAX_SUBTITLE: usize = 48;

/// What the tile shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CustomState {
    pub(crate) label: String,
    pub(crate) icon: String,
    pub(crate) subtitle: String,
    pub(crate) active: bool,
}

/// The tile's label / icon from config, falling back to the defaults for
/// blank fields.
fn shown(cfg: &CustomShortcutTile) -> (String, String) {
    let or = |v: &str, d: &str| {
        let v = v.trim();
        if v.is_empty() {
            d.to_string()
        } else {
            v.to_string()
        }
    };
    (
        or(&cfg.label, "Custom"),
        or(&cfg.icon, "utilities-terminal-symbolic"),
    )
}

/// The tile state for `cfg` and, when it has a state command, that
/// command's result (`Some((exit 0, stdout))`, `None` = didn't finish).
pub(crate) fn state_for(
    cfg: &CustomShortcutTile,
    probe: Option<Option<(bool, String)>>,
) -> CustomState {
    let (label, icon) = shown(cfg);
    let (subtitle, active) = if cfg.command.trim().is_empty() {
        ("Set a command in Settings".to_string(), false)
    } else {
        match probe {
            None => ("Run".to_string(), false),
            Some(None) => ("State unknown".to_string(), false),
            Some(Some((ok, out))) => {
                let line = out.lines().map(str::trim).find(|l| !l.is_empty());
                let sub = match line {
                    Some(l) => l
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(MAX_SUBTITLE)
                        .collect(),
                    None if ok => "On".to_string(),
                    None => "Off".to_string(),
                };
                (sub, ok)
            }
        }
    };
    CustomState {
        label,
        icon,
        subtitle,
        active,
    }
}

fn config() -> CustomShortcutTile {
    config_manager()
        .config()
        .get_untracked()
        .control_center
        .custom_shortcut_tile
}

/// Run the state command (if any): exit status + stdout, bounded.
async fn probe_state(cfg: &CustomShortcutTile) -> Option<Option<(bool, String)>> {
    let cmd = cfg.state_command.trim();
    if cmd.is_empty() {
        return None;
    }
    let run = crate::plugin_cmd::shell(cmd, &[])
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    Some(match tokio::time::timeout(STATE_TIMEOUT, run).await {
        Ok(Ok(out)) => Some((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )),
        _ => None,
    })
}

/// Re-read the tile (config + state command) off the GTK thread.
pub(super) fn refresh(sender: &ComponentSender<ControlCenterTilesModel>, settle: bool) {
    let cfg = config();
    sender.oneshot_command(async move {
        if settle {
            tokio::time::sleep(SETTLE).await;
        }
        let probe = probe_state(&cfg).await;
        ControlCenterTilesCommandOutput::CustomRefreshed(state_for(&cfg, probe))
    });
}

/// Tile click: run the command, then re-read the state.
pub(super) fn click(sender: &ComponentSender<ControlCenterTilesModel>) {
    let cfg = config();
    if cfg.command.trim().is_empty() {
        mshell_settings::open_settings_at_section("widgets/control_center");
        return;
    }
    crate::plugin_cmd::run_detached(&cfg.command, &[]);
    refresh(sender, true);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(command: &str, state: &str) -> CustomShortcutTile {
        CustomShortcutTile {
            label: "  ".into(),
            icon: String::new(),
            command: command.into(),
            state_command: state.into(),
        }
    }

    #[test]
    fn blank_fields_fall_back_and_no_command_is_inert() {
        let s = state_for(&cfg("", ""), None);
        assert_eq!(s.label, "Custom");
        assert_eq!(s.icon, "utilities-terminal-symbolic");
        assert_eq!(s.subtitle, "Set a command in Settings");
        assert!(!s.active);
        let s = state_for(&cfg("notify-send hi", ""), None);
        assert_eq!(s.subtitle, "Run");
    }

    #[test]
    fn the_state_command_drives_active_and_subtitle() {
        let c = cfg(
            "systemctl --user start syncthing",
            "systemctl --user is-active syncthing",
        );
        let on = state_for(&c, Some(Some((true, "active\n".into()))));
        assert!(on.active);
        assert_eq!(on.subtitle, "active");
        let off = state_for(&c, Some(Some((false, "\n  inactive \n".into()))));
        assert!(!off.active);
        assert_eq!(off.subtitle, "inactive");
        assert_eq!(
            state_for(&c, Some(Some((true, String::new())))).subtitle,
            "On"
        );
        assert_eq!(
            state_for(&c, Some(Some((false, String::new())))).subtitle,
            "Off"
        );
        assert_eq!(state_for(&c, Some(None)).subtitle, "State unknown");
        let long = state_for(&c, Some(Some((true, format!("{}\u{1b}x", "a".repeat(80))))));
        assert_eq!(long.subtitle.chars().count(), MAX_SUBTITLE);
    }

    #[tokio::test]
    async fn state_commands_run_through_sh() {
        let on = probe_state(&cfg("true", "echo up; exit 0")).await;
        assert_eq!(on, Some(Some((true, "up\n".into()))));
        let off = probe_state(&cfg("true", "exit 3")).await;
        assert_eq!(off, Some(Some((false, String::new()))));
        assert_eq!(probe_state(&cfg("true", " ")).await, None);
    }
}
