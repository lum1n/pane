//! Moving the pointer (mouse follows focus).

use core_foundation::base::CFTypeRef;
use pane_core::geometry::{Point, Rect};

use crate::ffi::*;

/// Where the pointer is now, in global top-left points.
pub fn location() -> Option<Point> {
    unsafe {
        let ev = CGEventCreate(std::ptr::null());
        if ev.is_null() {
            return None;
        }
        let p = CGEventGetLocation(ev);
        CFRelease(ev as CFTypeRef);
        Some(Point { x: p.x, y: p.y })
    }
}

/// Put the pointer in the middle of `rect`, unless it is already inside
/// (pointing at the window you focused should not make it jump).
pub fn warp_into(rect: Rect) {
    if rect.w <= 0.0 || rect.h <= 0.0 || location().is_some_and(|p| rect.contains(p)) {
        return;
    }
    let c = rect.center();
    unsafe {
        CGWarpMouseCursorPosition(CGPoint { x: c.x, y: c.y });
        // A warp freezes pointer input for a moment; reconnecting the mouse
        // ends that at once.
        CGAssociateMouseAndMouseCursorPosition(1);
    }
}
