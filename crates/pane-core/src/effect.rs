//! Outputs of the reducer, carried out by the runtime.

use serde::{Deserialize, Serialize};

use crate::event::ReplyToken;
use crate::geometry::Rect;
use crate::proto::{Reply, UiRequest};
use crate::state::ParkedEntry;
use crate::{Pid, WindowId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "effect", rename_all = "snake_case")]
pub enum Effect {
    /// Start a worker for the app and enumerate its windows.
    Observe {
        pid: Pid,
    },
    Unobserve {
        pid: Pid,
    },
    SetFrame {
        pid: Pid,
        wid: WindowId,
        rect: Rect,
        gen: u64,
        /// Use size, position, size (Electron/Chromium apps).
        double_set: bool,
        /// Set when parking: where to put the window back in an emergency.
        restore_to: Option<Rect>,
    },
    Raise {
        pid: Pid,
        wid: WindowId,
    },
    /// Give keyboard focus to the desktop (the workspace shown is empty).
    FocusDesktop,
    /// Raise and give keyboard focus.
    Focus {
        pid: Pid,
        wid: WindowId,
    },
    /// Un-minimize, raise and give keyboard focus.
    Restore {
        pid: Pid,
        wid: WindowId,
    },
    /// Write `parked.json` (fsync + rename) before parking anything.
    WriteParked {
        entries: Vec<ParkedEntry>,
    },
    /// State changed; the runtime debounces the write.
    PersistState,
    /// Something visible changed; the runtime rebuilds the snapshot.
    Snapshot,
    UiRequest(UiRequest),
    Reply {
        token: ReplyToken,
        reply: Reply,
    },
    /// Pull every window at a park corner back on screen (including apps
    /// Pane does not manage when `all_apps`).
    RecoverSweep {
        all_apps: bool,
    },
}
