//! Persisted runtime state (`state.json`) and the parked-window journal
//! (`parked.json`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::identity::PlacementRecord;
use crate::tree::SerNode;
use crate::{DisplayId, Pid, WindowId, WorkspaceId};

pub const STATE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    pub workspaces: Vec<WorkspaceState>,
    #[serde(default)]
    pub visible: BTreeMap<DisplayId, WorkspaceId>,
    #[serde(default)]
    pub remembered_visible: BTreeMap<DisplayId, WorkspaceId>,
    #[serde(default)]
    pub placements: Vec<PlacementRecord>,
    #[serde(default)]
    pub focus_display: Option<DisplayId>,
    #[serde(default)]
    pub focus_window: Option<WindowId>,
    /// Displays connected when the state was written.
    #[serde(default)]
    pub displays: Vec<DisplayId>,
    /// Last known frame of every display seen.
    #[serde(default)]
    pub display_frames: BTreeMap<DisplayId, Rect>,
    #[serde(default)]
    pub display_names: BTreeMap<DisplayId, String>,
    /// Profile applied last.
    #[serde(default)]
    pub active_profile: Option<String>,
    /// Profile slot each window fills.
    #[serde(default)]
    pub window_slots: BTreeMap<WindowId, crate::tree::AppSlot>,
    /// Scratchpad name -> its window.
    #[serde(default)]
    pub scratchpads: BTreeMap<String, WindowId>,
    /// Display set automatic profile choice last acted on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_topology: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub id: WorkspaceId,
    /// Home display; absent in files from before per-display workspaces.
    #[serde(default)]
    pub home: Option<DisplayId>,
    #[serde(default)]
    pub name: Option<String>,
    pub display: DisplayId,
    pub tree: SerNode,
    #[serde(default)]
    pub floating: Vec<FloatingState>,
    #[serde(default)]
    pub focused: Option<WindowId>,
    /// Layout preset the workspace keeps (`layout master-stack`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<crate::workspace::LayoutMode>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FloatingState {
    pub wid: WindowId,
    #[serde(default)]
    pub frame: Option<Rect>,
}

/// A window Pane moved off-screen, with where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ParkedEntry {
    pub pid: Pid,
    pub wid: WindowId,
    pub bundle_id: String,
    pub title: String,
    pub original: Rect,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ParkedFile {
    pub version: u32,
    pub entries: Vec<ParkedEntry>,
}
