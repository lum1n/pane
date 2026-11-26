//! `reduce(model, event) -> effects`: every state change goes through here.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::assign::{assign, next_free_name, AssignInput, WsInput};
use crate::display::{display_for_rect, park_rect, rank_displays, DisplayRef, Position};
use crate::effect::Effect;
use crate::event::{Event, WindowProbe};
use crate::geometry::{neighbor_in_direction, Direction, Orientation, Rect};
use crate::identity::{match_windows, PlacementRecord, WindowFingerprint};
use crate::model::{
    AppInfo, Model, PendingSlot, WindowInfo, WindowMode, FOLLOW_SUPPRESS_MS, GHOST_TTL_MS,
    MAX_RECORDS_PER_APP, PENDING_SLOT_MS, RESYNC_DELAY_MS, UNRESPONSIVE_AFTER_MS,
};
use crate::profile::{title_matches, Profile, ProfileDisplay};
use crate::proto::{Command, QueryKind, Reply, UiRequest};
use crate::reconcile;
use crate::rules::{decide, Decision, RuleAction, WindowTraits};
use crate::tree::{InsertAt, Leaf, MoveResult, NodeId, SerNode};
use crate::workspace::Cycle;
use crate::{Pid, WindowId, WorkspaceId};

/// Quiet time after a user move before a tiled window snaps back.
pub const SNAP_BACK_MS: u64 = 300;

pub fn reduce(m: &mut Model, ev: Event, now_ms: u64) -> Vec<Effect> {
    m.now_ms = now_ms;
    let mut fx = Vec::new();
    if let Some(pid) = ev.from_pid() {
        mark_responsive(m, pid, &mut fx);
    }
    match ev {
        Event::AppLaunched {
            pid,
            bundle_id,
            name,
            regular,
        } => {
            let Some(bundle_id) = bundle_id else {
                return fx;
            };
            if m.apps.contains_key(&pid) {
                return fx;
            }
            m.apps.insert(
                pid,
                AppInfo {
                    pid,
                    bundle_id,
                    name,
                    regular,
                    unresponsive: false,
                    awaiting_since: None,
                },
            );
            fx.push(Effect::Observe { pid });
        }
        Event::AppTerminated { pid } => app_terminated(m, pid, &mut fx),
        Event::WindowsDiscovered {
            pid,
            windows,
            initial,
        } => discover(m, pid, windows, initial, &mut fx),
        Event::WindowDestroyed { wid, .. } => window_destroyed(m, wid, &mut fx),
        Event::WindowFrameChanged { wid, frame, .. } => frame_changed(m, wid, frame, &mut fx),
        Event::TitleChanged { wid, title, .. } => {
            if let Some(w) = m.windows.get_mut(&wid) {
                w.title = title;
                fx.push(Effect::Snapshot);
            }
        }
        Event::FocusChanged { wid, .. } => {
            if let Some(wid) = wid {
                focus_changed(m, wid, &mut fx);
            }
        }
        Event::WindowMinimized { wid, minimized, .. } => {
            if let Some(w) = m.windows.get_mut(&wid) {
                w.minimized = minimized;
                m.last_sent.remove(&wid);
                m.layout_dirty = true;
                fx.push(Effect::Snapshot);
            }
        }
        Event::FrameApplied {
            wid,
            gen,
            ok,
            actual,
            ..
        } => {
            reconcile::on_frame_applied(m, wid, gen, ok, actual, &mut fx);
        }
        Event::DisplaysChanged { mut displays } => {
            rank_displays(&mut displays);
            let prev = if m.displays.is_empty() {
                m.known_displays.clone()
            } else {
                m.displays.iter().map(|d| d.id.clone()).collect()
            };
            m.displays = displays;
            reassign(m, &prev);
            m.known_displays = m.displays.iter().map(|d| d.id.clone()).collect();
            m.resync_at = Some(now_ms + RESYNC_DELAY_MS);
            m.layout_dirty = true;
            fx.push(Effect::PersistState);
            fx.push(Effect::Snapshot);
        }
        Event::Command { cmd, token } => {
            let reply = command(m, cmd, &mut fx);
            if let Some(token) = token {
                fx.push(Effect::Reply { token, reply });
            } else if let Reply::Error { message } = reply {
                fx.push(Effect::UiRequest(UiRequest::warn(message)));
            }
        }
        Event::ConfigLoaded { config } => {
            m.rules = config.compiled_rules().unwrap_or_default();
            m.config = *config;
            m.config_error = None;
            m.ensure_configured_workspaces();
            // Empty workspaces the new config no longer names go away, even
            // when visible, so the configured ones can take their display.
            let stale: Vec<WorkspaceId> = m
                .workspaces
                .iter()
                .filter(|w| {
                    !w.configured && m.tree.leaves(w.root).is_empty() && w.floating.is_empty()
                })
                .map(|w| w.id.clone())
                .collect();
            m.visible.retain(|_, v| !stale.contains(v));
            gc_workspaces(m);
            let prev = m.displays.iter().map(|d| d.id.clone()).collect();
            reassign(m, &prev);
            m.last_sent.clear();
            m.layout_dirty = true;
            fx.push(Effect::UiRequest(UiRequest::info("Config reloaded")));
            fx.push(Effect::PersistState);
            fx.push(Effect::Snapshot);
        }
        Event::ConfigFailed { error } => {
            fx.push(Effect::UiRequest(UiRequest::warn(format!(
                "Config error: {}",
                error.render()
            ))));
            m.config_error = Some(error);
            fx.push(Effect::Snapshot);
        }
        Event::RestoreParked { entries } => {
            m.pending_restore = entries;
        }
        Event::InitialScanDone => {
            m.initial_scan_done = true;
            if m.restored_count > 0 {
                fx.push(Effect::UiRequest(UiRequest::info(format!(
                    "Restored {} window(s) left hidden by a previous run",
                    m.restored_count
                ))));
            }
            // Entries for apps that are gone are recovered by the identity matcher.
            m.pending_restore.clear();
        }
        Event::AppOpenResult {
            request_id,
            ok,
            error,
        } => {
            if !ok {
                let gone: Vec<PendingSlot> = m
                    .pending_slots
                    .iter()
                    .filter(|p| p.request_id == request_id)
                    .cloned()
                    .collect();
                m.pending_slots.retain(|p| p.request_id != request_id);
                for p in &gone {
                    remove_slot_leaf(m, &p.workspace, &p.bundle_id);
                }
                if let Some(p) = gone.first() {
                    fx.push(Effect::UiRequest(UiRequest::warn(format!(
                        "Could not open {}: {}",
                        p.bundle_id,
                        error.unwrap_or_else(|| "unknown error".into())
                    ))));
                }
                m.layout_dirty = true;
            }
        }
        Event::Tick => tick(m, &mut fx),
        Event::WillSleep => fx.push(Effect::PersistState),
        Event::DidWake => {
            m.resync_at = Some(now_ms + RESYNC_DELAY_MS);
        }
        Event::Shutdown => {
            m.shutting_down = true;
            m.layout_dirty = true;
        }
    }
    fx
}

fn info(fx: &mut Vec<Effect>, msg: impl Into<String>) {
    fx.push(Effect::UiRequest(UiRequest::info(msg)));
}

fn warn(fx: &mut Vec<Effect>, msg: impl Into<String>) {
    fx.push(Effect::UiRequest(UiRequest::warn(msg)));
}

fn mark_responsive(m: &mut Model, pid: Pid, fx: &mut Vec<Effect>) {
    let Some(app) = m.apps.get_mut(&pid) else {
        return;
    };
    if app.unresponsive {
        app.unresponsive = false;
        app.awaiting_since = None;
        let name = app.name.clone();
        let wids: Vec<WindowId> = m
            .windows
            .values()
            .filter(|w| w.pid == pid)
            .map(|w| w.id)
            .collect();
        for w in wids {
            m.last_sent.remove(&w);
        }
        m.layout_dirty = true;
        info(fx, format!("{name} is responding again"));
    }
}

