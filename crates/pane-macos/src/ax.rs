//! Thin, safe-ish wrapper over AXUIElement.

use std::ffi::c_void;

use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::CFDictionary;
use core_foundation::string::CFString;
use core_foundation::url::CFURL;
use pane_core::{Rect, WindowId};

use crate::ffi::*;

/// A retained AXUIElementRef.
pub struct AxElement(AXUIElementRef);

// AXUIElementRef is a CF object; retain/release are thread-safe. Pane only
// *uses* an element on its owning worker thread (and in emergency restore).
unsafe impl Send for AxElement {}
unsafe impl Sync for AxElement {}

impl Clone for AxElement {
    fn clone(&self) -> Self {
        unsafe { CFRetain(self.0 as CFTypeRef) };
        AxElement(self.0)
    }
}

impl Drop for AxElement {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0 as CFTypeRef) };
        }
    }
}

impl PartialEq for AxElement {
    fn eq(&self, o: &Self) -> bool {
        unsafe { CFEqual(self.0 as CFTypeRef, o.0 as CFTypeRef) != 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxErr(pub AXError);

impl std::fmt::Display for AxErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AXError {}", self.0)
    }
}

fn check(e: AXError) -> Result<(), AxErr> {
    if e == kAXErrorSuccess {
        Ok(())
    } else {
        Err(AxErr(e))
    }
}

pub fn cfstr(s: &str) -> CFString {
    CFString::new(s)
}

impl AxElement {
    /// Take ownership of a +1 reference.
    ///
    /// # Safety
    /// `r` must be null or a valid AXUIElementRef the caller owns.
    pub unsafe fn from_create(r: AXUIElementRef) -> Option<AxElement> {
        (!r.is_null()).then_some(AxElement(r))
    }

    /// Retain a +0 reference.
    ///
    /// # Safety
    /// `r` must be null or a valid AXUIElementRef.
    pub unsafe fn from_get(r: AXUIElementRef) -> Option<AxElement> {
        if r.is_null() {
            return None;
        }
        CFRetain(r as CFTypeRef);
        Some(AxElement(r))
    }

    pub fn application(pid: i32) -> AxElement {
        unsafe {
            AxElement::from_create(AXUIElementCreateApplication(pid))
                .expect("AXUIElementCreateApplication")
        }
    }

    pub fn as_ptr(&self) -> AXUIElementRef {
        self.0
    }

    pub fn set_timeout(&self, seconds: f32) {
        unsafe {
            AXUIElementSetMessagingTimeout(self.0, seconds);
        }
    }

    pub fn attribute(&self, name: &str) -> Result<CFType, AxErr> {
        let mut out: CFTypeRef = std::ptr::null();
        let attr = cfstr(name);
        check(unsafe {
            AXUIElementCopyAttributeValue(self.0, attr.as_concrete_TypeRef(), &mut out)
        })?;
        if out.is_null() {
            return Err(AxErr(kAXErrorCannotComplete));
        }
        Ok(unsafe { CFType::wrap_under_create_rule(out) })
    }

    pub fn string(&self, name: &str) -> Option<String> {
        let v = self.attribute(name).ok()?;
        if let Some(s) = v.downcast::<CFString>() {
            return Some(s.to_string());
        }
        v.downcast::<CFURL>().map(|u| u.get_string().to_string())
    }

    pub fn bool(&self, name: &str) -> Option<bool> {
        let v = self.attribute(name).ok()?;
        v.downcast::<CFBoolean>().map(bool::from)
    }

    pub fn element(&self, name: &str) -> Option<AxElement> {
        let v = self.attribute(name).ok()?;
        unsafe { AxElement::from_get(v.as_CFTypeRef() as AXUIElementRef) }
    }

    pub fn elements(&self, name: &str) -> Vec<AxElement> {
        let Ok(v) = self.attribute(name) else {
            return vec![];
        };
        let arr: CFArray<CFType> =
            unsafe { CFArray::wrap_under_get_rule(v.as_CFTypeRef() as CFArrayRef) };
        arr.iter()
            .filter_map(|e| unsafe { AxElement::from_get(e.as_CFTypeRef() as AXUIElementRef) })
            .collect()
    }

    pub fn has_attribute(&self, name: &str) -> bool {
        self.attribute(name).is_ok()
    }

