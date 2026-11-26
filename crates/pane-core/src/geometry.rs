//! Geometry in global top-left points (the coordinate space of AX and CGDisplayBounds).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

impl Size {
    pub const ZERO: Size = Size { w: 0.0, h: 0.0 };

    pub fn new(w: f64, h: f64) -> Self {
        Size { w, h }
    }

    pub fn along(&self, o: Orientation) -> f64 {
        match o {
            Orientation::Horizontal => self.w,
            Orientation::Vertical => self.h,
        }
    }

    pub fn max(self, other: Size) -> Size {
        Size::new(self.w.max(other.w), self.h.max(other.h))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Rect { x, y, w, h }
    }

    pub fn origin(&self) -> Point {
        Point {
            x: self.x,
            y: self.y,
        }
    }

    pub fn size(&self) -> Size {
        Size::new(self.w, self.h)
    }

    pub fn max_x(&self) -> f64 {
        self.x + self.w
    }

    pub fn max_y(&self) -> f64 {
        self.y + self.h
    }

    pub fn center(&self) -> Point {
        Point {
            x: self.x + self.w / 2.0,
            y: self.y + self.h / 2.0,
        }
    }

    pub fn area(&self) -> f64 {
        self.w.max(0.0) * self.h.max(0.0)
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x && p.x < self.max_x() && p.y >= self.y && p.y < self.max_y()
    }

    pub fn inset(&self, d: f64) -> Rect {
        Rect::new(
            self.x + d,
            self.y + d,
            (self.w - 2.0 * d).max(0.0),
            (self.h - 2.0 * d).max(0.0),
        )
    }

    pub fn intersection(&self, o: &Rect) -> Option<Rect> {
        let x = self.x.max(o.x);
        let y = self.y.max(o.y);
        let mx = self.max_x().min(o.max_x());
        let my = self.max_y().min(o.max_y());
        (mx > x && my > y).then(|| Rect::new(x, y, mx - x, my - y))
    }

