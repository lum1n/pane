//! C ABI for `Pane.app`. All payloads are UTF-8 JSON; strings returned by
//! Rust must be released with `pane_free`. The header is `include/pane.h`.
#![allow(clippy::missing_safety_doc)]

use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use pane_agent::paths::Paths;
use pane_agent::platform::{native, Platform};
use pane_agent::{logging, runtime, Handle, Options};
use pane_core::event::Event;
use pane_core::proto::{Command, Reply};
use serde::Deserialize;

/// Called on the Pane core thread; hop to the main thread before touching UI.
pub type PaneCallback = Option<extern "C" fn(ctx: *mut c_void, json: *const c_char)>;

struct Ctx(*mut c_void);
// The context pointer is owned by the Swift side, which promises it stays
// valid until `pane_shutdown` returns and handles calls from any thread.
unsafe impl Send for Ctx {}
unsafe impl Sync for Ctx {}

static HANDLE: OnceLock<Handle> = OnceLock::new();
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

fn set_error(e: impl Into<String>) {
    if let Ok(mut l) = LAST_ERROR.lock() {
        *l = Some(e.into());
    }
}

fn to_c(s: String) -> *mut c_char {
    CString::new(s)
        .unwrap_or_else(|_| CString::new("{}").unwrap())
        .into_raw()
}

unsafe fn from_c<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    CStr::from_ptr(p).to_str().ok()
}

fn sink(cb: PaneCallback, ctx: &Arc<Ctx>) -> Option<runtime::Sink> {
    let cb = cb?;
    let ctx = ctx.clone();
    Some(Arc::new(move |json: &str| {
        if let Ok(c) = CString::new(json) {
            cb(ctx.0, c.as_ptr());
        }
    }))
}

/// Start Pane. Call once, on the main thread. Returns false on failure;
/// see `pane_last_error`.
#[no_mangle]
pub unsafe extern "C" fn pane_start(
    support_dir: *const c_char,
    on_snapshot: PaneCallback,
    on_request: PaneCallback,
    ctx: *mut c_void,
) -> bool {
    if HANDLE.get().is_some() {
        return true;
    }
    let root = from_c(support_dir)
        .map(Into::into)
        .unwrap_or_else(Paths::default_root);
    let paths = Paths::new(root);
    let _ = paths.ensure();
    logging::init(&paths.logs(), false);
    let ctx = Arc::new(Ctx(ctx));
    let mut opts = Options::new(paths);
    opts.on_snapshot = sink(on_snapshot, &ctx);
    opts.on_request = sink(on_request, &ctx);
    let platform: Arc<dyn Platform> = Arc::from(native());
    match runtime::start(platform, opts) {
        Ok(h) => {
            let _ = HANDLE.set(h);
            true
        }
        Err(e) => {
            tracing::error!("pane_start: {e}");
            set_error(e);
            false
        }
    }
}

/// Last error from `pane_start`, or NULL. Free with `pane_free`.
#[no_mangle]
pub extern "C" fn pane_last_error() -> *mut c_char {
    match LAST_ERROR.lock().ok().and_then(|l| l.clone()) {
        Some(e) => to_c(e),
        None => std::ptr::null_mut(),
    }
}

/// Run a `proto::Command` (JSON) and return the `proto::Reply` (JSON).
/// Blocks up to 5 s; do not call from the snapshot/request callbacks.
#[no_mangle]
pub unsafe extern "C" fn pane_command(json: *const c_char) -> *mut c_char {
    let reply = match (HANDLE.get(), from_c(json)) {
        (None, _) => Reply::err("Pane is not running"),
        (_, None) => Reply::err("invalid UTF-8"),
        (Some(h), Some(s)) => match serde_json::from_str::<Command>(s) {
            Ok(cmd) => h.command(cmd, Duration::from_secs(5)),
            Err(e) => Reply::err(format!("bad command: {e}")),
        },
    };
    to_c(serde_json::to_string(&reply).unwrap_or_default())
}

/// Events the shell observes through NSWorkspace / NSScreen.
#[derive(Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum ShellEvent {
    AppLaunched {
        pid: i32,
        #[serde(default)]
        bundle_id: Option<String>,
        #[serde(default)]
        name: String,
        #[serde(default = "yes")]
        regular: bool,
    },
    AppTerminated {
        pid: i32,
    },
    /// Must be sent on the main thread.
    ScreensChanged,
    WillSleep,
    DidWake,
    AppOpenResult {
        request_id: u64,
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
}

fn yes() -> bool {
    true
}

/// Deliver a shell event (JSON). Returns false if it was not understood.
#[no_mangle]
pub unsafe extern "C" fn pane_event(json: *const c_char) -> bool {
    let (Some(h), Some(s)) = (HANDLE.get(), from_c(json)) else {
        return false;
    };
    let Ok(ev) = serde_json::from_str::<ShellEvent>(s) else {
        return false;
    };
    match ev {
        ShellEvent::AppLaunched {
            pid,
            bundle_id,
            name,
            regular,
        } => h.send_event(Event::AppLaunched {
            pid,
            bundle_id,
            name,
            regular,
        }),
        ShellEvent::AppTerminated { pid } => h.send_event(Event::AppTerminated { pid }),
        ShellEvent::ScreensChanged => h.screens_changed(),
        ShellEvent::WillSleep => h.send_event(Event::WillSleep),
        ShellEvent::DidWake => {
            h.send_event(Event::DidWake);
            h.screens_changed();
        }
        ShellEvent::AppOpenResult {
            request_id,
            ok,
            error,
        } => h.send_event(Event::AppOpenResult {
            request_id,
            ok,
            error,
        }),
    }
    true
}

/// Unpark every window, persist and stop (at most ~3 s). Call from
/// `applicationWillTerminate`.
#[no_mangle]
pub extern "C" fn pane_shutdown() -> bool {
    match HANDLE.get() {
        Some(h) => h.shutdown(Duration::from_secs(3)),
        None => true,
    }
}

#[no_mangle]
pub unsafe extern "C" fn pane_free(s: *mut c_char) {
    if !s.is_null() {
        drop(CString::from_raw(s));
    }
}
