//! Bar visibility for `Frame`: toggling, revealing and hiding both bars
//! (optionally only the ones revealed by default).

use super::*;

impl Frame {
    /// `FrameInput::BarToggleAll`.
    pub(super) fn bars_toggle_all(&self, exclude_hidden_by_default: bool) {
        if exclude_hidden_by_default {
            if config_manager()
                .config()
                .bars()
                .top_bar()
                .reveal_by_default()
                .get_untracked()
            {
                self.top_bar.sender().emit(BarInput::ToggleRevealed);
            }
            if config_manager()
                .config()
                .bars()
                .bottom_bar()
                .reveal_by_default()
                .get_untracked()
            {
                self.bottom_bar.sender().emit(BarInput::ToggleRevealed);
            }
        } else {
            self.top_bar.sender().emit(BarInput::ToggleRevealed);
            self.bottom_bar.sender().emit(BarInput::ToggleRevealed);
        }
    }

    /// `FrameInput::BarRevealAll`.
    pub(super) fn bars_reveal_all(&self, exclude_hidden_by_default: bool) {
        if exclude_hidden_by_default {
            if config_manager()
                .config()
                .bars()
                .top_bar()
                .reveal_by_default()
                .get_untracked()
            {
                self.top_bar.sender().emit(BarInput::SetRevealed(true));
            }
            if config_manager()
                .config()
                .bars()
                .bottom_bar()
                .reveal_by_default()
                .get_untracked()
            {
                self.bottom_bar.sender().emit(BarInput::SetRevealed(true));
            }
        } else {
            self.top_bar.sender().emit(BarInput::SetRevealed(true));
            self.bottom_bar.sender().emit(BarInput::SetRevealed(true));
        }
    }

    /// `FrameInput::BarHideAll`.
    pub(super) fn bars_hide_all(&self, exclude_hidden_by_default: bool) {
        if exclude_hidden_by_default {
            if config_manager()
                .config()
                .bars()
                .top_bar()
                .reveal_by_default()
                .get_untracked()
            {
                self.top_bar.sender().emit(BarInput::SetRevealed(false));
            }
            if config_manager()
                .config()
                .bars()
                .bottom_bar()
                .reveal_by_default()
                .get_untracked()
            {
                self.bottom_bar.sender().emit(BarInput::SetRevealed(false));
            }
        } else {
            self.top_bar.sender().emit(BarInput::SetRevealed(false));
            self.bottom_bar.sender().emit(BarInput::SetRevealed(false));
        }
    }
}
