//! [`Viewport3D`]: the widget that draws a [`SampleBuffer`] through a
//! [`CellShader`] into the shared [`Frame`].
//!
//! This is the keystone of "one frame, two engines": the 3D renderer fills a
//! sample buffer at sub-cell resolution, and this widget resolves each cell with
//! a cell shader and composites it onto the TUI — transparent (background) cells
//! leave whatever is underneath untouched, so the viewport can float over panels
//! (`RiftEngine-Plan/07-phase-2-renderer-core.md` §2.5).
//!
//! Rendering is immediate-mode and read-only: the application owns the
//! [`SampleBuffer`], fills it each frame with `xre_render::draw_mesh`, then wraps
//! it in a `Viewport3D` to present it. Colors are emitted as `Color::Rgb`; the
//! presenter downgrades them to the terminal's depth at flush time.

use xre_core::Rect;
use xre_render::{CellShader, SampleBuffer};

use crate::frame::Frame;
use crate::widget::Widget;

/// A widget that resolves a [`SampleBuffer`] to glyphs via a [`CellShader`].
pub struct Viewport3D<'a> {
    buffer: &'a SampleBuffer,
    shader: &'a dyn CellShader,
}

impl<'a> Viewport3D<'a> {
    /// Present `buffer` resolved through `shader`.
    #[must_use]
    pub fn new(buffer: &'a SampleBuffer, shader: &'a dyn CellShader) -> Self {
        Self { buffer, shader }
    }

    /// The cell dimensions of the underlying sample buffer.
    #[must_use]
    pub const fn cells(&self) -> xre_core::math::UVec2 {
        self.buffer.cells()
    }
}

impl Widget for Viewport3D<'_> {
    fn render(&self, area: Rect, frame: &mut Frame) {
        let cells = self.buffer.cells();
        let cols = area.width().min(cells.x);
        let rows = area.height().min(cells.y);

        // With the `parallel` feature, shade the whole block row-parallel into a
        // thread-local scratch (zero per-frame allocation after warmup), then
        // composite serially. The shade results are identical to the serial loop,
        // so this only changes throughput, never pixels.
        #[cfg(feature = "parallel")]
        {
            use std::cell::RefCell;
            thread_local! {
                static SCRATCH: RefCell<Vec<Option<xre_core::Cell>>> =
                    const { RefCell::new(Vec::new()) };
            }
            SCRATCH.with(|s| {
                let mut scratch = s.borrow_mut();
                let count = (cols as usize) * (rows as usize);
                if scratch.len() < count {
                    scratch.resize(count, None);
                }
                xre_render::resolve_cells(self.buffer, self.shader, cols, rows, &mut scratch);
                for cy in 0..rows {
                    for cx in 0..cols {
                        // Transparent cells (no geometry) are skipped, leaving the
                        // underlying TUI cell untouched.
                        if let Some(cell) = scratch[(cy * cols + cx) as usize] {
                            frame.set(area.left() + cx, area.top() + cy, cell);
                        }
                    }
                }
            });
        }

        #[cfg(not(feature = "parallel"))]
        for cy in 0..rows {
            for cx in 0..cols {
                // Transparent cells (no geometry) are skipped, leaving the
                // underlying TUI cell untouched.
                if let Some(cell) = self.shader.shade(self.buffer, cx, cy) {
                    frame.set(area.left() + cx, area.top() + cy, cell);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::{Cell, CellBuffer};
    use xre_render::{LuminanceRamp, Sample};

    #[test]
    fn transparent_cells_leave_background() {
        let mut sbuf = SampleBuffer::new(UVec2::new(3, 1), 2, 2);
        sbuf.clear([0, 0, 0]);
        // Fill only the middle cell's samples.
        for j in 0..2 {
            for i in 0..2 {
                sbuf.plot(2 + i, j, Sample::new(1.0, [255, 255, 255], 0.5));
            }
        }
        let shader = LuminanceRamp::default();

        let mut cbuf = CellBuffer::new(UVec2::new(3, 1));
        cbuf.fill(Cell::new('.')); // background marker
        {
            let mut frame = Frame::root(&mut cbuf);
            Viewport3D::new(&sbuf, &shader).render(Rect::new(0, 0, 3, 1), &mut frame);
        }
        // Cell 0 and 2 were transparent → still '.'; cell 1 got a glyph.
        assert_eq!(cbuf.get(0, 0).unwrap().glyph, '.');
        assert_ne!(cbuf.get(1, 0).unwrap().glyph, '.');
        assert_eq!(cbuf.get(2, 0).unwrap().glyph, '.');
    }
}
