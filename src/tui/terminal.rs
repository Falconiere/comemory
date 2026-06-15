//! RAII terminal lifecycle for the explorer.
//!
//! [`TerminalGuard`] enters raw mode + the alternate screen on the **stderr**
//! render channel (stdout is reserved for the Enter-selected id) and restores
//! both on `Drop` — on every exit path, including an early `Err(..)`, since the
//! gate forbids `panic!`/`expect`/`unwrap` and we cannot rely on unwinding.
//!
//! [`Restore`] is the bare RAII primitive the guard's guarantee rests on: it
//! runs a closure exactly once on drop. It is unit-testable without a tty (the
//! crossterm calls in [`TerminalGuard`] need a real terminal), so the
//! "restore runs on scope exit, including the error path" contract is asserted
//! against `Restore`.

use std::io::{self, Stderr, Write};

use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::prelude::*;

/// Runs `f` exactly once when dropped — the RAII primitive behind
/// [`TerminalGuard`]. Generic over the closure so tests can inject a flag-
/// setter and assert teardown fires on every scope exit.
pub struct Restore<F: FnMut()> {
    f: Option<F>,
}

impl<F: FnMut()> Restore<F> {
    /// Arm the guard with the teardown closure.
    pub fn new(f: F) -> Self {
        Restore { f: Some(f) }
    }
}

impl<F: FnMut()> Drop for Restore<F> {
    fn drop(&mut self) {
        if let Some(mut f) = self.f.take() {
            f();
        }
    }
}

/// Owns the terminal's raw/alt-screen state and the ratatui terminal bound to
/// stderr. `Drop` restores cooked mode + the main screen; restore errors are
/// intentionally swallowed so a failing restore cannot mask the real exit.
pub struct TerminalGuard {
    terminal: Terminal<CrosstermBackend<Stderr>>,
}

impl TerminalGuard {
    /// Enter raw mode + the alternate screen on stderr and build the terminal.
    ///
    /// Each applied step is wrapped in a [`Restore`] *before* the next step is
    /// attempted, so a failure partway through rolls back what already
    /// succeeded (Drop only fires on a fully-constructed guard, so a naive
    /// version would leak raw mode / the alt screen on partial init). On
    /// success the step guards are defused — the returned `TerminalGuard` owns
    /// restoration from then on.
    pub fn enter() -> Result<TerminalGuard> {
        enable_raw_mode().map_err(Error::Io)?;
        let raw = Restore::new(|| {
            let _ = disable_raw_mode();
        });
        execute!(io::stderr(), EnterAlternateScreen).map_err(Error::Io)?;
        let screen = Restore::new(|| {
            let _ = execute!(io::stderr(), LeaveAlternateScreen);
        });
        let terminal = Terminal::new(CrosstermBackend::new(io::stderr())).map_err(Error::Io)?;
        std::mem::forget(raw);
        std::mem::forget(screen);
        Ok(TerminalGuard { terminal })
    }

    /// Mutable access to the ratatui terminal for drawing.
    pub fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stderr>> {
        &mut self.terminal
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stderr(), LeaveAlternateScreen);
        let _ = io::stderr().flush();
    }
}
