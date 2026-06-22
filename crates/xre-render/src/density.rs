//! Unicode density cell shaders (Stage 4.3).
//!
//! Alternatives to the glyph ramp that exploit block/braille glyphs:
//!
//! - [`HalfBlock`] — `▀` with foreground = top-half color, background =
//!   bottom-half color, doubling vertical *color* resolution (the truecolor
//!   showpiece).
//! - [`BlockShades`] — the ` ░▒▓█` coverage ramp (retro, ASCII-adjacent).
//! - [`Braille`] — a 2×4 dot matrix per cell from a luma threshold, great paired
//!   with wireframe for plotter-style output.
//!
//! Capability-driven selection lives in the application; these are never the
//! default on terminals that may lack the glyphs
//! (`RiftEngine-Plan/09-phase-4-advanced-shading-performance.md` §4.3).

use xre_core::{Cell, Color};

use crate::sample::SampleBuffer;
use crate::shader::CellShader;

/// Average the filled samples of a cell sub-region, returning
/// `(mean_rgb, mean_luma, filled, total)`.
///
/// Reads the SoA planes directly (one `planes()` borrow, a flat index per
/// sample) instead of the per-sample bounds-checked [`SampleBuffer::get`] — the
/// shading loop runs this for every cell every frame, and the values/order are
/// identical, so the output is byte-for-byte unchanged.
fn region_stats(
    buf: &SampleBuffer,
    cx: u32,
    cy: u32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
) -> ([u8; 3], f32, u32, u32) {
    let (sx, sy) = buf.samples_per_cell();
    let (luma_p, rgb_p, depth_p) = buf.planes();
    let width = buf.width() as usize;
    let mut sum = [0u32; 3];
    let mut luma = 0.0f32;
    let mut filled = 0u32;
    let mut total = 0u32;
    for j in y0..y1.min(sy) {
        let row_base = (cy * sy + j) as usize * width;
        for i in x0..x1.min(sx) {
            total += 1;
            let idx = row_base + (cx * sx + i) as usize;
            if depth_p[idx].is_finite() {
                let px = rgb_p[idx];
                sum[0] += u32::from(px[0]);
                sum[1] += u32::from(px[1]);
                sum[2] += u32::from(px[2]);
                luma += luma_p[idx];
                filled += 1;
            }
        }
    }
    if filled == 0 {
        return ([0, 0, 0], 0.0, 0, total);
    }
    (mean_rgb(sum, filled), luma / filled as f32, filled, total)
}

/// Mean of an RGB sum over `filled` samples (`filled == 0` ⇒ black, since the sum
/// is then zero too). Divides by `filled.max(1)` to avoid a branch on the hot path.
#[inline]
fn mean_rgb(sum: [u32; 3], filled: u32) -> [u8; 3] {
    let n = filled.max(1);
    [(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8]
}

/// Mean RGB and filled-count for the top (`rows 0..mid`) and bottom
/// (`rows mid..sy`) halves of a cell in a **single pass** over its samples
/// (`HalfBlock` previously walked the block twice). Reads the planes directly;
/// the per-half accumulation order matches the two old `region_stats` calls, so
/// the result is byte-for-byte unchanged.
fn halfblock_stats(buf: &SampleBuffer, cx: u32, cy: u32) -> ([u8; 3], u32, [u8; 3], u32) {
    let (sx, sy) = buf.samples_per_cell();
    let mid = sy / 2;
    let (_, rgb_p, depth_p) = buf.planes();
    let width = buf.width() as usize;
    let mut top = [0u32; 3];
    let mut bot = [0u32; 3];
    let mut top_filled = 0u32;
    let mut bot_filled = 0u32;
    for j in 0..sy {
        let row_base = (cy * sy + j) as usize * width;
        let is_top = j < mid;
        for i in 0..sx {
            let idx = row_base + (cx * sx + i) as usize;
            if depth_p[idx].is_finite() {
                let px = rgb_p[idx];
                let (acc, count) = if is_top {
                    (&mut top, &mut top_filled)
                } else {
                    (&mut bot, &mut bot_filled)
                };
                acc[0] += u32::from(px[0]);
                acc[1] += u32::from(px[1]);
                acc[2] += u32::from(px[2]);
                *count += 1;
            }
        }
    }
    (
        mean_rgb(top, top_filled),
        top_filled,
        mean_rgb(bot, bot_filled),
        bot_filled,
    )
}

/// The `▀` upper-half-block shader: top color in fg, bottom color in bg.
#[derive(Clone, Copy, Debug, Default)]
pub struct HalfBlock;

impl CellShader for HalfBlock {
    fn shade(&self, buf: &SampleBuffer, cx: u32, cy: u32) -> Option<Cell> {
        let (top_rgb, top_filled, bot_rgb, bot_filled) = halfblock_stats(buf, cx, cy);
        if top_filled == 0 && bot_filled == 0 {
            return None;
        }
        // '▀' shows fg in the top half, bg in the bottom half.
        let fg = if top_filled > 0 {
            Color::Rgb(top_rgb[0], top_rgb[1], top_rgb[2])
        } else {
            Color::Rgb(bot_rgb[0], bot_rgb[1], bot_rgb[2])
        };
        let bg = if bot_filled > 0 {
            Color::Rgb(bot_rgb[0], bot_rgb[1], bot_rgb[2])
        } else {
            Color::Rgb(top_rgb[0], top_rgb[1], top_rgb[2])
        };
        let glyph = if top_filled > 0 { '▀' } else { '▄' };
        let (fg, bg) = if top_filled > 0 { (fg, bg) } else { (bg, fg) };
        Some(Cell::new(glyph).fg(fg).bg(bg))
    }
}

/// The ` ░▒▓█` block-shade ramp.
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockShades;

const BLOCKS: [char; 5] = [' ', '░', '▒', '▓', '█'];

impl CellShader for BlockShades {
    fn shade(&self, buf: &SampleBuffer, cx: u32, cy: u32) -> Option<Cell> {
        let (sx, sy) = buf.samples_per_cell();
        let (rgb, luma, filled, total) = region_stats(buf, cx, cy, 0, 0, sx, sy);
        if filled == 0 {
            return None;
        }
        let coverage = (luma * filled as f32 / total as f32).clamp(0.0, 1.0);
        let idx = (coverage * (BLOCKS.len() - 1) as f32).round() as usize;
        Some(Cell::new(BLOCKS[idx.min(BLOCKS.len() - 1)]).fg(Color::Rgb(rgb[0], rgb[1], rgb[2])))
    }
}

/// A 2×4 braille dot-matrix shader driven by a luma threshold.
#[derive(Clone, Copy, Debug)]
pub struct Braille {
    /// Luma threshold above which a dot is lit (`0.0..=1.0`).
    pub threshold: f32,
}

impl Default for Braille {
    fn default() -> Self {
        Self { threshold: 0.5 }
    }
}

/// The braille bit for dot at grid `(col, row)` of a 2×4 matrix (Unicode order).
const fn braille_bit(col: u32, row: u32) -> u8 {
    // Dots 1..6 fill the top 3 rows column-major, then 7/8 the bottom row.
    match (col, row) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (0, 3) => 0x40,
        (1, 3) => 0x80,
        _ => 0,
    }
}

