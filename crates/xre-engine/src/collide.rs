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
            // `>=` (not `>`) so a *touching* contact (`t_near == tmin == 0`) still
            // yields a real face normal instead of leaving it zero — otherwise a
            // mover flush against a wall freezes (no block, no slide).
            if t_near >= tmin {
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
    // Never stay wedged inside geometry: push out of any overlap first, so a
    // mover that has (by floating-point drift, or a teleport) ended up inside a
    // wall can always escape rather than freeze.
    let mut pos = depenetrate(start, half, statics);
    for _ in 0..iterations {
        let mut best: Option<Hit> = None;
        for b in statics {
            if let Some(hit) = swept_box(pos, half, velocity, *b) {
                // A zero normal is a degenerate/grazing contact with no real entry
                // face — treat it as non-blocking so a touching mover can leave.
                if hit.normal.length_squared() < EPS * EPS {
                    continue;
                }
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

/// Push `pos` out of any static it overlaps, along the axis of least penetration.
///
/// Runs a couple of passes so an inside corner (two walls at once) fully
/// resolves. A non-overlapping `pos` is returned unchanged.
fn depenetrate(mut pos: Vec3, half: Vec3, statics: &[Aabb]) -> Vec3 {
    for _ in 0..2 {
        let mut resolved = true;
        for b in statics {
            if let Some(push) = penetration_pushout(pos, half, *b) {
                pos += push;
                resolved = false;
            }
        }
        if resolved {
            break;
        }
    }
    pos
}

/// The minimum-translation push separating a `half`-box centred at `pos` from
/// `box_`, or `None` when they do not overlap.
fn penetration_pushout(pos: Vec3, half: Vec3, box_: Aabb) -> Option<Vec3> {
    let emin = box_.min - half;
    let emax = box_.max + half;
    let mut depth = f32::INFINITY;
    let mut axis_i = 0;
    let mut sign = 0.0;
    for i in 0..3 {
        let o = axis(pos, i);
        let lo = axis(emin, i);
        let hi = axis(emax, i);
        if o <= lo || o >= hi {
            return None; // separated on this axis → no overlap
        }
        let to_lo = o - lo; // distance to exit toward the -axis face
        let to_hi = hi - o; // distance to exit toward the +axis face
        let (d, s) = if to_lo < to_hi {
            (to_lo, -1.0)
        } else {
            (to_hi, 1.0)
        };
        if d < depth {
            depth = d;
            axis_i = i;
            sign = s;
        }
    }
    Some(axis_unit(axis_i, sign) * (depth + EPS))
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

    fn tile(x: f32, z: f32) -> Aabb {
        Aabb {
            min: Vec3::new(x, -1.0, z),
            max: Vec3::new(x + 1.0, 1.0, z + 1.0),
        }
    }

    #[test]
    fn escapes_when_overlapping() {
        // Player center already inside a wall's Minkowski-expanded box, moving
        // out. Must not freeze.
        let half = Vec3::new(0.2, 0.5, 0.2);
        let wall = tile(3.0, 3.0); // expanded x/z slab = [2.8, 4.2]
        let start = Vec3::new(2.81, 0.0, 3.5); // inside expanded box (penetrating)
        let end = move_and_slide(start, half, Vec3::new(-0.1, 0.0, 0.0), &[wall], 4);
        assert!(
            end.x < start.x - 0.01,
            "should escape outward, got {end:?} (start {start:?})"
        );
    }

    #[test]
    fn corner_does_not_trap() {
        // Drive a player into an inside corner of two walls, then try to leave.
        let half = Vec3::new(0.2, 0.5, 0.2);
        let walls = [tile(3.0, 2.0), tile(2.0, 3.0)]; // corner at world (3,3)
        let mut pos = Vec3::new(2.5, 0.0, 2.5);
        for _ in 0..200 {
            pos = move_and_slide(pos, half, Vec3::new(0.04, 0.0, 0.04), &walls, 4);
        }
        let trapped = pos;
        for _ in 0..80 {
            pos = move_and_slide(pos, half, Vec3::new(-0.04, 0.0, -0.04), &walls, 4);
        }
        assert!(
            pos.x < trapped.x - 0.3 && pos.z < trapped.z - 0.3,
            "should escape the corner: trapped={trapped:?} after={pos:?}"
        );
    }

    #[test]
    fn touching_then_slides_along() {
        // Player flush against a wall on +x, then pushing diagonally (into +x and
        // along +z). X is blocked but Z must still slide.
        let half = Vec3::new(0.2, 0.5, 0.2);
        let wall = tile(3.0, 0.0); // x in [3,4], z in [0,1]; expanded x face at 2.8
        let start = Vec3::new(2.8, 0.0, 0.5); // touching the x face, in line in z
        let end = move_and_slide(start, half, Vec3::new(0.1, 0.0, 0.1), &[wall], 4);
        assert!(end.x <= 2.8 + 1e-3, "x must stay blocked: {end:?}");
        assert!(
            end.z > start.z + 0.05,
            "z must slide along the wall: {end:?}"
        );
    }

    fn inside_any(pos: Vec3, half: Vec3, walls: &[Aabb]) -> bool {
        walls.iter().any(|b| {
            let emin = b.min - half;
            let emax = b.max + half;
            pos.x > emin.x + 1e-6
                && pos.x < emax.x - 1e-6
                && pos.z > emin.z + 1e-6
                && pos.z < emax.z - 1e-6
        })
    }

    #[test]
    fn corner_approach_never_penetrates() {
        // Replicate step_player: yaw-relative diagonal (un-normalized!) into an
        // inside corner from many angles; report if the player ever penetrates.
        let half = Vec3::new(0.2, 0.5, 0.2);
        let walls = [tile(3.0, 2.0), tile(2.0, 3.0)]; // corner at world (3,3)
        let speed = 2.5;
        let dt = 1.0 / 60.0;
        let mut penetrated = 0;
        for a in 0..360 {
            let yaw = (a as f32).to_radians();
            let (sy, cy) = yaw.sin_cos();
            let forward = Vec3::new(cy, 0.0, sy);
            let rightv = Vec3::new(-sy, 0.0, cy);
            let mut pos = Vec3::new(2.5, 0.0, 2.5);
            for _ in 0..240 {
                // hold W+D (forward + strafe right), as in the FPS
                let vel = (forward + rightv) * (speed * dt);
                pos = move_and_slide(pos, half, vel, &walls, 4);
                if inside_any(pos, half, &walls) {
                    penetrated += 1;
                    break;
                }
            }
        }
        assert_eq!(
            penetrated, 0,
            "player penetrated a wall in {penetrated}/360 approaches"
        );
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
