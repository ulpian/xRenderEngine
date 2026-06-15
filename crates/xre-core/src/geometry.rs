//! Integer rectangle algebra. All drawing in the engine is [`Rect`]-scoped.
//!
//! Coordinates are unsigned with the origin at the top-left; `right`/`bottom`
//! are *exclusive*. Operations saturate rather than overflow, and degenerate
//! results collapse to an empty rect (zero size) rather than wrapping.

use crate::math::UVec2;

/// An axis-aligned rectangle in cell space.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rect {
    /// Top-left corner.
    pub pos: UVec2,
    /// Width and height in cells.
    pub size: UVec2,
}

impl Rect {
    /// Construct a rect from `(x, y)` and `(w, h)`.
    #[must_use]
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self {
            pos: UVec2::new(x, y),
            size: UVec2::new(w, h),
        }
    }

    /// A rect at the origin with the given size.
    #[must_use]
    pub const fn from_size(size: UVec2) -> Self {
        Self {
            pos: UVec2::ZERO,
            size,
        }
    }

    /// Left edge (inclusive).
    #[must_use]
    pub const fn left(&self) -> u32 {
        self.pos.x
    }

    /// Top edge (inclusive).
    #[must_use]
    pub const fn top(&self) -> u32 {
        self.pos.y
    }

    /// Right edge (exclusive), saturating.
    #[must_use]
    pub const fn right(&self) -> u32 {
        self.pos.x.saturating_add(self.size.x)
    }

    /// Bottom edge (exclusive), saturating.
    #[must_use]
    pub const fn bottom(&self) -> u32 {
        self.pos.y.saturating_add(self.size.y)
    }

    /// Width in cells.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.size.x
    }

    /// Height in cells.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.size.y
    }

    /// Number of cells covered.
    #[must_use]
    pub const fn area(&self) -> u32 {
        self.size.x * self.size.y
    }

    /// `true` if the rect covers no cells.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.size.x == 0 || self.size.y == 0
    }

    /// `true` if `point` lies inside the rect.
    #[must_use]
    pub const fn contains(&self, point: UVec2) -> bool {
        point.x >= self.left()
            && point.x < self.right()
            && point.y >= self.top()
            && point.y < self.bottom()
    }

    /// The largest rect contained in both `self` and `other`.
    ///
    /// The result never exceeds either input on any edge; non-overlapping
    /// inputs yield an empty rect anchored at the overlap origin.
    #[must_use]
    pub fn intersect(&self, other: Self) -> Self {
        let left = self.left().max(other.left());
        let top = self.top().max(other.top());
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Self {
            pos: UVec2::new(left, top),
            size: UVec2::new(right.saturating_sub(left), bottom.saturating_sub(top)),
        }
    }

    /// The smallest rect containing both `self` and `other`. An empty operand
    /// is treated as absent (returns the other).
    #[must_use]
    pub fn union(&self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return *self;
        }
        let left = self.left().min(other.left());
        let top = self.top().min(other.top());
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self {
            pos: UVec2::new(left, top),
            size: UVec2::new(right - left, bottom - top),
        }
    }

    /// Clamp `self` so it fits within `bounds` (an alias for [`Rect::intersect`]).
    #[must_use]
    pub fn clamp(&self, bounds: Self) -> Self {
        self.intersect(bounds)
    }

    /// Shrink the rect inward by `margin` cells on every side, collapsing to
    /// empty if the margins exceed the size.
    #[must_use]
    pub const fn inset(&self, margin: u32) -> Self {
        let dbl = margin.saturating_mul(2);
        Self {
            pos: UVec2::new(
                self.pos.x.saturating_add(margin),
                self.pos.y.saturating_add(margin),
            ),
            size: UVec2::new(
                self.size.x.saturating_sub(dbl),
                self.size.y.saturating_sub(dbl),
            ),
        }
    }

    /// Split into `(left, right)` at column offset `at` (relative to the rect),
    /// clamped to the rect's width.
    #[must_use]
    pub fn split_at_x(&self, at: u32) -> (Self, Self) {
        let at = at.min(self.size.x);
        (
            Self::new(self.pos.x, self.pos.y, at, self.size.y),
            Self::new(
                self.pos.x.saturating_add(at),
                self.pos.y,
                self.size.x - at,
                self.size.y,
            ),
        )
    }

    /// Split into `(top, bottom)` at row offset `at` (relative to the rect),
    /// clamped to the rect's height.
    #[must_use]
    pub fn split_at_y(&self, at: u32) -> (Self, Self) {
        let at = at.min(self.size.y);
        (
            Self::new(self.pos.x, self.pos.y, self.size.x, at),
            Self::new(
                self.pos.x,
                self.pos.y.saturating_add(at),
                self.size.x,
                self.size.y - at,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn intersect_of_disjoint_is_empty() {
        let a = Rect::new(0, 0, 2, 2);
        let b = Rect::new(5, 5, 2, 2);
        assert!(a.intersect(b).is_empty());
    }

    #[test]
    fn split_x_partitions_width() {
        let r = Rect::new(0, 0, 10, 4);
        let (l, rt) = r.split_at_x(3);
        assert_eq!(l.width(), 3);
        assert_eq!(rt.width(), 7);
        assert_eq!(rt.left(), 3);
        assert_eq!(l.width() + rt.width(), r.width());
    }

    #[test]
    fn inset_collapses_when_too_large() {
        assert!(Rect::new(0, 0, 4, 4).inset(3).is_empty());
    }

    proptest! {
        /// The intersection never exceeds either input on any edge
        /// (Stage 0.2 exit criterion).
        #[test]
        fn intersect_never_exceeds_inputs(
            ax in 0u32..100, ay in 0u32..100, aw in 0u32..100, ah in 0u32..100,
            bx in 0u32..100, by in 0u32..100, bw in 0u32..100, bh in 0u32..100,
        ) {
            let a = Rect::new(ax, ay, aw, ah);
            let b = Rect::new(bx, by, bw, bh);
            let i = a.intersect(b);
            // Area never exceeds either input, empty or not.
            prop_assert!(i.area() <= a.area() && i.area() <= b.area());
            // When the overlap is real, every edge lies within both inputs.
            if !i.is_empty() {
                prop_assert!(i.left() >= a.left() && i.left() >= b.left());
                prop_assert!(i.top() >= a.top() && i.top() >= b.top());
                prop_assert!(i.right() <= a.right() && i.right() <= b.right());
                prop_assert!(i.bottom() <= a.bottom() && i.bottom() <= b.bottom());
            }
        }

        /// Every cell of the intersection lies in both inputs.
        #[test]
        fn intersect_points_in_both(
            ax in 0u32..20, ay in 0u32..20, aw in 0u32..20, ah in 0u32..20,
            bx in 0u32..20, by in 0u32..20, bw in 0u32..20, bh in 0u32..20,
        ) {
            let a = Rect::new(ax, ay, aw, ah);
            let b = Rect::new(bx, by, bw, bh);
            let i = a.intersect(b);
            for y in i.top()..i.bottom() {
                for x in i.left()..i.right() {
                    let p = UVec2::new(x, y);
                    prop_assert!(a.contains(p) && b.contains(p));
                }
            }
        }
    }
}
