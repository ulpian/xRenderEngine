//! The sub-cell [`SampleBuffer`]: the renderer's true canvas.
//!
//! "Characters are not pixels." The renderer rasterizes at sub-cell resolution —
//! each terminal cell owns an `SX × SY` block of samples (2×4 by default) — and a
//! [`crate::CellShader`] later collapses each block into one [`xre_core::Cell`].
//! The buffer is **Struct-of-Arrays** (separate luma / rgb / depth planes) and
//! persistent, so a steady-state frame allocates nothing
//! (`RiftEngine-Plan/07-phase-2-renderer-core.md` §2.1).

use xre_core::math::UVec2;

/// Below this many samples a row-band fill stays serial — rayon's per-call
/// overhead isn't worth it for tiny viewports. Mirrors the rasterizer's
/// `PARALLEL_MIN_SAMPLES` intent (the raycaster's per-band work is lighter, so
/// the bar is lower).
#[cfg(feature = "parallel")]
const PARALLEL_FILL_MIN_SAMPLES: usize = 8192;

/// One sub-cell sample: scalar luma, color, and depth.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    /// Lit luminance in `0.0..=1.0` (drives glyph selection).
    pub luma: f32,
    /// Resolved color (pre-quantization).
    pub rgb: [u8; 3],
    /// Depth; smaller is nearer. [`f32::INFINITY`] marks an empty sample.
    pub depth: f32,
}

impl Sample {
    /// An empty (background) sample at infinite depth.
    pub const EMPTY: Self = Self {
        luma: 0.0,
        rgb: [0, 0, 0],
        depth: f32::INFINITY,
    };

    /// A sample with the given luma/rgb at `depth`.
    #[must_use]
    pub const fn new(luma: f32, rgb: [u8; 3], depth: f32) -> Self {
        Self { luma, rgb, depth }
    }

    /// Whether this sample holds geometry (finite depth).
    #[must_use]
    pub const fn is_filled(&self) -> bool {
        self.depth.is_finite()
    }
}

/// A persistent sub-cell render target in Struct-of-Arrays layout.
#[derive(Clone, Debug)]
pub struct SampleBuffer {
    cells: UVec2,
    sx: u32,
    sy: u32,
    width: u32,
    height: u32,
    luma: Vec<f32>,
    rgb: Vec<[u8; 3]>,
    depth: Vec<f32>,
    background: [u8; 3],
}

impl SampleBuffer {
    /// A buffer for a `cells`-sized viewport with `sx × sy` samples per cell.
    ///
    /// `sx`/`sy` are clamped to at least 1.
    #[must_use]
    pub fn new(cells: UVec2, sx: u32, sy: u32) -> Self {
        let sx = sx.max(1);
        let sy = sy.max(1);
        let width = cells.x * sx;
        let height = cells.y * sy;
        let len = (width as usize) * (height as usize);
        Self {
            cells,
            sx,
            sy,
            width,
            height,
            luma: vec![0.0; len],
            rgb: vec![[0, 0, 0]; len],
            depth: vec![f32::INFINITY; len],
            background: [0, 0, 0],
        }
    }

    /// Viewport size in cells.
    #[must_use]
    pub const fn cells(&self) -> UVec2 {
        self.cells
    }

    /// Samples per cell, `(sx, sy)`.
    #[must_use]
    pub const fn samples_per_cell(&self) -> (u32, u32) {
        (self.sx, self.sy)
    }

    /// Sample-grid width (`cells.x * sx`).
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Sample-grid height (`cells.y * sy`).
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// Resize for a new viewport / samples-per-cell, reallocating only if the
    /// total length changes (otherwise the existing buffers are reused).
    pub fn resize(&mut self, cells: UVec2, sx: u32, sy: u32) {
        let sx = sx.max(1);
        let sy = sy.max(1);
        self.cells = cells;
        self.sx = sx;
        self.sy = sy;
        self.width = cells.x * sx;
        self.height = cells.y * sy;
        let len = (self.width as usize) * (self.height as usize);
        if self.luma.len() != len {
            self.luma.resize(len, 0.0);
            self.rgb.resize(len, [0, 0, 0]);
            self.depth.resize(len, f32::INFINITY);
        }
    }

