//! RAII terminal lifecycle: raw mode + alternate screen, restored on drop and
//! on panic.

use std::io::{self, Write};
use std::sync::Once;

use crossterm::cursor::{Hide, Show};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};

use crate::error::Result;

static PANIC_HOOK: Once = Once::new();

/// An RAII guard owning the terminal's raw-mode + alternate-screen state.
///
/// [`TerminalGuard::enter`] switches the terminal into raw mode on the alternate
/// screen and hides the cursor. The original state is restored when the guard is
/// dropped — and a panic hook is installed so an unwinding panic restores the
/// terminal too, rather than leaving the user staring at a wrecked shell.
#[must_use = "dropping the guard immediately restores the terminal"]
pub struct TerminalGuard;

impl TerminalGuard {
    /// Enter raw mode and the alternate screen, hiding the cursor.
    ///
    /// # Errors
    /// Returns [`crate::TermError::Io`] if the terminal cannot be reconfigured.
    pub fn enter() -> Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, Hide)?;
        out.flush()?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort: nothing useful to do with an error while tearing down.
        let _ = restore();
    }
}

/// Leave the alternate screen, show the cursor and disable raw mode.
fn restore() -> Result<()> {
    let mut out = io::stdout();
    execute!(out, LeaveAlternateScreen, Show)?;
    disable_raw_mode()?;
    out.flush()?;
    Ok(())
}

/// Install (once) a panic hook that restores the terminal before delegating to
/// the previous hook so the panic message is still printed.
fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = restore();
            previous(info);
        }));
    });
}
