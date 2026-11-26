//! Inputs to the reducer.

use serde::{Deserialize, Serialize};

use crate::config::{Config, ConfigError};
use crate::display::Display;
use crate::geometry::{Rect, Size};
use crate::proto::Command;
use crate::rules::WindowRole;
use crate::state::ParkedEntry;
use crate::{Pid, WindowId};

/// Everything a worker learns about a window when it first sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowProbe {
    pub wid: WindowId,
    pub pid: Pid,
    pub title: String,
    pub role: WindowRole,
    pub resizable: bool,
    pub has_close_button: bool,
    pub modal: bool,
    pub minimized: bool,
    pub frame: Rect,
    #[serde(default)]
    pub document: Option<String>,
    #[serde(default)]
    pub min_size: Option<Size>,
}

/// Identifies the client waiting for a reply to a command.
pub type ReplyToken = u64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    AppLaunched {
        pid: Pid,
        bundle_id: Option<String>,
        name: String,
        /// NSApplicationActivationPolicyRegular.
        regular: bool,
    },
    AppTerminated {
        pid: Pid,
    },
    /// Windows found by enumerating an app (`initial`: at Pane start-up or
    /// when the app was first observed) or a single new window.
    WindowsDiscovered {
        pid: Pid,
        windows: Vec<WindowProbe>,
        initial: bool,
    },
    WindowDestroyed {
        pid: Pid,
        wid: WindowId,
    },
    /// Moved or resized; `frame` is the read-back frame.
    WindowFrameChanged {
        pid: Pid,
        wid: WindowId,
        frame: Rect,
    },
    TitleChanged {
        pid: Pid,
        wid: WindowId,
        title: String,
    },
    FocusChanged {
        pid: Pid,
        wid: Option<WindowId>,
    },
    WindowMinimized {
        pid: Pid,
        wid: WindowId,
        minimized: bool,
    },
    FrameApplied {
        pid: Pid,
        wid: WindowId,
        gen: u64,
        ok: bool,
        actual: Option<Rect>,
    },
    DisplaysChanged {
        displays: Vec<Display>,
    },
    Command {
        cmd: Command,
        #[serde(default)]
        token: Option<ReplyToken>,
    },
    ConfigLoaded {
        config: Box<Config>,
    },
    ConfigFailed {
        error: ConfigError,
    },
    /// Parked windows left behind by a previous run that died.
    RestoreParked {
        entries: Vec<ParkedEntry>,
    },
    /// Every app running at start-up has been enumerated.
    InitialScanDone,
    AppOpenResult {
        request_id: u64,
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    },
    Tick,
    WillSleep,
    DidWake,
    Shutdown,
}

impl Event {
    /// The app this event proves is alive, if any.
    pub fn from_pid(&self) -> Option<Pid> {
        match self {
            Event::WindowsDiscovered { pid, .. }
            | Event::WindowDestroyed { pid, .. }
            | Event::WindowFrameChanged { pid, .. }
            | Event::TitleChanged { pid, .. }
            | Event::FocusChanged { pid, .. }
            | Event::WindowMinimized { pid, .. }
            | Event::FrameApplied { pid, .. } => Some(*pid),
            _ => None,
        }
    }
}
