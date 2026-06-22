//! Golden snapshot of a raycast frame: pins the raycaster's exact output so any
//! accidental change to its arithmetic (e.g. during the parallel port) is caught.
//! Frames are text, so we snapshot the shaded glyph grid via `insta`.
//!
//! Run with: `cargo test -p xre-engine --features grid-raycaster --test raycaster_golden`.
#![cfg(feature = "grid-raycaster")]
#![allow(clippy::unwrap_used, clippy::cast_precision_loss)]

use xre_core::math::{UVec2, Vec2};
use xre_engine::raycaster::{PointLight2D, Raycaster, TileMap};
use xre_render::{builtin_cell_shaders, SampleBuffer};

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

/// Render a fixed scene and reduce it to an ASCII grid with the `ramp` shader.
fn render_ascii(parallel: bool) -> String {
    let map = TileMap::parse(MAP);
    let rc = Raycaster::default();
    let light = PointLight2D {
        pos: Vec2::new(map.width() as f32 / 2.0, 1.5),
        intensity: 1.4,
        radius: 5.0,
    };
    let size = UVec2::new(60, 24);
    let mut buf = SampleBuffer::new(size, 2, 4);
    buf.clear([10, 10, 16]);
    rc.render_textured_forced(
        &mut buf,
        parallel,
        &map,
        Vec2::new(1.5, 1.5),
        0.6,
        0.05,
        None,
        Some(light),
    );

    let shaders = builtin_cell_shaders();
    let (_, shader) = shaders.iter().find(|(n, _)| *n == "ramp").unwrap();
    let cells = buf.cells();
    let mut out = String::with_capacity(((cells.x + 1) * cells.y) as usize);
    for cy in 0..cells.y {
        for cx in 0..cells.x {
            out.push(shader.shade(&buf, cx, cy).map_or(' ', |c| c.glyph));
        }
        out.push('\n');
    }
    out
}

#[test]
fn raycast_frame_matches_golden() {
    insta::assert_snapshot!("raycast_frame", render_ascii(false));
}

#[test]
fn parallel_frame_matches_golden() {
    // Same snapshot name → also asserts the parallel path equals the golden.
    insta::assert_snapshot!("raycast_frame", render_ascii(true));
}
