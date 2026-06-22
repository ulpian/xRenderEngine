//! [`MouseRouter`]: hit-testing for an immediate-mode UI.
//!
//! There is no retained widget tree, so the router mirrors [`crate::FocusManager`]:
//! widgets re-register their hit regions every frame during render, and the
//! router resolves a [`MouseEvent`] to the [`FocusId`] under the cursor. Because
//! render happens after input, an application routes *this* frame's events
//! against the regions registered *last* frame — a one-frame latency that is
//! standard for immediate-mode UIs and invisible at interactive frame rates.
//!
//! The per-frame pattern:
//!
//! ```ignore
//! // input phase — route against last frame's regions
//! for ev in events.drain() {
//!     if let Event::Mouse(m) = ev {
//!         if let Some(id) = router.route(&m) {
//!             if matches!(m.kind, MouseKind::Down(_)) { focus.focus(id); }
//!             dispatch(id, &m); // app maps id -> widget + area, calls handle_mouse
//!         }
//!     }
//! }
//! // render phase — rebuild regions in paint order
//! router.begin_frame();
//! router.register(LIST_ID, list_rect);
//! list.render_stateful(list_rect, &mut frame, &mut list_state);
//! ```

use xre_core::math::UVec2;
use xre_core::Rect;
use xre_term::{MouseEvent, MouseKind};

use crate::focus::FocusId;

/// A per-frame mouse hit-test registry.
///
/// Cleared and rebuilt every render via [`MouseRouter::begin_frame`] +
/// [`MouseRouter::register`]; the backing storage keeps its capacity so steady
/// state allocates nothing (the zero-alloc-per-frame invariant). Drag *capture*
/// survives the rebuild so a press keeps routing to the same widget until release.
#[derive(Clone, Debug, Default)]
pub struct MouseRouter {
    regions: Vec<(FocusId, Rect)>,
    capture: Option<FocusId>,
}

impl MouseRouter {
    /// An empty router.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Clear the registered regions at the start of a render pass, retaining the
    /// backing capacity. Drag capture is preserved.
    pub fn begin_frame(&mut self) {
        self.regions.clear();
    }

    /// Register `rect` as the hit region for `id`. Call during render in paint
    /// order: later registrations sit on top and win overlapping hit-tests.
    pub fn register(&mut self, id: FocusId, rect: Rect) {
        self.regions.push((id, rect));
    }

    /// The id of the topmost region containing cell `(col, row)`, if any.
    #[must_use]
    pub fn hit(&self, col: u32, row: u32) -> Option<FocusId> {
        let p = UVec2::new(col, row);
        self.regions
            .iter()
            .rev()
            .find(|(_, rect)| rect.contains(p))
            .map(|(id, _)| *id)
    }

    /// Resolve a mouse event to its target id, honouring drag capture.
    ///
    /// - `Down` hit-tests under the cursor and captures the target so subsequent
    ///   drags keep reaching it (e.g. a 1-cell scrollbar thumb).
    /// - `Drag` routes to the captured target (falling back to a hit-test).
    /// - `Up` routes to the captured target, then releases capture.
    /// - `Moved`/`ScrollUp`/`ScrollDown` plainly hit-test under the cursor, so
    ///   scrolling targets whatever is beneath the pointer, not the focused widget.
    #[must_use]
    pub fn route(&mut self, ev: &MouseEvent) -> Option<FocusId> {
        match ev.kind {
            MouseKind::Down(_) => {
                let id = self.hit(ev.col, ev.row);
                self.capture = id;
                id
            }
            MouseKind::Drag(_) => self.capture.or_else(|| self.hit(ev.col, ev.row)),
            MouseKind::Up(_) => self.capture.take().or_else(|| self.hit(ev.col, ev.row)),
            _ => self.hit(ev.col, ev.row),
        }
    }

    /// The id currently holding drag capture, if any.
    #[must_use]
    pub const fn captured(&self) -> Option<FocusId> {
        self.capture
    }
}

/// A pointer gesture over a 3D viewport, ready to drive a camera controller.
///
/// [`crate::Viewport3D`] is a read-only presenter and owns no camera, so an app
/// translates mouse input into one of these and applies it to its own controller
/// (e.g. `xre_cello::OrbitController`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ViewportGesture {
    /// Orbit by the given yaw/pitch deltas in radians.
    Orbit {
        /// Yaw delta (radians); positive for rightward drag.
        dyaw: f32,
        /// Pitch delta (radians); positive for upward drag.
        dpitch: f32,
    },
    /// Zoom by the given factor (negative = in, positive = out).
    Zoom(f32),
    /// No camera-affecting gesture.
    None,
}

