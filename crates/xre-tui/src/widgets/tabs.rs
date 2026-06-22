//! [`Tabs`]: a horizontal row of selectable labels.

use xre_core::{Attrs, Rect, Style};
use xre_term::{MouseButton, MouseEvent, MouseKind};

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

    /// The index of the tab whose label covers cell `(col, row)`, if any.
    ///
    /// Re-walks the exact layout used by [`Tabs::render`] (`" {title} "` labels
    /// separated by 3-cell `" {divider} "` gaps); a click on a divider yields
    /// `None`.
    #[must_use]
    pub fn hit(&self, area: Rect, col: u32, row: u32) -> Option<usize> {
        if area.is_empty() || row != area.top() {
            return None;
        }
        let mut x = area.left();
        for (i, title) in self.titles.iter().enumerate() {
            if x >= area.right() {
                break;
            }
            if i > 0 {
                x += 3; // " {divider} "
            }
            let label_w = title.chars().count() as u32 + 2; // " {title} "
            if col >= x && col < x + label_w {
                return Some(i);
            }
            x += label_w;
        }
        None
    }

    /// Handle a mouse event against the strip at `area`.
    ///
    /// Returns the index of the tab a left click landed on (the application owns
    /// the active index and applies it), or `None`.
    #[must_use]
    pub fn handle_mouse(&self, ev: &MouseEvent, area: Rect) -> Option<usize> {
        if ev.kind == MouseKind::Down(MouseButton::Left) {
            return self.hit(area, ev.col, ev.row);
        }
        None
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

    #[test]
    fn hit_maps_columns_to_tabs() {
        let titles: Vec<String> = vec!["one".into(), "two".into(), "three".into()];
        let tabs = Tabs::new(&titles, 0);
        let area = Rect::new(0, 0, 30, 1);
        // " one " = cols 0..5; " │ " = 5..8; " two " = 8..13; " │ " = 13..16;
        // " three " = 16..23.
        assert_eq!(tabs.hit(area, 0, 0), Some(0));
        assert_eq!(tabs.hit(area, 4, 0), Some(0));
        assert_eq!(tabs.hit(area, 6, 0), None, "divider is not a tab");
        assert_eq!(tabs.hit(area, 9, 0), Some(1));
        assert_eq!(tabs.hit(area, 18, 0), Some(2));
        assert_eq!(tabs.hit(area, 1, 1), None, "wrong row");
    }
}
