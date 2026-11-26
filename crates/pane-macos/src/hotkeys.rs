//! Global hotkeys and the hold-the-modifier-and-point gesture, through two
//! CGEventTaps on one thread: an active keyboard tap (it swallows hotkeys)
//! and a listen-only tap for the pointer and modifier keys (it can never
//! delay the cursor).

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use crossbeam_channel::Sender;
use pane_core::config::{keycode, Chord, Mods};
use pane_core::geometry::Point;
use pane_core::gesture::{
    mod_bits, GesturePhase, GestureTracker, KeyVerdict, ALT, CMD, CTRL, SHIFT,
};
use pane_core::proto::Command;

use crate::ffi::*;

/// macOS keycode of Escape.
const KEY_ESCAPE: u16 = 53;

type Table = HashMap<(u16, u8), Command>;

struct TapState {
    table: Arc<RwLock<Table>>,
    suspended: Arc<AtomicBool>,
    gesture: Arc<Mutex<GestureTracker>>,
    tx: Sender<Command>,
}

impl TapState {
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
            st.with_tracker(|t| t.on_mouse_moved(m, p)).flatten()
        }
        kCGEventFlagsChanged => {
            let (m, p) = (event_mods(event), event_point(event));
            st.with_tracker(|t| t.on_flags(m, p)).flatten()
        }
        kCGEventLeftMouseDown | kCGEventRightMouseDown | kCGEventOtherMouseDown => {
            st.with_tracker(|t| t.on_mouse_down()).flatten()
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
        let (ok_tx, ok_rx) = crossbeam_channel::bounded(1);
        let state = Box::new(TapState {
            table: table.clone(),
            suspended: suspended.clone(),
            gesture: tracker.clone(),
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
                    | (1u64 << kCGEventRightMouseDown)
                    | (1u64 << kCGEventOtherMouseDown);
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
                tx,
            }),
            _ => Err("could not create the keyboard event tap (grant Accessibility, and Input Monitoring if asked)".into()),
        }
    }

    pub fn set_suspended(&self, s: bool) {
        self.suspended.store(s, Ordering::Relaxed);
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
