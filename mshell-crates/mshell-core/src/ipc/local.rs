//! `LocalCommand`s: bus verbs that drive a shell service (Game Mode, the
//! Pomodoro timer, eye care) rather than open a surface. Run on the GTK thread like
//! every other command, but need no frame / monitor lookup.

use super::*;

pub(super) fn dispatch(cmd: LocalCommand) {
    match cmd {
        LocalCommand::GameMode(target) => {
            // Just flip the persisted `active` flag; the reactive reconcile
            // effect (mshell-core `run`) does the apply, so the CLI, the
            // Settings toggle, and startup share one path.
            let active = config_manager().config().game_mode().get_untracked().active;
            let target = target.unwrap_or(!active); // None = toggle
            config_manager().update_config(|c| c.game_mode.active = target);
        }
        LocalCommand::GameModeStatus(reply) => {
            let on = config_manager().config().game_mode().get_untracked().active;
            let _ = reply.send(if on { "on" } else { "off" }.to_string());
        }
        LocalCommand::Pomodoro(cmd, reply) => {
            let _ = reply.send(mshell_frame::pomodoro::run(cmd).status_line());
        }
        LocalCommand::EyeCare(cmd, reply) => {
            let _ = reply.send(mshell_frame::eyecare::run(cmd).status_line());
        }
    }
}
