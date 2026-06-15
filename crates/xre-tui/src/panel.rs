//! [`Panel`]: a bordered, titled, padded container that clips its children.
//!
//! A panel draws a border (in one of several [`BorderSet`]s — ASCII is always
//! available for degraded mode), an optional aligned title, and exposes the
//! [`Panel::inner`] rect children draw into. Nesting clips via [`Rect`]
//! intersection in the [`Frame`], so children can never escape their panel
//! (`RiftEngine-Plan/06-phase-1-tui-core.md` §1.3).

use xre_core::{Cell, Rect, Style};

use crate::frame::Frame;

/// The six glyphs that draw a box border.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BorderSet {
    /// Top-left corner.
    pub top_left: char,
    /// Top-right corner.
    pub top_right: char,
    /// Bottom-left corner.
    pub bottom_left: char,
    /// Bottom-right corner.
    pub bottom_right: char,
    /// Horizontal edge.
    pub horizontal: char,
    /// Vertical edge.
    pub vertical: char,
}

impl BorderSet {
    /// Pure ASCII (`+-|`) — guaranteed to render in any terminal/locale.
    pub const ASCII: Self = Self {
        top_left: '+',
        top_right: '+',
        bottom_left: '+',
        bottom_right: '+',
        horizontal: '-',
        vertical: '|',
    };
    /// Light box-drawing (`┌─┐│`).
    pub const LIGHT: Self = Self {
        top_left: '┌',
        top_right: '┐',
        bottom_left: '└',
        bottom_right: '┘',
        horizontal: '─',
        vertical: '│',
    };
    /// Rounded corners (`╭─╮│`).
    pub const ROUNDED: Self = Self {
        top_left: '╭',
        top_right: '╮',
        bottom_left: '╰',
        bottom_right: '╯',
        horizontal: '─',
        vertical: '│',
    };
    /// Double-line (`╔═╗║`).
    pub const DOUBLE: Self = Self {
        top_left: '╔',
        top_right: '╗',
        bottom_left: '╚',
        bottom_right: '╝',
        horizontal: '═',
        vertical: '║',
    };
    /// Heavy (`┏━┓┃`).
    pub const HEAVY: Self = Self {
        top_left: '┏',
        top_right: '┓',
        bottom_left: '┗',
        bottom_right: '┛',
        horizontal: '━',
        vertical: '┃',
    };
}

/// Where a panel title sits along the top edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TitleAlign {
    /// Left-aligned (after the corner).
    #[default]
    Left,
    /// Centered.
    Center,
    /// Right-aligned (before the corner).
    Right,
}

/// A bordered container.
#[derive(Clone, Debug)]
pub struct Panel {
    title: Option<String>,
    title_align: TitleAlign,
    border: Option<BorderSet>,
    border_style: Style,
    title_style: Style,
    fill: Option<Cell>,
    padding: u32,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            title: None,
            title_align: TitleAlign::Left,
            border: Some(BorderSet::LIGHT),
            border_style: Style::DEFAULT,
            title_style: Style::DEFAULT,
            fill: None,
            padding: 0,
        }
    }
}

impl Panel {
    /// A panel with a light border and no title.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Builder: set the title.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Builder: set the title alignment.
    #[must_use]
    pub const fn title_align(mut self, align: TitleAlign) -> Self {
        self.title_align = align;
        self
    }

    /// Builder: set the border glyph set (or `None` for a borderless panel).
    #[must_use]
    pub const fn border(mut self, border: Option<BorderSet>) -> Self {
        self.border = border;
        self
    }

    /// Builder: set the border style (color/attributes).
    #[must_use]
    pub const fn border_style(mut self, style: Style) -> Self {
        self.border_style = style;
        self
    }

    /// Builder: set the title style.
    #[must_use]
    pub const fn title_style(mut self, style: Style) -> Self {
        self.title_style = style;
        self
    }

    /// Builder: fill the interior with `cell` before children draw.
    #[must_use]
    pub const fn fill(mut self, cell: Cell) -> Self {
        self.fill = Some(cell);
        self
    }