    pub fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect::new(
            x,
            y,
            self.max_x().max(o.max_x()) - x,
            self.max_y().max(o.max_y()) - y,
        )
    }

    /// True when every edge is within `tol` points.
    pub fn approx_eq(&self, o: &Rect, tol: f64) -> bool {
        (self.x - o.x).abs() <= tol
            && (self.y - o.y).abs() <= tol
            && (self.w - o.w).abs() <= tol
            && (self.h - o.h).abs() <= tol
    }

    pub fn origin_near(&self, o: &Rect, tol: f64) -> bool {
        (self.x - o.x).abs() <= tol && (self.y - o.y).abs() <= tol
    }

    /// Position (start) and length along an axis.
    pub fn span(&self, o: Orientation) -> (f64, f64) {
        match o {
            Orientation::Horizontal => (self.x, self.w),
            Orientation::Vertical => (self.y, self.h),
        }
    }

    pub fn with_span(&self, o: Orientation, start: f64, len: f64) -> Rect {
        match o {
            Orientation::Horizontal => Rect::new(start, self.y, len, self.h),
            Orientation::Vertical => Rect::new(self.x, start, self.w, len),
        }
    }

    /// Move (and if needed shrink) `self` so it lies inside `area`.
    pub fn fit_into(&self, area: &Rect) -> Rect {
        let w = self.w.min(area.w).max(1.0);
        let h = self.h.min(area.h).max(1.0);
        let x = self.x.clamp(area.x, area.max_x() - w);
        let y = self.y.clamp(area.y, area.max_y() - h);
        Rect::new(x, y, w, h)
    }

    /// Express `self` relative to `outer` as fractions in 0..1.
    pub fn normalized_in(&self, outer: &Rect) -> Rect {
        if outer.w <= 0.0 || outer.h <= 0.0 {
            return Rect::default();
        }
        Rect::new(
            (self.x - outer.x) / outer.w,
            (self.y - outer.y) / outer.h,
            self.w / outer.w,
            self.h / outer.h,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    /// Children side by side (split along x).
    #[default]
    #[serde(alias = "h")]
    Horizontal,
    /// Children stacked (split along y).
    #[serde(alias = "v")]
    Vertical,
}

impl Orientation {
    pub fn flip(self) -> Self {
        match self {
            Orientation::Horizontal => Orientation::Vertical,
            Orientation::Vertical => Orientation::Horizontal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

impl Direction {
    pub fn orientation(self) -> Orientation {
        match self {
            Direction::Left | Direction::Right => Orientation::Horizontal,
            Direction::Up | Direction::Down => Orientation::Vertical,
        }
    }

    /// Right and Down move towards larger indices.
    pub fn is_forward(self) -> bool {
        matches!(self, Direction::Right | Direction::Down)
    }

    pub fn opposite(self) -> Self {
        match self {
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "left" | "west" => Direction::Left,
            "right" | "east" => Direction::Right,
            "up" | "north" => Direction::Up,
            "down" | "south" => Direction::Down,
            _ => return None,
        })
    }
}

/// Pick the rect from `candidates` that is the nearest neighbour of `from` in `dir`.
///
/// A candidate qualifies when its centre lies beyond `from`'s edge in `dir`.
/// Candidates overlapping `from` on the perpendicular axis win over ones that
/// do not; ties go to the smallest gap, then to the smallest perpendicular
/// offset between centres.
pub fn neighbor_in_direction<K: Clone>(
    from: &Rect,
    candidates: &[(K, Rect)],
    dir: Direction,
) -> Option<K> {
    let c = from.center();
    let mut best: Option<(bool, f64, f64, K)> = None;
    for (k, r) in candidates {
        let rc = r.center();
        let (beyond, gap, perp, overlap) = match dir {
            Direction::Left => (
                rc.x < from.x + 0.5,
                from.x - r.max_x(),
                (rc.y - c.y).abs(),
                r.y < from.max_y() && r.max_y() > from.y,
            ),
            Direction::Right => (
                rc.x > from.max_x() - 0.5,
                r.x - from.max_x(),
                (rc.y - c.y).abs(),
                r.y < from.max_y() && r.max_y() > from.y,
            ),
            Direction::Up => (
                rc.y < from.y + 0.5,
                from.y - r.max_y(),
                (rc.x - c.x).abs(),
                r.x < from.max_x() && r.max_x() > from.x,
            ),
            Direction::Down => (
                rc.y > from.max_y() - 0.5,
                r.y - from.max_y(),
                (rc.x - c.x).abs(),
                r.x < from.max_x() && r.max_x() > from.x,
            ),
        };
        if !beyond || r.approx_eq(from, 0.01) {
            continue;
        }
        let gap = gap.max(0.0);
        let better = match &best {
            None => true,
            Some((bo, bg, bp, _)) => {
                (overlap && !bo)
                    || (overlap == *bo
                        && (gap < *bg - 0.5 || ((gap - bg).abs() <= 0.5 && perp < *bp)))
            }
        };
        if better {
            best = Some((overlap, gap, perp, k.clone()));
        }
    }
    best.map(|b| b.3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_into_moves_and_shrinks() {
        let area = Rect::new(0.0, 0.0, 100.0, 100.0);
        assert_eq!(
            Rect::new(90.0, -5.0, 20.0, 20.0).fit_into(&area),
            Rect::new(80.0, 0.0, 20.0, 20.0)
        );
        assert_eq!(
            Rect::new(10.0, 10.0, 200.0, 50.0).fit_into(&area),
            Rect::new(0.0, 10.0, 100.0, 50.0)
        );
    }

    #[test]
    fn neighbor_prefers_overlap() {
        let from = Rect::new(0.0, 0.0, 100.0, 100.0);
        let cands = vec![
            ("far_overlap", Rect::new(300.0, 50.0, 100.0, 100.0)),
            ("near_no_overlap", Rect::new(110.0, 200.0, 50.0, 50.0)),
            ("left", Rect::new(-100.0, 0.0, 100.0, 100.0)),
        ];
        assert_eq!(
            neighbor_in_direction(&from, &cands, Direction::Right),
            Some("far_overlap")
        );
        assert_eq!(
            neighbor_in_direction(&from, &cands, Direction::Left),
            Some("left")
        );
        assert_eq!(neighbor_in_direction(&from, &cands, Direction::Up), None);
    }
}
