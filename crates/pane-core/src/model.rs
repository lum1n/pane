//! The whole window-manager state owned by the core thread.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::config::{Config, ConfigError};
use crate::display::{Display, DisplayRef};
use crate::geometry::{Rect, Size};
use crate::identity::PlacementRecord;
use crate::rules::{Rule, WindowRole};
use crate::state::{FloatingState, ParkedEntry, State, WorkspaceState, STATE_VERSION};
use crate::tree::{AppSlot, Leaf, NodeId, Tree};
use crate::workspace::Workspace;
use crate::{DisplayId, Pid, WindowId, WorkspaceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    Tiled,
    Floating,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WindowInfo {
    pub id: WindowId,
    pub pid: Pid,
    pub bundle_id: String,
    pub title: String,
    pub role: WindowRole,
    pub resizable: bool,
    pub document: Option<String>,
    /// Learned from frames the app refused to shrink below.
    pub min_size: Option<Size>,
    /// Last read-back frame.
    pub actual: Rect,
    pub ordinal: u32,
    pub mode: WindowMode,
    pub workspace: WorkspaceId,
    pub minimized: bool,
    /// Gave up setting its frame.
    pub stubborn: bool,
    /// Needs size, position, size to land correctly.
    pub double_set: bool,
    pub float_frame: Option<Rect>,
    /// Creation order, for "newest" decisions.
    pub seq: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AppInfo {
    pub pid: Pid,
    pub bundle_id: String,
    pub name: String,
    pub regular: bool,
    pub unresponsive: bool,
    /// Oldest unanswered SetFrame (unix ms).
    pub awaiting_since: Option<u64>,
}

/// The last frame sent for a window and the retry bookkeeping around it.
#[derive(Clone, Debug, PartialEq)]
pub struct SentFrame {
    pub rect: Rect,
    pub gen: u64,
    pub at_ms: u64,
    pub attempts: u32,
    pub next_retry_ms: Option<u64>,
    pub settled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingSlot {
    pub request_id: u64,
    pub bundle_id: String,
    pub workspace: WorkspaceId,
    pub deadline_ms: u64,
}

pub const RETRY_BACKOFF_MS: [u64; 5] = [50, 150, 400, 1000, 2500];
pub const UNRESPONSIVE_AFTER_MS: u64 = 2000;
pub const PENDING_SLOT_MS: u64 = 30_000;
pub const GHOST_TTL_MS: u64 = 10 * 60 * 1000;
pub const RESYNC_DELAY_MS: u64 = 1000;
pub const FOLLOW_SUPPRESS_MS: u64 = 500;
pub const MAX_RECORDS_PER_APP: usize = 40;

#[derive(Clone, Debug)]
pub struct Model {
    pub config: Config,
    pub rules: Vec<Rule>,
    pub config_error: Option<ConfigError>,
    pub displays: Vec<Display>,
    /// Display ids connected before the latest change (or in `state.json`).
    pub known_displays: BTreeSet<DisplayId>,
    pub workspaces: Vec<Workspace>,
    pub visible: BTreeMap<DisplayId, WorkspaceId>,
    pub remembered_visible: BTreeMap<DisplayId, WorkspaceId>,
    pub tree: Tree,
    pub windows: BTreeMap<WindowId, WindowInfo>,
    pub ignored: BTreeSet<WindowId>,
    pub apps: BTreeMap<Pid, AppInfo>,
    pub placements: Vec<PlacementRecord>,
    pub focus_display: Option<DisplayId>,
    pub focus_window: Option<WindowId>,
    pub last_sent: BTreeMap<WindowId, SentFrame>,
    /// Target frame per window from the last reconcile.
    pub desired: BTreeMap<WindowId, Rect>,
    /// Frames of profile slots waiting for their app, per workspace.
    pub slot_frames: Vec<(WorkspaceId, AppSlot, Rect)>,
    pub parked: BTreeMap<WindowId, ParkedEntry>,
    pub pending_slots: Vec<PendingSlot>,
    pub pending_restore: Vec<ParkedEntry>,
    pub restored_count: usize,
    /// Window leaves whose window is gone but may come back (unix ms since).
    pub ghosts: BTreeMap<WindowId, u64>,
    /// Floating windows from `state.json` waiting to be rediscovered.
    pub expected_floating: BTreeMap<WindowId, (WorkspaceId, Option<Rect>)>,
    pub now_ms: u64,
    pub gen: u64,
    pub seq: u64,
    pub next_request_id: u64,
    pub layout_dirty: bool,
    pub resync_at: Option<u64>,
    pub suppress_follow_until: u64,
    pub shutting_down: bool,
    pub initial_scan_done: bool,
}

impl Model {
    pub fn new(config: Config, now_ms: u64) -> Model {
        let rules = config.compiled_rules().unwrap_or_default();
        let mut m = Model {
            config,
            rules,
            config_error: None,
            displays: vec![],
            known_displays: BTreeSet::new(),
            workspaces: vec![],
            visible: BTreeMap::new(),
            remembered_visible: BTreeMap::new(),
            tree: Tree::new(),
            windows: BTreeMap::new(),
            ignored: BTreeSet::new(),
            apps: BTreeMap::new(),
            placements: vec![],
            focus_display: None,
            focus_window: None,
            last_sent: BTreeMap::new(),
            desired: BTreeMap::new(),
            slot_frames: vec![],
            parked: BTreeMap::new(),
            pending_slots: vec![],
            pending_restore: vec![],
            restored_count: 0,
            ghosts: BTreeMap::new(),
            expected_floating: BTreeMap::new(),
            now_ms,
            gen: 0,
            seq: 0,
            next_request_id: 1,
            layout_dirty: true,
            resync_at: None,
            suppress_follow_until: 0,
            shutting_down: false,
            initial_scan_done: false,
        };
        m.ensure_configured_workspaces();
        m
    }

    /// Rebuild from `state.json`. Window leaves become ghosts until their
    /// windows are rediscovered. Displays arrive later via `DisplaysChanged`.
    pub fn from_state(config: Config, state: &State, now_ms: u64) -> Model {
        let mut m = Model::new(config, now_ms);
        let configured: BTreeMap<String, Vec<DisplayRef>> =
            m.config.workspace_prefs().into_iter().collect();
        for ws in &state.workspaces {
            let root = match m.workspaces.iter().find(|w| w.id == ws.id).map(|w| w.root) {
                Some(r) => {
                    m.tree.fill_from_ser(r, &ws.tree);
                    m.workspaces.retain(|w| w.id != ws.id);
                    r
                }
                None => m.tree.root_from_ser(&ws.tree),
            };
            for wid in m.tree.windows(root) {
                m.ghosts.insert(wid, now_ms);
            }
            for f in &ws.floating {
                m.expected_floating.insert(f.wid, (ws.id.clone(), f.frame));
            }
            let (prefs, is_cfg) = match configured.get(ws.id.as_str()) {
                Some(p) => (p.clone(), true),
                None => (
                    ws.prefs.iter().map(|p| m.config.display_ref(p)).collect(),
                    false,
                ),
            };
            m.workspaces.push(Workspace {
                id: ws.id.clone(),
                prefs,
                display: ws.display.clone(),
                root,
                floating: vec![],
                focused: ws.focused,
                last_display: ws.last_display.clone(),
                configured: is_cfg,
            });
        }
        m.visible = state.visible.clone();
        m.remembered_visible = state.remembered_visible.clone();
        m.placements = state.placements.clone();
        m.focus_display = state.focus_display.clone();
        m.focus_window = state.focus_window;
        m.known_displays = state.displays.iter().cloned().collect();
        m.ensure_configured_workspaces();
        m
    }

    pub fn to_state(&self) -> State {
        State {
            version: STATE_VERSION,
            workspaces: self
                .workspaces
                .iter()
                .map(|w| WorkspaceState {
                    id: w.id.clone(),
                    prefs: if w.configured {
                        vec![]
                    } else {
                        w.prefs.iter().map(|p| p.to_short()).collect()
                    },
                    display: w.display.clone(),
                    last_display: w.last_display.clone(),
                    tree: self.tree.to_ser(w.root),
                    floating: w
                        .floating
                        .iter()
                        .map(|wid| FloatingState {
                            wid: *wid,
                            frame: self.windows.get(wid).and_then(|x| x.float_frame),
                        })
                        .collect(),
                    focused: w.focused,
                })
                .collect(),
            visible: self.visible.clone(),
            remembered_visible: self.remembered_visible.clone(),
            placements: self.placements.clone(),
            focus_display: self.focus_display.clone(),
            focus_window: self.focus_window,
            displays: self.displays.iter().map(|d| d.id.clone()).collect(),
        }
    }

    /// Make sure every `[[workspace]]` from the config exists, in config
    /// order, with current preferences.
    pub fn ensure_configured_workspaces(&mut self) {
        for (name, prefs) in self.config.workspace_prefs() {
            let id = WorkspaceId::from(name.as_str());
            if let Some(w) = self.workspaces.iter_mut().find(|w| w.id == id) {
                if w.prefs != prefs {
                    // Preferences changed: let the next assignment place it afresh.
                    w.prefs = prefs;
                    w.display = String::new();
                }
                w.configured = true;
            } else {
                let root = self
                    .tree
                    .new_root(Default::default(), self.config.settings.default_layout);
                self.workspaces.push(Workspace {
                    id,
                    prefs,
                    display: String::new(),
                    root,
                    floating: vec![],
                    focused: None,
                    last_display: None,
                    configured: true,
                });
            }
        }
        // Configured ones first, in config order; others keep their order.
        let order: Vec<String> = self
            .config
            .workspace_prefs()
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        self.workspaces.sort_by_key(|w| {
            order
                .iter()
                .position(|n| n == w.id.as_str())
                .unwrap_or(usize::MAX)
        });
        for w in &mut self.workspaces {
            if !order.iter().any(|n| n == w.id.as_str()) {
                w.configured = false;
            }
        }
    }

    pub fn main_display_id(&self) -> Option<DisplayId> {
        self.displays
            .iter()
            .find(|d| d.is_main)
            .or(self.displays.first())
            .map(|d| d.id.clone())
    }

    pub fn display(&self, id: &str) -> Option<&Display> {
        self.displays.iter().find(|d| d.id == id)
    }

    pub fn ws(&self, id: &str) -> Option<&Workspace> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub fn ws_mut(&mut self, id: &str) -> Option<&mut Workspace> {
        self.workspaces.iter_mut().find(|w| w.id == id)
    }

    pub fn is_visible(&self, ws: &str) -> bool {
        self.visible.values().any(|v| v == ws)
    }

    pub fn visible_ws(&self, display: &str) -> Option<&Workspace> {
        self.visible.get(display).and_then(|w| self.ws(w))
    }

    /// The focused display, falling back to the main display.
    pub fn focused_display(&self) -> Option<DisplayId> {
        self.focus_display
            .clone()
            .filter(|d| self.display(d).is_some())
            .or_else(|| self.main_display_id())
    }

    pub fn focused_ws_id(&self) -> Option<WorkspaceId> {
        self.focused_display()
            .and_then(|d| self.visible.get(&d).cloned())
    }

    /// Focused window if it is in the focused workspace.
    pub fn focused_window(&self) -> Option<WindowId> {
        let ws = self.focused_ws_id()?;
        self.focus_window
            .filter(|w| self.windows.get(w).is_some_and(|x| x.workspace == ws))
    }

    pub fn window_node(&self, wid: WindowId) -> Option<NodeId> {
        let w = self.windows.get(&wid)?;
        self.tree.find_window(self.ws(&w.workspace)?.root, wid)
    }

    /// Tiling area of a display.
    pub fn tiling_area(&self, display: &Display) -> Rect {
        display.visible_frame.inset(self.config.settings.outer_gap)
    }

    pub fn next_gen(&mut self) -> u64 {
        self.gen += 1;
        self.gen
    }

    pub fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    /// Create a workspace if missing, on `display` (default: focused).
    pub fn ensure_workspace(&mut self, name: &str, display: Option<DisplayId>) -> WorkspaceId {
        let id = WorkspaceId::from(name);
        if self.ws(name).is_some() {
            return id;
        }
        let display = display
            .or_else(|| self.focused_display())
            .unwrap_or_default();
        let root = self
            .tree
            .new_root(Default::default(), self.config.settings.default_layout);
        self.workspaces.push(Workspace {
            id: id.clone(),
            prefs: vec![DisplayRef::Uuid(display.clone())],
            display,
            root,
            floating: vec![],
            focused: None,
            last_display: None,
            configured: false,
        });
        id
    }

    /// Is a leaf laid out? Ghosts, minimized and floating windows are not.
    pub fn leaf_present(&self, leaf: &Leaf) -> bool {
        match leaf {
            Leaf::Window(w) => self
                .windows
                .get(w)
                .is_some_and(|x| !x.minimized && x.mode == WindowMode::Tiled),
            Leaf::Slot(_) => true,
        }
    }

    pub fn leaf_min_size(&self, leaf: &Leaf) -> Size {
        match leaf {
            Leaf::Window(w) => self
                .windows
                .get(w)
                .and_then(|x| x.min_size)
                .unwrap_or(Size::ZERO),
            Leaf::Slot(_) => Size::ZERO,
        }
    }

    /// Earliest time the reducer wants a `Tick`.
    pub fn next_deadline(&self) -> Option<u64> {
        let mut t: Vec<u64> = Vec::new();
        t.extend(self.last_sent.values().filter_map(|s| s.next_retry_ms));
        t.extend(
            self.apps
                .values()
                .filter(|a| !a.unresponsive)
                .filter_map(|a| a.awaiting_since.map(|s| s + UNRESPONSIVE_AFTER_MS)),
        );
        t.extend(self.pending_slots.iter().map(|p| p.deadline_ms));
        t.extend(self.resync_at);
        t.extend(self.ghosts.values().map(|s| s + GHOST_TTL_MS));
        t.into_iter().min()
    }

    /// SetFrames sent but not yet confirmed.
    pub fn outstanding_sets(&self) -> usize {
        self.last_sent
            .iter()
            .filter(|(w, s)| {
                !s.settled
                    && self
                        .windows
                        .get(w)
                        .is_some_and(|x| self.apps.get(&x.pid).is_some_and(|a| !a.unresponsive))
            })
            .count()
    }
}
