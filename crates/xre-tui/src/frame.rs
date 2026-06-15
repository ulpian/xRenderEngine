//! The [`Frame`]: a clipped drawing context over a [`CellBuffer`].
//!
//! Widgets never touch the [`CellBuffer`] directly; they draw through a
//! [`Frame`], which carries a clip [`Rect`]. Every write is intersected with the
//! clip, so a child can *never* paint outside the region its parent gave it
//! (the `Frame::with_clip` guarantee from `RiftEngine-Plan/06-phase-1-tui-core.md`
//! §1.3). This is also the seam the `Viewport3D` widget plugs the 3D renderer
//! into — exactly the `Canvas`/`CanDraw` junction admired in gemini-engine
//! ([14](14-gemini-engine-analysis.md)), but over a richer cell model.

use unicode_width::UnicodeWidthChar;
use xre_core::math::UVec2;
use xre_core::{Cell, CellBuffer, Rect, Style};

/// How a draw operation behaves when it runs past the clip's right edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum WrappingMode {
    /// Drop anything past the edge (the default; gemini-engine's `Ignore`).
    #[default]
    Ignore,
    /// Continue on the next row within the clip (handy for 2D sprites/text).
    Wrap,
    /// Stop the whole operation at the edge.
    Clamp,
}

/// A clipped view onto a [`CellBuffer`] for drawing.
///
/// Construct the root frame with [`Frame::root`]; obtain a clipped sub-frame
/// with [`Frame::region`] (or run a closure under a tighter clip with
/// [`Frame::with_clip`]). Coordinates passed to draw methods are *absolute*
/// buffer coordinates; the clip decides what actually lands.
pub struct Frame<'a> {
    buf: &'a mut CellBuffer,
    clip: Rect,
}

impl<'a> Frame<'a> {
    /// A root frame whose clip is the whole buffer.
    pub const fn root(buf: &'a mut CellBuffer) -> Self {
        let clip = buf.area();
        Self { buf, clip }
    }

    /// A frame over `buf` clipped to `clip` (intersected with the buffer).
    pub fn new(buf: &'a mut CellBuffer, clip: Rect) -> Self {
        let clip = clip.intersect(buf.area());
        Self { buf, clip }
    }

    /// The current clip region (absolute buffer coordinates).
    #[must_use]
    pub const fn area(&self) -> Rect {
        self.clip
    }

