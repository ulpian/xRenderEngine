//! [`Camera`] and [`Projection`] — the view and clip transforms.
//!
//! The projection is **cell-aspect aware** (`RiftEngine-Plan/03-rendering-pipeline-spec.md`
//! §A1): a terminal cell is roughly twice as tall as it is wide, so the
//! horizontal field of view is widened by [`Projection::cell_aspect`] to keep
//! circles round instead of egg-shaped. This is the principled version of
//! gemini-engine's hard-coded `character_width_multiplier = 2.0`.

use xre_core::math::{Mat4, Vec3};
use xre_core::Transform;

/// How the scene is flattened to clip space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Projection {
    /// Perspective projection.
    Perspective {
        /// Vertical field of view, radians.
        fov_y: f32,
        /// Near plane distance (> 0).
        near: f32,
        /// Far plane distance.
        far: f32,
    },
    /// Orthographic projection.
    Orthographic {
        /// Half the vertical extent in world units.
        height: f32,
        /// Near plane distance.
        near: f32,
        /// Far plane distance.
        far: f32,
    },
}

impl Default for Projection {
    fn default() -> Self {
        Self::Perspective {
            fov_y: core::f32::consts::FRAC_PI_3, // 60°
            near: 0.1,
            far: 100.0,
        }
    }
}

impl Projection {
    /// The aspect-correction factor for non-square cells. A cell ~2× taller than
    /// wide gives `0.5`; pass the real ratio if known.
    pub const DEFAULT_CELL_ASPECT: f32 = 0.5;

    /// The near plane distance.
    #[must_use]
    pub const fn near(&self) -> f32 {
        match self {
            Self::Perspective { near, .. } | Self::Orthographic { near, .. } => *near,
        }
    }

    /// The far plane distance.
    #[must_use]
    pub const fn far(&self) -> f32 {
        match self {
            Self::Perspective { far, .. } | Self::Orthographic { far, .. } => *far,
        }
    }

    /// Build the projection matrix for a viewport of `cols × rows` cells, where
    /// each cell has width/height ratio `cell_aspect` (~0.5 for terminals).
    #[must_use]
    pub fn matrix(&self, cols: u32, rows: u32, cell_aspect: f32) -> Mat4 {
        let rows = rows.max(1) as f32;
        let cols = cols.max(1) as f32;
        // Physical aspect ratio of the viewport accounting for cell shape.
        let aspect = (cols * cell_aspect) / rows;
        match *self {
            Self::Perspective { fov_y, near, far } => {
                Mat4::perspective_rh(fov_y, aspect.max(f32::EPSILON), near, far)
            }
            Self::Orthographic { height, near, far } => {
                let h = height.max(f32::EPSILON);
                let w = h * aspect;
                Mat4::orthographic_rh(-w, w, -h, h, near, far)
            }
        }
    }
}

/// A camera: a pose plus a [`Projection`].
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    /// The camera's world transform (looks down local `-Z`).
    pub transform: Transform,
    /// The projection.
    pub projection: Projection,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, 5.0)),
            projection: Projection::default(),
        }
    }
}

impl Camera {
    /// A camera at `eye` looking at `target` with the default projection.
    #[must_use]
    pub fn look_at(eye: Vec3, target: Vec3) -> Self {
        Self {
            transform: Transform::look_at(eye, target),
            projection: Projection::default(),
        }
    }

    /// Builder: set the projection.
    #[must_use]
    pub const fn with_projection(mut self, projection: Projection) -> Self {
        self.projection = projection;
        self
    }

    /// The view matrix (inverse of the camera's world transform).
    #[must_use]
    pub fn view_matrix(&self) -> Mat4 {
        self.transform.to_mat4().inverse()
    }

    /// The combined `projection · view` matrix for a `cols × rows` viewport.
    #[must_use]
    pub fn view_projection(&self, cols: u32, rows: u32, cell_aspect: f32) -> Mat4 {
        self.projection.matrix(cols, rows, cell_aspect) * self.view_matrix()
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::float_cmp,
        clippy::field_reassign_with_default
    )]
    use super::*;

    #[test]
    fn perspective_projects_forward_point_inside_ndc() {
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);
        let vp = cam.view_projection(80, 40, Projection::DEFAULT_CELL_ASPECT);
        // A point at the origin is in front of the camera → finite, w > 0.
        let clip = vp * xre_core::math::Vec4::new(0.0, 0.0, 0.0, 1.0);
        assert!(clip.w > 0.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < 1.0 && ndc.y.abs() < 1.0);
    }

    #[test]
    fn point_behind_camera_has_negative_w() {
        let cam = Camera::look_at(Vec3::new(0.0, 0.0, 5.0), Vec3::ZERO);
        let vp = cam.view_projection(80, 40, Projection::DEFAULT_CELL_ASPECT);
        // A point behind the camera (z = 10, camera at z = 5 looking toward 0).
        let clip = vp * xre_core::math::Vec4::new(0.0, 0.0, 10.0, 1.0);
        assert!(clip.w <= 0.0, "point behind camera must have w <= 0");
    }

    #[test]
    fn near_far_accessors() {
        let p = Projection::Perspective {
            fov_y: 1.0,
            near: 0.2,
            far: 50.0,
        };
        assert_eq!(p.near(), 0.2);
        assert_eq!(p.far(), 50.0);
    }
}
