//! `glyphgen` — offline font-calibration library for xRenderEngine.
//!
//! Stage 0.4 (see `RiftEngine-Plan/05-phase-0-foundations.md` §0.4): turn a
//! monospace font into a **luminance ramp** `[(coverage, char)]` — the
//! asciimare `grays.py` model reproduced for any font — and emit it as Rust
//! source consumed by `xre-render`. Shape vectors and the quantized LUT
//! (Phase 4) extend this; the schema below leaves room for them.
//!
//! Two ramp sources:
//! - [`measure_font`]: rasterize each glyph with [`ab_glyph`] and measure ink
//!   coverage. This is the real, font-calibrated path.
//! - [`builtin_ramp`]: a font-free, density-ordered ASCII ramp with uniform
//!   coverage spacing — useful for bootstrapping and as a default artifact when
//!   no font file is available (we ship *measurements*, never font outlines).
//!
//! The crate is a dev tool: [`ab_glyph`] never ships in the engine.
#![deny(missing_docs)]

use std::cmp::Ordering;
use std::fmt::Write as _;

use ab_glyph::{Font, FontRef, ScaleFont};

/// Errors produced by `glyphgen`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GlyphGenError {
    /// The supplied bytes could not be parsed as a font.
    #[error("could not parse font data")]
    FontParse,
    /// The charset contained no glyphs to measure.
    #[error("empty charset")]
    EmptyCharset,
}

/// A `Result` specialised to [`GlyphGenError`].
pub type Result<T> = std::result::Result<T, GlyphGenError>;

/// One entry in a luminance ramp: a glyph and its normalized ink coverage in
/// `0.0..=1.0` (0 = blank, 1 = densest glyph in the set).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RampEntry {
    /// Normalized ink coverage.
    pub coverage: f32,
    /// The glyph.
    pub glyph: char,
}

/// A density-ordered printable-ASCII set, sparsest first. Used by
/// [`builtin_ramp`] when no font is available.
pub const DENSITY_ORDER: &str =
    " .'`^\",:;Il!i><~+_-?][}{1)(|\\/tfjrxnuvczXYUJCLQ0OZmwqpdbkhao*#MW&8%B@$";

/// The default charset to measure: the 95 printable ASCII characters.
#[must_use]
pub fn default_charset() -> String {
    (0x20u8..=0x7E).map(char::from).collect()
}

/// Build a font-free ramp from [`DENSITY_ORDER`] with uniform coverage spacing.
///
/// Coverage is the entry's normalized rank, not a measurement; run
/// [`measure_font`] for true per-font coverage. The ordering is still useful for
/// nearest-neighbor selection.
#[must_use]
pub fn builtin_ramp() -> Vec<RampEntry> {
    let chars: Vec<char> = DENSITY_ORDER.chars().collect();
    let last = (chars.len().saturating_sub(1)).max(1) as f32;
    chars
        .into_iter()
        .enumerate()
        .map(|(i, glyph)| RampEntry {
            coverage: i as f32 / last,
            glyph,
        })
        .collect()
}

/// Measure per-glyph ink coverage from a TTF/OTF font.
///
/// Each glyph is rasterized at `px` pixels and its summed anti-aliased coverage
/// is divided by the cell area (advance width × line height), then normalized so
/// the densest glyph is `1.0`. Entries are sorted ascending and de-duplicated by
/// visual distance (`min_gap`) so the ramp stays crisp.
///
/// # Errors
/// Returns [`GlyphGenError::FontParse`] if `font_data` is not a valid font, or
/// [`GlyphGenError::EmptyCharset`] if `charset` is empty.
pub fn measure_font(
    font_data: &[u8],
    charset: &str,
    px: f32,
    min_gap: f32,
) -> Result<Vec<RampEntry>> {
    if charset.is_empty() {
        return Err(GlyphGenError::EmptyCharset);
    }
    let font = FontRef::try_from_slice(font_data).map_err(|_| GlyphGenError::FontParse)?;
    let scaled = font.as_scaled(px);
    let cell_h = (scaled.ascent() - scaled.descent()).max(1.0);

    let mut raw: Vec<(char, f32)> = Vec::new();
    for ch in charset.chars() {
        let id = font.glyph_id(ch);
        let cell_w = scaled.h_advance(id).max(1.0);
        let area = cell_w * cell_h;
        let ink = font
            .outline_glyph(id.with_scale(px))
            .map_or(0.0, |outline| {
                let mut sum = 0.0f32;
                outline.draw(|_, _, c| sum += c);
                sum
            });
        raw.push((ch, ink / area));
    }

    let max = raw.iter().map(|&(_, c)| c).fold(0.0f32, f32::max);
    let norm = if max > 0.0 { max } else { 1.0 };
    let mut entries: Vec<RampEntry> = raw
        .into_iter()
        .map(|(glyph, c)| RampEntry {
            glyph,
            coverage: (c / norm).clamp(0.0, 1.0),
        })
        .collect();
    entries.sort_by(|a, b| {
        a.coverage
            .partial_cmp(&b.coverage)
            .unwrap_or(Ordering::Equal)
    });
    dedupe_by_gap(&mut entries, min_gap);
    Ok(entries)
}

