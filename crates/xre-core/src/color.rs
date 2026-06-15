//! Terminal color with a capability-aware downgrade chain.
//!
//! A [`Color`] is authored at full fidelity ([`Color::Rgb`]) and resolved to the
//! terminal's actual [`ColorDepth`] *at present time, not at draw time* (see
//! `RiftEngine-Plan/02-architecture.md` §3). The downgrade chain is
//! `Rgb → Ansi256 → Ansi16 → mono`.

/// The color fidelity a terminal supports, detected by the capability probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorDepth {
    /// No color; only the default foreground/background.
    Mono,
    /// The 16 standard ANSI colors.
    Ansi16,
    /// The 256-color xterm palette (16 base + 6×6×6 cube + 24 grays).
    Ansi256,
    /// 24-bit "truecolor".
    TrueColor,
}

/// A terminal color.
///
/// [`Color::Default`] means "the terminal's own default" and is never
/// downgraded. All other variants resolve toward the target [`ColorDepth`] via
/// [`Color::resolve`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// The terminal's default color (unset).
    Default,
    /// An index into the 16-color ANSI palette (`0..16`).
    Ansi16(u8),
    /// An index into the 256-color xterm palette (`0..256`).
    Ansi256(u8),
    /// A 24-bit RGB color.
    Rgb(u8, u8, u8),
}

impl Color {
    /// Resolve this color to the given terminal [`ColorDepth`].
    ///
    /// Colors already at or below the target depth pass through unchanged;
    /// richer colors are mapped to the nearest representable value. Under
    /// [`ColorDepth::Mono`] every concrete color collapses to
    /// [`Color::Default`].
    #[must_use]
    pub fn resolve(self, depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::TrueColor => self,
            ColorDepth::Ansi256 => match self {
                Self::Rgb(r, g, b) => Self::Ansi256(rgb_to_ansi256(r, g, b)),
                other => other,
            },
            ColorDepth::Ansi16 => match self {
                Self::Rgb(r, g, b) => Self::Ansi16(rgb_to_ansi16(r, g, b)),
                Self::Ansi256(i) => {
                    let (r, g, b) = ansi256_to_rgb(i);
                    Self::Ansi16(rgb_to_ansi16(r, g, b))
                }
                other => other,
            },
            ColorDepth::Mono => Self::Default,
        }
    }
}

/// The six component levels of the 6×6×6 color cube.
const CUBE_STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];

/// Map an 8-bit component to its 0..6 cube index (xterm thresholds).
const fn component_to_cube_index(v: u8) -> usize {
    if v < 48 {
        0
    } else if v < 115 {
        1
    } else {
        ((v as usize) - 35) / 40
    }
}

/// Squared Euclidean distance between two RGB triples.
fn dist2(a: (u8, u8, u8), b: (u8, u8, u8)) -> i32 {
    let dr = i32::from(a.0) - i32::from(b.0);
    let dg = i32::from(a.1) - i32::from(b.1);
    let db = i32::from(a.2) - i32::from(b.2);
    dr * dr + dg * dg + db * db
}

/// Convert RGB to the nearest 256-color xterm palette index, choosing between
/// the 6×6×6 cube and the 24-step gray ramp by whichever is closer.
#[must_use]
pub fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    let ri = component_to_cube_index(r);
    let gi = component_to_cube_index(g);
    let bi = component_to_cube_index(b);
    let cube_index = 16 + 36 * ri + 6 * gi + bi;
    let cube_rgb = (CUBE_STEPS[ri], CUBE_STEPS[gi], CUBE_STEPS[bi]);

    // Gray ramp 232..=255 holds the values 8, 18, ..., 238.
    let avg = (i32::from(r) + i32::from(g) + i32::from(b)) / 3;
    let gray_step = ((avg - 3).clamp(0, 230) / 10) as usize;
    let gray_level = (8 + gray_step * 10) as u8;
    let gray_index = 232 + gray_step;
    let gray_rgb = (gray_level, gray_level, gray_level);

    let target = (r, g, b);
    if dist2(target, gray_rgb) < dist2(target, cube_rgb) {
        gray_index as u8
    } else {
        cube_index as u8
    }
}

