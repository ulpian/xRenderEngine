//! The 6-D shape-vector cell shader (Stage 4.2, the alexharri technique).
//!
//! A luminance ramp only knows *how much* ink a cell needs; it cannot tell a
//! top-heavy cell from a bottom-heavy one, so shallow edges stair-step. The
//! shape-vector shader instead samples ink coverage in six staggered regions
//! (2 columns × 3 rows) and picks the glyph whose own 6-D coverage vector is
//! nearest — so a `.`/`:`/`!` transition resolves *smoothly*
//! (`RiftEngine-Plan/09-phase-4-advanced-shading-performance.md` §4.2).
//!
//! Two refinements from the article are applied: the glyph table is
//! **normalized per component** (skip it and lookups collapse to border glyphs),
//! and a **contrast enhancement** (normalize-by-max, exponent, denormalize)
//! sharpens the cell vector before matching.

use xre_core::{Cell, Color};

use crate::sample::SampleBuffer;
use crate::shader::CellShader;

/// The six coverage regions of a cell, in order:
/// top-left, top-right, mid-left, mid-right, bottom-left, bottom-right.
pub type ShapeVec = [f32; 6];

/// A glyph plus its measured 6-D ink-coverage vector.
#[derive(Clone, Copy, Debug)]
pub struct ShapeGlyph {
    /// The glyph.
    pub glyph: char,
    /// Its per-region ink coverage.
    pub vector: ShapeVec,
}

/// A set of glyphs with normalized shape vectors for nearest-neighbour lookup.
#[derive(Clone, Debug)]
pub struct ShapeTable {
    glyphs: Vec<ShapeGlyph>,
}

impl Default for ShapeTable {
    fn default() -> Self {
        Self::builtin_ascii()
    }
}

impl ShapeTable {
    /// Build a table from raw glyph vectors, normalizing each component across
    /// the set to span `0.0..=1.0` (the article's critical fix).
    #[must_use]
    pub fn new(mut glyphs: Vec<ShapeGlyph>) -> Self {
        if glyphs.is_empty() {
            return Self { glyphs };
        }
        let mut max = [f32::EPSILON; 6];
        for g in &glyphs {
            for (m, &v) in max.iter_mut().zip(&g.vector) {
                *m = m.max(v);
            }
        }
        for g in &mut glyphs {
            for (v, &m) in g.vector.iter_mut().zip(&max) {
                *v /= m;
            }
        }
        Self { glyphs }
    }

    /// A hand-authored ASCII shape table covering the ramp from blank to solid.
    /// Good enough to demonstrate directional selection out of the box; a
    /// font-measured table from `glyphgen --shapes` is sharper.
    #[must_use]
    pub fn builtin_ascii() -> Self {
        // [tl, tr, ml, mr, bl, br]
        let raw = [
            (' ', [0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ('.', [0.0, 0.0, 0.0, 0.0, 0.35, 0.35]),
            ('\'', [0.4, 0.4, 0.0, 0.0, 0.0, 0.0]),
            ('`', [0.45, 0.0, 0.0, 0.0, 0.0, 0.0]),
            ('-', [0.0, 0.0, 0.55, 0.55, 0.0, 0.0]),
            ('_', [0.0, 0.0, 0.0, 0.0, 0.6, 0.6]),
            ('^', [0.5, 0.5, 0.1, 0.1, 0.0, 0.0]),
            (':', [0.0, 0.0, 0.45, 0.45, 0.45, 0.45]),
            ('=', [0.0, 0.0, 0.6, 0.6, 0.4, 0.4]),
            ('/', [0.0, 0.6, 0.35, 0.35, 0.6, 0.0]),
            ('\\', [0.6, 0.0, 0.35, 0.35, 0.0, 0.6]),
            ('|', [0.55, 0.55, 0.6, 0.6, 0.55, 0.55]),
            ('!', [0.45, 0.45, 0.45, 0.45, 0.25, 0.25]),
            ('o', [0.3, 0.3, 0.55, 0.55, 0.45, 0.45]),
            ('L', [0.55, 0.05, 0.55, 0.05, 0.6, 0.5]),
            ('7', [0.6, 0.6, 0.1, 0.35, 0.05, 0.4]),
            ('O', [0.6, 0.6, 0.7, 0.7, 0.6, 0.6]),
            ('#', [0.8, 0.8, 0.9, 0.9, 0.8, 0.8]),
            ('@', [0.95, 0.95, 0.95, 0.95, 0.95, 0.95]),
        ];
        Self::new(
            raw.into_iter()
                .map(|(glyph, vector)| ShapeGlyph { glyph, vector })
                .collect(),
        )
    }

    /// The number of glyphs.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.glyphs.len()
    }

    /// Whether the table is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// The glyph whose vector is nearest to `target` (Euclidean, brute force —
    /// the runtime table is small; the font-atlas path uses a quantized LUT).
    #[must_use]
    pub fn nearest(&self, target: ShapeVec) -> char {
        let mut best = ' ';
        let mut best_d = f32::MAX;
        for g in &self.glyphs {
            let mut d = 0.0;
            for (&a, &b) in g.vector.iter().zip(&target) {
                let diff = a - b;
                d += diff * diff;
            }
            if d < best_d {
                best_d = d;
                best = g.glyph;
            }
        }
        best
    }
}

/// The shape-vector cell shader.
#[derive(Clone, Debug)]
pub struct ShapeVector {
    table: ShapeTable,
    /// Contrast sharpening exponent bias (`0.0` = off).
    contrast: f32,
}

impl Default for ShapeVector {
    fn default() -> Self {
        Self {
            table: ShapeTable::default(),
            contrast: 0.6,
        }
    }
}

impl ShapeVector {
    /// A shader over `table`.
    #[must_use]
    pub const fn new(table: ShapeTable) -> Self {
        Self {
            table,
            contrast: 0.6,
        }
    }

