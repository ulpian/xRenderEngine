//! Keyframe animation: [`Track`]s, [`Clip`]s, [`Animator`]s and [`Tween`]s
//! (Stage 5.3).
//!
//! A [`Track<T>`] interpolates keyframes (T = f32 / `Vec3` / `Quat`, the latter
//! via slerp) with per-segment [`Easing`]. A [`Clip`] bundles tracks into a
//! [`Transform`] animation; an [`Animator`] plays it (loop / ping-pong / speed /
//! crossfade). [`Tween`]s are procedural one-shots for UI and camera moves — and
//! they work on panel rects too, so sliding panels are a freebie
//! (`RiftEngine-Plan/10-phase-5-game-engine.md` §5.3).

use xre_core::math::{Quat, Vec3};
use xre_core::Transform;

/// An easing curve mapping linear `0..1` time to eased `0..1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Easing {
    /// No easing.
    #[default]
    Linear,
    /// Smoothstep (ease in/out).
    Smooth,
    /// Ease-in (quadratic).
    EaseIn,
    /// Ease-out (quadratic).
    EaseOut,
    /// Hold the start value until `t == 1`.
    Step,
}

impl Easing {
    /// Apply the curve to `t` (clamped to `0..1`).
    #[must_use]
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Self::Linear => t,
            Self::Smooth => t * t * (3.0 - 2.0 * t),
            Self::EaseIn => t * t,
            Self::EaseOut => t * (2.0 - t),
            Self::Step => {
                if t >= 1.0 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// A value that can be interpolated between keyframes.
pub trait Lerpable: Copy {
    /// Interpolate from `self` to `other` by `t`.
    #[must_use]
    fn lerp(self, other: Self, t: f32) -> Self;
}

impl Lerpable for f32 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}
impl Lerpable for Vec3 {
    fn lerp(self, other: Self, t: f32) -> Self {
        self + (other - self) * t
    }
}
impl Lerpable for Quat {
    fn lerp(self, other: Self, t: f32) -> Self {
        self.slerp(other, t)
    }
}

/// One keyframe.
#[derive(Clone, Copy, Debug)]
pub struct Key<T> {
    /// Time, seconds.
    pub time: f32,
    /// Value at this time.
    pub value: T,
    /// Easing applied across the segment *leading into* this key.
    pub easing: Easing,
}

/// A keyframed channel of `T` values.
#[derive(Clone, Debug, Default)]
pub struct Track<T> {
    keys: Vec<Key<T>>,
}

impl<T: Lerpable> Track<T> {
    /// An empty track.
    #[must_use]
    pub const fn new() -> Self {
        Self { keys: Vec::new() }
    }

    /// Add a keyframe (kept sorted by time).
    #[must_use]
    pub fn key(mut self, time: f32, value: T, easing: Easing) -> Self {
        let k = Key {
            time,
            value,
            easing,
        };
        let pos = self.keys.partition_point(|existing| existing.time < time);
        self.keys.insert(pos, k);
        self
    }

    /// The duration (time of the last key), or 0 if empty.
    #[must_use]
    pub fn duration(&self) -> f32 {
        self.keys.last().map_or(0.0, |k| k.time)
    }

    /// Sample the track at `t`, clamping to the endpoints.
    #[must_use]
    pub fn sample(&self, t: f32) -> Option<T> {
        if self.keys.is_empty() {
            return None;
        }
        if t <= self.keys[0].time {
            return Some(self.keys[0].value);
        }
        let last = self.keys.len() - 1;
        if t >= self.keys[last].time {
            return Some(self.keys[last].value);
        }
        // Find the segment [i, i+1] containing t.
        let i = self.keys.partition_point(|k| k.time <= t) - 1;
        let a = &self.keys[i];
        let b = &self.keys[i + 1];
        let span = (b.time - a.time).max(f32::EPSILON);
        let local = ((t - a.time) / span).clamp(0.0, 1.0);
        Some(a.value.lerp(b.value, b.easing.apply(local)))
    }
}

/// How an [`Animator`] handles reaching the end of its [`Clip`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PlayMode {
    /// Play once and stop at the end.
    #[default]
    Once,
    /// Loop back to the start.
    Loop,
    /// Bounce back and forth.
    PingPong,
}

