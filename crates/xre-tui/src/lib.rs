//! `xre-tui` — the panels, grids, and widgets layer for xRenderEngine.
//!
//! Phase 1 delivers the toolkit the 3D layer later draws through:
//!
//! - [`Frame`]: a clipped drawing context over a `CellBuffer`; children can
//!   never paint outside the region they are given.
//! - [`Layout`]/[`GridLayout`]: a constraint-based, integer, remainder-exact
//!   solver (the off-by-one row is the classic TUI bug).
//! - [`Panel`]: bordered, titled, padded containers with ASCII-safe degraded
//!   rendering.
//! - The v1 widget set ([`Text`], [`List`], [`Table`], [`Gauge`], [`Sparkline`],
//!   [`Tabs`], [`Input`], [`Log`], [`Separator`]) plus a [`Theme`] and a
//!   [`FocusManager`].
//!
//! The keystone `Viewport3D` widget (which drives `xre-render`) lands in Phase 2.
#![deny(missing_docs)]

mod focus;
mod frame;
mod layout;
mod mouse;
mod panel;
mod theme;
#[cfg(feature = "render")]
mod viewport;
mod widget;
mod widgets;

#[cfg(feature = "render")]
pub use viewport::Viewport3D;

pub use focus::{FocusId, FocusManager};
pub use frame::{Frame, WrappingMode};
pub use layout::{Constraint, Direction, GridLayout, Layout};
pub use mouse::{viewport_gesture, MouseRouter, ViewportGesture};
pub use panel::{BorderSet, Panel, TitleAlign};
pub use theme::Theme;
pub use widget::Widget;
pub use widgets::{
    Align, Gauge, Input, List, ListState, Log, Scrollbar, ScrollbarOrientation, ScrollbarState,
    Separator, Sparkline, Spinner, Table, Tabs, Text,
};

// Re-export the core drawing vocabulary so downstream code can `use xre_tui::*`.
pub use xre_core::{Cell, CellBuffer, Color, Rect, Style};
