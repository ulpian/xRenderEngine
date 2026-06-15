//! The constraint-based [`Layout`] solver and [`GridLayout`].
//!
//! A single-pass integer solver splits a [`Rect`] along one axis according to a
//! list of [`Constraint`]s. Remainder distribution is defined and tested — the
//! off-by-one row is the classic TUI layout bug
//! (`RiftEngine-Plan/06-phase-1-tui-core.md` §1.3), so the rules here are exact:
//!
//! 1. Fixed constraints ([`Constraint::Len`], [`Constraint::Pct`],
//!    [`Constraint::Ratio`]) claim their computed size first.
//! 2. The slack is shared among flexible constraints ([`Constraint::Fill`],
//!    [`Constraint::Min`], [`Constraint::Max`]) in proportion to weight, then
//!    clamped to each one's bounds, with freed space redistributed.
//! 3. Any integer-rounding remainder is handed out one cell at a time to the
//!    flexible segments left-to-right (or, with none, to the last segment).

use xre_core::Rect;

/// The axis a [`Layout`] divides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Split into columns (left to right).
    Horizontal,
    /// Split into rows (top to bottom).
    Vertical,
}

/// A sizing rule for one segment of a [`Layout`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Constraint {
    /// An exact number of cells.
    Len(u32),
    /// A percentage `0..=100` of the available length.
    Pct(u16),
    /// A `num/den` fraction of the available length.
    Ratio(u32, u32),
    /// At least this many cells; grows with available slack.
    Min(u32),
    /// At most this many cells; grows with slack up to the cap.
    Max(u32),
    /// A flexible share of the slack, weighted by the given factor.
    Fill(u16),
}

impl Constraint {
    /// The fixed size this constraint claims up front (0 for flexible ones).
    fn fixed_size(self, avail: u32) -> Option<u32> {
        match self {
            Self::Len(n) => Some(n.min(avail)),
            Self::Pct(p) => Some((u32::from(p.min(100)) * avail) / 100),
            Self::Ratio(n, d) => Some(if d == 0 { 0 } else { (n * avail) / d.max(1) }),
            Self::Min(_) | Self::Max(_) | Self::Fill(_) => None,
        }
    }

    fn weight(self) -> u32 {
        match self {
            Self::Fill(w) => u32::from(w).max(1),
            Self::Min(_) | Self::Max(_) => 1,
            _ => 0,
        }
    }

    const fn lower(self) -> u32 {
        match self {
            Self::Min(n) => n,
            _ => 0,
        }
    }

    const fn upper(self) -> u32 {
        match self {
            Self::Max(n) => n,
            _ => u32::MAX,
        }
    }
}

/// Solve `constraints` over `total` cells with `gap` cells between segments,
/// returning each segment's length. Lengths sum to at most `total`.
#[must_use]
pub fn solve(total: u32, constraints: &[Constraint], gap: u32) -> Vec<u32> {
    let n = constraints.len();
    if n == 0 {
        return Vec::new();
    }
    let total_gap = gap.saturating_mul((n - 1) as u32);
    let avail = total.saturating_sub(total_gap);

    let mut sizes = vec![0u32; n];
    let mut flexible: Vec<usize> = Vec::new();
    let mut fixed_sum = 0u32;
    for (i, c) in constraints.iter().enumerate() {
        if let Some(s) = c.fixed_size(avail) {
            sizes[i] = s;
            fixed_sum = fixed_sum.saturating_add(s);
        } else {
            sizes[i] = c.lower();
            fixed_sum = fixed_sum.saturating_add(c.lower());
            flexible.push(i);
        }
    }

    if fixed_sum > avail {
        // Over-subscribed: scale everything down proportionally to fit.
        shrink_to_fit(&mut sizes, avail);
        return sizes;
    }

    // Distribute slack among flexible segments, respecting upper bounds.
    let mut slack = avail - fixed_sum;
    // Iterate so that clamping at a Max frees space for the others.
    let mut active: Vec<usize> = flexible.clone();
    while slack > 0 && !active.is_empty() {
        let total_weight: u32 = active.iter().map(|&i| constraints[i].weight()).sum();
        if total_weight == 0 {
            break;
        }
        let mut any_capped = false;
        let mut given = 0u32;
        // Snapshot to avoid borrow issues while mutating sizes.
        let mut next_active = Vec::new();
        for &i in &active {
            let w = constraints[i].weight();
            let share = (slack * w) / total_weight;
            let cap = constraints[i].upper();
            let room = cap.saturating_sub(sizes[i]);
            let add = share.min(room);
            sizes[i] += add;
            given += add;
            if add < room {
                next_active.push(i);
            } else {
                any_capped = true;
            }
        }
        slack -= given;
        active = next_active;
        if given == 0 && !any_capped {
            break; // rounding floor reached; handle remainder below
        }
    }

    // Hand out the integer-rounding remainder one cell at a time to the
    // flexible segments. With no flexible segment, leftover space stays
    // unallocated — two `Len(5)` in a width-20 area give `[5, 5]`, not `[5, 15]`.
    if slack > 0 && !flexible.is_empty() {
        let targets: Vec<usize> = flexible
            .iter()
            .copied()
            .filter(|&i| sizes[i] < constraints[i].upper())
            .collect();
        let targets = if targets.is_empty() {
            flexible.clone()
        } else {
            targets
        };
        let mut t = 0;
        while slack > 0 {
            sizes[targets[t % targets.len()]] += 1;
            slack -= 1;
            t += 1;
        }
    }

    sizes
}