/// A named transform animation built from per-channel tracks.
#[derive(Clone, Debug, Default)]
pub struct Clip {
    /// Translation channel.
    pub translation: Track<Vec3>,
    /// Rotation channel.
    pub rotation: Track<Quat>,
    /// Scale channel.
    pub scale: Track<Vec3>,
}

impl Clip {
    /// The clip duration (max of its channels).
    #[must_use]
    pub fn duration(&self) -> f32 {
        self.translation
            .duration()
            .max(self.rotation.duration())
            .max(self.scale.duration())
    }

    /// Sample the clip into a [`Transform`], falling back to `base` for unset
    /// channels.
    #[must_use]
    pub fn sample(&self, t: f32, base: Transform) -> Transform {
        Transform {
            translation: self.translation.sample(t).unwrap_or(base.translation),
            rotation: self.rotation.sample(t).unwrap_or(base.rotation),
            scale: self.scale.sample(t).unwrap_or(base.scale),
        }
    }
}

/// Plays a [`Clip`] with play/pause/seek/loop/ping-pong/speed/crossfade.
#[derive(Clone, Debug)]
pub struct Animator {
    clip: Clip,
    time: f32,
    speed: f32,
    mode: PlayMode,
    playing: bool,
    forward: bool,
}

impl Animator {
    /// An animator for `clip`, playing forward from 0.
    #[must_use]
    pub const fn new(clip: Clip) -> Self {
        Self {
            clip,
            time: 0.0,
            speed: 1.0,
            mode: PlayMode::Once,
            playing: true,
            forward: true,
        }
    }

    /// Builder: set the play mode.
    #[must_use]
    pub const fn mode(mut self, mode: PlayMode) -> Self {
        self.mode = mode;
        self
    }

    /// Builder: set the playback speed multiplier.
    #[must_use]
    pub const fn speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }

    /// Pause / resume.
    pub const fn set_playing(&mut self, playing: bool) {
        self.playing = playing;
    }

    /// Seek to an absolute time.
    pub const fn seek(&mut self, time: f32) {
        self.time = time;
    }

    /// The current playhead time.
    #[must_use]
    pub const fn time(&self) -> f32 {
        self.time
    }

    /// Advance the playhead by `dt` honouring the play mode.
    pub fn update(&mut self, dt: f32) {
        if !self.playing {
            return;
        }
        let dur = self.clip.duration();
        if dur <= 0.0 {
            return;
        }
        let delta = dt * self.speed * if self.forward { 1.0 } else { -1.0 };
        self.time += delta;
        match self.mode {
            PlayMode::Once => self.time = self.time.clamp(0.0, dur),
            PlayMode::Loop => self.time = self.time.rem_euclid(dur),
            PlayMode::PingPong => {
                if self.time > dur {
                    self.time = dur - (self.time - dur);
                    self.forward = false;
                } else if self.time < 0.0 {
                    self.time = -self.time;
                    self.forward = true;
                }
            }
        }
    }

    /// Sample the current transform against `base`.
    #[must_use]
    pub fn sample(&self, base: Transform) -> Transform {
        self.clip.sample(self.time, base)
    }
}

/// A procedural one-shot interpolation for UI and camera moves.
#[derive(Clone, Copy, Debug)]
pub struct Tween<T> {
    from: T,
    to: T,
    duration: f32,
    easing: Easing,
    elapsed: f32,
}

impl<T: Lerpable> Tween<T> {
    /// Tween from `from` to `to` over `duration` seconds.
    #[must_use]
    pub const fn new(from: T, to: T) -> Self {
        Self {
            from,
            to,
            duration: 1.0,
            easing: Easing::Smooth,
            elapsed: 0.0,
        }
    }