// ---------------------------------------------------------------- displays

fn reassign(m: &mut Model, prev: &BTreeSet<String>) {
    if m.displays.is_empty() {
        return;
    }
    let inputs: Vec<WsInput> = m
        .workspaces
        .iter()
        .map(|w| WsInput {
            id: w.id.clone(),
            prefs: w.prefs.clone(),
            current: Some(w.display.clone()).filter(|d| !d.is_empty()),
            last_display: w.last_display.clone(),
        })
        .collect();
    let a = assign(&AssignInput {
        displays: &m.displays,
        prev_displays: prev,
        workspaces: &inputs,
        prev_visible: &m.visible,
        remembered_visible: &m.remembered_visible,
        return_on_reconnect: m.config.settings.return_workspaces_on_reconnect,
    });
    for (w, d) in &a.evicted {
        if let Some(ws) = m.ws_mut(w) {
            ws.last_display.get_or_insert(d.clone());
        }
    }
    for ws in &mut m.workspaces {
        if let Some(d) = a.ws_display.get(&ws.id) {
            ws.display = d.clone();
        }
        if ws.last_display.as_ref() == Some(&ws.display) {
            ws.last_display = None;
        }
    }
    for (w, d) in &a.created {
        m.ensure_workspace(w, Some(d.clone()));
    }
    m.visible = a.visible;
    for (d, w) in &m.visible {
        m.remembered_visible.insert(d.clone(), w.clone());
    }
    if m.focus_display
        .as_ref()
        .is_none_or(|d| m.display(d).is_none())
    {
        m.focus_display = m.main_display_id();
    }
}

// ---------------------------------------------------------------- windows

fn traits<'a>(p: &'a WindowProbe, app: &'a AppInfo) -> WindowTraits<'a> {
    WindowTraits {
        bundle_id: &app.bundle_id,
        title: &p.title,
        role: p.role,
        resizable: p.resizable,
        modal: p.modal,
        has_close_button: p.has_close_button,
        width: p.frame.w,
        height: p.frame.h,
        regular_app: app.regular,
    }
}

/// Is `r` sitting at some display's park corner?
pub fn looks_parked(m: &Model, r: &Rect) -> bool {
    m.displays
        .iter()
        .any(|d| park_rect(d, r.size()).origin_near(r, 4.0))
}

enum Plan {
    Tile(InsertAt, Option<f64>),
    Replace(NodeId),
    Float(Rect),
}

fn discover(
    m: &mut Model,
    pid: Pid,
    probes: Vec<WindowProbe>,
    initial: bool,
    fx: &mut Vec<Effect>,
) {
    let Some(app) = m.apps.get(&pid).cloned() else {
        return;
    };
    let existing = m.windows.values().filter(|w| w.pid == pid).count();
    let mut fresh: Vec<(WindowProbe, u32)> = Vec::new();
    for (i, p) in probes.into_iter().enumerate() {
        if let Some(w) = m.windows.get_mut(&p.wid) {
            w.actual = p.frame;
            w.title = p.title;
            continue;
        }
        if m.ignored.contains(&p.wid) {
            continue;
        }
        let ordinal = if initial { i } else { existing + i } as u32;
        fresh.push((p, ordinal));
    }
    if fresh.is_empty() {
        return;
    }

    // Windows a previous run left parked.
    let mut restore: BTreeMap<WindowId, Rect> = BTreeMap::new();
    for (p, _) in &fresh {
        let pos = m.pending_restore.iter().position(|e| {
            (e.pid == pid && e.wid == p.wid)
                || (e.bundle_id == app.bundle_id && e.title == p.title && !p.title.is_empty())
        });
        if let Some(pos) = pos {
            let e = m.pending_restore.remove(pos);
            m.restored_count += 1;
            restore.insert(p.wid, e.original);
        }
    }

    let decisions: Vec<Decision> = fresh
        .iter()
        .map(|(p, _)| {
            decide(
                &m.rules,
                &traits(p, &app),
                m.config.settings.auto_float_below,
            )
        })
        .collect();

    // Placement records, matched globally for the whole batch.
    let mut record_for: BTreeMap<usize, usize> = BTreeMap::new();
    if m.config.settings.restore_placements {
        let cand: Vec<usize> = (0..fresh.len())
            .filter(|i| {
                let d = &decisions[*i];
                d.action == RuleAction::Tile || (d.action == RuleAction::Float && d.explicit)
            })
            .filter(|i| {
                !in_any_tree(m, fresh[*i].0.wid)
                    && !m.expected_floating.contains_key(&fresh[*i].0.wid)
            })
            .collect();
        let fps: Vec<WindowFingerprint> = cand
            .iter()
            .map(|i| fingerprint(&app.bundle_id, &fresh[*i].0, fresh[*i].1))
            .collect();
        let rec_idx: Vec<usize> = (0..m.placements.len())
            .filter(|r| {
                m.placements[*r].live.is_none() && m.placements[*r].fp.bundle_id == app.bundle_id
            })
            .collect();
        let recs: Vec<&PlacementRecord> = rec_idx.iter().map(|r| &m.placements[*r]).collect();
        for (wi, ri) in match_windows(&fps, &recs) {
            record_for.insert(cand[wi], rec_idx[ri]);
        }
    }

    let mut changed = false;
    for (i, (p, ordinal)) in fresh.into_iter().enumerate() {
        let d = &decisions[i];
        if d.action == RuleAction::Ignore {
            m.ignored.insert(p.wid);
            if let Some(orig) = restore.get(&p.wid) {
                fx.push(set_frame_once(m, &app, p.wid, *orig));
            }
            continue;
        }
        let parked_now = looks_parked(m, &p.frame);
        let restore_frame = restore.get(&p.wid).copied();
        let sane_frame = |m: &Model, ws: &str| -> Rect {
            if !parked_now {
                return p.frame;
            }
            let area = m
                .ws(ws)
                .and_then(|w| m.display(&w.display))
                .map(|d| d.visible_frame)
                .unwrap_or(p.frame);
            match restore_frame {
                Some(r) => r.fit_into(&area),
                None => Rect::new(
                    area.center().x - p.frame.w / 2.0,
                    area.center().y - p.frame.h / 2.0,
                    p.frame.w,
                    p.frame.h,
                )
                .fit_into(&area),
            }
        };

        // 1. Leaf from state.json / ghost of an app that quit.
        let mut target: Option<(WorkspaceId, Plan)> = None;
        if let Some((ws, node)) = find_in_trees(m, p.wid) {
            target = Some((ws, Plan::Replace(node)));
        } else if let Some((ws, frame)) = m.expected_floating.remove(&p.wid) {
            if m.ws(&ws).is_some() {
                let f = frame.unwrap_or_else(|| sane_frame(m, &ws));
                target = Some((ws, Plan::Float(f)));
            }
        }
        // 2. A profile slot waiting for this app.
        if target.is_none() && d.action == RuleAction::Tile {
            if let Some(pos) = m
                .pending_slots
                .iter()
                .position(|s| s.bundle_id == app.bundle_id)
            {
                let s = m.pending_slots.remove(pos);
                if let Some(node) = m.ws(&s.workspace).and_then(|w| {
                    m.tree.find_leaf(
                        w.root,
                        |l| matches!(l, Leaf::Slot(sl) if sl.bundle_id == s.bundle_id),
                    )
                }) {
                    target = Some((s.workspace.clone(), Plan::Replace(node)));
                }
            }
        }
        // 3. A placement record from an earlier run.
        let record = record_for.get(&i).copied();
        if target.is_none() {
            if let Some(ri) = record {
                let r = m.placements[ri].clone();
                let ws = m.ensure_workspace(&r.workspace, None);
                if r.floating {
                    let f = r.float_frame.unwrap_or_else(|| sane_frame(m, &ws));
                    target = Some((ws, Plan::Float(f)));
                } else {
                    let root = m.ws(&ws).unwrap().root;
                    let ghost = r
                        .last_wid
                        .filter(|w| !m.windows.contains_key(w))
                        .and_then(|w| m.tree.find_window(root, w));
                    target = Some(match ghost {
                        Some(node) => (ws, Plan::Replace(node)),
                        None => (
                            ws,
                            Plan::Tile(InsertAt::Path(r.anchor.clone()), Some(r.weight)),
                        ),
                    });
                }
            }
        }
        // 4. Rules / defaults.
        let (ws, plan) = match target {
            Some(t) => t,
            None => {
                let ws = d
                    .workspace
                    .as_ref()
                    .map(|w| m.ensure_workspace(w, None))
                    .or_else(|| d.display.as_ref().and_then(|r| visible_on_ref(m, r)))
                    .or_else(|| {
                        if initial {
                            display_for_rect(&m.displays, &p.frame)
                                .and_then(|d| m.visible.get(&d.id).cloned())
                        } else {
                            None
                        }
                    })
                    .or_else(|| m.focused_ws_id())
                    .or_else(|| m.workspaces.first().map(|w| w.id.clone()));
                let Some(ws) = ws else { continue };
                let plan = if d.action == RuleAction::Float {
                    Plan::Float(sane_frame(m, &ws))
                } else {
                    Plan::Tile(default_insert(m, &ws), None)
                };
                (ws, plan)
            }
        };

        let seq = m.next_seq();
        let mut info = WindowInfo {
            id: p.wid,
            pid,
            bundle_id: app.bundle_id.clone(),
            title: p.title.clone(),
            role: p.role,
            resizable: p.resizable,
            document: p.document.clone(),
            min_size: p.min_size,
            actual: p.frame,
            ordinal,
            mode: WindowMode::Tiled,
            workspace: ws.clone(),
            minimized: p.minimized,
            stubborn: false,
            double_set: m.config.is_electron(&app.bundle_id),
            float_frame: None,
            seq,
        };
        let root = m.ws(&ws).unwrap().root;
        match plan {
            Plan::Tile(at, weight) => {
                let n = m.tree.insert_leaf(root, Leaf::Window(p.wid), at);
                if let Some(w) = weight {
                    m.tree.set_weight(n, w);
                }
            }
            Plan::Replace(node) => {
                if let Some(Leaf::Window(old)) = m.tree.leaf(node).cloned() {
                    m.ghosts.remove(&old);
                }
                *m.tree.leaf_mut(node).unwrap() = Leaf::Window(p.wid);
            }
            Plan::Float(frame) => {
                info.mode = WindowMode::Floating;
                info.float_frame = Some(frame);
                m.ws_mut(&ws).unwrap().floating.push(p.wid);
            }
        }
        m.ghosts.remove(&p.wid);
        let standard = d.action == RuleAction::Tile || d.explicit;
        m.windows.insert(p.wid, info);
        if let Some(ri) = record {
            m.placements[ri].live = Some(p.wid);
        } else if standard {
            let fp = fingerprint(&app.bundle_id, &p, ordinal);
            m.placements.push(PlacementRecord {
                fp,
                workspace: ws.clone(),
                anchor: vec![],
                weight: 1.0,
                floating: false,
                float_frame: None,
                seen_at: m.now_ms,
                last_wid: Some(p.wid),
                live: Some(p.wid),
            });
        }
        refresh_record(m, p.wid);
        changed = true;
    }
    if changed {
        m.layout_dirty = true;
        fx.push(Effect::PersistState);
        fx.push(Effect::Snapshot);
    }
}

