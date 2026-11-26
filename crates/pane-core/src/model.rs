//! The whole window-manager state owned by the core thread.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::Serialize;

use crate::config::{Config, ConfigError};
use crate::display::Display;
use crate::drop::DropTarget;
use crate::geometry::{Rect, Size};
use crate::identity::PlacementRecord;
use crate::rules::{Rule, WindowRole};
use crate::state::{FloatingState, ParkedEntry, State, WorkspaceState, STATE_VERSION};
use crate::tree::{AppSlot, Leaf, NodeId, Tree};
use crate::workspace::{name_order, ws_id, Workspace};
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
    /// Why it is tiled or floating (for `pane query windows`).
    pub reason: String,
    /// The profile slot this window fills. When it closes, the slot comes
    /// back so the app's next window returns to the same place.
    pub slot: Option<AppSlot>,
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
    /// The frame was a park rect (hiding the window).
    pub parked: bool,
}

/// A window being moved by holding the modifier and pointing.
#[derive(Clone, Debug, PartialEq)]
pub struct Gesture {
    pub wid: WindowId,
    /// Where it would go if dropped now (what the overlay shows).
    pub target: Option<DropTarget>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PendingSlot {
    pub request_id: u64,
    pub bundle_id: String,
    pub workspace: WorkspaceId,
    pub deadline_ms: u64,
}

/// An app Pane asked the shell to open for a scratchpad or `summon`.
#[derive(Clone, Debug, PartialEq)]
pub struct PendingLaunch {
    pub request_id: u64,
    pub bundle_id: String,
    /// The scratchpad its first window becomes, if any.
    pub scratchpad: Option<String>,
    pub deadline_ms: u64,
}

pub const RETRY_BACKOFF_MS: [u64; 5] = [50, 150, 400, 1000, 2500];
pub const UNRESPONSIVE_AFTER_MS: u64 = 2000;
pub const PENDING_SLOT_MS: u64 = 30_000;
pub const GHOST_TTL_MS: u64 = 10 * 60 * 1000;
pub const RESYNC_DELAY_MS: u64 = 1000;
pub const FOLLOW_SUPPRESS_MS: u64 = 1000;
/// How often the watchdog checks that shown windows are on screen.
pub const WATCHDOG_MS: u64 = 2000;
pub const MAX_RECORDS_PER_APP: usize = 40;

#[derive(Clone, Debug)]
pub struct Model {
    pub config: Config,
    pub rules: Vec<Rule>,
    pub config_error: Option<ConfigError>,
    pub displays: Vec<Display>,
    /// Display ids connected before the latest change (or in `state.json`).
    pub known_displays: BTreeSet<DisplayId>,
    /// Last known frame of every display seen (to place unplugged ones).
    pub display_frames: BTreeMap<DisplayId, Rect>,
    /// Last known name of every display seen (labels for guests).
    pub display_names: BTreeMap<DisplayId, String>,
    pub workspaces: Vec<Workspace>,
    pub visible: BTreeMap<DisplayId, WorkspaceId>,
    pub remembered_visible: BTreeMap<DisplayId, WorkspaceId>,
    pub tree: Tree,
    pub windows: BTreeMap<WindowId, WindowInfo>,
    pub ignored: BTreeSet<WindowId>,
    /// Windows ignored by rules, per app (diagnostics).
    pub ignored_by_pid: BTreeMap<Pid, usize>,
    pub apps: BTreeMap<Pid, AppInfo>,
    pub placements: Vec<PlacementRecord>,
    pub focus_display: Option<DisplayId>,
    pub focus_window: Option<WindowId>,
    /// Window macOS last reported as focused in the frontmost app.
    pub os_focus: Option<WindowId>,
    /// When each window last had focus (ms), for the palette's ordering.
    pub focus_seen: BTreeMap<WindowId, u64>,
    pub last_sent: BTreeMap<WindowId, SentFrame>,
    /// Tiled windows the user is resizing by dragging an edge: the latest
    /// frame seen and when to commit it as new tile weights.
    pub drags: BTreeMap<WindowId, (Rect, u64)>,
    /// The gesture in progress (runtime only, not persisted).
    pub gesture: Option<Gesture>,
    /// Target frame per window from the last reconcile.
    pub desired: BTreeMap<WindowId, Rect>,
    /// Frames of profile slots waiting for their app, per workspace.
    pub slot_frames: Vec<(WorkspaceId, AppSlot, Rect)>,
    pub parked: BTreeMap<WindowId, ParkedEntry>,
    pub pending_slots: Vec<PendingSlot>,
    pub pending_launches: Vec<PendingLaunch>,
    /// Scratchpad name -> its window (persisted).
    pub scratch: BTreeMap<String, WindowId>,
    /// Scratchpad windows toggled off: parked wherever their workspace is.
    pub scratch_hidden: BTreeSet<WindowId>,
    pub pending_restore: Vec<ParkedEntry>,
    pub restored_count: usize,
    /// Window leaves whose window is gone but may come back (unix ms since).
    pub ghosts: BTreeMap<WindowId, u64>,
    /// Name of the profile applied last; stays until the user applies another.
    pub active_profile: Option<String>,
    /// Display set (`profile::topology_key`) that automatic profile choice
    /// last acted on (persisted). A desk change away from it picks again;
    /// on the same desk, a profile the user chose stays.
    pub auto_topology: Option<String>,
    /// Floating windows from `state.json` waiting to be rediscovered.
    pub expected_floating: BTreeMap<WindowId, (WorkspaceId, Option<Rect>)>,
    /// Layouts from before the last tree-changing commands, oldest first
    /// (at most `undo::UNDO_DEPTH`; runtime only, not persisted).
    pub undo: VecDeque<crate::undo::LayoutState>,
    pub now_ms: u64,
    pub gen: u64,
    pub seq: u64,
    pub next_request_id: u64,
    pub layout_dirty: bool,
    pub resync_at: Option<u64>,
    pub suppress_follow_until: u64,
    pub next_watchdog_ms: u64,
    pub shutting_down: bool,
    pub initial_scan_done: bool,
}

impl Model {
    pub fn new(config: Config, now_ms: u64) -> Model {
        let rules = config.compiled_rules().unwrap_or_default();
        Model {
            config,
            rules,
            config_error: None,
            displays: vec![],
            known_displays: BTreeSet::new(),
            display_frames: BTreeMap::new(),
            display_names: BTreeMap::new(),
            workspaces: vec![],
            visible: BTreeMap::new(),
            remembered_visible: BTreeMap::new(),
            tree: Tree::new(),
            windows: BTreeMap::new(),
            ignored: BTreeSet::new(),
            ignored_by_pid: BTreeMap::new(),
            apps: BTreeMap::new(),
            placements: vec![],
            focus_display: None,
            focus_window: None,
            os_focus: None,
            focus_seen: BTreeMap::new(),
            last_sent: BTreeMap::new(),
            drags: BTreeMap::new(),
            gesture: None,
            desired: BTreeMap::new(),
            slot_frames: vec![],
            parked: BTreeMap::new(),
            pending_slots: vec![],
            pending_launches: vec![],
            scratch: BTreeMap::new(),
            scratch_hidden: BTreeSet::new(),
            pending_restore: vec![],
            restored_count: 0,
            ghosts: BTreeMap::new(),
            expected_floating: BTreeMap::new(),
            active_profile: None,
            auto_topology: None,
            undo: VecDeque::new(),
            now_ms,
            gen: 0,
            seq: 0,
            next_request_id: 1,
            layout_dirty: true,
            resync_at: None,
            suppress_follow_until: 0,
            next_watchdog_ms: 0,
            shutting_down: false,
            initial_scan_done: false,
        }
    }

