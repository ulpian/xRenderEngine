//! The grid raycaster backend (Stage 5.6, feature `grid-raycaster`).
//!
//! A 2-D tile map is rendered with a per-sample-column DDA (Amanatides–Woo) into
//! the standard [`SampleBuffer`], so **every cell shader works on it for free** —
//! block shades for retro, shape-vector for crisp. Walls are distance-shaded with
//! a side-darkening cue and floor/ceiling bands; the map also yields static
//! colliders for the swept-AABB resolver
//! (`RiftEngine-Plan/10-phase-5-game-engine.md` §5.6).

use std::cell::RefCell;

use xre_core::math::Vec2;
use xre_core::math::Vec3;
use xre_render::{Aabb, RowBand, Sample, SampleBuffer, TextureSampler};

/// A 2-D tile map: `0` is empty, any other value is a wall id (used to pick a
/// color). Parsed from a text `.map` grid where `#`/digits are walls.
#[derive(Clone, Debug)]
pub struct TileMap {
    width: u32,
    height: u32,
    tiles: Vec<u8>,
}

impl TileMap {
    /// Parse a text map: each line is a row; `#` and `1`..`9` are walls, ` `/`.`
    /// are empty. Ragged rows are padded with empty cells.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let rows: Vec<&str> = text.lines().filter(|l| !l.is_empty()).collect();
        let width = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0) as u32;
        let height = rows.len() as u32;
        let mut tiles = vec![0u8; (width * height) as usize];
        for (y, row) in rows.iter().enumerate() {
            for (x, ch) in row.chars().enumerate() {
                let id = match ch {
                    '#' => 1,
                    '1'..='9' => ch as u8 - b'0',
                    _ => 0,
                };
                tiles[y * width as usize + x] = id;
            }
        }
        Self {
            width,
            height,
            tiles,
        }
    }

    /// Map width in tiles.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Map height in tiles.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The wall id at `(x, y)` (0 = empty / out of bounds treated as solid).
    #[must_use]
    pub fn tile(&self, x: i32, y: i32) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            return 1; // outside the map is solid
        }
        self.tiles[y as usize * self.width as usize + x as usize]
    }

    /// Whether `(x, y)` is solid.
    #[must_use]
    pub fn is_solid(&self, x: i32, y: i32) -> bool {
        self.tile(x, y) != 0
    }

    /// One axis-aligned box collider per solid tile (unit cubes at integer
    /// coordinates, tall enough to block movement), for the swept resolver.
    #[must_use]
    pub fn colliders(&self) -> Vec<Aabb> {
        let mut out = Vec::new();
        for y in 0..self.height as i32 {
            for x in 0..self.width as i32 {
                if self.is_solid(x, y) {
                    out.push(Aabb {
                        min: Vec3::new(x as f32, -1.0, y as f32),
                        max: Vec3::new(x as f32 + 1.0, 1.0, y as f32 + 1.0),
                    });
                }
            }
        }
        out
    }
}

/// Ambient floor added under a point light so lit areas never go pitch black.
const LIGHT_AMBIENT: f32 = 0.85;

/// A small color palette indexed by wall id.
fn wall_color(id: u8, side: u8) -> [u8; 3] {
    let base = match id {
        1 => [180, 70, 70],
        2 => [70, 160, 90],
        3 => [80, 110, 200],
        4 => [200, 180, 90],
        _ => [150, 150, 150],
    };
    // Darken Y-side walls slightly for a depth cue (u16 math avoids overflow).
    if side == 1 {
        let dim = |c: u8| (u16::from(c) * 7 / 10) as u8;
        [dim(base[0]), dim(base[1]), dim(base[2])]
    } else {
        base
    }
}

/// A faked 2-D point light for the raycaster.
///
/// The raycaster has no real 3-D lighting model, so this is a cheap proximity
/// glow: wall hit points (and, more mildly, the camera) are brightened by their
/// distance to `pos`, blended on top of the existing distance-fog shading. Use
/// it to mark a landmark — e.g. a lamp at the top of the map.
#[derive(Clone, Copy, Debug)]
pub struct PointLight2D {
    /// Light position in tile coordinates (same space as the camera `pos`).
    pub pos: Vec2,
    /// Peak brightness added at the light (before the distance falloff).
    pub intensity: f32,
    /// Falloff radius in tiles: brightness is halved near this distance.
    pub radius: f32,
}

