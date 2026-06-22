//! Cell shaders: collapse a sub-cell sample block into one [`Cell`].
//!
//! The [`CellShader`] trait is the second half of the two-stage pipeline — the
//! renderer fills a [`SampleBuffer`] at sub-cell resolution, then a shader maps
//! each `SX × SY` block to a glyph + color. [`LuminanceRamp`] is the Phase 2
//! shader (asciimare's calibrated-ramp model); the 6-D `ShapeVector` shader and
//! the Unicode density modes arrive in Phase 4.
//!
//! Transparency: a cell with no filled samples returns `None`, leaving whatever
//! is underneath untouched — this is what lets a 3D viewport float over the TUI
//! (`RiftEngine-Plan/07-phase-2-renderer-core.md` §2.5).

use xre_core::{Cell, Color};

use crate::density::{BlockShades, Braille, HalfBlock};
use crate::sample::SampleBuffer;
use crate::shape::ShapeVector;

/// Maps one cell's sub-cell samples to a [`Cell`], or `None` if transparent.
///
/// The `Sync` bound lets [`resolve_cells`] shade cells in parallel across threads
/// (Stage 4.5); every built-in shader is immutable state and satisfies it.
pub trait CellShader: Sync {
    /// Shade the cell at grid position `(cx, cy)` of `buf`.
    fn shade(&self, buf: &SampleBuffer, cx: u32, cy: u32) -> Option<Cell>;
}

/// Resolve a `cols × rows` block of cells through `shader`, writing each result
/// (`None` for a transparent cell) into `out` in row-major order.
///
/// With the `parallel` feature the rows are shaded concurrently; because each
/// cell is independent and read-only over `buf`, the output is **byte-identical**
/// to the serial loop. `out` must hold at least `cols * rows` slots.
pub fn resolve_cells(
    buf: &SampleBuffer,
    shader: &dyn CellShader,
    cols: u32,
    rows: u32,
    out: &mut [Option<Cell>],
) {
    let count = (cols as usize) * (rows as usize);
    debug_assert!(out.len() >= count, "resolve_cells: `out` is too small");

    #[cfg(feature = "parallel")]
    {
        use rayon::prelude::*;
        out[..count]
            .par_chunks_mut(cols.max(1) as usize)
            .enumerate()
            .for_each(|(cy, row)| {
                for (cx, slot) in row.iter_mut().enumerate() {
                    *slot = shader.shade(buf, cx as u32, cy as u32);
                }
            });
    }
    #[cfg(not(feature = "parallel"))]
    {
        for cy in 0..rows {
            for cx in 0..cols {
                out[(cy * cols + cx) as usize] = shader.shade(buf, cx, cy);
            }
        }
    }
}

/// The built-in cell shaders paired with short display names, in a stable order.
///
/// Convenience for tools and demos that let the user cycle shaders at runtime
/// (`xre view`, the `spinning-cube` and `rift-fps` examples). Boxes one instance
/// of each shader; intended to be called once at startup, not per frame.
#[must_use]
pub fn builtin_cell_shaders() -> Vec<(&'static str, Box<dyn CellShader>)> {
    vec![
        ("ramp", Box::new(LuminanceRamp::default())),
        ("shape", Box::new(ShapeVector::default())),
        ("half-block", Box::new(HalfBlock)),
        ("blocks", Box::new(BlockShades)),
        ("braille", Box::new(Braille::default())),
    ]
}

/// How a [`LuminanceRamp`] reduces a cell's samples to a single coverage value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LumaBias {
    /// Coverage-weighted mean luma (background counts as 0).
    #[default]
    Mean,
    /// The brightest sample, scaled by coverage (favors highlights/edges).
    MaxBiased,
}

/// The asciimare luminance-ramp shader: pick a glyph by mean luma, color it by
/// the mean sample RGB.
#[derive(Clone, Debug)]
pub struct LuminanceRamp {
    /// `(coverage, glyph)` entries sorted ascending by coverage.
    ramp: Vec<(f32, char)>,
    bias: LumaBias,
    /// If set, darken the cell background proportionally to depth (volume cue).
    depth_darken: bool,
}

impl Default for LuminanceRamp {
    fn default() -> Self {
        Self::from_density(DENSITY_ORDER)
    }
}

/// A density-ordered printable-ASCII ramp, sparsest first (matches glyphgen's
/// built-in ramp so the renderer needs no font atlas to start).
pub const DENSITY_ORDER: &str =
    " .'`^\",:;Il!i><~+_-?][}{1)(|\\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$";

impl LuminanceRamp {
    /// Build a ramp from a density-ordered string (sparsest char first),
    /// spacing coverage uniformly across `0.0..=1.0`.
    #[must_use]
    pub fn from_density(chars: &str) -> Self {
        let glyphs: Vec<char> = chars.chars().collect();
        let last = glyphs.len().saturating_sub(1).max(1) as f32;
        let ramp = glyphs
            .into_iter()
            .enumerate()
            .map(|(i, g)| (i as f32 / last, g))
            .collect();
        Self {
            ramp,
            bias: LumaBias::Mean,
            depth_darken: false,
        }
    }

