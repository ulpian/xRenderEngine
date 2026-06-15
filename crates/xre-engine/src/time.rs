//! Loop timing: the [`FixedTimestep`] accumulator, [`FramePacer`], and the
//! [`Time`] resource (Stage 5.1).
//!
//! Updates run at a fixed `dt` (default 60 Hz) via an accumulator, with the
//! render interpolating by [`Time::alpha`]; the accumulator is clamped to bound
//! the spiral of death. A fixed `dt` plus a recorded input stream yields a
//! bit-identical state hash — the determinism guarantee that makes replays and
//! golden tests possible (`RiftEngine-Plan/10-phase-5-game-engine.md` §5.1).

use std::time::{Duration, Instant};

/// Per-frame timing made available to game code.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Time {
    /// The fixed update timestep, seconds.
    pub dt: f32,
    /// Total simulated time, seconds.
    pub elapsed: f32,
    /// The update tick counter.
    pub frame: u64,
    /// Interpolation factor `0.0..=1.0` between the last two fixed states, for
    /// the renderer to smooth motion.
    pub alpha: f32,
}

impl Time {
    /// A zeroed `Time` for the given fixed `dt`.
    #[must_use]
    pub const fn new(dt: f32) -> Self {
        Self {
            dt,
            elapsed: 0.0,
            frame: 0,
            alpha: 0.0,
        }
    }
}

/// A fixed-timestep accumulator.
#[derive(Clone, Copy, Debug)]
pub struct FixedTimestep {
    step: f32,
    accumulator: f32,
    max_steps: u32,
    elapsed: f32,
    frame: u64,
}

impl FixedTimestep {
    /// A timestep running at `hz` updates per second (clamped sane).
    #[must_use]
    pub fn new(hz: f32) -> Self {
        let step = 1.0 / hz.clamp(1.0, 1000.0);
        Self {
            step,
            accumulator: 0.0,
            max_steps: 8,
            elapsed: 0.0,
            frame: 0,
        }
    }

    /// The fixed step length in seconds.
    #[must_use]
    pub const fn step(&self) -> f32 {
        self.step
    }

    /// Feed a wall-clock frame delta and return how many fixed updates to run.
    ///
    /// The accumulator is clamped to `max_steps * step` so a long stall (a debug
    /// breakpoint, a GC pause) cannot trigger an unbounded catch-up burst — the
    /// spiral-of-death guard.
    pub fn advance(&mut self, frame_dt: f32) -> u32 {
        // Ignore negative/NaN deltas defensively.
        let frame_dt = if frame_dt.is_finite() {
            frame_dt.max(0.0)
        } else {
            0.0
        };
        self.accumulator += frame_dt;
        let cap = self.max_steps as f32 * self.step;
        if self.accumulator > cap {
            self.accumulator = cap;
        }
        let mut steps = 0;
        while self.accumulator >= self.step && steps < self.max_steps {
            self.accumulator -= self.step;
            self.elapsed += self.step;
            self.frame += 1;
            steps += 1;
        }
        steps
    }

    /// The current render interpolation alpha.
    #[must_use]
    pub fn alpha(&self) -> f32 {
        (self.accumulator / self.step).clamp(0.0, 1.0)
    }

    /// Snapshot the current [`Time`] resource.
    #[must_use]
    pub fn time(&self) -> Time {
        Time {
            dt: self.step,
            elapsed: self.elapsed,
            frame: self.frame,
            alpha: self.alpha(),
        }
    }
}

/// Sleeps to hold a target frame rate, with a short spin tail for precision
/// (the Command_Line_3D `PaceMaker`, OS-precision-tuned).
#[derive(Clone, Copy, Debug)]
pub struct FramePacer {
    target: Duration,
}

impl FramePacer {
    /// A pacer targeting `fps` frames per second.
    #[must_use]
    pub fn new(fps: f32) -> Self {
        let secs = 1.0 / fps.clamp(1.0, 1000.0);
        Self {
            target: Duration::from_secs_f32(secs),
        }
    }

    /// The remaining budget after `frame_start`, suitable as an input poll
    /// timeout. Zero if the frame already overran.
    #[must_use]
    pub fn remaining(&self, frame_start: Instant) -> Duration {
        self.target
            .checked_sub(frame_start.elapsed())
            .unwrap_or(Duration::ZERO)
    }

    /// Sleep until `frame_start + target`. Uses a coarse sleep then a short busy
    /// spin for the last millisecond to land precisely.
    pub fn pace(&self, frame_start: Instant) {
        let deadline = frame_start + self.target;
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let remaining = deadline.saturating_duration_since(now);
        if remaining > Duration::from_millis(2) {
            std::thread::sleep(remaining.saturating_sub(Duration::from_millis(1)));
        }
        while Instant::now() < deadline {
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp, clippy::unwrap_used)]
    use super::*;

    #[test]
    fn accumulator_runs_expected_steps() {
        let mut ts = FixedTimestep::new(100.0); // exact 10 ms step
                                                // A 35 ms frame → 3 whole steps, 5 ms left over.
        assert_eq!(ts.advance(0.035), 3);
        assert_eq!(ts.time().frame, 3);
        assert!((ts.alpha() - 0.5).abs() < 1e-3);
    }

    #[test]
    fn spiral_of_death_is_clamped() {
        let mut ts = FixedTimestep::new(60.0);
        // A 10-second stall must not run hundreds of steps.
        let steps = ts.advance(10.0);
        assert!(steps <= 8, "ran {steps} steps, expected the cap");
    }

    #[test]
    fn alpha_tracks_partial_accumulation() {
        let mut ts = FixedTimestep::new(100.0); // 10 ms step
        ts.advance(0.005); // half a step
        assert!((ts.alpha() - 0.5).abs() < 1e-4);
    }

    #[test]
    fn determinism_same_input_same_state() {
        // Same dt sequence → identical accumulated frame count and elapsed time.
        let seq = [0.016f32, 0.02, 0.001, 0.05, 0.0166, 0.033];
        let run = || {
            let mut ts = FixedTimestep::new(60.0);
            let mut total = 0u32;
            for &d in &seq {
                total += ts.advance(d);
            }
            (total, ts.time().frame, ts.time().elapsed.to_bits())
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn negative_and_nan_deltas_are_ignored() {
        let mut ts = FixedTimestep::new(60.0);
        assert_eq!(ts.advance(f32::NAN), 0);
        assert_eq!(ts.advance(-1.0), 0);
        assert_eq!(ts.time().frame, 0);
    }

    #[test]
    fn pacer_remaining_is_zero_after_overrun() {
        let pacer = FramePacer::new(60.0);
        let start = Instant::now()
            .checked_sub(Duration::from_millis(100))
            .unwrap();
        assert_eq!(pacer.remaining(start), Duration::ZERO);
    }
}