    /// Builder: set the contrast knob (maps to the enhancement exponent).
    #[must_use]
    pub const fn contrast(mut self, contrast: f32) -> Self {
        self.contrast = contrast;
        self
    }

    /// Compute a cell's raw 6-D coverage vector (filled fraction per region).
    fn cell_vector(buf: &SampleBuffer, cx: u32, cy: u32) -> (ShapeVec, [u8; 3], u32) {
        let (sx, sy) = buf.samples_per_cell();
        let xm = sx / 2;
        let y1 = sy / 3;
        let y2 = (2 * sy) / 3;
        let mut cover = [0.0f32; 6];
        let mut counts = [0u32; 6];
        let mut r = 0u32;
        let mut g = 0u32;
        let mut b = 0u32;
        let mut filled_total = 0u32;
        for j in 0..sy {
            let row = if j < y1 {
                0
            } else if j < y2 {
                1
            } else {
                2
            };
            for i in 0..sx {
                let col = u32::from(i >= xm);
                let region = (row * 2 + col) as usize;
                counts[region] += 1;
                let s = buf.get(cx * sx + i, cy * sy + j);
                if s.is_filled() {
                    cover[region] += 1.0;
                    r += u32::from(s.rgb[0]);
                    g += u32::from(s.rgb[1]);
                    b += u32::from(s.rgb[2]);
                    filled_total += 1;
                }
            }
        }
        for (c, &n) in cover.iter_mut().zip(&counts) {
            if n > 0 {
                *c /= n as f32;
            }
        }
        let inv = if filled_total > 0 {
            1.0 / filled_total as f32
        } else {
            0.0
        };
        let rgb = [
            (r as f32 * inv) as u8,
            (g as f32 * inv) as u8,
            (b as f32 * inv) as u8,
        ];
        (cover, rgb, filled_total)
    }

    /// Contrast-enhance a vector: normalize by its max, raise to `1 + contrast`,
    /// denormalize — sharpening the dominant directions.
    fn enhance(&self, mut v: ShapeVec) -> ShapeVec {
        let max = v.iter().copied().fold(0.0f32, f32::max);
        if max <= f32::EPSILON {
            return v;
        }
        let exp = 1.0 + self.contrast;
        for c in &mut v {
            *c = (*c / max).powf(exp) * max;
        }
        v
    }
}

impl CellShader for ShapeVector {
    fn shade(&self, buf: &SampleBuffer, cx: u32, cy: u32) -> Option<Cell> {
        let (raw, rgb, filled) = Self::cell_vector(buf, cx, cy);
        if filled == 0 {
            return None;
        }
        let enhanced = self.enhance(raw);
        let glyph = self.table.nearest(enhanced);
        Some(Cell::new(glyph).fg(Color::Rgb(rgb[0], rgb[1], rgb[2])))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::sample::Sample;
    use xre_core::math::UVec2;

    /// Fill a single cell's samples within a row range with bright ink.
    fn cell_fill(rows: std::ops::Range<u32>) -> SampleBuffer {
        let mut buf = SampleBuffer::new(UVec2::new(1, 1), 2, 6);
        buf.clear([0, 0, 0]);
        for j in rows {
            for i in 0..2 {
                buf.plot(i, j, Sample::new(1.0, [200, 200, 200], 0.5));
            }
        }
        buf
    }

    #[test]
    fn table_is_normalized() {
        let t = ShapeTable::builtin_ascii();
        // '@' is the densest glyph, so at least one of its components hits 1.0.
        let at = t.glyphs.iter().find(|g| g.glyph == '@').unwrap();
        assert!(at.vector.iter().any(|&c| (c - 1.0).abs() < 1e-6));
    }

    #[test]
    fn top_and_bottom_ink_pick_different_glyphs() {
        // Same *amount* of ink, different *distribution* — the whole point of
        // shape vectors. A luminance ramp would pick the same glyph for both.
        let shader = ShapeVector::default();
        let top = shader.shade(&cell_fill(0..2), 0, 0).unwrap();
        let bottom = shader.shade(&cell_fill(4..6), 0, 0).unwrap();
        assert_ne!(
            top.glyph, bottom.glyph,
            "top-heavy and bottom-heavy cells must differ ({} vs {})",
            top.glyph, bottom.glyph
        );
    }

    #[test]
    fn shallow_gradient_does_not_stair_step() {
        // A shallow diagonal boundary across several cells should yield a smooth
        // run of distinct glyphs, not a hard two-glyph step (the staircasing
        // regression). We sweep coverage height and count distinct glyphs.
        let shader = ShapeVector::default();
        let mut glyphs = Vec::new();
        for height in 0..=6 {
            let cell = shader.shade(&cell_fill(0..height), 0, 0);
            glyphs.push(cell.map_or(' ', |c| c.glyph));
        }
        let distinct: std::collections::HashSet<char> = glyphs.iter().copied().collect();
        assert!(
            distinct.len() >= 4,
            "expected a smooth transition, got {glyphs:?}"
        );
    }

    #[test]
    fn empty_cell_is_transparent() {
        let mut buf = SampleBuffer::new(UVec2::new(1, 1), 2, 4);
        buf.clear([0, 0, 0]);
        assert_eq!(ShapeVector::default().shade(&buf, 0, 0), None);
    }
}