/// One glyph's 6-D shape vector (Phase 4.1).
///
/// Ink coverage in the six staggered regions, ordered `[top-left, top-right,
/// mid-left, mid-right, bottom-left, bottom-right]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShapeEntry {
    /// The glyph.
    pub glyph: char,
    /// Per-region ink coverage, normalized per component across the set.
    pub vector: [f32; 6],
}

/// Measure 6-D shape vectors for `charset` (Phase 4.1).
///
/// Each glyph is rasterized and its anti-aliased ink binned into a 2×3 region
/// grid — the alexharri shape vectors, the offline half of the
/// `xre_render::ShapeVector` shader. The result is normalized per component so
/// each region spans `0.0..=1.0` (the article's critical fix). The full
/// quantized LUT is a later optimization; this emits the per-glyph table the
/// runtime brute-force matcher consumes.
///
/// # Errors
/// Returns [`GlyphGenError::FontParse`] / [`GlyphGenError::EmptyCharset`] as
/// [`measure_font`] does.
pub fn measure_shapes(font_data: &[u8], charset: &str, px: f32) -> Result<Vec<ShapeEntry>> {
    if charset.is_empty() {
        return Err(GlyphGenError::EmptyCharset);
    }
    let font = FontRef::try_from_slice(font_data).map_err(|_| GlyphGenError::FontParse)?;
    let mut entries: Vec<ShapeEntry> = Vec::new();
    for ch in charset.chars() {
        let id = font.glyph_id(ch);
        let mut bins = [0.0f32; 6];
        if let Some(outline) = font.outline_glyph(id.with_scale(px)) {
            let bounds = outline.px_bounds();
            let w = (bounds.width()).max(1.0);
            let h = (bounds.height()).max(1.0);
            outline.draw(|x, y, c| {
                let col = usize::from((x as f32 / w) >= 0.5);
                let frac = (y as f32 / h).clamp(0.0, 0.999);
                let row = (frac * 3.0) as usize;
                bins[row * 2 + col] += c;
            });
        }
        entries.push(ShapeEntry {
            glyph: ch,
            vector: bins,
        });
    }
    // Normalize per component across the whole set.
    let mut max = [f32::EPSILON; 6];
    for e in &entries {
        for (m, &v) in max.iter_mut().zip(&e.vector) {
            *m = m.max(v);
        }
    }
    for e in &mut entries {
        for (v, &m) in e.vector.iter_mut().zip(&max) {
            *v /= m;
        }
    }
    Ok(entries)
}

/// Emit Rust source for a shape-vector table consumable by `xre_render`.
#[must_use]
pub fn emit_shapes_source(static_name: &str, entries: &[ShapeEntry]) -> String {
    let mut out = String::new();
    out.push_str("//! Auto-generated by glyphgen. Do not edit by hand.\n");
    out.push_str("//! Font-measured 6-D glyph shape vectors (alexharri technique).\n\n");
    let _ = writeln!(
        out,
        "/// Shape table ({} glyphs): (glyph, [tl, tr, ml, mr, bl, br]).",
        entries.len()
    );
    let _ = writeln!(out, "pub static {static_name}: &[(char, [f32; 6])] = &[");
    for e in entries {
        let v = e.vector;
        let _ = writeln!(
            out,
            "    ({:?}, [{:.3}, {:.3}, {:.3}, {:.3}, {:.3}, {:.3}]),",
            e.glyph, v[0], v[1], v[2], v[3], v[4], v[5]
        );
    }
    out.push_str("];\n");
    out
}

