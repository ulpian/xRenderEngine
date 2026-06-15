//! Criterion benchmarks for the render pipeline stages (Stage 4.5).
//!
//! These establish baselines for the rasterizer, the cell shaders, and the full
//! per-frame cost (raster + shade) so CI can flag regressions against the spec §D
//! budget. With the `parallel` feature the rasterizer and `resolve_cells` run
//! row-parallel; compare against `--no-default-features` to read the speedup.
#![allow(missing_docs)] // criterion_group!/main! generate undocumented pub items

use criterion::{criterion_group, criterion_main, Criterion};
use std::hint::black_box;
use xre_core::math::{UVec2, Vec3};
use xre_core::{Cell, Transform};
use xre_render::{
    draw_mesh, resolve_cells, BlockShades, Braille, Camera, CellShader, Cull, HalfBlock, LightRig,
    LuminanceRamp, Material, Mesh, Projection, Rasterizer, SampleBuffer, ShadeMode, ShapeVector,
};

fn bench_raster(c: &mut Criterion) {
    let mesh = Mesh::uv_sphere(1.0, 32, 48);
    let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
    let vp = cam.view_projection(120, 36, Projection::DEFAULT_CELL_ASPECT);
    let rig = LightRig::default();
    let mut buf = SampleBuffer::new(UVec2::new(120, 36), 2, 4);
    c.bench_function("raster_sphere_120x36", |b| {
        b.iter(|| {
            buf.clear([0, 0, 0]);
            draw_mesh(
                &mut buf,
                &mesh,
                Transform::IDENTITY.to_mat4(),
                vp,
                &rig,
                &Material::default(),
                ShadeMode::PerSample,
                Cull::Back,
            );
            black_box(&buf);
        });
    });
}

fn bench_shaders(c: &mut Criterion) {
    let mesh = Mesh::torus(1.2, 0.4, 32, 16);
    let cam = Camera::look_at(Vec3::new(0.0, 1.5, 3.0), Vec3::ZERO);
    let vp = cam.view_projection(120, 36, Projection::DEFAULT_CELL_ASPECT);
    let mut buf = SampleBuffer::new(UVec2::new(120, 36), 2, 4);
    buf.clear([0, 0, 0]);
    draw_mesh(
        &mut buf,
        &mesh,
        Transform::IDENTITY.to_mat4(),
        vp,
        &LightRig::default(),
        &Material::default(),
        ShadeMode::PerSample,
        Cull::Back,
    );

    let ramp = LuminanceRamp::default();
    let shape = ShapeVector::default();
    let mut group = c.benchmark_group("cell_shade_120x36");
    // Resolve the whole grid through `resolve_cells` — the row-parallel path the
    // Viewport3D uses — into a reusable scratch, so this tracks real frame cost.
    let run = |g: &mut criterion::BenchmarkGroup<_>, name: &str, s: &dyn CellShader| {
        let (cols, rows) = (buf.cells().x, buf.cells().y);
        let mut scratch: Vec<Option<Cell>> = vec![None; (cols * rows) as usize];
        g.bench_function(name, |b| {
            b.iter(|| {
                resolve_cells(&buf, s, cols, rows, &mut scratch);
                black_box(&scratch);
            });
        });
    };
    run(&mut group, "ramp", &ramp);
    run(&mut group, "shape", &shape);
    run(&mut group, "halfblock", &HalfBlock);
    run(&mut group, "blocks", &BlockShades);
    run(&mut group, "braille", &Braille::default());
    group.finish();
}

/// The full per-frame cost (rasterize + cell shade) at the spec §D reference
/// viewport — the number the ≤ 8 ms budget gates.
fn bench_frame(c: &mut Criterion) {
    let mesh = Mesh::uv_sphere(1.0, 32, 48);
    let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
    let vp = cam.view_projection(120, 36, Projection::DEFAULT_CELL_ASPECT);
    let rig = LightRig::default();
    let shader = ShapeVector::default();
    let mut buf = SampleBuffer::new(UVec2::new(120, 36), 2, 4);
    let mut rz = Rasterizer::new();
    let mut cells: Vec<Option<Cell>> = vec![None; 120 * 36];
    c.bench_function("frame_sphere_120x36", |b| {
        b.iter(|| {
            buf.clear([0, 0, 0]);
            rz.draw_mesh(
                &mut buf,
                &mesh,
                Transform::IDENTITY.to_mat4(),
                vp,
                &rig,
                &Material::default(),
                ShadeMode::PerSample,
                Cull::Back,
            );
            resolve_cells(&buf, &shader, 120, 36, &mut cells);
            black_box(&cells);
        });
    });
}

criterion_group!(benches, bench_raster, bench_shaders, bench_frame);
criterion_main!(benches);
