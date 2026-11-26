//! A CFRunLoop source that lets other threads wake a run loop and have a
//! closure run on it.

use std::ffi::c_void;

use crate::ffi::*;

/// Wakes a thread's run loop. Created on that thread; sendable anywhere.
pub struct Waker {
    rl: CFRunLoopRef,
    source: CFRunLoopSourceRef,
}

// CFRunLoopWakeUp and CFRunLoopSourceSignal are documented thread-safe.
unsafe impl Send for Waker {}
unsafe impl Sync for Waker {}

struct Callback(Box<dyn FnMut()>);

extern "C" fn perform(info: *const c_void) {
    // SAFETY: `info` is the leaked Box<Callback> owned by the source, used
    // only on the run loop's own thread.
    let cb = unsafe { &mut *(info as *mut Callback) };
    (cb.0)();
}

impl Waker {
    /// Install a source on the current thread's run loop that calls `f`
    /// whenever [`Waker::wake`] is called from any thread.
    pub fn install(f: impl FnMut() + 'static) -> Waker {
        let info = Box::into_raw(Box::new(Callback(Box::new(f)))) as *mut c_void;
        let mut ctx = CFRunLoopSourceContext {
            version: 0,
            info,
            retain: std::ptr::null(),
            release: std::ptr::null(),
            copy_description: std::ptr::null(),
            equal: std::ptr::null(),
            hash: std::ptr::null(),
            schedule: std::ptr::null(),
            cancel: std::ptr::null(),
            perform,
        };
        unsafe {
            let source = CFRunLoopSourceCreate(std::ptr::null(), 0, &mut ctx);
            let rl = CFRunLoopGetCurrent();
            // Keep the run loop object alive after its thread exits, so a late
            // wake() from another thread is harmless.
            CFRetain(rl as _);
            CFRunLoopAddSource(rl, source, kCFRunLoopDefaultMode);
            Waker { rl, source }
        }
    }

    pub fn wake(&self) {
        unsafe {
            CFRunLoopSourceSignal(self.source);
            CFRunLoopWakeUp(self.rl);
        }
    }

    pub fn stop(&self) {
        unsafe { CFRunLoopStop(self.rl) }
    }

    /// Stop calling the closure (the source stays allocated until drop).
    pub fn invalidate(&self) {
        unsafe { CFRunLoopSourceInvalidate(self.source) }
    }
}

impl Drop for Waker {
    fn drop(&mut self) {
        // No invalidate here: a signalled source must still deliver (the
        // worker's Quit). The run loop holds its own reference to the source.
        unsafe {
            CFRelease(self.source as _);
            CFRelease(self.rl as _);
        }
    }
}

impl Waker {
    /// Stop the run loop of the calling thread.
    pub fn stop_current() {
        unsafe { CFRunLoopStop(CFRunLoopGetCurrent()) }
    }
}

pub fn run_current() {
    unsafe { CFRunLoopRun() }
}