/// Drop entries whose coverage is within `min_gap` of the previously kept one,
/// keeping the first (sparsest) of each cluster. Assumes `entries` is sorted by
/// coverage ascending.
fn dedupe_by_gap(entries: &mut Vec<RampEntry>, min_gap: f32) {
    if entries.is_empty() {
        return;
    }
    let mut kept = 0usize;
    for i in 1..entries.len() {
        if entries[i].coverage - entries[kept].coverage >= min_gap {
            kept += 1;
            entries[kept] = entries[i];
        }
    }
    entries.truncate(kept + 1);
}

/// Select a glyph for a target `coverage` via nearest-neighbor on a ramp sorted
/// ascending by coverage (binary search, `O(log n)` — asciimare's model).
///
/// Returns a space for an empty ramp.
#[must_use]
pub fn select_glyph(entries: &[RampEntry], coverage: f32) -> char {
    if entries.is_empty() {
        return ' ';
    }
    let pos = entries.partition_point(|e| e.coverage < coverage);
    if pos == 0 {
        return entries[0].glyph;
    }
    if pos >= entries.len() {
        return entries[entries.len() - 1].glyph;
    }
    let lo = &entries[pos - 1];
    let hi = &entries[pos];
    if coverage - lo.coverage <= hi.coverage - coverage {
        lo.glyph
    } else {
        hi.glyph
    }
}

/// Emit Rust source defining a `pub static <static_name>: &[(f32, char)]` ramp.
///
/// The generated file is meant to be checked in and `include!`d (or copied) by
/// `xre-render` — the calibration is data, never logic.
#[must_use]
pub fn emit_ramp_source(static_name: &str, entries: &[RampEntry]) -> String {
    let mut out = String::new();
    out.push_str("//! Auto-generated by glyphgen. Do not edit by hand.\n");
    out.push_str("//! A font-calibrated luminance ramp: (coverage, glyph), sorted ascending.\n\n");
    let _ = writeln!(
        out,
        "/// Luminance ramp ({} entries), sparsest to densest.",
        entries.len()
    );
    let _ = writeln!(out, "pub static {static_name}: &[(f32, char)] = &[");
    for entry in entries {
        let _ = writeln!(out, "    ({:.4}, {:?}),", entry.coverage, entry.glyph);
    }
    out.push_str("];\n");
    out
}