fn fingerprint(bundle: &str, p: &WindowProbe, ordinal: u32) -> WindowFingerprint {
    WindowFingerprint::new(
        bundle,
        &p.title,
        p.document.as_deref(),
        ordinal,
        p.frame.size(),
    )
}

fn set_frame_once(m: &mut Model, app: &AppInfo, wid: WindowId, rect: Rect) -> Effect {
    let gen = m.next_gen();
    Effect::SetFrame {
        pid: app.pid,
        wid,
        rect,
        gen,
        double_set: m.config.is_electron(&app.bundle_id),
        restore_to: None,
    }
}

fn in_any_tree(m: &Model, wid: WindowId) -> bool {
    find_in_trees(m, wid).is_some()
}

fn find_in_trees(m: &Model, wid: WindowId) -> Option<(WorkspaceId, NodeId)> {
    m.workspaces
        .iter()
        .find_map(|w| m.tree.find_window(w.root, wid).map(|n| (w.id.clone(), n)))
}

fn visible_on_ref(m: &Model, r: &DisplayRef) -> Option<WorkspaceId> {
    let d = r.resolve(&m.displays)?;
    m.visible.get(&d.id).cloned()
}

/// Next to the workspace's focused tiled window, else at the end.
fn default_insert(m: &Model, ws: &str) -> InsertAt {
    let Some(w) = m.ws(ws) else {
        return InsertAt::End;
    };
    match w.focused.and_then(|f| m.tree.find_window(w.root, f)) {
        Some(n)
            if m.windows
                .get(&w.focused.unwrap())
                .is_some_and(|x| x.mode == WindowMode::Tiled) =>
        {
            InsertAt::After(n)
        }
        _ => InsertAt::End,
    }
}

/// Update the live placement record of a window from the model.
fn refresh_record(m: &mut Model, wid: WindowId) {
    let Some(w) = m.windows.get(&wid) else { return };
    let Some(ri) = m.placements.iter().position(|r| r.live == Some(wid)) else {
        return;
    };
    let (anchor, weight) = match m.window_node(wid) {
        Some(n) => (m.tree.path_of(n), m.tree.weight_of(n)),
        None => (m.placements[ri].anchor.clone(), m.placements[ri].weight),
    };
    let size = if looks_parked(m, &w.actual) {
        m.placements[ri].fp.size
    } else {
        w.actual.size()
    };
    let r = &mut m.placements[ri];
    r.fp = WindowFingerprint::new(
        &w.bundle_id,
        &w.title,
        w.document.as_deref(),
        w.ordinal,
        size,
    );
    r.workspace = w.workspace.clone();
    r.anchor = anchor;
    r.weight = weight;
    r.floating = w.mode == WindowMode::Floating;
    r.float_frame = w.float_frame;
    r.seen_at = m.now_ms;
    r.last_wid = Some(wid);
}

/// Refresh every live record and drop expired ones. Called before persisting.
pub fn refresh_placements(m: &mut Model) {
    let live: Vec<WindowId> = m.placements.iter().filter_map(|r| r.live).collect();
    for w in live {
        refresh_record(m, w);
    }
    let ttl = m.config.settings.placement_memory_days as u64 * 86_400_000;
    let now = m.now_ms;
    m.placements
        .retain(|r| r.live.is_some() || r.seen_at + ttl >= now);
    // Cap per app, dropping the oldest records that are not live.
    let mut per_app: BTreeMap<String, Vec<(u64, usize)>> = BTreeMap::new();
    for (i, r) in m.placements.iter().enumerate() {
        if r.live.is_none() {
            per_app
                .entry(r.fp.bundle_id.clone())
                .or_default()
                .push((r.seen_at, i));
        }
    }
    let mut drop: BTreeSet<usize> = BTreeSet::new();
    for (_, mut v) in per_app {
        if v.len() > MAX_RECORDS_PER_APP {
            v.sort();
            drop.extend(v[..v.len() - MAX_RECORDS_PER_APP].iter().map(|x| x.1));
        }
    }
    let mut i = 0;
    m.placements.retain(|_| {
        let keep = !drop.contains(&i);
        i += 1;
        keep
    });
}

