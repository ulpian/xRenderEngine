//! Zero-allocation-per-frame gate (Stage 4.5, `RiftEngine-Plan/03-rendering-pipeline-spec.md` §D).
//!
//! Built only under `--features dhat-heap`, which links the dhat heap profiler as
//! the global allocator. We warm a [`Rasterizer`] (the first frame grows its
//! scratch), then assert a steady-state frame allocates **nothing** — the
//! persistent-buffer invariant the perf budget depends on.
//!
//! The viewport here is sized below the parallel threshold so the measured frame
//! runs the serial fill: it exercises this crate's scratch reuse without folding
//! in rayon's one-time pool setup.
//!
//! Run with: `cargo test -p xre-render --features dhat-heap --test zero_alloc`.
#![allow(clippy::unwrap_used)]

use xre_core::math::{UVec2, Vec3};
use xre_render::{
    Camera, Cull, LightRig, Material, Mesh, Projection, Rasterizer, SampleBuffer, ShadeMode,
};

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[test]
fn steady_state_frame_allocates_nothing() {
    let _profiler = dhat::Profiler::builder().testing().build();

    let mesh = Mesh::uv_sphere(1.0, 24, 32);
    let cam = Camera::look_at(Vec3::new(0.0, 0.0, 3.0), Vec3::ZERO);
    let vp = cam.view_projection(40, 20, Projection::DEFAULT_CELL_ASPECT);
    let rig = LightRig::default();
    let material = Material::default();
    let mut buf = SampleBuffer::new(UVec2::new(40, 20), 2, 4);
    let mut rz = Rasterizer::new();

    let frame = |buf: &mut SampleBuffer, rz: &mut Rasterizer| {
        buf.clear([0, 0, 0]);
        rz.draw_mesh(
            buf,
            &mesh,
            xre_core::Transform::IDENTITY.to_mat4(),
            vp,
            &rig,
            &material,
            ShadeMode::PerSample,
            Cull::Back,
        );
    };

    // Warm-up: grows the persistent scratch and the sample buffer.
    frame(&mut buf, &mut rz);

    // Steady state: a second identical frame must not touch the allocator.
    let before = dhat::HeapStats::get();
    frame(&mut buf, &mut rz);
    let after = dhat::HeapStats::get();

    assert_eq!(
        after.total_blocks,
        before.total_blocks,
        "steady-state frame allocated {} block(s)",
        after.total_blocks - before.total_blocks
    );
    assert_eq!(
        after.total_bytes,
        before.total_bytes,
        "steady-state frame allocated {} byte(s)",
        after.total_bytes - before.total_bytes
    );
}
