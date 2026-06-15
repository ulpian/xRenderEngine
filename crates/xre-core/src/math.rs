//! Math primitives: a curated re-export of [`glam`] plus a TRS [`Transform`].
//!
//! The engine standardises on [`glam`] for vectors, matrices and quaternions
//! (architecture decision: hand-rolled math was the tax paid by every analysed
//! C++ source). Rotations are always quaternions — never Euler angles.

pub use glam::{IVec2, IVec3, Mat3, Mat4, Quat, UVec2, Vec2, Vec3, Vec4};

/// A translation–rotation–scale transform.
///
/// `to_mat4` composes the canonical `T * R * S` model matrix. The world up
/// vector for [`Transform::look_at`] is `+Y`, matching the engine's
/// right-handed, Y-up, looks-down-`-Z` convention.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Position in the parent space.
    pub translation: Vec3,
    /// Orientation as a unit quaternion.
    pub rotation: Quat,
    /// Per-axis scale.
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Transform {
    /// The identity transform: no translation, no rotation, unit scale.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// A transform with only a translation.
    #[must_use]
    pub const fn from_translation(translation: Vec3) -> Self {
        Self {
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }

    /// A transform with only a rotation.
    #[must_use]
    pub const fn from_rotation(rotation: Quat) -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Compose the `T * R * S` model matrix for this transform.
    #[must_use]
    pub fn to_mat4(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Build a transform positioned at `eye` and oriented so that its local
    /// `-Z` axis points toward `target`, with `+Y` as the up reference.
    ///
    /// If `eye` and `target` coincide (or the look direction is parallel to up)
    /// the rotation falls back to identity rather than producing `NaN`s.
    #[must_use]
    pub fn look_at(eye: Vec3, target: Vec3) -> Self {
        let forward = target - eye;
        let rotation = forward.try_normalize().map_or(Quat::IDENTITY, |fwd| {
            // The camera looks down -Z, so local +Z points away from the target.
            let z_axis = -fwd;
            let right = Vec3::Y.cross(z_axis);
            right.try_normalize().map_or(Quat::IDENTITY, |right| {
                let up = z_axis.cross(right);
                Quat::from_mat3(&Mat3::from_cols(right, up, z_axis))
            })
        });
        Self {
            translation: eye,
            rotation,
            scale: Vec3::ONE,
        }
    }

    /// Rotate in place about the local `+Y` axis by `radians`.
    pub fn rotate_y(&mut self, radians: f32) {
        self.rotation = (self.rotation * Quat::from_rotation_y(radians)).normalize();
    }

    /// The unit forward direction (local `-Z`) in parent space.
    #[must_use]
    pub fn forward(&self) -> Vec3 {
        self.rotation * Vec3::NEG_Z
    }

    /// The unit right direction (local `+X`) in parent space.
    #[must_use]
    pub fn right(&self) -> Vec3 {
        self.rotation * Vec3::X
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_to_mat4_is_identity() {
        assert_eq!(Transform::IDENTITY.to_mat4(), Mat4::IDENTITY);
    }

    #[test]
    fn look_at_faces_target() {
        let t = Transform::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
        // Looking from +Z toward the origin, forward should be ~ -Z.
        assert!((t.forward() - Vec3::NEG_Z).length() < 1e-5);
    }

    #[test]
    fn look_at_degenerate_is_finite() {
        let t = Transform::look_at(Vec3::ZERO, Vec3::ZERO);
        assert!(t.rotation.is_finite());
    }

    #[test]
    fn rotate_y_keeps_unit_quaternion() {
        let mut t = Transform::IDENTITY;
        for _ in 0..1000 {
            t.rotate_y(0.1);
        }
        assert!((t.rotation.length() - 1.0).abs() < 1e-4);
    }
}
