use anyhow::Result;
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Initialize the terminal for TUI mode
pub fn init() -> Result<Tui> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Restore the terminal to normal mode
pub fn restore() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Leave the TUI's alternate screen and disable raw mode so a reused CLI
/// `commands::*` function gets the real terminal (its normal stdout,
/// progress spinners, and `y/N` prompts all work exactly as they do from
/// the plain CLI).
fn suspend() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}

/// Re-enter the TUI's alternate screen, re-enable raw mode, and clear the
/// terminal so the next `Frame` draws over a blank screen instead of
/// whatever the suspended command printed.
fn resume(terminal: &mut Tui) -> Result<()> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    terminal.clear()?;
    Ok(())
}

/// Run `f` with the TUI suspended (lazygit/gitui-style): leave the alt
/// screen + disable raw mode, run `f` against the real terminal, then
/// **always** restore the TUI before returning — even if `suspend` itself
/// fails, or `f` returns `Err`. This is the only way a confirmed
/// `tui::app::Action` may run its `commands::*` call (see `tui::mod::run`);
/// it guarantees the user is never left in a broken raw-mode/alt-screen
/// terminal no matter how the command finishes.
pub fn with_suspended<T>(terminal: &mut Tui, f: impl FnOnce() -> Result<T>) -> Result<T> {
    let suspend_result = suspend();

    // Only run `f` if suspend actually succeeded — but either way fall
    // through to `resume` below unconditionally, so a partially-failed
    // suspend can't skip the restore.
    let run_result = match suspend_result {
        Ok(()) => f(),
        Err(e) => Err(e),
    };

    let resume_result = resume(terminal);

    match run_result {
        // Command succeeded: surface a resume failure if there was one,
        // otherwise return the command's value.
        Ok(value) => resume_result.map(|()| value),
        // Command (or suspend) failed: that error is what the caller needs
        // to see. We still attempted `resume` above unconditionally; a
        // secondary resume failure is swallowed here so it doesn't mask the
        // more useful original error.
        Err(e) => Err(e),
    }
}
