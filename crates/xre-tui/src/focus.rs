//! [`FocusManager`]: tab order and the focus ring.
//!
//! Widgets that accept input register a [`FocusId`]; the manager tracks which is
//! focused and cycles focus forward/backward (Tab / Shift-Tab). Event routing in
//! an application checks [`FocusManager::is_focused`] to deliver input
//! focused-first (`RiftEngine-Plan/06-phase-1-tui-core.md` §1.4).

/// A stable identifier for a focusable widget within one [`FocusManager`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusId(pub u32);

/// Tracks focus order and the currently-focused widget.
#[derive(Clone, Debug, Default)]
pub struct FocusManager {
    order: Vec<FocusId>,
    current: Option<usize>,
}

impl FocusManager {
    /// An empty focus manager.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `id` at the end of the tab order. Re-registering is a no-op.
    /// The first registered widget becomes focused.
    pub fn register(&mut self, id: FocusId) {
        if !self.order.contains(&id) {
            self.order.push(id);
            if self.current.is_none() {
                self.current = Some(0);
            }
        }
    }

    /// Clear all registrations (e.g. when the UI is rebuilt).
    pub fn clear(&mut self) {
        self.order.clear();
        self.current = None;
    }

    /// The currently-focused id, if any.
    #[must_use]
    pub fn focused(&self) -> Option<FocusId> {
        self.current.and_then(|i| self.order.get(i).copied())
    }

    /// Whether `id` is currently focused.
    #[must_use]
    pub fn is_focused(&self, id: FocusId) -> bool {
        self.focused() == Some(id)
    }

    /// Move focus to the next widget in tab order (wrapping).
    pub fn focus_next(&mut self) {
        if self.order.is_empty() {
            return;
        }
        self.current = Some(self.current.map_or(0, |i| (i + 1) % self.order.len()));
    }

    /// Move focus to the previous widget in tab order (wrapping).
    pub fn focus_prev(&mut self) {
        if self.order.is_empty() {
            return;
        }
        let n = self.order.len();
        self.current = Some(self.current.map_or(0, |i| (i + n - 1) % n));
    }

    /// Focus a specific id if it is registered.
    pub fn focus(&mut self, id: FocusId) {
        if let Some(pos) = self.order.iter().position(|&x| x == id) {
            self.current = Some(pos);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_registration_is_focused() {
        let mut fm = FocusManager::new();
        fm.register(FocusId(10));
        assert_eq!(fm.focused(), Some(FocusId(10)));
    }

    #[test]
    fn tab_cycles_and_wraps() {
        let mut fm = FocusManager::new();
        fm.register(FocusId(1));
        fm.register(FocusId(2));
        fm.register(FocusId(3));
        assert!(fm.is_focused(FocusId(1)));
        fm.focus_next();
        assert!(fm.is_focused(FocusId(2)));
        fm.focus_next();
        fm.focus_next();
        assert!(fm.is_focused(FocusId(1)), "wraps to start");
        fm.focus_prev();
        assert!(fm.is_focused(FocusId(3)), "wraps backward");
    }

    #[test]
    fn duplicate_register_is_noop() {
        let mut fm = FocusManager::new();
        fm.register(FocusId(1));
        fm.register(FocusId(1));
        assert_eq!(fm.focused(), Some(FocusId(1)));
        fm.focus_next();
        assert!(fm.is_focused(FocusId(1)));
    }
}
