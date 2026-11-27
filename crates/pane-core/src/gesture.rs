//! Hold the Pane modifier and point to move a window: the state machine
//! that tells a gesture apart from hotkeys and ordinary modifier use.
//!
//! The event tap feeds it raw input; it answers with [`GesturePhase`]s for
//! the core. Modifiers are bit sets ([`SHIFT`], [`CTRL`], [`ALT`], [`CMD`]);
//! other bits (caps lock) are ignored.

use serde::{Deserialize, Serialize};

use crate::config::Mods;
use crate::geometry::Point;

pub const SHIFT: u8 = 1;
pub const CTRL: u8 = 2;
pub const ALT: u8 = 4;
pub const CMD: u8 = 8;
const ALL: u8 = SHIFT | CTRL | ALT | CMD;

/// The pointer must move this far (points) with the modifier held before
/// a gesture starts.
pub const ACTIVATE_DISTANCE: f64 = 24.0;
/// Minimum pointer travel (points) between two `Move`s.
pub const MOVE_STEP: f64 = 4.0;

pub fn mod_bits(m: &Mods) -> u8 {
    (m.shift as u8 * SHIFT) | (m.ctrl as u8 * CTRL) | (m.alt as u8 * ALT) | (m.cmd as u8 * CMD)
}

/// Positions are global top-left points (like AX and the model).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GesturePhase {
    /// The gesture started. `at` is where the modifier went down (the
    /// window there moves); `pointer` is where the pointer is now.
    Begin {
        at: Point,
        pointer: Point,
    },
    Move {
        at: Point,
    },
    /// The modifier was released: drop the window at `at`.
    Drop {
        at: Point,
    },
    Cancel,
}

