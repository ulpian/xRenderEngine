//! `xre bench` — report the render-pipeline timings on the user's machine
//! (Stage 4.5 / 6.3). Mirrors the criterion baselines so users can compare their
//! hardware to the documented budget.

use std::time::Instant;

use xre::core::math::{UVec2, Vec3};
use xre::core::{Cell, Transform};
use xre::prelude::*;

/// How the renderer was compiled — surfaced so the reported numbers are read in
/// the right light (the row-parallel path is on by default).
const PARALLELISM: &str = if cfg!(feature = "parallel") {
    "row-parallel (rayon)"
} else {
    "single core"
};

/// Run the benchmark suite and print a small table.
#[allow(clippy::unnecessary_wraps)] // uniform Result signature across subcommands
pub fn run(_args: &[String]) -> Result<(), String> {
    println!("xre bench — software render pipeline [{PARALLELISM}]\n");
    println!("{:<28} {:>10} {:>12}", "scene", "tris", "draw (ms)");
    println!("{}", "-".repeat(52));

    bench("cube 120x36", &Mesh::cube(), 120, 36);
    bench("sphere 120x36", &Mesh::uv_sphere(1.0, 32, 48), 120, 36);
    bench("torus 120x36", &Mesh::torus(1.2, 0.4, 48, 24), 120, 36);
    bench("sphere 200x60", &Mesh::uv_sphere(1.0, 48, 64), 200, 60);

    // Cell-shade throughput on a filled buffer.
    let mut buf = SampleBuffer::new(UVec2::new(120, 36), 2, 4);
    fill(&mut buf, &Mesh::uv_sphere(1.0, 32, 48), 120, 36);
    println!(
        "\n{:<28} {:>10} {:>12}",
        "cell shader 120x36", "", "shade (ms)"
    );
    println!("{}", "-".repeat(52));
    shade_bench("LuminanceRamp", &buf, &LuminanceRamp::default());
    shade_bench("ShapeVector", &buf, &ShapeVector::default());
    shade_bench("HalfBlock", &buf, &HalfBlock);
    shade_bench("Braille", &buf, &Braille::default());
    Ok(())
}

fn fill(buf: &mut SampleBuffer, mesh: &Mesh, cols: u32, rows: u32) {
    buf.clear([0, 0, 0]);
    let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
    let vp = cam.view_projection(cols, rows, Projection::DEFAULT_CELL_ASPECT);
    draw_mesh(
        buf,
        mesh,
        Transform::IDENTITY.to_mat4(),
        vp,
        &LightRig::default(),
        &Material::default(),
        ShadeMode::PerSample,
        Cull::Back,
    );
}

fn bench(name: &str, mesh: &Mesh, cols: u32, rows: u32) {
    let mut buf = SampleBuffer::new(UVec2::new(cols, rows), 2, 4);
    // Warm up, then time a batch.
    fill(&mut buf, mesh, cols, rows);
    let iters = 50;
    let start = Instant::now();
    for _ in 0..iters {
        fill(&mut buf, mesh, cols, rows);
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(iters);
    println!("{name:<28} {:>10} {ms:>12.3}", mesh.triangle_count());
}

fn shade_bench(name: &str, buf: &SampleBuffer, shader: &dyn CellShader) {
    let (cols, rows) = (buf.cells().x, buf.cells().y);
    let mut out: Vec<Option<Cell>> = vec![None; (cols * rows) as usize];
    let iters = 100;
    let start = Instant::now();
    for _ in 0..iters {
        resolve_cells(buf, shader, cols, rows, &mut out);
        std::hint::black_box(&out);
    }
    let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(iters);
    println!("{name:<28} {:>10} {ms:>12.3}", "");
}
