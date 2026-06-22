//! The diffed [`Presenter`]: turn a [`CellBuffer`] frame into the minimal byte
//! stream that updates the terminal from the previously-shown frame.
//!
//! This is the generalisation of Command_Line_3D's single `WriteConsoleOutput`
//! to ANSI terminals, and the antidote to Ymael's per-cell full repaint
//! (measured ~50× more bytes — see `RiftEngine-Plan/06-phase-1-tui-core.md` §1.1).
//!
//! The algorithm, per row:
//!
//! - compare the incoming frame against the [`Presenter`]'s record of what is
//!   currently displayed, cell by cell;
//! - emit a cursor-move (`CUP`) only when the next changed cell is not where the
//!   cursor already sits — so a run of changed cells costs one move, not one per
//!   cell;
//! - emit only the SGR parameters that actually changed since the last styled
//!   cell (a small state machine, not a reset-per-cell);
//! - track wide-glyph occupancy so a double-width glyph (CJK, emoji) consumes
//!   two columns and its shadow cell is never independently drawn.
//!
//! Colors are resolved to the terminal's [`ColorDepth`] *here, at present time*,
//! so the rest of the engine authors at full fidelity (architecture §3).

use std::io::{self, Write};

use unicode_width::UnicodeWidthChar;
use xre_core::math::UVec2;
use xre_core::{Attrs, Cell, CellBuffer, Color, ColorDepth};

use crate::capabilities::Capabilities;
use crate::error::Result;

/// The terminal's SGR state as the presenter believes it to be.
///
/// `None` for the colors would mean "unknown"; we instead always track a
/// concrete value and force a leading reset on a full redraw, which keeps the
/// diffing branch-free.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Style {
    fg: Color,
    bg: Color,
    attrs: Attrs,
}

impl Style {
    const DEFAULT: Self = Self {
        fg: Color::Default,
        bg: Color::Default,
        attrs: Attrs::NONE,
    };
}

/// Display width of a glyph in terminal columns (0-width glyphs are treated as
/// 1 so the cursor model never stalls).
fn glyph_width(ch: char) -> u32 {
    match UnicodeWidthChar::width(ch) {
        Some(0) | None => 1,
        Some(w) => w as u32,
    }
}

/// A diffed terminal presenter writing to an arbitrary [`Write`] sink.
///
/// Construct one with [`Presenter::new`] (writing to stdout via
/// [`Presenter::stdout`]); call [`Presenter::present`] once per frame. The
/// presenter keeps a private record of what is on screen and never re-allocates
/// it after warmup (Gate G1's zero-per-frame-allocation invariant).
pub struct Presenter<W: Write> {
    out: W,
    /// What is currently on the terminal (post-resolve glyphs/colors).
    displayed: CellBuffer,
    /// Color depth used to resolve cell colors before emitting.
    depth: ColorDepth,
    /// Whether the terminal honours synchronized output (DEC 2026).
    synchronized: bool,
    /// SGR state as last emitted.
    style: Style,
    /// Force a clear + redraw on the next present (first frame / post-resize).
    force_redraw: bool,
    /// Reusable scratch byte buffer; flushed to `out` once per frame.
    scratch: Vec<u8>,
}

impl Presenter<io::Stdout> {
    /// A presenter that writes to standard output, sized to `caps`.
    #[must_use]
    pub fn stdout(caps: &Capabilities) -> Self {
        Self::new(io::stdout(), caps)
    }
}

impl<W: Write> Presenter<W> {
    /// Create a presenter writing to `out`, sized and configured from `caps`.
    #[must_use]
    pub fn new(out: W, caps: &Capabilities) -> Self {
        Self {
            out,
            displayed: CellBuffer::new(caps.size),
            depth: caps.color,
            synchronized: caps.synchronized_output,
            style: Style::DEFAULT,
            force_redraw: true,
            scratch: Vec::with_capacity(8 * 1024),
        }
    }

    /// The size of the displayed buffer in cells.
    #[must_use]
    pub const fn size(&self) -> UVec2 {
        self.displayed.size()
    }

    /// React to a terminal resize: reallocate the displayed buffer and force a
    /// full redraw on the next [`Presenter::present`].
    pub fn resize(&mut self, size: UVec2) {
        if size != self.displayed.size() {
            self.displayed.resize(size);
        }
        self.force_redraw = true;
    }

    /// Force the next [`Presenter::present`] to repaint every cell (e.g. after
    /// the terminal was written to by foreign code).
    pub const fn invalidate(&mut self) {
        self.force_redraw = true;
    }

