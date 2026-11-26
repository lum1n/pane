//! Workspaces: a tree root plus floating windows, shown on one display.

use serde::{Deserialize, Serialize};

use crate::display::DisplayRef;
use crate::tree::NodeId;
use crate::{DisplayId, WindowId, WorkspaceId};

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub prefs: Vec<DisplayRef>,
    /// Display the workspace is on.
    pub display: DisplayId,
    pub root: NodeId,
    /// Floating windows, bottom to top.
    pub floating: Vec<WindowId>,
    /// Last focused window in this workspace.
    pub focused: Option<WindowId>,
    /// Display it was evicted from by a disconnect.
    pub last_display: Option<DisplayId>,
    /// From config: never garbage-collected.
    pub configured: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cycle {
    Next,
    Prev,
}
