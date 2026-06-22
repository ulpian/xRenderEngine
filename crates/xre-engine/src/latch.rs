//! [`LatchAxis`] — a sticky direction for terminals that cannot report held keys.
//!
//! Without the kitty keyboard protocol a terminal never sends key *releases* and
//! only auto-repeats the *last* key, so two keys (e.g. W+D) can never read as
//! simultaneously held. A latch sidesteps this: a directional *press* **sets** a
//! sticky `-1 / 0 / +1` value that persists until it is set the other way or
//! cleared. Because setting is idempotent, *holding* a key (whose auto-repeat
//! arrives as a stream of presses) keeps the same value rather than flipping it,
//! and two perpendicular latches stay set together for continuous diagonal
//! movement on any terminal.
//!
//! Drive it from rising edges ([`InputMap::pressed`](crate::InputMap::pressed)),
//! which fire on press and on auto-repeat even on press-only terminals.

/// A sticky `-1 / 0 / +1` axis set by directional presses.
///
/// ```
/// use xre_engine::LatchAxis;
/// let mut strafe = LatchAxis::default();
/// strafe.set_positive(); // press (or hold) D → +1 (move right)
/// assert_eq!(strafe.value(), 1.0);
/// strafe.set_positive(); // holding re-fires, but stays +1 (idempotent)
/// assert_eq!(strafe.value(), 1.0);
/// strafe.set_negative(); // press A → -1 (reverse to left)
/// assert_eq!(strafe.value(), -1.0);
/// strafe.clear();        // a dedicated "stop" key halts the axis
/// assert_eq!(strafe.value(), 0.0);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LatchAxis {
    /// Invariant: always one of `-1`, `0`, `1`.
    value: i8,
}

impl LatchAxis {
    /// Latch the positive direction (`+1`). Idempotent, so holding the key keeps
    /// it set rather than flipping it.
    pub const fn set_positive(&mut self) {
        self.value = 1;
    }

    /// Latch the negative direction (`-1`). Idempotent.
    pub const fn set_negative(&mut self) {
        self.value = -1;
    }

    /// Reset to the neutral (`0`) position.
    pub const fn clear(&mut self) {
        self.value = 0;
    }

    /// `true` when latched in either direction.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.value != 0
    }

    /// The current value as `-1.0 / 0.0 / +1.0`, ready to scale a velocity.
    #[must_use]
    pub fn value(&self) -> f32 {
        f32::from(self.value)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    use super::*;

    #[test]
    fn sets_reverses_and_clears() {
        let mut a = LatchAxis::default();
        assert_eq!(a.value(), 0.0);
        assert!(!a.is_active());

        a.set_positive();
        assert_eq!(a.value(), 1.0);
        assert!(a.is_active());

        // Setting the same direction again is idempotent (hold-safe).
        a.set_positive();
        assert_eq!(a.value(), 1.0);

        // The opposite direction reverses.
        a.set_negative();
        assert_eq!(a.value(), -1.0);

        a.clear();
        assert_eq!(a.value(), 0.0);
        assert!(!a.is_active());
    }
}
