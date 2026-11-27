//! Global hotkeys and the hold-the-modifier-and-point gesture, through two
//! CGEventTaps on one thread: an active keyboard tap (it swallows hotkeys)
//! and a listen-only tap for the pointer, modifier keys and trackpad
//! touches (it can never delay the cursor or take a system gesture away).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crossbeam_channel::Sender;
use pane_core::config::{keycode, Chord, Mods};
use pane_core::geometry::Point;
use pane_core::gesture::{
    mod_bits, DragPhase, GesturePhase, GestureTracker, KeyVerdict, ALT, CMD, CTRL, SHIFT,
};
use pane_core::proto::Command;
use pane_core::swipe::SwipeTracker;

use crate::ffi::*;

/// macOS keycode of Escape.
const KEY_ESCAPE: u16 = 53;
/// Pointer travel (points) between two `Command::Hover`s.
const HOVER_STEP: f64 = 4.0;
/// Pointer travel (points) between two `DragPhase::Move`s.
const DRAG_STEP: f64 = 4.0;

type Table = HashMap<(u16, u8), Command>;

struct TapState {
    table: Arc<RwLock<Table>>,
    suspended: Arc<AtomicBool>,
    gesture: Arc<Mutex<GestureTracker>>,
    /// Focus follows mouse is on: report pointer moves.
    hover: Arc<AtomicBool>,
    /// Where the last `Hover` was sent (tap thread only).
    last_hover: Cell<Option<Point>>,
    /// Title-bar drags are on: report left-button drags and releases.
    drag: Arc<AtomicBool>,
    /// Where the last `DragPhase::Move` was sent (tap thread only).
    last_drag: Cell<Option<Point>>,
    /// Trackpad swipes are on: watch gesture events' touches.
    swipes: Arc<AtomicBool>,
    /// Tap thread only.
    swipe: RefCell<SwipeTracker>,
    tx: Sender<Command>,
}

impl TapState {
    /// Focus follows mouse: report where the pointer went, every few points,
    /// unless the gesture modifier is held.
    fn hover(&self, at: Point) {
        if !self.hover.load(Ordering::Relaxed) || self.with_tracker(|t| t.is_busy()) != Some(false)
        {
            return;
        }
        let far = self
            .last_hover
            .get()
            .is_none_or(|p| (p.x - at.x).hypot(p.y - at.y) >= HOVER_STEP);
        if far {
            self.last_hover.set(Some(at));
            let _ = self.tx.send(Command::Hover { at });
        }
    }

    /// The pointer moved with the left button held (or, `up`, let go).
    fn drag(&self, at: Point, up: bool) {
        if !self.drag.load(Ordering::Relaxed) {
            return;
        }
        let phase = if up {
            self.last_drag.set(None);
            DragPhase::Release { at }
        } else {
            let far = self
                .last_drag
                .get()
                .is_none_or(|p| (p.x - at.x).hypot(p.y - at.y) >= DRAG_STEP);
            if !far {
                return;
            }
            self.last_drag.set(Some(at));
            DragPhase::Move { at }
        };
        let _ = self.tx.send(Command::Drag { phase });
    }

    /// A trackpad gesture event: feed its touches to the swipe tracker.
    unsafe fn touches(&self, event: CGEventRef) {
        if !self.swipes.load(Ordering::Relaxed) {
            return;
        }
        let touches = crate::touches::touching(event);
        let swipe = self.swipe.borrow_mut().on_touches(&touches);
        if let Some(swipe) = swipe {
            let at = event_point(event);
            let _ = self.tx.send(Command::Swipe { swipe, at });
        }
    }

    fn send_phase(&self, phase: Option<GesturePhase>) {
        if let Some(phase) = phase {
            let _ = self.tx.send(Command::Gesture { phase });
        }
    }

    fn with_tracker<T>(&self, f: impl FnOnce(&mut GestureTracker) -> T) -> Option<T> {
        self.gesture.lock().ok().map(|mut t| f(&mut t))
    }
}

static KEY_PORT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static MOUSE_PORT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn mods_of(c: &Chord) -> u8 {
    (c.shift as u8 * SHIFT) | (c.ctrl as u8 * CTRL) | (c.alt as u8 * ALT) | (c.cmd as u8 * CMD)
}

fn build(bindings: &[(Chord, Command)]) -> Table {
    bindings
        .iter()
        .filter_map(|(c, cmd)| keycode(&c.key).map(|k| ((k, mods_of(c)), cmd.clone())))
        .collect()
}

