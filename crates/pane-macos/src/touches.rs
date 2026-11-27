//! Fingers on the trackpad, read from a gesture event the pointer tap saw
//! (`NSEvent.allTouches`). No private MultitouchSupport framework.

use objc2::encode::{Encoding, RefEncode};
use objc2::rc::autoreleasepool;
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSPoint;
use pane_core::swipe::Touch;

use crate::ffi::CGEventRef;

/// `NSTouchPhaseEnded | NSTouchPhaseCancelled`.
const LIFTED: usize = 8 | 16;

/// The opaque struct behind `CGEventRef`, so the message send carries the
/// encoding `+[NSEvent eventWithCGEvent:]` declares.
#[repr(C)]
struct CGEventOpaque {
    _private: [u8; 0],
}

unsafe impl RefEncode for CGEventOpaque {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
}

/// The fingers still touching in `event` (an `NSEventTypeGesture` event).
///
/// # Safety
/// `event` must be a live CGEvent.
pub unsafe fn touching(event: CGEventRef) -> Vec<Touch> {
    autoreleasepool(|_| {
        let ns: *mut AnyObject =
            msg_send![class!(NSEvent), eventWithCGEvent: event as *mut CGEventOpaque];
        if ns.is_null() {
            return vec![];
        }
        let set: *mut AnyObject = msg_send![ns, allTouches];
        if set.is_null() {
            return vec![];
        }
        let all: *mut AnyObject = msg_send![set, allObjects];
        let count: usize = msg_send![all, count];
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let t: *mut AnyObject = msg_send![all, objectAtIndex: i];
            let phase: usize = msg_send![t, phase];
            if phase & LIFTED != 0 {
                continue;
            }
            let p: NSPoint = msg_send![t, normalizedPosition];
            let identity: *mut AnyObject = msg_send![t, identity];
            let id: usize = msg_send![identity, hash];
            out.push(Touch {
                id: id as u64,
                x: p.x,
                y: p.y,
            });
        }
        out
    })
}
