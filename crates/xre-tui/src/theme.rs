//! [`Theme`]: a named style table.
//!
//! Widgets look styles up by dotted key (`"panel.title"`, `"list.selected"`)
//! rather than hard-coding colors, so an application can restyle the whole UI by
//! swapping one [`Theme`]. Two built-ins ship: [`Theme::default`] (a truecolor
//! scheme) and [`Theme::mono`] (attributes only, for 16-color/ASCII terminals).

use std::collections::HashMap;

use xre_core::{Attrs, Color, Style};

/// A lookup table from style keys to [`Style`]s, with a fallback.
#[derive(Clone, Debug)]
pub struct Theme {
    styles: HashMap<String, Style>,
    fallback: Style,
}

impl Default for Theme {
    fn default() -> Self {
        let mut t = Self {
            styles: HashMap::new(),
            fallback: Style::DEFAULT,
        };
        let fg = |r, g, b| Style::fg(Color::Rgb(r, g, b));
        t.set("text", Style::DEFAULT);
        t.set("text.dim", Style::DEFAULT.with_attrs(Attrs::DIM));
        t.set("panel.border", fg(90, 100, 120));
        t.set("panel.title", fg(120, 200, 255).with_attrs(Attrs::BOLD));
        t.set("list.item", Style::DEFAULT);
        t.set(
            "list.selected",
            Style::fg(Color::Rgb(20, 20, 30)).with_bg(Color::Rgb(120, 200, 255)),
        );
        t.set("gauge.bar", fg(120, 220, 140));
        t.set("gauge.bg", Style::fg(Color::Rgb(50, 60, 70)));
        t.set("sparkline", fg(200, 180, 100));
        t.set("tabs.active", fg(255, 255, 255).with_attrs(Attrs::BOLD));
        t.set("tabs.inactive", Style::DEFAULT.with_attrs(Attrs::DIM));
        t.set("input.text", Style::DEFAULT);
        t.set(
            "input.cursor",
            Style::fg(Color::Rgb(0, 0, 0)).with_bg(Color::Rgb(220, 220, 220)),
        );
        t.set("log.line", Style::DEFAULT);
        t.set("focus.ring", fg(255, 200, 80));
        t
    }
}

impl Theme {
    /// An empty theme whose every lookup returns `fallback`.
    #[must_use]
    pub fn blank(fallback: Style) -> Self {
        Self {
            styles: HashMap::new(),
            fallback,
        }
    }

    /// A monochrome theme: no colors, only attributes (bold/dim/reverse-ish via
    /// background). Suitable for 16-color or ASCII terminals.
    #[must_use]
    pub fn mono() -> Self {
        let mut t = Self::blank(Style::DEFAULT);
        t.set("panel.title", Style::DEFAULT.with_attrs(Attrs::BOLD));
        t.set("text.dim", Style::DEFAULT.with_attrs(Attrs::DIM));
        t.set(
            "list.selected",
            Style::DEFAULT.with_attrs(Attrs::BOLD | Attrs::UNDERLINE),
        );
        t.set("tabs.active", Style::DEFAULT.with_attrs(Attrs::BOLD));
        t.set("tabs.inactive", Style::DEFAULT.with_attrs(Attrs::DIM));
        t.set("input.cursor", Style::DEFAULT.with_attrs(Attrs::UNDERLINE));
        t.set("focus.ring", Style::DEFAULT.with_attrs(Attrs::BOLD));
        t
    }

    /// Insert or replace the style for `key`.
    pub fn set(&mut self, key: impl Into<String>, style: Style) {
        self.styles.insert(key.into(), style);
    }

    /// The style for `key`, or the theme's fallback if unset.
    #[must_use]
    pub fn style(&self, key: &str) -> Style {
        self.styles.get(key).copied().unwrap_or(self.fallback)
    }

    /// The fallback style used for unknown keys.
    #[must_use]
    pub const fn fallback(&self) -> Style {
        self.fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_key_resolves() {
        let t = Theme::default();
        assert!(t.style("panel.title").attrs.contains(Attrs::BOLD));
    }

    #[test]
    fn unknown_key_falls_back() {
        let t = Theme::blank(Style::fg(Color::Ansi16(1)));
        assert_eq!(t.style("nope"), Style::fg(Color::Ansi16(1)));
    }

    #[test]
    fn mono_has_no_rgb() {
        let t = Theme::mono();
        // mono selected style uses attributes, never RGB colors.
        assert_eq!(t.style("list.selected").fg, Color::Default);
    }
}