impl PointLight2D {
    /// Proximity brightness `intensity / (1 + (d/radius)^2)` at `at`.
    #[must_use]
    fn brightness(&self, at: Vec2) -> f32 {
        let d = at.distance(self.pos) / self.radius.max(1e-3);
        self.intensity / (1.0 + d * d)
    }
}

/// Renders a [`TileMap`] from a first-person camera into a [`SampleBuffer`].
#[derive(Clone, Copy, Debug)]
pub struct Raycaster {
    /// Horizontal field of view, radians.
    pub fov: f32,
    /// Far clip used to normalize wall depth into `0..1`.
    pub far: f32,
}

impl Default for Raycaster {
    fn default() -> Self {
        Self {
            fov: core::f32::consts::FRAC_PI_3,
            far: 32.0,
        }
    }
}

/// The first wall a single ray meets (see [`Raycaster::raycast`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// Euclidean distance from the ray origin to the wall, in tiles.
    pub distance: f32,
    /// World/tile-space hit point (`pos + dir * distance`).
    pub point: Vec2,
    /// Wall id at the hit tile (`1..=9`).
    pub wall_id: u8,
    /// Which face was struck: `0` = an X-facing face, `1` = a Y-facing face.
    pub side: u8,
    /// Fraction `0.0..1.0` along the hit face (the texture U coordinate).
    pub wall_x: f32,
    /// Unit face normal, pointing back toward the ray origin.
    pub normal: Vec2,
}

impl Raycaster {
    /// Cast a single ray from `pos` along `dir`, returning the first wall it hits.
    ///
    /// Returns `None` only when `dir` is ~zero or the ray escapes the map without
    /// meeting a wall. Out-of-bounds tiles are solid (see [`TileMap::tile`]), so a
    /// bounded map always hits. This shares the same DDA traversal as the rendered
    /// columns, so a pick is consistent with what is drawn.
    #[must_use]
    pub fn raycast(&self, map: &TileMap, pos: Vec2, dir: Vec2) -> Option<RayHit> {
        let len = dir.length();
        if len < 1e-6 {
            return None;
        }
        let dir = dir / len;
        let (dist, side, id, wall_x) = cast(map, pos, dir.x, dir.y);
        if id == 0 {
            return None;
        }
        // The struck face points back toward the origin: opposite the ray's sign
        // on the axis that was crossed.
        let normal = if side == 0 {
            Vec2::new(if dir.x > 0.0 { -1.0 } else { 1.0 }, 0.0)
        } else {
            Vec2::new(0.0, if dir.y > 0.0 { -1.0 } else { 1.0 })
        };
        Some(RayHit {
            distance: dist,
            point: Vec2::new(pos.x + dir.x * dist, pos.y + dir.y * dist),
            wall_id: id,
            side,
            wall_x,
            normal,
        })
    }

    /// The unit ray direction for a normalized horizontal screen coordinate
    /// `screen_x` in `0.0..=1.0` (`0` = the left edge of the FOV, `1` = the right
    /// edge), matching the per-column rays cast by [`Raycaster::render`]:
    /// `yaw − fov/2 + screen_x · fov`.
    #[must_use]
    pub fn ray_dir(&self, yaw: f32, screen_x: f32) -> Vec2 {
        let angle = yaw - self.fov * 0.5 + screen_x.clamp(0.0, 1.0) * self.fov;
        let (s, c) = angle.sin_cos();
        Vec2::new(c, s)
    }

    /// Render the map into `buf`. The camera is at `pos` (tile coordinates)
    /// looking along `yaw` (radians); `pitch` shifts the horizon.
    ///
    /// Walls are flat-colored from the id palette. For textured and/or lit walls
    /// use [`Raycaster::render_textured`].
    pub fn render(&self, buf: &mut SampleBuffer, map: &TileMap, pos: Vec2, yaw: f32, pitch: f32) {
        self.render_textured(buf, map, pos, yaw, pitch, None, None);
    }

