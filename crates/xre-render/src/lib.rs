//! `xre-render` — the software 3D core for xRenderEngine.
//!
//! The two-stage pipeline that is the engine's reason to exist:
//!
//! 1. **Rasterize at sub-cell resolution** into a [`SampleBuffer`] (SoA luma /
//!    rgb / depth). A [`Camera`]/[`Projection`] transforms vertices, near-plane
//!    [`clip_near`]ping splits straddling triangles, and [`draw_mesh`] fills
//!    depth-tested, perspective-correct, Lambert-lit samples.
//! 2. **Resolve samples to glyphs** with a [`CellShader`]. [`LuminanceRamp`] is
//!    the Phase 2 shader; richer shape-vector and Unicode shaders land in Phase 4.
//!
//! Procedural [`Mesh`]es (cube, plane, uv-sphere, torus) provide content before
//! the OBJ loader exists (Phase 3). Steady-state frames allocate nothing.
#![deny(missing_docs)]
// Determinism gate: result paths must avoid FMA (`mul_add`), whose fuse-or-not
// behaviour varies by platform and would break the bit-identical golden-frame
// guarantee. We therefore keep plain `a*b + c` and silence the nursery lint that
// would push us toward `mul_add` (`RiftEngine-Plan/07-phase-2-renderer-core.md`).
#![allow(clippy::suboptimal_flops)]

mod camera;
mod clip;
mod density;
mod light;
mod material;
mod mesh;
mod raster;
mod sample;
mod settings;
mod shader;
mod shape;

pub use camera::{Camera, Projection};
pub use clip::{clip_near, ClipVertex};
pub use density::{BlockShades, Braille, HalfBlock};
pub use light::{luminance, Light, LightRig};
pub use material::{Material, TexHandle};
pub use mesh::{Aabb, Mesh};
pub use raster::{draw_mesh, draw_mesh_textured, Cull, Rasterizer, ShadeMode, TextureSampler};
pub use sample::{Sample, SampleBuffer};
pub use settings::RenderSettings;
pub use shader::{
    builtin_cell_shaders, resolve_cells, CellShader, LumaBias, LuminanceRamp, DENSITY_ORDER,
};
pub use shape::{ShapeGlyph, ShapeTable, ShapeVec, ShapeVector};
