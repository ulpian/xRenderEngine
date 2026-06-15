//! [`RenderSettings`]: the knobs a `Viewport3D` exposes over the pipeline.

use crate::raster::{Cull, ShadeMode};

/// Configuration for one rendered frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderSettings {
    /// Samples per cell `(sx, sy)` — the sub-cell resolution (2×4 default).
    pub samples_per_cell: (u32, u32),
    /// Which triangle facings to discard.
    pub cull: Cull,
    /// Lighting evaluation mode.
    pub shade_mode: ShadeMode,
    /// Background color the sample buffer is cleared to.
    pub background: [u8; 3],
    /// Draw triangle edges instead of filled faces.
    pub wireframe: bool,
}

impl Default for RenderSettings {
    fn default() -> Self {
        Self {
            samples_per_cell: (2, 4),
            cull: Cull::Back,
            shade_mode: ShadeMode::PerSample,
            background: [0, 0, 0],
            wireframe: false,
        }
    }
}

impl RenderSettings {
    /// Builder: set samples per cell.
    #[must_use]
    pub const fn samples(mut self, sx: u32, sy: u32) -> Self {
        self.samples_per_cell = (sx, sy);
        self
    }

    /// Builder: set the shading mode.
    #[must_use]
    pub const fn shade_mode(mut self, mode: ShadeMode) -> Self {
        self.shade_mode = mode;
        self
    }

    /// Builder: set the cull mode.
    #[must_use]
    pub const fn cull(mut self, cull: Cull) -> Self {
        self.cull = cull;
        self
    }

    /// Builder: toggle wireframe.
    #[must_use]
    pub const fn wireframe(mut self, on: bool) -> Self {
        self.wireframe = on;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_2x4_backface_persample() {
        let s = RenderSettings::default();
        assert_eq!(s.samples_per_cell, (2, 4));
        assert_eq!(s.cull, Cull::Back);
        assert_eq!(s.shade_mode, ShadeMode::PerSample);
    }
}