/// Translate a mouse event over a viewport into a [`ViewportGesture`].
///
/// `prev` is the previous drag cell (the app tracks it across events); `sens` is
/// radians-per-cell for orbiting. Drags without a previous cell, and all
/// non-drag/non-scroll events, yield [`ViewportGesture::None`].
#[must_use]
pub fn viewport_gesture(ev: &MouseEvent, prev: Option<(u32, u32)>, sens: f32) -> ViewportGesture {
    match ev.kind {
        MouseKind::Drag(_) => {
            if let Some((px, py)) = prev {
                let dx = ev.col as f32 - px as f32;
                let dy = ev.row as f32 - py as f32;
                ViewportGesture::Orbit {
                    dyaw: dx * sens,
                    dpitch: -dy * sens,
                }
            } else {
                ViewportGesture::None
            }
        }
        MouseKind::ScrollUp => ViewportGesture::Zoom(-1.0),
        MouseKind::ScrollDown => ViewportGesture::Zoom(1.0),
        _ => ViewportGesture::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xre_term::{Modifiers, MouseButton};

    fn ev(kind: MouseKind, col: u32, row: u32) -> MouseEvent {
        MouseEvent {
            kind,
            col,
            row,
            mods: Modifiers::NONE,
        }
    }

    #[test]
    fn topmost_region_wins() {
        let mut r = MouseRouter::new();
        r.register(FocusId(1), Rect::new(0, 0, 10, 10));
        r.register(FocusId(2), Rect::new(2, 2, 4, 4)); // drawn on top
        assert_eq!(r.hit(3, 3), Some(FocusId(2)));
        assert_eq!(r.hit(8, 8), Some(FocusId(1)));
        assert_eq!(r.hit(20, 20), None);
    }

    #[test]
    fn drag_capture_persists_off_region() {
        let mut r = MouseRouter::new();
        r.register(FocusId(7), Rect::new(0, 0, 2, 8)); // a narrow scrollbar
                                                       // Press inside captures it.
        assert_eq!(
            r.route(&ev(MouseKind::Down(MouseButton::Left), 1, 1)),
            Some(FocusId(7))
        );
        // Drag off the region still routes to the captured id.
        assert_eq!(
            r.route(&ev(MouseKind::Drag(MouseButton::Left), 50, 4)),
            Some(FocusId(7))
        );
        // Release routes to it, then clears capture.
        assert_eq!(
            r.route(&ev(MouseKind::Up(MouseButton::Left), 50, 4)),
            Some(FocusId(7))
        );
        assert_eq!(r.captured(), None);
    }

    #[test]
    fn scroll_hit_tests_under_cursor() {
        let mut r = MouseRouter::new();
        r.register(FocusId(1), Rect::new(0, 0, 4, 4));
        r.register(FocusId(2), Rect::new(10, 0, 4, 4));
        // No capture: scroll targets whatever is under the pointer.
        assert_eq!(r.route(&ev(MouseKind::ScrollUp, 11, 1)), Some(FocusId(2)));
        assert_eq!(r.route(&ev(MouseKind::ScrollDown, 1, 1)), Some(FocusId(1)));
        assert_eq!(r.captured(), None, "scroll never captures");
    }

    #[test]
    fn begin_frame_clears_regions_keeps_capture() {
        let mut r = MouseRouter::new();
        r.register(FocusId(3), Rect::new(0, 0, 4, 4));
        let _ = r.route(&ev(MouseKind::Down(MouseButton::Left), 1, 1));
        r.begin_frame();
        assert_eq!(r.hit(1, 1), None, "regions cleared");
        assert_eq!(r.captured(), Some(FocusId(3)), "capture survives rebuild");
    }

    #[test]
    fn viewport_drag_orbits() {
        let g = viewport_gesture(
            &ev(MouseKind::Drag(MouseButton::Left), 12, 8),
            Some((10, 10)),
            0.5,
        );
        assert_eq!(
            g,
            ViewportGesture::Orbit {
                dyaw: 1.0,
                dpitch: 1.0
            }
        );
        let none = viewport_gesture(&ev(MouseKind::Drag(MouseButton::Left), 12, 8), None, 0.5);
        assert_eq!(none, ViewportGesture::None);
    }
}
