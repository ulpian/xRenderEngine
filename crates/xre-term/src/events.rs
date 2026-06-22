//! Normalized input events and the frame-coherent [`EventQueue`].
//!
//! Crossterm's platform-specific events (Windows VT vs. unix escape sequences)
//! are normalized into a small, stable vocabulary *before* they reach the
//! application (see `RiftEngine-Plan/06-phase-1-tui-core.md` §1.2). The
//! conversion is the pure function [`Event::from_crossterm`], so the
//! normalization layer is unit-tested without a live terminal.
//!
//! [`EventQueue::pump`] drains *all* currently-available events into a buffer
//! each frame, so every event for frame N is visible at update N — the
//! determinism prerequisite for the Phase 5 game loop.

use std::time::Duration;

use crossterm::event::{self as ct};
use xre_core::math::UVec2;

use crate::error::Result;

/// Keyboard modifier flags, packed into a small bitset.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifiers held.
    pub const NONE: Self = Self(0);
    /// Shift.
    pub const SHIFT: Self = Self(1 << 0);
    /// Control.
    pub const CTRL: Self = Self(1 << 1);
    /// Alt / Option.
    pub const ALT: Self = Self(1 << 2);
    /// Super / Command / Windows.
    pub const SUPER: Self = Self(1 << 3);

    /// `true` if every flag in `other` is held.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// `true` if no modifier is held.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Combine two modifier sets.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::fmt::Debug for Modifiers {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let mut parts = Vec::new();
        if self.contains(Self::CTRL) {
            parts.push("CTRL");
        }
        if self.contains(Self::ALT) {
            parts.push("ALT");
        }
        if self.contains(Self::SHIFT) {
            parts.push("SHIFT");
        }
        if self.contains(Self::SUPER) {
            parts.push("SUPER");
        }
        write!(
            f,
            "Modifiers({})",
            if parts.is_empty() {
                "NONE".to_string()
            } else {
                parts.join("|")
            }
        )
    }
}

/// A logical key, platform quirks erased.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    /// A character key (already case-folded by the terminal per Shift state).
    Char(char),
    /// Enter / Return.
    Enter,
    /// Escape.
    Esc,
    /// Backspace.
    Backspace,
    /// Tab.
    Tab,
    /// Shift-Tab / back-tab.
    BackTab,
    /// Delete (forward).
    Delete,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Function key `F1`..`F24`.
    F(u8),
}

/// Whether a key event is an initial press, an auto-repeat, or a release.
///
/// `Repeat` and `Release` are only distinguishable when the keyboard
/// enhancement (kitty) protocol is active (see
/// [`crate::GuardOptions::keyboard_enhancement`]); without it every key event
/// arrives as [`KeyState::Press`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeyState {
    /// Initial press.
    Press,
    /// Auto-repeat while the key is held.
    Repeat,
    /// Release.
    Release,
}

/// A normalized key event.
///
/// [`Key::state`] distinguishes press, auto-repeat and release. Without the
/// kitty protocol the terminal only reports presses/repeats, so `state` is
/// always [`KeyState::Press`] and held/released state is synthesised from their
/// absence (see `xre-engine`'s `InputMap`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// Which key.
    pub code: KeyCode,
    /// Modifiers held at the time of the event.
    pub mods: Modifiers,
    /// Whether this is a press, auto-repeat or release.
    pub state: KeyState,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// Left button.
    Left,
    /// Right button.
    Right,
    /// Middle button.
    Middle,
}

/// What a mouse event represents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MouseKind {
    /// A button was pressed.
    Down(MouseButton),
    /// A button was released.
    Up(MouseButton),
    /// The mouse moved with a button held.
    Drag(MouseButton),
    /// The mouse moved with no button held.
    Moved,
    /// Scroll wheel up.
    ScrollUp,
    /// Scroll wheel down.
    ScrollDown,
}

/// A normalized mouse event at a cell position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MouseEvent {
    /// What happened.
    pub kind: MouseKind,
    /// Column (cell x).
    pub col: u32,
    /// Row (cell y).
    pub row: u32,
    /// Modifiers held.
    pub mods: Modifiers,
}

