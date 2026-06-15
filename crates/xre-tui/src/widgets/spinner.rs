//! [`Spinner`] — an indeterminate throbber for "work in progress" states.
//!
//! Unlike [`crate::Gauge`], a spinner shows no measurable ratio; it is for work
//! whose progress cannot be quantified (e.g. parsing a file of unknown shape).
//! The caller owns the animation counter and advances it each tick (typically
//! off a wall-clock accumulator); `render` only draws the glyph it is told to.

use xre_core::{Color, Rect, Style};

use crate::frame::Frame;
use crate::widget::Widget;

/// Braille throbber frames (the default, smoothest set).
const BRAILLE: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
/// ASCII fallback frames for degraded terminals (`|/-\`).
const ASCII_FRAMES: [char; 4] = ['|', '/', '-', '\\'];

/// An indeterminate spinner: an animated throbber glyph with an optional label.
///
/// The `frame` index is supplied by the caller and taken modulo the frame count,
/// so any counter value is safe (no panic, no wraparound concerns). Advance it
/// on a time accumulator for a steady rate independent of render cadence.
///
/// ```
/// use xre_tui::Spinner;
/// // Drawn through a `Frame`; `tick` is the caller's animation counter.
/// let tick = 7usize;
/// let spinner = Spinner::new(tick).label("Loading…");
/// assert_eq!(Spinner::new(0).glyph(), '⠋');
/// ```
#[derive(Clone, Debug)]
pub struct Spinner {
    frame: usize,
    label: Option<String>,
    style: Style,
    ascii: bool,
}

impl Spinner {
    /// A spinner showing the glyph for `frame` (taken modulo the frame count).
    #[must_use]
    pub const fn new(frame: usize) -> Self {
        Self {
            frame,
            label: None,
            style: Style::fg(Color::Rgb(120, 200, 255)),
            ascii: false,
        }
    }

    /// Builder: text drawn one space after the glyph.
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Builder: set the glyph (and label) style.
    #[must_use]
    pub const fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Builder: use the ASCII frame set (`|/-\`) for degraded terminals.
    #[must_use]
    pub const fn ascii(mut self, ascii: bool) -> Self {
        self.ascii = ascii;
        self
    }

    /// The glyph for the current frame. Pure, so it is unit-testable without a
    /// [`Frame`] and never panics for any `frame` value.
    #[must_use]
    pub const fn glyph(&self) -> char {
        if self.ascii {
            ASCII_FRAMES[self.frame % ASCII_FRAMES.len()]
        } else {
            BRAILLE[self.frame % BRAILLE.len()]
        }
    }
}

impl Widget for Spinner {
    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let (x, y) = (area.left(), area.top());
        frame.set(x, y, self.style.cell(self.glyph()));
        if let Some(label) = &self.label {
            // +2: the glyph plus one space. `print` is clip-safe, so a label that
            // overruns the area is dropped rather than escaping it.
            frame.print(x + 2, y, label, self.style);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::CellBuffer;

    fn row(buf: &CellBuffer, y: u32) -> String {
        (0..buf.width())
            .map(|x| buf.get(x, y).unwrap().glyph)
            .collect()
    }

    #[test]
    fn frame_zero_is_first_glyph() {
        let mut buf = CellBuffer::new(UVec2::new(1, 1));
        {
            let mut f = Frame::root(&mut buf);
            Spinner::new(0).render(Rect::new(0, 0, 1, 1), &mut f);
        }
        assert_eq!(buf.get(0, 0).unwrap().glyph, '⠋');
    }

    #[test]
    fn frame_wraps_modulo() {
        // The braille set has 10 frames, so 10 wraps back to frame 0.
        assert_eq!(Spinner::new(10).glyph(), Spinner::new(0).glyph());
        // A large counter must not panic and must stay in range.
        assert!(BRAILLE.contains(&Spinner::new(123_456_789).glyph()));
    }

    #[test]
    fn ascii_fallback_uses_ascii_set() {
        assert_eq!(Spinner::new(0).ascii(true).glyph(), '|');
        assert_eq!(Spinner::new(1).ascii(true).glyph(), '/');
        assert_eq!(Spinner::new(4).ascii(true).glyph(), '|');
    }

    #[test]
    fn label_drawn_after_glyph_with_gap() {
        let mut buf = CellBuffer::new(UVec2::new(6, 1));
        {
            let mut f = Frame::root(&mut buf);
            Spinner::new(0)
                .label("Hi")
                .render(Rect::new(0, 0, 6, 1), &mut f);
        }
        // glyph at col 0, gap at col 1, "Hi" at cols 2-3.
        assert_eq!(buf.get(0, 0).unwrap().glyph, '⠋');
        assert_eq!(buf.get(2, 0).unwrap().glyph, 'H');
        assert_eq!(buf.get(3, 0).unwrap().glyph, 'i');
    }

    #[test]
    fn empty_area_is_noop() {
        let mut buf = CellBuffer::new(UVec2::new(1, 1));
        let before = row(&buf, 0);
        {
            let mut f = Frame::root(&mut buf);
            Spinner::new(0).render(Rect::new(0, 0, 0, 0), &mut f);
        }
        assert_eq!(row(&buf, 0), before, "empty area must draw nothing");
    }
}