/// Run the glyphgen command-line interface.
///
/// `args` is the argument list **excluding** the program name. Recognised flags:
/// `--font <path>`, `--builtin`, `--out <path>`, `--name <ident>`,
/// `--px <f32>`, `--charset <string>`, `--min-gap <f32>`, `--help`.
///
/// Without `--out` the generated source is printed to stdout. With neither
/// `--font` nor `--builtin`, the built-in density ramp is used.
///
/// # Errors
/// Returns a human-readable message if arguments are invalid or a file
/// operation fails.
pub fn run_cli(args: &[String]) -> std::result::Result<(), String> {
    let mut font: Option<String> = None;
    let mut out: Option<String> = None;
    let mut name: Option<String> = None;
    let mut charset: Option<String> = None;
    let mut px = 64.0f32;
    let mut min_gap = 0.004f32;
    let mut builtin = false;

    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut take = |flag: &str| -> std::result::Result<String, String> {
            it.next()
                .cloned()
                .ok_or_else(|| format!("flag {flag} requires a value"))
        };
        match arg.as_str() {
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            "--builtin" => builtin = true,
            "--font" => font = Some(take("--font")?),
            "--out" => out = Some(take("--out")?),
            "--name" => name = Some(take("--name")?),
            "--charset" => charset = Some(take("--charset")?),
            "--px" => {
                px = take("--px")?
                    .parse()
                    .map_err(|_| "invalid --px".to_string())?;
            }
            "--min-gap" => {
                min_gap = take("--min-gap")?
                    .parse()
                    .map_err(|_| "invalid --min-gap".to_string())?;
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let charset = charset.unwrap_or_else(default_charset);
    let (entries, default_name) = if let Some(path) = font {
        let data = std::fs::read(&path).map_err(|e| format!("reading {path}: {e}"))?;
        let entries = measure_font(&data, &charset, px, min_gap).map_err(|e| e.to_string())?;
        (entries, ramp_name_from_path(&path))
    } else {
        if !builtin {
            eprintln!("glyphgen: no --font given; emitting the built-in density ramp");
        }
        (builtin_ramp(), "RAMP_GENERIC".to_string())
    };

    let name = name.unwrap_or(default_name);
    let source = emit_ramp_source(&name, &entries);
    match out {
        Some(path) => {
            std::fs::write(&path, &source).map_err(|e| format!("writing {path}: {e}"))?;
            eprintln!(
                "glyphgen: wrote {} entries as `{name}` to {path}",
                entries.len()
            );
        }
        None => print!("{source}"),
    }
    Ok(())
}

/// Derive an UPPER_SNAKE static name from a font path's file stem.
fn ramp_name_from_path(path: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("ramp");
    let cleaned: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("RAMP_{cleaned}")
}

/// Print CLI usage to stdout.
fn print_usage() {
    println!("glyphgen — calibrate a font into an xRenderEngine glyph ramp");
    println!("usage: glyphgen [--font <ttf>] [--builtin] [--out <atlas.rs>]");
    println!("                [--name <IDENT>] [--px <f32>] [--charset <str>] [--min-gap <f32>]");
    println!("with no --font, the built-in density ramp is emitted.");
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn ramp_name_from_path_is_sanitised() {
        assert_eq!(ramp_name_from_path("/fonts/Menlo.ttf"), "RAMP_MENLO");
        assert_eq!(
            ramp_name_from_path("JetBrains-Mono.ttf"),
            "RAMP_JETBRAINS_MONO"
        );
    }

    #[test]
    fn default_charset_is_95_printable() {
        let cs = default_charset();
        assert_eq!(cs.chars().count(), 95);
        assert!(cs.starts_with(' '));
        assert!(cs.ends_with('~'));
    }

    #[test]
    fn builtin_ramp_is_monotonic() {
        let ramp = builtin_ramp();
        assert!(ramp.len() > 10);
        assert_eq!(ramp.first().unwrap().glyph, ' ');
        assert!((ramp.first().unwrap().coverage - 0.0).abs() < 1e-6);
        assert!((ramp.last().unwrap().coverage - 1.0).abs() < 1e-6);
        for pair in ramp.windows(2) {
            assert!(pair[1].coverage >= pair[0].coverage);
        }
    }

    #[test]
    fn select_glyph_hits_extremes() {
        let ramp = builtin_ramp();
        assert_eq!(select_glyph(&ramp, 0.0), ' ');
        assert_eq!(select_glyph(&ramp, 1.0), ramp.last().unwrap().glyph);
        // Out-of-range clamps.
        assert_eq!(select_glyph(&ramp, -5.0), ' ');
        assert_eq!(select_glyph(&ramp, 5.0), ramp.last().unwrap().glyph);
        assert_eq!(select_glyph(&[], 0.5), ' ');
    }

    #[test]
    fn dedupe_keeps_spaced_entries() {
        let mut e = vec![
            RampEntry {
                coverage: 0.0,
                glyph: 'a',
            },
            RampEntry {
                coverage: 0.01,
                glyph: 'b',
            },
            RampEntry {
                coverage: 0.5,
                glyph: 'c',
            },
            RampEntry {
                coverage: 0.51,
                glyph: 'd',
            },
            RampEntry {
                coverage: 1.0,
                glyph: 'e',
            },
        ];
        dedupe_by_gap(&mut e, 0.1);
        let glyphs: Vec<char> = e.iter().map(|x| x.glyph).collect();
        assert_eq!(glyphs, vec!['a', 'c', 'e']);
    }

    #[test]
    fn emit_shapes_source_is_well_formed() {
        let entries = vec![
            ShapeEntry {
                glyph: '.',
                vector: [0.0, 0.0, 0.0, 0.0, 1.0, 1.0],
            },
            ShapeEntry {
                glyph: '@',
                vector: [1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
            },
        ];
        let src = emit_shapes_source("SHAPES_TEST", &entries);
        assert!(src.contains("pub static SHAPES_TEST: &[(char, [f32; 6])] = &["));
        assert!(src.contains("'@'"));
        assert!(src.contains("];"));
    }

    #[test]
    fn measure_shapes_rejects_empty_charset() {
        assert!(matches!(
            measure_shapes(&[], "", 64.0),
            Err(GlyphGenError::EmptyCharset)
        ));
    }

    #[test]
    fn emit_source_is_well_formed() {
        let ramp = builtin_ramp();
        let src = emit_ramp_source("RAMP_TEST", &ramp);
        assert!(src.contains("pub static RAMP_TEST: &[(f32, char)] = &["));
        assert!(src.contains("];"));
        assert!(src.contains("' '")); // the space entry, debug-formatted
    }
}