/// Take a window out of its workspace. With `ghost`, a tiled window's leaf
/// stays in the tree (taking no space) so the window can come back to it.
fn detach_window(m: &mut Model, wid: WindowId, ghost: bool) -> Option<WorkspaceId> {
    let w = m.windows.get(&wid)?.clone();
    let ws_id = w.workspace.clone();
    if let Some(node) = m.window_node(wid) {
        if ghost {
            m.ghosts.insert(wid, m.now_ms);
        } else {
            m.tree.remove(node);
        }
    }
    if let Some(ws) = m.ws_mut(&ws_id) {
        ws.floating.retain(|f| *f != wid);
        if ws.focused == Some(wid) {
            ws.focused = None;
        }
    }
    Some(ws_id)
}

fn forget_window(m: &mut Model, wid: WindowId, ghost: bool) -> Option<WorkspaceId> {
    refresh_record(m, wid);
    if let Some(r) = m.placements.iter_mut().find(|r| r.live == Some(wid)) {
        r.live = None;
        if !ghost {
            r.last_wid = None;
        }
    }
    let ws = detach_window(m, wid, ghost);
    m.windows.remove(&wid);
    m.last_sent.remove(&wid);
    m.desired.remove(&wid);
    m.parked.remove(&wid);
    if m.focus_window == Some(wid) {
        m.focus_window = None;
    }
    ws
}

fn app_terminated(m: &mut Model, pid: Pid, fx: &mut Vec<Effect>) {
    if m.apps.remove(&pid).is_none() {
        return;
    }
    let wids: Vec<WindowId> = m
        .windows
        .values()
        .filter(|w| w.pid == pid)
        .map(|w| w.id)
        .collect();
    let had_parked = wids.iter().any(|w| m.parked.contains_key(w));
    for w in &wids {
        forget_window(m, *w, true);
    }
    fx.push(Effect::Unobserve { pid });
    if had_parked {
        fx.push(Effect::WriteParked {
            entries: m.parked.values().cloned().collect(),
        });
    }
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
}

fn window_destroyed(m: &mut Model, wid: WindowId, fx: &mut Vec<Effect>) {
    if m.ignored.remove(&wid) {
        return;
    }
    let was_focused = m.focus_window == Some(wid);
    let had_parked = m.parked.contains_key(&wid);
    let Some(ws) = forget_window(m, wid, false) else {
        return;
    };
    if had_parked {
        fx.push(Effect::WriteParked {
            entries: m.parked.values().cloned().collect(),
        });
    }
    if was_focused && m.is_visible(&ws) {
        // Keep focus inside the workspace instead of letting macOS pick a
        // window that may be parked on another one.
        m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
        if let Some(next) = first_window(m, &ws) {
            focus_fx(m, next, fx);
        }
    }
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
}

fn frame_changed(m: &mut Model, wid: WindowId, frame: Rect, fx: &mut Vec<Effect>) {
    let Some(w) = m.windows.get_mut(&wid) else {
        return;
    };
    w.actual = frame;
    if w.minimized || w.stubborn {
        return;
    }
    let mode = w.mode;
    let ws = w.workspace.clone();
    if let Some(s) = m.last_sent.get(&wid) {
        if s.rect.approx_eq(&frame, 1.0) || m.now_ms.saturating_sub(s.at_ms) <= 150 {
            return; // Our own move, or the tail of it.
        }
    }
    let visible = m.is_visible(&ws);
    match (mode, visible) {
        (WindowMode::Floating, true) => {
            m.windows.get_mut(&wid).unwrap().float_frame = Some(frame);
            m.last_sent.remove(&wid);
            m.desired.insert(wid, frame);
            // Dragged to another display: join that display's workspace.
            if let Some(d) = display_for_rect(&m.displays, &frame) {
                let target = m.visible.get(&d.id).cloned();
                if let Some(target) = target.filter(|t| *t != ws) {
                    detach_window(m, wid, false);
                    m.ws_mut(&target).unwrap().floating.push(wid);
                    m.windows.get_mut(&wid).unwrap().workspace = target;
                    fx.push(Effect::Snapshot);
                    fx.push(Effect::PersistState);
                }
            }
        }
        _ => {
            // Tiled windows snap back and parked windows get re-parked, but
            // only once the moves stop: resending on every event would fight
            // a drag in progress.
            match m.last_sent.get_mut(&wid) {
                Some(s) => {
                    s.settled = false;
                    s.next_retry_ms = Some(m.now_ms + SNAP_BACK_MS);
                }
                None => m.layout_dirty = true,
            }
        }
    }
}

fn focus_changed(m: &mut Model, wid: WindowId, fx: &mut Vec<Effect>) {
    let Some(w) = m.windows.get(&wid) else { return };
    let ws = w.workspace.clone();
    if !m.is_visible(&ws) {
        if m.now_ms < m.suppress_follow_until {
            if let Some(f) = m.focused_window() {
                focus_fx(m, f, fx);
            }
            return;
        }
        // Cmd-Tab or a Dock click on a parked window: show its workspace.
        show_workspace(m, &ws, fx, false);
    }
    let d = m.ws(&ws).map(|w| w.display.clone());
    m.focus_window = Some(wid);
    m.focus_display = d;
    if let Some(w) = m.ws_mut(&ws) {
        w.focused = Some(wid);
    }
    fx.push(Effect::Snapshot);
}

fn focus_fx(m: &mut Model, wid: WindowId, fx: &mut Vec<Effect>) {
    let Some(w) = m.windows.get(&wid) else { return };
    let (pid, ws) = (w.pid, w.workspace.clone());
    fx.push(Effect::Focus { pid, wid });
    m.focus_window = Some(wid);
    m.focus_display = m.ws(&ws).map(|w| w.display.clone());
    if let Some(w) = m.ws_mut(&ws) {
        w.focused = Some(wid);
    }
    fx.push(Effect::Snapshot);
}

fn first_window(m: &Model, ws: &str) -> Option<WindowId> {
    let w = m.ws(ws)?;
    m.tree
        .windows(w.root)
        .into_iter()
        .find(|x| {
            m.windows
                .get(x)
                .is_some_and(|i| !i.minimized && i.mode == WindowMode::Tiled)
        })
        .or_else(|| w.floating.last().copied())
}

fn show_workspace(m: &mut Model, ws: &str, fx: &mut Vec<Effect>, focus: bool) {
    let Some(w) = m.ws(ws) else { return };
    let mut d = w.display.clone();
    if m.display(&d).is_none() {
        let Some(fd) = m.focused_display() else {
            return;
        };
        d = fd;
        m.ws_mut(ws).unwrap().display = d.clone();
    }
    m.visible.insert(d.clone(), ws.into());
    m.remembered_visible.insert(d.clone(), ws.into());
    m.focus_display = Some(d);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    m.layout_dirty = true;
    let floats = m.ws(ws).unwrap().floating.clone();
    for f in floats {
        if let Some(i) = m.windows.get(&f) {
            fx.push(Effect::Raise { pid: i.pid, wid: f });
        }
    }
    if focus {
        let w = m.ws(ws).unwrap();
        let target = w
            .focused
            .filter(|f| m.windows.get(f).is_some_and(|i| i.workspace == ws))
            .or_else(|| first_window(m, ws));
        match target {
            Some(t) => focus_fx(m, t, fx),
            None => m.focus_window = None,
        }
    }
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
}

/// After an empty, unconfigured workspace stops being visible, drop it.
fn gc_workspaces(m: &mut Model) {
    let dead: Vec<WorkspaceId> = m
        .workspaces
        .iter()
        .filter(|w| !w.configured && !m.visible.values().any(|v| *v == w.id))
        .filter(|w| m.tree.leaves(w.root).is_empty() && w.floating.is_empty())
        .filter(|w| !m.pending_slots.iter().any(|p| p.workspace == w.id))
        .map(|w| w.id.clone())
        .collect();
    for id in dead {
        if let Some(pos) = m.workspaces.iter().position(|w| w.id == id) {
            let w = m.workspaces.remove(pos);
            m.tree.remove_root(w.root);
            m.remembered_visible.retain(|_, v| *v != id);
        }
    }
}

