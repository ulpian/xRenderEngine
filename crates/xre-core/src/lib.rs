//! `xre-core` — foundational types shared across the xRenderEngine workspace.
//!
//! This crate implements Stage 0.2 of the roadmap (see
//! `RiftEngine-Plan/02-architecture.md` §3 and `05-phase-0-foundations.md` §0.2):
//!
//! - [`math`]: a curated re-export of [`glam`] plus a TRS [`Transform`].
//! - [`Color`] and [`ColorDepth`]: a terminal color with a capability-aware
//!   [`Color::resolve`] downgrade chain (`Rgb` → `256` → `16` → mono).
//! - [`Cell`], [`Attrs`], and [`CellBuffer`]: the universal render target —
//!   everything in the engine ultimately writes [`Cell`]s into a [`CellBuffer`].
//! - [`Rect`]: integer rectangle algebra (intersect, union, clamp, split).
//! - [`CoreError`]: the crate's error type ([`thiserror`]-derived, no panics).
#![deny(missing_docs)]

pub mod math;
pub mod oklab;

mod cell;
mod color;
mod error;
mod geometry;

pub use cell::{Attrs, Cell, CellBuffer, Style};
pub use color::{Color, ColorDepth};
pub use error::{CoreError, Result};
pub use geometry::Rect;
pub use oklab::Oklab;

pub use math::Transform;
