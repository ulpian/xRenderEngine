//! [`Mesh`] geometry and the bundled procedural test meshes.
//!
//! A mesh is an indexed triangle list with parallel position / normal / uv
//! arrays (Struct-of-Arrays, matching the vertex-pull rasterizer). The
//! procedural cube / plane / uv-sphere / torus exist so Phase 2 has content to
//! light before the OBJ loader lands in Phase 3
//! (`RiftEngine-Plan/07-phase-2-renderer-core.md` §2.4).

use core::f32::consts::TAU;

use xre_core::math::{Vec2, Vec3};

/// An axis-aligned bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum corner.
    pub min: Vec3,
    /// Maximum corner.
    pub max: Vec3,
}

impl Aabb {
    /// The center point.
    #[must_use]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// The full extent (max − min).
    #[must_use]
    pub fn extent(&self) -> Vec3 {
        self.max - self.min
    }

    /// The radius of the bounding sphere centered at [`Aabb::center`].
    #[must_use]
    pub fn bounding_radius(&self) -> f32 {
        self.extent().length() * 0.5
    }
}

/// An indexed triangle mesh.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    /// Vertex positions.
    pub positions: Vec<Vec3>,
    /// Per-vertex normals (parallel to `positions`).
    pub normals: Vec<Vec3>,
    /// Per-vertex texture coordinates (parallel to `positions`).
    pub uvs: Vec<Vec2>,
    /// Triangles as triples of indices into the vertex arrays.
    pub indices: Vec<[u32; 3]>,
}

impl Mesh {
    /// The number of triangles.
    #[must_use]
    pub const fn triangle_count(&self) -> usize {
        self.indices.len()
    }

    /// The axis-aligned bounding box, or a zero box for an empty mesh.
    #[must_use]
    pub fn aabb(&self) -> Aabb {
        if self.positions.is_empty() {
            return Aabb {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
            };
        }
        let mut min = self.positions[0];
        let mut max = self.positions[0];
        for &p in &self.positions {
            min = min.min(p);
            max = max.max(p);
        }
        Aabb { min, max }
    }

    /// Recompute smooth per-vertex normals by averaging adjacent face normals.
    /// Used after loading geometry that lacks normals (Phase 3) and to validate
    /// the procedural meshes.
    pub fn recompute_smooth_normals(&mut self) {
        // Keep the prior normals to fall back on at vertices whose only adjacent
        // faces are degenerate (e.g. UV-sphere poles), where the area-weighted
        // sum is zero and a blind `Y` fallback would be wrong.
        let prior = core::mem::take(&mut self.normals);
        let mut acc = vec![Vec3::ZERO; self.positions.len()];
        for &[a, b, c] in &self.indices {
            let (ia, ib, ic) = (a as usize, b as usize, c as usize);
            let face = (self.positions[ib] - self.positions[ia])
                .cross(self.positions[ic] - self.positions[ia]);
            // Unnormalized face normal weights by area — a fine smoothing choice.
            acc[ia] += face;
            acc[ib] += face;
            acc[ic] += face;
        }
        self.normals = acc
            .into_iter()
            .enumerate()
            .map(|(i, n)| {
                n.try_normalize()
                    .or_else(|| prior.get(i).and_then(|p| p.try_normalize()))
                    .unwrap_or(Vec3::Y)
            })
            .collect();
    }

