//! `xre` — the user-facing facade for xRenderEngine.
//!
//! This crate re-exports the workspace's sub-crates under stable names and
//! exposes the curated [`prelude`]. The `xre::run` entry point lands with the
//! engine layer in Phase 5; see `RiftEngine-Plan/02-architecture.md` §9 for the
//! intended public API.
#![deny(missing_docs)]

pub use xre_cello as scene; // the scene/assets crate (formerly `xre-scene`)
pub use xre_core as core;
pub use xre_engine as engine;
pub use xre_render as render;
pub use xre_term as term;
pub use xre_tui as tui;

/// Curated re-exports for ergonomic `use xre::prelude::*;` imports.
///
/// Grows as the engine's public API stabilises (see Phase 6.1). For now it
/// surfaces the foundational `xre-core` types and the terminal entry points
/// that exist after Phase 0.
pub mod prelude {
    pub use crate::core::math::{Mat4, Quat, UVec2, Vec2, Vec3, Vec4};
    pub use crate::core::{Attrs, Cell, CellBuffer, Color, ColorDepth, Rect, Style, Transform};
    pub use crate::term::{
        Capabilities, Event, EventQueue, Key, KeyCode, Modifiers, Presenter, TerminalGuard,
        UnicodeLevel,
    };
    pub use crate::tui::{
        BorderSet, Constraint, Frame, Gauge, GridLayout, Layout, List, ListState, Log, Panel,
        Spinner, Tabs, Text, Theme, Viewport3D, Widget,
    };

    pub use crate::engine::{
        run, EngineConfig, FixedTimestep, FramePacer, Game, InputMap, Schedule, Time, World,
    };
    pub use crate::render::{
        builtin_cell_shaders, draw_mesh, draw_mesh_textured, resolve_cells, BlockShades, Braille,
        Camera, CellShader, Cull, HalfBlock, Light, LightRig, LuminanceRamp, Material, Mesh,
        Projection, Rasterizer, RenderSettings, Sample, SampleBuffer, ShadeMode, ShapeTable,
        ShapeVector, TextureSampler,
    };
    pub use crate::scene::{
        load_image_file, load_obj_file, parse_obj, FpsController, ObjModel, OrbitController, Scene,
        Texture,
    };
}
