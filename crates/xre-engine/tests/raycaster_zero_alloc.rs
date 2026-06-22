//! Zero-allocation-per-frame gate for the grid raycaster (Stage 4.5).
//!
//! Built only under `--features dhat-heap`, which links the dhat heap profiler as
//! the global allocator. We warm one frame (the sample buffer grows once), then
//! assert a steady-state frame allocates **nothing** — the raycaster fills the
//! persistent buffer in place, with no per-frame scratch.
//!
//! The viewport is sized below the parallel threshold so the measured frame runs
//! the serial fill, excluding rayon's one-time pool setup from the count (the same
//! technique as `xre-render`'s zero-alloc test).
//!
//! Run with:
//! `cargo test -p xre-engine --features "dhat-heap grid-raycaster" --test raycaster_zero_alloc`.
#![cfg(all(feature = "dhat-heap", feature = "grid-raycaster"))]
#![allow(clippy::unwrap_used, clippy::cast_precision_loss)]

use xre_core::math::{UVec2, Vec2};
use xre_engine::raycaster::{PointLight2D, Raycaster, TileMap};
use xre_render::{SampleBuffer, TextureSampler};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

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

#[test]
fn steady_state_raycast_allocates_nothing() {
    let _profiler = dhat::Profiler::builder().testing().build();

    let map = TileMap::parse(MAP);
    let rc = Raycaster::default();
    let tex = Checker;
    let light = PointLight2D {
        pos: Vec2::new(map.width() as f32 / 2.0, 1.5),
        intensity: 1.4,
        radius: 5.0,
    };
    // 30x20 cells at 2x4 = 60x80 = 4800 samples — below the parallel threshold,
    // so the serial fill runs and no rayon pool is spun up.
    let mut buf = SampleBuffer::new(UVec2::new(30, 20), 2, 4);

    let frame = |buf: &mut SampleBuffer| {
        buf.clear([10, 10, 16]);
        rc.render_textured(
            buf,
            &map,
            Vec2::new(1.5, 1.5),
            0.6,
            0.05,
            Some(&tex),
            Some(light),
        );
    };

    // Warm-up: grows the sample buffer once.
    frame(&mut buf);

    let before = dhat::HeapStats::get();
    frame(&mut buf);
    let after = dhat::HeapStats::get();

    assert_eq!(
        after.total_blocks,
        before.total_blocks,
        "steady-state raycast allocated {} block(s)",
        after.total_blocks - before.total_blocks
    );
    assert_eq!(
        after.total_bytes,
        before.total_bytes,
        "steady-state raycast allocated {} byte(s)",
        after.total_bytes - before.total_bytes
    );
}