    /// Clear to `background` color, luma 0, depth +∞ (no geometry).
    pub fn clear(&mut self, background: [u8; 3]) {
        self.background = background;
        self.luma.fill(0.0);
        self.rgb.fill(background);
        self.depth.fill(f32::INFINITY);
    }

    /// The background color set by the last [`SampleBuffer::clear`].
    #[must_use]
    pub const fn background(&self) -> [u8; 3] {
        self.background
    }

    #[inline]
    const fn index(&self, x: u32, y: u32) -> Option<usize> {
        if x < self.width && y < self.height {
            Some((y as usize) * (self.width as usize) + (x as usize))
        } else {
            None
        }
    }

    /// The sample at `(x, y)`, or [`Sample::EMPTY`] if out of bounds.
    #[must_use]
    pub fn get(&self, x: u32, y: u32) -> Sample {
        self.index(x, y).map_or(Sample::EMPTY, |i| Sample {
            luma: self.luma[i],
            rgb: self.rgb[i],
            depth: self.depth[i],
        })
    }

    /// Depth-tested write: store `sample` at `(x, y)` only if it is nearer than
    /// what is already there (LESS test). Out-of-bounds writes are ignored.
    pub fn plot(&mut self, x: u32, y: u32, sample: Sample) {
        if let Some(i) = self.index(x, y) {
            if sample.depth < self.depth[i] {
                self.luma[i] = sample.luma;
                self.rgb[i] = sample.rgb;
                self.depth[i] = sample.depth;
            }
        }
    }

    /// Unconditional write (no depth test) — used by the wireframe/2D paths.
    pub fn put(&mut self, x: u32, y: u32, sample: Sample) {
        if let Some(i) = self.index(x, y) {
            self.luma[i] = sample.luma;
            self.rgb[i] = sample.rgb;
            self.depth[i] = sample.depth;
        }
    }

    /// Debug pixel-lambda fill (the Command_Line_3D `FillRectangle` homage): set
    /// every sample from `f(x, y)`. Great for test fixtures and gradients.
    pub fn fill_with(&mut self, mut f: impl FnMut(u32, u32) -> Sample) {
        for y in 0..self.height {
            for x in 0..self.width {
                let s = f(x, y);
                self.put(x, y, s);
            }
        }
    }

    /// The raw luma / rgb / depth planes (row-major, sample resolution).
    #[must_use]
    pub fn planes(&self) -> (&[f32], &[[u8; 3]], &[f32]) {
        (&self.luma, &self.rgb, &self.depth)
    }

    /// Plot a point at floating sample coordinates with depth testing.
    pub fn point(&mut self, x: f32, y: f32, sample: Sample) {
        if x < 0.0 || y < 0.0 {
            return;
        }
        self.plot(x as u32, y as u32, sample);
    }

    /// Draw a depth-tested line between two sample-space endpoints (DDA), linearly
    /// interpolating depth and luma. Wireframe arrives before triangles, making
    /// every later stage visually debuggable (§2.1).
    #[allow(clippy::many_single_char_names)]
    pub fn line(&mut self, a: (f32, f32, f32), b: (f32, f32, f32), rgb: [u8; 3], luma: f32) {
        let (x0, y0, z0) = a;
        let (x1, y1, z1) = b;
        let dx = x1 - x0;
        let dy = y1 - y0;
        let steps = dx.abs().max(dy.abs()).ceil().max(1.0);
        let inv = 1.0 / steps;
        for i in 0..=(steps as u32) {
            let t = i as f32 * inv;
            let x = x0 + dx * t;
            let y = y0 + dy * t;
            let z = z0 + (z1 - z0) * t;
            if x >= 0.0 && y >= 0.0 {
                self.plot(x as u32, y as u32, Sample::new(luma, rgb, z));
            }
        }
    }

