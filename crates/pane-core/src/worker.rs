//! Commands from the runtime to a per-app AX worker.

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::WindowId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WorkerCmd {
    SetFrame {
        wid: WindowId,
        rect: Rect,
        gen: u64,
        double_set: bool,
        restore_to: Option<Rect>,
    },
    Raise {
        wid: WindowId,
    },
    Focus {
        wid: WindowId,
    },
    /// Re-enumerate windows and report them as `WindowsDiscovered`.
    Enumerate,
    /// Pull windows sitting at a display's park corner back on screen: to
    /// their entry in `known` if listed, else centred on that display.
    /// `displays` holds (frame, visible_frame) pairs.
    Recover {
        displays: Vec<(Rect, Rect)>,
        known: Vec<(WindowId, Rect)>,
    },
    Quit,
}