    /// Rebuild from `state.json`. Window leaves become ghosts until their
    /// windows are rediscovered. Displays arrive later via `DisplaysChanged`.
    pub fn from_state(config: Config, state: &State, now_ms: u64) -> Model {
        let mut m = Model::new(config, now_ms);
        for ws in &state.workspaces {
            // Files from before per-display workspaces have no `home`: the
            // workspace belongs to the display it was on.
            let home = ws.home.clone().unwrap_or_else(|| ws.display.clone());
            let name = ws
                .name
                .clone()
                .unwrap_or_else(|| crate::workspace::split_id(&ws.id).1.to_string());
            let id = ws_id(&home, &name);
            if m.ws(&id).is_some() {
                continue;
            }
            let root = m.tree.root_from_ser(&ws.tree);
            for leaf in m.tree.leaves(root) {
                let Some(Leaf::Window(wid)) = m.tree.leaf(leaf).cloned() else {
                    continue;
                };
                match state.window_slots.get(&wid) {
                    // Windows that filled a profile slot come back as the
                    // slot: their app's next window (any id) returns there.
                    Some(slot) => *m.tree.leaf_mut(leaf).unwrap() = Leaf::Slot(slot.clone()),
                    None => {
                        m.ghosts.insert(wid, now_ms);
                    }
                }
            }
            for f in &ws.floating {
                m.expected_floating.insert(f.wid, (id.clone(), f.frame));
            }
            m.workspaces.push(Workspace {
                id,
                home,
                name,
                display: ws.display.clone(),
                root,
                floating: vec![],
                focused: ws.focused,
                fullscreen: None,
            });
        }
        let migrate = |map: &BTreeMap<DisplayId, WorkspaceId>| -> BTreeMap<DisplayId, WorkspaceId> {
            map.iter()
                .map(|(d, w)| {
                    (
                        d.clone(),
                        if w.contains('/') {
                            w.clone()
                        } else {
                            ws_id(d, w)
                        },
                    )
                })
                .collect()
        };
        m.visible = migrate(&state.visible);
        m.remembered_visible = migrate(&state.remembered_visible);
        m.placements = state.placements.clone();
        for r in &mut m.placements {
            if !r.workspace.contains('/') {
                // Old global name: find the workspace it became.
                if let Some(w) = m.workspaces.iter().find(|w| w.name == r.workspace.as_str()) {
                    r.workspace = w.id.clone();
                }
            }
        }
        m.focus_display = state.focus_display.clone();
        m.focus_window = state.focus_window;
        m.known_displays = state.displays.iter().cloned().collect();
        m.display_frames = state.display_frames.clone();
        m.display_names = state.display_names.clone();
        m.active_profile = state.active_profile.clone();
        m.auto_topology = state.auto_topology.clone();
        m.scratch = state.scratchpads.clone();
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
                    home: Some(w.home.clone()),
                    name: Some(w.name.clone()),
                    display: w.display.clone(),
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
            display_frames: self.display_frames.clone(),
            display_names: self.display_names.clone(),
            active_profile: self.active_profile.clone(),
            window_slots: self
                .windows
                .values()
                .filter_map(|w| w.slot.clone().map(|s| (w.id, s)))
                .collect(),
            scratchpads: self.scratch.clone(),
            auto_topology: self.auto_topology.clone(),
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

    /// Is the window meant to be on screen: its workspace is shown and it
    /// is not a scratchpad toggled off. (Minimized windows count as shown.)
    pub fn window_shown(&self, w: &WindowInfo) -> bool {
        self.is_visible(&w.workspace) && !self.scratch_hidden.contains(&w.id)
    }

    /// The scratchpad a window is, if any.
    pub fn scratchpad_of(&self, wid: WindowId) -> Option<&str> {
        self.scratch
            .iter()
            .find(|(_, w)| **w == wid)
            .map(|(n, _)| n.as_str())
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

    /// Workspace `name` of display `home`, created if missing. A new
    /// workspace is on its home display, or on that display's host while
    /// it is unplugged.
    pub fn ensure_workspace(&mut self, home: &str, name: &str) -> WorkspaceId {
        // Never create a workspace for a display Pane has never seen (an
        // empty or stale id): it would be a hidden guest nobody can reach.
        let fallback;
        let home = if self.knows_display(home) {
            home
        } else {
            fallback = self.focused_display().unwrap_or_default();
            fallback.as_str()
        };
        let id = ws_id(home, name);
        if self.ws(&id).is_some() {
            return id;
        }
        let display = if self.display(home).is_some() {
            home.to_string()
        } else {
            self.workspaces
                .iter()
                .find(|w| w.home == home)
                .map(|w| w.display.clone())
                .or_else(|| self.focused_display())
                .unwrap_or_else(|| home.to_string())
        };
        let root = self
            .tree
            .new_root(Default::default(), self.config.settings.default_layout);
        self.workspaces.push(Workspace {
            id: id.clone(),
            home: home.to_string(),
            name: name.to_string(),
            display,
            root,
            floating: vec![],
            focused: None,
            fullscreen: None,
        });
        id
    }

    /// Connected now, seen before, or owning workspaces.
    pub fn knows_display(&self, id: &str) -> bool {
        !id.is_empty()
            && (self.display(id).is_some()
                || self.display_frames.contains_key(id)
                || self.workspaces.iter().any(|w| w.home == id))
    }

    /// Workspaces on `display` (its own first, by name, then guests).
    pub fn workspaces_on(&self, display: &str) -> Vec<&Workspace> {
        let mut v: Vec<&Workspace> = self
            .workspaces
            .iter()
            .filter(|w| w.display == display)
            .collect();
        v.sort_by_key(|w| (w.is_guest(), w.home.clone(), name_order(&w.name)));
        v
    }

    /// Resolve a workspace target as typed by the user: `2` (on the focused
    /// display) or `<display>:2` (`left:2`, `main:web`, `uuid:...:3`, a config
    /// alias). Unplugged displays can be named by their old position too.
    pub fn resolve_target(&self, target: &str) -> Option<(DisplayId, String)> {
        let target = target.trim();
        match target.rsplit_once(':') {
            Some((d, name)) if !d.is_empty() => {
                let r = self.config.display_ref(d);
                let home = r
                    .resolve(&self.displays)
                    .map(|x| x.id.clone())
                    .or_else(|| {
                        self.workspaces
                            .iter()
                            .find(|w| w.home.eq_ignore_ascii_case(d.trim_start_matches("uuid:")))
                            .map(|w| w.home.clone())
                    })?;
                Some((home, name.trim().to_string()))
            }
            _ => Some((self.focused_display()?, target.to_string())),
        }
    }

    /// Is a leaf laid out? Ghosts, minimized and floating windows are not.
    pub fn leaf_present(&self, leaf: &Leaf) -> bool {
        match leaf {
            Leaf::Window(w) => {
                self.windows
                    .get(w)
                    .is_some_and(|x| !x.minimized && x.mode == WindowMode::Tiled)
                    && !self.scratch_hidden.contains(w)
            }
            // A profile slot takes space only while its app is being launched;
            // otherwise it waits invisibly for the app's next window.
            Leaf::Slot(s) => self
                .pending_slots
                .iter()
                .any(|p| p.bundle_id == s.bundle_id),
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
        t.extend(self.pending_launches.iter().map(|p| p.deadline_ms));
        t.extend(self.drags.values().map(|d| d.1));
        t.extend(self.resync_at);
        if !self.windows.is_empty() {
            t.push(self.next_watchdog_ms.max(self.now_ms));
        }
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
