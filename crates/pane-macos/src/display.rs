//! Displays in global top-left points, and change notifications.

use std::ffi::c_void;

use core_foundation::base::TCFType;
use core_foundation::string::CFString;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSRect, NSString};
use pane_core::display::{rank_displays, Display};
use pane_core::Rect;

use crate::ffi::*;

struct ScreenInfo {
    number: u32,
    /// Bottom-left Cocoa coordinates.
    frame: NSRect,
    visible: NSRect,
    name: String,
    scale: f64,
}

fn screens() -> Vec<ScreenInfo> {
    autoreleasepool(|_| unsafe {
        let arr: Option<Retained<AnyObject>> = msg_send![class!(NSScreen), screens];
        let Some(arr) = arr else { return vec![] };
        let count: usize = msg_send![&*arr, count];
        let key = NSString::from_str("NSScreenNumber");
        let mut out = Vec::new();
        for i in 0..count {
            let s: *mut AnyObject = msg_send![&*arr, objectAtIndex: i];
            if s.is_null() {
                continue;
            }
            let desc: *mut AnyObject = msg_send![s, deviceDescription];
            let num: *mut AnyObject = msg_send![desc, objectForKey: &*key];
            let number: u32 = if num.is_null() {
                0
            } else {
                msg_send![num, unsignedIntValue]
            };
            let frame: NSRect = msg_send![s, frame];
            let visible: NSRect = msg_send![s, visibleFrame];
            let scale: f64 = msg_send![s, backingScaleFactor];
            let name: Option<Retained<NSString>> = msg_send![s, localizedName];
            out.push(ScreenInfo {
                number,
                frame,
                visible,
                name: name.map(|n| n.to_string()).unwrap_or_default(),
                scale,
            });
        }
        out
    })
}

fn uuid_string(d: CGDirectDisplayID) -> String {
    unsafe {
        let u = CGDisplayCreateUUIDFromDisplayID(d);
        if u.is_null() {
            return format!("display-{d}");
        }
        let s = CFUUIDCreateString(std::ptr::null(), u);
        CFRelease(u);
        if s.is_null() {
            return format!("display-{d}");
        }
        CFString::wrap_under_create_rule(s).to_string()
    }
}

/// Connected displays, ranked left to right.
///
/// Frames come from CGDisplayBounds (top-left). NSScreen is used only for
/// the visible-frame insets (menu bar, Dock), names and scale; it is matched
/// via NSScreenNumber and converted with `top = primary.height - (y + h)`.
pub fn displays() -> Vec<Display> {
    let mut ids = [0u32; 32];
    let mut count = 0u32;
    unsafe {
        if CGGetActiveDisplayList(ids.len() as u32, ids.as_mut_ptr(), &mut count) != 0 {
            return vec![];
        }
    }
    let main = unsafe { CGMainDisplayID() };
    let screens = screens();
    let mut out: Vec<Display> = ids[..count as usize]
        .iter()
        .map(|&id| {
            let b = unsafe { CGDisplayBounds(id) };
            let frame = Rect::new(b.origin.x, b.origin.y, b.size.width, b.size.height);
            let s = screens.iter().find(|s| s.number == id);
            let visible_frame = s
                .map(|s| {
                    // Insets relative to NSScreen.frame, applied to the CG frame.
                    let f = s.frame;
                    let v = s.visible;
                    let left = v.origin.x - f.origin.x;
                    let bottom = v.origin.y - f.origin.y;
                    let right = (f.origin.x + f.size.width) - (v.origin.x + v.size.width);
                    let top = (f.origin.y + f.size.height) - (v.origin.y + v.size.height);
                    Rect::new(
                        frame.x + left,
                        frame.y + top,
                        frame.w - left - right,
                        frame.h - top - bottom,
                    )
                })
                .unwrap_or(frame);
            Display {
                id: uuid_string(id),
                name: s
                    .map(|s| s.name.clone())
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| format!("Display {id}")),
                frame,
                visible_frame,
                is_main: id == main,
                scale: s.map(|s| s.scale).unwrap_or(1.0),
                position_rank: 0,
            }
        })
        .collect();
    rank_displays(&mut out);
    out
}

/// Convert a Cocoa (bottom-left) rect to global top-left points.
pub fn cocoa_to_global(r: NSRect) -> Rect {
    let primary_h = unsafe { CGDisplayBounds(CGMainDisplayID()).size.height };
    Rect::new(
        r.origin.x,
        primary_h - (r.origin.y + r.size.height),
        r.size.width,
        r.size.height,
    )
}

type ChangeFn = Box<dyn Fn() + Send + Sync>;

unsafe extern "C" fn reconfigured(_d: CGDirectDisplayID, flags: u32, user: *mut c_void) {
    const BEGIN: u32 = 1;
    if flags & BEGIN != 0 {
        return;
    }
    let f = &*(user as *const ChangeFn);
    f();
}

/// Call `f` (on the main run loop) after every display reconfiguration.
/// Calls come in bursts; the runtime debounces them.
pub fn watch(f: impl Fn() + Send + Sync + 'static) {
    let boxed: Box<ChangeFn> = Box::new(Box::new(f));
    unsafe {
        CGDisplayRegisterReconfigurationCallback(reconfigured, Box::into_raw(boxed) as *mut c_void);
    }
}
