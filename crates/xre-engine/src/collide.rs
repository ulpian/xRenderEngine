//! Collision-lite: AABB/sphere colliders, swept AABB resolution and a
//! uniform-grid broadphase (Stage 5.5).
//!
//! 0.1's physics is "stop, slide, trigger" — no impulse solver. The swept test
//! (Minkowski-expanded box vs. a ray) makes movement **tunnel-proof at any
//! speed/`dt`**, the property the game loop relies on
//! (`RiftEngine-Plan/10-phase-5-game-engine.md` §5.5).

use std::collections::HashMap;

use xre_core::math::Vec3;
use xre_render::Aabb;

const EPS: f32 = 1e-4;

#[inline]
const fn axis(v: Vec3, i: usize) -> f32 {
    match i {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

#[inline]
const fn axis_unit(i: usize, sign: f32) -> Vec3 {
    match i {
        0 => Vec3::new(sign, 0.0, 0.0),
        1 => Vec3::new(0.0, sign, 0.0),
        _ => Vec3::new(0.0, 0.0, sign),
    }
}

/// A box collider as a center and half-extents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxCollider {
    /// Center position.
    pub center: Vec3,
    /// Half-extents on each axis.
    pub half: Vec3,
}

impl BoxCollider {
    /// A collider from center and half-extents.
    #[must_use]
    pub const fn new(center: Vec3, half: Vec3) -> Self {
        Self { center, half }
    }

    /// The world-space AABB.
    #[must_use]
    pub fn aabb(&self) -> Aabb {
        Aabb {
            min: self.center - self.half,
            max: self.center + self.half,
        }
    }

    /// Whether this collider overlaps `other`.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        let a = self.aabb();
        let b = other.aabb();
        a.min.x <= b.max.x
            && a.max.x >= b.min.x
            && a.min.y <= b.max.y
            && a.max.y >= b.min.y
            && a.min.z <= b.max.z
            && a.max.z >= b.min.z
    }
}

/// A sphere collider.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sphere {
    /// Center.
    pub center: Vec3,
    /// Radius.
    pub radius: f32,
}

impl Sphere {
    /// Whether two spheres overlap.
    #[must_use]
    pub fn overlaps(&self, other: &Self) -> bool {
        let r = self.radius + other.radius;
        self.center.distance_squared(other.center) <= r * r
    }
}

/// The result of a swept collision: the fraction of the move completed before
/// contact and the contact face normal.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    /// Time of impact in `0.0..=1.0` along the attempted move.
    pub toi: f32,
    /// The contact face normal (unit, points away from the static box).
    pub normal: Vec3,
}

/// Sweep a box against a static AABB (Minkowski expansion + slab ray test).
///
/// The box of half-extents `half` centred at `start` moves along `velocity`
/// against `box_`. Returns the earliest entry, or `None` if it does not hit
/// within `0..1`.
#[must_use]
pub fn swept_box(start: Vec3, half: Vec3, velocity: Vec3, box_: Aabb) -> Option<Hit> {
    let emin = box_.min - half;
    let emax = box_.max + half;
    let mut tmin = 0.0f32;
    let mut tmax = 1.0f32;
    let mut normal = Vec3::ZERO;
    for i in 0..3 {
        let d = axis(velocity, i);
        let o = axis(start, i);
        let lo = axis(emin, i);
        let hi = axis(emax, i);
        if d.abs() < EPS {
            if o < lo || o > hi {
                return None; // parallel and outside this slab
            }
        } else {
            let inv = 1.0 / d;
            let ta = (lo - o) * inv;
            let tb = (hi - o) * inv;
            // Near/far entry times, with the sign of the face being entered.
            let (t_near, t_far, sign) = if ta > tb {
                (tb, ta, 1.0)
            } else {
                (ta, tb, -1.0)
            };
            if t_near > tmin {
                tmin = t_near;
                normal = axis_unit(i, sign);
            }
            if t_far < tmax {
                tmax = t_far;
            }
            if tmin > tmax {
                return None;
            }
        }
    }
    Some(Hit { toi: tmin, normal })
}