/// Proportionally shrink `sizes` so their sum is at most `avail`.
fn shrink_to_fit(sizes: &mut [u32], avail: u32) {
    let sum: u32 = sizes.iter().sum();
    if sum == 0 {
        return;
    }
    let mut running = 0u32;
    let mut acc = 0u64;
    let denom = u64::from(sum);
    for s in sizes.iter_mut() {
        acc += u64::from(*s) * u64::from(avail);
        // Largest-remainder style: cumulative rounding avoids drift.
        let cum = (acc / denom) as u32;
        *s = cum - running;
        running = cum;
    }
}

/// A configured division of a [`Rect`] along one [`Direction`].
#[derive(Clone, Debug)]
pub struct Layout {
    direction: Direction,
    constraints: Vec<Constraint>,
    gap: u32,
}

impl Layout {
    /// A horizontal layout (columns) with the given constraints.
    #[must_use]
    pub fn horizontal(constraints: impl Into<Vec<Constraint>>) -> Self {
        Self {
            direction: Direction::Horizontal,
            constraints: constraints.into(),
            gap: 0,
        }
    }

    /// A vertical layout (rows) with the given constraints.
    #[must_use]
    pub fn vertical(constraints: impl Into<Vec<Constraint>>) -> Self {
        Self {
            direction: Direction::Vertical,
            constraints: constraints.into(),
            gap: 0,
        }
    }

    /// Builder: set the gap (in cells) between adjacent segments.
    #[must_use]
    pub const fn gap(mut self, gap: u32) -> Self {
        self.gap = gap;
        self
    }

    /// Split `area` into one [`Rect`] per constraint.
    #[must_use]
    pub fn split(&self, area: Rect) -> Vec<Rect> {
        let total = match self.direction {
            Direction::Horizontal => area.width(),
            Direction::Vertical => area.height(),
        };
        let sizes = solve(total, &self.constraints, self.gap);
        let mut out = Vec::with_capacity(sizes.len());
        let mut offset = 0u32;
        for size in sizes {
            let rect = match self.direction {
                Direction::Horizontal => {
                    Rect::new(area.left() + offset, area.top(), size, area.height())
                }
                Direction::Vertical => {
                    Rect::new(area.left(), area.top() + offset, area.width(), size)
                }
            };
            out.push(rect);
            offset += size + self.gap;
        }
        out
    }
}

/// An `n × m` grid layout: independent column and row constraints, plus gaps.
#[derive(Clone, Debug)]
pub struct GridLayout {
    cols: Vec<Constraint>,
    rows: Vec<Constraint>,
    col_gap: u32,
    row_gap: u32,
}

impl GridLayout {
    /// A grid with the given column and row constraints.
    #[must_use]
    pub fn new(cols: impl Into<Vec<Constraint>>, rows: impl Into<Vec<Constraint>>) -> Self {
        Self {
            cols: cols.into(),
            rows: rows.into(),
            col_gap: 0,
            row_gap: 0,
        }
    }

    /// Builder: set the gaps (cells) between columns and rows.
    #[must_use]
    pub const fn gaps(mut self, col_gap: u32, row_gap: u32) -> Self {
        self.col_gap = col_gap;
        self.row_gap = row_gap;
        self
    }

    /// The number of columns and rows.
    #[must_use]
    pub const fn dims(&self) -> (usize, usize) {
        (self.cols.len(), self.rows.len())
    }

