//! Global hotkeys through a CGEventTap on its own thread.

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, RwLock};

use crossbeam_channel::Sender;
use pane_core::config::{keycode, Chord};
use pane_core::proto::Command;

use crate::ffi::*;

const SHIFT: u8 = 1;
const CTRL: u8 = 2;
const ALT: u8 = 4;
const CMD: u8 = 8;

type Table = HashMap<(u16, u8), Command>;

struct TapState {
    table: Arc<RwLock<Table>>,
    tx: Sender<Command>,
}

static TAP_PORT: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

fn mods_of(c: &Chord) -> u8 {
    (c.shift as u8 * SHIFT) | (c.ctrl as u8 * CTRL) | (c.alt as u8 * ALT) | (c.cmd as u8 * CMD)
}

fn build(bindings: &[(Chord, Command)]) -> Table {
    bindings
        .iter()
        .filter_map(|(c, cmd)| keycode(&c.key).map(|k| ((k, mods_of(c)), cmd.clone())))
        .collect()
}

unsafe extern "C" fn tap_cb(
    _proxy: *mut c_void,
    kind: u32,
    event: CGEventRef,
    user: *mut c_void,
) -> CGEventRef {
    if kind == kCGEventTapDisabledByTimeout || kind == kCGEventTapDisabledByUserInput {
        let port = TAP_PORT.load(Ordering::Acquire);
        if !port.is_null() {
            CGEventTapEnable(port, true);
        }
        return event;
    }
    if kind != kCGEventKeyDown {
        return event;
    }
    let st = &*(user as *const TapState);
    let code = CGEventGetIntegerValueField(event, kCGKeyboardEventKeycode) as u16;
    let flags = CGEventGetFlags(event);
    let mods = ((flags & kCGEventFlagMaskShift != 0) as u8 * SHIFT)
        | ((flags & kCGEventFlagMaskControl != 0) as u8 * CTRL)
        | ((flags & kCGEventFlagMaskAlternate != 0) as u8 * ALT)
        | ((flags & kCGEventFlagMaskCommand != 0) as u8 * CMD);
    if mods == 0 {
        return event;
    }
    let hit = st
        .table
        .read()
        .ok()
        .and_then(|t| t.get(&(code, mods)).cloned());
    match hit {
        Some(cmd) => {
            let _ = st.tx.send(cmd);
            std::ptr::null_mut() // swallow
        }
        None => event,
    }
}

/// Handle to the running tap; update bindings on config reload.
#[derive(Clone)]
pub struct Hotkeys {
    table: Arc<RwLock<Table>>,
}

impl Hotkeys {
    /// Start the tap on a dedicated thread. Fails when macOS refuses the
    /// tap (Accessibility / Input Monitoring not granted).
    pub fn start(bindings: &[(Chord, Command)], tx: Sender<Command>) -> Result<Hotkeys, String> {
        let table = Arc::new(RwLock::new(build(bindings)));
        let (ok_tx, ok_rx) = crossbeam_channel::bounded(1);
        let state = Box::new(TapState {
            table: table.clone(),
            tx,
        });
        std::thread::Builder::new()
            .name("pane-hotkeys".into())
            .spawn(move || unsafe {
                let user = Box::into_raw(state) as *mut c_void;
                let mask = 1u64 << kCGEventKeyDown;
                let port = CGEventTapCreate(
                    kCGSessionEventTap,
                    kCGHeadInsertEventTap,
                    kCGEventTapOptionDefault,
                    mask,
                    tap_cb,
                    user,
                );
                if port.is_null() {
                    let _ = ok_tx.send(false);
                    return;
                }
                TAP_PORT.store(port, Ordering::Release);
                let src = CFMachPortCreateRunLoopSource(std::ptr::null(), port, 0);
                CFRunLoopAddSource(CFRunLoopGetCurrent(), src, kCFRunLoopCommonModes);
                CGEventTapEnable(port, true);
                let _ = ok_tx.send(true);
                CFRunLoopRun();
            })
            .map_err(|e| e.to_string())?;
        match ok_rx.recv() {
            Ok(true) => Ok(Hotkeys { table }),
            _ => Err("could not create the keyboard event tap (grant Accessibility, and Input Monitoring if asked)".into()),
        }
    }

    pub fn update(&self, bindings: &[(Chord, Command)]) {
        if let Ok(mut t) = self.table.write() {
            *t = build(bindings);
        }
    }
}