/// Drop ghost leaves in a workspace before the user rearranges it.
fn prune_ghosts(m: &mut Model, ws: &str) {
    let Some(root) = m.ws(ws).map(|w| w.root) else {
        return;
    };
    let ghosts: Vec<NodeId> = m
        .tree
        .leaves(root)
        .into_iter()
        .filter(|n| matches!(m.tree.leaf(*n), Some(Leaf::Window(w)) if !m.windows.contains_key(w)))
        .collect();
    for n in ghosts {
        if let Some(Leaf::Window(w)) = m.tree.leaf(n).cloned() {
            m.ghosts.remove(&w);
        }
        m.tree.remove(n);
    }
}

fn remove_slot_leaf(m: &mut Model, ws: &str, bundle: &str) {
    let Some(root) = m.ws(ws).map(|w| w.root) else {
        return;
    };
    if let Some(n) = m.tree.find_leaf(
        root,
        |l| matches!(l, Leaf::Slot(s) if s.bundle_id == bundle),
    ) {
        m.tree.remove(n);
    }
}

fn tick(m: &mut Model, fx: &mut Vec<Effect>) {
    let now = m.now_ms;
    // Unresponsive apps.
    for app in m.apps.values_mut() {
        if !app.unresponsive
            && app
                .awaiting_since
                .is_some_and(|s| now >= s + UNRESPONSIVE_AFTER_MS)
        {
            app.unresponsive = true;
            fx.push(Effect::UiRequest(UiRequest::warn(format!(
                "{} is not responding; leaving its windows alone",
                app.name
            ))));
        }
    }
    // Expired profile slots.
    let expired: Vec<PendingSlot> = m
        .pending_slots
        .iter()
        .filter(|p| p.deadline_ms <= now)
        .cloned()
        .collect();
    if !expired.is_empty() {
        m.pending_slots.retain(|p| p.deadline_ms > now);
        for p in &expired {
            remove_slot_leaf(m, &p.workspace, &p.bundle_id);
        }
        warn(
            fx,
            format!("{} did not open a window in time", expired[0].bundle_id),
        );
        m.layout_dirty = true;
    }
    // Ghosts nobody came back for.
    let old: Vec<WindowId> = m
        .ghosts
        .iter()
        .filter(|(_, s)| now >= **s + GHOST_TTL_MS)
        .map(|(w, _)| *w)
        .collect();
    for w in old {
        m.ghosts.remove(&w);
        if let Some((_, n)) = find_in_trees(m, w) {
            m.tree.remove(n);
        }
        fx.push(Effect::PersistState);
    }
    // Second pass after display changes / wake: resend frames that drifted.
    if m.resync_at.is_some_and(|t| now >= t) {
        m.resync_at = None;
        let drifted: Vec<WindowId> = m
            .last_sent
            .iter()
            .filter(|(w, s)| {
                m.windows
                    .get(w)
                    .is_some_and(|i| !i.actual.approx_eq(&s.rect, 1.0))
            })
            .map(|(w, _)| *w)
            .collect();
        for w in drifted {
            m.last_sent.remove(&w);
        }
        m.layout_dirty = true;
    }
}

// ---------------------------------------------------------------- commands

fn command(m: &mut Model, cmd: Command, fx: &mut Vec<Effect>) -> Reply {
    if m.layout_dirty {
        fx.extend(reconcile::refresh_desired(m));
    }
    let r = match cmd {
        Command::Focus { dir } => cmd_focus(m, dir, fx),
        Command::Move { dir } => cmd_move(m, dir, fx),
        Command::Resize { axis, delta } => cmd_resize(m, axis, delta),
        Command::Workspace { name } => {
            let fd = m.focused_display();
            let ws = m.ensure_workspace(&name, fd);
            show_workspace(m, &ws, fx, true);
            Reply::ok()
        }
        Command::MoveToWorkspace { name } => cmd_move_to_workspace(m, &name, fx),
        Command::FocusDisplay { which } => {
            let Some(d) = cycle_display(m, which) else {
                return Reply::err("no display");
            };
            m.focus_display = Some(d.clone());
            match m.visible.get(&d).cloned() {
                Some(ws) => {
                    let t = m
                        .ws(&ws)
                        .unwrap()
                        .focused
                        .filter(|f| m.windows.contains_key(f))
                        .or_else(|| first_window(m, &ws));
                    match t {
                        Some(t) => focus_fx(m, t, fx),
                        None => m.focus_window = None,
                    }
                }
                None => m.focus_window = None,
            }
            fx.push(Effect::Snapshot);
            Reply::ok()
        }
        Command::MoveWorkspaceToDisplay { which } => cmd_move_ws_to_display(m, which, fx),
        Command::ToggleOrientation | Command::ToggleLayout => {
            let Some(ws) = m.focused_ws_id() else {
                return Reply::err("no workspace");
            };
            prune_ghosts(m, &ws);
            let root = m.ws(&ws).unwrap().root;
            let node = m
                .focused_window()
                .and_then(|w| m.window_node(w))
                .unwrap_or(root);
            if matches!(cmd, Command::ToggleOrientation) {
                m.tree.toggle_orientation(node);
            } else {
                m.tree.toggle_layout(node);
            }
            m.layout_dirty = true;
            fx.push(Effect::PersistState);
            Reply::ok()
        }
        Command::ToggleFloat => cmd_toggle_float(m, fx),
        Command::SwapWorkspaces => cmd_swap_workspaces(m, fx),
        Command::Reload => Reply::err("reload is handled by the runtime"),
        Command::ProfilePicker => {
            fx.push(Effect::UiRequest(UiRequest::ProfilePicker));
            Reply::ok()
        }
        Command::OpenEditor => {
            fx.push(Effect::UiRequest(UiRequest::OpenEditor));
            Reply::ok()
        }
        Command::Recover { all_apps } => cmd_recover(m, all_apps, fx),
        Command::Query { what } => query(m, what),
        Command::ProfileCapture { name } => {
            Reply::data(&capture(m, name.as_deref().unwrap_or("Captured")))
        }
        Command::ProfileApply {
            profile: Some(p), ..
        } => apply_profile(m, &p, fx),
        Command::ProfileApply { profile: None, .. } => Reply::err("profile not loaded"),
        Command::ProfileEdit { profile, edit } => match crate::profile::edit(&profile, &edit) {
            Ok(p) => Reply::data(&p),
            Err(e) => Reply::err(e),
        },
        Command::ProfileSave { .. }
        | Command::ProfileLoad { .. }
        | Command::ProfileDelete { .. }
        | Command::ProfileList => Reply::err("profile storage is handled by the runtime"),
        Command::Ping => Reply::data(&"pong"),
    };
    gc_workspaces(m);
    r
}

fn cycle_display(m: &Model, which: Cycle) -> Option<String> {
    let n = m.displays.len();
    if n == 0 {
        return None;
    }
    let cur = m
        .focused_display()
        .and_then(|d| m.displays.iter().position(|x| x.id == d))
        .unwrap_or(0);
    let i = match which {
        Cycle::Next => (cur + 1) % n,
        Cycle::Prev => (cur + n - 1) % n,
    };
    Some(m.displays[i].id.clone())
}

fn rects_in_ws(m: &Model, ws: &str) -> Vec<(WindowId, Rect)> {
    let Some(w) = m.ws(ws) else { return vec![] };
    let mut v: Vec<(WindowId, Rect)> = m
        .tree
        .windows(w.root)
        .into_iter()
        .filter_map(|x| {
            m.desired
                .get(&x)
                .map(|r| (x, *r))
                .filter(|_| m.windows.get(&x).is_some_and(|i| !i.minimized))
        })
        .collect();
    for f in &w.floating {
        if let Some(i) = m.windows.get(f).filter(|i| !i.minimized) {
            v.push((*f, i.float_frame.unwrap_or(i.actual)));
        }
    }
    v
}

