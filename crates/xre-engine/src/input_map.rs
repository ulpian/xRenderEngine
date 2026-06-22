//! Action-based [`InputMap`] with contexts and edge/level semantics (Stage 5.4).
//!
//! Bindings map raw input to named `Action`s, fixing the hard-coded WASD/ZQSD
//! pattern of the research sources. Held state comes from a per-key *down-set*
//! rather than from "an action fired this frame", so **genuinely simultaneous
//! keys** (e.g. W+D for diagonal movement) are tracked correctly.
//!
//! Two regimes feed the down-set:
//! - **Kitty protocol active** (call [`InputMap::set_release_reporting`] with
//!   `true`): key *release* events are authoritative — a key is held from press
//!   until release, with no time decay.
//! - **No protocol** (the default): terminals never report releases and only
//!   auto-repeat the *last* key, so held state is synthesised — a key lingers for
//!   a short grace window after its last press/repeat and is then released. This
//!   smooths single-key movement; for robust multi-key input on such terminals,
//!   pair it with [`crate::LatchAxis`].
//!
//! Edge queries (`pressed`/`released`) survive level collapse: a fast tap that
//! presses *and* releases within one frame still reports `pressed`
//! (`RiftEngine-Plan/10-phase-5-game-engine.md` §5.4).

use std::collections::{HashMap, HashSet};

use xre_term::{Event, KeyCode, KeyState, Modifiers, MouseButton, MouseKind};

/// Default grace window (seconds) for synthesised holds when key releases are
/// not reported by the terminal — long enough to bridge the auto-repeat gap.
const GRACE_SECS: f32 = 0.15;

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

/// A key currently down, with the modifiers it arrived with and the grace
/// seconds remaining before a synthesised release (`f32::INFINITY` when the
/// terminal reports releases, so only a real release clears it).
#[derive(Clone, Copy, Debug)]
struct KeyHold {
    mods: Modifiers,
    grace: f32,
}

/// Maps raw input to named actions with edge/level query.
#[derive(Debug, Default)]
pub struct InputMap {
    contexts: HashMap<String, Context>,
    stack: Vec<String>,
    /// Keys currently down (or within their grace window). Source of truth for
    /// keyboard `held` state.
    down: HashMap<KeyCode, KeyHold>,
    /// Mouse buttons currently down. Source of truth for mouse `held` state.
    mouse_down: HashSet<MouseButton>,
    /// Actions held at the end of last frame (snapshot taken in `begin_frame`).
    previous: HashSet<String>,
    /// Actions that saw a press/down edge this frame (survives same-frame release).
    pressed_edges: HashSet<String>,
    /// Actions that received *any* press or auto-repeat this frame (not just the
    /// fresh edge). Drives repeat-friendly actions like a movement latch.
    signal_edges: HashSet<String>,
    /// Actions that saw a release/up edge this frame.
    released_edges: HashSet<String>,
    /// Whether the terminal reports key releases (kitty protocol active).
    release_reporting: bool,
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