    pub fn settable(&self, name: &str) -> bool {
        let mut b: Boolean = 0;
        let attr = cfstr(name);
        unsafe {
            AXUIElementIsAttributeSettable(self.0, attr.as_concrete_TypeRef(), &mut b)
                == kAXErrorSuccess
                && b != 0
        }
    }

    pub fn set_bool(&self, name: &str, v: bool) -> Result<(), AxErr> {
        let attr = cfstr(name);
        let b = CFBoolean::from(v);
        check(unsafe {
            AXUIElementSetAttributeValue(self.0, attr.as_concrete_TypeRef(), b.as_CFTypeRef())
        })
    }

    pub fn perform(&self, action: &str) -> Result<(), AxErr> {
        let a = cfstr(action);
        check(unsafe { AXUIElementPerformAction(self.0, a.as_concrete_TypeRef()) })
    }

    pub fn window_id(&self) -> Option<WindowId> {
        let mut wid: u32 = 0;
        let e = unsafe { _AXUIElementGetWindow(self.0, &mut wid) };
        (e == kAXErrorSuccess && wid != 0).then_some(WindowId(wid))
    }

    pub fn pid(&self) -> Option<i32> {
        let mut pid = 0;
        (unsafe { AXUIElementGetPid(self.0, &mut pid) } == kAXErrorSuccess).then_some(pid)
    }

    fn value<T: Default>(&self, name: &str, kind: u32) -> Option<T> {
        let v = self.attribute(name).ok()?;
        let mut out = T::default();
        let ok = unsafe {
            AXValueGetValue(
                v.as_CFTypeRef() as AXValueRef,
                kind,
                &mut out as *mut T as *mut c_void,
            )
        };
        (ok != 0).then_some(out)
    }

    pub fn frame(&self) -> Option<Rect> {
        let p: CGPoint = self.value("AXPosition", kAXValueCGPointType)?;
        let s: CGSize = self.value("AXSize", kAXValueCGSizeType)?;
        Some(Rect::new(p.x, p.y, s.width, s.height))
    }

    pub fn set_position(&self, x: f64, y: f64) -> Result<(), AxErr> {
        let p = CGPoint { x, y };
        self.set_value(
            "AXPosition",
            kAXValueCGPointType,
            &p as *const CGPoint as *const c_void,
        )
    }

    pub fn set_size(&self, w: f64, h: f64) -> Result<(), AxErr> {
        let s = CGSize {
            width: w,
            height: h,
        };
        self.set_value(
            "AXSize",
            kAXValueCGSizeType,
            &s as *const CGSize as *const c_void,
        )
    }

    fn set_value(&self, name: &str, kind: u32, ptr: *const c_void) -> Result<(), AxErr> {
        unsafe {
            let v = AXValueCreate(kind, ptr);
            if v.is_null() {
                return Err(AxErr(kAXErrorCannotComplete));
            }
            let attr = cfstr(name);
            let r =
                AXUIElementSetAttributeValue(self.0, attr.as_concrete_TypeRef(), v as CFTypeRef);
            CFRelease(v as CFTypeRef);
            check(r)
        }
    }

    /// Move then resize. With `double_set`, size first too: apps that clamp
    /// the size against the screen they are on (Chromium) need it.
    pub fn set_frame(&self, r: &Rect, double_set: bool) -> Result<(), AxErr> {
        if double_set {
            let _ = self.set_size(r.w, r.h);
        }
        // Always apply both: AX often reports an error for a position change
        // that did happen (notably off-screen), and stopping there would
        // leave the window moved but not resized.
        let pos = self.set_position(r.x, r.y);
        let size = self.set_size(r.w, r.h);
        pos.and(size)
    }
}

/// Is Pane trusted for Accessibility? With `prompt`, macOS shows its dialog.
pub fn is_trusted(prompt: bool) -> bool {
    unsafe {
        if !prompt {
            return AXIsProcessTrusted() != 0;
        }
        let key = CFString::from_static_string("AXTrustedCheckOptionPrompt");
        let dict = CFDictionary::from_CFType_pairs(&[(
            key.as_CFType(),
            CFBoolean::true_value().as_CFType(),
        )]);
        AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef()) != 0
    }
}

/// Is this process allowed Input Monitoring (listening to key events)?
/// Never prompts.
pub fn input_monitoring() -> bool {
    unsafe { CGPreflightListenEventAccess() }
}