impl CellShader for Braille {
    fn shade(&self, buf: &SampleBuffer, cx: u32, cy: u32) -> Option<Cell> {
        let (sx, sy) = buf.samples_per_cell();
        let mut bits = 0u8;
        let mut r = 0u32;
        let mut g = 0u32;
        let mut b = 0u32;
        let mut lit = 0u32;
        for dr in 0..4 {
            for dc in 0..2 {
                // Map this dot to its sub-region of the sample grid and test the
                // mean luma against the threshold.
                let x0 = dc * sx / 2;
                let x1 = (dc + 1) * sx / 2;
                let y0 = dr * sy / 4;
                let y1 = (dr + 1) * sy / 4;
                let (rgb, luma, filled, _) =
                    region_stats(buf, cx, cy, x0, y0, x1.max(x0 + 1), y1.max(y0 + 1));
                if filled > 0 && luma >= self.threshold {
                    bits |= braille_bit(dc, dr);
                    r += u32::from(rgb[0]);
                    g += u32::from(rgb[1]);
                    b += u32::from(rgb[2]);
                    lit += 1;
                }
            }
        }
        if bits == 0 {
            return None;
        }
        let glyph = char::from_u32(0x2800 + u32::from(bits)).unwrap_or('⠿');
        let fg = Color::Rgb((r / lit) as u8, (g / lit) as u8, (b / lit) as u8);
        Some(Cell::new(glyph).fg(fg))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::sample::Sample;
    use xre_core::math::UVec2;

    fn buf_with(top: bool, bottom: bool) -> SampleBuffer {
        let mut buf = SampleBuffer::new(UVec2::new(1, 1), 2, 4);
        buf.clear([0, 0, 0]);
        if top {
            for i in 0..2 {
                buf.plot(i, 0, Sample::new(1.0, [255, 0, 0], 0.5));
                buf.plot(i, 1, Sample::new(1.0, [255, 0, 0], 0.5));
            }
        }
        if bottom {
            for i in 0..2 {
                buf.plot(i, 2, Sample::new(1.0, [0, 0, 255], 0.5));
                buf.plot(i, 3, Sample::new(1.0, [0, 0, 255], 0.5));
            }
        }
        buf
    }

    #[test]
    fn halfblock_splits_colors() {
        let cell = HalfBlock.shade(&buf_with(true, true), 0, 0).unwrap();
        assert_eq!(cell.glyph, '▀');
        assert_eq!(cell.fg, Color::Rgb(255, 0, 0)); // top red
        assert_eq!(cell.bg, Color::Rgb(0, 0, 255)); // bottom blue
    }

    #[test]
    fn halfblock_empty_is_transparent() {
        assert_eq!(HalfBlock.shade(&buf_with(false, false), 0, 0), None);
    }

    #[test]
    fn blockshades_full_is_solid() {
        let cell = BlockShades.shade(&buf_with(true, true), 0, 0).unwrap();
        assert_eq!(cell.glyph, '█');
    }

    #[test]
    fn braille_lights_dots() {
        let cell = Braille::default()
            .shade(&buf_with(true, true), 0, 0)
            .unwrap();
        // All 8 dots lit → solid braille cell U+28FF.
        assert_eq!(cell.glyph, '\u{28FF}');
    }

    #[test]
    fn braille_empty_is_transparent() {
        assert_eq!(
            Braille::default().shade(&buf_with(false, false), 0, 0),
            None
        );
    }
}
