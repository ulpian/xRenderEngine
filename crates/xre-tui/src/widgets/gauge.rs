//! [`Gauge`] (a labelled progress bar) and [`Sparkline`] (a mini bar chart).

use xre_core::{Color, Rect, Style};

use crate::frame::Frame;
use crate::widget::Widget;

/// The eighths used for sub-cell horizontal bar fills (`▏▎▍▌▋▊▉█`).
const HBLOCKS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];
/// The eighths used for vertical sparkline bars (` ▁▂▃▄▅▆▇█`).
const VBLOCKS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// A horizontal progress bar with sub-cell precision and an optional label.
#[derive(Clone, Debug)]
pub struct Gauge {
    ratio: f32,
    label: Option<String>,
    bar_style: Style,
    bg_style: Style,
    ascii: bool,
}

impl Gauge {
    /// A gauge filled to `ratio` (clamped to `0.0..=1.0`).
    #[must_use]
    pub const fn new(ratio: f32) -> Self {
        Self {
            ratio: ratio.clamp(0.0, 1.0),
            label: None,
            bar_style: Style::fg(Color::Rgb(120, 220, 140)),
            bg_style: Style::fg(Color::Rgb(50, 60, 70)),
            ascii: false,
        }
    }

    /// Builder: overlay a centered label (e.g. `"80%"`).
    #[must_use]
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Builder: set the filled-bar style.
    #[must_use]
    pub const fn bar_style(mut self, style: Style) -> Self {
        self.bar_style = style;
        self
    }

    /// Builder: set the empty-track style.
    #[must_use]
    pub const fn bg_style(mut self, style: Style) -> Self {
        self.bg_style = style;
        self
    }

    /// Builder: use `#`/`-` blocks instead of Unicode eighths (degraded mode).
    #[must_use]
    pub const fn ascii(mut self, ascii: bool) -> Self {
        self.ascii = ascii;
        self
    }
}

impl Widget for Gauge {
    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let mut f = frame.region(area);
        let y = area.top();
        let w = area.width();
        let filled_eighths = (self.ratio * (w * 8) as f32).round() as u32;
        let full = filled_eighths / 8;
        let rem = (filled_eighths % 8) as usize;

        for col in 0..w {
            let x = area.left() + col;
            let (glyph, style) = if col < full {
                (if self.ascii { '#' } else { '█' }, self.bar_style)
            } else if col == full && rem > 0 {
                let partial = if self.ascii { '#' } else { HBLOCKS[rem - 1] };
                (partial, self.bar_style)
            } else {
                (if self.ascii { '-' } else { ' ' }, self.bg_style)
            };
            f.set(x, y, style.cell(glyph));
        }

        // Overlay the label centered, replacing the glyph but keeping each
        // cell's existing background (so the bar shows through behind the text).
        if let Some(label) = &self.label {
            let lw = label.chars().count() as u32;
            if lw <= w {
                let start = area.left() + (w - lw) / 2;
                for (j, ch) in label.chars().enumerate() {
                    let x = start + j as u32;
                    f.overlay_glyph(x, y, ch);
                }
            }
        }
    }
}

/// A compact vertical bar chart (one column per sample) using block eighths.
#[derive(Clone, Debug)]
pub struct Sparkline {
    data: Vec<f32>,
    style: Style,
    max: Option<f32>,
}

impl Sparkline {
    /// A sparkline over `data` (auto-scaled to the data's max by default).
    #[must_use]
    pub fn new(data: impl Into<Vec<f32>>) -> Self {
        Self {
            data: data.into(),
            style: Style::fg(Color::Rgb(200, 180, 100)),
            max: None,
        }
    }

    /// Builder: set the bar style.
    #[must_use]
    pub const fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Builder: fix the value mapped to a full-height bar (instead of auto-max).
    #[must_use]
    pub const fn max(mut self, max: f32) -> Self {
        self.max = Some(max);
        self
    }
}

impl Widget for Sparkline {
    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() || self.data.is_empty() {
            return;
        }
        let max = self
            .max
            .unwrap_or_else(|| self.data.iter().copied().fold(0.0_f32, f32::max))
            .max(f32::EPSILON);
        let y = area.bottom() - 1;
        let w = area.width() as usize;
        // Show the most recent `w` samples (right-aligned, newest on the right).
        let start = self.data.len().saturating_sub(w);
        for (i, &v) in self.data[start..].iter().enumerate() {
            let x = area.left() + i as u32;
            let level = ((v / max).clamp(0.0, 1.0) * 8.0).round() as usize;
            frame.set(x, y, self.style.cell(VBLOCKS[level.min(8)]));
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
    fn gauge_half_full_ascii() {
        let mut buf = CellBuffer::new(UVec2::new(4, 1));
        {
            let mut f = Frame::root(&mut buf);
            Gauge::new(0.5)
                .ascii(true)
                .render(Rect::new(0, 0, 4, 1), &mut f);
        }
        assert_eq!(row(&buf, 0), "##--");
    }

    #[test]
    fn gauge_full_is_all_blocks() {
        let mut buf = CellBuffer::new(UVec2::new(3, 1));
        {
            let mut f = Frame::root(&mut buf);
            Gauge::new(1.0).render(Rect::new(0, 0, 3, 1), &mut f);
        }
        assert_eq!(row(&buf, 0), "███");
    }

    #[test]
    fn gauge_label_overlays() {
        let mut buf = CellBuffer::new(UVec2::new(5, 1));
        {
            let mut f = Frame::root(&mut buf);
            Gauge::new(1.0)
                .label("OK")
                .render(Rect::new(0, 0, 5, 1), &mut f);
        }
        assert!(row(&buf, 0).contains("OK"));
    }

    #[test]
    fn sparkline_scales_to_max() {
        let mut buf = CellBuffer::new(UVec2::new(3, 1));
        {
            let mut f = Frame::root(&mut buf);
            Sparkline::new(vec![0.0, 5.0, 10.0])
                .max(10.0)
                .render(Rect::new(0, 0, 3, 1), &mut f);
        }
        assert_eq!(row(&buf, 0), " ▄█");
    }
}
