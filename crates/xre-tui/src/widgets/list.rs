//! [`List`]: a scrollable, single-selection list with a sticky viewport.

use xre_core::{Rect, Style};

use crate::frame::Frame;
use crate::widget::Widget;

/// Persistent selection + scroll state for a [`List`].
///
/// Kept by the application across frames; the [`List`] widget borrows it to
/// render and the app drives it with [`ListState::select_next`] etc. in response
/// to input.
#[derive(Clone, Debug, Default)]
pub struct ListState {
    selected: usize,
    offset: usize,
    len: usize,
}

impl ListState {
    /// Fresh state with nothing selected and no scroll.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The selected index (meaningful only when the list is non-empty).
    #[must_use]
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// Set the number of items, clamping the selection into range.
    pub const fn set_len(&mut self, len: usize) {
        self.len = len;
        if self.selected >= len {
            self.selected = len.saturating_sub(1);
        }
    }

    /// Move the selection down by one (clamped).
    pub fn select_next(&mut self) {
        if self.len > 0 {
            self.selected = (self.selected + 1).min(self.len - 1);
        }
    }

    /// Move the selection up by one (clamped).
    pub const fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Select a specific index (clamped to range).
    pub fn select(&mut self, index: usize) {
        self.selected = if self.len == 0 {
            0
        } else {
            index.min(self.len - 1)
        };
    }

    /// Recompute the scroll offset so the selection is visible in `height` rows.
    const fn ensure_visible(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
    }
}

/// A list of string items rendered against a [`ListState`].
#[derive(Clone, Debug)]
pub struct List<'a> {
    items: &'a [String],
    item_style: Style,
    selected_style: Style,
    highlight: &'a str,
}

impl<'a> List<'a> {
    /// A list over `items`.
    #[must_use]
    pub const fn new(items: &'a [String]) -> Self {
        Self {
            items,
            item_style: Style::DEFAULT,
            selected_style: Style::DEFAULT.with_attrs(xre_core::Attrs::BOLD),
            highlight: "> ",
        }
    }

    /// Builder: set the normal item style.
    #[must_use]
    pub const fn item_style(mut self, style: Style) -> Self {
        self.item_style = style;
        self
    }

    /// Builder: set the selected item style (drawn across the whole row).
    #[must_use]
    pub const fn selected_style(mut self, style: Style) -> Self {
        self.selected_style = style;
        self
    }

    /// Builder: set the prefix drawn before the selected item (`"> "` default).
    #[must_use]
    pub const fn highlight_symbol(mut self, symbol: &'a str) -> Self {
        self.highlight = symbol;
        self
    }

    /// Render against `state`, updating its scroll offset to keep the selection
    /// visible. This is the stateful entry point; the [`Widget`] impl renders
    /// without a selection.
    pub fn render_stateful(&self, area: Rect, frame: &mut Frame, state: &mut ListState) {
        if area.is_empty() {
            return;
        }
        state.set_len(self.items.len());
        state.ensure_visible(area.height() as usize);
        let mut f = frame.region(area);
        let indent = self.highlight.chars().count() as u32;
        for (row, item) in self
            .items
            .iter()
            .enumerate()
            .skip(state.offset)
            .take(area.height() as usize)
        {
            let y = area.top() + (row - state.offset) as u32;
            let selected = row == state.selected;
            let style = if selected {
                self.selected_style
            } else {
                self.item_style
            };
            if selected {
                f.style_rect(Rect::new(area.left(), y, area.width(), 1), style);
                f.print(area.left(), y, self.highlight, style);
            }
            f.print(area.left() + indent, y, item, style);
        }
    }
}

impl Widget for List<'_> {
    fn render(&self, area: Rect, frame: &mut Frame) {
        let mut state = ListState::new();
        self.render_stateful(area, frame, &mut state);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::CellBuffer;

    fn rows(buf: &CellBuffer) -> Vec<String> {
        (0..buf.height())
            .map(|y| {
                (0..buf.width())
                    .map(|x| buf.get(x, y).unwrap().glyph)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn selection_clamps_and_scrolls() {
        let items: Vec<String> = (0..5).map(|i| format!("item{i}")).collect();
        let mut state = ListState::new();
        state.set_len(items.len());
        state.select(99); // clamps to 4
        assert_eq!(state.selected(), 4);

        let mut buf = CellBuffer::new(UVec2::new(8, 2));
        {
            let mut f = Frame::root(&mut buf);
            List::new(&items).render_stateful(Rect::new(0, 0, 8, 2), &mut f, &mut state);
        }
        // Only the last two items are visible (offset scrolled to keep #4).
        let r = rows(&buf);
        assert!(r[1].contains("item4"));
        assert!(r[0].contains("item3"));
    }

    #[test]
    fn highlight_symbol_on_selected() {
        let items: Vec<String> = vec!["a".into(), "b".into()];
        let mut state = ListState::new();
        state.set_len(2);
        state.select(0);
        let mut buf = CellBuffer::new(UVec2::new(4, 2));
        {
            let mut f = Frame::root(&mut buf);
            List::new(&items).render_stateful(Rect::new(0, 0, 4, 2), &mut f, &mut state);
        }
        let r = rows(&buf);
        assert_eq!(&r[0][..2], "> ");
        assert_eq!(&r[1][..2], "  ");
    }
}