    /// The [`ColorDepth`] colors are resolved to before emission.
    #[must_use]
    pub const fn color_depth(&self) -> ColorDepth {
        self.depth
    }

    /// Override the [`ColorDepth`] colors are resolved to at present time.
    ///
    /// Capping a truecolor terminal to [`ColorDepth::Ansi256`] makes each color
    /// SGR shorter (`38;5;N` vs `38;2;R;G;B`) **and** collapses near-identical
    /// colors onto the 256-entry palette, so adjacent cells coalesce and the
    /// diffed stream shrinks dramatically — the main lever for a dense, fully
    /// repainted truecolor 3D viewport that is terminal-I/O-bound. Forces a full
    /// redraw so the on-screen record is re-resolved at the new depth.
    pub const fn set_color_depth(&mut self, depth: ColorDepth) {
        self.depth = depth;
        self.force_redraw = true;
    }

    /// Diff `frame` against what is on screen and flush the minimal update.
    ///
    /// `frame` must match the presenter's [`Presenter::size`]; if it does not
    /// (a resize the caller has not yet reported), the presenter adopts the new
    /// size and repaints fully.
    ///
    /// # Errors
    /// Returns [`crate::TermError::Io`] if writing to the sink fails.
    pub fn present(&mut self, frame: &CellBuffer) -> Result<()> {
        if frame.size() != self.displayed.size() {
            self.displayed.resize(frame.size());
            self.force_redraw = true;
        }
        self.scratch.clear();
        if self.synchronized {
            self.scratch.extend_from_slice(b"\x1b[?2026h");
        }
        if self.force_redraw {
            // Reset SGR, clear the screen, home the cursor.
            self.scratch.extend_from_slice(b"\x1b[0m\x1b[2J\x1b[H");
            self.style = Style::DEFAULT;
        }

        // `cursor` is the column/row the terminal cursor sits at, or None when
        // its position is unknown and a move must be forced.
        let mut cursor: Option<(u32, u32)> = if self.force_redraw {
            Some((0, 0))
        } else {
            None
        };
        let width = frame.width();
        for y in 0..frame.height() {
            let mut x = 0;
            while x < width {
                let new_cell = frame.get(x, y).copied().unwrap_or_default();
                let old_cell = self.displayed.get(x, y).copied().unwrap_or_default();
                if !self.force_redraw && new_cell == old_cell {
                    x += 1;
                    continue;
                }
                self.emit_cell_at(&mut cursor, x, y, new_cell);
                if let Some(c) = self.displayed.get_mut(x, y) {
                    *c = new_cell;
                }
                let w = glyph_width(new_cell.glyph).min(width - x);
                // Columns covered by a wide glyph's shadow: record them as drawn.
                for shadow in 1..w {
                    if let Some(c) = self.displayed.get_mut(x + shadow, y) {
                        *c = frame.get(x + shadow, y).copied().unwrap_or_default();
                    }
                }
                x += w.max(1);
            }
        }

        if self.synchronized {
            self.scratch.extend_from_slice(b"\x1b[?2026l");
        }
        self.force_redraw = false;
        self.out.write_all(&self.scratch)?;
        self.out.flush()?;
        Ok(())
    }

    /// Emit one cell at `(x, y)`: a cursor-move if needed, an SGR diff, the glyph.
    fn emit_cell_at(&mut self, cursor: &mut Option<(u32, u32)>, x: u32, y: u32, cell: Cell) {
        if *cursor != Some((x, y)) {
            // CUP is 1-based: row;colH.
            push_csi_uu(&mut self.scratch, y + 1, x + 1, b'H');
        }
        let desired = Style {
            fg: cell.fg.resolve(self.depth),
            bg: cell.bg.resolve(self.depth),
            attrs: cell.attrs,
        };
        emit_sgr(&mut self.scratch, self.style, desired);
        self.style = desired;
        push_char(&mut self.scratch, cell.glyph);
        let w = glyph_width(cell.glyph);
        *cursor = Some((x + w, y));
    }

    /// Consume the presenter and return the underlying sink.
    pub fn into_inner(self) -> W {
        self.out
    }
}

