//! [`Log`]: a bounded ring buffer of lines with follow mode.

use std::collections::VecDeque;

use xre_core::{Rect, Style};

use crate::frame::Frame;
use crate::widget::Widget;
use crate::widgets::Text;

/// A scrolling log: a capped ring buffer of lines, rendered newest-at-bottom.
///
/// In *follow* mode (the default) the view sticks to the latest lines; scrolling
/// up turns follow off until the user scrolls back to the bottom.
#[derive(Clone, Debug)]
pub struct Log {
    lines: VecDeque<String>,
    capacity: usize,
    follow: bool,
    scroll: usize,
    style: Style,
}

impl Log {
    /// A log holding at most `capacity` lines.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            lines: VecDeque::with_capacity(capacity.min(1024)),
            capacity: capacity.max(1),
            follow: true,
            scroll: 0,
            style: Style::DEFAULT,
        }
    }

    /// Builder: set the line style.
    #[must_use]
    pub const fn style(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Append a line, evicting the oldest if at capacity.
    pub fn push(&mut self, line: impl Into<String>) {
        if self.lines.len() == self.capacity {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
        if self.follow {
            self.scroll = 0;
        }
    }

    /// Number of stored lines.
    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    /// `true` if no lines are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Scroll up by `n` lines (disables follow).
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = (self.scroll + n).min(self.lines.len().saturating_sub(1));
        self.follow = false;
    }

    /// Scroll down by `n` lines; re-enables follow when the bottom is reached.
    pub const fn scroll_down(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
        if self.scroll == 0 {
            self.follow = true;
        }
    }
}

impl Widget for Log {
    fn render(&self, area: Rect, frame: &mut Frame) {
        if area.is_empty() {
            return;
        }
        let height = area.height() as usize;
        // Bottom line index (exclusive) after applying scroll-back.
        let end = self.lines.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(height);
        let mut f = frame.region(area);
        for (i, line) in self.lines.iter().skip(start).take(end - start).enumerate() {
            let y = area.top() + i as u32;
            Text::styled(line.clone(), self.style)
                .render_into(Rect::new(area.left(), y, area.width(), 1), &mut f);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use xre_core::math::UVec2;
    use xre_core::CellBuffer;

    fn rows(buf: &CellBuffer) -> Vec<String> {
        (0..buf.height())
            .map(|y| {
                (0..buf.width())
                    .map(|x| buf.get(x, y).unwrap().glyph)
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn ring_buffer_evicts_oldest() {
        let mut log = Log::new(2);
        log.push("a");
        log.push("b");
        log.push("c");
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn follow_shows_latest_at_bottom() {
        let mut log = Log::new(10);
        for i in 0..5 {
            log.push(format!("line{i}"));
        }
        let mut buf = CellBuffer::new(UVec2::new(8, 2));
        {
            let mut f = Frame::root(&mut buf);
            log.render(Rect::new(0, 0, 8, 2), &mut f);
        }
        let r = rows(&buf);
        assert_eq!(r, vec!["line3".to_string(), "line4".to_string()]);
    }

    #[test]
    fn scroll_up_then_down_restores_follow() {
        let mut log = Log::new(10);
        for i in 0..5 {
            log.push(format!("l{i}"));
        }
        log.scroll_up(2);
        let mut buf = CellBuffer::new(UVec2::new(4, 2));
        {
            let mut f = Frame::root(&mut buf);
            log.render(Rect::new(0, 0, 4, 2), &mut f);
        }
        assert_eq!(rows(&buf), vec!["l1".to_string(), "l2".to_string()]);
        log.scroll_down(5);
        let mut buf2 = CellBuffer::new(UVec2::new(4, 2));
        {
            let mut f = Frame::root(&mut buf2);
            log.render(Rect::new(0, 0, 4, 2), &mut f);
        }
        assert_eq!(rows(&buf2), vec!["l3".to_string(), "l4".to_string()]);
    }
}
