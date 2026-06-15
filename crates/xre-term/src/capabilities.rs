//! Startup capability probe: color depth, Unicode level, size, synchronized
//! output.
//!
//! The probe reads environment heuristics (`COLORTERM`, `TERM`, `NO_COLOR`,
//! locale variables and `TERM_PROGRAM`). The decision logic is factored into
//! pure functions so it can be unit-tested without a live terminal; only
//! [`Capabilities::probe`] touches the real environment.

use xre_core::math::UVec2;
use xre_core::ColorDepth;

/// How much of Unicode the terminal can be expected to render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnicodeLevel {
    /// Full Unicode, including half-blocks, braille and box-drawing.
    Full,
    /// Block-drawing characters but assume narrow coverage otherwise.
    HalfBlocks,
    /// ASCII only — the safe fallback.
    AsciiOnly,
}

/// A snapshot of what the host terminal supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// Color fidelity.
    pub color: ColorDepth,
    /// Unicode rendering level.
    pub unicode: UnicodeLevel,
    /// Terminal size in cells (`x` = columns, `y` = rows).
    pub size: UVec2,
    /// Whether the terminal advertises synchronized-output (DEC mode 2026)
    /// support, which lets the presenter flush tear-free frames.
    pub synchronized_output: bool,
}

impl Capabilities {
    /// The conservative fallback: 16 colors, ASCII only, an 80×24 grid.
    pub const FALLBACK: Self = Self {
        color: ColorDepth::Ansi16,
        unicode: UnicodeLevel::AsciiOnly,
        size: UVec2::new(80, 24),
        synchronized_output: false,
    };

    /// Probe the live environment for terminal capabilities.
    ///
    /// Never fails: anything undetectable falls back to a safe default
    /// (see [`Capabilities::FALLBACK`]).
    #[must_use]
    pub fn probe() -> Self {
        let env = |key: &str| std::env::var(key).ok();
        let color = color_depth_from_env(
            env("COLORTERM").as_deref(),
            env("TERM").as_deref(),
            env("NO_COLOR").is_some(),
        );
        let unicode = unicode_level_from_env(
            env("LC_ALL").as_deref(),
            env("LC_CTYPE").as_deref(),
            env("LANG").as_deref(),
        );
        let size = crossterm::terminal::size().map_or(Self::FALLBACK.size, |(cols, rows)| {
            UVec2::new(u32::from(cols), u32::from(rows))
        });
        let synchronized_output = synchronized_output_from_env(env("TERM_PROGRAM").as_deref());
        Self {
            color,
            unicode,
            size,
            synchronized_output,
        }
    }
}

/// Decide color depth from the relevant environment variables.
///
/// `NO_COLOR` (any value) forces [`ColorDepth::Mono`]; a `COLORTERM` of
/// `truecolor`/`24bit` implies truecolor; a `256`-flavoured `TERM` implies the
/// 256-color palette; anything else is assumed to handle the 16 ANSI colors.
#[must_use]
pub fn color_depth_from_env(
    colorterm: Option<&str>,
    term: Option<&str>,
    no_color: bool,
) -> ColorDepth {
    if no_color {
        return ColorDepth::Mono;
    }
    if let Some(ct) = colorterm {
        if ct.eq_ignore_ascii_case("truecolor") || ct.eq_ignore_ascii_case("24bit") {
            return ColorDepth::TrueColor;
        }
    }
    if let Some(term) = term {
        if term == "dumb" {
            return ColorDepth::Mono;
        }
        if term.contains("256") {
            return ColorDepth::Ansi256;
        }
        if term.contains("color") || term.contains("xterm") || term.contains("screen") {
            return ColorDepth::Ansi16;
        }
    }
    ColorDepth::Ansi16
}

/// Decide the Unicode level from the locale environment. A `UTF-8`/`UTF8`
/// codeset in any of the locale variables implies full Unicode; otherwise we
/// stay on the ASCII-safe path.
#[must_use]
pub fn unicode_level_from_env(
    lc_all: Option<&str>,
    lc_ctype: Option<&str>,
    lang: Option<&str>,
) -> UnicodeLevel {
    let is_utf8 = |v: Option<&str>| {
        v.is_some_and(|s| {
            let s = s.to_ascii_uppercase();
            s.contains("UTF-8") || s.contains("UTF8")
        })
    };
    if is_utf8(lc_all) || is_utf8(lc_ctype) || is_utf8(lang) {
        UnicodeLevel::Full
    } else {
        UnicodeLevel::AsciiOnly
    }
}

/// Heuristic for synchronized-output support based on `TERM_PROGRAM`.
///
/// A handful of terminals are known to honour DEC mode 2026; absent a runtime
/// DECRQM query (deferred to Phase 1), we allow-list those and default to off.
#[must_use]
pub fn synchronized_output_from_env(term_program: Option<&str>) -> bool {
    matches!(
        term_program,
        Some("iTerm.app" | "WezTerm" | "kitty" | "ghostty")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_forces_mono() {
        assert_eq!(
            color_depth_from_env(Some("truecolor"), Some("xterm-256color"), true),
            ColorDepth::Mono
        );
    }

    #[test]
    fn truecolor_detected() {
        assert_eq!(
            color_depth_from_env(Some("truecolor"), Some("xterm"), false),
            ColorDepth::TrueColor
        );
        assert_eq!(
            color_depth_from_env(Some("24bit"), None, false),
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn term_256_detected() {
        assert_eq!(
            color_depth_from_env(None, Some("xterm-256color"), false),
            ColorDepth::Ansi256
        );
    }

    #[test]
    fn dumb_term_is_mono_plain_is_16() {
        assert_eq!(
            color_depth_from_env(None, Some("dumb"), false),
            ColorDepth::Mono
        );
        assert_eq!(
            color_depth_from_env(None, Some("xterm"), false),
            ColorDepth::Ansi16
        );
        assert_eq!(color_depth_from_env(None, None, false), ColorDepth::Ansi16);
    }

    #[test]
    fn utf8_locale_enables_full_unicode() {
        assert_eq!(
            unicode_level_from_env(Some("en_US.UTF-8"), None, None),
            UnicodeLevel::Full
        );
        assert_eq!(
            unicode_level_from_env(None, None, Some("C.utf8")),
            UnicodeLevel::Full
        );
        assert_eq!(
            unicode_level_from_env(None, None, Some("C")),
            UnicodeLevel::AsciiOnly
        );
    }

    #[test]
    fn sync_output_allowlist() {
        assert!(synchronized_output_from_env(Some("kitty")));
        assert!(!synchronized_output_from_env(Some("Apple_Terminal")));
        assert!(!synchronized_output_from_env(None));
    }
}