    /// Builder: set the duration.
    #[must_use]
    pub const fn over(mut self, duration: f32) -> Self {
        self.duration = duration;
        self
    }

    /// Builder: set the easing.
    #[must_use]
    pub const fn ease(mut self, easing: Easing) -> Self {
        self.easing = easing;
        self
    }

    /// Advance by `dt` and return the current value.
    pub fn update(&mut self, dt: f32) -> T {
        self.elapsed = (self.elapsed + dt).min(self.duration);
        self.value()
    }

    /// The current value without advancing.
    #[must_use]
    pub fn value(&self) -> T {
        let t = if self.duration <= 0.0 {
            1.0
        } else {
            self.elapsed / self.duration
        };
        self.from.lerp(self.to, self.easing.apply(t))
    }

    /// Whether the tween has reached its target.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.elapsed >= self.duration
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn easing_endpoints() {
        for e in [
            Easing::Linear,
            Easing::Smooth,
            Easing::EaseIn,
            Easing::EaseOut,
        ] {
            assert!((e.apply(0.0)).abs() < 1e-6);
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6);
        }
        assert_eq!(Easing::Step.apply(0.99), 0.0);
        assert_eq!(Easing::Step.apply(1.0), 1.0);
    }

    #[test]
    fn track_interpolates_between_keys() {
        let track =
            Track::<f32>::new()
                .key(0.0, 0.0, Easing::Linear)
                .key(1.0, 10.0, Easing::Linear);
        assert_eq!(track.sample(0.5), Some(5.0));
        assert_eq!(track.sample(-1.0), Some(0.0)); // clamp low
        assert_eq!(track.sample(2.0), Some(10.0)); // clamp high
    }

    #[test]
    fn track_quat_slerps() {
        let q1 = Quat::IDENTITY;
        let q2 = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let track = Track::<Quat>::new()
            .key(0.0, q1, Easing::Linear)
            .key(1.0, q2, Easing::Linear);
        let mid = track.sample(0.5).unwrap();
        assert!(mid.is_normalized());
    }

    #[test]
    fn animator_loops() {
        let clip = Clip {
            translation: Track::new().key(0.0, Vec3::ZERO, Easing::Linear).key(
                1.0,
                Vec3::X,
                Easing::Linear,
            ),
            ..Clip::default()
        };
        let mut anim = Animator::new(clip).mode(PlayMode::Loop);
        anim.update(1.5); // past the end → wraps to 0.5
        assert!((anim.time() - 0.5).abs() < 1e-4);
    }

    #[test]
    fn animator_pingpong_reverses() {
        let clip = Clip {
            translation: Track::new().key(0.0, Vec3::ZERO, Easing::Linear).key(
                1.0,
                Vec3::X,
                Easing::Linear,
            ),
            ..Clip::default()
        };
        let mut anim = Animator::new(clip).mode(PlayMode::PingPong);
        anim.update(1.2); // bounce: 1.2 → 0.8, now going backward
        assert!((anim.time() - 0.8).abs() < 1e-4);
    }

    #[test]
    fn tween_converges_to_target() {
        let mut tween = Tween::new(0.0f32, 100.0).over(1.0).ease(Easing::Linear);
        assert_eq!(tween.update(0.5), 50.0);
        let v = tween.update(0.6); // clamps at duration
        assert_eq!(v, 100.0);
        assert!(tween.finished());
    }

    #[test]
    fn clip_sample_uses_base_for_unset_channels() {
        let clip = Clip {
            translation: Track::new().key(0.0, Vec3::Y, Easing::Linear),
            ..Clip::default()
        };
        let base = Transform::from_translation(Vec3::ZERO);
        let t = clip.sample(0.0, base);
        assert_eq!(t.translation, Vec3::Y);
        assert_eq!(t.rotation, base.rotation); // unset → base
    }
}
