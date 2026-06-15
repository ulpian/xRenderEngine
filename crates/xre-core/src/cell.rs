//! The universal render target: [`Cell`], [`Attrs`], and [`CellBuffer`].
//!
//! Every producer in the engine — the rasterizer, the cell shaders, the TUI
//! widgets — ultimately writes [`Cell`]s into a [`CellBuffer`], which the
//! presenter diffs and flushes. This is the generalisation of
//! Command_Line_3D's `CHAR_INFO[]` (see `RiftEngine-Plan/02-architecture.md` §3).

use core::fmt;
use core::ops::{BitOr, BitOrAssign};

use crate::color::Color;
use crate::geometry::Rect;
use crate::math::UVec2;

/// Text styling attributes packed into a small bitset.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Attrs(u8);

impl Attrs {
    /// No attributes.
    pub const NONE: Self = Self(0);
    /// Bold / increased intensity.
    pub const BOLD: Self = Self(1 << 0);
    /// Dim / decreased intensity.
    pub const DIM: Self = Self(1 << 1);
    /// Italic.
    pub const ITALIC: Self = Self(1 << 2);
    /// Underline.
    pub const UNDERLINE: Self = Self(1 << 3);

    /// `true` if every flag in `other` is set in `self`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Return `self` with the flags in `other` set.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Return `self` with the flags in `other` cleared.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// `true` if no attribute is set.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The raw bit pattern.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl BitOr for Attrs {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Attrs {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl fmt::Debug for Attrs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names = [None; 4];
        if self.contains(Self::BOLD) {
            names[0] = Some("BOLD");
        }
        if self.contains(Self::DIM) {
            names[1] = Some("DIM");
        }
        if self.contains(Self::ITALIC) {
            names[2] = Some("ITALIC");
        }
        if self.contains(Self::UNDERLINE) {
            names[3] = Some("UNDERLINE");
        }
        let mut first = true;
        write!(f, "Attrs(")?;
        for name in names.into_iter().flatten() {
            if !first {
                write!(f, " | ")?;
            }
            write!(f, "{name}")?;
            first = false;
        }
        if first {
            write!(f, "NONE")?;
        }
        write!(f, ")")
    }
}

/// A color + attribute style, independent of any glyph.
///
/// This is the unit a [`crate::Color`] theme entry resolves to and the value a
/// widget applies to the glyphs it draws. [`Style::apply`] stamps it onto a
/// [`Cell`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Style {
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Styling attributes.
    pub attrs: Attrs,
}

impl Default for Style {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Style {
    /// The default style: default colors, no attributes.
    pub const DEFAULT: Self = Self {
        fg: Color::Default,
        bg: Color::Default,
        attrs: Attrs::NONE,
    };

    /// A style with the given foreground color.
    #[must_use]
    pub const fn fg(fg: Color) -> Self {
        Self {
            fg,
            bg: Color::Default,
            attrs: Attrs::NONE,
        }
    }

    /// Builder: set the background color.
    #[must_use]
    pub const fn with_bg(mut self, bg: Color) -> Self {
        self.bg = bg;
        self
    }

    /// Builder: set the foreground color.
    #[must_use]
    pub const fn with_fg(mut self, fg: Color) -> Self {
        self.fg = fg;
        self
    }

    /// Builder: set the attributes.
    #[must_use]
    pub const fn with_attrs(mut self, attrs: Attrs) -> Self {
        self.attrs = attrs;
        self
    }

    /// Stamp this style's colors and attributes onto `cell`, keeping its glyph.
    #[must_use]
    pub const fn apply(self, mut cell: Cell) -> Cell {
        cell.fg = self.fg;
        cell.bg = self.bg;
        cell.attrs = self.attrs;
        cell
    }

    /// A cell with `glyph` drawn in this style.
    #[must_use]
    pub const fn cell(self, glyph: char) -> Cell {
        Cell {
            glyph,
            fg: self.fg,
            bg: self.bg,
            attrs: self.attrs,
        }
    }
}

/// A single terminal cell: a glyph plus foreground/background color and
/// styling attributes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    /// The character to print.
    pub glyph: char,
    /// Foreground color.
    pub fg: Color,
    /// Background color.
    pub bg: Color,
    /// Styling attributes.
    pub attrs: Attrs,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            glyph: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: Attrs::NONE,
        }
    }
}

impl Cell {
    /// A cell with the given glyph and default colors/attributes.
    #[must_use]
    pub fn new(glyph: char) -> Self {
        Self {
            glyph,
            ..Self::default()
        }
    }

    /// Builder: set the foreground color.
    #[must_use]
    pub const fn fg(mut self, fg: Color) -> Self {
        self.fg = fg;
        self
    }

    /// Builder: set the background color.
    #[must_use]
    pub const fn bg(mut self, bg: Color) -> Self {
        self.bg = bg;
        self
    }

    /// Builder: set the attributes.
    #[must_use]
    pub const fn attrs(mut self, attrs: Attrs) -> Self {
        self.attrs = attrs;
        self
    }
}

/// A row-major grid of [`Cell`]s — the engine's universal render target.
///
/// Coordinates are `(x, y)` with the origin at the top-left. Out-of-bounds
/// accesses are *clipped*, never panicking, in keeping with the no-panic policy.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct CellBuffer {
    size: UVec2,
    cells: Vec<Cell>,
}

impl CellBuffer {
    /// Create a buffer of `size` cells, filled with [`Cell::default`].
    #[must_use]
    pub fn new(size: UVec2) -> Self {
        let len = (size.x as usize) * (size.y as usize);
        Self {
            size,
            cells: vec![Cell::default(); len],
        }
    }

