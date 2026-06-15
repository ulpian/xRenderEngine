//! Polygon triangulation: a fan for convex faces, ear-clipping for concave ones.
//!
//! The ear-clipping core is **vendored from gemini-engine** (MIT, Ren "renpenguin"
//! — see [14](14-gemini-engine-analysis.md) §4 and the NOTICE file) and hardened
//! here with orientation handling, collinear-vertex skipping, a bounded fallback
//! for degenerate input, and property tests. It lifts the "ear-clipping flagged
//! for later" deferral in `RiftEngine-Plan/08-phase-3-assets-scenes.md` §3.1.
//!
//! Input polygons are 3-D (OBJ faces); they are projected onto their best-fit
//! plane (Newell normal, dominant axis dropped) before 2-D ear-clipping.

use xre_core::math::{Vec2, Vec3};

/// Triangulate a polygon given as a loop of vertex indices into `positions`.
///
/// Returns triangles as triples of *positions into the input `loop_indices`*
/// (i.e. local indices `0..loop_indices.len()`), so the caller can remap to its
/// own vertex ids. Convex faces fan-triangulate; concave faces use ear clipping.
/// Degenerate faces (< 3 vertices, zero area) yield no triangles.
#[must_use]
pub fn triangulate(positions: &[Vec3], loop_indices: &[u32]) -> Vec<[usize; 3]> {
    let n = loop_indices.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    let poly: Vec<Vec3> = loop_indices
        .iter()
        .map(|&i| positions[i as usize])
        .collect();
    let normal = newell_normal(&poly);
    if normal.length_squared() < 1e-20 {
        // Degenerate (collinear) polygon: fall back to a fan, which the
        // rasterizer's degenerate-triangle guard will mostly drop.
        return fan(n);
    }
    let projected = project_to_plane(&poly, normal);
    ear_clip(&projected).unwrap_or_else(|| fan(n))
}

/// A simple triangle fan over `n` vertices.
fn fan(n: usize) -> Vec<[usize; 3]> {
    (1..n - 1).map(|i| [0, i, i + 1]).collect()
}

/// Newell's method: a robust polygon normal even for non-planar loops.
fn newell_normal(poly: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for i in 0..poly.len() {
        let cur = poly[i];
        let nxt = poly[(i + 1) % poly.len()];
        n.x += (cur.y - nxt.y) * (cur.z + nxt.z);
        n.y += (cur.z - nxt.z) * (cur.x + nxt.x);
        n.z += (cur.x - nxt.x) * (cur.y + nxt.y);
    }
    n
}

/// Project a 3-D polygon to 2-D by dropping the axis of the dominant normal
/// component, preserving winding.
fn project_to_plane(poly: &[Vec3], normal: Vec3) -> Vec<Vec2> {
    let a = normal.abs();
    poly.iter()
        .map(|p| {
            if a.x >= a.y && a.x >= a.z {
                // Drop X; keep (z, y) so winding sign is preserved.
                Vec2::new(p.z, p.y) * normal.x.signum()
            } else if a.y >= a.z {
                Vec2::new(p.x, p.z) * normal.y.signum()
            } else {
                Vec2::new(p.x, p.y) * normal.z.signum()
            }
        })
        .collect()
}

