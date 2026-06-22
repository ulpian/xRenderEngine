//! [`Scrollbar`]: a track-and-thumb indicator for scrollable content.
//!
//! Modeled on ratatui's scrollbar but idiomatic to this crate's stateful
//! pattern: a stateless [`Scrollbar`] renderer draws against a [`ScrollbarState`]
//! the application owns across frames. The thumb geometry is computed with
//! integer-only arithmetic ([`thumb_geometry`]) so frames stay bit-identical
//! across platforms (the determinism gate).
//!
//! A scrollbar does not lay itself out: reserve a one-cell strip next to the
//! content with [`crate::Layout`] and render the scrollbar into it, e.g.
//!
//! ```
//! use xre_tui::{Constraint, Layout, Rect, Scrollbar, ScrollbarOrientation, ScrollbarState};
//!
//! // Reserve a 1-cell strip on the right for the scrollbar.
//! let regions = Layout::horizontal([Constraint::Fill(1), Constraint::Len(1)])
//!     .split(Rect::new(0, 0, 20, 8));
//! let (content, track) = (regions[0], regions[1]);
//!
//! // The app owns the state across frames; drive `position` from the content's
//! // scroll offset, then render the bar into `track` with a `Frame`.
//! let mut state = ScrollbarState::new(40).viewport_length(8).position(12);
//! let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
//! // bar.render_stateful(track, &mut frame, &mut state);
//! # let _ = (content, track, bar, state);
//! ```

use xre_core::{Color, Rect, Style};
use xre_term::{MouseButton, MouseEvent, MouseKind};

use crate::frame::Frame;
use crate::widget::Widget;

/// Where the scrollbar sits relative to the content it tracks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScrollbarOrientation {
    /// Vertical bar on the right edge (the common case).
    #[default]
    VerticalRight,
    /// Vertical bar on the left edge.
    VerticalLeft,
    /// Horizontal bar along the bottom edge.
    HorizontalBottom,
    /// Horizontal bar along the top edge.
    HorizontalTop,
}

impl ScrollbarOrientation {
    /// `true` for the vertical orientations.
    #[must_use]
    pub const fn is_vertical(self) -> bool {
        matches!(self, Self::VerticalRight | Self::VerticalLeft)
    }
}

/// Persistent scroll metrics, owned by the application across frames.
///
/// All three quantities are in *content units* (list items, log lines, …):
/// `content_length` is the total scrollable size, `viewport_length` is how many
/// of those are visible, and `position` is the index of the first visible one.
/// [`Scrollbar`] maps them onto the physical track when it renders.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScrollbarState {
    content_length: usize,
    position: usize,
    viewport_length: usize,
}

impl ScrollbarState {
    /// State for content of `content_length` items, nothing scrolled yet.
    #[must_use]
    pub const fn new(content_length: usize) -> Self {
        Self {
            content_length,
            position: 0,
            viewport_length: 0,
        }
    }

    /// Builder: set the total content length.
    #[must_use]
    pub const fn content_length(mut self, content_length: usize) -> Self {
        self.content_length = content_length;
        self
    }

    /// Builder: set the visible (viewport) length in content units.
    #[must_use]
    pub const fn viewport_length(mut self, viewport_length: usize) -> Self {
        self.viewport_length = viewport_length;
        self
    }

    /// Builder: set the scroll position (clamped at render time).
    #[must_use]
    pub const fn position(mut self, position: usize) -> Self {
        self.position = position;
        self
    }

    /// Set the total content length, re-clamping the position into range.
    pub const fn set_content_length(&mut self, content_length: usize) {
        self.content_length = content_length;
        self.clamp();
    }

    /// Set the visible length, re-clamping the position into range.
    pub const fn set_viewport_length(&mut self, viewport_length: usize) {
        self.viewport_length = viewport_length;
        self.clamp();
    }

    /// Set the scroll position, clamped to `0..=max_position`.
    pub fn set_position(&mut self, position: usize) {
        self.position = position.min(self.max_position());
    }