    /// Builder: set interior padding (cells) on all sides.
    #[must_use]
    pub const fn padding(mut self, padding: u32) -> Self {
        self.padding = padding;
        self
    }

    /// The interior rect children should draw into, given the panel's `area`.
    #[must_use]
    pub fn inner(&self, area: Rect) -> Rect {
        let border = u32::from(self.border.is_some());
        area.inset(border).inset(self.padding)
    }

    /// Draw the panel border, title and fill into `frame` at `area`. Returns the
    /// inner rect (also obtainable via [`Panel::inner`]).
    pub fn render(&self, area: Rect, frame: &mut Frame) -> Rect {
        if area.is_empty() {
            return self.inner(area);
        }
        let mut f = frame.region(area);
        if let Some(bs) = self.border {
            self.draw_border(&mut f, area, bs);
        }
        let inner = self.inner(area);
        if let Some(cell) = self.fill {
            f.fill_rect(inner, cell);
        }
        inner
    }

    fn draw_border(&self, f: &mut Frame, area: Rect, bs: BorderSet) {
        let (lx, ty) = (area.left(), area.top());
        let (rx, by) = (area.right() - 1, area.bottom() - 1);
        let cell = |g| self.border_style.cell(g);
        // Edges.
        for x in lx..=rx {
            f.set(x, ty, cell(bs.horizontal));
            f.set(x, by, cell(bs.horizontal));
        }
        for y in ty..=by {
            f.set(lx, y, cell(bs.vertical));
            f.set(rx, y, cell(bs.vertical));
        }
        // Corners.
        f.set(lx, ty, cell(bs.top_left));
        f.set(rx, ty, cell(bs.top_right));
        f.set(lx, by, cell(bs.bottom_left));
        f.set(rx, by, cell(bs.bottom_right));
        // Title.
        if let Some(title) = &self.title {
            self.draw_title(f, area, title);
        }
    }

    fn draw_title(&self, f: &mut Frame, area: Rect, title: &str) {
        // The title sits on the top edge, inset one cell from each corner.
        let avail = area.width().saturating_sub(2);
        if avail == 0 {
            return;
        }
        let chars: Vec<char> = title.chars().take(avail as usize).collect();
        let len = chars.len() as u32;
        let start = area.left()
            + 1
            + match self.title_align {
                TitleAlign::Left => 0,
                TitleAlign::Center => (avail - len) / 2,
                TitleAlign::Right => avail - len,
            };
        for (i, ch) in chars.into_iter().enumerate() {
            f.set(start + i as u32, area.top(), self.title_style.cell(ch));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::CellBuffer;

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
    fn inner_accounts_for_border_and_padding() {
        let p = Panel::new().padding(1);
        // border (1) + padding (1) = 2 on each side.
        assert_eq!(p.inner(Rect::new(0, 0, 10, 10)), Rect::new(2, 2, 6, 6));
    }

    #[test]
    fn borderless_inner_is_area() {
        let p = Panel::new().border(None);
        assert_eq!(p.inner(Rect::new(0, 0, 10, 10)), Rect::new(0, 0, 10, 10));
    }

    #[test]
    fn renders_ascii_box_with_title() {
        let mut buf = CellBuffer::new(UVec2::new(8, 4));
        {
            let mut f = Frame::root(&mut buf);
            Panel::new()
                .border(Some(BorderSet::ASCII))
                .title("Hi")
                .render(Rect::new(0, 0, 8, 4), &mut f);
        }
        assert_eq!(
            render_to_string(&buf),
            "+Hi----+\n|      |\n|      |\n+------+\n"
        );
    }

    #[test]
    fn title_is_clipped_to_width() {
        let mut buf = CellBuffer::new(UVec2::new(5, 3));
        {
            let mut f = Frame::root(&mut buf);
            Panel::new()
                .border(Some(BorderSet::ASCII))
                .title("LongTitle")
                .render(Rect::new(0, 0, 5, 3), &mut f);
        }
        // Width 5 → 3 inner edge cells for the title.
        let top: String = (0..5).map(|x| buf.get(x, 0).unwrap().glyph).collect();
        assert_eq!(top, "+Lon+");
    }
}