    /// Render the map like [`Raycaster::render`], optionally texturing the walls
    /// and adding a faked 2-D point light.
    ///
    /// When `wall_tex` is `Some`, each wall column is sampled from the texture
    /// (Wolfenstein-style: the ray's wall-hit fraction picks the U coordinate and
    /// the screen row picks V) instead of using the flat id palette. When `light`
    /// is `Some`, wall hit points (and, more gently, the floor/ceiling bands) are
    /// brightened by their proximity to the light, blended over the distance fog.
    #[allow(clippy::too_many_arguments)]
    pub fn render_textured(
        &self,
        buf: &mut SampleBuffer,
        map: &TileMap,
        pos: Vec2,
        yaw: f32,
        pitch: f32,
        wall_tex: Option<&dyn TextureSampler>,
        light: Option<PointLight2D>,
    ) {
        self.render_bands(buf, FillKind::Auto, map, pos, yaw, pitch, wall_tex, light);
    }

    /// Render like [`Raycaster::render_textured`] but force the serial or parallel
    /// row-band fill. Exposed only for the determinism gate that asserts the two
    /// paths are byte-identical; prefer [`Raycaster::render_textured`] in real code.
    #[doc(hidden)]
    #[allow(clippy::too_many_arguments)]
    pub fn render_textured_forced(
        &self,
        buf: &mut SampleBuffer,
        parallel: bool,
        map: &TileMap,
        pos: Vec2,
        yaw: f32,
        pitch: f32,
        wall_tex: Option<&dyn TextureSampler>,
        light: Option<PointLight2D>,
    ) {
        let kind = if parallel {
            FillKind::Parallel
        } else {
            FillKind::Serial
        };
        self.render_bands(buf, kind, map, pos, yaw, pitch, wall_tex, light);
    }

    /// The shared raycast: fill the sample buffer in row bands (serial or rayon,
    /// chosen by `kind`). Each band recomputes its columns' DDA spans — a pure,
    /// deterministic function of the column index and camera — then shades its own
    /// rows, so every sample is written exactly once and the output is identical
    /// across bandings (the byte-for-byte determinism the golden tests pin).
    #[allow(clippy::too_many_arguments)]
    fn render_bands(
        self,
        buf: &mut SampleBuffer,
        kind: FillKind,
        map: &TileMap,
        pos: Vec2,
        yaw: f32,
        pitch: f32,
        wall_tex: Option<&dyn TextureSampler>,
        light: Option<PointLight2D>,
    ) {
        let w = buf.width();
        let h = buf.height();
        if w == 0 || h == 0 {
            return;
        }
        let half_fov = self.fov * 0.5;
        let fov = self.fov;
        let far = self.far;
        let horizon = (h as f32 * 0.5) + pitch * h as f32;
        // A camera-relative lift for the floor/ceiling bands near the light.
        let cam_lift = light.map_or(0.0, |l| l.brightness(pos) * 0.4);

        // Each column's DDA span is a pure function of the column and camera, so
        // it is identical for every row band. Compute all `w` columns ONCE here
        // (serial, cheap, deterministic) into a reused thread-local scratch — the
        // per-band closure used to recompute them, which under rayon meant
        // `threads*3` redundant DDA + light passes per frame. The parallel fill
        // then only *reads* the shared slice, so the output stays byte-identical
        // and each sample is still written exactly once. The scratch grows on the
        // warm-up frame and is reused after, preserving the zero-alloc invariant.
        thread_local! {
            static SPANS: RefCell<Vec<ColumnSpan>> = const { RefCell::new(Vec::new()) };
        }
        SPANS.with(|cell| {
            let mut spans = cell.borrow_mut();
            spans.clear();
            spans.extend((0..w).map(|sx| {
                column_span(map, pos, yaw, half_fov, fov, far, h, horizon, light, sx, w)
            }));
            let spans: &[ColumnSpan] = &spans;

            let fill = |band: &mut RowBand| {
                for sx in 0..w {
                    let span = &spans[sx as usize];
                    for y_local in 0..band.height() {
                        let sample = shade_sample(
                            span,
                            (band.y0() + y_local) as f32,
                            horizon,
                            cam_lift,
                            h,
                            wall_tex,
                        );
                        band.put(sx, y_local, sample);
                    }
                }
            };

            match kind {
                FillKind::Auto => buf.par_row_bands(fill),
                FillKind::Serial => buf.par_row_bands_forced(false, fill),
                FillKind::Parallel => buf.par_row_bands_forced(true, fill),
            }
        });
    }
}

/// Which fill path [`Raycaster::render_bands`] takes: `Auto` lets the buffer
/// decide (rayon when worthwhile), the others force it for the determinism gate.
#[derive(Clone, Copy)]
enum FillKind {
    Auto,
    Serial,
    Parallel,
}

