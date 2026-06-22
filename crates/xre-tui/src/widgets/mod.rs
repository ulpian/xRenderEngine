//! The v1 widget set (Stage 1.4).
//!
//! Every widget implements [`crate::Widget`] — immediate-mode draw over a
//! [`crate::Frame`]. Stateful widgets keep their state in the value (a
//! [`ListState`] for [`List`], the buffer/cursor for [`Input`]) and mutate it
//! via inherent `handle_*` methods; rendering only borrows it.

mod gauge;
mod input;
mod list;
mod log;
mod scrollbar;
mod spinner;
mod table;
mod tabs;
mod text;

pub use gauge::{Gauge, Sparkline};
pub use input::Input;
pub use list::{List, ListState};
pub use log::Log;
pub use scrollbar::{Scrollbar, ScrollbarOrientation, ScrollbarState};
pub use spinner::Spinner;
pub use table::Table;
pub use tabs::Tabs;
pub use text::{Separator, Text};

/// Horizontal alignment shared by several widgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Align {
    /// Align to the left edge.
    #[default]
    Left,
    /// Center within the available width.
    Center,
    /// Align to the right edge.
    Right,
}

impl Align {
    /// The starting column offset for content of `content_w` cells within
    /// `avail_w` cells.
    #[must_use]
    pub(crate) const fn offset(self, content_w: u32, avail_w: u32) -> u32 {
        if content_w >= avail_w {
            return 0;
        }
        match self {
            Self::Left => 0,
            Self::Center => (avail_w - content_w) / 2,
            Self::Right => avail_w - content_w,
        }
    }
}
