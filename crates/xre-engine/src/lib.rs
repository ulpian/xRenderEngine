//! `xre-engine` — the application layer for xRenderEngine (Phase 5).
//!
//! Turns the renderer + TUI into a *game* engine:
//!
//! - [`FixedTimestep`]/[`FramePacer`]/[`Time`]: a deterministic fixed-step loop
//!   with interpolated render and precise pacing.
//! - [`ecs`]: a thin [`hecs`] facade with provided components and a system
//!   [`Schedule`].
//! - [`anim`]: keyframe [`Track`]s/[`Clip`]s/[`Animator`]s and procedural
//!   [`Tween`]s.
//! - [`InputMap`]: action bindings with contexts and edge/level query.
//! - [`collide`]: tunnel-proof swept-AABB resolution and a uniform-grid
//!   broadphase.
//! - `raycaster` (feature `grid-raycaster`): a tile-map FPS backend rendering
//!   into the standard `SampleBuffer`.
//! - [`Game`] + [`run`]: the fixed-timestep entry point that drives the diffed
//!   presenter and input pump.
#![deny(missing_docs)]
// Determinism gate: plain `a*b + c` over FMA (`mul_add`); see xre-render.
#![allow(clippy::suboptimal_flops)]

pub mod anim;
pub mod collide;
pub mod ecs;
mod input_map;
mod latch;
#[cfg(feature = "grid-raycaster")]
pub mod raycaster;
mod time;

pub use anim::{Animator, Clip, Easing, PlayMode, Track, Tween};
pub use ecs::{Schedule, World};
pub use input_map::{Binding, InputMap};
pub use latch::LatchAxis;
pub use time::{FixedTimestep, FramePacer, Time};

use std::time::Instant;

use xre_core::CellBuffer;
use xre_term::{Capabilities, Event, EventQueue, Presenter, TerminalGuard};

/// Errors raised by the engine [`run`] loop.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// A terminal backend operation failed.
    #[error(transparent)]
    Term(#[from] xre_term::TermError),
}

/// Configuration for [`run`].
#[derive(Clone, Copy, Debug)]
pub struct EngineConfig {
    /// Fixed update rate (Hz).
    pub update_hz: f32,
    /// Render frame cap (FPS).
    pub target_fps: f32,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            update_hz: 60.0,
            target_fps: 60.0,
        }
    }
}

/// A game driven by [`run`]: fixed-step `update`, interpolated `render`.
pub trait Game {
    /// Advance the simulation by one fixed `time.dt`, consuming this step's
    /// input events.
    fn update(&mut self, time: &Time, events: &[Event]);

    /// Draw the current state into `buf` (already sized to the terminal).
    fn render(&mut self, buf: &mut CellBuffer, time: &Time);

    /// Return `false` to exit the loop.
    fn running(&self) -> bool;
}

/// Run `game` to completion on the real terminal: raw mode, a diffed presenter,
/// a fixed-timestep update with a frame-coherent input queue, and frame pacing.
///
/// # Errors
/// Returns [`EngineError`] if terminal I/O fails. The [`TerminalGuard`] restores
/// the terminal on return *and* on panic.
pub fn run(mut game: impl Game, config: EngineConfig) -> Result<(), EngineError> {
    let _guard = TerminalGuard::enter()?;
    let caps = Capabilities::probe();
    let mut presenter = Presenter::stdout(&caps);
    let mut buf = CellBuffer::new(caps.size);
    let mut events = EventQueue::new();
    let mut timestep = FixedTimestep::new(config.update_hz);
    let pacer = FramePacer::new(config.target_fps);
    let mut last = Instant::now();

    while game.running() {
        let frame_start = Instant::now();
        events.pump(pacer.remaining(frame_start))?;
        let mut drained: Vec<Event> = events.drain().collect();
        for ev in &drained {
            if let Event::Resize(size) = ev {
                buf.resize(*size);
                presenter.resize(*size);
            }
        }

        let frame_dt = last.elapsed().as_secs_f32();
        last = Instant::now();
        let steps = timestep.advance(frame_dt);
        for _ in 0..steps {
            let time = timestep.time();
            game.update(&time, &drained);
            // Events belong to the first update of the frame only (frame-coherent).
            drained.clear();
        }

        if buf.size() != presenter.size() {
            buf.resize(presenter.size());
        }
        game.render(&mut buf, &timestep.time());
        presenter.present(&buf)?;
        pacer.pace(frame_start);
    }
    Ok(())
}

// Re-export the content vocabulary games build on.
pub use xre_cello::{FpsController, OrbitController, Scene};
pub use xre_render::{
    draw_mesh, Camera, CellShader, Cull, Light, LightRig, LuminanceRamp, Material, Mesh,
    Projection, SampleBuffer, ShadeMode,
};