fn neighbor_display(m: &Model, from: &str, dir: Direction) -> Option<String> {
    let cur = m.display(from)?;
    let others: Vec<(String, Rect)> = m
        .displays
        .iter()
        .filter(|d| d.id != from)
        .map(|d| (d.id.clone(), d.frame))
        .collect();
    neighbor_in_direction(&cur.frame, &others, dir)
}

fn cmd_focus(m: &mut Model, dir: Direction, fx: &mut Vec<Effect>) -> Reply {
    let Some(d) = m.focused_display() else {
        return Reply::err("no display");
    };
    let Some(ws) = m.visible.get(&d).cloned() else {
        return Reply::err("no workspace");
    };
    let rects = rects_in_ws(m, &ws);
    let cur = m.focused_window();
    let from = cur.and_then(|c| rects.iter().find(|r| r.0 == c).map(|r| r.1));
    if let (Some(c), Some(from)) = (cur, from) {
        let others: Vec<(WindowId, Rect)> = rects.iter().filter(|r| r.0 != c).cloned().collect();
        if let Some(t) = neighbor_in_direction(&from, &others, dir) {
            focus_fx(m, t, fx);
            return Reply::ok();
        }
    }
    // Cross to the neighbouring display, empty or not.
    let Some(nd) = neighbor_display(m, &d, dir) else {
        return Reply::ok();
    };
    let from = from.unwrap_or_else(|| m.display(&d).unwrap().frame);
    m.focus_display = Some(nd.clone());
    m.focus_window = None;
    if let Some(nws) = m.visible.get(&nd).cloned() {
        let cands = rects_in_ws(m, &nws);
        let t = neighbor_in_direction(&from, &cands, dir).or_else(|| cands.first().map(|c| c.0));
        if let Some(t) = t {
            focus_fx(m, t, fx);
        }
    }
    fx.push(Effect::Snapshot);
    Reply::ok()
}

fn cmd_move(m: &mut Model, dir: Direction, fx: &mut Vec<Effect>) -> Reply {
    let Some(wid) = m.focused_window() else {
        return Reply::err("no focused window");
    };
    let ws = m.windows[&wid].workspace.clone();
    let floating = m.windows[&wid].mode == WindowMode::Floating;
    if !floating {
        prune_ghosts(m, &ws);
        let node = m.window_node(wid).unwrap();
        if let MoveResult::Moved(_) = m.tree.move_leaf(node, dir) {
            m.layout_dirty = true;
            fx.push(Effect::PersistState);
            return Reply::ok();
        }
    }
    // At the edge: move to the neighbouring display's workspace.
    let d = m.ws(&ws).unwrap().display.clone();
    let Some(nd) = neighbor_display(m, &d, dir) else {
        return Reply::ok();
    };
    let Some(target) = m.visible.get(&nd).cloned() else {
        return Reply::ok();
    };
    detach_window(m, wid, false);
    let root = m.ws(&target).unwrap().root;
    if floating {
        let (src, dst) = (
            m.display(&d).unwrap().visible_frame,
            m.display(&nd).unwrap().visible_frame,
        );
        let w = m.windows.get_mut(&wid).unwrap();
        let f = w.float_frame.unwrap_or(w.actual);
        let moved = Rect::new(
            dst.x + (f.x - src.x) / src.w * dst.w,
            dst.y + (f.y - src.y) / src.h * dst.h,
            f.w,
            f.h,
        )
        .fit_into(&dst);
        w.float_frame = Some(moved);
        m.ws_mut(&target).unwrap().floating.push(wid);
    } else {
        // Enter from the side we came from.
        let rc = m.tree.container(root).unwrap();
        let at =
            if rc.orientation == dir.orientation() && dir.is_forward() && !rc.children.is_empty() {
                InsertAt::Path(vec![0])
            } else {
                InsertAt::End
            };
        m.tree.insert_leaf(root, Leaf::Window(wid), at);
    }
    m.windows.get_mut(&wid).unwrap().workspace = target.clone();
    m.ws_mut(&target).unwrap().focused = Some(wid);
    m.focus_display = Some(nd);
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    Reply::ok()
}

fn cmd_resize(m: &mut Model, axis: Orientation, delta: f64) -> Reply {
    let Some(wid) = m.focused_window() else {
        return Reply::err("no focused window");
    };
    let w = &m.windows[&wid];
    if w.mode == WindowMode::Floating {
        let Some(d) = m.ws(&w.workspace).and_then(|ws| m.display(&ws.display)) else {
            return Reply::ok();
        };
        let area = d.visible_frame;
        let f = w.float_frame.unwrap_or(w.actual);
        let nf = match axis {
            Orientation::Horizontal => Rect::new(f.x, f.y, (f.w + delta * area.w).max(100.0), f.h),
            Orientation::Vertical => Rect::new(f.x, f.y, f.w, (f.h + delta * area.h).max(60.0)),
        }
        .fit_into(&area);
        m.windows.get_mut(&wid).unwrap().float_frame = Some(nf);
    } else {
        let ws = w.workspace.clone();
        prune_ghosts(m, &ws);
        let node = m.window_node(wid).unwrap();
        m.tree.resize(node, axis, delta);
    }
    m.layout_dirty = true;
    Reply::ok()
}

fn cmd_move_to_workspace(m: &mut Model, name: &str, fx: &mut Vec<Effect>) -> Reply {
    let Some(wid) = m.focused_window() else {
        return Reply::err("no focused window");
    };
    let src = m.windows[&wid].workspace.clone();
    if src == name {
        return Reply::ok();
    }
    let fd = m.focused_display();
    let target = m.ensure_workspace(name, fd);
    let floating = m.windows[&wid].mode == WindowMode::Floating;
    detach_window(m, wid, false);
    if floating {
        m.ws_mut(&target).unwrap().floating.push(wid);
    } else {
        let at = default_insert(m, &target);
        let root = m.ws(&target).unwrap().root;
        m.tree.insert_leaf(root, Leaf::Window(wid), at);
    }
    m.windows.get_mut(&wid).unwrap().workspace = target.clone();
    m.ws_mut(&target).unwrap().focused = Some(wid);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    match first_window(m, &src) {
        Some(n) => focus_fx(m, n, fx),
        None => m.focus_window = None,
    }
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    Reply::ok()
}

fn cmd_move_ws_to_display(m: &mut Model, which: Cycle, fx: &mut Vec<Effect>) -> Reply {
    let Some(src) = m.focused_display() else {
        return Reply::err("no display");
    };
    let Some(target) = cycle_display(m, which) else {
        return Reply::err("no display");
    };
    if src == target {
        return Reply::ok();
    }
    let Some(ws) = m.visible.get(&src).cloned() else {
        return Reply::err("no workspace");
    };
    {
        let w = m.ws_mut(&ws).unwrap();
        w.display = target.clone();
        w.last_display = None;
    }
    m.visible.insert(target.clone(), ws.clone());
    m.remembered_visible.insert(target.clone(), ws.clone());
    // The source display needs something to show.
    let replacement = m
        .workspaces
        .iter()
        .find(|w| w.display == src && w.id != ws)
        .map(|w| w.id.clone())
        .unwrap_or_else(|| {
            let taken = m.workspaces.iter().map(|w| w.id.clone()).collect();
            let name = next_free_name(&taken);
            m.ensure_workspace(&name, Some(src.clone()))
        });
    m.visible.insert(src.clone(), replacement.clone());
    m.remembered_visible.insert(src, replacement);
    m.focus_display = Some(target);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    Reply::ok()
}

