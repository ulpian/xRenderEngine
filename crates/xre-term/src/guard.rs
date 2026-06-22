//! RAII terminal lifecycle: raw mode + alternate screen, restored on drop and
//! on panic.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Once;

use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, supports_keyboard_enhancement, EnterAlternateScreen,
    LeaveAlternateScreen,
};

use crate::error::Result;

static PANIC_HOOK: Once = Once::new();

/// Whether the live guard enabled mouse capture. Read by the panic hook (which
/// has no access to the guard value) so an unwinding panic disables capture too,
/// rather than leaving the shell spewing mouse escape sequences.
static MOUSE_CAPTURED: AtomicBool = AtomicBool::new(false);

/// Whether the live guard pushed keyboard-enhancement flags. Read by the panic
/// hook so an unwinding panic pops them too, rather than leaving the shell with
/// the kitty protocol active.
static KBD_ENHANCED: AtomicBool = AtomicBool::new(false);

/// Whether the live guard set the OSC 22 mouse-pointer shape, so the panic hook
/// resets it too rather than leaving the shell with an overridden pointer.
static POINTER_SET: AtomicBool = AtomicBool::new(false);

/// The keyboard-enhancement flags we request when the protocol is enabled.
///
/// `REPORT_EVENT_TYPES` adds press/repeat/release event kinds, and
/// `REPORT_ALL_KEYS_AS_ESCAPE_CODES` is required for *plain-text* keys (WASD) to
/// report releases at all — together they let the input map track genuinely
/// simultaneous held keys (e.g. W+D for diagonal movement).
/// `DISAMBIGUATE_ESCAPE_CODES` is deliberately omitted so a lone <kbd>Esc</kbd>
/// still reads as Esc (menus rely on it).
const fn enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        .union(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES)
}

/// Options controlling what [`TerminalGuard::enter_with`] switches on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuardOptions {
    /// Enable mouse reporting (button presses, drags and scroll) so the
    /// application receives [`crate::Event::Mouse`] events.
    ///
    /// While capture is on, the terminal forwards mouse actions to the program
    /// instead of performing native click-drag text selection; users can still
    /// select text by holding **Shift** (most terminals) or **Option** (macOS).
    pub mouse: bool,
    /// Enable the keyboard enhancement (kitty) protocol when the terminal
    /// supports it, so key *release* events are reported and genuinely
    /// simultaneous held keys (e.g. W+D) can be tracked.
    ///
    /// Falls back silently to press-only input on terminals without the
    /// protocol; query [`TerminalGuard::keyboard_enhanced`] to learn whether it
    /// actually took effect.
    pub keyboard_enhancement: bool,
    /// Request a mouse-pointer shape via the OSC 22 sequence (an X11 cursor name
    /// such as `"default"` for an arrow or `"pointer"` for a hand), so the
    /// terminal shows a select pointer instead of the text I-beam over the app.
    ///
    /// Best-effort: terminals that don't understand OSC 22 ignore it, and the
    /// shape is reset on restore. `None` leaves the pointer untouched.
    pub pointer_shape: Option<&'static str>,
}

impl Default for GuardOptions {
    /// Mouse capture and keyboard enhancement **on**, and an arrow mouse pointer
    /// requested — the engine is mouse-interactive and tracks held keys by
    /// default, so a select pointer fits better than the text I-beam.
    fn default() -> Self {
        Self {
            mouse: true,
            keyboard_enhancement: true,
            pointer_shape: Some("default"),
        }
    }
}

/// An RAII guard owning the terminal's raw-mode + alternate-screen state.
///
/// [`TerminalGuard::enter`] switches the terminal into raw mode on the alternate
/// screen, hides the cursor and (by default) enables mouse capture and the
/// keyboard enhancement protocol where supported. The original state is restored
/// when the guard is dropped — and a panic hook is installed so an unwinding
/// panic restores the terminal too, rather than leaving the user staring at a
/// wrecked shell.
///
/// Use [`TerminalGuard::enter_with`] with [`GuardOptions`] to opt out of mouse
/// capture (e.g. to preserve native terminal text selection) or keyboard
/// enhancement, and [`TerminalGuard::keyboard_enhanced`] to learn whether the
/// protocol took effect.
#[must_use = "dropping the guard immediately restores the terminal"]
pub struct TerminalGuard {
    mouse: bool,
    kbd: bool,
    pointer: bool,
}