    /// Tell the map whether the terminal reports key *releases* (kitty protocol).
    ///
    /// When `true`, held keys persist until their release event with no decay;
    /// when `false` (the default), held state is synthesised with a grace window.
    /// Wire this from [`xre_term::TerminalGuard::keyboard_enhanced`].
    pub const fn set_release_reporting(&mut self, on: bool) {
        self.release_reporting = on;
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

    /// Start a new input frame: snapshot held actions for edge detection, decay
    /// the grace windows by `dt` seconds, and clear this frame's edges.
    ///
    /// Pass the simulation `dt`; under a fixed timestep the decay is
    /// deterministic (bit-identical holds across runs).
    pub fn begin_frame(&mut self, dt: f32) {
        // Snapshot end-of-last-frame held actions before advancing time. Reuse the
        // `previous` set's allocation (clear + refill) rather than building a fresh
        // HashSet each frame — keeps the steady-state loop allocation-free.
        let mut prev = std::mem::take(&mut self.previous);
        prev.clear();
        self.fill_held_actions(&mut prev);
        self.previous = prev;
        // Decay grace windows; keys whose grace runs out are released. Entries
        // with `INFINITY` grace (release-reporting on) never expire here.
        self.down.retain(|_, h| {
            h.grace -= dt;
            h.grace > 0.0
        });
        self.pressed_edges.clear();
        self.signal_edges.clear();
        self.released_edges.clear();
    }

    /// Feed one normalized [`Event`], updating the down-set and edges.
    pub fn feed(&mut self, event: &Event) {
        let Some(active) = self.stack.last().cloned() else {
            return;
        };
        match event {
            Event::Key(k) => match k.state {
                KeyState::Press => {
                    // A press/repeat always "signals" (drives the latch); only a
                    // *fresh* press is a rising edge. Terminals without the kitty
                    // protocol resend auto-repeats as `Press`, so guarding the edge
                    // on "not already down" stops `pressed` firing every repeat.
                    let fresh = !self.down.contains_key(&k.code);
                    self.down.insert(k.code, self.new_hold(k.mods));
                    let actions = self.matched_actions(&active, |b| key_matches(b, k.code, k.mods));
                    self.signal_edges.extend(actions.iter().cloned());
                    if fresh {
                        self.pressed_edges.extend(actions);
                    }
                }
                KeyState::Repeat => {
                    // Refresh the grace window and signal (not a fresh press).
                    self.down.insert(k.code, self.new_hold(k.mods));
                    let actions = self.matched_actions(&active, |b| key_matches(b, k.code, k.mods));
                    self.signal_edges.extend(actions);
                }
                KeyState::Release => {
                    self.down.remove(&k.code);
                    let actions = self.matched_actions(&active, |b| key_matches(b, k.code, k.mods));
                    self.released_edges.extend(actions);
                }
                _ => {}
            },
            Event::Mouse(m) => match m.kind {
                MouseKind::Down(btn) => {
                    self.mouse_down.insert(btn);
                    let actions = self.matched_actions(&active, |b| mouse_matches(b, btn));
                    self.signal_edges.extend(actions.iter().cloned());
                    self.pressed_edges.extend(actions);
                }
                MouseKind::Up(btn) => {
                    self.mouse_down.remove(&btn);
                    let actions = self.matched_actions(&active, |b| mouse_matches(b, btn));
                    self.released_edges.extend(actions);
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// Whether `action` is held this frame (any bound key/button is down).
    #[must_use]
    pub fn held(&self, action: &str) -> bool {
        self.active_bindings(action)
            .is_some_and(|bindings| bindings.iter().any(|b| self.binding_down(b)))
    }

    /// Whether `action` became active this frame (edge: rising). Fires even for a
    /// press+release within the same frame.
    #[must_use]
    pub fn pressed(&self, action: &str) -> bool {
        self.pressed_edges.contains(action)
            || (self.held(action) && !self.previous.contains(action))
    }

    /// Whether `action` received a press *or auto-repeat* this frame.
    ///
    /// Unlike [`pressed`](Self::pressed) (the fresh rising edge), this fires on
    /// every repeat, so a re-pressed or held key re-asserts each frame. Use it
    /// for repeat-friendly actions like a movement latch, where a held or
    /// re-pressed direction must keep (re)setting its value.
    #[must_use]
    pub fn pressed_repeat(&self, action: &str) -> bool {
        self.signal_edges.contains(action)
    }

    /// Whether `action` stopped being active this frame (edge: falling).
    #[must_use]
    pub fn released(&self, action: &str) -> bool {
        !self.held(action)
            && (self.released_edges.contains(action) || self.previous.contains(action))
    }

    /// A `-1.0 / 0.0 / +1.0` axis from a negative/positive action pair (held).
    #[must_use]
    pub fn axis(&self, negative: &str, positive: &str) -> f32 {
        f32::from(self.held(positive)) - f32::from(self.held(negative))
    }

    /// A fresh hold for a key press/repeat: infinite grace when releases are
    /// authoritative, else the finite synthesis window.
    const fn new_hold(&self, mods: Modifiers) -> KeyHold {
        KeyHold {
            mods,
            grace: if self.release_reporting {
                f32::INFINITY
            } else {
                GRACE_SECS
            },
        }
    }

    /// The bindings for `action` in the active context, if any.
    fn active_bindings(&self, action: &str) -> Option<&Vec<Binding>> {
        let active = self.stack.last()?;
        self.contexts.get(active)?.get(action)
    }

    /// Whether a single binding's key/button is currently down.
    fn binding_down(&self, b: &Binding) -> bool {
        match b {
            Binding::Key(code, mods) => self.down.get(code).is_some_and(|h| h.mods.contains(*mods)),
            Binding::Mouse(btn) => self.mouse_down.contains(btn),
        }
    }

    /// Fill `set` with the actions currently held in the active context, reusing
    /// the caller's allocation (the per-frame snapshot in [`Self::begin_frame`]).
    fn fill_held_actions(&self, set: &mut HashSet<String>) {
        if let Some(active) = self.stack.last() {
            if let Some(context) = self.contexts.get(active) {
                for (action, bindings) in context {
                    if bindings.iter().any(|b| self.binding_down(b)) {
                        set.insert(action.clone());
                    }
                }
            }
        }
    }

    /// Every action in the active context with a binding matching `pred`.
    fn matched_actions(&self, active: &str, pred: impl Fn(&Binding) -> bool) -> Vec<String> {
        self.contexts.get(active).map_or_else(Vec::new, |context| {
            context
                .iter()
                .filter(|(_, bindings)| bindings.iter().any(&pred))
                .map(|(action, _)| action.clone())
                .collect()
        })
    }
}

fn key_matches(b: &Binding, code: KeyCode, mods: Modifiers) -> bool {
    matches!(b, Binding::Key(c, m) if *c == code && mods.contains(*m))
}

fn mouse_matches(b: &Binding, btn: MouseButton) -> bool {
    matches!(b, Binding::Mouse(x) if *x == btn)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    use super::*;
    use xre_term::Key;

    const DT: f32 = 0.016;

    fn key_event(c: char) -> Event {
        key_state(c, KeyState::Press)
    }

    fn key_state(c: char, state: KeyState) -> Event {
        Event::Key(Key {
            code: KeyCode::Char(c),
            mods: Modifiers::NONE,
            state,
        })
    }

    fn map() -> InputMap {
        let mut m = InputMap::new();
        m.bind("game", "left", Binding::key(KeyCode::Char('a')));
        m.bind("game", "right", Binding::key(KeyCode::Char('d')));
        m.bind("game", "up", Binding::key(KeyCode::Char('w')));
        m.bind("game", "jump", Binding::key(KeyCode::Char(' ')));
        m.push_context("game");
        m
    }

    #[test]
    fn pressed_then_held_then_released() {
        // No-protocol path: held is synthesised from press/repeat + grace decay.
        let mut m = map();
        m.begin_frame(DT);
        m.feed(&key_event('a'));
        assert!(m.pressed("left"));
        assert!(m.held("left"));
        assert!(!m.released("left"));

        // Next frame, the key repeats → still held, not a fresh press.
        m.begin_frame(DT);
        m.feed(&key_state('a', KeyState::Repeat));
        assert!(!m.pressed("left"));
        assert!(m.held("left"));

        // Stop feeding: within grace it lingers...
        m.begin_frame(DT);
        assert!(m.held("left"));
        // ...then a frame that advances past the grace releases it.
        m.begin_frame(GRACE_SECS);
        assert!(!m.held("left"));
        assert!(m.released("left"));
    }

    #[test]
    fn release_event_drops_held_immediately() {
        // Protocol path: held persists with no decay until the release arrives.
        let mut m = map();
        m.set_release_reporting(true);
        m.begin_frame(DT);
        m.feed(&key_event('a'));
        assert!(m.held("left"));

        // Many idle frames pass with no events — still held (no spurious decay).
        for _ in 0..100 {
            m.begin_frame(DT);
        }
        assert!(m.held("left"));

        m.feed(&key_state('a', KeyState::Release));
        assert!(!m.held("left"));
        assert!(m.released("left"));
    }

    #[test]
    fn simultaneous_keys_are_both_held() {
        // The headline: W and D held at once → both axes active (diagonal).
        let mut m = map();
        m.set_release_reporting(true);
        m.begin_frame(DT);
        m.feed(&key_event('w'));
        m.feed(&key_event('d'));
        assert!(m.held("up"));
        assert!(m.held("right"));
        assert_eq!(m.axis("left", "right"), 1.0);

        // Releasing one leaves the other held.
        m.feed(&key_state('w', KeyState::Release));
        assert!(!m.held("up"));
        assert!(m.held("right"));
    }

    #[test]
    fn same_frame_tap_registers_pressed() {
        // Press and release within one frame must still report a rising edge.
        let mut m = map();
        m.set_release_reporting(true);
        m.begin_frame(DT);
        m.feed(&key_event('a'));
        m.feed(&key_state('a', KeyState::Release));
        assert!(
            m.pressed("left"),
            "a fast tap must still register as pressed"
        );
        assert!(!m.held("left"));
        assert!(m.released("left"));
    }

    #[test]
    fn axis_from_key_pair() {
        let mut m = map();
        m.set_release_reporting(true);
        m.begin_frame(DT);
        m.feed(&key_event('d'));
        assert_eq!(m.axis("left", "right"), 1.0);
        m.feed(&key_state('d', KeyState::Release));
        m.feed(&key_event('a'));
        assert_eq!(m.axis("left", "right"), -1.0);
    }

    #[test]
    fn deterministic_under_fixed_dt() {
        // The same event/dt script yields identical edge/level booleans each run.
        let run = || {
            let mut m = map();
            let mut trace = Vec::new();
            for frame in 0..8 {
                m.begin_frame(DT);
                match frame {
                    0 => m.feed(&key_event('w')),
                    1 => m.feed(&key_state('w', KeyState::Repeat)),
                    2 => {
                        m.feed(&key_event('d'));
                    }
                    4 => m.feed(&key_state('d', KeyState::Repeat)),
                    _ => {}
                }
                for action in ["up", "right"] {
                    trace.push((m.held(action), m.pressed(action), m.released(action)));
                }
            }
            trace
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn rebind_replaces_bindings() {
        let mut m = map();
        m.rebind("game", "jump", Binding::key(KeyCode::Char('w')));
        m.begin_frame(DT);
        m.feed(&key_event(' ')); // old binding no longer active
        assert!(!m.held("jump"));
        m.begin_frame(DT);
        m.feed(&key_event('w'));
        assert!(m.held("jump"));
    }

    #[test]
    fn contexts_isolate_bindings() {
        let mut m = InputMap::new();
        m.bind("menu", "confirm", Binding::key(KeyCode::Enter));
        m.bind("game", "fire", Binding::key(KeyCode::Char('f')));
        m.push_context("menu");
        m.begin_frame(DT);
        m.feed(&key_event('f')); // 'f' is a game binding, inactive in menu
        assert!(!m.held("fire"));
        m.push_context("game");
        m.begin_frame(DT);
        m.feed(&key_event('f'));
        assert!(m.held("fire"));
        assert_eq!(m.pop_context().as_deref(), Some("game"));
        assert_eq!(m.active_context(), Some("menu"));
    }
}
