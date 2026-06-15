//! [`Input`]: a single-line text editor with a cursor and command history.

use xre_core::{Rect, Style};
use xre_term::{Key, KeyCode};

use crate::frame::Frame;
use crate::widget::Widget;

/// A single-line editable text field.
///
/// State (buffer, cursor, history) lives in the value; drive it by feeding
/// [`Key`]s to [`Input::handle_key`], which returns `Some(line)` when Enter
/// commits the current text. Render with the [`Widget`] impl (which shows the
/// cursor when [`Input::focused`] is set).
#[derive(Clone, Debug, Default)]
pub struct Input {
    buffer: Vec<char>,
    cursor: usize,
    history: Vec<String>,
    history_pos: Option<usize>,
    focused: bool,
    text_style: Style,
    cursor_style: Style,
}

impl Input {
    /// An empty input field.
    #[must_use]
    pub fn new() -> Self {
        Self {
            cursor_style: Style::DEFAULT.with_attrs(xre_core::Attrs::UNDERLINE),
            ..Self::default()
        }
    }

    /// Builder: set whether the cursor is drawn (focused).
    #[must_use]
    pub const fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Set focus (the cursor is only drawn when focused).
    pub const fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Builder: set the text style.
    #[must_use]
    pub const fn text_style(mut self, style: Style) -> Self {
        self.text_style = style;
        self
    }

    /// Builder: set the cursor style.
    #[must_use]
    pub const fn cursor_style(mut self, style: Style) -> Self {
        self.cursor_style = style;
        self
    }

    /// The current text.
    #[must_use]
    pub fn value(&self) -> String {
        self.buffer.iter().collect()
    }

    /// `true` if the buffer is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Replace the buffer with `text` and move the cursor to the end.
    pub fn set_value(&mut self, text: &str) {
        self.buffer = text.chars().collect();
        self.cursor = self.buffer.len();
    }

    /// Clear the buffer.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
        self.history_pos = None;
    }

    /// Feed a key. Returns `Some(line)` when Enter commits a non-handled-as-edit
    /// line (also pushed onto history); `None` otherwise.
    pub fn handle_key(&mut self, key: Key) -> Option<String> {
        match key.code {
            KeyCode::Char(c) => {
                self.buffer.insert(self.cursor, c);
                self.cursor += 1;
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    self.cursor -= 1;
                    self.buffer.remove(self.cursor);
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.buffer.len() {
                    self.buffer.remove(self.cursor);
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.buffer.len()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.buffer.len(),
            KeyCode::Up => self.history_step(-1),
            KeyCode::Down => self.history_step(1),
            KeyCode::Enter => {
                let line = self.value();
                if !line.is_empty() {
                    self.history.push(line.clone());
                }
                self.clear();
                return Some(line);
            }
            _ => {}
        }
        None
    }

    /// Navigate history by `delta` (−1 = older, +1 = newer).
    fn history_step(&mut self, delta: i32) {
        if self.history.is_empty() {
            return;
        }
        let new_pos = match (self.history_pos, delta) {
            (None, -1) => Some(self.history.len() - 1),
            (Some(p), -1) => Some(p.saturating_sub(1)),
            (Some(p), 1) if p + 1 < self.history.len() => Some(p + 1),
            (Some(_), 1) => None,
            _ => self.history_pos,
        };
        self.history_pos = new_pos;
        match new_pos {
            Some(p) => self.set_value(&self.history[p].clone()),
            None => self.clear(),
        }
    }

    /// The visible window of the buffer for `width` cells, scrolled to keep the
    /// cursor in view, returned as `(text, cursor_col)`.
    fn view(&self, width: u32) -> (String, u32) {
        if width == 0 {
            return (String::new(), 0);
        }
        let w = width as usize;
        // Reserve a column for the cursor at end-of-line.
        let start = self.cursor.saturating_sub(w.saturating_sub(1));
        let end = (start + w).min(self.buffer.len());
        let text: String = self.buffer[start..end].iter().collect();
        ((text), (self.cursor - start) as u32)
    }
}

impl Widget for Input {
    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let mut f = frame.region(area);
        let y = area.top();
        let (text, cursor_col) = self.view(area.width());
        f.print(area.left(), y, &text, self.text_style);
        if self.focused {
            let cx = area.left() + cursor_col;
            // Underline the cell at the cursor (or a space past the text end).
            let glyph = text.chars().nth(cursor_col as usize).unwrap_or(' ');
            f.set(cx, y, self.cursor_style.cell(glyph));
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::CellBuffer;
    use xre_term::Modifiers;

    fn key(code: KeyCode) -> Key {
        Key {
            code,
            mods: Modifiers::NONE,
        }
    }

    #[test]
    fn typing_and_editing() {
        let mut input = Input::new();
        for c in "helo".chars() {
            input.handle_key(key(KeyCode::Char(c)));
        }
        // Move left once and insert 'l' to fix "helo" → "hello".
        input.handle_key(key(KeyCode::Left));
        input.handle_key(key(KeyCode::Char('l')));
        assert_eq!(input.value(), "hello");
        input.handle_key(key(KeyCode::Backspace));
        assert_eq!(input.value(), "helo");
    }

    #[test]
    fn enter_commits_and_clears() {
        let mut input = Input::new();
        input.set_value("run");
        let out = input.handle_key(key(KeyCode::Enter));
        assert_eq!(out.as_deref(), Some("run"));
        assert!(input.is_empty());
        // History recall.
        input.handle_key(key(KeyCode::Up));
        assert_eq!(input.value(), "run");
    }

    #[test]
    fn renders_with_cursor_when_focused() {
        let mut input = Input::new().focused(true);
        input.set_value("hi");
        let mut buf = CellBuffer::new(UVec2::new(5, 1));
        {
            let mut f = Frame::root(&mut buf);
            input.render(Rect::new(0, 0, 5, 1), &mut f);
        }
        assert_eq!(buf.get(0, 0).unwrap().glyph, 'h');
        // Cursor sits just past "hi" at column 2, underlined.
        assert!(buf
            .get(2, 0)
            .unwrap()
            .attrs
            .contains(xre_core::Attrs::UNDERLINE));
    }
}