impl TerminalGuard {
    /// Enter raw mode and the alternate screen with the default options
    /// (mouse capture **on**), hiding the cursor.
    ///
    /// # Errors
    /// Returns [`crate::TermError::Io`] if the terminal cannot be reconfigured.
    pub fn enter() -> Result<Self> {
        Self::enter_with(GuardOptions::default())
    }

    /// Enter raw mode and the alternate screen with explicit [`GuardOptions`].
    ///
    /// # Errors
    /// Returns [`crate::TermError::Io`] if the terminal cannot be reconfigured.
    pub fn enter_with(opts: GuardOptions) -> Result<Self> {
        install_panic_hook();
        enable_raw_mode()?;
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen, Hide)?;
        MOUSE_CAPTURED.store(opts.mouse, Ordering::SeqCst);
        if opts.mouse {
            execute!(out, EnableMouseCapture)?;
        }
        // `supports_keyboard_enhancement` does a terminal round-trip, so only
        // probe when the caller asked for it. Store the *actual* outcome so Drop
        // and the panic hook pop the flags only if we pushed them.
        let kbd = opts.keyboard_enhancement && supports_keyboard_enhancement().unwrap_or(false);
        KBD_ENHANCED.store(kbd, Ordering::SeqCst);
        if kbd {
            execute!(out, PushKeyboardEnhancementFlags(enhancement_flags()))?;
        }
        // Request a mouse-pointer shape (OSC 22). Unsupported terminals ignore
        // the sequence; reset on restore so the shell's pointer is untouched.
        POINTER_SET.store(opts.pointer_shape.is_some(), Ordering::SeqCst);
        if let Some(shape) = opts.pointer_shape {
            write!(out, "\x1b]22;{shape}\x1b\\")?;
        }
        out.flush()?;
        Ok(Self {
            mouse: opts.mouse,
            kbd,
            pointer: opts.pointer_shape.is_some(),
        })
    }

    /// Whether the keyboard enhancement (kitty) protocol was successfully
    /// enabled.
    ///
    /// When `true`, key *release* events are delivered and held keys can be
    /// tracked exactly; when `false`, the host terminal reports presses only and
    /// callers should fall back (e.g. a latch or grace window) for robust
    /// multi-key input.
    #[must_use]
    pub const fn keyboard_enhanced(&self) -> bool {
        self.kbd
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        // Best-effort: nothing useful to do with an error while tearing down.
        let _ = restore(self.mouse, self.kbd, self.pointer);
        MOUSE_CAPTURED.store(false, Ordering::SeqCst);
        KBD_ENHANCED.store(false, Ordering::SeqCst);
        POINTER_SET.store(false, Ordering::SeqCst);
    }
}

/// Pop keyboard-enhancement flags, disable mouse capture and reset the pointer
/// shape (if they were on), leave the alternate screen, show the cursor and
/// disable raw mode.
///
/// Enhancement flags are popped and mouse capture disabled *before* leaving the
/// alternate screen so no stray reporting escapes leak into the user's shell.
fn restore(mouse: bool, kbd: bool, pointer: bool) -> Result<()> {
    let mut out = io::stdout();
    if kbd {
        execute!(out, PopKeyboardEnhancementFlags)?;
    }
    if mouse {
        execute!(out, DisableMouseCapture)?;
    }
    if pointer {
        // OSC 22 with an empty shape resets the pointer to the terminal default.
        write!(out, "\x1b]22;\x1b\\")?;
    }
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
            let _ = restore(
                MOUSE_CAPTURED.load(Ordering::SeqCst),
                KBD_ENHANCED.load(Ordering::SeqCst),
                POINTER_SET.load(Ordering::SeqCst),
            );
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhancement_flags_are_event_types_and_all_keys() {
        let flags = enhancement_flags();
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_EVENT_TYPES));
        assert!(flags.contains(KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES));
        // Esc disambiguation stays off so menus keep their lone-Esc behavior.
        assert!(!flags.contains(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES));
    }

    #[test]
    fn default_options_request_an_arrow_pointer() {
        // The engine is mouse-interactive: a select pointer beats the I-beam.
        assert_eq!(GuardOptions::default().pointer_shape, Some("default"));
    }
}
