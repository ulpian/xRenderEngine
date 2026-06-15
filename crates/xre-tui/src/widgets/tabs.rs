//! [`Tabs`]: a horizontal row of selectable labels.

use xre_core::{Attrs, Rect, Style};

use crate::frame::Frame;
use crate::widget::Widget;

/// A single-row tab strip; the active tab is styled distinctly.
#[derive(Clone, Debug)]
pub struct Tabs<'a> {
    titles: &'a [String],
    selected: usize,
    active_style: Style,
    inactive_style: Style,
    divider: char,
}

impl<'a> Tabs<'a> {
    /// A tab strip over `titles` with `selected` active.
    #[must_use]
    pub fn new(titles: &'a [String], selected: usize) -> Self {
        Self {
            titles,
            selected,
            active_style: Style::DEFAULT.with_attrs(Attrs::BOLD | Attrs::UNDERLINE),
            inactive_style: Style::DEFAULT.with_attrs(Attrs::DIM),
            divider: '│',
        }
    }

    /// Builder: set the active-tab style.
    #[must_use]
    pub const fn active_style(mut self, style: Style) -> Self {
        self.active_style = style;
        self
    }

    /// Builder: set the inactive-tab style.
    #[must_use]
    pub const fn inactive_style(mut self, style: Style) -> Self {
        self.inactive_style = style;
        self
    }

    /// Builder: set the divider glyph between tabs.
    #[must_use]
    pub const fn divider(mut self, divider: char) -> Self {
        self.divider = divider;
        self
    }
}

impl Widget for Tabs<'_> {
    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let mut f = frame.region(area);
        let y = area.top();
        let mut x = area.left();
        for (i, title) in self.titles.iter().enumerate() {
            if x >= area.right() {
                break;
            }
            if i > 0 {
                f.print(
                    x,
                    y,
                    &format!(" {} ", self.divider),
                    Style::DEFAULT.with_attrs(Attrs::DIM),
                );
                x += 3;
            }
            let style = if i == self.selected {
                self.active_style
            } else {
                self.inactive_style
            };
            let label = format!(" {title} ");
            f.print(x, y, &label, style);
            x += label.chars().count() as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::CellBuffer;

    #[test]
    fn active_tab_is_bold() {
        let titles: Vec<String> = vec!["one".into(), "two".into()];
        let mut buf = CellBuffer::new(UVec2::new(20, 1));
        {
            let mut f = Frame::root(&mut buf);
            Tabs::new(&titles, 1).render(Rect::new(0, 0, 20, 1), &mut f);
        }
        // Find a glyph from "two" and check it is bold.
        let mut found_bold = false;
        for x in 0..20 {
            let c = buf.get(x, 0).unwrap();
            if c.glyph == 't' && c.attrs.contains(Attrs::BOLD) {
                found_bold = true;
            }
        }
        assert!(found_bold, "active tab should be bold");
    }
}