/// A normalized input event.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A key was pressed.
    Key(Key),
    /// A mouse action occurred.
    Mouse(MouseEvent),
    /// The terminal was resized to the given cell dimensions.
    Resize(UVec2),
    /// Text was pasted (bracketed paste).
    Paste(String),
    /// The terminal window gained focus.
    FocusGained,
    /// The terminal window lost focus.
    FocusLost,
}

impl Event {
    /// Normalize a crossterm event into an engine [`Event`].
    ///
    /// Returns `None` only for events the engine has no vocabulary for
    /// (unmappable keys, rare mouse kinds). Key releases are carried as
    /// [`KeyState::Release`]; they arrive only when the kitty protocol is active.
    #[must_use]
    pub fn from_crossterm(ev: ct::Event) -> Option<Self> {
        match ev {
            ct::Event::Key(k) => Some(Self::Key(Key {
                code: map_key_code(k.code)?,
                mods: map_mods(k.modifiers),
                state: map_key_state(k.kind),
            })),
            ct::Event::Mouse(m) => Some(Self::Mouse(MouseEvent {
                kind: map_mouse_kind(m.kind)?,
                col: u32::from(m.column),
                row: u32::from(m.row),
                mods: map_mods(m.modifiers),
            })),
            ct::Event::Resize(cols, rows) => {
                Some(Self::Resize(UVec2::new(u32::from(cols), u32::from(rows))))
            }
            ct::Event::Paste(s) => Some(Self::Paste(s)),
            ct::Event::FocusGained => Some(Self::FocusGained),
            ct::Event::FocusLost => Some(Self::FocusLost),
        }
    }
}

const fn map_key_state(kind: ct::KeyEventKind) -> KeyState {
    match kind {
        ct::KeyEventKind::Press => KeyState::Press,
        ct::KeyEventKind::Repeat => KeyState::Repeat,
        ct::KeyEventKind::Release => KeyState::Release,
    }
}

const fn map_mods(m: ct::KeyModifiers) -> Modifiers {
    let mut out = Modifiers::NONE;
    if m.contains(ct::KeyModifiers::SHIFT) {
        out = out.with(Modifiers::SHIFT);
    }
    if m.contains(ct::KeyModifiers::CONTROL) {
        out = out.with(Modifiers::CTRL);
    }
    if m.contains(ct::KeyModifiers::ALT) {
        out = out.with(Modifiers::ALT);
    }
    if m.contains(ct::KeyModifiers::SUPER) {
        out = out.with(Modifiers::SUPER);
    }
    out
}

const fn map_key_code(code: ct::KeyCode) -> Option<KeyCode> {
    Some(match code {
        ct::KeyCode::Char(c) => KeyCode::Char(c),
        ct::KeyCode::Enter => KeyCode::Enter,
        ct::KeyCode::Esc => KeyCode::Esc,
        ct::KeyCode::Backspace => KeyCode::Backspace,
        ct::KeyCode::Tab => KeyCode::Tab,
        ct::KeyCode::BackTab => KeyCode::BackTab,
        ct::KeyCode::Delete => KeyCode::Delete,
        ct::KeyCode::Insert => KeyCode::Insert,
        ct::KeyCode::Home => KeyCode::Home,
        ct::KeyCode::End => KeyCode::End,
        ct::KeyCode::PageUp => KeyCode::PageUp,
        ct::KeyCode::PageDown => KeyCode::PageDown,
        ct::KeyCode::Up => KeyCode::Up,
        ct::KeyCode::Down => KeyCode::Down,
        ct::KeyCode::Left => KeyCode::Left,
        ct::KeyCode::Right => KeyCode::Right,
        ct::KeyCode::F(n) => KeyCode::F(n),
        _ => return None,
    })
}

fn map_mouse_kind(kind: ct::MouseEventKind) -> Option<MouseKind> {
    let button = |b: ct::MouseButton| match b {
        ct::MouseButton::Left => MouseButton::Left,
        ct::MouseButton::Right => MouseButton::Right,
        ct::MouseButton::Middle => MouseButton::Middle,
    };
    Some(match kind {
        ct::MouseEventKind::Down(b) => MouseKind::Down(button(b)),
        ct::MouseEventKind::Up(b) => MouseKind::Up(button(b)),
        ct::MouseEventKind::Drag(b) => MouseKind::Drag(button(b)),
        ct::MouseEventKind::Moved => MouseKind::Moved,
        ct::MouseEventKind::ScrollUp => MouseKind::ScrollUp,
        ct::MouseEventKind::ScrollDown => MouseKind::ScrollDown,
        // ScrollLeft / ScrollRight are dropped (rare, no engine action).
        _ => return None,
    })
}