    /// The buffer dimensions in cells.
    #[must_use]
    pub const fn size(&self) -> UVec2 {
        self.size
    }

    /// Width in cells.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.size.x
    }

    /// Height in cells.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.size.y
    }

    /// The full buffer as a [`Rect`] rooted at the origin.
    #[must_use]
    pub const fn area(&self) -> Rect {
        Rect::new(0, 0, self.size.x, self.size.y)
    }

    /// All cells in row-major order.
    #[must_use]
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Flat row-major index of `(x, y)`, or `None` if out of bounds.
    #[must_use]
    const fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.size.x && y < self.size.y {
            Some((y as usize) * (self.size.x as usize) + (x as usize))
        } else {
            None
        }
    }

    /// The cell at `(x, y)`, or `None` if out of bounds.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Option<&Cell> {
        self.index(x, y).map(|i| &self.cells[i])
    }

    /// A mutable reference to the cell at `(x, y)`, or `None` if out of bounds.
    pub fn get_mut(&mut self, x: u32, y: u32) -> Option<&mut Cell> {
        self.index(x, y).map(|i| &mut self.cells[i])
    }

    /// Write `cell` at `(x, y)`. Out-of-bounds writes are silently ignored.
    pub fn set(&mut self, x: u32, y: u32, cell: Cell) {
        if let Some(i) = self.index(x, y) {
            self.cells[i] = cell;
        }
    }

    /// Fill the entire buffer with `cell`.
    pub fn fill(&mut self, cell: Cell) {
        self.cells.fill(cell);
    }

    /// Fill the cells inside `rect` (clipped to the buffer) with `cell`.
    pub fn fill_rect(&mut self, rect: Rect, cell: Cell) {
        let r = rect.intersect(self.area());
        for y in r.top()..r.bottom() {
            for x in r.left()..r.right() {
                self.set(x, y, cell);
            }
        }
    }

    /// Overwrite this buffer's contents with a copy of `src`, reusing the
    /// existing allocation when the sizes already match.
    ///
    /// This is the presenter's hot-path update of its "displayed" buffer: after
    /// a frame is flushed it records exactly what is now on screen without
    /// allocating (the zero-per-frame-allocation invariant of Gate G1).
    pub fn copy_from(&mut self, src: &Self) {
        self.size = src.size;
        self.cells.clone_from(&src.cells);
    }

    /// Resize the buffer to `size`, resetting every cell to the default.
    pub fn resize(&mut self, size: UVec2) {
        self.size = size;
        let len = (size.x as usize) * (size.y as usize);
        self.cells.clear();
        self.cells.resize(len, Cell::default());
    }

    /// Copy `src` into this buffer with its top-left at `dst`, clipping any
    /// part of `src` that would fall outside the destination.
    pub fn blit(&mut self, dst: UVec2, src: &Self) {
        for sy in 0..src.height() {
            for sx in 0..src.width() {
                if let Some(cell) = src.get(sx, sy) {
                    self.set(dst.x + sx, dst.y + sy, *cell);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn px(g: char) -> Cell {
        Cell::new(g)
    }

    #[test]
    fn new_buffer_is_default_filled() {
        let buf = CellBuffer::new(UVec2::new(3, 2));
        assert_eq!(buf.cells().len(), 6);
        assert!(buf.cells().iter().all(|c| *c == Cell::default()));
    }

    #[test]
    fn set_get_roundtrip_and_clipping() {
        let mut buf = CellBuffer::new(UVec2::new(2, 2));
        buf.set(1, 1, px('X'));
        assert_eq!(buf.get(1, 1), Some(&px('X')));
        // Out of bounds: ignored, no panic, returns None.
        buf.set(9, 9, px('!'));
        assert_eq!(buf.get(9, 9), None);
    }

    #[test]
    fn fill_rect_clips_to_buffer() {
        let mut buf = CellBuffer::new(UVec2::new(3, 3));
        buf.fill_rect(Rect::new(1, 1, 10, 10), px('#'));
        assert_eq!(buf.get(0, 0), Some(&Cell::default()));
        assert_eq!(buf.get(2, 2), Some(&px('#')));
    }

    #[test]
    fn blit_clips_overhang() {
        let mut dst = CellBuffer::new(UVec2::new(3, 3));
        let mut src = CellBuffer::new(UVec2::new(2, 2));
        src.fill(px('o'));
        dst.blit(UVec2::new(2, 2), &src); // only (2,2) lands; rest clipped
        assert_eq!(dst.get(2, 2), Some(&px('o')));
        assert_eq!(dst.get(0, 0), Some(&Cell::default()));
    }

    #[test]
    fn copy_from_matches_source_and_reuses_capacity() {
        let mut a = CellBuffer::new(UVec2::new(3, 3));
        let mut b = CellBuffer::new(UVec2::new(3, 3));
        b.fill(px('Z'));
        let cap_before = a.cells().len();
        a.copy_from(&b);
        assert_eq!(a, b);
        assert_eq!(a.cells().len(), cap_before);
        // Copying a differently-sized buffer adopts its size.
        let c = CellBuffer::new(UVec2::new(5, 1));
        a.copy_from(&c);
        assert_eq!(a.size(), UVec2::new(5, 1));
    }

    #[test]
    fn attrs_compose_and_query() {
        let a = Attrs::BOLD | Attrs::UNDERLINE;
        assert!(a.contains(Attrs::BOLD));
        assert!(a.contains(Attrs::UNDERLINE));
        assert!(!a.contains(Attrs::ITALIC));
        assert!(a.without(Attrs::BOLD).contains(Attrs::UNDERLINE));
        assert!(!a.without(Attrs::BOLD).contains(Attrs::BOLD));
    }
}