/// The per-column data shared by every row of a wall column: the DDA hit (depth,
/// side, texture U), the on-screen wall band, and the precomputed shade/flat
/// color. Computed once per column per band (cheap, deterministic).
#[derive(Clone, Copy)]
struct ColumnSpan {
    depth: f32,
    line_h: f32,
    top: f32,
    start: f32,
    end: f32,
    shade: f32,
    flat: [u8; 3],
    side: u8,
    wall_x: f32,
}

/// Cast the ray for screen column `sx` and reduce it to a [`ColumnSpan`].
#[allow(clippy::too_many_arguments)]
fn column_span(
    map: &TileMap,
    pos: Vec2,
    yaw: f32,
    half_fov: f32,
    fov: f32,
    far: f32,
    h: u32,
    horizon: f32,
    light: Option<PointLight2D>,
    sx: u32,
    w: u32,
) -> ColumnSpan {
    // Ray angle across the FOV.
    let t = sx as f32 / (w.max(2) - 1) as f32; // 0..1
    let angle = yaw - half_fov + t * fov;
    let (rdx, rdy) = (angle.cos(), angle.sin());
    let (dist, side, id, wall_x) = cast(map, pos, rdx, rdy);
    // Correct fish-eye: project onto the view direction.
    let perp = (dist * (angle - yaw).cos()).max(0.05);
    let depth = (perp / far).clamp(0.0, 1.0);
    let line_h = (h as f32 / perp).min(h as f32 * 4.0);
    let top = horizon - line_h * 0.5; // unfloored band top, for V mapping
    let start = top.floor();
    let end = (horizon + line_h * 0.5).ceil();
    let base_shade = (1.0 - depth).clamp(0.05, 1.0);
    // Light at the wall hit point (in tile space), blended over the fog.
    let hit = Vec2::new(pos.x + rdx * dist, pos.y + rdy * dist);
    let lit = light.map_or(1.0, |l| (LIGHT_AMBIENT + l.brightness(hit)).min(1.6));
    let shade = (base_shade * lit).clamp(0.0, 1.0);
    // Flat palette color used when no texture is bound.
    let flat = wall_color(id, side);
    ColumnSpan {
        depth,
        line_h,
        top,
        start,
        end,
        shade,
        flat,
        side,
        wall_x,
    }
}

/// Shade one sample at screen row `yf` of a wall column — ceiling band, textured
/// (or flat) wall, or floor band. Byte-identical to the original inline loop.
fn shade_sample(
    span: &ColumnSpan,
    yf: f32,
    horizon: f32,
    cam_lift: f32,
    h: u32,
    wall_tex: Option<&dyn TextureSampler>,
) -> Sample {
    if yf < span.start {
        // Ceiling band: darken with height, lifted near the light.
        let f = (yf / horizon.max(1.0)).clamp(0.0, 1.0);
        Sample::new(((0.15 + 0.1 * f) + cam_lift).min(1.0), [30, 30, 45], 0.95)
    } else if yf <= span.end {
        let rgb = wall_tex.map_or_else(
            || {
                [
                    (f32::from(span.flat[0]) * span.shade) as u8,
                    (f32::from(span.flat[1]) * span.shade) as u8,
                    (f32::from(span.flat[2]) * span.shade) as u8,
                ]
            },
            |tex| {
                // Texture V from the row's position within the full band.
                let v = ((yf - span.top) / span.line_h.max(1e-3)).clamp(0.0, 1.0);
                let texel = tex.sample(Vec2::new(span.wall_x, v));
                // Y-side faces are dimmed for a depth cue, matching the flat
                // palette's side darkening.
                let side_dim = if span.side == 1 { 0.7 } else { 1.0 };
                [
                    (f32::from(texel[0]) * span.shade * side_dim) as u8,
                    (f32::from(texel[1]) * span.shade * side_dim) as u8,
                    (f32::from(texel[2]) * span.shade * side_dim) as u8,
                ]
            },
        );
        Sample::new((span.shade * 0.9).clamp(0.0, 1.0), rgb, span.depth)
    } else {
        // Floor band: brighten toward the camera, lifted near the light.
        let f = ((h as f32 - yf) / (h as f32 - horizon).max(1.0)).clamp(0.0, 1.0);
        Sample::new(
            ((0.2 + 0.2 * (1.0 - f)) + cam_lift).min(1.0),
            [40, 35, 30],
            0.96,
        )
    }
}