    /// A unit cube centered at the origin (per-face normals, 24 vertices).
    #[must_use]
    pub fn cube() -> Self {
        let mut mesh = Self::default();
        // (normal, two in-plane axes) for each of the six faces.
        let faces = [
            (Vec3::Z, Vec3::X, Vec3::Y),
            (Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y),
            (Vec3::X, Vec3::NEG_Z, Vec3::Y),
            (Vec3::NEG_X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::X, Vec3::NEG_Z),
            (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        ];
        for (normal, u, v) in faces {
            let base = mesh.positions.len() as u32;
            let center = normal * 0.5;
            // Corners: (-u,-v), (u,-v), (u,v), (-u,v).
            let corners = [
                center - u * 0.5 - v * 0.5,
                center + u * 0.5 - v * 0.5,
                center + u * 0.5 + v * 0.5,
                center - u * 0.5 + v * 0.5,
            ];
            let uvs = [
                Vec2::new(0.0, 1.0),
                Vec2::new(1.0, 1.0),
                Vec2::new(1.0, 0.0),
                Vec2::new(0.0, 0.0),
            ];
            for (p, uv) in corners.into_iter().zip(uvs) {
                mesh.positions.push(p);
                mesh.normals.push(normal);
                mesh.uvs.push(uv);
            }
            mesh.indices.push([base, base + 1, base + 2]);
            mesh.indices.push([base, base + 2, base + 3]);
        }
        mesh
    }

    /// A flat plane of side `size` in the XZ plane, facing `+Y`, centered at the
    /// origin (two triangles).
    #[must_use]
    pub fn plane(size: f32) -> Self {
        let h = size * 0.5;
        let positions = vec![
            Vec3::new(-h, 0.0, -h),
            Vec3::new(h, 0.0, -h),
            Vec3::new(h, 0.0, h),
            Vec3::new(-h, 0.0, h),
        ];
        let normals = vec![Vec3::Y; 4];
        let uvs = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        Self {
            positions,
            normals,
            uvs,
            indices: vec![[0, 1, 2], [0, 2, 3]],
        }
    }

    /// A flat quad in the XY plane facing `+Z`, sized `aspect` wide by `1.0` tall
    /// and centered at the origin (two triangles). UVs are laid out top-down so a
    /// texture sampled with [`crate::TextureSampler`] is not vertically flipped:
    /// the top edge (`+Y`) maps to `v = 0`. Used by the image viewer to display a
    /// picture as a textured surface.
    #[must_use]
    pub fn image_quad(aspect: f32) -> Self {
        let hx = aspect.max(f32::EPSILON) * 0.5;
        let hy = 0.5;
        let positions = vec![
            Vec3::new(-hx, hy, 0.0),  // top-left
            Vec3::new(hx, hy, 0.0),   // top-right
            Vec3::new(hx, -hy, 0.0),  // bottom-right
            Vec3::new(-hx, -hy, 0.0), // bottom-left
        ];
        let normals = vec![Vec3::Z; 4];
        let uvs = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 1.0),
        ];
        Self {
            positions,
            normals,
            uvs,
            indices: vec![[0, 1, 2], [0, 2, 3]],
        }
    }

    /// A UV sphere of `radius` with `rings` latitudinal and `sectors`
    /// longitudinal divisions (analytic normals).
    #[must_use]
    pub fn uv_sphere(radius: f32, rings: u32, sectors: u32) -> Self {
        let rings = rings.max(2);
        let sectors = sectors.max(3);
        let mut mesh = Self::default();
        for r in 0..=rings {
            let v = r as f32 / rings as f32;
            let phi = v * core::f32::consts::PI; // 0..π
            let (sp, cp) = phi.sin_cos();
            for s in 0..=sectors {
                let u = s as f32 / sectors as f32;
                let theta = u * TAU; // 0..2π
                let (st, ct) = theta.sin_cos();
                let normal = Vec3::new(sp * ct, cp, sp * st);
                mesh.positions.push(normal * radius);
                mesh.normals.push(normal);
                mesh.uvs.push(Vec2::new(u, v));
            }
        }
        let stride = sectors + 1;
        for r in 0..rings {
            for s in 0..sectors {
                let i0 = r * stride + s;
                let i1 = i0 + stride;
                // CCW-outward winding (cross product points along +normal).
                mesh.indices.push([i0, i0 + 1, i1]);
                mesh.indices.push([i0 + 1, i1 + 1, i1]);
            }
        }
        mesh
    }

    /// A torus with the given major/minor radii and segment counts.
    #[must_use]
    pub fn torus(major: f32, minor: f32, major_seg: u32, minor_seg: u32) -> Self {
        let major_seg = major_seg.max(3);
        let minor_seg = minor_seg.max(3);
        let mut mesh = Self::default();
        for i in 0..=major_seg {
            let u = i as f32 / major_seg as f32 * TAU;
            let (su, cu) = u.sin_cos();
            let center = Vec3::new(cu * major, 0.0, su * major);
            for j in 0..=minor_seg {
                let v = j as f32 / minor_seg as f32 * TAU;
                let (sv, cv) = v.sin_cos();
                let normal = Vec3::new(cu * cv, sv, su * cv);
                mesh.positions.push(center + normal * minor);
                mesh.normals.push(normal);
                mesh.uvs.push(Vec2::new(
                    i as f32 / major_seg as f32,
                    j as f32 / minor_seg as f32,
                ));
            }
        }
        let stride = minor_seg + 1;
        for i in 0..major_seg {
            for j in 0..minor_seg {
                let a = i * stride + j;
                let b = a + stride;
                // CCW-outward winding.
                mesh.indices.push([a, a + 1, b]);
                mesh.indices.push([a + 1, b + 1, b]);
            }
        }
        mesh
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
    fn cube_has_12_triangles_and_unit_extent() {
        let cube = Mesh::cube();
        assert_eq!(cube.triangle_count(), 12);
        let aabb = cube.aabb();
        assert!((aabb.extent() - Vec3::ONE).length() < 1e-5);
    }

    #[test]
    fn sphere_vertices_lie_on_radius() {
        let s = Mesh::uv_sphere(2.0, 8, 12);
        for p in &s.positions {
            assert!((p.length() - 2.0).abs() < 1e-4);
        }
        // Normals point outward (same direction as position).
        for (p, n) in s.positions.iter().zip(&s.normals) {
            assert!(p.normalize().dot(*n) > 0.99);
        }
    }

    #[test]
    fn torus_bounds_make_sense() {
        let t = Mesh::torus(2.0, 0.5, 16, 8);
        let aabb = t.aabb();
        // Outer radius 2.5 in the XZ plane.
        assert!((aabb.max.x - 2.5).abs() < 1e-3);
        assert!((aabb.max.y - 0.5).abs() < 1e-3);
    }

    #[test]
    fn recomputed_normals_match_analytic_for_sphere() {
        let (rings, sectors) = (16u32, 24u32);
        let mut s = Mesh::uv_sphere(1.0, rings, sectors);
        let analytic = s.normals.clone();
        s.recompute_smooth_normals();
        let stride = sectors + 1;
        // Area-weighted smoothing is ill-defined at the UV-sphere poles (the
        // adjacent faces are degenerate slivers), so check the body rows only.
        let mut worst = 0.0f32;
        for (i, (a, b)) in analytic.iter().zip(&s.normals).enumerate() {
            let ring = i as u32 / stride;
            if ring == 0 || ring == rings {
                continue;
            }
            worst = worst.max(1.0 - a.dot(*b));
        }
        assert!(worst < 0.05, "worst normal deviation {worst}");
    }

    #[test]
    fn plane_faces_up() {
        let p = Mesh::plane(2.0);
        assert!(p.normals.iter().all(|n| *n == Vec3::Y));
        assert_eq!(p.triangle_count(), 2);
    }

    #[test]
    fn image_quad_faces_camera_with_topdown_uvs() {
        let q = Mesh::image_quad(2.0);
        assert_eq!(q.triangle_count(), 2);
        assert!(q.normals.iter().all(|n| *n == Vec3::Z));
        // Width = aspect (2.0), height = 1.0.
        let aabb = q.aabb();
        assert!((aabb.extent().x - 2.0).abs() < 1e-6);
        assert!((aabb.extent().y - 1.0).abs() < 1e-6);
        // The top-left vertex (max Y) carries uv (0, 0): not vertically flipped.
        let top_left = q
            .positions
            .iter()
            .position(|p| *p == Vec3::new(-1.0, 0.5, 0.0))
            .unwrap();
        assert_eq!(q.uvs[top_left], Vec2::new(0.0, 0.0));
    }
}