    /// Build from a font-calibrated ramp (e.g. produced by `glyphgen`). Entries
    /// are sorted ascending by coverage.
    #[must_use]
    pub fn from_ramp(mut entries: Vec<(f32, char)>) -> Self {
        entries.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));
        Self {
            ramp: entries,
            bias: LumaBias::Mean,
            depth_darken: false,
        }
    }

    /// Builder: set the luma reduction bias.
    #[must_use]
    pub const fn bias(mut self, bias: LumaBias) -> Self {
        self.bias = bias;
        self
    }

    /// Builder: enable depth-darkened cell backgrounds.
    #[must_use]
    pub const fn depth_darken(mut self, on: bool) -> Self {
        self.depth_darken = on;
        self
    }

    /// Select the glyph nearest to `coverage` by binary search (asciimare's
    /// `O(log n)` lookup).
    #[must_use]
    pub fn select(&self, coverage: f32) -> char {
        if self.ramp.is_empty() {
            return ' ';
        }
        let pos = self.ramp.partition_point(|e| e.0 < coverage);
        if pos == 0 {
            return self.ramp[0].1;
        }
        if pos >= self.ramp.len() {
            return self.ramp[self.ramp.len() - 1].1;
        }
        let (lo_c, lo_g) = self.ramp[pos - 1];
        let (hi_c, hi_g) = self.ramp[pos];
        if coverage - lo_c <= hi_c - coverage {
            lo_g
        } else {
            hi_g
        }
    }
}

impl CellShader for LuminanceRamp {
    fn shade(&self, buf: &SampleBuffer, cx: u32, cy: u32) -> Option<Cell> {
        let (sx, sy) = buf.samples_per_cell();
        let total = (sx * sy) as f32;
        // Read the SoA planes directly (flat index per sample) instead of the
        // per-sample bounds-checked `get`; same values and accumulation order, so
        // the shaded output is byte-for-byte identical.
        let (luma_p, rgb_p, depth_p) = buf.planes();
        let width = buf.width() as usize;
        let mut luma_sum = 0.0f32;
        let mut luma_max = 0.0f32;
        let mut r = 0u32;
        let mut g = 0u32;
        let mut b = 0u32;
        let mut filled = 0u32;
        let mut depth_sum = 0.0f32;
        for j in 0..sy {
            let row_base = (cy * sy + j) as usize * width;
            for i in 0..sx {
                let idx = row_base + (cx * sx + i) as usize;
                let depth = depth_p[idx];
                if depth.is_finite() {
                    let luma = luma_p[idx];
                    let px = rgb_p[idx];
                    luma_sum += luma;
                    luma_max = luma_max.max(luma);
                    r += u32::from(px[0]);
                    g += u32::from(px[1]);
                    b += u32::from(px[2]);
                    depth_sum += depth;
                    filled += 1;
                }
            }
        }
        if filled == 0 {
            return None; // transparent: leave the underlying cell untouched
        }
        let coverage = match self.bias {
            LumaBias::Mean => luma_sum / total,
            LumaBias::MaxBiased => luma_max * (filled as f32 / total),
        };
        let glyph = self.select(coverage.clamp(0.0, 1.0));
        let fg = Color::Rgb((r / filled) as u8, (g / filled) as u8, (b / filled) as u8);
        let mut cell = Cell::new(glyph).fg(fg);
        if self.depth_darken {
            let avg_depth = (depth_sum / filled as f32).clamp(0.0, 1.0);
            let shade = (1.0 - avg_depth * 0.6).clamp(0.0, 1.0);
            let bg = |c: u8| ((f32::from(c)) * shade * 0.25) as u8;
            cell = cell.bg(Color::Rgb(
                bg((r / filled) as u8),
                bg((g / filled) as u8),
                bg((b / filled) as u8),
            ));
        }
        Some(cell)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::float_cmp,
        clippy::field_reassign_with_default
    )]
    use super::*;
    use crate::sample::Sample;
    use xre_core::math::UVec2;

    #[test]
    fn empty_cell_is_transparent() {
        let mut buf = SampleBuffer::new(UVec2::new(1, 1), 2, 2);
        buf.clear([0, 0, 0]);
        assert_eq!(LuminanceRamp::default().shade(&buf, 0, 0), None);
    }

    #[test]
    fn full_bright_cell_picks_dense_glyph() {
        let mut buf = SampleBuffer::new(UVec2::new(1, 1), 2, 2);
        buf.clear([0, 0, 0]);
        for j in 0..2 {
            for i in 0..2 {
                buf.plot(i, j, Sample::new(1.0, [255, 255, 255], 0.5));
            }
        }
        let ramp = LuminanceRamp::default();
        let cell = ramp.shade(&buf, 0, 0).unwrap();
        // The densest glyph in the default ramp is '$'.
        assert_eq!(cell.glyph, '$');
        assert_eq!(cell.fg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn partial_coverage_picks_sparser_glyph() {
        let mut buf = SampleBuffer::new(UVec2::new(1, 1), 2, 2);
        buf.clear([0, 0, 0]);
        // One of four samples filled bright → coverage ~0.25.
        buf.plot(0, 0, Sample::new(1.0, [255, 255, 255], 0.5));
        let ramp = LuminanceRamp::default();
        let cell = ramp.shade(&buf, 0, 0).unwrap();
        let dense = ramp.select(1.0);
        assert_ne!(cell.glyph, dense, "quarter coverage should not be densest");
    }

    #[test]
    fn select_hits_extremes() {
        let ramp = LuminanceRamp::default();
        assert_eq!(ramp.select(0.0), ' ');
        assert_eq!(ramp.select(1.0), '$');
        assert_eq!(ramp.select(-1.0), ' ');
    }

    #[test]
    fn builtin_shaders_are_named_uniquely_and_shade() {
        let shaders = builtin_cell_shaders();
        assert_eq!(shaders.len(), 5);
        let mut names: Vec<&str> = shaders.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 5, "shader names must be unique");

        // Every shader must handle an empty buffer (transparent) without panicking.
        let mut buf = SampleBuffer::new(UVec2::new(1, 1), 2, 4);
        buf.clear([0, 0, 0]);
        for (_, shader) in &shaders {
            assert_eq!(shader.shade(&buf, 0, 0), None);
        }
    }
}