    /// Borrow a sub-frame clipped to `rect ∩ self.area()`.
    pub fn region(&mut self, rect: Rect) -> Frame<'_> {
        Frame {
            clip: rect.intersect(self.clip),
            buf: self.buf,
        }
    }

    /// Run `f` with the clip tightened to `rect`, then restore.
    pub fn with_clip(&mut self, rect: Rect, f: impl FnOnce(&mut Frame<'_>)) {
        let mut sub = self.region(rect);
        f(&mut sub);
    }

    /// Set the cell at `(x, y)` if it lies inside the clip.
    pub fn set(&mut self, x: u32, y: u32, cell: Cell) {
        if self.clip.contains(UVec2::new(x, y)) {
            self.buf.set(x, y, cell);
        }
    }

    /// Fill the clip with `cell`.
    pub fn fill(&mut self, cell: Cell) {
        self.buf.fill_rect(self.clip, cell);
    }

    /// Fill `rect` (clipped) with `cell`.
    pub fn fill_rect(&mut self, rect: Rect, cell: Cell) {
        self.buf.fill_rect(rect.intersect(self.clip), cell);
    }

    /// Replace only the glyph at `(x, y)` (clipped), keeping the cell's existing
    /// colors and attributes — used to overlay labels on top of fills.
    pub fn overlay_glyph(&mut self, x: u32, y: u32, glyph: char) {
        if self.clip.contains(UVec2::new(x, y)) {
            if let Some(c) = self.buf.get_mut(x, y) {
                c.glyph = glyph;
            }
        }
    }

    /// Apply `style`'s colors/attributes to every cell of `rect` (clipped),
    /// leaving glyphs untouched. Used for selection highlights and backgrounds.
    pub fn style_rect(&mut self, rect: Rect, style: Style) {
        let r = rect.intersect(self.clip);
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                if let Some(c) = self.buf.get_mut(x, y) {
                    *c = style.apply(*c);
                }
            }
        }
    }

    /// Draw `text` starting at `(x, y)` in `style`, honouring `wrap`.
    ///
    /// Returns the `(x, y)` the cursor advanced to. Wide glyphs occupy two
    /// columns; a wide glyph that would straddle the right edge is dropped.
    pub fn put_str(
        &mut self,
        x: u32,
        y: u32,
        text: &str,
        style: Style,
        wrap: WrappingMode,
    ) -> (u32, u32) {
        let left = self.clip.left();
        let right = self.clip.right();
        let bottom = self.clip.bottom();
        let mut cx = x;
        let mut cy = y;
        for ch in text.chars() {
            if ch == '\n' {
                cx = x;
                cy += 1;
                continue;
            }
            let w = UnicodeWidthChar::width(ch).unwrap_or(1).max(1) as u32;
            if cx + w > right {
                match wrap {
                    WrappingMode::Ignore => {
                        cx += w;
                        continue;
                    }
                    WrappingMode::Clamp => break,
                    WrappingMode::Wrap => {
                        cx = self.clip.left().max(left);
                        cy += 1;
                    }
                }
            }
            if cy >= bottom {
                break;
            }
            self.set(cx, cy, style.cell(ch));
            cx += w;
        }
        (cx, cy)
    }

    /// Convenience: draw a single-line string clipped (no wrap).
    pub fn print(&mut self, x: u32, y: u32, text: &str, style: Style) {
        self.put_str(x, y, text, style, WrappingMode::Ignore);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::Color;

    fn render_to_string(buf: &CellBuffer) -> String {
        let mut s = String::new();
        for y in 0..buf.height() {
            for x in 0..buf.width() {
                s.push(buf.get(x, y).map_or(' ', |c| c.glyph));
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn put_str_clips_to_region() {
        let mut buf = CellBuffer::new(UVec2::new(6, 2));
        {
            let mut f = Frame::root(&mut buf);
            let mut sub = f.region(Rect::new(1, 0, 3, 2));
            sub.print(0, 0, "ABCDEF", Style::DEFAULT); // x starts left of clip
        }
        // Only columns 1..4 are writable; 'B','C','D' land there.
        assert_eq!(render_to_string(&buf), " BCD  \n      \n");
    }

    #[test]
    fn child_cannot_paint_outside_parent() {
        let mut buf = CellBuffer::new(UVec2::new(5, 3));
        {
            let mut f = Frame::root(&mut buf);
            f.with_clip(Rect::new(1, 1, 2, 1), |inner| {
                inner.fill(Cell::new('#'));
            });
        }
        assert_eq!(render_to_string(&buf), "     \n ##  \n     \n");
    }

    #[test]
    fn wrap_mode_moves_to_next_row() {
        let mut buf = CellBuffer::new(UVec2::new(3, 3));
        {
            let mut f = Frame::root(&mut buf);
            f.put_str(0, 0, "ABCDEF", Style::DEFAULT, WrappingMode::Wrap);
        }
        assert_eq!(render_to_string(&buf), "ABC\nDEF\n   \n");
    }

    #[test]
    fn style_rect_keeps_glyphs() {
        let mut buf = CellBuffer::new(UVec2::new(2, 1));
        buf.set(0, 0, Cell::new('x'));
        buf.set(1, 0, Cell::new('y'));
        {
            let mut f = Frame::root(&mut buf);
            f.style_rect(Rect::new(0, 0, 2, 1), Style::fg(Color::Rgb(1, 2, 3)));
        }
        assert_eq!(buf.get(0, 0).unwrap().glyph, 'x');
        assert_eq!(buf.get(0, 0).unwrap().fg, Color::Rgb(1, 2, 3));
    }
}