    /// The current scroll position (index of the first visible item).
    #[must_use]
    pub const fn get_position(&self) -> usize {
        self.position
    }

    /// The largest valid scroll position.
    #[must_use]
    pub const fn max_position(&self) -> usize {
        self.content_length.saturating_sub(self.viewport_length)
    }

    /// Position the content so the thumb tracks a click/drag on track cell
    /// `cell` within a track of `track_len` cells (used for thumb dragging).
    pub fn set_from_track_cell(&mut self, cell: u32, track_len: u32) {
        let max_pos = self.max_position();
        if track_len <= 1 || max_pos == 0 {
            self.position = 0;
            return;
        }
        let cell = (cell as usize).min(track_len as usize - 1);
        // Map the cursor cell across the track onto the content range.
        self.position = round_div(cell * max_pos, track_len as usize - 1).min(max_pos);
    }

    /// Clamp the position into `0..=max_position`.
    const fn clamp(&mut self) {
        let max_pos = self.max_position();
        if self.position > max_pos {
            self.position = max_pos;
        }
    }
}

/// A stateless track-and-thumb scrollbar renderer.
#[derive(Clone, Debug)]
pub struct Scrollbar {
    orientation: ScrollbarOrientation,
    track_style: Style,
    thumb_style: Style,
    track_glyph: char,
    thumb_glyph: char,
    begin_glyph: Option<char>,
    end_glyph: Option<char>,
}

impl Scrollbar {
    /// A scrollbar with the given orientation and Unicode glyphs.
    #[must_use]
    pub const fn new(orientation: ScrollbarOrientation) -> Self {
        Self {
            orientation,
            track_style: Style::fg(Color::Rgb(60, 70, 85)),
            thumb_style: Style::fg(Color::Rgb(120, 200, 255)),
            track_glyph: if orientation.is_vertical() {
                '│'
            } else {
                '─'
            },
            thumb_glyph: '█',
            begin_glyph: None,
            end_glyph: None,
        }
    }

    /// Builder: swap to ASCII glyphs (`|`/`-` track, `#` thumb) for degraded
    /// terminals, or back to Unicode.
    #[must_use]
    pub const fn ascii(mut self, ascii: bool) -> Self {
        if ascii {
            self.track_glyph = if self.orientation.is_vertical() {
                '|'
            } else {
                '-'
            };
            self.thumb_glyph = '#';
        } else {
            self.track_glyph = if self.orientation.is_vertical() {
                '│'
            } else {
                '─'
            };
            self.thumb_glyph = '█';
        }
        self
    }

    /// Builder: set the empty-track style.
    #[must_use]
    pub const fn track_style(mut self, style: Style) -> Self {
        self.track_style = style;
        self
    }

    /// Builder: set the thumb style.
    #[must_use]
    pub const fn thumb_style(mut self, style: Style) -> Self {
        self.thumb_style = style;
        self
    }

    /// Builder: override the track glyph.
    #[must_use]
    pub const fn track_glyph(mut self, glyph: char) -> Self {
        self.track_glyph = glyph;
        self
    }

    /// Builder: override the thumb glyph.
    #[must_use]
    pub const fn thumb_glyph(mut self, glyph: char) -> Self {
        self.thumb_glyph = glyph;
        self
    }

    /// Builder: draw optional end-caps at the start/end of the track (e.g.
    /// `Some('▲')`/`Some('▼')`). Each consumes one track cell.
    #[must_use]
    pub const fn caps(mut self, begin: Option<char>, end: Option<char>) -> Self {
        self.begin_glyph = begin;
        self.end_glyph = end;
        self
    }