fn cmd_swap_workspaces(m: &mut Model, fx: &mut Vec<Effect>) -> Reply {
    let Some(d1) = m.focused_display() else {
        return Reply::err("no display");
    };
    let Some(d2) = cycle_display(m, Cycle::Next) else {
        return Reply::err("no display");
    };
    if d1 == d2 {
        return Reply::ok();
    }
    let (Some(w1), Some(w2)) = (m.visible.get(&d1).cloned(), m.visible.get(&d2).cloned()) else {
        return Reply::err("no workspace");
    };
    m.ws_mut(&w1).unwrap().display = d2.clone();
    m.ws_mut(&w2).unwrap().display = d1.clone();
    for (d, w) in [(&d1, &w2), (&d2, &w1)] {
        m.visible.insert(d.clone(), w.clone());
        m.remembered_visible.insert(d.clone(), w.clone());
    }
    // Focus follows the focused workspace to its new display.
    m.focus_display = Some(d2);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    Reply::ok()
}

fn cmd_toggle_float(m: &mut Model, fx: &mut Vec<Effect>) -> Reply {
    let Some(wid) = m.focused_window() else {
        return Reply::err("no focused window");
    };
    let w = m.windows[&wid].clone();
    let ws = w.workspace.clone();
    match w.mode {
        WindowMode::Tiled => {
            let area = m
                .ws(&ws)
                .and_then(|x| m.display(&x.display))
                .map(|d| d.visible_frame);
            let base = m.desired.get(&wid).copied().unwrap_or(w.actual);
            let frame = match area {
                Some(a) => {
                    let (fw, fh) = (
                        (a.w * 0.6).max(base.w.min(a.w)),
                        (a.h * 0.6).max(base.h.min(a.h)),
                    );
                    let fw = fw.min(base.w.max(a.w * 0.5));
                    let fh = fh.min(base.h.max(a.h * 0.5));
                    Rect::new(a.center().x - fw / 2.0, a.center().y - fh / 2.0, fw, fh).fit_into(&a)
                }
                None => base,
            };
            detach_window(m, wid, false);
            let wi = m.windows.get_mut(&wid).unwrap();
            wi.mode = WindowMode::Floating;
            wi.float_frame = Some(frame);
            m.ws_mut(&ws).unwrap().floating.push(wid);
            fx.push(Effect::Raise { pid: w.pid, wid });
        }
        WindowMode::Floating => {
            detach_window(m, wid, false);
            let at = default_insert(m, &ws);
            let root = m.ws(&ws).unwrap().root;
            m.tree.insert_leaf(root, Leaf::Window(wid), at);
            let wi = m.windows.get_mut(&wid).unwrap();
            wi.mode = WindowMode::Tiled;
            wi.stubborn = false;
        }
    }
    m.ws_mut(&ws).unwrap().focused = Some(wid);
    refresh_record(m, wid);
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    Reply::ok()
}

fn cmd_recover(m: &mut Model, all_apps: bool, fx: &mut Vec<Effect>) -> Reply {
    let mut moved = 0;
    let hidden: Vec<(WorkspaceId, WorkspaceId)> = m
        .workspaces
        .iter()
        .filter(|w| !m.is_visible(&w.id))
        .filter_map(|w| {
            m.visible
                .get(&w.display)
                .or_else(|| m.visible.values().next())
                .map(|v| (w.id.clone(), v.clone()))
        })
        .collect();
    for (from, to) in hidden {
        let (root, floats) = {
            let w = m.ws(&from).unwrap();
            (w.root, w.floating.clone())
        };
        let to_root = m.ws(&to).unwrap().root;
        for wid in m.tree.windows(root) {
            if m.windows.contains_key(&wid) {
                let n = m.tree.find_window(root, wid).unwrap();
                m.tree.remove(n);
                m.tree
                    .insert_leaf(to_root, Leaf::Window(wid), InsertAt::End);
                m.windows.get_mut(&wid).unwrap().workspace = to.clone();
                moved += 1;
            }
        }
        for f in floats {
            m.ws_mut(&from).unwrap().floating.retain(|x| *x != f);
            m.ws_mut(&to).unwrap().floating.push(f);
            if let Some(w) = m.windows.get_mut(&f) {
                w.workspace = to.clone();
            }
            moved += 1;
        }
    }
    for w in m.windows.values_mut() {
        w.stubborn = false;
    }
    m.last_sent.clear();
    m.layout_dirty = true;
    fx.push(Effect::RecoverSweep { all_apps });
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    info(fx, format!("Brought {moved} hidden window(s) back"));
    Reply::data(&serde_json::json!({ "moved": moved }))
}

// ---------------------------------------------------------------- queries

#[derive(Serialize)]
pub struct WindowSummary {
    pub id: WindowId,
    pub pid: Pid,
    pub bundle_id: String,
    pub title: String,
    pub workspace: WorkspaceId,
    pub display: String,
    pub mode: WindowMode,
    pub visible: bool,
    pub frame: Rect,
    pub minimized: bool,
    pub stubborn: bool,
}

#[derive(Serialize)]
pub struct WorkspaceSummary {
    pub name: WorkspaceId,
    pub display: String,
    pub visible: bool,
    pub windows: usize,
    pub focused: Option<WindowId>,
    pub configured: bool,
}

fn query(m: &Model, what: QueryKind) -> Reply {
    match what {
        QueryKind::Windows => {
            let v: Vec<WindowSummary> = m
                .windows
                .values()
                .map(|w| WindowSummary {
                    id: w.id,
                    pid: w.pid,
                    bundle_id: w.bundle_id.clone(),
                    title: w.title.clone(),
                    workspace: w.workspace.clone(),
                    display: m
                        .ws(&w.workspace)
                        .map(|x| x.display.clone())
                        .unwrap_or_default(),
                    mode: w.mode,
                    visible: m.is_visible(&w.workspace),
                    frame: w.actual,
                    minimized: w.minimized,
                    stubborn: w.stubborn,
                })
                .collect();
            Reply::data(&v)
        }
        QueryKind::Displays => Reply::data(&m.displays),
        QueryKind::Workspaces => {
            let v: Vec<WorkspaceSummary> = m
                .workspaces
                .iter()
                .map(|w| WorkspaceSummary {
                    name: w.id.clone(),
                    display: w.display.clone(),
                    visible: m.is_visible(&w.id),
                    windows: m
                        .tree
                        .windows(w.root)
                        .iter()
                        .filter(|x| m.windows.contains_key(x))
                        .count()
                        + w.floating.len(),
                    focused: w.focused,
                    configured: w.configured,
                })
                .collect();
            Reply::data(&v)
        }
        QueryKind::Tree => {
            let v: BTreeMap<String, SerNode> = m
                .workspaces
                .iter()
                .map(|w| (w.id.to_string(), m.tree.to_ser(w.root)))
                .collect();
            Reply::data(&v)
        }
        QueryKind::Snapshot => Reply::data(&crate::snapshot::build(m)),
        QueryKind::Config => Reply::data(&m.config),
    }
}

// ---------------------------------------------------------------- profiles

/// How a display is named in a captured profile.
fn display_ref_for(m: &Model, idx: usize) -> String {
    let d = &m.displays[idx];
    if let Some(a) = m.config.displays.iter().find(|a| {
        m.config
            .display_ref(&a.name)
            .resolve(&m.displays)
            .is_some_and(|x| x.id == d.id)
    }) {
        return a.name.clone();
    }
    let pos = match (m.displays.len(), d.position_rank) {
        (1, _) => return "main".into(),
        (2, 0) | (3, 0) => Position::Left,
        (2, _) | (3, 2) => Position::Right,
        (3, _) => Position::Center,
        _ => return format!("uuid:{}", d.id),
    };
    DisplayRef::Position(pos).to_short()
}