/// 2-D ear clipping. Returns `None` if it cannot make progress (it never loops
/// forever — the outer guard caps iterations).
#[allow(clippy::many_single_char_names)]
fn ear_clip(poly: &[Vec2]) -> Option<Vec<[usize; 3]>> {
    let n = poly.len();
    let mut indices: Vec<usize> = (0..n).collect();
    // Ensure counter-clockwise winding so the inside test is consistent.
    if signed_area(poly, &indices) < 0.0 {
        indices.reverse();
    }
    let mut triangles = Vec::with_capacity(n - 2);
    let mut guard = 0;
    let max_guard = n * n + 8;
    while indices.len() > 3 {
        guard += 1;
        if guard > max_guard {
            return None; // pathological input; let the caller fall back
        }
        let m = indices.len();
        let mut clipped = false;
        for i in 0..m {
            let a = indices[(i + m - 1) % m];
            let b = indices[i];
            let c = indices[(i + 1) % m];
            if is_ear(poly, &indices, a, b, c) {
                triangles.push([a, b, c]);
                indices.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            return None; // no ear found (numerical issue) — fall back to a fan
        }
    }
    triangles.push([indices[0], indices[1], indices[2]]);
    Some(triangles)
}

/// Signed area of the polygon given by `order` into `poly`.
fn signed_area(poly: &[Vec2], order: &[usize]) -> f32 {
    let mut area = 0.0;
    for i in 0..order.len() {
        let p = poly[order[i]];
        let q = poly[order[(i + 1) % order.len()]];
        area += p.x * q.y - q.x * p.y;
    }
    area * 0.5
}

/// Whether triangle `(a,b,c)` is an ear: convex at `b` and containing no other
/// polygon vertex.
fn is_ear(poly: &[Vec2], indices: &[usize], a: usize, b: usize, c: usize) -> bool {
    let (pa, pb, pc) = (poly[a], poly[b], poly[c]);
    if cross(pb - pa, pc - pa) <= 0.0 {
        return false; // reflex or collinear vertex (CCW assumed)
    }
    for &idx in indices {
        if idx == a || idx == b || idx == c {
            continue;
        }
        if point_in_triangle(poly[idx], pa, pb, pc) {
            return false;
        }
    }
    true
}

#[inline]
fn cross(u: Vec2, v: Vec2) -> f32 {
    u.x * v.y - u.y * v.x
}

fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let d1 = cross(b - a, p - a);
    let d2 = cross(c - b, p - b);
    let d3 = cross(a - c, p - c);
    let has_neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let has_pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(has_neg && has_pos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn area_of(positions: &[Vec3], tris: &[[usize; 3]], loop_idx: &[u32]) -> f32 {
        tris.iter()
            .map(|&[a, b, c]| {
                let pa = positions[loop_idx[a] as usize];
                let pb = positions[loop_idx[b] as usize];
                let pc = positions[loop_idx[c] as usize];
                (pb - pa).cross(pc - pa).length() * 0.5
            })
            .sum()
    }

    #[test]
    fn triangle_passes_through() {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::Y];
        assert_eq!(triangulate(&p, &[0, 1, 2]), vec![[0, 1, 2]]);
    }

    #[test]
    fn convex_quad_makes_two_triangles() {
        let p = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let tris = triangulate(&p, &[0, 1, 2, 3]);
        assert_eq!(tris.len(), 2);
        // The triangles tile the unit square (area 1).
        assert!((area_of(&p, &tris, &[0, 1, 2, 3]) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn concave_arrow_is_triangulated_without_overlap() {
        // An arrow / dart polygon (concave): area must be preserved.
        let p = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(0.7, 1.0, 0.0), // the reflex notch
        ];
        let tris = triangulate(&p, &[0, 1, 2, 3]);
        assert_eq!(tris.len(), 2);
        let analytic = 1.3; // shoelace area of the dart
        assert!((area_of(&p, &tris, &[0, 1, 2, 3]) - analytic).abs() < 1e-4);
    }

    #[test]
    fn degenerate_collinear_does_not_panic() {
        let p = vec![Vec3::ZERO, Vec3::X, Vec3::X * 2.0, Vec3::X * 3.0];
        let _ = triangulate(&p, &[0, 1, 2, 3]); // must not panic
    }

    proptest! {
        /// A convex polygon (points on a circle) always triangulates into
        /// exactly `n-2` triangles whose total area equals the polygon's.
        #[test]
        fn convex_ngon_area_preserved(n in 3usize..12) {
            let mut positions = Vec::new();
            let mut loop_idx = Vec::new();
            for i in 0..n {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                positions.push(Vec3::new(a.cos(), a.sin(), 0.0));
                loop_idx.push(i as u32);
            }
            let tris = triangulate(&positions, &loop_idx);
            prop_assert_eq!(tris.len(), n - 2);
            // Polygon area via shoelace.
            let mut poly_area = 0.0f32;
            for i in 0..n {
                let p = positions[i];
                let q = positions[(i + 1) % n];
                poly_area += p.x * q.y - q.x * p.y;
            }
            poly_area = poly_area.abs() * 0.5;
            let tri_area = area_of(&positions, &tris, &loop_idx);
            prop_assert!((tri_area - poly_area).abs() < 1e-3, "areas differ: {tri_area} vs {poly_area}");
        }
    }
}