/// Move a box against statics, stopping and sliding along contacts.
///
/// Up to `iterations` resolution passes run; each uses the swept test, so the
/// move is tunnel-proof — no static is skipped however fast the mover travels.
#[must_use]
pub fn move_and_slide(
    start: Vec3,
    half: Vec3,
    mut velocity: Vec3,
    statics: &[Aabb],
    iterations: u32,
) -> Vec3 {
    let mut pos = start;
    for _ in 0..iterations {
        let mut best: Option<Hit> = None;
        for b in statics {
            if let Some(hit) = swept_box(pos, half, velocity, *b) {
                if best.is_none_or(|h| hit.toi < h.toi) {
                    best = Some(hit);
                }
            }
        }
        match best {
            None => {
                pos += velocity;
                break;
            }
            Some(hit) => {
                let contact = (hit.toi - EPS).max(0.0);
                pos += velocity * contact;
                // Slide: remove the velocity component into the surface.
                let remaining = velocity * (1.0 - contact);
                velocity = remaining - hit.normal * remaining.dot(hit.normal);
                if velocity.length_squared() < EPS * EPS {
                    break;
                }
            }
        }
    }
    pos
}

/// A uniform-grid broadphase mapping cells to the indices of overlapping boxes.
#[derive(Debug, Default)]
pub struct UniformGrid {
    cell: f32,
    buckets: HashMap<(i32, i32, i32), Vec<usize>>,
}

impl UniformGrid {
    /// A grid with the given cell size.
    #[must_use]
    pub fn new(cell: f32) -> Self {
        Self {
            cell: cell.max(EPS),
            buckets: HashMap::new(),
        }
    }

    fn key(&self, p: Vec3) -> (i32, i32, i32) {
        (
            (p.x / self.cell).floor() as i32,
            (p.y / self.cell).floor() as i32,
            (p.z / self.cell).floor() as i32,
        )
    }

    /// Insert box index `idx` covering AABB `b`.
    pub fn insert(&mut self, idx: usize, b: Aabb) {
        let lo = self.key(b.min);
        let hi = self.key(b.max);
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    self.buckets.entry((x, y, z)).or_default().push(idx);
                }
            }
        }
    }

    /// Candidate box indices overlapping `query` (deduplicated).
    #[must_use]
    pub fn query(&self, query: Aabb) -> Vec<usize> {
        let lo = self.key(query.min);
        let hi = self.key(query.max);
        let mut out = Vec::new();
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                for z in lo.2..=hi.2 {
                    if let Some(v) = self.buckets.get(&(x, y, z)) {
                        out.extend_from_slice(v);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use proptest::prelude::*;

    fn wall(x: f32) -> Aabb {
        // A thin wall at x, spanning a tall/wide area.
        Aabb {
            min: Vec3::new(x - 0.05, -10.0, -10.0),
            max: Vec3::new(x + 0.05, 10.0, 10.0),
        }
    }

    #[test]
    fn swept_detects_frontal_hit() {
        let hit = swept_box(
            Vec3::ZERO,
            Vec3::splat(0.1),
            Vec3::new(1.0, 0.0, 0.0),
            wall(0.5),
        );
        assert!(hit.is_some());
        let h = hit.unwrap();
        assert!(h.toi > 0.0 && h.toi < 1.0);
        assert!(h.normal.x < 0.0, "normal should face the mover");
    }

    #[test]
    fn slide_along_wall_keeps_tangential_motion() {
        // Move diagonally into a wall on +X: X is stopped, Z slides through.
        let end = move_and_slide(
            Vec3::ZERO,
            Vec3::splat(0.1),
            Vec3::new(1.0, 0.0, 1.0),
            &[wall(0.5)],
            4,
        );
        assert!(end.x < 0.5, "should not pass the wall: {end:?}");
        assert!(end.z > 0.5, "tangential motion should continue: {end:?}");
    }

    #[test]
    fn grid_query_returns_candidates() {
        let mut grid = UniformGrid::new(1.0);
        grid.insert(0, wall(0.5));
        grid.insert(1, wall(5.0));
        let near = grid.query(Aabb {
            min: Vec3::new(0.0, 0.0, 0.0),
            max: Vec3::new(0.6, 0.1, 0.1),
        });
        assert!(near.contains(&0));
        assert!(!near.contains(&1));
    }

    proptest! {
        /// Swept movement never tunnels through a wall at any speed or step.
        #[test]
        fn no_tunneling(speed in 0.1f32..1000.0, dir in -1.0f32..1.0) {
            let half = Vec3::splat(0.1);
            // Start left of the wall, move right at an arbitrary speed.
            let vel = Vec3::new(speed, dir, dir);
            let end = move_and_slide(Vec3::new(-1.0, 0.0, 0.0), half, vel, &[wall(0.0)], 4);
            // The mover's near face must never cross to the far side of the wall.
            prop_assert!(end.x + half.x <= 0.05 + 1e-2, "tunneled to x={}", end.x);
        }
    }
}
