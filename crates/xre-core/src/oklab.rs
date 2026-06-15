//! OKLab perceptual color space for palette mapping (Stage 4.4).
//!
//! Nearest-color quantization in raw RGB (Euclidean) mismatches human
//! perception — greens look closer than they are, dark colors collapse. OKLab
//! (Björn Ottosson, 2020) is a cheap perceptually-uniform space, so the closest
//! palette entry by OKLab distance is the closest *looking* one
//! (`RiftEngine-Plan/09-phase-4-advanced-shading-performance.md` §4.4).
// Plain `a*b + c` (determinism, as elsewhere); the conversion matrices read
// clearer with single-letter cone/axis names from the published spec.
#![allow(clippy::suboptimal_flops, clippy::many_single_char_names)]

/// A color in the OKLab perceptual space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklab {
    /// Perceived lightness.
    pub l: f32,
    /// Green–red axis.
    pub a: f32,
    /// Blue–yellow axis.
    pub b: f32,
}

/// sRGB gamma → linear.
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl Oklab {
    /// Convert an 8-bit sRGB triple to OKLab.
    #[must_use]
    pub fn from_srgb(rgb: [u8; 3]) -> Self {
        let r = srgb_to_linear(f32::from(rgb[0]) / 255.0);
        let g = srgb_to_linear(f32::from(rgb[1]) / 255.0);
        let b = srgb_to_linear(f32::from(rgb[2]) / 255.0);

        let l = 0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b;

        let l_ = l.cbrt();
        let m_ = m.cbrt();
        let s_ = s.cbrt();

        Self {
            l: 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
            a: 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
            b: 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
        }
    }

    /// Squared perceptual distance to `other` (cheaper than the metric distance,
    /// monotonic, fine for nearest-neighbour selection).
    #[must_use]
    pub fn distance2(self, other: Self) -> f32 {
        let dl = self.l - other.l;
        let da = self.a - other.a;
        let db = self.b - other.b;
        dl * dl + da * da + db * db
    }
}

/// Map an RGB triple to the nearest entry of `palette` by OKLab distance,
/// returning its index (or 0 for an empty palette).
#[must_use]
pub fn nearest_oklab(rgb: [u8; 3], palette: &[[u8; 3]]) -> usize {
    let target = Oklab::from_srgb(rgb);
    let mut best = 0;
    let mut best_d = f32::MAX;
    for (i, &p) in palette.iter().enumerate() {
        let d = target.distance2(Oklab::from_srgb(p));
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    #![allow(clippy::float_cmp)]
    use super::*;

    #[test]
    fn black_is_origin() {
        let c = Oklab::from_srgb([0, 0, 0]);
        assert!(c.l.abs() < 1e-4 && c.a.abs() < 1e-4 && c.b.abs() < 1e-4);
    }

    #[test]
    fn white_lightness_is_one() {
        // Reference: OKLab L of white ≈ 1.0 (Ottosson's spec).
        let c = Oklab::from_srgb([255, 255, 255]);
        assert!((c.l - 1.0).abs() < 1e-3, "white L = {}", c.l);
        assert!(c.a.abs() < 1e-3 && c.b.abs() < 1e-3);
    }

    #[test]
    fn reference_red_value() {
        // Ottosson's published value for sRGB red (#ff0000):
        // L≈0.6279, a≈0.2249, b≈0.1258.
        let c = Oklab::from_srgb([255, 0, 0]);
        assert!((c.l - 0.6279).abs() < 2e-3, "L={}", c.l);
        assert!((c.a - 0.2249).abs() < 2e-3, "a={}", c.a);
        assert!((c.b - 0.1258).abs() < 2e-3, "b={}", c.b);
    }

    #[test]
    fn nearest_prefers_perceptual_match() {
        let palette = [[0, 0, 0], [255, 255, 255], [255, 0, 0]];
        assert_eq!(nearest_oklab([250, 10, 10], &palette), 2);
        assert_eq!(nearest_oklab([20, 20, 20], &palette), 0);
    }
}