    /// Handle a mouse event against the scrollbar drawn at `area`, mutating
    /// `state`. Returns `true` if consumed.
    ///
    /// A left press or drag jumps/drags the thumb to the cursor cell (relying on
    /// the [`crate::MouseRouter`]'s drag capture so a drag keeps tracking when the
    /// cursor leaves the narrow track); the wheel nudges the position by one.
    pub fn handle_mouse(&self, ev: &MouseEvent, area: Rect, state: &mut ScrollbarState) -> bool {
        if area.is_empty() {
            return false;
        }
        let vertical = self.orientation.is_vertical();
        let track = if vertical {
            area.height()
        } else {
            area.width()
        };
        match ev.kind {
            MouseKind::Down(MouseButton::Left) | MouseKind::Drag(MouseButton::Left) => {
                let cell = if vertical {
                    ev.row.saturating_sub(area.top())
                } else {
                    ev.col.saturating_sub(area.left())
                };
                state.set_from_track_cell(cell, track);
                true
            }
            MouseKind::ScrollUp => {
                state.set_position(state.get_position().saturating_sub(1));
                true
            }
            MouseKind::ScrollDown => {
                state.set_position(state.get_position() + 1);
                true
            }
            _ => false,
        }
    }

    /// Render against `state`, mapping the content metrics onto `area`'s track.
    pub fn render_stateful(&self, area: Rect, frame: &mut Frame, state: &mut ScrollbarState) {
        if area.is_empty() {
            return;
        }
        state.clamp();
        let vertical = self.orientation.is_vertical();
        let track = if vertical {
            area.height()
        } else {
            area.width()
        };
        let (thumb_start, thumb_len) = thumb_geometry(
            track,
            state.content_length,
            state.viewport_length,
            state.position,
        );
        let mut f = frame.region(area);

        // The fixed cross-axis coordinate of the 1-cell strip.
        let line = match self.orientation {
            ScrollbarOrientation::VerticalRight => area.right() - 1,
            ScrollbarOrientation::VerticalLeft => area.left(),
            ScrollbarOrientation::HorizontalBottom => area.bottom() - 1,
            ScrollbarOrientation::HorizontalTop => area.top(),
        };

        for i in 0..track {
            let in_thumb = i >= thumb_start && i < thumb_start + thumb_len;
            let mut glyph = if in_thumb {
                self.thumb_glyph
            } else {
                self.track_glyph
            };
            let style = if in_thumb {
                self.thumb_style
            } else {
                self.track_style
            };
            if i == 0 {
                if let Some(cap) = self.begin_glyph {
                    glyph = cap;
                }
            } else if i == track - 1 {
                if let Some(cap) = self.end_glyph {
                    glyph = cap;
                }
            }
            if vertical {
                f.set(line, area.top() + i, style.cell(glyph));
            } else {
                f.set(area.left() + i, line, style.cell(glyph));
            }
        }
    }
}

impl Widget for Scrollbar {
    /// Renders an empty full-length track (no content metrics available).
    fn render(&self, area: Rect, frame: &mut Frame) {
        let mut state = ScrollbarState::new(0);
        self.render_stateful(area, frame, &mut state);
    }
}

/// Round `a / b` to the nearest integer (ties up). `b` must be non-zero.
const fn round_div(a: usize, b: usize) -> usize {
    (a + b / 2) / b
}

