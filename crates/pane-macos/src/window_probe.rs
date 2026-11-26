//! Classify an AX window into a [`WindowProbe`].

use pane_core::event::WindowProbe;
use pane_core::rules::WindowRole;

use crate::ax::AxElement;

pub fn probe(pid: i32, el: &AxElement) -> Option<WindowProbe> {
    let wid = el.window_id()?;
    let role = el.string("AXRole").unwrap_or_default();
    if role != "AXWindow" {
        return None;
    }
    let subrole = el.string("AXSubrole").unwrap_or_default();
    let role = match subrole.as_str() {
        "AXStandardWindow" => WindowRole::Standard,
        "AXDialog" | "AXSystemDialog" => WindowRole::Dialog,
        "AXFloatingWindow" | "AXSystemFloatingWindow" => WindowRole::Floating,
        "AXSheet" => WindowRole::Sheet,
        _ => WindowRole::Other,
    };
    let frame = el.frame().unwrap_or_default();
    Some(WindowProbe {
        wid,
        pid,
        title: el.string("AXTitle").unwrap_or_default(),
        role,
        resizable: el.settable("AXSize"),
        has_close_button: el.has_attribute("AXCloseButton"),
        modal: el.bool("AXModal").unwrap_or(false),
        minimized: el.bool("AXMinimized").unwrap_or(false),
        frame,
        document: el.string("AXDocument").filter(|d| !d.is_empty()),
        min_size: None,
    })
}

/// Native fullscreen windows live on their own Space; leave them alone.
pub fn is_fullscreen(el: &AxElement) -> bool {
    el.bool("AXFullScreen").unwrap_or(false)
}
