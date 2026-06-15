//! [`Table`]: columns laid out by the shared [`Layout`] solver.

use xre_core::{Attrs, Rect, Style};

use crate::frame::Frame;
use crate::layout::{Constraint, Layout};
use crate::widget::Widget;
use crate::widgets::Align;

/// A simple text table: a header row plus body rows, with per-column
/// [`Constraint`]s reusing the [`Layout`] solver (so column sizing matches the
/// rest of the toolkit).
#[derive(Clone, Debug)]
pub struct Table<'a> {
    header: Option<Vec<String>>,
    rows: &'a [Vec<String>],
    widths: Vec<Constraint>,
    col_gap: u32,
    header_style: Style,
    cell_style: Style,
    align: Align,
}

impl<'a> Table<'a> {
    /// A table over `rows` with the given column constraints.
    #[must_use]
    pub fn new(rows: &'a [Vec<String>], widths: impl Into<Vec<Constraint>>) -> Self {
        Self {
            header: None,
            rows,
            widths: widths.into(),
            col_gap: 1,
            header_style: Style::DEFAULT.with_attrs(Attrs::BOLD),
            cell_style: Style::DEFAULT,
            align: Align::Left,
        }
    }

    /// Builder: add a header row.
    #[must_use]
    pub fn header(mut self, header: impl Into<Vec<String>>) -> Self {
        self.header = Some(header.into());
        self
    }

    /// Builder: set the gap between columns.
    #[must_use]
    pub const fn col_gap(mut self, gap: u32) -> Self {
        self.col_gap = gap;
        self
    }

    /// Builder: set the header style.
    #[must_use]
    pub const fn header_style(mut self, style: Style) -> Self {
        self.header_style = style;
        self
    }

    /// Builder: set the body-cell style.
    #[must_use]
    pub const fn cell_style(mut self, style: Style) -> Self {
        self.cell_style = style;
        self
    }

    /// Builder: set the cell alignment.
    #[must_use]
    pub const fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    fn draw_row(&self, f: &mut Frame, cols: &[Rect], y: u32, cells: &[String], style: Style) {
        for (rect, text) in cols.iter().zip(cells) {
            if rect.width() == 0 {
                continue;
            }
            let tw = text.chars().take(rect.width() as usize).count() as u32;
            let x = rect.left() + self.align.offset(tw, rect.width());
            let clipped: String = text.chars().take(rect.width() as usize).collect();
            f.print(x, y, &clipped, style);
        }
    }
}

impl Widget for Table<'_> {
    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let cols = Layout::horizontal(self.widths.clone())
            .gap(self.col_gap)
            .split(area);
        let mut f = frame.region(area);
        let mut y = area.top();
        if let Some(header) = &self.header {
            self.draw_row(&mut f, &cols, y, header, self.header_style);
            y += 1;
        }
        for row in self.rows {
            if y >= area.bottom() {
                break;
            }
            self.draw_row(&mut f, &cols, y, row, self.cell_style);
            y += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::CellBuffer;

    fn rows_of(buf: &CellBuffer) -> Vec<String> {
        (0..buf.height())
            .map(|y| {
                (0..buf.width())
                    .map(|x| buf.get(x, y).unwrap().glyph)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn header_and_body_align_to_columns() {
        let data = vec![vec!["a".into(), "1".into()], vec!["b".into(), "2".into()]];
        let mut buf = CellBuffer::new(UVec2::new(7, 3));
        {
            let mut f = Frame::root(&mut buf);
            Table::new(&data, [Constraint::Len(3), Constraint::Len(3)])
                .header(vec!["k".to_string(), "v".to_string()])
                .render(Rect::new(0, 0, 7, 3), &mut f);
        }
        let r = rows_of(&buf);
        assert_eq!(&r[0][..1], "k");
        assert_eq!(&r[0][4..5], "v"); // second column starts after len 3 + gap 1
        assert_eq!(&r[1][..1], "a");
        assert_eq!(&r[2][4..5], "2");
    }
}