/// Compute the thumb's `(start, length)` in track cells.
///
/// Integer-only and total over every input, so it is safe to call from the
/// deterministic render path and to property-test directly:
/// - `track == 0` → `(0, 0)`.
/// - nothing to scroll (`content == 0` or `content <= viewport`) → `(0, track)`.
/// - otherwise the thumb is at least one cell, never exceeds the track, sits at
///   the top for `position == 0`, and reaches the bottom (`start + len == track`)
///   at the maximum position.
#[must_use]
pub fn thumb_geometry(track: u32, content: usize, viewport: usize, position: usize) -> (u32, u32) {
    if track == 0 {
        return (0, 0);
    }
    if content == 0 || content <= viewport {
        return (0, track);
    }
    let track_u = track as usize;
    let len = round_div(track_u * viewport, content).clamp(1, track_u);
    let max_pos = content - viewport;
    let pos = position.min(max_pos);
    let travel = track_u - len;
    let start = if max_pos == 0 {
        0
    } else {
        round_div(travel * pos, max_pos)
    };
    (start as u32, len as u32)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::CellBuffer;

    fn col(buf: &CellBuffer, x: u32) -> String {
        (0..buf.height())
            .map(|y| buf.get(x, y).unwrap().glyph)
            .collect()
    }

    #[test]
    fn empty_track_is_zero() {
        assert_eq!(thumb_geometry(0, 100, 10, 5), (0, 0));
    }

    #[test]
    fn nothing_to_scroll_fills_track() {
        assert_eq!(thumb_geometry(8, 5, 8, 0), (0, 8)); // content < viewport
        assert_eq!(thumb_geometry(8, 8, 8, 0), (0, 8)); // content == viewport
        assert_eq!(thumb_geometry(8, 0, 0, 0), (0, 8)); // empty content
    }

    #[test]
    fn thumb_anchors_at_extremes() {
        // 40 items, 8 visible, max position = 32.
        let (start_top, len_top) = thumb_geometry(8, 40, 8, 0);
        assert_eq!(start_top, 0, "top sits at the top");
        let (start_bot, len_bot) = thumb_geometry(8, 40, 8, 32);
        assert_eq!(start_bot + len_bot, 8, "bottom reaches the end");
        assert_eq!(len_top, len_bot, "thumb length is position-invariant");
        assert!(len_top >= 1);
    }

    #[test]
    fn thumb_is_at_least_one_cell() {
        // Tiny track, huge content: thumb collapses to 1 cell, never 0.
        let (_, len) = thumb_geometry(1, 1000, 10, 0);
        assert_eq!(len, 1);
    }

    #[test]
    fn renders_thumb_at_top() {
        let mut buf = CellBuffer::new(UVec2::new(1, 4));
        let mut state = ScrollbarState::new(8).viewport_length(4).position(0);
        {
            let mut f = Frame::root(&mut buf);
            Scrollbar::new(ScrollbarOrientation::VerticalRight).render_stateful(
                Rect::new(0, 0, 1, 4),
                &mut f,
                &mut state,
            );
        }
        // 8 items / 4 visible → 2-cell thumb at the top, track below.
        assert_eq!(col(&buf, 0), "██││");
    }

    #[test]
    fn set_from_track_cell_maps_extremes() {
        let mut state = ScrollbarState::new(40).viewport_length(8);
        state.set_from_track_cell(0, 8);
        assert_eq!(state.get_position(), 0);
        state.set_from_track_cell(7, 8);
        assert_eq!(state.get_position(), state.max_position());
    }

    #[test]
    fn set_position_clamps() {
        let mut state = ScrollbarState::new(10).viewport_length(4);
        state.set_position(999);
        assert_eq!(state.get_position(), 6); // 10 - 4
    }

    use proptest::prelude::*;

    proptest! {
        /// The thumb always fits inside the track and is never zero-width when
        /// there is something to show.
        #[test]
        fn thumb_fits_track(
            track in 0u32..64,
            content in 0usize..1000,
            viewport in 0usize..1000,
            position in 0usize..1000,
        ) {
            let (start, len) = thumb_geometry(track, content, viewport, position);
            prop_assert!(start + len <= track);
            if track > 0 {
                prop_assert!(len >= 1);
            }
        }

        /// Scrolling further down never moves the thumb up.
        #[test]
        fn thumb_position_is_monotonic(
            track in 1u32..64,
            content in 2usize..1000,
            viewport in 1usize..999,
            pa in 0usize..1000,
            pb in 0usize..1000,
        ) {
            prop_assume!(viewport < content);
            let (lo, _) = thumb_geometry(track, content, viewport, pa.min(pb));
            let (hi, _) = thumb_geometry(track, content, viewport, pa.max(pb));
            prop_assert!(lo <= hi);
        }
    }

    #[test]
    fn drag_moves_thumb() {
        use xre_term::{Modifiers, MouseButton};
        let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut state = ScrollbarState::new(40).viewport_length(8);
        let area = Rect::new(0, 0, 1, 8);
        let drag = MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            col: 0,
            row: 7,
            mods: Modifiers::NONE,
        };
        assert!(bar.handle_mouse(&drag, area, &mut state));
        assert_eq!(state.get_position(), state.max_position());
    }
}
