//! Golden-frame tests for the [`Scrollbar`] widget: the track-and-thumb rendered
//! to text at several scroll positions. Frames are text, so they diff perfectly
//! and stay byte-identical across CI OSes (the determinism gate). No mouse input
//! is involved — pure rendering.
#![allow(clippy::unwrap_used)]

use insta::assert_snapshot;
use xre_core::math::UVec2;
use xre_core::CellBuffer;
use xre_tui::{Frame, Rect, Scrollbar, ScrollbarOrientation, ScrollbarState};

/// Render a vertical scrollbar of height `h` for the given content metrics into a
/// 1-column buffer and return its glyph column as a newline-joined string.
fn vbar(content: usize, viewport: usize, position: usize, h: u32) -> String {
    let mut buf = CellBuffer::new(UVec2::new(1, h));
    {
        let mut f = Frame::root(&mut buf);
        let mut state = ScrollbarState::new(content)
            .viewport_length(viewport)
            .position(position);
        Scrollbar::new(ScrollbarOrientation::VerticalRight).render_stateful(
            Rect::new(0, 0, 1, h),
            &mut f,
            &mut state,
        );
    }
    (0..h)
        .map(|y| buf.get(0, y).unwrap().glyph.to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn vertical_scrollbar_positions() {
    // 24 items, 6 visible → a 2-cell thumb travelling a 6-cell track.
    let h = 6;
    let combined = format!(
        "top:\n{}\n\nmid:\n{}\n\nbot:\n{}\n",
        vbar(24, 6, 0, h),
        vbar(24, 6, 9, h),
        vbar(24, 6, 18, h),
    );
    assert_snapshot!(combined);
}

#[test]
fn vertical_scrollbar_full_when_content_fits() {
    // Content fits the viewport → the thumb fills the whole track.
    assert_snapshot!(vbar(4, 8, 0, 6));
}