/// DDA from `pos` along `(rdx, rdy)`, returning `(distance, side, wall_id,
/// wall_x)`, where `wall_x` is the `0.0..1.0` fraction along the hit wall face
/// (the texture U coordinate, flipped by ray direction so textures aren't
/// mirrored between opposite faces).
fn cast(map: &TileMap, pos: Vec2, rdx: f32, rdy: f32) -> (f32, u8, u8, f32) {
    let mut map_x = pos.x.floor() as i32;
    let mut map_y = pos.y.floor() as i32;
    let delta_x = if rdx.abs() < 1e-6 {
        1e6
    } else {
        (1.0 / rdx).abs()
    };
    let delta_y = if rdy.abs() < 1e-6 {
        1e6
    } else {
        (1.0 / rdy).abs()
    };
    let (step_x, mut side_x) = if rdx < 0.0 {
        (-1, (pos.x - map_x as f32) * delta_x)
    } else {
        (1, (map_x as f32 + 1.0 - pos.x) * delta_x)
    };
    let (step_y, mut side_y) = if rdy < 0.0 {
        (-1, (pos.y - map_y as f32) * delta_y)
    } else {
        (1, (map_y as f32 + 1.0 - pos.y) * delta_y)
    };

    let mut side = 0u8;
    for _ in 0..256 {
        if side_x < side_y {
            side_x += delta_x;
            map_x += step_x;
            side = 0;
        } else {
            side_y += delta_y;
            map_y += step_y;
            side = 1;
        }
        let id = map.tile(map_x, map_y);
        if id != 0 {
            let dist = if side == 0 {
                side_x - delta_x
            } else {
                side_y - delta_y
            };
            let dist = dist.max(0.05);
            // Where along the hit face the ray landed (the texture U). For an
            // X-side hit it's the Y coordinate of the hit, and vice versa.
            let mut wall_x = if side == 0 {
                pos.y + dist * rdy
            } else {
                pos.x + dist * rdx
            };
            wall_x -= wall_x.floor();
            // Flip so the two opposite faces of a wall aren't mirror images.
            if (side == 0 && rdx > 0.0) || (side == 1 && rdy < 0.0) {
                wall_x = 1.0 - wall_x;
            }
            return (dist, side, id, wall_x);
        }
    }
    (32.0, side, 0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xre_core::math::UVec2;

    const MAP: &str = "\
#####
#...#
#.#.#
#...#
#####";

    #[test]
    fn parse_dimensions_and_solidity() {
        let map = TileMap::parse(MAP);
        assert_eq!(map.width(), 5);
        assert_eq!(map.height(), 5);
        assert!(map.is_solid(0, 0)); // border wall
        assert!(!map.is_solid(1, 1)); // interior
        assert!(map.is_solid(2, 2)); // the pillar
    }

    #[test]
    fn out_of_bounds_is_solid() {
        let map = TileMap::parse(MAP);
        assert!(map.is_solid(-1, 0));
        assert!(map.is_solid(99, 99));
    }

    #[test]
    fn colliders_cover_walls() {
        let map = TileMap::parse(MAP);
        // Border (16) + the single interior pillar = 17 solid tiles.
        assert_eq!(map.colliders().len(), 17);
    }

    #[test]
    fn render_fills_the_buffer() {
        let map = TileMap::parse(MAP);
        let mut buf = SampleBuffer::new(UVec2::new(40, 20), 2, 2);
        buf.clear([0, 0, 0]);
        Raycaster::default().render(&mut buf, &map, Vec2::new(1.5, 1.5), 0.3, 0.0);
        let filled = buf.planes().2.iter().filter(|d| d.is_finite()).count();
        assert!(filled > 0, "raycaster should fill samples");
    }

    #[test]
    fn cast_hits_a_wall() {
        let map = TileMap::parse(MAP);
        // From the interior shooting toward +x hits the right wall.
        let (dist, _side, id, wall_x) = cast(&map, Vec2::new(1.5, 1.5), 1.0, 0.0);
        assert!(id != 0);
        assert!(dist > 0.0 && dist < 5.0);
        // wall_x is always a valid texture U coordinate.
        assert!((0.0..=1.0).contains(&wall_x));
    }

    #[test]
    fn raycast_returns_hit_point_and_normal() {
        let map = TileMap::parse(MAP);
        // From the interior shooting toward +x hits the right wall at x = 4.
        let hit = Raycaster::default()
            .raycast(&map, Vec2::new(1.5, 1.5), Vec2::new(1.0, 0.0))
            .expect("a closed map always hits");
        assert!(hit.wall_id != 0);
        assert!(
            (hit.point.x - 4.0).abs() < 1e-3,
            "hit point on the right wall"
        );
        assert!((hit.point.y - 1.5).abs() < 1e-3, "ray stays on its row");
        assert_eq!(hit.side, 0, "an X-facing face");
        assert_eq!(hit.normal, Vec2::new(-1.0, 0.0), "normal faces the origin");
        assert!((hit.distance - 2.5).abs() < 1e-3);
    }

    #[test]
    fn raycast_normalizes_direction() {
        let map = TileMap::parse(MAP);
        // A non-unit direction must give the same hit as its normalization.
        let a = Raycaster::default()
            .raycast(&map, Vec2::new(1.5, 1.5), Vec2::new(5.0, 0.0))
            .expect("a closed map always hits");
        let b = Raycaster::default()
            .raycast(&map, Vec2::new(1.5, 1.5), Vec2::new(1.0, 0.0))
            .expect("a closed map always hits");
        assert_eq!(a, b);
    }

    #[test]
    fn raycast_rejects_zero_direction() {
        let map = TileMap::parse(MAP);
        assert!(Raycaster::default()
            .raycast(&map, Vec2::new(1.5, 1.5), Vec2::ZERO)
            .is_none());
    }

    #[test]
    fn ray_dir_center_is_forward() {
        let rc = Raycaster::default();
        let yaw = 0.7_f32;
        let dir = rc.ray_dir(yaw, 0.5);
        assert!((dir.x - yaw.cos()).abs() < 1e-6);
        assert!((dir.y - yaw.sin()).abs() < 1e-6);
        // The edges straddle the forward direction by half the FOV.
        let left = rc.ray_dir(yaw, 0.0);
        let right = rc.ray_dir(yaw, 1.0);
        let ang = |d: Vec2| d.y.atan2(d.x);
        assert!((ang(left) - (yaw - rc.fov * 0.5)).abs() < 1e-5);
        assert!((ang(right) - (yaw + rc.fov * 0.5)).abs() < 1e-5);
    }

    #[test]
    fn render_textured_fills_the_buffer() {
        // A 2x2 checker texture and a light; render_textured must fill samples.
        let tex = TestTex;
        let light = PointLight2D {
            pos: Vec2::new(2.5, 1.5),
            intensity: 1.5,
            radius: 4.0,
        };
        let map = TileMap::parse(MAP);
        let mut buf = SampleBuffer::new(UVec2::new(40, 20), 2, 2);
        buf.clear([0, 0, 0]);
        Raycaster::default().render_textured(
            &mut buf,
            &map,
            Vec2::new(1.5, 1.5),
            0.3,
            0.0,
            Some(&tex),
            Some(light),
        );
        let filled = buf.planes().2.iter().filter(|d| d.is_finite()).count();
        assert!(filled > 0, "textured raycaster should fill samples");
    }

    #[test]
    fn render_matches_textured_none() {
        // render() must be exactly render_textured(.., None, None).
        let map = TileMap::parse(MAP);
        let pos = Vec2::new(1.5, 1.5);
        let mut a = SampleBuffer::new(UVec2::new(40, 20), 2, 2);
        let mut b = SampleBuffer::new(UVec2::new(40, 20), 2, 2);
        a.clear([0, 0, 0]);
        b.clear([0, 0, 0]);
        let rc = Raycaster::default();
        rc.render(&mut a, &map, pos, 0.3, 0.0);
        rc.render_textured(&mut b, &map, pos, 0.3, 0.0, None, None);
        assert_eq!(a.planes().2, b.planes().2, "depth planes must match");
    }

    /// A tiny 2x2 checker, just enough to exercise the textured path.
    struct TestTex;
    impl TextureSampler for TestTex {
        fn sample(&self, uv: Vec2) -> [u8; 3] {
            let on = ((uv.x * 2.0) as i32 + (uv.y * 2.0) as i32) % 2 == 0;
            if on {
                [200, 180, 160]
            } else {
                [60, 50, 40]
            }
        }
    }
}