    /// Serial iterator over the buffer split into `band_rows`-tall horizontal
    /// bands (the whole buffer in one band when `band_rows >= height`). Used by
    /// the serial rasterizer path and as the fallback below the parallel
    /// threshold (Stage 4.5).
    pub(crate) fn row_bands_mut(&mut self, band_rows: u32) -> impl Iterator<Item = RowBand<'_>> {
        let band_rows = band_rows.max(1);
        let n = ((band_rows * self.width) as usize).max(1);
        let width = self.width;
        self.luma
            .chunks_mut(n)
            .zip(self.rgb.chunks_mut(n))
            .zip(self.depth.chunks_mut(n))
            .enumerate()
            .map(move |(i, ((luma, rgb), depth))| RowBand {
                luma,
                rgb,
                depth,
                y0: i as u32 * band_rows,
                width,
            })
    }

    /// Parallel iterator over disjoint `band_rows`-tall bands — the unit of
    /// row-parallel rasterization. Each band owns a contiguous, non-aliasing slice
    /// of every plane, so bands can be filled concurrently with no synchronization
    /// and **bit-identical** results to [`SampleBuffer::row_bands_mut`] (each pixel
    /// is computed once, by exactly one band).
    #[cfg(feature = "parallel")]
    pub(crate) fn par_row_bands_mut(
        &mut self,
        band_rows: u32,
    ) -> impl rayon::iter::IndexedParallelIterator<Item = RowBand<'_>> {
        use rayon::prelude::*;
        let band_rows = band_rows.max(1);
        let n = ((band_rows * self.width) as usize).max(1);
        let width = self.width;
        self.luma
            .par_chunks_mut(n)
            .zip(self.rgb.par_chunks_mut(n))
            .zip(self.depth.par_chunks_mut(n))
            .enumerate()
            .map(move |(i, ((luma, rgb), depth))| RowBand {
                luma,
                rgb,
                depth,
                y0: i as u32 * band_rows,
                width,
            })
    }

    /// Fill the buffer in disjoint horizontal row bands, handing each [`RowBand`]
    /// to `f`. With the `parallel` feature on and a large enough buffer the bands
    /// run concurrently across rayon; otherwise a single serial band covers the
    /// whole buffer. Because the bands are disjoint and `f` writes each sample
    /// exactly once, the result is **byte-identical** regardless of thread count —
    /// the determinism guarantee the grid raycaster relies on. `f` may recompute
    /// per-column state once per band; that's deterministic, so it doesn't affect
    /// the output.
    pub fn par_row_bands(&mut self, f: impl Fn(&mut RowBand) + Sync) {
        self.par_row_bands_forced(self.should_band_parallel(), f);
    }

    /// Like [`SampleBuffer::par_row_bands`] but forces the serial or parallel path.
    /// Exposed for the determinism gate that asserts the two are byte-identical;
    /// prefer [`SampleBuffer::par_row_bands`] in normal code. Requesting `parallel`
    /// without the `parallel` feature compiled in runs serially.
    #[doc(hidden)]
    pub fn par_row_bands_forced(&mut self, parallel: bool, f: impl Fn(&mut RowBand) + Sync) {
        #[cfg(feature = "parallel")]
        if parallel {
            use rayon::iter::ParallelIterator;
            let bands = (rayon::current_num_threads() as u32 * 3).max(1);
            let band_rows = self.height.div_ceil(bands).max(1);
            self.par_row_bands_mut(band_rows)
                .for_each(|mut band| f(&mut band));
            return;
        }
        let _ = parallel;
        let height = self.height.max(1);
        for mut band in self.row_bands_mut(height) {
            f(&mut band);
        }
    }

    /// Whether a row-band fill has enough samples (and threads) to be worth
    /// parallelizing. Always `false` without the `parallel` feature.
    #[cfg(feature = "parallel")]
    fn should_band_parallel(&self) -> bool {
        let samples = (self.width as usize) * (self.height as usize);
        // Gate on the cheap, pure sample count *first*: a sub-threshold buffer
        // then never calls `current_num_threads()`, which would otherwise spin up
        // rayon's global pool (and its threads' asynchronous startup allocations)
        // for a frame that renders serially anyway — breaking the
        // zero-alloc-per-frame invariant for small viewports.
        samples >= PARALLEL_FILL_MIN_SAMPLES && rayon::current_num_threads() >= 2
    }

    #[cfg(not(feature = "parallel"))]
    #[allow(clippy::unused_self)]
    const fn should_band_parallel(&self) -> bool {
        false
    }
}

