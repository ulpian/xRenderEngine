//! `xre-cello` — scenes, assets, and animation for xRenderEngine.
//!
//! Phase 3 builds real content on top of `xre-render`:
//!
//! - [`parse_obj`]/[`load_obj_file`]: a from-scratch, fuzz-hardened OBJ/MTL
//!   loader with a vendored ear-clipping [`triangulate`]r for concave faces.
//! - [`Scene`]: an arena scene graph with dirty-flag world-matrix propagation and
//!   flat draw-list extraction (queries hoisted out of pixel loops).
//! - Camera controllers ([`OrbitController`], [`FpsController`]) fed by actions.
//! - [`Texture`], [`load_image_file`], and the PNG/JPEG/PGM/PPM/BMP readers for
//!   material maps and the image viewer.
#![deny(missing_docs)]
// Determinism gate: keep plain `a*b + c` over FMA (`mul_add`); see xre-render.
#![allow(clippy::suboptimal_flops)]

mod controllers;
mod obj;
mod scene;
mod texture;
mod triangulate;

pub use controllers::{FpsController, LookAtConstraint, OrbitController};
pub use obj::{load_obj_file, parse_mtl, parse_obj, ObjError, ObjModel, ObjObject};
pub use scene::{DrawItem, NodeId, NodeKind, Scene};
pub use texture::{load_image_file, Texture, TextureError};
pub use triangulate::triangulate;

// Re-export the renderer types scene content is built from.
pub use xre_render::{Camera, Light, LightRig, Material, Mesh, Projection};