unsafe fn event_mods(event: CGEventRef) -> u8 {
    let flags = CGEventGetFlags(event);
    ((flags & kCGEventFlagMaskShift != 0) as u8 * SHIFT)
        | ((flags & kCGEventFlagMaskControl != 0) as u8 * CTRL)
        | ((flags & kCGEventFlagMaskAlternate != 0) as u8 * ALT)
        | ((flags & kCGEventFlagMaskCommand != 0) as u8 * CMD)
}

unsafe fn event_point(event: CGEventRef) -> Point {
    let p = CGEventGetLocation(event);
    Point { x: p.x, y: p.y }
}

/// macOS turned a tap off (it was too slow, or user input): turn both back
/// on and forget any half-seen gesture.
unsafe fn reenable(st: &TapState) {
    for port in [&KEY_PORT, &MOUSE_PORT] {
        let p = port.load(Ordering::Acquire);
        if !p.is_null() {
            CGEventTapEnable(p, true);
        }
    }
    st.swipe.borrow_mut().reset();
    st.send_phase(st.with_tracker(|t| t.reset()).flatten());
}

unsafe extern "C" fn key_cb(
    _proxy: *mut c_void,
    kind: u32,
    event: CGEventRef,
    user: *mut c_void,
) -> CGEventRef {
    let st = &*(user as *const TapState);
    if kind == kCGEventTapDisabledByTimeout || kind == kCGEventTapDisabledByUserInput {
        reenable(st);
        return event;
    }
    if kind != kCGEventKeyDown {
        return event;
    }
    let code = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) as u16;
    let mods = event_mods(event);
    let hit = if mods == 0 || st.suspended.load(Ordering::Relaxed) {
        None
    } else {
        st.table
            .read()
            .ok()
            .and_then(|t| t.get(&(code, mods)).cloned())
    };
    // Any key ends (or rules out) a gesture; Escape is eaten when it
    // cancels one.
    let (verdict, phase) = st
        .with_tracker(|t| t.on_key_down(code == KEY_ESCAPE && hit.is_none()))
        .unwrap_or((KeyVerdict::Pass, None));
    st.send_phase(phase);
    match hit {
        Some(cmd) => {
            let _ = st.tx.send(cmd);
            std::ptr::null_mut() // swallow
        }
        None if verdict == KeyVerdict::Swallow => std::ptr::null_mut(),
        None => event,
    }
}

#[allow(non_upper_case_globals)] // CG constants as patterns
unsafe extern "C" fn mouse_cb(
    _proxy: *mut c_void,
    kind: u32,
    event: CGEventRef,
    user: *mut c_void,
) -> CGEventRef {
    let st = &*(user as *const TapState);
    let phase = match kind {
        kCGEventTapDisabledByTimeout | kCGEventTapDisabledByUserInput => {
            reenable(st);
            return event;
        }
        kCGEventMouseMoved => {
            let (m, p) = (event_mods(event), event_point(event));
            let phase = st.with_tracker(|t| t.on_mouse_moved(m, p)).flatten();
            st.hover(p);
            phase
        }
        kCGEventFlagsChanged => {
            let (m, p) = (event_mods(event), event_point(event));
            st.with_tracker(|t| t.on_flags(m, p)).flatten()
        }
        kCGEventLeftMouseDragged => {
            st.drag(event_point(event), false);
            None
        }
        kCGEventLeftMouseUp => {
            st.drag(event_point(event), true);
            None
        }
        kCGEventLeftMouseDown | kCGEventRightMouseDown | kCGEventOtherMouseDown => {
            st.with_tracker(|t| t.on_mouse_down()).flatten()
        }
        kCGEventGesture => {
            st.touches(event);
            None
        }
        _ => None,
    };
    st.send_phase(phase);
    event
}

/// Handle to the running taps; update bindings on config reload.
#[derive(Clone)]
pub struct Hotkeys {
    table: Arc<RwLock<Table>>,
    suspended: Arc<AtomicBool>,
    gesture: Arc<Mutex<GestureTracker>>,
    hover: Arc<AtomicBool>,
    drag: Arc<AtomicBool>,
    swipes: Arc<AtomicBool>,
    tx: Sender<Command>,
}

