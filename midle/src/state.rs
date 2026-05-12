//! Manager state — the source of truth for which steps have fired,
//! whether we're paused, and any pending notification handles.

use crate::config::{Config, Step};
use crate::actions;
use std::time::{Duration, Instant};
use tracing::{debug, info};

pub struct Manager {
    pub cfg: Config,
    /// One bit per `cfg.steps` index: true once that step's
    /// `command` has been executed in the current idle run.
    pub fired: Vec<bool>,
    /// When `Some`, the daemon is paused until this instant (or
    /// forever if `None` is paused).
    pub pause: PauseState,
    /// Manual toggle from the CLI / status bar — when true, all
    /// step firings are suppressed regardless of timeouts.
    pub inhibit: bool,
}

#[derive(Debug, Clone, Copy)]
pub enum PauseState {
    Running,
    UntilInstant(Instant),
    Indefinite,
}

impl Manager {
    pub fn new(cfg: Config) -> Self {
        let n = cfg.steps.len();
        Self {
            cfg,
            fired: vec![false; n],
            pause: PauseState::Running,
            inhibit: false,
        }
    }

    pub fn replace_config(&mut self, cfg: Config) {
        self.fired = vec![false; cfg.steps.len()];
        self.cfg = cfg;
    }

    pub fn is_suppressed(&self) -> bool {
        if self.inhibit {
            return true;
        }
        match self.pause {
            PauseState::Running => false,
            PauseState::Indefinite => true,
            PauseState::UntilInstant(t) => Instant::now() < t,
        }
    }

    /// Called when a step's idle threshold fires. Executes its
    /// `command` (unless suppressed) and remembers the firing.
    pub fn on_step_idled(&mut self, idx: usize) {
        if self.is_suppressed() {
            debug!(idx, "idle event suppressed (pause / inhibit)");
            return;
        }
        if idx >= self.cfg.steps.len() || self.fired[idx] {
            return;
        }
        let step = &self.cfg.steps[idx].clone();
        if self.cfg.settings.notify_before_action && step.notify {
            actions::notify(&step.name, "ready");
        }
        info!(name = %step.name, "step fired");
        actions::spawn_shell(&step.name, &step.command);
        self.fired[idx] = true;
    }

    /// Called on user activity. Resets the run by executing each
    /// fired step's `resume_command` in reverse order, then clearing
    /// the fired bitmap.
    pub fn on_active(&mut self) {
        let any = self.fired.iter().any(|&b| b);
        if !any {
            return;
        }
        info!("user activity — resuming");
        let steps: Vec<Step> = self.cfg.steps.clone();
        for (i, step) in steps.iter().enumerate().rev() {
            if !self.fired[i] {
                continue;
            }
            if let Some(cmd) = step.resume_command.as_deref()
                && !cmd.trim().is_empty()
            {
                actions::spawn_shell(&format!("{}.resume", step.name), cmd);
            }
        }
        self.fired.fill(false);
    }

    pub fn pause_for(&mut self, dur: Option<Duration>) {
        match dur {
            Some(d) => {
                self.pause = PauseState::UntilInstant(Instant::now() + d);
                info!(secs = d.as_secs(), "paused");
            }
            None => {
                self.pause = PauseState::Indefinite;
                info!("paused indefinitely");
            }
        }
    }

    pub fn resume_from_pause(&mut self) {
        let was_paused = !matches!(self.pause, PauseState::Running);
        self.pause = PauseState::Running;
        if was_paused {
            info!("resumed from pause");
            if self.cfg.settings.notify_on_unpause {
                actions::notify("resume", "idle timer resumed");
            }
        }
    }

    pub fn toggle_inhibit(&mut self) -> bool {
        self.inhibit = !self.inhibit;
        info!(inhibit = self.inhibit, "inhibit toggled");
        self.inhibit
    }
}
