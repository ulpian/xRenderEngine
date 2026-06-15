//! Action-based [`InputMap`] with contexts and edge/level semantics (Stage 5.4).
//!
//! Bindings map raw input to named `Action`s, fixing the hard-coded WASD/ZQSD
//! pattern of the research sources. `pressed`/`held`/`released` are computed from
//! the frame-coherent event queue. Terminals without the kitty protocol cannot
//! report key *release*, so `held` is synthesised from press/repeat: an action is
//! held while its key keeps arriving and releases the frame after it stops
//! (`RiftEngine-Plan/10-phase-5-game-engine.md` §5.4).

use std::collections::{HashMap, HashSet};

use xre_term::{Event, KeyCode, Modifiers, MouseButton};

/// A single binding: a key (with required modifiers) or a mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Binding {
    /// A keyboard key with required modifiers.
    Key(KeyCode, Modifiers),
    /// A mouse button.
    Mouse(MouseButton),
}

impl Binding {
    /// A key binding with no modifiers.
    #[must_use]
    pub const fn key(code: KeyCode) -> Self {
        Self::Key(code, Modifiers::NONE)
    }
}

/// A named set of action→bindings, e.g. `"gameplay"` vs `"menu"`.
type Context = HashMap<String, Vec<Binding>>;

/// Maps raw input to named actions with edge/level query.
#[derive(Debug, Default)]
pub struct InputMap {
    contexts: HashMap<String, Context>,
    stack: Vec<String>,
    /// Actions whose input arrived this frame.
    current: HashSet<String>,
    /// Actions that were active last frame.
    previous: HashSet<String>,
}

impl InputMap {
    /// An empty input map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Bind `action` to `binding` within `context` (created on demand).
    pub fn bind(&mut self, context: &str, action: &str, binding: Binding) {
        self.contexts
            .entry(context.to_string())
            .or_default()
            .entry(action.to_string())
            .or_default()
            .push(binding);
    }

    /// Replace all bindings for `action` in `context` with a single `binding`
    /// (the rebinding API).
    pub fn rebind(&mut self, context: &str, action: &str, binding: Binding) {
        self.contexts
            .entry(context.to_string())
            .or_default()
            .insert(action.to_string(), vec![binding]);
    }

    /// Push `context` onto the active stack (its bindings take effect).
    pub fn push_context(&mut self, context: &str) {
        self.stack.push(context.to_string());
    }

    /// Pop the active context.
    pub fn pop_context(&mut self) -> Option<String> {
        self.stack.pop()
    }

    /// The active context name, if any.
    #[must_use]
    pub fn active_context(&self) -> Option<&str> {
        self.stack.last().map(String::as_str)
    }

    /// Start a new input frame: last frame's actions become the baseline.
    pub fn begin_frame(&mut self) {
        std::mem::swap(&mut self.previous, &mut self.current);
        self.current.clear();
    }

    /// Feed one normalized [`Event`], activating any matching actions.
    pub fn feed(&mut self, event: &Event) {
        let Some(active) = self.stack.last() else {
            return;
        };
        let Some(context) = self.contexts.get(active) else {
            return;
        };
        let matches = |b: &Binding| match (b, event) {
            (Binding::Key(code, mods), Event::Key(k)) => k.code == *code && k.mods.contains(*mods),
            (Binding::Mouse(btn), Event::Mouse(m)) => {
                matches!(m.kind, xre_term::MouseKind::Down(b) if b == *btn)
            }
            _ => false,
        };
        for (action, bindings) in context {
            if bindings.iter().any(matches) {
                self.current.insert(action.clone());
            }
        }
    }

    /// Whether `action`'s input arrived this frame (level: held this frame).
    #[must_use]
    pub fn held(&self, action: &str) -> bool {
        self.current.contains(action)
    }

    /// Whether `action` became active this frame (edge: rising).
    #[must_use]
    pub fn pressed(&self, action: &str) -> bool {
        self.current.contains(action) && !self.previous.contains(action)
    }

    /// Whether `action` stopped being active this frame (edge: falling).
    #[must_use]
    pub fn released(&self, action: &str) -> bool {
        !self.current.contains(action) && self.previous.contains(action)
    }

    /// A `-1.0 / 0.0 / +1.0` axis from a negative/positive action pair (held).
    #[must_use]
    pub fn axis(&self, negative: &str, positive: &str) -> f32 {
        f32::from(self.held(positive)) - f32::from(self.held(negative))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    use super::*;
    use xre_term::Key;

    fn key_event(c: char) -> Event {
        Event::Key(Key {
            code: KeyCode::Char(c),
            mods: Modifiers::NONE,
        })
    }

    fn map() -> InputMap {
        let mut m = InputMap::new();
        m.bind("game", "left", Binding::key(KeyCode::Char('a')));
        m.bind("game", "right", Binding::key(KeyCode::Char('d')));
        m.bind("game", "jump", Binding::key(KeyCode::Char(' ')));
        m.push_context("game");
        m
    }

    #[test]
    fn pressed_then_held_then_released() {
        let mut m = map();
        m.begin_frame();
        m.feed(&key_event('a'));
        assert!(m.pressed("left"));
        assert!(m.held("left"));
        assert!(!m.released("left"));

        // Next frame, the key repeats → still held, no longer a fresh press.
        m.begin_frame();
        m.feed(&key_event('a'));
        assert!(!m.pressed("left"));
        assert!(m.held("left"));

        // Next frame, no input → released.
        m.begin_frame();
        assert!(!m.held("left"));
        assert!(m.released("left"));
    }

    #[test]
    fn axis_from_key_pair() {
        let mut m = map();
        m.begin_frame();
        m.feed(&key_event('d'));
        assert_eq!(m.axis("left", "right"), 1.0);
        m.begin_frame();
        m.feed(&key_event('a'));
        assert_eq!(m.axis("left", "right"), -1.0);
    }

    #[test]
    fn rebind_replaces_bindings() {
        let mut m = map();
        m.rebind("game", "jump", Binding::key(KeyCode::Char('w')));
        m.begin_frame();
        m.feed(&key_event(' ')); // old binding no longer active
        assert!(!m.held("jump"));
        m.begin_frame();
        m.feed(&key_event('w'));
        assert!(m.held("jump"));
    }

    #[test]
    fn contexts_isolate_bindings() {
        let mut m = InputMap::new();
        m.bind("menu", "confirm", Binding::key(KeyCode::Enter));
        m.bind("game", "fire", Binding::key(KeyCode::Char('f')));
        m.push_context("menu");
        m.begin_frame();
        m.feed(&key_event('f')); // 'f' is a game binding, inactive in menu
        assert!(!m.held("fire"));
        m.push_context("game");
        m.begin_frame();
        m.feed(&key_event('f'));
        assert!(m.held("fire"));
        assert_eq!(m.pop_context().as_deref(), Some("game"));
        assert_eq!(m.active_context(), Some("menu"));
    }
}
