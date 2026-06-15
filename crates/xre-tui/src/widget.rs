//! The [`Widget`] trait: immediate-mode drawing over retained layout.
//!
//! A widget renders itself into a [`Frame`] at a given [`Rect`]. State that
//! must persist between frames (a list's selection, an input's cursor) lives in
//! the widget value itself; rendering borrows it immutably. Interactive widgets
//! additionally expose `handle_event` inherent methods (not part of this trait,
//! which stays object-safe and trivial to implement).

use xre_core::Rect;

use crate::frame::Frame;

/// Anything that can draw itself into a region of a [`Frame`].
pub trait Widget {
    /// Draw into `area`. Implementations must not assume `area` is non-empty and
    /// must rely on the frame's clipping rather than bounds-checking themselves.
    fn render(&self, area: Rect, frame: &mut Frame);
}

impl<T: Widget + ?Sized> Widget for &T {
    fn render(&self, area: Rect, frame: &mut Frame) {
        (**self).render(area, frame);
    }
}
