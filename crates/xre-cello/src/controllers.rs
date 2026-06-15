//! Input-agnostic camera controllers (Stage 3.3).
//!
//! Controllers are fed *actions* (deltas already decoded from input), not raw
//! keys, so they work identically under any input mapping. [`OrbitController`]
//! orbits a target (the viewer default); [`FpsController`] is a free-fly camera
//! with **true** pitch — fixing justMoritz's row-shift hack
//! (`RiftEngine-Plan/08-phase-3-assets-scenes.md` §3.3).

use xre_core::math::Vec3;
use xre_core::Transform;
use xre_render::Camera;

/// An orbit camera: yaw / pitch / distance around a target, with damping.
#[derive(Clone, Copy, Debug)]
pub struct OrbitController {
    /// Orbit target (look-at point).
    pub target: Vec3,
    /// Yaw angle (radians).
    pub yaw: f32,
    /// Pitch angle (radians), clamped away from the poles.
    pub pitch: f32,
    /// Distance from the target.
    pub distance: f32,
    /// Damping factor per second (0 = snap, 1 = never).
    pub damping: f32,
    // Smoothed values the camera actually uses.
    cur_yaw: f32,
    cur_pitch: f32,
    cur_distance: f32,
}

impl Default for OrbitController {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            yaw: 0.6,
            pitch: 0.4,
            distance: 5.0,
            damping: 0.85,
            cur_yaw: 0.6,
            cur_pitch: 0.4,
            cur_distance: 5.0,
        }
    }
}

impl OrbitController {
    /// An orbit controller around `target` at `distance`.
    #[must_use]
    pub fn new(target: Vec3, distance: f32) -> Self {
        Self {
            target,
            distance,
            cur_distance: distance,
            ..Self::default()
        }
    }

    /// Apply yaw/pitch deltas (radians), clamping pitch to ±85°.
    pub fn rotate(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        let limit = 85.0f32.to_radians();
        self.pitch = (self.pitch + dpitch).clamp(-limit, limit);
    }

    /// Zoom by a multiplicative factor (clamped to a sane range).
    pub fn zoom(&mut self, factor: f32) {
        self.distance = (self.distance * factor).clamp(0.2, 1000.0);
    }

    /// Advance the damping toward the target angles over `dt` seconds.
    pub fn update(&mut self, dt: f32) {
        // Exponential smoothing independent of frame rate.
        let t = 1.0 - self.damping.powf(dt * 60.0);
        self.cur_yaw += (self.yaw - self.cur_yaw) * t;
        self.cur_pitch += (self.pitch - self.cur_pitch) * t;
        self.cur_distance += (self.distance - self.cur_distance) * t;
    }

    /// The current eye position derived from the smoothed orbit angles.
    #[must_use]
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.cur_yaw.sin_cos();
        let (sp, cp) = self.cur_pitch.sin_cos();
        let dir = Vec3::new(cp * sy, sp, cp * cy);
        self.target + dir * self.cur_distance
    }

    /// Apply this controller to `camera` (sets a look-at transform).
    pub fn apply(&self, camera: &mut Camera) {
        camera.transform = Transform::look_at(self.eye(), self.target);
    }
}

/// A free-fly first-person camera with true pitch.
#[derive(Clone, Copy, Debug)]
pub struct FpsController {
    /// World position.
    pub position: Vec3,
    /// Yaw (radians).
    pub yaw: f32,
    /// Pitch (radians), clamped to ±89°.
    pub pitch: f32,
    /// Movement speed (units/second).
    pub speed: f32,
}

impl Default for FpsController {
    fn default() -> Self {
        Self {
            position: Vec3::new(0.0, 0.0, 5.0),
            yaw: core::f32::consts::PI, // look toward -Z
            pitch: 0.0,
            speed: 4.0,
        }
    }
}

impl FpsController {
    /// A controller at `position`.
    #[must_use]
    pub fn new(position: Vec3) -> Self {
        Self {
            position,
            ..Self::default()
        }
    }

    /// Look by yaw/pitch deltas (radians), clamping pitch.
    pub fn look(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw += dyaw;
        let limit = 89.0f32.to_radians();
        self.pitch = (self.pitch + dpitch).clamp(-limit, limit);
    }

    /// The forward direction (true 3-D, with pitch).
    #[must_use]
    pub fn forward(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        Vec3::new(cp * sy, sp, cp * cy)
    }

    /// Move by local `(right, up, forward)` amounts scaled by `speed * dt`.
    pub fn move_local(&mut self, right: f32, up: f32, forward: f32, dt: f32) {
        let fwd = self.forward();
        let right_vec = fwd.cross(Vec3::Y).normalize_or_zero();
        let delta = (right_vec * right + Vec3::Y * up + fwd * forward) * (self.speed * dt);
        self.position += delta;
    }

    /// Apply to `camera`.
    pub fn apply(&self, camera: &mut Camera) {
        camera.transform = Transform::look_at(self.position, self.position + self.forward());
    }
}

/// A constraint that orients a transform to face a target point each update.
#[derive(Clone, Copy, Debug)]
pub struct LookAtConstraint {
    /// The point to face.
    pub target: Vec3,
}

impl LookAtConstraint {
    /// Constrain `transform` to look at `self.target` from its current position.
    pub fn apply(&self, transform: &mut Transform) {
        let looked = Transform::look_at(transform.translation, self.target);
        transform.rotation = looked.rotation;
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::float_cmp)]
    use super::*;

    #[test]
    fn orbit_pitch_is_clamped() {
        let mut o = OrbitController::default();
        o.rotate(0.0, 100.0); // way past vertical
        assert!(o.pitch < 86.0f32.to_radians());
    }

    #[test]
    fn orbit_damping_converges() {
        let mut o = OrbitController::new(Vec3::ZERO, 5.0);
        o.rotate(1.0, 0.0);
        for _ in 0..240 {
            o.update(1.0 / 60.0);
        }
        assert!((o.cur_yaw - o.yaw).abs() < 1e-2, "damping did not converge");
    }

    #[test]
    fn orbit_eye_is_at_distance() {
        let o = OrbitController::new(Vec3::ZERO, 7.0);
        assert!((o.eye().length() - 7.0).abs() < 1e-3);
    }

    #[test]
    fn fps_pitch_clamped_and_moves_forward() {
        let mut f = FpsController::new(Vec3::ZERO);
        f.pitch = 0.0;
        f.yaw = 0.0; // forward ≈ +Z
        let before = f.position;
        f.move_local(0.0, 0.0, 1.0, 1.0);
        assert!((f.position - before).length() > 0.0);
        f.look(0.0, 100.0);
        assert!(f.pitch <= 89.0f32.to_radians() + 1e-4);
    }

    #[test]
    fn lookat_orients_toward_target() {
        let mut t = Transform::from_translation(Vec3::new(0.0, 0.0, 5.0));
        LookAtConstraint { target: Vec3::ZERO }.apply(&mut t);
        assert!((t.forward() - Vec3::NEG_Z).length() < 1e-4);
    }
}
