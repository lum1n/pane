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
    /// Profile applied last (stays until another is applied).
    #[serde(default)]
    pub active_profile: Option<String>,
    /// Filled in by the runtime.
    #[serde(default)]
    pub profiles: Vec<String>,
    /// Saved profiles whose `[when]` matches the connected displays. Filled
    /// in by the runtime.
    #[serde(default)]
    pub desk_profiles: Vec<String>,
    #[serde(default)]
    pub ax_trusted: bool,
    /// How the shell draws the focus border and dimming.
    #[serde(default)]
    pub appearance: SnapAppearance,
}

/// Focus border and dimming, from `[settings]`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapAppearance {
    /// Points; 0 = no border.
    pub border_width: f64,
    /// RGBA in 0..=1; `None` = the system accent colour.
    pub border_color: Option<[f64; 4]>,
    /// Darkening of unfocused windows, 0..=0.8.
    pub dim_unfocused: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapDisplay {
    pub id: String,
    pub name: String,
    pub frame: Rect,
    pub visible_frame: Rect,
    pub is_main: bool,
    pub rank: usize,
    /// Name of the workspace shown ("2").
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
    pub id: String,
    /// Name within its display ("2").
    pub name: String,
    /// Display it is on now.
    pub display: String,
    /// Display it belongs to.
    pub home: String,
    /// Hosted here while its own display is unplugged.
    pub guest: bool,
    /// For menus: "2", or "DELL U2720Q: 2" for guests.
    pub label: String,
    /// Pass as `name` to the `workspace` command.
    pub target: String,
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
                // A zoomed workspace shows just the zoomed tile (and floats).
                let tiled = match ws.fullscreen {
                    Some(z) => vec![z],
                    None => m.tree.windows(ws.root),
                };
                for wid in tiled.into_iter().chain(ws.floating.iter().copied()) {
                    let (Some(w), Some(r)) = (m.windows.get(&wid), m.desired.get(&wid)) else {
                        continue;
                    };
                    if w.minimized || m.scratch_hidden.contains(&wid) {
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
                visible_workspace: ws.map(|w| w.name.clone()),
                focused: focused_display.as_deref() == Some(d.id.as_str()),
                tiles,
            }
        })
        .collect();
    let mut workspaces: Vec<SnapWorkspace> = Vec::new();
    for d in &m.displays {
        for w in m.workspaces_on(&d.id) {
            let windows = m
                .tree
                .windows(w.root)
                .iter()
                .filter(|x| m.windows.contains_key(x))
                .count()
                + w.floating
                    .iter()
                    .filter(|f| !m.scratch_hidden.contains(f))
                    .count();
            let visible = m.is_visible(&w.id);
            // Empty workspaces only show up while visible.
            if windows == 0 && !visible && !w.is_guest() {
                continue;
            }
            let label = if w.is_guest() {
                let home = m
                    .display_names
                    .get(&w.home)
                    .cloned()
                    .unwrap_or_else(|| "Unplugged display".into());
                format!("{home}: {}", w.name)
            } else {
                w.name.clone()
            };
            workspaces.push(SnapWorkspace {
                id: w.id.to_string(),
                name: w.name.clone(),
                display: w.display.clone(),
                home: w.home.clone(),
                guest: w.is_guest(),
                label,
                target: format!("uuid:{}:{}", w.home, w.name),
                visible,
                windows,
            });
        }
    }
    Snapshot {
        displays,
        workspaces,
        focused_display,
        focused_window: focused_window.map(|w| w.0),
        config_error: m.config_error.as_ref().map(|e| e.render()),
        active_profile: m.active_profile.clone(),
        profiles: vec![],
        desk_profiles: vec![],
        ax_trusted: true,
        appearance: SnapAppearance {
            border_width: m.config.settings.border_width,
            border_color: m.config.border_rgba().unwrap_or(None),
            dim_unfocused: m.config.settings.dim_unfocused,
        },
    }
}