    /// Solve the grid over `area`, returning a row-major matrix of cell rects:
    /// `cells[row][col]`.
    #[must_use]
    pub fn split(&self, area: Rect) -> Vec<Vec<Rect>> {
        let col_sizes = solve(area.width(), &self.cols, self.col_gap);
        let row_sizes = solve(area.height(), &self.rows, self.row_gap);
        let mut rows = Vec::with_capacity(row_sizes.len());
        let mut y = area.top();
        for &rh in &row_sizes {
            let mut cols = Vec::with_capacity(col_sizes.len());
            let mut x = area.left();
            for &cw in &col_sizes {
                cols.push(Rect::new(x, y, cw, rh));
                x += cw + self.col_gap;
            }
            rows.push(cols);
            y += rh + self.row_gap;
        }
        rows
    }

    /// The bounding rect of the grid cells spanning `cols`/`rows` (inclusive
    /// ranges), useful for cell spanning. Returns an empty rect if out of range.
    #[must_use]
    pub fn span(&self, area: Rect, cols: (usize, usize), rows: (usize, usize)) -> Rect {
        let grid = self.split(area);
        let (c0, c1) = cols;
        let (r0, r1) = rows;
        if r0 >= grid.len() || r1 >= grid.len() || grid.is_empty() {
            return Rect::default();
        }
        let first = grid[r0].get(c0).copied().unwrap_or_default();
        let last = grid
            .get(r1)
            .and_then(|row| row.get(c1))
            .copied()
            .unwrap_or(first);
        first.union(last)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_lengths_sum_exactly() {
        let s = solve(20, &[Constraint::Len(5), Constraint::Len(5)], 0);
        assert_eq!(s, vec![5, 5]);
    }

    #[test]
    fn fill_takes_remainder() {
        let s = solve(20, &[Constraint::Len(4), Constraint::Fill(1)], 0);
        assert_eq!(s, vec![4, 16]);
    }

    #[test]
    fn two_fills_split_evenly_with_remainder_left_first() {
        // 21 cells, two equal fills → 11 + 10 (extra cell to the leftmost).
        let s = solve(21, &[Constraint::Fill(1), Constraint::Fill(1)], 0);
        assert_eq!(s, vec![11, 10]);
        assert_eq!(s.iter().sum::<u32>(), 21);
    }

    #[test]
    fn weighted_fill() {
        let s = solve(30, &[Constraint::Fill(2), Constraint::Fill(1)], 0);
        assert_eq!(s, vec![20, 10]);
    }

    #[test]
    fn percentages() {
        let s = solve(100, &[Constraint::Pct(25), Constraint::Pct(75)], 0);
        assert_eq!(s, vec![25, 75]);
    }

    #[test]
    fn min_grows_max_caps() {
        // Fill wants everything but Max(5) caps it; Min(3) keeps its floor.
        let s = solve(
            20,
            &[Constraint::Max(5), Constraint::Fill(1), Constraint::Min(3)],
            0,
        );
        assert_eq!(s.iter().sum::<u32>(), 20);
        assert_eq!(s[0], 5); // capped
        assert!(s[2] >= 3); // floor respected
    }

    #[test]
    fn gap_is_accounted() {
        // 10 cells, two fills, 2-cell gap → 8 to split → 4 + 4.
        let s = solve(10, &[Constraint::Fill(1), Constraint::Fill(1)], 2);
        assert_eq!(s, vec![4, 4]);
    }

    #[test]
    fn degenerate_zero_and_one() {
        assert_eq!(solve(0, &[Constraint::Fill(1)], 0), vec![0]);
        assert_eq!(solve(1, &[Constraint::Len(5)], 0), vec![1]); // clamped
        assert!(solve(5, &[], 0).is_empty());
    }

    #[test]
    fn split_produces_adjacent_rects() {
        let area = Rect::new(0, 0, 30, 10);
        let parts = Layout::horizontal([Constraint::Len(10), Constraint::Fill(1)]).split(area);
        assert_eq!(parts[0], Rect::new(0, 0, 10, 10));
        assert_eq!(parts[1], Rect::new(10, 0, 20, 10));
    }

    #[test]
    fn grid_splits_row_major() {
        let area = Rect::new(0, 0, 20, 10);
        let g = GridLayout::new(
            [Constraint::Fill(1), Constraint::Fill(1)],
            [Constraint::Fill(1), Constraint::Fill(1)],
        );
        let cells = g.split(area);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0][0], Rect::new(0, 0, 10, 5));
        assert_eq!(cells[1][1], Rect::new(10, 5, 10, 5));
    }

    #[test]
    fn grid_span_unions_cells() {
        let area = Rect::new(0, 0, 20, 10);
        let g = GridLayout::new(
            [Constraint::Fill(1), Constraint::Fill(1)],
            [Constraint::Fill(1), Constraint::Fill(1)],
        );
        // Span both columns of row 0.
        let r = g.span(area, (0, 1), (0, 0));
        assert_eq!(r, Rect::new(0, 0, 20, 5));
    }
}
