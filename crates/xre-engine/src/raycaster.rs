//! The grid raycaster backend (Stage 5.6, feature `grid-raycaster`).
//!
//! A 2-D tile map is rendered with a per-sample-column DDA (Amanatides–Woo) into
//! the standard [`SampleBuffer`], so **every cell shader works on it for free** —
//! block shades for retro, shape-vector for crisp. Walls are distance-shaded with
//! a side-darkening cue and floor/ceiling bands; the map also yields static
//! colliders for the swept-AABB resolver
//! (`RiftEngine-Plan/10-phase-5-game-engine.md` §5.6).

use xre_core::math::Vec2;
use xre_core::math::Vec3;
use xre_render::{Aabb, Sample, SampleBuffer};

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

impl Raycaster {
    /// Render the map into `buf`. The camera is at `pos` (tile coordinates)
    /// looking along `yaw` (radians); `pitch` shifts the horizon.
    pub fn render(&self, buf: &mut SampleBuffer, map: &TileMap, pos: Vec2, yaw: f32, pitch: f32) {
        let w = buf.width();
        let h = buf.height();
        if w == 0 || h == 0 {
            return;
        }
        let half_fov = self.fov * 0.5;
        let horizon = (h as f32 * 0.5) + pitch * h as f32;

        for sx in 0..w {
            // Ray angle across the FOV.
            let t = sx as f32 / (w.max(2) - 1) as f32; // 0..1
            let angle = yaw - half_fov + t * self.fov;
            let (rdx, rdy) = (angle.cos(), angle.sin());
            let (dist, side, id) = cast(map, pos, rdx, rdy);
            // Correct fish-eye: project onto the view direction.
            let perp = (dist * (angle - yaw).cos()).max(0.05);
            let depth = (perp / self.far).clamp(0.0, 1.0);

            let line_h = (h as f32 / perp).min(h as f32 * 4.0);
            let start = (horizon - line_h * 0.5).floor();
            let end = (horizon + line_h * 0.5).ceil();
            let shade = (1.0 - depth).clamp(0.05, 1.0);
            let rgb = wall_color(id, side);
            let wall_rgb = [
                (f32::from(rgb[0]) * shade) as u8,
                (f32::from(rgb[1]) * shade) as u8,
                (f32::from(rgb[2]) * shade) as u8,
            ];

            for sy in 0..h {
                let yf = sy as f32;
                let sample = if yf < start {
                    // Ceiling band: darken with height.
                    let f = (yf / horizon.max(1.0)).clamp(0.0, 1.0);
                    Sample::new(0.15 + 0.1 * f, [30, 30, 45], 0.95)
                } else if yf <= end {
                    Sample::new((shade * 0.9).clamp(0.0, 1.0), wall_rgb, depth)
                } else {
                    // Floor band: brighten toward the camera.
                    let f = ((h as f32 - yf) / (h as f32 - horizon).max(1.0)).clamp(0.0, 1.0);
                    Sample::new(0.2 + 0.2 * (1.0 - f), [40, 35, 30], 0.96)
                };
                buf.put(sx, sy, sample);
            }
        }
    }
}

/// DDA from `pos` along `(rdx, rdy)`, returning `(distance, side, wall_id)`.
fn cast(map: &TileMap, pos: Vec2, rdx: f32, rdy: f32) -> (f32, u8, u8) {
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
            return (dist.max(0.05), side, id);
        }
    }
    (32.0, side, 0)
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
        let (dist, _side, id) = cast(&map, Vec2::new(1.5, 1.5), 1.0, 0.0);
        assert!(id != 0);
        assert!(dist > 0.0 && dist < 5.0);
    }
}
