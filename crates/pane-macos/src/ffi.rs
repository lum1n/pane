//! Hand-written declarations for the system calls Pane uses.
#![allow(non_upper_case_globals, non_snake_case, dead_code)]

use std::ffi::c_void;

use core_foundation::array::CFArrayRef;
use core_foundation::base::CFTypeRef;
use core_foundation::dictionary::CFDictionaryRef;
use core_foundation::string::CFStringRef;

pub type Boolean = u8;
pub type AXError = i32;
pub type AXUIElementRef = *const c_void;
pub type AXObserverRef = *const c_void;
pub type AXValueRef = *const c_void;
pub type CFRunLoopRef = *mut c_void;
pub type CFRunLoopSourceRef = *mut c_void;
pub type CFMachPortRef = *mut c_void;
pub type CGEventRef = *mut c_void;
pub type CGDirectDisplayID = u32;

pub const kAXErrorSuccess: AXError = 0;
pub const kAXErrorCannotComplete: AXError = -25204;
pub const kAXErrorNotificationAlreadyRegistered: AXError = -25209;

pub const kAXValueCGPointType: u32 = 1;
pub const kAXValueCGSizeType: u32 = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

pub type AXObserverCallback = unsafe extern "C" fn(
    observer: AXObserverRef,
    element: AXUIElementRef,
    notification: CFStringRef,
    refcon: *mut c_void,
);

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    pub fn AXUIElementCreateApplication(pid: libc::pid_t) -> AXUIElementRef;
    pub fn AXUIElementCreateSystemWide() -> AXUIElementRef;
    pub fn AXUIElementCopyAttributeValue(
        el: AXUIElementRef,
        attr: CFStringRef,
        value: *mut CFTypeRef,
    ) -> AXError;
    pub fn AXUIElementSetAttributeValue(
        el: AXUIElementRef,
        attr: CFStringRef,
        value: CFTypeRef,
    ) -> AXError;
    pub fn AXUIElementIsAttributeSettable(
        el: AXUIElementRef,
        attr: CFStringRef,
        settable: *mut Boolean,
    ) -> AXError;
    pub fn AXUIElementPerformAction(el: AXUIElementRef, action: CFStringRef) -> AXError;
    pub fn AXUIElementSetMessagingTimeout(el: AXUIElementRef, seconds: f32) -> AXError;
    pub fn AXUIElementGetPid(el: AXUIElementRef, pid: *mut libc::pid_t) -> AXError;
    pub fn AXValueCreate(kind: u32, value: *const c_void) -> AXValueRef;
    pub fn AXValueGetValue(v: AXValueRef, kind: u32, out: *mut c_void) -> Boolean;
    pub fn AXIsProcessTrusted() -> Boolean;
    pub fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> Boolean;
    pub fn AXObserverCreate(
        pid: libc::pid_t,
        callback: AXObserverCallback,
        out: *mut AXObserverRef,
    ) -> AXError;
    pub fn AXObserverAddNotification(
        o: AXObserverRef,
        el: AXUIElementRef,
        n: CFStringRef,
        refcon: *mut c_void,
    ) -> AXError;
    pub fn AXObserverRemoveNotification(
        o: AXObserverRef,
        el: AXUIElementRef,
        n: CFStringRef,
    ) -> AXError;
    pub fn AXObserverGetRunLoopSource(o: AXObserverRef) -> CFRunLoopSourceRef;
    /// Private, but stable for over a decade; used by every macOS tiler.
    pub fn _AXUIElementGetWindow(el: AXUIElementRef, wid: *mut u32) -> AXError;
}

