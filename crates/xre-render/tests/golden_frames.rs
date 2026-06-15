//! Golden-frame tests (Stage 2.4 / Gate G2): lit primitives rendered to text and
//! snapshotted. Frames are text, so they diff perfectly and must be byte-
//! identical across CI OSes (the determinism gate).

use insta::assert_snapshot;
use xre_core::math::{UVec2, Vec3};
use xre_core::Transform;
use xre_render::{
    draw_mesh, Camera, CellShader, Cull, LightRig, LuminanceRamp, Material, Mesh, Projection,
    SampleBuffer, ShadeMode,
};

/// Render one mesh under a fixed light rig and resolve to an ASCII frame.
fn render_to_text(mesh: &Mesh, shade_mode: ShadeMode) -> String {
    let (cols, rows) = (40u32, 20u32);
    let mut buf = SampleBuffer::new(UVec2::new(cols, rows), 2, 4);
    buf.clear([0, 0, 0]);
    let cam = Camera::look_at(Vec3::new(2.5, 2.0, 3.5), Vec3::ZERO);
    let vp = cam.view_projection(cols, rows, Projection::DEFAULT_CELL_ASPECT);
    let rig = LightRig::default();
    draw_mesh(
        &mut buf,
        mesh,
        Transform::IDENTITY.to_mat4(),
        vp,
        &rig,
        &Material::default(),
        shade_mode,
        Cull::Back,
    );
    let shader = LuminanceRamp::default();
    let mut out = String::new();
    for cy in 0..rows {
        for cx in 0..cols {
            out.push(shader.shade(&buf, cx, cy).map_or(' ', |c| c.glyph));
        }
        out.push('\n');
    }
    out
}

#[test]
fn lit_cube_per_sample() {
    assert_snapshot!(render_to_text(&Mesh::cube(), ShadeMode::PerSample));
}

#[test]
fn lit_sphere_per_sample() {
    assert_snapshot!(render_to_text(
        &Mesh::uv_sphere(1.3, 24, 32),
        ShadeMode::PerSample
    ));
}

#[test]
fn lit_torus_flat() {
    assert_snapshot!(render_to_text(
        &Mesh::torus(1.2, 0.45, 32, 16),
        ShadeMode::Flat
    ));
}
