//! Near-plane clipping in clip space (Sutherland–Hodgman).
//!
//! This is the fix for the bug class shared by Ymael (no near clip) and
//! gemini-engine 1.2.0 (a no-op near-clip whose `continue` targets the inner
//! loop — see [14](14-gemini-engine-analysis.md) §2). A triangle straddling the
//! near plane is *split* into 0–2 triangles with all attributes interpolated at
//! the clip-space intersections, so the perspective divide is always safe and no
//! garbage coordinates ever reach the rasterizer
//! (`RiftEngine-Plan/07-phase-2-renderer-core.md` §2.2).
//!
//! The projection uses glam's `perspective_rh` (0..1 depth), so the near plane
//! is `z_clip >= 0`.

use xre_core::math::{Vec2, Vec3, Vec4};

/// A vertex carried through clip space with its interpolatable attributes.
#[derive(Clone, Copy, Debug)]
pub struct ClipVertex {
    /// Homogeneous clip-space position (`P·V·M·pos`).
    pub clip: Vec4,
    /// World-space position (for lighting).
    pub world: Vec3,
    /// World-space normal.
    pub normal: Vec3,
    /// Texture coordinate.
    pub uv: Vec2,
}

impl ClipVertex {
    /// Linearly interpolate every attribute between `self` and `other` at `t`.
    #[must_use]
    pub fn lerp(&self, other: &Self, t: f32) -> Self {
        Self {
            clip: self.clip.lerp(other.clip, t),
            world: self.world.lerp(other.world, t),
            normal: self.normal.lerp(other.normal, t),
            uv: self.uv.lerp(other.uv, t),
        }
    }
}

/// Signed distance to the near plane (positive = in front / inside).
#[inline]
fn near_dist(v: &ClipVertex) -> f32 {
    v.clip.z
}

/// Clip `tri` against the near plane, appending the resulting triangles (0, 1 or
/// 2) to `out`. Attributes are interpolated at the intersection points.
pub fn clip_near(tri: [ClipVertex; 3], out: &mut Vec<[ClipVertex; 3]>) {
    let eps = 1e-7;
    let d = [near_dist(&tri[0]), near_dist(&tri[1]), near_dist(&tri[2])];
    let inside = [d[0] >= 0.0, d[1] >= 0.0, d[2] >= 0.0];
    let count = inside.iter().filter(|&&b| b).count();

    match count {
        0 => {}             // fully behind the near plane: discard
        3 => out.push(tri), // fully in front: keep
        _ => {
            // Build the clipped polygon by walking edges (Sutherland–Hodgman).
            let mut poly: Vec<ClipVertex> = Vec::with_capacity(4);
            for i in 0..3 {
                let cur = tri[i];
                let nxt = tri[(i + 1) % 3];
                let dc = d[i];
                let dn = d[(i + 1) % 3];
                if dc >= 0.0 {
                    poly.push(cur);
                }
                // Edge crosses the plane: add the intersection.
                if (dc >= 0.0) != (dn >= 0.0) {
                    let denom = dc - dn;
                    let t = if denom.abs() < eps { 0.0 } else { dc / denom };
                    poly.push(cur.lerp(&nxt, t));
                }
            }
            // Fan-triangulate the (3- or 4-vertex) clipped polygon.
            for i in 1..poly.len().saturating_sub(1) {
                out.push([poly[0], poly[i], poly[i + 1]]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(clip: Vec4) -> ClipVertex {
        ClipVertex {
            clip,
            world: Vec3::ZERO,
            normal: Vec3::Y,
            uv: Vec2::ZERO,
        }
    }

    #[test]
    fn fully_in_front_is_kept() {
        let tri = [
            v(Vec4::new(0.0, 0.0, 1.0, 2.0)),
            v(Vec4::new(1.0, 0.0, 1.0, 2.0)),
            v(Vec4::new(0.0, 1.0, 1.0, 2.0)),
        ];
        let mut out = Vec::new();
        clip_near(tri, &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn fully_behind_is_discarded() {
        let tri = [
            v(Vec4::new(0.0, 0.0, -1.0, 2.0)),
            v(Vec4::new(1.0, 0.0, -1.0, 2.0)),
            v(Vec4::new(0.0, 1.0, -1.0, 2.0)),
        ];
        let mut out = Vec::new();
        clip_near(tri, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn one_vertex_behind_makes_a_quad_two_triangles() {
        // One vertex behind (z<0), two in front → clipped to a quad → 2 tris.
        let tri = [
            v(Vec4::new(0.0, 0.0, 1.0, 2.0)),
            v(Vec4::new(1.0, 0.0, 1.0, 2.0)),
            v(Vec4::new(0.0, 1.0, -1.0, 2.0)),
        ];
        let mut out = Vec::new();
        clip_near(tri, &mut out);
        assert_eq!(out.len(), 2);
        // Every output vertex is on or in front of the near plane.
        for t in &out {
            for vert in t {
                assert!(near_dist(vert) >= -1e-5, "vertex left behind the plane");
            }
        }
    }

    #[test]
    fn two_vertices_behind_makes_one_triangle() {
        let tri = [
            v(Vec4::new(0.0, 0.0, 1.0, 2.0)),
            v(Vec4::new(1.0, 0.0, -1.0, 2.0)),
            v(Vec4::new(0.0, 1.0, -1.0, 2.0)),
        ];
        let mut out = Vec::new();
        clip_near(tri, &mut out);
        assert_eq!(out.len(), 1);
    }
}
