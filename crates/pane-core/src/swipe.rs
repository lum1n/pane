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
    /// Stable for as long as the finger stays down.
    pub id: u64,
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Default)]
pub struct SwipeTracker {
    /// Fingers at the start of the stroke, sorted by id.
    start: Vec<Touch>,
    /// A swipe fired in this stroke.
    fired: bool,
}

impl SwipeTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forget the stroke (the tap was disabled, swipes were turned off).
    pub fn reset(&mut self) {
        self.start.clear();
        self.fired = false;
    }

    /// The fingers touching the trackpad now (lifted ones left out).
    pub fn on_touches(&mut self, touches: &[Touch]) -> Option<Swipe> {
        let mut now = touches.to_vec();
        now.sort_by_key(|t| t.id);
        now.dedup_by_key(|t| t.id);
        let same_fingers =
            now.len() == self.start.len() && now.iter().zip(&self.start).all(|(a, b)| a.id == b.id);
        if !same_fingers {
            // A new stroke whenever a finger lands or lifts.
            self.start = now;
            self.fired = false;
            return None;
        }
        if self.fired || !matches!(now.len(), 3 | 4) {
            return None;
        }
        let n = now.len() as f64;
        let dx = now
            .iter()
            .zip(&self.start)
            .map(|(a, b)| a.x - b.x)
            .sum::<f64>()
            / n;
        let dy = now
            .iter()
            .zip(&self.start)
            .map(|(a, b)| a.y - b.y)
            .sum::<f64>()
            / n;
        let swipe = match now.len() {
            3 if dx.abs() >= SWIPE_DISTANCE && dx.abs() >= SWIPE_STRAIGHTNESS * dy.abs() => {
                Some(if dx < 0.0 { Swipe::Left } else { Swipe::Right })
            }
            4 if dy >= SWIPE_DISTANCE && dy >= SWIPE_STRAIGHTNESS * dx.abs() => Some(Swipe::Up),
            _ => None,
        };
        // Fingers that all move the same way: every finger must have gone
        // the swipe's way, which rules out pinches and rotations.
        let swipe = swipe.filter(|s| {
            now.iter().zip(&self.start).all(|(a, b)| match s {
                Swipe::Left => a.x < b.x,
                Swipe::Right => a.x > b.x,
                Swipe::Up => a.y > b.y,
            })
        });
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
                id: i as u64 + 1,
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
