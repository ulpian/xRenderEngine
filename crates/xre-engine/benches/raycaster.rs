//! Criterion benchmark for the grid raycaster (Stage 4.5 throughput baseline).
//!
//! Renders a fixed 120x36 textured + lit viewport via `render_textured`, forcing
//! the serial and parallel row-band fills so CI can read the speedup directly.
//! With the `parallel` feature on, `auto` matches the `parallel` case.
//!
//! Run with: `cargo bench -p xre-engine --features grid-raycaster --bench raycaster`.
#![allow(missing_docs)] // criterion_group!/main! generate undocumented pub items
#![allow(clippy::cast_precision_loss)]

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
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

fn bench_raycaster(c: &mut Criterion) {
    let map = TileMap::parse(MAP);
    let rc = Raycaster::default();
    let tex = Checker;
    let light = PointLight2D {
        pos: Vec2::new(map.width() as f32 / 2.0, 1.5),
        intensity: 1.4,
        radius: 5.0,
    };
    let mut buf = SampleBuffer::new(UVec2::new(120, 36), 2, 4);
    let (pos, yaw, pitch) = (Vec2::new(1.5, 1.5), 0.6, 0.05);

    let mut group = c.benchmark_group("raycaster_120x36");
    group.bench_function("serial", |b| {
        b.iter(|| {
            buf.clear([10, 10, 16]);
            rc.render_textured_forced(
                &mut buf,
                false,
                &map,
                pos,
                yaw,
                pitch,
                Some(&tex),
                Some(light),
            );
            black_box(&buf);
        });
    });
    group.bench_function("parallel", |b| {
        b.iter(|| {
            buf.clear([10, 10, 16]);
            rc.render_textured_forced(
                &mut buf,
                true,
                &map,
                pos,
                yaw,
                pitch,
                Some(&tex),
                Some(light),
            );
            black_box(&buf);
        });
    });
    group.finish();
}

criterion_group!(benches, bench_raycaster);
criterion_main!(benches);
