//! Determinism gate: the serial and parallel row-band raycaster fills must be
//! **byte-identical** (the Stage 4.5 invariant the golden frames depend on).
//! Mirrors the rasterizer's `*_parallel_is_bit_identical_to_serial` tests.
//!
//! Run with: `cargo test -p xre-engine --features grid-raycaster --test raycaster_determinism`.
#![cfg(feature = "grid-raycaster")]
#![allow(clippy::unwrap_used, clippy::cast_precision_loss)]

use xre_core::math::{UVec2, Vec2};
use xre_engine::raycaster::{PointLight2D, Raycaster, TileMap};
use xre_render::{SampleBuffer, TextureSampler};

const MAP: &str = "\
###############
#......#......#
#.####.#.####.#
#.#..........#.
#.#.####.###.#.
#...#......#..E#
#.#.#.####.#.#.#
#.#......P...#.#
#.############.#
#.....P.......#.#
###############";

/// A tiny deterministic checker texture, enough to exercise the textured path.
struct Checker;
impl TextureSampler for Checker {
    fn sample(&self, uv: Vec2) -> [u8; 3] {
        let on = ((uv.x * 8.0) as i32 + (uv.y * 8.0) as i32) % 2 == 0;
        if on {
            [200, 180, 160]
        } else {
            [60, 50, 40]
        }
    }
}

/// True iff two buffers are bit-identical across all three planes (f32 bit
/// patterns, so this is the real golden-frame equality — NaN included).
fn planes_bit_equal(a: &SampleBuffer, b: &SampleBuffer) -> bool {
    let (al, ar, ad) = a.planes();
    let (bl, br, bd) = b.planes();
    ar == br
        && al.len() == bl.len()
        && ad.len() == bd.len()
        && al.iter().zip(bl).all(|(x, y)| x.to_bits() == y.to_bits())
        && ad.iter().zip(bd).all(|(x, y)| x.to_bits() == y.to_bits())
}

fn scene() -> (TileMap, Raycaster, PointLight2D, Vec2, f32, f32) {
    let map = TileMap::parse(MAP);
    let light = PointLight2D {
        pos: Vec2::new(map.width() as f32 / 2.0, 1.5),
        intensity: 1.4,
        radius: 5.0,
    };
    (
        map,
        Raycaster::default(),
        light,
        Vec2::new(1.5, 1.5),
        0.6,
        0.05,
    )
}

#[test]
fn serial_and_parallel_raycaster_are_bit_identical() {
    let (map, rc, light, pos, yaw, pitch) = scene();
    let tex = Checker;
    // 120x36 cells at 2x4 = 240x144 = 34,560 samples — well over the parallel
    // threshold, so the parallel path really splits into multiple bands.
    let size = UVec2::new(120, 36);

    let mut serial = SampleBuffer::new(size, 2, 4);
    let mut parallel = SampleBuffer::new(size, 2, 4);
    serial.clear([10, 10, 16]);
    parallel.clear([10, 10, 16]);
    rc.render_textured_forced(
        &mut serial,
        false,
        &map,
        pos,
        yaw,
        pitch,
        Some(&tex),
        Some(light),
    );
    rc.render_textured_forced(
        &mut parallel,
        true,
        &map,
        pos,
        yaw,
        pitch,
        Some(&tex),
        Some(light),
    );

    assert!(
        planes_bit_equal(&serial, &parallel),
        "parallel raycaster diverged from serial"
    );
}

#[test]
fn auto_path_matches_forced_serial() {
    let (map, rc, light, pos, yaw, pitch) = scene();
    let size = UVec2::new(120, 36);

    let mut auto = SampleBuffer::new(size, 2, 4);
    let mut serial = SampleBuffer::new(size, 2, 4);
    auto.clear([10, 10, 16]);
    serial.clear([10, 10, 16]);
    rc.render_textured(&mut auto, &map, pos, yaw, pitch, None, Some(light));
    rc.render_textured_forced(&mut serial, false, &map, pos, yaw, pitch, None, Some(light));

    assert!(
        planes_bit_equal(&auto, &serial),
        "auto raycaster path diverged from forced-serial"
    );
}
