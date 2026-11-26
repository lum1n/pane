//! AXObserver on the current thread's run loop.

use std::cell::RefCell;
use std::ffi::c_void;

use core_foundation::base::TCFType;
use core_foundation::string::{CFString, CFStringRef};

use crate::ax::{cfstr, AxElement};
use crate::ffi::*;

pub const APP_NOTIFICATIONS: &[&str] = &[
    "AXWindowCreated",
    "AXFocusedWindowChanged",
    "AXApplicationActivated",
    "AXApplicationShown",
];

pub const WINDOW_NOTIFICATIONS: &[&str] = &[
    "AXUIElementDestroyed",
    "AXWindowMoved",
    "AXWindowResized",
    "AXTitleChanged",
    "AXWindowMiniaturized",
    "AXWindowDeminiaturized",
];

type Handler = Box<dyn FnMut(AxElement, &str)>;

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = const { RefCell::new(None) };
}

unsafe extern "C" fn callback(
    _o: AXObserverRef,
    el: AXUIElementRef,
    n: CFStringRef,
    _refcon: *mut c_void,
) {
    let Some(el) = AxElement::from_get(el) else {
        return;
    };
    let name = CFString::wrap_under_get_rule(n).to_string();
    HANDLER.with(|h| {
        // A handler that re-enters the run loop would find the cell borrowed;
        // dropping such a nested notification is harmless.
        if let Ok(mut h) = h.try_borrow_mut() {
            if let Some(f) = h.as_mut() {
                f(el, &name);
            }
        }
    });
}

pub struct Observer {
    obs: AXObserverRef,
}

impl Observer {
    /// Create an observer for `pid`, attach it to this thread's run loop and
    /// route its notifications to `handler` (on this thread).
    pub fn new(pid: i32, handler: impl FnMut(AxElement, &str) + 'static) -> Option<Observer> {
        let mut obs: AXObserverRef = std::ptr::null();
        unsafe {
            if AXObserverCreate(pid, callback, &mut obs) != kAXErrorSuccess || obs.is_null() {
                return None;
            }
            CFRunLoopAddSource(
                CFRunLoopGetCurrent(),
                AXObserverGetRunLoopSource(obs),
                kCFRunLoopDefaultMode,
            );
        }
        HANDLER.with(|h| *h.borrow_mut() = Some(Box::new(handler)));
        Some(Observer { obs })
    }

    /// Returns false when the app is not ready yet (worth retrying).
    pub fn add(&self, el: &AxElement, notification: &str) -> bool {
        let n = cfstr(notification);
        let r = unsafe {
            AXObserverAddNotification(
                self.obs,
                el.as_ptr(),
                n.as_concrete_TypeRef(),
                std::ptr::null_mut(),
            )
        };
        r == kAXErrorSuccess || r == kAXErrorNotificationAlreadyRegistered
    }

    pub fn remove(&self, el: &AxElement, notification: &str) {
        let n = cfstr(notification);
        unsafe {
            AXObserverRemoveNotification(self.obs, el.as_ptr(), n.as_concrete_TypeRef());
        }
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        HANDLER.with(|h| *h.borrow_mut() = None);
        unsafe {
            CFRunLoopRemoveSource(
                CFRunLoopGetCurrent(),
                AXObserverGetRunLoopSource(self.obs),
                kCFRunLoopDefaultMode,
            );
            CFRelease(self.obs as _);
        }
    }
}