/// Append a char's UTF-8 bytes to `out`.
fn push_char(out: &mut Vec<u8>, ch: char) {
    let mut buf = [0u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
}

/// Append `CSI <a>;<b><final>` with two decimal parameters.
fn push_csi_uu(out: &mut Vec<u8>, a: u32, b: u32, final_byte: u8) {
    out.extend_from_slice(b"\x1b[");
    push_u32(out, a);
    out.push(b';');
    push_u32(out, b);
    out.push(final_byte);
}

/// Append a `u32` as decimal ASCII without allocating.
fn push_u32(out: &mut Vec<u8>, mut v: u32) {
    if v == 0 {
        out.push(b'0');
        return;
    }
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    while v > 0 {
        i -= 1;
        digits[i] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    out.extend_from_slice(&digits[i..]);
}

/// Append one SGR parameter directly to `out`, opening the `CSI` (`\x1b[`) on the
/// first param and inserting `;` separators thereafter. Tracking `started` lets us
/// emit straight into the reusable scratch buffer with **no per-cell allocation**
/// (the hot path repaints thousands of cells per frame — Gate G1's zero-alloc
/// invariant). The byte output is identical to the previous `Vec<u16>`-join.
#[inline]
fn push_param(out: &mut Vec<u8>, started: &mut bool, p: u16) {
    if *started {
        out.push(b';');
    } else {
        out.extend_from_slice(b"\x1b[");
        *started = true;
    }
    push_u32(out, u32::from(p));
}

/// Emit the minimal SGR sequence transitioning the terminal from `current` to
/// `desired`. Only the parameters that changed are written, straight into `out`.
fn emit_sgr(out: &mut Vec<u8>, current: Style, desired: Style) {
    if current == desired {
        return;
    }
    let mut started = false;

    // Attribute changes, set then reset.
    let add = |flag: Attrs| desired.attrs.contains(flag) && !current.attrs.contains(flag);
    let remove = |flag: Attrs| current.attrs.contains(flag) && !desired.attrs.contains(flag);
    if add(Attrs::BOLD) {
        push_param(out, &mut started, 1);
    }
    if add(Attrs::DIM) {
        push_param(out, &mut started, 2);
    }
    if add(Attrs::ITALIC) {
        push_param(out, &mut started, 3);
    }
    if add(Attrs::UNDERLINE) {
        push_param(out, &mut started, 4);
    }
    // Bold and dim share the 22 reset; emit it if either is being cleared.
    if remove(Attrs::BOLD) || remove(Attrs::DIM) {
        push_param(out, &mut started, 22);
        // Re-assert the one that should remain on.
        if desired.attrs.contains(Attrs::BOLD) {
            push_param(out, &mut started, 1);
        }
        if desired.attrs.contains(Attrs::DIM) {
            push_param(out, &mut started, 2);
        }
    }
    if remove(Attrs::ITALIC) {
        push_param(out, &mut started, 23);
    }
    if remove(Attrs::UNDERLINE) {
        push_param(out, &mut started, 24);
    }

    if current.fg != desired.fg {
        push_color_params(out, &mut started, desired.fg, false);
    }
    if current.bg != desired.bg {
        push_color_params(out, &mut started, desired.bg, true);
    }

    // `current != desired` always pushes at least one param, so `started` is set;
    // close the sequence. (If nothing was pushed we emit nothing, as before.)
    if started {
        out.push(b'm');
    }
}

/// Push the SGR parameters that select `color` for foreground (`is_bg=false`)
/// or background (`is_bg=true`) directly into `out` via [`push_param`].
fn push_color_params(out: &mut Vec<u8>, started: &mut bool, color: Color, is_bg: bool) {
    match color {
        Color::Default => push_param(out, started, if is_bg { 49 } else { 39 }),
        Color::Ansi16(i) => {
            let i = u16::from(i & 0x0F);
            let base = if is_bg { 40 } else { 30 };
            let bright_base = if is_bg { 100 } else { 90 };
            if i < 8 {
                push_param(out, started, base + i);
            } else {
                push_param(out, started, bright_base + (i - 8));
            }
        }
        Color::Ansi256(i) => {
            push_param(out, started, if is_bg { 48 } else { 38 });
            push_param(out, started, 5);
            push_param(out, started, u16::from(i));
        }
        Color::Rgb(r, g, b) => {
            push_param(out, started, if is_bg { 48 } else { 38 });
            push_param(out, started, 2);
            push_param(out, started, u16::from(r));
            push_param(out, started, u16::from(g));
            push_param(out, started, u16::from(b));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    /// A throwaway VT parser: applies the presenter's byte stream to a grid and
    /// cursor/style, so a test can assert "what the terminal would show" equals
    /// the intended frame. Handles exactly the subset the presenter emits: CUP
    /// (`H`), SGR (`m`), DEC private mode set/reset (`h`/`l`, ignored), ED (`2J`),
    /// and printable text.
    struct VtScreen {
        w: u32,
        h: u32,
        cells: Vec<(char, Style)>,
        cx: u32,
        cy: u32,
        style: Style,
    }

    impl VtScreen {
        fn new(w: u32, h: u32) -> Self {
            Self {
                w,
                h,
                cells: vec![(' ', Style::DEFAULT); (w * h) as usize],
                cx: 0,
                cy: 0,
                style: Style::DEFAULT,
            }
        }

        fn feed(&mut self, bytes: &[u8]) {
            let s = std::str::from_utf8(bytes).expect("presenter emits valid utf-8");
            let mut chars = s.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    assert_eq!(chars.next(), Some('['), "only CSI sequences expected");
                    let mut private = false;
                    if chars.peek() == Some(&'?') {
                        private = true;
                        chars.next();
                    }
                    let mut params = String::new();
                    let final_byte = loop {
                        let c = chars.next().expect("unterminated CSI");
                        if c.is_ascii_alphabetic() {
                            break c;
                        }
                        params.push(c);
                    };
                    self.apply_csi(private, &params, final_byte);
                } else {
                    self.print(c);
                }
            }
        }

        fn apply_csi(&mut self, private: bool, params: &str, final_byte: char) {
            if private {
                return; // ?2026h / ?2026l — synchronized output, irrelevant to state.
            }
            let nums: Vec<u32> = params.split(';').map(|p| p.parse().unwrap_or(0)).collect();
            match final_byte {
                'H' => {
                    let row = nums.first().copied().unwrap_or(1).max(1);
                    let col = nums.get(1).copied().unwrap_or(1).max(1);
                    self.cy = row - 1;
                    self.cx = col - 1;
                }
                'J' => {
                    if nums.first().copied().unwrap_or(0) == 2 {
                        self.cells.fill((' ', Style::DEFAULT));
                    }
                }
                'm' => self.apply_sgr(&nums),
                other => panic!("unexpected CSI final byte {other:?}"),
            }
        }

        fn apply_sgr(&mut self, nums: &[u32]) {
            let mut i = 0;
            while i < nums.len() {
                match nums[i] {
                    0 => self.style = Style::DEFAULT,
                    1 => self.style.attrs = self.style.attrs.with(Attrs::BOLD),
                    2 => self.style.attrs = self.style.attrs.with(Attrs::DIM),
                    3 => self.style.attrs = self.style.attrs.with(Attrs::ITALIC),
                    4 => self.style.attrs = self.style.attrs.with(Attrs::UNDERLINE),
                    22 => {
                        self.style.attrs =
                            self.style.attrs.without(Attrs::BOLD).without(Attrs::DIM);
                    }
                    23 => self.style.attrs = self.style.attrs.without(Attrs::ITALIC),
                    24 => self.style.attrs = self.style.attrs.without(Attrs::UNDERLINE),
                    39 => self.style.fg = Color::Default,
                    49 => self.style.bg = Color::Default,
                    38 | 48 => {
                        let is_bg = nums[i] == 48;
                        let kind = nums[i + 1];
                        let color = if kind == 5 {
                            let c = Color::Ansi256(nums[i + 2] as u8);
                            i += 2;
                            c
                        } else {
                            let c =
                                Color::Rgb(nums[i + 2] as u8, nums[i + 3] as u8, nums[i + 4] as u8);
                            i += 4;
                            c
                        };
                        if is_bg {
                            self.style.bg = color;
                        } else {
                            self.style.fg = color;
                        }
                    }
                    n @ 30..=37 => self.style.fg = Color::Ansi16((n - 30) as u8),
                    n @ 40..=47 => self.style.bg = Color::Ansi16((n - 40) as u8),
                    n @ 90..=97 => self.style.fg = Color::Ansi16((n - 90 + 8) as u8),
                    n @ 100..=107 => self.style.bg = Color::Ansi16((n - 100 + 8) as u8),
                    other => panic!("unexpected SGR param {other}"),
                }
                i += 1;
            }
        }

        fn print(&mut self, c: char) {
            if self.cx < self.w && self.cy < self.h {
                let idx = (self.cy * self.w + self.cx) as usize;
                self.cells[idx] = (c, self.style);
            }
            self.cx += glyph_width(c);
        }

        /// Assert the visible grid matches `frame`, resolving `frame`'s colors to
        /// `depth` exactly as the presenter would. Wide-glyph shadow cells are
        /// skipped.
        fn assert_matches(&self, frame: &CellBuffer, depth: ColorDepth) {
            let mut x = 0;
            for y in 0..frame.height() {
                x = 0;
                while x < frame.width() {
                    let cell = *frame.get(x, y).unwrap();
                    let (gc, gstyle) = self.cells[(y * self.w + x) as usize];
                    assert_eq!(gc, cell.glyph, "glyph mismatch at ({x},{y})");
                    assert_eq!(
                        gstyle.fg,
                        cell.fg.resolve(depth),
                        "fg mismatch at ({x},{y})"
                    );
                    assert_eq!(
                        gstyle.bg,
                        cell.bg.resolve(depth),
                        "bg mismatch at ({x},{y})"
                    );
                    assert_eq!(gstyle.attrs, cell.attrs, "attrs mismatch at ({x},{y})");
                    x += glyph_width(cell.glyph).max(1);
                }
            }
            let _ = x;
        }
    }

    fn caps(size: UVec2, depth: ColorDepth) -> Capabilities {
        Capabilities {
            color: depth,
            unicode: crate::UnicodeLevel::Full,
            size,
            synchronized_output: false,
            mouse: false,
        }
    }

    #[test]
    fn full_redraw_reproduces_frame() {
        let size = UVec2::new(6, 3);
        let c = caps(size, ColorDepth::TrueColor);
        let mut frame = CellBuffer::new(size);
        frame.set(0, 0, Cell::new('H').fg(Color::Rgb(200, 10, 10)));
        frame.set(1, 0, Cell::new('i').attrs(Attrs::BOLD));
        frame.set(2, 1, Cell::new('X').bg(Color::Ansi256(21)));
        let mut p = Presenter::new(Vec::new(), &c);
        p.present(&frame).unwrap();
        let mut vt = VtScreen::new(size.x, size.y);
        vt.feed(&p.into_inner());
        vt.assert_matches(&frame, ColorDepth::TrueColor);
    }

    #[test]
    fn incremental_diff_reproduces_frame() {
        let size = UVec2::new(8, 4);
        let c = caps(size, ColorDepth::Ansi256);
        let mut frame = CellBuffer::new(size);
        frame.fill(Cell::new('.'));
        let mut p = Presenter::new(Vec::new(), &c);
        p.present(&frame).unwrap();
        // Change a handful of cells and present again; replay both frames.
        frame.set(3, 2, Cell::new('@').fg(Color::Rgb(0, 255, 0)));
        frame.set(4, 2, Cell::new('#').attrs(Attrs::UNDERLINE));
        let bytes = {
            let mut p2 = p;
            p2.present(&frame).unwrap();
            p2.into_inner()
        };
        // The incremental stream must NOT contain a full clear.
        assert!(
            !bytes.windows(3).any(|w| w == b"\x1b[2J"),
            "second present should not clear the screen"
        );
        // Replay the whole conversation onto a fresh screen.
        let mut p = Presenter::new(Vec::new(), &c);
        let mut full = CellBuffer::new(size);
        full.fill(Cell::new('.'));
        p.present(&full).unwrap();
        let mut all = p.into_inner();
        all.extend_from_slice(&bytes);
        let mut vt = VtScreen::new(size.x, size.y);
        vt.feed(&all);
        vt.assert_matches(&frame, ColorDepth::Ansi256);
    }

    #[test]
    fn unchanged_frame_emits_no_drawing() {
        let size = UVec2::new(5, 2);
        let c = caps(size, ColorDepth::TrueColor);
        let mut frame = CellBuffer::new(size);
        frame.set(1, 1, Cell::new('q'));

        // The sink accumulates across presents, so a re-present of the same
        // frame must add zero bytes: one present and two presents are equal.
        let one = {
            let mut p = Presenter::new(Vec::new(), &c);
            p.present(&frame).unwrap();
            p.into_inner()
        };
        let two = {
            let mut p = Presenter::new(Vec::new(), &c);
            p.present(&frame).unwrap();
            p.present(&frame).unwrap();
            p.into_inner()
        };
        assert_eq!(one, two, "an identical second present must emit nothing");
    }

    #[test]
    fn wide_glyph_consumes_two_columns() {
        let size = UVec2::new(6, 1);
        let c = caps(size, ColorDepth::TrueColor);
        let mut frame = CellBuffer::new(size);
        frame.set(0, 0, Cell::new('世'));
        frame.set(2, 0, Cell::new('a'));
        let mut p = Presenter::new(Vec::new(), &c);
        p.present(&frame).unwrap();
        let mut vt = VtScreen::new(size.x, size.y);
        vt.feed(&p.into_inner());
        assert_eq!(vt.cells[0].0, '世');
        assert_eq!(vt.cells[2].0, 'a');
    }
}