#[repr(C)]
pub struct CFRunLoopSourceContext {
    pub version: isize,
    pub info: *mut c_void,
    pub retain: *const c_void,
    pub release: *const c_void,
    pub copy_description: *const c_void,
    pub equal: *const c_void,
    pub hash: *const c_void,
    pub schedule: *const c_void,
    pub cancel: *const c_void,
    pub perform: extern "C" fn(info: *const c_void),
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    pub static kCFRunLoopDefaultMode: CFStringRef;
    pub static kCFRunLoopCommonModes: CFStringRef;
    pub fn CFRunLoopGetCurrent() -> CFRunLoopRef;
    pub fn CFRunLoopGetMain() -> CFRunLoopRef;
    pub fn CFRunLoopRun();
    pub fn CFRunLoopRunInMode(
        mode: CFStringRef,
        seconds: f64,
        return_after_source_handled: Boolean,
    ) -> i32;
    pub fn CFRunLoopStop(rl: CFRunLoopRef);
    pub fn CFRunLoopWakeUp(rl: CFRunLoopRef);
    pub fn CFRunLoopAddSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    pub fn CFRunLoopRemoveSource(rl: CFRunLoopRef, source: CFRunLoopSourceRef, mode: CFStringRef);
    pub fn CFRunLoopSourceCreate(
        alloc: *const c_void,
        order: isize,
        ctx: *mut CFRunLoopSourceContext,
    ) -> CFRunLoopSourceRef;
    pub fn CFRunLoopSourceSignal(source: CFRunLoopSourceRef);
    pub fn CFRunLoopSourceInvalidate(source: CFRunLoopSourceRef);
    pub fn CFMachPortCreateRunLoopSource(
        alloc: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> CFRunLoopSourceRef;
    pub fn CFUUIDCreateString(alloc: *const c_void, uuid: *const c_void) -> CFStringRef;
    pub fn CFRetain(cf: CFTypeRef) -> CFTypeRef;
    pub fn CFRelease(cf: CFTypeRef);
    pub fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> Boolean;
}

pub type CGDisplayReconfigurationCallBack =
    unsafe extern "C" fn(display: CGDirectDisplayID, flags: u32, user: *mut c_void);
pub type CGEventTapCallBack = unsafe extern "C" fn(
    proxy: *mut c_void,
    kind: u32,
    event: CGEventRef,
    user: *mut c_void,
) -> CGEventRef;

pub const kCGSessionEventTap: u32 = 1;
pub const kCGHeadInsertEventTap: u32 = 0;
pub const kCGEventTapOptionDefault: u32 = 0;
pub const kCGEventKeyDown: u32 = 10;
pub const kCGEventTapDisabledByTimeout: u32 = 0xFFFF_FFFE;
pub const kCGEventTapDisabledByUserInput: u32 = 0xFFFF_FFFF;
pub const kCGKeyboardEventKeycode: u32 = 9;
pub const kCGEventFlagMaskShift: u64 = 0x0002_0000;
pub const kCGEventFlagMaskControl: u64 = 0x0004_0000;
pub const kCGEventFlagMaskAlternate: u64 = 0x0008_0000;
pub const kCGEventFlagMaskCommand: u64 = 0x0010_0000;
pub const kCGWindowListOptionAll: u32 = 0;
pub const kCGWindowListExcludeDesktopElements: u32 = 1 << 4;
pub const kCGNullWindowID: u32 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    pub fn CGGetActiveDisplayList(
        max: u32,
        displays: *mut CGDirectDisplayID,
        count: *mut u32,
    ) -> i32;
    pub fn CGDisplayBounds(d: CGDirectDisplayID) -> CGRect;
    pub fn CGMainDisplayID() -> CGDirectDisplayID;
    pub fn CGDisplayRegisterReconfigurationCallback(
        cb: CGDisplayReconfigurationCallBack,
        user: *mut c_void,
    ) -> i32;
    pub fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: CGEventTapCallBack,
        user: *mut c_void,
    ) -> CFMachPortRef;
    pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    pub fn CGEventGetIntegerValueField(ev: CGEventRef, field: u32) -> i64;
    pub fn CGEventGetFlags(ev: CGEventRef) -> u64;
}

#[link(name = "ColorSync", kind = "framework")]
extern "C" {
    pub fn CGDisplayCreateUUIDFromDisplayID(d: CGDirectDisplayID) -> *const c_void;
}