/// The left mouse button with no modifier gesture: where the pointer is
/// while the button is held, and where it was let go. With these the core
/// turns a title-bar drag of a tiled window into a drop.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DragPhase {
    /// The pointer moved with the left button down.
    Move { at: Point },
    /// The left button went up at `at`.
    Release { at: Point },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyVerdict {
    Pass,
    Swallow,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum State {
    Idle,
    /// Modifier held, pointer not far enough yet.
    Armed {
        start: Point,
    },
    Active {
        last: Point,
    },
    /// A key or button was used with the modifier: not a gesture until the
    /// modifier is released.
    Blocked,
}

#[derive(Clone, Debug)]
pub struct GestureTracker {
    mods: Option<u8>,
    state: State,
}

fn dist(a: Point, b: Point) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

impl GestureTracker {
    /// `mods: None` disables gestures.
    pub fn new(mods: Option<u8>) -> Self {
        GestureTracker {
            mods: mods.map(|m| m & ALL).filter(|m| *m != 0),
            state: State::Idle,
        }
    }

    pub fn set_mods(&mut self, mods: Option<u8>) -> Option<GesturePhase> {
        let out = self.reset();
        *self = GestureTracker::new(mods);
        out
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, State::Active { .. })
    }

    /// The modifier is held (a gesture may be starting or under way).
    pub fn is_busy(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    /// Forget everything (the tap was re-enabled after a timeout).
    pub fn reset(&mut self) -> Option<GesturePhase> {
        let was_active = self.is_active();
        self.state = State::Idle;
        was_active.then_some(GesturePhase::Cancel)
    }

    /// Does `mods` still hold every gesture modifier?
    fn held(&self, mods: u8) -> bool {
        self.mods.is_some_and(|g| mods & g == g)
    }

    fn exact(&self, mods: u8) -> bool {
        self.mods == Some(mods & ALL)
    }

    pub fn on_flags(&mut self, mods: u8, at: Point) -> Option<GesturePhase> {
        self.mods?;
        match self.state {
            State::Idle => {
                if self.exact(mods) {
                    self.state = State::Armed { start: at };
                }
                None
            }
            State::Armed { .. } => {
                if !self.held(mods) {
                    self.state = State::Idle;
                } else if !self.exact(mods) {
                    // More modifiers: a hotkey chord on its way.
                    self.state = State::Blocked;
                }
                None
            }
            State::Active { .. } => {
                if self.held(mods) {
                    return None; // Extra modifiers are ignored.
                }
                self.state = State::Idle;
                Some(GesturePhase::Drop { at })
            }
            State::Blocked => {
                if !self.held(mods) {
                    self.state = State::Idle;
                }
                None
            }
        }
    }

    pub fn on_key_down(&mut self, is_escape: bool) -> (KeyVerdict, Option<GesturePhase>) {
        match self.state {
            State::Armed { .. } => {
                self.state = State::Blocked;
                (KeyVerdict::Pass, None)
            }
            State::Active { .. } => {
                self.state = State::Blocked;
                let v = if is_escape {
                    KeyVerdict::Swallow
                } else {
                    KeyVerdict::Pass
                };
                (v, Some(GesturePhase::Cancel))
            }
            _ => (KeyVerdict::Pass, None),
        }
    }

    pub fn on_mouse_down(&mut self) -> Option<GesturePhase> {
        match self.state {
            State::Armed { .. } => {
                self.state = State::Blocked;
                None
            }
            State::Active { .. } => {
                self.state = State::Blocked;
                Some(GesturePhase::Cancel)
            }
            _ => None,
        }
    }

    pub fn on_mouse_moved(&mut self, mods: u8, at: Point) -> Option<GesturePhase> {
        self.mods?;
        if !self.held(mods) {
            // The release was missed (lost flagsChanged).
            let was_active = self.is_active();
            self.state = State::Idle;
            return was_active.then_some(GesturePhase::Drop { at });
        }
        match self.state {
            State::Idle => {
                if self.exact(mods) {
                    self.state = State::Armed { start: at };
                }
                None
            }
            State::Armed { start } => {
                if !self.exact(mods) {
                    self.state = State::Blocked;
                    return None;
                }
                if dist(start, at) > ACTIVATE_DISTANCE {
                    self.state = State::Active { last: at };
                    return Some(GesturePhase::Begin {
                        at: start,
                        pointer: at,
                    });
                }
                None
            }
            State::Active { last } => {
                if dist(last, at) >= MOVE_STEP {
                    self.state = State::Active { last: at };
                    return Some(GesturePhase::Move { at });
                }
                None
            }
            State::Blocked => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> Point {
        Point { x, y }
    }

    fn active() -> GestureTracker {
        let mut t = GestureTracker::new(Some(ALT));
        assert_eq!(t.on_flags(ALT, p(0.0, 0.0)), None);
        assert_eq!(
            t.on_mouse_moved(ALT, p(30.0, 0.0)),
            Some(GesturePhase::Begin {
                at: p(0.0, 0.0),
                pointer: p(30.0, 0.0)
            })
        );
        t
    }

    #[test]
    fn arm_activate_move_drop() {
        let mut t = active();
        assert_eq!(t.on_mouse_moved(ALT, p(32.0, 0.0)), None, "below MOVE_STEP");
        assert_eq!(
            t.on_mouse_moved(ALT, p(40.0, 0.0)),
            Some(GesturePhase::Move { at: p(40.0, 0.0) })
        );
        assert_eq!(
            t.on_flags(0, p(41.0, 0.0)),
            Some(GesturePhase::Drop { at: p(41.0, 0.0) })
        );
        assert!(!t.is_active());
    }

    #[test]
    fn nothing_below_the_threshold() {
        let mut t = GestureTracker::new(Some(ALT));
        t.on_flags(ALT, p(0.0, 0.0));
        assert_eq!(t.on_mouse_moved(ALT, p(10.0, 10.0)), None);
        assert_eq!(t.on_mouse_moved(ALT, p(16.0, 16.0)), None);
        assert_eq!(t.on_flags(0, p(16.0, 16.0)), None);
    }

    #[test]
    fn key_while_armed_blocks_until_release() {
        let mut t = GestureTracker::new(Some(ALT));
        t.on_flags(ALT, p(0.0, 0.0));
        assert_eq!(t.on_key_down(false), (KeyVerdict::Pass, None));
        assert_eq!(t.on_mouse_moved(ALT, p(100.0, 0.0)), None);
        t.on_flags(0, p(100.0, 0.0));
        // A fresh press works again.
        t.on_flags(ALT, p(100.0, 0.0));
        assert!(matches!(
            t.on_mouse_moved(ALT, p(200.0, 0.0)),
            Some(GesturePhase::Begin { .. })
        ));
    }

    #[test]
    fn mouse_down_while_armed_blocks() {
        let mut t = GestureTracker::new(Some(ALT));
        t.on_flags(ALT, p(0.0, 0.0));
        assert_eq!(t.on_mouse_down(), None);
        assert_eq!(t.on_mouse_moved(ALT, p(100.0, 0.0)), None);
    }

    #[test]
    fn extra_modifier_while_armed_blocks() {
        let mut t = GestureTracker::new(Some(ALT));
        t.on_flags(ALT, p(0.0, 0.0));
        t.on_flags(ALT | SHIFT, p(0.0, 0.0));
        assert_eq!(t.on_mouse_moved(ALT | SHIFT, p(100.0, 0.0)), None);
        t.on_flags(ALT, p(100.0, 0.0));
        assert_eq!(t.on_mouse_moved(ALT, p(200.0, 0.0)), None, "still blocked");
    }

    #[test]
    fn key_while_active_cancels_and_escape_is_swallowed() {
        let mut t = active();
        assert_eq!(
            t.on_key_down(false),
            (KeyVerdict::Pass, Some(GesturePhase::Cancel))
        );
        assert_eq!(t.on_flags(0, p(0.0, 0.0)), None, "no drop after cancel");
        let mut t = active();
        assert_eq!(
            t.on_key_down(true),
            (KeyVerdict::Swallow, Some(GesturePhase::Cancel))
        );
        assert_eq!(t.on_key_down(true), (KeyVerdict::Pass, None));
    }

    #[test]
    fn mouse_down_while_active_cancels() {
        let mut t = active();
        assert_eq!(t.on_mouse_down(), Some(GesturePhase::Cancel));
        assert_eq!(t.on_mouse_moved(ALT, p(300.0, 0.0)), None);
    }

    #[test]
    fn lost_flags_on_move_drops() {
        let mut t = active();
        assert_eq!(
            t.on_mouse_moved(0, p(50.0, 0.0)),
            Some(GesturePhase::Drop { at: p(50.0, 0.0) })
        );
    }

    #[test]
    fn extra_modifiers_while_active_are_ignored() {
        let mut t = active();
        assert_eq!(t.on_flags(ALT | CMD, p(30.0, 0.0)), None);
        assert!(matches!(
            t.on_mouse_moved(ALT | CMD, p(60.0, 0.0)),
            Some(GesturePhase::Move { .. })
        ));
        // Releasing the extra one keeps going; releasing alt drops.
        assert_eq!(t.on_flags(ALT, p(60.0, 0.0)), None);
        assert!(matches!(
            t.on_flags(CMD, p(60.0, 0.0)),
            Some(GesturePhase::Drop { .. })
        ));
    }

    #[test]
    fn caps_lock_is_ignored_and_disabled_does_nothing() {
        let mut t = GestureTracker::new(Some(ALT));
        t.on_flags(ALT | 0x10, p(0.0, 0.0));
        assert!(t.on_mouse_moved(ALT | 0x10, p(50.0, 0.0)).is_some());
        let mut t = GestureTracker::new(None);
        t.on_flags(ALT, p(0.0, 0.0));
        assert_eq!(t.on_mouse_moved(ALT, p(50.0, 0.0)), None);
    }

    #[test]
    fn reset_cancels_an_active_gesture() {
        let mut t = active();
        assert_eq!(t.reset(), Some(GesturePhase::Cancel));
        assert_eq!(t.reset(), None);
    }

    #[test]
    fn wire_format() {
        let j = serde_json::to_string(&GesturePhase::Drop { at: p(1.0, 2.0) }).unwrap();
        assert_eq!(j, r#"{"type":"drop","at":{"x":1.0,"y":2.0}}"#);
        let j = serde_json::to_string(&DragPhase::Release { at: p(1.0, 2.0) }).unwrap();
        assert_eq!(j, r#"{"type":"release","at":{"x":1.0,"y":2.0}}"#);
    }
}
