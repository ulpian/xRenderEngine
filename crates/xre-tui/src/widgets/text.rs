//! [`Text`] (multiline, aligned, wrapped) and [`Separator`].

use unicode_width::UnicodeWidthStr;
use xre_core::{Rect, Style};

use crate::frame::Frame;
use crate::widget::Widget;
use crate::widgets::Align;

/// A block of styled text with optional wrapping and per-line alignment.
///
/// ```
/// use xre_tui::{Text, Frame, Align};
/// use xre_core::{CellBuffer, Rect, math::UVec2};
/// let mut buf = CellBuffer::new(UVec2::new(10, 1));
/// let mut frame = Frame::root(&mut buf);
/// Text::raw("hello").align(Align::Right).render_into(Rect::new(0, 0, 10, 1), &mut frame);
/// ```
#[derive(Clone, Debug)]
pub struct Text {
    content: String,
    style: Style,
    align: Align,
    wrap: bool,
}

impl Text {
    /// Plain text in the default style, left-aligned, no wrap.
    #[must_use]
    pub fn raw(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            style: Style::DEFAULT,
            align: Align::Left,
            wrap: false,
        }
    }

    /// Text in a given style.
    #[must_use]
    pub fn styled(content: impl Into<String>, style: Style) -> Self {
        Self {
            style,
            ..Self::raw(content)
        }
    }

    /// Builder: set the alignment.
    #[must_use]
    pub const fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Builder: set the style.
    #[must_use]
    pub const fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Builder: enable word/char wrapping to the area width.
    #[must_use]
    pub const fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    /// Render at `area` (inherent alias for the [`Widget`] impl, ergonomic in
    /// builder chains).
    pub fn render_into(&self, area: Rect, frame: &mut Frame) {
        self.render(area, frame);
    }

    /// Break `content` into the lines to draw, applying wrapping if enabled.
    fn lines(&self, width: u32) -> Vec<String> {
        let mut out = Vec::new();
        for raw_line in self.content.split('\n') {
            if !self.wrap || raw_line.width() as u32 <= width {
                out.push(raw_line.to_string());
                continue;
            }
            out.extend(wrap_line(raw_line, width));
        }
        out
    }
}

/// Greedily wrap one logical line to `width` cells, breaking on spaces where
/// possible and hard-breaking over-long words.
fn wrap_line(line: &str, width: u32) -> Vec<String> {
    let width = width.max(1) as usize;
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_w = 0usize;
    for word in line.split(' ') {
        let ww = word.width();
        if ww > width {
            // Hard-break the long word by characters.
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_w = 0;
            }
            for ch in word.chars() {
                let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                if current_w + cw > width {
                    lines.push(std::mem::take(&mut current));
                    current_w = 0;
                }
                current.push(ch);
                current_w += cw;
            }
            continue;
        }
        let extra = if current.is_empty() { ww } else { ww + 1 };
        if current_w + extra > width {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            current_w = ww;
        } else {
            if !current.is_empty() {
                current.push(' ');
                current_w += 1;
            }
            current.push_str(word);
            current_w += ww;
        }
    }
    lines.push(current);
    lines
}

impl Widget for Text {
    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let mut f = frame.region(area);
        for (i, line) in self.lines(area.width()).into_iter().enumerate() {
            let y = area.top() + i as u32;
            if y >= area.bottom() {
                break;
            }
            let lw = line.width() as u32;
            let x = area.left() + self.align.offset(lw, area.width());
            f.print(x, y, &line, self.style);
        }
    }
}

/// A horizontal or vertical rule one cell thick.
#[derive(Clone, Copy, Debug)]
pub struct Separator {
    glyph: char,
    style: Style,
    vertical: bool,
}

impl Default for Separator {
    fn default() -> Self {
        Self {
            glyph: '─',
            style: Style::DEFAULT,
            vertical: false,
        }
    }
}

impl Separator {
    /// A horizontal separator (`─`).
    #[must_use]
    pub fn horizontal() -> Self {
        Self::default()
    }

    /// A vertical separator (`│`).
    #[must_use]
    pub fn vertical() -> Self {
        Self {
            glyph: '│',
            vertical: true,
            ..Self::default()
        }
    }

    /// Builder: use an ASCII glyph (`-` / `|`) for degraded mode.
    #[must_use]
    pub const fn ascii(mut self) -> Self {
        self.glyph = if self.vertical { '|' } else { '-' };
        self
    }

    /// Builder: set the glyph.
    #[must_use]
    pub const fn glyph(mut self, glyph: char) -> Self {
        self.glyph = glyph;
        self
    }

    /// Builder: set the style.
    #[must_use]
    pub const fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }
}

impl Widget for Separator {
    fn render(&self, area: Rect, frame: &mut Frame) {
        let cell = self.style.cell(self.glyph);
        if self.vertical {
            let x = area.left();
            for y in area.top()..area.bottom() {
                frame.set(x, y, cell);
            }
        } else {
            let y = area.top();
            for x in area.left()..area.right() {
                frame.set(x, y, cell);
            }
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
    fn right_align() {
        let mut buf = CellBuffer::new(UVec2::new(8, 1));
        {
            let mut f = Frame::root(&mut buf);
            Text::raw("hi")
                .align(Align::Right)
                .render(Rect::new(0, 0, 8, 1), &mut f);
        }
        assert_eq!(row(&buf, 0), "      hi");
    }

    #[test]
    fn wrap_splits_lines() {
        let mut buf = CellBuffer::new(UVec2::new(5, 3));
        {
            let mut f = Frame::root(&mut buf);
            Text::raw("alpha beta")
                .wrap(true)
                .render(Rect::new(0, 0, 5, 3), &mut f);
        }
        assert_eq!(row(&buf, 0), "alpha");
        assert_eq!(row(&buf, 1), "beta ");
    }

    #[test]
    fn hard_break_long_word() {
        let lines = wrap_line("abcdefgh", 3);
        assert_eq!(lines, vec!["abc", "def", "gh"]);
    }

    #[test]
    fn separator_fills_row() {
        let mut buf = CellBuffer::new(UVec2::new(4, 1));
        {
            let mut f = Frame::root(&mut buf);
            Separator::horizontal()
                .ascii()
                .render(Rect::new(0, 0, 4, 1), &mut f);
        }
        assert_eq!(row(&buf, 0), "----");
    }
}
