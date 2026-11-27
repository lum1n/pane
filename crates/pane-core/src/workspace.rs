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
    /// Tiled window zoomed to fill the display (others stay behind it).
    pub fullscreen: Option<WindowId>,
    /// Layout preset kept up as tiles come and go (`layout ...`).
    pub layout: Option<LayoutMode>,
}

/// A layout a workspace keeps: the reducer re-arranges its tiles into this
/// shape whenever the number of tiles changes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case")]
pub enum LayoutMode {
    /// One large tile on the left, the others stacked in a column on the
    /// right. `ratio` is the master tile's share of the width.
    MasterStack { ratio: f64 },
    /// `n` columns of equal width, each stacking its tiles.
    Columns { n: usize },
}

/// Smallest and largest master share.
pub const MASTER_RATIO_RANGE: std::ops::RangeInclusive<f64> = 0.1..=0.9;
/// Most columns `layout columns N` accepts.
pub const MAX_COLUMNS: usize = 12;

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
