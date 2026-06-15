//! `xre-term` — the terminal backend for xRenderEngine.
//!
//! Phase 0 lands the riskiest parts of the backend up front (Stage 0.3):
//!
//! - [`Capabilities`]: a startup probe of color depth, Unicode level, terminal
//!   size and synchronized-output support, driven by environment heuristics so
//!   the detection logic is unit-testable without a real terminal.
//! - [`TerminalGuard`]: an RAII guard that enters raw mode + the alternate
//!   screen and **guarantees restore on drop *and* on panic** — a terminal left
//!   in raw mode is the cardinal TUI sin (see `RiftEngine-Plan/02-architecture.md` §4).
//!
//! Phase 1 adds the diffed [`Presenter`] (minimal-byte frame updates) and the
//! input event pump ([`Event`]/[`EventQueue`]).
#![deny(missing_docs)]

mod capabilities;
mod error;
mod events;
mod guard;
mod presenter;

pub use capabilities::{Capabilities, UnicodeLevel};
pub use error::{Result, TermError};
pub use events::{Event, EventQueue, Key, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind};
pub use guard::TerminalGuard;
pub use presenter::Presenter;

// Re-exported so downstream crates can name the color depth without depending on
// `xre-core` directly.
pub use xre_core::ColorDepth;
