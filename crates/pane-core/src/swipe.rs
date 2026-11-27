//! Trackpad swipes: tells a three-finger swipe left/right or a four-finger
//! swipe up apart from scrolling, pinching and resting fingers.
//!
//! The event tap feeds it every gesture event's touches (positions on the
//! trackpad, 0..1, y pointing up as in `NSTouch.normalizedPosition`). It
//! answers with at most one [`Swipe`] per stroke: after a swipe fires,
//! nothing more fires until the finger count changes or every finger lifts.

use serde::{Deserialize, Serialize};

/// Average finger travel (share of the trackpad) that makes a swipe.
pub const SWIPE_DISTANCE: f64 = 0.15;
/// The travel along the swipe must be this many times the travel across.
pub const SWIPE_STRAIGHTNESS: f64 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Swipe {
    /// Three fingers to the left: the next workspace.
    Left,
    /// Three fingers to the right: the previous workspace.
    Right,
    /// Four fingers up: the palette.
    Up,
}

/// One finger on the trackpad.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub x: f64,
    pub y: f64,
}

/// Where a set of fingers is: their centre, and how far apart they are
/// (mean distance from the centre).
fn centre_and_spread(touches: &[Touch]) -> (f64, f64, f64) {
    let n = touches.len().max(1) as f64;
    let cx = touches.iter().map(|t| t.x).sum::<f64>() / n;
    let cy = touches.iter().map(|t| t.y).sum::<f64>() / n;
    let spread = touches
        .iter()
        .map(|t| (t.x - cx).hypot(t.y - cy))
        .sum::<f64>()
        / n;
    (cx, cy, spread)
}

/// Fingers are told apart only by how many there are, not by
/// `NSTouch.identity` (whose hash is not promised to stay the same from one
/// event to the next).
#[derive(Debug, Default)]
pub struct SwipeTracker {
    /// Fingers in the stroke.
    count: usize,
    /// Centre and spread at the start of the stroke.
    start: (f64, f64, f64),
    /// A swipe fired in this stroke.
    fired: bool,
}

impl SwipeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget the stroke (the tap was disabled, swipes were turned off).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The fingers touching the trackpad now (lifted ones left out).
    pub fn on_touches(&mut self, touches: &[Touch]) -> Option<Swipe> {
        let now = centre_and_spread(touches);
        if touches.len() != self.count {
            // A new stroke whenever a finger lands or lifts.
            self.count = touches.len();
            self.start = now;
            self.fired = false;
            return None;
        }
        if self.fired || !matches!(self.count, 3 | 4) {
            return None;
        }
        let dx = now.0 - self.start.0;
        let dy = now.1 - self.start.1;
        // Fingers spreading or closing (a pinch) while the centre drifts.
        let pinch = (now.2 - self.start.2).abs();
        let swipe = match self.count {
            3 if dx.abs() >= SWIPE_DISTANCE && dx.abs() >= SWIPE_STRAIGHTNESS * dy.abs() => {
                Some(if dx < 0.0 { Swipe::Left } else { Swipe::Right })
            }
            4 if dy >= SWIPE_DISTANCE && dy >= SWIPE_STRAIGHTNESS * dx.abs() => Some(Swipe::Up),
            _ => None,
        };
        let swipe = swipe.filter(|_| pinch < 0.5 * dx.hypot(dy));
        self.fired = swipe.is_some();
        swipe
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingers(n: usize, dx: f64, dy: f64) -> Vec<Touch> {
        (0..n)
            .map(|i| Touch {
                x: 0.3 + 0.1 * i as f64 + dx,
                y: 0.5 + dy,
            })
            .collect()
    }

    fn stroke(t: &mut SwipeTracker, n: usize, path: &[(f64, f64)]) -> Vec<Swipe> {
        path.iter()
            .filter_map(|&(dx, dy)| t.on_touches(&fingers(n, dx, dy)))
            .collect()
    }

    #[test]
    fn three_fingers_left_and_right() {
        let mut t = SwipeTracker::new();
        let left = [(0.0, 0.0), (-0.05, 0.0), (-0.1, 0.01), (-0.2, 0.02)];
        assert_eq!(stroke(&mut t, 3, &left), vec![Swipe::Left]);
        t.on_touches(&[]);
        let right = [(0.0, 0.0), (0.2, -0.03)];
        assert_eq!(stroke(&mut t, 3, &right), vec![Swipe::Right]);
    }

    #[test]
    fn fires_once_per_stroke() {
        let mut t = SwipeTracker::new();
        let path = [(0.0, 0.0), (-0.2, 0.0), (-0.4, 0.0), (0.0, 0.0), (0.3, 0.0)];
        assert_eq!(stroke(&mut t, 3, &path), vec![Swipe::Left]);
        // Lifting every finger starts a new stroke.
        t.on_touches(&[]);
        assert_eq!(
            stroke(&mut t, 3, &[(0.0, 0.0), (-0.2, 0.0)]),
            vec![Swipe::Left]
        );
    }

    #[test]
    fn four_fingers_up_only() {
        let mut t = SwipeTracker::new();
        assert_eq!(
            stroke(&mut t, 4, &[(0.0, 0.0), (0.02, 0.2)]),
            vec![Swipe::Up]
        );
        t.on_touches(&[]);
        assert!(stroke(&mut t, 4, &[(0.0, 0.0), (0.0, -0.3)]).is_empty());
        t.on_touches(&[]);
        assert!(stroke(&mut t, 4, &[(0.0, 0.0), (-0.3, 0.0)]).is_empty());
        t.on_touches(&[]);
        assert!(stroke(&mut t, 3, &[(0.0, 0.0), (0.0, 0.3)]).is_empty());
    }

    #[test]
    fn two_and_five_fingers_and_diagonals_do_nothing() {
        let mut t = SwipeTracker::new();
        assert!(stroke(&mut t, 2, &[(0.0, 0.0), (-0.4, 0.0)]).is_empty());
        t.on_touches(&[]);
        assert!(stroke(&mut t, 5, &[(0.0, 0.0), (-0.4, 0.0)]).is_empty());
        t.on_touches(&[]);
        assert!(stroke(&mut t, 3, &[(0.0, 0.0), (-0.2, 0.2)]).is_empty());
        t.on_touches(&[]);
        assert!(stroke(&mut t, 3, &[(0.0, 0.0), (-0.1, 0.0)]).is_empty());
    }

    #[test]
    fn a_finger_landing_restarts_the_stroke() {
        let mut t = SwipeTracker::new();
        t.on_touches(&fingers(2, 0.0, 0.0));
        // The third finger lands after the first two moved: travel counts
        // from here, not from where the first two started.
        assert_eq!(t.on_touches(&fingers(3, -0.1, 0.0)), None);
        assert_eq!(t.on_touches(&fingers(3, -0.2, 0.0)), None);
        assert_eq!(t.on_touches(&fingers(3, -0.3, 0.0)), Some(Swipe::Left));
    }

    #[test]
    fn pinch_is_not_a_swipe() {
        let mut t = SwipeTracker::new();
        let start = fingers(3, 0.0, 0.0);
        t.on_touches(&start);
        // Fingers spread: the average moves left but one finger goes right.
        let spread: Vec<Touch> = start
            .iter()
            .enumerate()
            .map(|(i, f)| Touch {
                x: f.x + [-0.5, -0.2, 0.1][i],
                ..*f
            })
            .collect();
        assert_eq!(t.on_touches(&spread), None);
    }
}