/// A disjoint horizontal slice of the sample planes — the unit of row-parallel
/// rasterization and raycasting (Stage 4.5).
///
/// Owns the rows `[y0, y0 + height)` of the buffer, so distinct bands never alias
/// and can be written on separate threads. Handed to the closure passed to
/// [`SampleBuffer::par_row_bands`].
pub struct RowBand<'a> {
    luma: &'a mut [f32],
    rgb: &'a mut [[u8; 3]],
    depth: &'a mut [f32],
    y0: u32,
    width: u32,
}

impl RowBand<'_> {
    /// The first global sample-row index this band covers.
    #[inline]
    #[must_use]
    pub const fn y0(&self) -> u32 {
        self.y0
    }

    /// The number of sample rows in this band.
    #[inline]
    #[must_use]
    pub const fn height(&self) -> u32 {
        (self.luma.len() / self.width as usize) as u32
    }

    /// The sample-grid width (same for every band).
    #[inline]
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Depth-tested write at band-local row `y_local` (`0..height`) and column
    /// `x` — the [`SampleBuffer::plot`] LESS test against this band's own slice.
    #[inline]
    pub(crate) fn plot(&mut self, x: u32, y_local: u32, sample: Sample) {
        let idx = (y_local as usize) * (self.width as usize) + x as usize;
        if sample.depth < self.depth[idx] {
            self.luma[idx] = sample.luma;
            self.rgb[idx] = sample.rgb;
            self.depth[idx] = sample.depth;
        }
    }

    /// Unconditional write (no depth test) at band-local row `y_local` (`0..height`)
    /// and column `x` — the band-local [`SampleBuffer::put`], used by the raycaster
    /// which paints every sample exactly once.
    #[inline]
    pub fn put(&mut self, x: u32, y_local: u32, sample: Sample) {
        let idx = (y_local as usize) * (self.width as usize) + x as usize;
        self.luma[idx] = sample.luma;
        self.rgb[idx] = sample.rgb;
        self.depth[idx] = sample.depth;
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

    #[test]
    fn dimensions_scale_with_samples() {
        let b = SampleBuffer::new(UVec2::new(10, 5), 2, 4);
        assert_eq!(b.width(), 20);
        assert_eq!(b.height(), 20);
        assert_eq!(b.samples_per_cell(), (2, 4));
    }

    #[test]
    fn depth_test_keeps_nearest() {
        let mut b = SampleBuffer::new(UVec2::new(1, 1), 1, 1);
        b.clear([0, 0, 0]);
        b.plot(0, 0, Sample::new(1.0, [255, 0, 0], 5.0));
        b.plot(0, 0, Sample::new(1.0, [0, 255, 0], 9.0)); // farther: rejected
        assert_eq!(b.get(0, 0).rgb, [255, 0, 0]);
        b.plot(0, 0, Sample::new(1.0, [0, 0, 255], 2.0)); // nearer: accepted
        assert_eq!(b.get(0, 0).rgb, [0, 0, 255]);
    }

    #[test]
    fn clear_marks_background_empty() {
        let mut b = SampleBuffer::new(UVec2::new(2, 2), 1, 1);
        b.clear([10, 20, 30]);
        assert!(!b.get(0, 0).is_filled());
        assert_eq!(b.get(0, 0).rgb, [10, 20, 30]);
    }

    #[test]
    fn resize_reuses_when_len_unchanged() {
        let mut b = SampleBuffer::new(UVec2::new(4, 4), 1, 1);
        b.resize(UVec2::new(2, 2), 2, 2); // same total length (16)
        assert_eq!(b.width(), 4);
        assert_eq!(b.height(), 4);
    }

    #[test]
    fn line_plots_endpoints() {
        let mut b = SampleBuffer::new(UVec2::new(4, 1), 1, 1);
        b.clear([0, 0, 0]);
        b.line((0.0, 0.0, 1.0), (3.0, 0.0, 1.0), [255, 255, 255], 1.0);
        assert!(b.get(0, 0).is_filled());
        assert!(b.get(3, 0).is_filled());
    }

    #[test]
    fn out_of_bounds_is_ignored() {
        let mut b = SampleBuffer::new(UVec2::new(1, 1), 1, 1);
        b.plot(99, 99, Sample::new(1.0, [1, 2, 3], 0.0));
        assert_eq!(b.get(99, 99), Sample::EMPTY);
    }
}
