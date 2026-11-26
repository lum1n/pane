//! Workspaces belong to a display: each display has its own "1", "2", ...
//! A workspace is a tree root plus floating windows.

use serde::{Deserialize, Serialize};

use crate::tree::NodeId;
use crate::{DisplayId, WindowId, WorkspaceId};

#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    /// `<home display>/<name>`, unique.
    pub id: WorkspaceId,
    /// Display the workspace belongs to (CGDisplay UUID).
    pub home: DisplayId,
    /// Name within its display: "1", "2", "web".
    pub name: String,
    /// Display it is on now: `home`, or a host while `home` is unplugged.
    pub display: DisplayId,
    pub root: NodeId,
    /// Floating windows, bottom to top.
    pub floating: Vec<WindowId>,
    /// Last focused window in this workspace.
    pub focused: Option<WindowId>,
}

impl Workspace {
    /// Hosted on another display because its own is unplugged.
    pub fn is_guest(&self) -> bool {
        self.display != self.home
    }
}

/// Id of workspace `name` on display `home`.
pub fn ws_id(home: &str, name: &str) -> WorkspaceId {
    WorkspaceId::from(format!("{home}/{name}"))
}

/// Split an id back into (home, name).
pub fn split_id(id: &str) -> (&str, &str) {
    id.rsplit_once('/').unwrap_or(("", id))
}

/// Order workspaces numerically, then by name.
pub fn name_order(name: &str) -> (u64, String) {
    (name.parse::<u64>().unwrap_or(u64::MAX), name.to_string())
}

/// Workspace names may not contain '/' or ':' (used in ids and targets).
pub fn valid_name(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && !n.contains('/') && !n.contains(':') && n.len() <= 32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Cycle {
    Next,
    Prev,
}
