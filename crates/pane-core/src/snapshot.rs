//! What the menu bar icon and the editor draw.

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::model::{Model, WindowMode};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub displays: Vec<SnapDisplay>,
    pub workspaces: Vec<SnapWorkspace>,
    pub focused_display: Option<String>,
    pub focused_window: Option<u32>,
    #[serde(default)]
    pub config_error: Option<String>,
    /// Filled in by the runtime.
    #[serde(default)]
    pub profiles: Vec<String>,
    #[serde(default)]
    pub ax_trusted: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapDisplay {
    pub id: String,
    pub name: String,
    pub frame: Rect,
    pub visible_frame: Rect,
    pub is_main: bool,
    pub rank: usize,
    pub visible_workspace: Option<String>,
    pub focused: bool,
    pub tiles: Vec<SnapTile>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapTile {
    pub wid: Option<u32>,
    pub bundle_id: String,
    pub title: String,
    /// Fractions of the display frame.
    pub rect: Rect,
    pub focused: bool,
    pub floating: bool,
    /// A profile slot still waiting for its app.
    pub pending: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapWorkspace {
    pub name: String,
    pub display: String,
    pub visible: bool,
    pub windows: usize,
}

pub fn build(m: &Model) -> Snapshot {
    let focused_display = m.focused_display();
    let focused_window = m.focused_window();
    let displays = m
        .displays
        .iter()
        .map(|d| {
            let ws = m.visible_ws(&d.id);
            let mut tiles = Vec::new();
            if let Some(ws) = ws {
                for wid in m
                    .tree
                    .windows(ws.root)
                    .into_iter()
                    .chain(ws.floating.iter().copied())
                {
                    let (Some(w), Some(r)) = (m.windows.get(&wid), m.desired.get(&wid)) else {
                        continue;
                    };
                    if w.minimized {
                        continue;
                    }
                    tiles.push(SnapTile {
                        wid: Some(wid.0),
                        bundle_id: w.bundle_id.clone(),
                        title: w.title.clone(),
                        rect: r.normalized_in(&d.frame),
                        focused: focused_window == Some(wid),
                        floating: w.mode == WindowMode::Floating,
                        pending: false,
                    });
                }
                for (wsid, slot, r) in &m.slot_frames {
                    if *wsid == ws.id {
                        tiles.push(SnapTile {
                            wid: None,
                            bundle_id: slot.bundle_id.clone(),
                            title: String::new(),
                            rect: r.normalized_in(&d.frame),
                            focused: false,
                            floating: false,
                            pending: true,
                        });
                    }
                }
            }
            SnapDisplay {
                id: d.id.clone(),
                name: d.name.clone(),
                frame: d.frame,
                visible_frame: d.visible_frame,
                is_main: d.is_main,
                rank: d.position_rank,
                visible_workspace: ws.map(|w| w.id.to_string()),
                focused: focused_display.as_deref() == Some(d.id.as_str()),
                tiles,
            }
        })
        .collect();
    let workspaces = m
        .workspaces
        .iter()
        .map(|w| SnapWorkspace {
            name: w.id.to_string(),
            display: w.display.clone(),
            visible: m.is_visible(&w.id),
            windows: m
                .tree
                .windows(w.root)
                .iter()
                .filter(|x| m.windows.contains_key(x))
                .count()
                + w.floating.len(),
        })
        .collect();
    Snapshot {
        displays,
        workspaces,
        focused_display,
        focused_window: focused_window.map(|w| w.0),
        config_error: m.config_error.as_ref().map(|e| e.render()),
        profiles: vec![],
        ax_trusted: true,
    }
}