/// A frame-coherent input queue.
///
/// Call [`EventQueue::pump`] once per frame with the remaining frame budget as
/// the timeout; it blocks up to that long for the *first* event, then drains
/// every event already buffered so the whole frame's input is visible together.
/// [`EventQueue::drain`] hands the events to the application.
#[derive(Default)]
pub struct EventQueue {
    pending: Vec<Event>,
}

impl EventQueue {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Block up to `timeout` for input, then drain all immediately-available
    /// events into the queue. Returns `true` if any event was buffered.
    ///
    /// # Errors
    /// Returns [`crate::TermError::Io`] if polling or reading the terminal fails.
    pub fn pump(&mut self, timeout: Duration) -> Result<bool> {
        let mut any = false;
        if ct::poll(timeout)? {
            loop {
                if let Some(ev) = Event::from_crossterm(ct::read()?) {
                    self.pending.push(ev);
                    any = true;
                }
                if !ct::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
        Ok(any)
    }

    /// Push a synthetic event (for testing or injected input).
    pub fn push(&mut self, ev: Event) {
        self.pending.push(ev);
    }

    /// `true` if there are no buffered events.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Number of buffered events.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.pending.len()
    }

    /// Drain the buffered events in arrival order.
    pub fn drain(&mut self) -> std::vec::Drain<'_, Event> {
        self.pending.drain(..)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn key_press_normalizes() {
        let ev = ct::Event::Key(ct::KeyEvent::new(
            ct::KeyCode::Char('a'),
            ct::KeyModifiers::CONTROL,
        ));
        let out = Event::from_crossterm(ev).unwrap();
        assert_eq!(
            out,
            Event::Key(Key {
                code: KeyCode::Char('a'),
                mods: Modifiers::CTRL,
                state: KeyState::Press,
            })
        );
    }

    #[test]
    fn key_release_is_carried() {
        let mut k = ct::KeyEvent::new(ct::KeyCode::Char('a'), ct::KeyModifiers::NONE);
        k.kind = ct::KeyEventKind::Release;
        assert_eq!(
            Event::from_crossterm(ct::Event::Key(k)),
            Some(Event::Key(Key {
                code: KeyCode::Char('a'),
                mods: Modifiers::NONE,
                state: KeyState::Release,
            }))
        );
    }

    #[test]
    fn key_repeat_carries_repeat_state() {
        let mut k = ct::KeyEvent::new(ct::KeyCode::Char('w'), ct::KeyModifiers::NONE);
        k.kind = ct::KeyEventKind::Repeat;
        assert_eq!(
            Event::from_crossterm(ct::Event::Key(k)),
            Some(Event::Key(Key {
                code: KeyCode::Char('w'),
                mods: Modifiers::NONE,
                state: KeyState::Repeat,
            }))
        );
    }

    #[test]
    fn resize_carries_dimensions() {
        let out = Event::from_crossterm(ct::Event::Resize(120, 40)).unwrap();
        assert_eq!(out, Event::Resize(UVec2::new(120, 40)));
    }

    #[test]
    fn mouse_scroll_and_drag_map() {
        let ev = ct::Event::Mouse(ct::MouseEvent {
            kind: ct::MouseEventKind::ScrollUp,
            column: 3,
            row: 4,
            modifiers: ct::KeyModifiers::NONE,
        });
        let out = Event::from_crossterm(ev).unwrap();
        assert_eq!(
            out,
            Event::Mouse(MouseEvent {
                kind: MouseKind::ScrollUp,
                col: 3,
                row: 4,
                mods: Modifiers::NONE,
            })
        );
    }

    #[test]
    fn queue_drains_in_order() {
        let mut q = EventQueue::new();
        q.push(Event::FocusGained);
        q.push(Event::Resize(UVec2::new(1, 1)));
        assert_eq!(q.len(), 2);
        assert_eq!(q.drain().count(), 2);
        assert!(q.is_empty());
    }
}
