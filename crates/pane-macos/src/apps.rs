//! Which applications are running and which of them own windows.

use std::collections::BTreeSet;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, TCFType};
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::NSString;
use pane_core::{Rect, WindowId};

use crate::ffi::*;

#[derive(Clone, Debug, PartialEq)]
pub struct RunningApp {
    pub pid: i32,
    pub bundle_id: Option<String>,
    pub name: String,
    /// NSApplicationActivationPolicyRegular.
    pub regular: bool,
}

/// One entry of CGWindowListCopyWindowInfo.
#[derive(Clone, Debug)]
pub struct CgWindow {
    pub wid: WindowId,
    pub pid: i32,
    pub layer: i64,
    pub bounds: Rect,
    pub owner: String,
    pub title: String,
}

fn num(d: &CFDictionary<CFString, CFType>, key: &'static str) -> Option<i64> {
    d.find(CFString::from_static_string(key))
        .and_then(|v| v.downcast::<CFNumber>())
        .and_then(|n| n.to_i64())
}

fn fnum(d: &CFDictionary<CFString, CFType>, key: &'static str) -> Option<f64> {
    d.find(CFString::from_static_string(key))
        .and_then(|v| v.downcast::<CFNumber>())
        .and_then(|n| n.to_f64())
}

fn string(d: &CFDictionary<CFString, CFType>, key: &'static str) -> String {
    d.find(CFString::from_static_string(key))
        .and_then(|v| v.downcast::<CFString>())
        .map(|s| s.to_string())
        .unwrap_or_default()
}

/// All windows the window server knows about, across Spaces.
pub fn window_list() -> Vec<CgWindow> {
    let raw = unsafe {
        CGWindowListCopyWindowInfo(
            kCGWindowListOptionAll | kCGWindowListExcludeDesktopElements,
            kCGNullWindowID,
        )
    };
    if raw.is_null() {
        return vec![];
    }
    let arr: CFArray<CFType> = unsafe { CFArray::wrap_under_create_rule(raw as CFArrayRef) };
    let mut out = Vec::new();
    for item in arr.iter() {
        let d: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(item.as_CFTypeRef() as CFDictionaryRef) };
        let bounds = d
            .find(CFString::from_static_string("kCGWindowBounds"))
            .map(|b| {
                let bd: CFDictionary<CFString, CFType> = unsafe {
                    CFDictionary::wrap_under_get_rule(b.as_CFTypeRef() as CFDictionaryRef)
                };
                Rect::new(
                    fnum(&bd, "X").unwrap_or(0.0),
                    fnum(&bd, "Y").unwrap_or(0.0),
                    fnum(&bd, "Width").unwrap_or(0.0),
                    fnum(&bd, "Height").unwrap_or(0.0),
                )
            })
            .unwrap_or_default();
        out.push(CgWindow {
            wid: WindowId(num(&d, "kCGWindowNumber").unwrap_or(0) as u32),
            pid: num(&d, "kCGWindowOwnerPID").unwrap_or(0) as i32,
            layer: num(&d, "kCGWindowLayer").unwrap_or(0),
            bounds,
            owner: string(&d, "kCGWindowOwnerName"),
            title: string(&d, "kCGWindowName"),
        });
    }
    out
}

/// Pids owning at least one normal-layer window, excluding ourselves.
pub fn window_owner_pids() -> BTreeSet<i32> {
    let me = std::process::id() as i32;
    window_list()
        .into_iter()
        .filter(|w| w.layer == 0 && w.pid != me && w.pid > 0)
        .map(|w| w.pid)
        .collect()
}

pub fn app_info(pid: i32) -> Option<RunningApp> {
    autoreleasepool(|_| unsafe {
        let app: Option<Retained<AnyObject>> =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        let app = app?;
        let terminated: bool = msg_send![&*app, isTerminated];
        if terminated {
            return None;
        }
        let policy: isize = msg_send![&*app, activationPolicy];
        // NSApplicationActivationPolicyProhibited: background processes that
        // cannot own normal windows.
        if policy == 2 {
            return None;
        }
        let bundle: Option<Retained<NSString>> = msg_send![&*app, bundleIdentifier];
        let name: Option<Retained<NSString>> = msg_send![&*app, localizedName];
        // Unbundled binaries (`cargo run`) have no bundle id: use the
        // executable's file name so rules and placements can name them.
        let exe = || -> Option<String> {
            let url: Option<Retained<AnyObject>> = msg_send![&*app, executableURL];
            let url = url?;
            let last: Option<Retained<NSString>> = msg_send![&*url, lastPathComponent];
            last.map(|s| s.to_string()).filter(|s| !s.is_empty())
        };
        let bundle_id = bundle
            .map(|b| b.to_string())
            .filter(|b| !b.is_empty())
            .or_else(exe);
        let name = name.map(|n| n.to_string()).filter(|n| !n.is_empty());
        Some(RunningApp {
            pid,
            name: name.or_else(|| bundle_id.clone()).unwrap_or_default(),
            bundle_id,
            regular: policy == 0,
        })
    })
}

/// Apps that own windows right now.
pub fn running_apps() -> Vec<RunningApp> {
    window_owner_pids()
        .into_iter()
        .filter_map(app_info)
        .collect()
}

pub fn is_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 || *libc::__error() == libc::EPERM }
}

/// Bring an app to the front (used together with AXRaise).
pub fn activate(pid: i32) {
    autoreleasepool(|_| unsafe {
        let app: Option<Retained<AnyObject>> =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        if let Some(app) = app {
            // NSApplicationActivateAllWindows is not wanted; 0 = default.
            let _: bool = msg_send![&*app, activateWithOptions: 0usize];
        }
    })
}

/// Activate Finder, which gives keyboard focus to the desktop when none of
/// its windows is in front.
pub fn activate_finder() {
    autoreleasepool(|_| unsafe {
        let bundle = NSString::from_str("com.apple.finder");
        let list: Option<Retained<AnyObject>> = msg_send![class!(NSRunningApplication), runningApplicationsWithBundleIdentifier: &*bundle];
        let Some(list) = list else { return };
        let count: usize = msg_send![&*list, count];
        if count == 0 {
            return;
        }
        let app: *mut AnyObject = msg_send![&*list, objectAtIndex: 0usize];
        if !app.is_null() {
            let _: bool = msg_send![app, activateWithOptions: 0usize];
        }
    })
}