impl Hotkeys {
    /// Start the taps on a dedicated thread. Fails when macOS refuses the
    /// keyboard tap (Accessibility / Input Monitoring not granted). Without
    /// the pointer tap, hotkeys still work but gestures do not.
    pub fn start(
        bindings: &[(Chord, Command)],
        gesture: Option<Mods>,
        tx: Sender<Command>,
    ) -> Result<Hotkeys, String> {
        let table = Arc::new(RwLock::new(build(bindings)));
        let suspended = Arc::new(AtomicBool::new(false));
        let tracker = Arc::new(Mutex::new(GestureTracker::new(
            gesture.as_ref().map(mod_bits),
        )));
        let hover = Arc::new(AtomicBool::new(false));
        let drag = Arc::new(AtomicBool::new(false));
        let swipes = Arc::new(AtomicBool::new(false));
        let (ok_tx, ok_rx) = crossbeam_channel::bounded(1);
        let state = Box::new(TapState {
            table: table.clone(),
            suspended: suspended.clone(),
            gesture: tracker.clone(),
            hover: hover.clone(),
            last_hover: Cell::new(None),
            drag: drag.clone(),
            last_drag: Cell::new(None),
            swipes: swipes.clone(),
            swipe: RefCell::new(SwipeTracker::new()),
            tx: tx.clone(),
        });
        std::thread::Builder::new()
            .name("pane-hotkeys".into())
            .spawn(move || unsafe {
                // Lives as long as the thread (the process).
                let user = Box::into_raw(state) as *mut c_void;
                let port = CGEventTapCreate(
                    kCGSessionEventTap,
                    kCGHeadInsertEventTap,
                    kCGEventTapOptionDefault,
                    1u64 << kCGEventKeyDown,
                    key_cb,
                    user,
                );
                if port.is_null() {
                    let _ = ok_tx.send(false);
                    return;
                }
                KEY_PORT.store(port, Ordering::Release);
                let rl = CFRunLoopGetCurrent();
                let src = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
                CFRunLoopAddSource(rl, src, kCFRunLoopCommonModes);
                CGEventTapEnable(port, true);

                let mouse_mask = (1u64 << kCGEventMouseMoved)
                    | (1u64 << kCGEventFlagsChanged)
                    | (1u64 << kCGEventLeftMouseDown)
                    | (1u64 << kCGEventLeftMouseDragged)
                    | (1u64 << kCGEventLeftMouseUp)
                    | (1u64 << kCGEventRightMouseDown)
                    | (1u64 << kCGEventOtherMouseDown)
                    | (1u64 << kCGEventGesture);
                let mport = CGEventTapCreate(
                    kCGSessionEventTap,
                    kCGTailAppendEventTap,
                    kCGEventTapOptionListenOnly,
                    mouse_mask,
                    mouse_cb,
                    user,
                );
                if mport.is_null() {
                    tracing::warn!("could not create the pointer event tap; gestures are off");
                } else {
                    MOUSE_PORT.store(mport, Ordering::Release);
                    let msrc = CFMachPortCreateRunLoopSource(std::ptr::null(), mport, 0);
                    CFRunLoopAddSource(rl, msrc, kCFRunLoopCommonModes);
                    CGEventTapEnable(mport, true);
                }
                let _ = ok_tx.send(true);
                CFRunLoopRun();
            })
            .map_err(|e| e.to_string())?;
        match ok_rx.recv() {
            Ok(true) => Ok(Hotkeys {
                table,
                suspended,
                gesture: tracker,
                hover,
                drag,
                swipes,
                tx,
            }),
            _ => Err("could not create the keyboard event tap (grant Accessibility, and Input Monitoring if asked)".into()),
        }
    }

    pub fn set_suspended(&self, s: bool) {
        self.suspended.store(s, Ordering::Relaxed);
    }

    /// Focus follows mouse: send `Command::Hover` as the pointer moves.
    pub fn set_hover(&self, on: bool) {
        self.hover.store(on, Ordering::Relaxed);
    }

    /// Title-bar drags: send `Command::Drag` while the left button is held.
    pub fn set_drag(&self, on: bool) {
        self.drag.store(on, Ordering::Relaxed);
    }

    /// Trackpad swipes: send `Command::Swipe` for three-finger left/right
    /// and four-finger up.
    pub fn set_swipes(&self, on: bool) {
        self.swipes.store(on, Ordering::Relaxed);
    }

    pub fn update(&self, bindings: &[(Chord, Command)], gesture: Option<Mods>) {
        if let Ok(mut t) = self.table.write() {
            *t = build(bindings);
        }
        let phase = self
            .gesture
            .lock()
            .ok()
            .and_then(|mut t| t.set_mods(gesture.as_ref().map(mod_bits)));
        if let Some(phase) = phase {
            let _ = self.tx.send(Command::Gesture { phase });
        }
    }
}