/// The RGB values of the 16 standard ANSI colors (xterm defaults).
const ANSI16_RGB: [(u8, u8, u8); 16] = [
    (0, 0, 0),
    (128, 0, 0),
    (0, 128, 0),
    (128, 128, 0),
    (0, 0, 128),
    (128, 0, 128),
    (0, 128, 128),
    (192, 192, 192),
    (128, 128, 128),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (0, 0, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// Convert RGB to the nearest of the 16 standard ANSI colors.
#[must_use]
pub fn rgb_to_ansi16(r: u8, g: u8, b: u8) -> u8 {
    let target = (r, g, b);
    let mut best = 0usize;
    let mut best_dist = i32::MAX;
    let mut idx = 0;
    while idx < ANSI16_RGB.len() {
        let dist = dist2(target, ANSI16_RGB[idx]);
        if dist < best_dist {
            best_dist = dist;
            best = idx;
        }
        idx += 1;
    }
    best as u8
}

/// Convert a 256-color xterm palette index back to an RGB triple.
#[must_use]
pub const fn ansi256_to_rgb(i: u8) -> (u8, u8, u8) {
    match i {
        0..=15 => ANSI16_RGB[i as usize],
        16..=231 => {
            let n = (i - 16) as usize;
            (
                CUBE_STEPS[n / 36],
                CUBE_STEPS[(n / 6) % 6],
                CUBE_STEPS[n % 6],
            )
        }
        232..=255 => {
            let level = 8 + (i as usize - 232) * 10;
            let level = level as u8;
            (level, level, level)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truecolor_passes_through() {
        let c = Color::Rgb(12, 34, 56);
        assert_eq!(c.resolve(ColorDepth::TrueColor), c);
    }

    #[test]
    fn black_and_white_map_exactly_to_256() {
        assert_eq!(rgb_to_ansi256(0, 0, 0), 16);
        assert_eq!(rgb_to_ansi256(255, 255, 255), 231);
    }

    #[test]
    fn pure_cube_corner_maps_to_256() {
        // (95, 0, 0) is cube index (1,0,0) = 16 + 36 = 52, an exact corner.
        assert_eq!(rgb_to_ansi256(95, 0, 0), 52);
    }

    #[test]
    fn mid_gray_prefers_gray_ramp() {
        // 128,128,128 is closer to gray-ramp 8+12*10=128 than to any cube step.
        let idx = rgb_to_ansi256(128, 128, 128);
        assert!((232..=255).contains(&idx), "expected gray ramp, got {idx}");
    }

    #[test]
    fn black_and_white_map_to_16() {
        assert_eq!(rgb_to_ansi16(0, 0, 0), 0);
        assert_eq!(rgb_to_ansi16(255, 255, 255), 15);
        assert_eq!(rgb_to_ansi16(250, 5, 5), 9); // bright red
    }

    #[test]
    fn ansi256_roundtrips_through_rgb() {
        // Cube and gray indices reconstruct to a value that re-maps to itself.
        for i in 16u8..=255 {
            let (r, g, b) = ansi256_to_rgb(i);
            assert_eq!(rgb_to_ansi256(r, g, b), i, "index {i} failed to round-trip");
        }
    }

    #[test]
    fn mono_collapses_to_default() {
        assert_eq!(
            Color::Rgb(10, 20, 30).resolve(ColorDepth::Mono),
            Color::Default
        );
        assert_eq!(Color::Default.resolve(ColorDepth::Mono), Color::Default);
    }

    #[test]
    fn ansi256_downgrades_to_16() {
        // 256-index white (231) should land on ANSI 16 white (15).
        assert_eq!(
            Color::Ansi256(231).resolve(ColorDepth::Ansi16),
            Color::Ansi16(15)
        );
    }
}