pub fn capture(m: &Model, name: &str) -> Profile {
    let mut displays = Vec::new();
    for (i, d) in m.displays.iter().enumerate() {
        let Some(ws) = m.visible_ws(&d.id) else {
            continue;
        };
        let ser = m.tree.to_ser(ws.root);
        let counts = {
            let mut c: BTreeMap<String, usize> = BTreeMap::new();
            for w in m.tree.windows(ws.root) {
                if let Some(i) = m.windows.get(&w) {
                    *c.entry(i.bundle_id.clone()).or_default() += 1;
                }
            }
            c
        };
        let tree = to_slots(m, &ser, &counts).unwrap_or_else(SerNode::empty);
        displays.push(ProfileDisplay {
            display: display_ref_for(m, i),
            workspace: ws.id.to_string(),
            tree,
        });
    }
    Profile {
        name: name.into(),
        version: crate::profile::PROFILE_VERSION,
        displays,
    }
}

fn to_slots(m: &Model, n: &SerNode, counts: &BTreeMap<String, usize>) -> Option<SerNode> {
    match n {
        SerNode::Window { id } => {
            let w = m
                .windows
                .get(id)
                .filter(|w| !w.minimized && w.mode == WindowMode::Tiled)?;
            let title = (counts.get(&w.bundle_id).copied().unwrap_or(0) > 1 && !w.title.is_empty())
                .then(|| format!("^{}$", regex::escape(&w.title)));
            Some(SerNode::Slot {
                bundle_id: w.bundle_id.clone(),
                title,
                launch_args: vec![],
            })
        }
        SerNode::Slot { .. } => Some(n.clone()),
        SerNode::Container {
            orientation,
            layout,
            weights,
            children,
        } => {
            let mut ws = Vec::new();
            let mut cs = Vec::new();
            for (i, c) in children.iter().enumerate() {
                if let Some(s) = to_slots(m, c, counts) {
                    cs.push(s);
                    ws.push(weights.get(i).copied().unwrap_or(1.0));
                }
            }
            match cs.len() {
                0 => None,
                1 if !matches!(cs[0], SerNode::Container { .. }) => Some(SerNode::Container {
                    orientation: *orientation,
                    layout: *layout,
                    weights: vec![1.0],
                    children: cs,
                }),
                1 => cs.pop(),
                _ => Some(SerNode::Container {
                    orientation: *orientation,
                    layout: *layout,
                    weights: ws,
                    children: cs,
                }),
            }
        }
    }
}

#[derive(Serialize)]
struct ApplySummary {
    matched: usize,
    launching: Vec<String>,
    missing_displays: Vec<String>,
}

pub fn apply_profile(m: &mut Model, p: &Profile, fx: &mut Vec<Effect>) -> Reply {
    let mut missing = Vec::new();
    let mut targets: Vec<(WorkspaceId, Vec<WindowId>)> = Vec::new();
    for pd in &p.displays {
        let Some(d) = m
            .config
            .display_ref(&pd.display)
            .resolve(&m.displays)
            .map(|d| d.id.clone())
        else {
            missing.push(pd.display.clone());
            continue;
        };
        if targets.iter().any(|(w, _)| w == pd.workspace.as_str()) {
            continue;
        }
        let ws = m.ensure_workspace(&pd.workspace, Some(d.clone()));
        // Whatever the display showed before keeps its own display.
        m.ws_mut(&ws).unwrap().display = d.clone();
        m.ws_mut(&ws).unwrap().last_display = None;
        m.visible.insert(d.clone(), ws.clone());
        m.remembered_visible.insert(d.clone(), ws.clone());
        let root = m.ws(&ws).unwrap().root;
        let orphans: Vec<WindowId> = m
            .tree
            .windows(root)
            .into_iter()
            .filter(|w| m.windows.contains_key(w))
            .collect();
        for w in m.tree.windows(root) {
            m.ghosts.remove(&w);
        }
        m.tree.fill_from_ser(root, &pd.tree);
        targets.push((ws, orphans));
    }
    // Match running windows to slots: same workspace and title first.
    let mut slots: Vec<(WorkspaceId, NodeId, crate::tree::AppSlot)> = Vec::new();
    for (ws, _) in &targets {
        let root = m.ws(ws).unwrap().root;
        for n in m.tree.leaves(root) {
            if let Some(Leaf::Slot(s)) = m.tree.leaf(n) {
                slots.push((ws.clone(), n, s.clone()));
            }
        }
    }
    let orphan_ws: BTreeMap<WindowId, WorkspaceId> = targets
        .iter()
        .flat_map(|(ws, o)| o.iter().map(move |w| (*w, ws.clone())))
        .collect();
    let mut taken: BTreeSet<WindowId> = BTreeSet::new();
    let mut assigned: Vec<Option<WindowId>> = vec![None; slots.len()];
    for pass in 0..4 {
        let same_ws = pass % 2 == 0;
        let use_title = pass < 2;
        for (si, (ws, _, slot)) in slots.iter().enumerate() {
            if assigned[si].is_some() {
                continue;
            }
            let cand = m
                .windows
                .values()
                .filter(|w| !taken.contains(&w.id) && w.bundle_id == slot.bundle_id && !w.minimized)
                .filter(|w| w.role == crate::rules::WindowRole::Standard)
                .filter(|w| !same_ws || orphan_ws.get(&w.id) == Some(ws) || w.workspace == *ws)
                .filter(|w| !use_title || title_matches(slot, &w.title))
                .min_by_key(|w| (w.ordinal, w.seq))
                .map(|w| w.id);
            if let Some(c) = cand {
                taken.insert(c);
                assigned[si] = Some(c);
            }
        }
    }
    let mut matched = 0;
    let mut launch: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (si, (ws, node, slot)) in slots.iter().enumerate() {
        match assigned[si] {
            Some(wid) => {
                if !orphan_ws.contains_key(&wid) {
                    detach_window(m, wid, false);
                } else if let Some(src) = m.ws_mut(&orphan_ws[&wid]) {
                    src.floating.retain(|f| *f != wid);
                }
                *m.tree.leaf_mut(*node).unwrap() = Leaf::Window(wid);
                let w = m.windows.get_mut(&wid).unwrap();
                w.mode = WindowMode::Tiled;
                w.workspace = ws.clone();
                matched += 1;
            }
            None => {
                launch
                    .entry(slot.bundle_id.clone())
                    .or_insert_with(|| slot.launch_args.clone());
            }
        }
    }
    // Leftover windows of those workspaces stay, appended.
    for (ws, orphans) in &targets {
        let root = m.ws(ws).unwrap().root;
        for w in orphans {
            if !taken.contains(w)
                && m.windows
                    .get(w)
                    .is_some_and(|x| x.mode == WindowMode::Tiled)
            {
                m.tree.insert_leaf(root, Leaf::Window(*w), InsertAt::End);
            }
        }
    }
    let mut launching = Vec::new();
    for (bundle, args) in launch {
        let request_id = m.next_request_id;
        m.next_request_id += 1;
        for (ws, _, slot) in slots.iter().filter(|(_, _, s)| s.bundle_id == bundle) {
            if m.ws(ws).is_some_and(|w| {
                m.tree
                    .find_leaf(w.root, |l| matches!(l, Leaf::Slot(s) if s == slot))
                    .is_some()
            }) {
                m.pending_slots.push(PendingSlot {
                    request_id,
                    bundle_id: bundle.clone(),
                    workspace: ws.clone(),
                    deadline_ms: m.now_ms + PENDING_SLOT_MS,
                });
            }
        }
        fx.push(Effect::UiRequest(UiRequest::LaunchApp {
            request_id,
            bundle_id: bundle.clone(),
            args,
        }));
        launching.push(bundle);
    }
    for d in &missing {
        warn(
            fx,
            format!("Profile `{}`: display `{d}` is not connected", p.name),
        );
    }
    m.last_sent.clear();
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    Reply::data(&ApplySummary {
        matched,
        launching,
        missing_displays: missing,
    })
}
