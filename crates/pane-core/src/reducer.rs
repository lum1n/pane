//! `reduce(model, event) -> effects`: every state change goes through here.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::assign::{assign, next_free_name, AssignInput, WsInput};
use crate::config::ScratchpadConfig;
use crate::display::{display_for_rect, park_corners, rank_displays, DisplayRef, Position};
use crate::drop::{self, DropTarget, Source};
use crate::effect::Effect;
use crate::event::{Event, WindowProbe};
use crate::geometry::{neighbor_in_direction, Direction, Orientation, Point, Rect};
use crate::gesture::GesturePhase;
use crate::identity::{match_windows, PlacementRecord, WindowFingerprint};
use crate::model::{
    AppInfo, Gesture, Model, PendingLaunch, PendingSlot, WindowInfo, WindowMode,
    FOLLOW_SUPPRESS_MS, GHOST_TTL_MS, MAX_RECORDS_PER_APP, PENDING_SLOT_MS, RESYNC_DELAY_MS,
    UNRESPONSIVE_AFTER_MS, WATCHDOG_MS,
};
use crate::profile::{title_matches, Profile, ProfileDisplay};
use crate::proto::{Command, LayoutPreset, QueryKind, Reply, UiRequest};
use crate::reconcile;
use crate::rules::{decide, Decision, RuleAction, TilePlacement, WindowTraits};
use crate::tree::{InsertAt, Leaf, MoveResult, NodeId, SerNode, Tree};
use crate::undo;
use crate::workspace::{split_id, valid_name, ws_id, Cycle, LayoutMode, MASTER_RATIO_RANGE};
use crate::{Pid, WindowId, WorkspaceId};

/// Quiet time after a user move before a tiled window snaps back.
pub const SNAP_BACK_MS: u64 = 300;
/// Focus follows mouse: how long the pointer must rest over a window
/// before it gets focus.
pub const HOVER_FOCUS_MS: u64 = 150;
/// A tiled window must change size by more than this (points) to count as
/// an edge drag rather than rounding noise.
const DRAG_THRESHOLD: f64 = 3.0;

pub fn reduce(m: &mut Model, ev: Event, now_ms: u64) -> Vec<Effect> {
    // Focus moved by the user through Pane (hotkeys, palette, CLI), not by
    // the pointer or the OS.
    let keyboard = matches!(&ev, Event::Command { cmd, .. }
        if !cmd.is_query() && !matches!(cmd, Command::Gesture { .. } | Command::Hover { .. }));
    if keyboard {
        // A pending hover must not undo what the user just chose.
        m.hover = None;
    }
    let before = (m.focus_window, m.focus_display.clone());
    let tiles_before = layout_tile_counts(m);
    let mut fx = reduce_event(m, ev, now_ms);
    keep_layouts(m, &tiles_before, &mut fx);
    if keyboard
        && m.config.settings.mouse_follows_focus
        && (m.focus_window, m.focus_display.clone()) != before
    {
        warp_to_focus(m, &mut fx);
    }
    // Remember when each window had focus (the palette lists by recency).
    if let Some(w) = m.focus_window.filter(|w| m.windows.contains_key(w)) {
        m.focus_seen.insert(w, now_ms);
    }
    if m.focus_seen.len() > m.windows.len() + 64 {
        let live = &m.windows;
        m.focus_seen.retain(|w, _| live.contains_key(w));
    }
    fx
}

/// Mouse follows focus: put the pointer on the focused window, or in the
/// middle of the focused display when it shows no window.
fn warp_to_focus(m: &mut Model, fx: &mut Vec<Effect>) {
    // Frames of a workspace just shown are only known after a layout pass.
    if m.layout_dirty {
        fx.extend(reconcile::refresh_desired(m));
    }
    let rect = match m.focus_window.and_then(|w| m.windows.get(&w)) {
        Some(w) => m
            .desired
            .get(&w.id)
            .copied()
            .or(w.float_frame)
            .unwrap_or(w.actual),
        None => match m.focus_display.as_deref().and_then(|d| m.display(d)) {
            Some(d) => d.visible_frame,
            None => return,
        },
    };
    fx.push(Effect::WarpPointer { rect });
}

/// Tile count and master width of every workspace that keeps a layout
/// preset, by root.
fn layout_tile_counts(m: &Model) -> Vec<(NodeId, usize, Option<f64>)> {
    m.workspaces
        .iter()
        .filter(|w| w.layout.is_some())
        .map(|w| {
            (
                w.root,
                m.tree.leaves(w.root).len(),
                m.tree.master_share(w.root),
            )
        })
        .collect()
}

/// Re-arrange workspaces with a layout preset whose number of tiles changed
/// (a window opened, closed, moved in or out). Rearranging by hand is left
/// alone until then.
fn keep_layouts(m: &mut Model, before: &[(NodeId, usize, Option<f64>)], fx: &mut Vec<Effect>) {
    let mut changed = false;
    for i in 0..m.workspaces.len() {
        let root = m.workspaces[i].root;
        if m.workspaces[i].layout.is_none() {
            continue;
        }
        let Some((_, n, share)) = before.iter().find(|(r, _, _)| *r == root) else {
            continue;
        };
        if m.tree.leaves(root).len() != *n {
            arrange(m, i, *share);
            changed = true;
        }
    }
    if changed {
        m.layout_dirty = true;
        if !fx.iter().any(|f| matches!(f, Effect::PersistState)) {
            fx.push(Effect::PersistState);
        }
    }
}

/// Re-shape workspace `i` into its layout preset. Master-stack keeps
/// `share`, the master's width before the change, if it had one (the user
/// may have resized it).
fn arrange(m: &mut Model, i: usize, share: Option<f64>) {
    let root = m.workspaces[i].root;
    match m.workspaces[i].layout {
        Some(LayoutMode::MasterStack { ratio }) => {
            let ratio = share
                .filter(|r| MASTER_RATIO_RANGE.contains(r))
                .unwrap_or(ratio);
            m.workspaces[i].layout = Some(LayoutMode::MasterStack { ratio });
            m.tree.arrange_master_stack(root, ratio);
        }
        Some(LayoutMode::Columns { n }) => m.tree.arrange_columns(root, n),
        None => {}
    }
}

fn reduce_event(m: &mut Model, ev: Event, now_ms: u64) -> Vec<Effect> {
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
            // Unbundled apps (`cargo run`, scripts) are known by their name.
            let Some(bundle_id) = bundle_id
                .filter(|b| !b.is_empty())
                .or_else(|| (!name.is_empty()).then(|| name.clone()))
            else {
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
        Event::FocusChanged {
            wid,
            frontmost,
            activated,
            ..
        } => {
            if let Some(wid) = wid {
                focus_changed(m, wid, frontmost, activated, &mut fx);
            }
        }
        Event::WindowMinimized { wid, minimized, .. } => {
            if let Some(w) = m.windows.get_mut(&wid) {
                tracing::info!(%wid, minimized, "window minimized state");
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
            cancel_gesture(m, &mut fx);
            rank_displays(&mut displays);
            m.displays = displays;
            reassign(m);
            for d in &m.displays {
                m.display_frames.insert(d.id.clone(), d.frame);
                m.display_names.insert(d.id.clone(), d.name.clone());
            }
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
            float_newly_floated_apps(m);
            prune_scratchpads(m);
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
                if let Some(p) = m
                    .pending_launches
                    .iter()
                    .position(|p| p.request_id == request_id)
                {
                    let p = m.pending_launches.remove(p);
                    fx.push(Effect::UiRequest(UiRequest::warn(format!(
                        "Could not open {}: {}",
                        p.bundle_id,
                        error.clone().unwrap_or_else(|| "unknown error".into())
                    ))));
                }
                let gone: Vec<PendingSlot> = m
                    .pending_slots
                    .iter()
                    .filter(|p| p.request_id == request_id)
                    .cloned()
                    .collect();
                // The slot stays in the layout (taking no space) for when
                // the app is opened some other way.
                m.pending_slots.retain(|p| p.request_id != request_id);
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
            cancel_gesture(m, &mut fx);
            m.shutting_down = true;
            m.layout_dirty = true;
        }
    }
    // The window being moved is gone.
    if m.gesture
        .as_ref()
        .is_some_and(|g| !m.windows.contains_key(&g.wid))
    {
        cancel_gesture(m, &mut fx);
    }
    fx
}

/// After a config change: tiled windows that the rules now explicitly
/// float (an app added to `float_apps`, a new float rule) float right away.
fn float_newly_floated_apps(m: &mut Model) {
    let targets: Vec<WindowId> = m
        .windows
        .values()
        .filter(|w| w.mode == WindowMode::Tiled)
        .filter(|w| {
            let regular = m.apps.get(&w.pid).is_none_or(|a| a.regular);
            let traits = WindowTraits {
                bundle_id: &w.bundle_id,
                title: &w.title,
                role: w.role,
                resizable: w.resizable,
                modal: false,
                has_close_button: true,
                width: w.actual.w,
                height: w.actual.h,
                regular_app: regular,
            };
            let d = decide(&m.rules, &traits, m.config.settings.auto_float_below);
            d.explicit && d.action == RuleAction::Float
        })
        .map(|w| w.id)
        .collect();
    for wid in targets {
        let w = m.windows[&wid].clone();
        let area = m
            .ws(&w.workspace)
            .and_then(|x| m.display(&x.display))
            .map(|d| d.visible_frame);
        let size = w.actual.size();
        let frame = match area {
            Some(a) => {
                let (fw, fh) = (size.w.min(a.w * 0.8), size.h.min(a.h * 0.8));
                Rect::new(a.center().x - fw / 2.0, a.center().y - fh / 2.0, fw, fh).fit_into(&a)
            }
            None => w.actual,
        };
        detach_window(m, wid, false);
        let wi = m.windows.get_mut(&wid).unwrap();
        wi.mode = WindowMode::Floating;
        wi.float_frame = Some(frame);
        wi.reason = "rule: float".into();
        if let Some(ws) = m.ws_mut(&w.workspace) {
            ws.floating.push(wid);
        }
        refresh_record(m, wid);
    }
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

fn reassign(m: &mut Model) {
    if m.displays.is_empty() {
        return;
    }
    let inputs: Vec<WsInput> = m
        .workspaces
        .iter()
        .map(|w| WsInput {
            id: w.id.clone(),
            home: w.home.clone(),
            name: w.name.clone(),
        })
        .collect();
    let a = assign(&AssignInput {
        displays: &m.displays,
        workspaces: &inputs,
        prev_visible: &m.visible,
        remembered_visible: &m.remembered_visible,
        last_frames: &m.display_frames,
    });
    for (old, new) in &a.rehome {
        rehome_display(m, old, new);
    }
    for ws in &mut m.workspaces {
        let id = ws.id.clone();
        if let Some(d) = a.ws_display.get(&id) {
            ws.display = d.clone();
        }
    }
    for (id, d) in &a.created {
        let (_, name) = split_id(id);
        let name = name.to_string();
        m.ensure_workspace(d, &name);
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

/// Every workspace of display `old` now belongs to `new` (a replaced
/// monitor): rewrite ids everywhere they are referenced.
fn rehome_display(m: &mut Model, old: &str, new: &str) {
    let ids: Vec<(WorkspaceId, String)> = m
        .workspaces
        .iter()
        .filter(|w| w.home == old)
        .map(|w| (w.id.clone(), w.name.clone()))
        .collect();
    for (id, name) in ids {
        rename_ws(m, &id, new, &name);
    }
    if let Some(v) = m.remembered_visible.remove(old) {
        m.remembered_visible.entry(new.to_string()).or_insert(v);
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
    m.displays.iter().any(|d| {
        park_corners(d, r.size())
            .iter()
            .any(|c| c.origin_near(r, 4.0))
    })
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
            *m.ignored_by_pid.entry(pid).or_default() += 1;
            if let Some(orig) = restore.get(&p.wid) {
                fx.push(set_frame_once(m, &app, p.wid, *orig));
            }
            continue;
        }
        let parked_now = looks_parked(m, &p.frame);
        let restore_frame = restore.get(&p.wid).copied();
        let sane_frame = |m: &Model, ws: &str| -> Rect {
            let ws_display = m.ws(ws).and_then(|w| m.display(&w.display));
            // Keep the window's frame when it is on screen and on the
            // workspace's own display; otherwise bring it there.
            let on_ws_display = display_for_rect(&m.displays, &p.frame)
                .zip(ws_display)
                .is_some_and(|(a, b)| a.id == b.id);
            if !parked_now && (on_ws_display || ws_display.is_none()) {
                return p.frame;
            }
            let area = ws_display.map(|d| d.visible_frame).unwrap_or(p.frame);
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
        let mut why: Option<&str> = None;
        if let Some((ws, node)) = find_in_trees(m, p.wid) {
            target = Some((ws, Plan::Replace(node)));
            why = Some("restored from the saved layout");
        } else if let Some((ws, frame)) = m.expected_floating.remove(&p.wid) {
            if m.ws(&ws).is_some() {
                let f = frame.unwrap_or_else(|| sane_frame(m, &ws));
                target = Some((ws, Plan::Float(f)));
                why = Some("restored from the saved layout (floating)");
            }
        }
        // A window of an app opened for a scratchpad becomes it; one opened
        // by `summon` just goes where new windows go (the focused display).
        let mut scratch_claim: Option<String> = None;
        if target.is_none() && p.role == crate::rules::WindowRole::Standard {
            if let Some(pos) = m.pending_launches.iter().position(|l| {
                l.bundle_id.eq_ignore_ascii_case(&app.bundle_id)
                    || l.bundle_id.eq_ignore_ascii_case(&app.name)
            }) {
                let l = m.pending_launches.remove(pos);
                let sp = l
                    .scratchpad
                    .as_deref()
                    .and_then(|n| m.config.scratchpad(n).cloned());
                let here = m
                    .focused_display()
                    .and_then(|d| Some((m.visible.get(&d).cloned()?, m.display(&d).cloned()?)));
                if let (Some(sp), Some((ws, disp))) = (sp, here) {
                    target = Some((ws, Plan::Float(scratch_frame(&sp, &disp))));
                    why = Some("scratchpad");
                    scratch_claim = Some(sp.name().to_string());
                }
            }
        }
        // 2. A profile slot waiting for this app (any time, not just right
        //    after applying: the profile stays until another is applied).
        let mut claimed: Option<crate::tree::AppSlot> = None;
        if target.is_none()
            && d.action == RuleAction::Tile
            && p.role == crate::rules::WindowRole::Standard
        {
            if let Some((ws, node, slot)) = find_slot_for(m, &app.bundle_id, &p.title) {
                if let Some(pos) = m
                    .pending_slots
                    .iter()
                    .position(|s| s.bundle_id == app.bundle_id)
                {
                    m.pending_slots.remove(pos);
                }
                target = Some((ws, Plan::Replace(node)));
                why = Some("profile slot");
                claimed = Some(slot);
            }
        }
        // 3. A placement record from an earlier run.
        let record = record_for.get(&i).copied();
        if target.is_none() {
            if let Some(ri) = record {
                let r = m.placements[ri].clone();
                why = Some(if r.floating {
                    "remembered placement (floating)"
                } else {
                    "remembered placement"
                });
                let (home, name) = split_id(&r.workspace);
                let (home, name) = (home.to_string(), name.to_string());
                // A record from an older layout may name a display Pane no
                // longer knows: then use the window's own display.
                let home = if m.knows_display(&home) {
                    home
                } else {
                    (if initial && !m.initial_scan_done {
                        display_for_rect(&m.displays, &p.frame).map(|x| x.id.clone())
                    } else {
                        None
                    })
                    .or_else(|| m.focused_display())
                    .unwrap_or_default()
                };
                let ws = m.ensure_workspace(&home, &name);
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
        let mut placement = TilePlacement::default();
        let (ws, plan) = match target {
            Some(t) => t,
            None => {
                // A rule's workspace is that workspace of the rule's display,
                // else of the display the window opens on.
                let rule_display = d
                    .display
                    .as_ref()
                    .and_then(|r| r.resolve(&m.displays))
                    .map(|x| x.id.clone());
                // Windows that already existed when Pane started stay on the
                // display they are on. Anything opened later goes to the
                // display you are working on: macOS tends to open new apps
                // on the main display, and following that would also move
                // the hotkeys' target display there.
                let natural = if initial && !m.initial_scan_done {
                    display_for_rect(&m.displays, &p.frame).map(|x| x.id.clone())
                } else {
                    m.focused_display()
                };
                let ws = match &d.workspace {
                    Some(name) => rule_display
                        .clone()
                        .or(natural.clone())
                        .map(|dd| m.ensure_workspace(&dd, name)),
                    None => rule_display
                        .as_ref()
                        .and_then(|dd| m.visible.get(dd).cloned()),
                }
                .or_else(|| natural.and_then(|dd| m.visible.get(&dd).cloned()))
                .or_else(|| m.focused_ws_id())
                .or_else(|| m.workspaces.first().map(|w| w.id.clone()));
                let Some(ws) = ws else { continue };
                let plan = if d.action == RuleAction::Float {
                    Plan::Float(sane_frame(m, &ws))
                } else {
                    placement = d.placement;
                    Plan::Tile(rule_insert(m, &ws, &placement), None)
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
            reason: why.map(str::to_string).unwrap_or_else(|| d.reason.clone()),
            slot: claimed,
        };
        let root = m.ws(&ws).unwrap().root;
        if !matches!(plan, Plan::Float(_)) {
            // A new tile must be seen: end the zoom of its workspace.
            if let Some(w) = m.ws_mut(&ws) {
                w.fullscreen = None;
            }
        }
        match plan {
            Plan::Tile(at, weight) => {
                let n = m.tree.insert_leaf(root, Leaf::Window(p.wid), at);
                if let Some(w) = weight {
                    m.tree.set_weight(n, w);
                }
                apply_placement(&mut m.tree, n, &placement);
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
        if let Some(name) = scratch_claim {
            m.windows.get_mut(&p.wid).unwrap().reason = format!("scratchpad {name}");
            m.scratch.insert(name, p.wid);
            fx.push(Effect::Focus { pid, wid: p.wid });
            set_focus(m, p.wid, fx);
        }
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

/// Where a new tile goes under a rule's `position` and `orientation`:
/// an edge of the workspace, a split of the focused tile, or (without
/// either) where new windows usually go.
fn rule_insert(m: &Model, ws: &str, p: &TilePlacement) -> InsertAt {
    let Some(w) = m.ws(ws) else {
        return InsertAt::End;
    };
    if let Some(dir) = p.position.and_then(|pos| pos.edge()) {
        return InsertAt::Split {
            target: w.root,
            dir,
        };
    }
    let Some(o) = p.orientation else {
        return default_insert(m, ws);
    };
    let target = match default_insert(m, ws) {
        InsertAt::After(n) => n,
        _ => w.root,
    };
    let dir = match o {
        Orientation::Horizontal => Direction::Right,
        Orientation::Vertical => Direction::Down,
    };
    InsertAt::Split { target, dir }
}

/// A rule's `layout` and `weight` for the tile `n` just inserted: the
/// layout of its split, and its share of that split.
fn apply_placement(tree: &mut Tree, n: NodeId, p: &TilePlacement) {
    let Some(parent) = tree.parent(n) else { return };
    if let Some(layout) = p.layout {
        if let Some(c) = tree.container_mut(parent) {
            c.layout = layout;
        }
    }
    if let Some(share) = p.weight {
        let others: f64 = tree
            .children(parent)
            .iter()
            .filter(|c| **c != n)
            .map(|c| tree.weight_of(*c))
            .sum();
        if others > 0.0 {
            tree.set_weight(n, others * share / (1.0 - share));
        }
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
/// The window no longer fills its display (if it did).
fn unzoom(m: &mut Model, wid: WindowId) {
    for w in &mut m.workspaces {
        if w.fullscreen == Some(wid) {
            w.fullscreen = None;
            m.layout_dirty = true;
        }
    }
}

fn detach_window(m: &mut Model, wid: WindowId, ghost: bool) -> Option<WorkspaceId> {
    unzoom(m, wid);
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

/// An unfilled profile slot for `bundle`: one whose title regex matches
/// first, preferring a slot whose launch is pending, then visible
/// workspaces, then any.
fn find_slot_for(
    m: &Model,
    bundle: &str,
    title: &str,
) -> Option<(WorkspaceId, NodeId, crate::tree::AppSlot)> {
    let mut cands: Vec<(u8, WorkspaceId, NodeId, crate::tree::AppSlot)> = Vec::new();
    for w in &m.workspaces {
        for n in m.tree.leaves(w.root) {
            if let Some(Leaf::Slot(s)) = m.tree.leaf(n) {
                if s.bundle_id != bundle {
                    continue;
                }
                let title_ok = title_matches(s, title);
                let pending = m
                    .pending_slots
                    .iter()
                    .any(|p| p.workspace == w.id && p.bundle_id == bundle);
                let rank =
                    (!title_ok as u8) * 4 + (!pending as u8) * 2 + (!m.is_visible(&w.id) as u8);
                cands.push((rank, w.id.clone(), n, s.clone()));
            }
        }
    }
    cands.sort_by_key(|c| c.0);
    cands.into_iter().next().map(|(_, w, n, s)| (w, n, s))
}

fn forget_window(m: &mut Model, wid: WindowId, ghost: bool) -> Option<WorkspaceId> {
    // A scratchpad whose window is gone opens a new one next time.
    m.scratch.retain(|_, w| *w != wid);
    m.scratch_hidden.remove(&wid);
    // A window filling a profile slot gives the slot back: the layout keeps
    // the profile's shape and the app's next window returns there.
    if let (Some(slot), Some(node)) = (
        m.windows.get(&wid).and_then(|w| w.slot.clone()),
        m.window_node(wid),
    ) {
        if m.windows[&wid].mode == WindowMode::Tiled {
            unzoom(m, wid);
            *m.tree.leaf_mut(node).unwrap() = Leaf::Slot(slot);
            if let Some(r) = m.placements.iter_mut().find(|r| r.live == Some(wid)) {
                r.live = None;
                r.last_wid = None;
            }
            let ws = m.windows[&wid].workspace.clone();
            if let Some(w) = m.ws_mut(&ws) {
                if w.focused == Some(wid) {
                    w.focused = None;
                }
            }
            m.windows.remove(&wid);
            m.last_sent.remove(&wid);
            m.desired.remove(&wid);
            m.parked.remove(&wid);
            if m.focus_window == Some(wid) {
                m.focus_window = None;
            }
            return Some(ws);
        }
    }
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
    if !visible && reconcile::mostly_hidden(m, &frame) {
        // A hidden window that macOS nudged (or the app moved) but that is
        // still out of sight: leave it, re-parking would only loop.
        if let Some(s) = m.last_sent.get_mut(&wid) {
            s.settled = true;
            s.next_retry_ms = None;
        }
        return;
    }
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
            // A tiled window whose size changed was resized by dragging an
            // edge: once the drag stops, its tile takes that size.
            if mode == WindowMode::Tiled && visible {
                let old = m.last_sent.get(&wid).map(|s| s.rect);
                let zoomed = m.ws(&ws).is_some_and(|w| w.fullscreen.is_some());
                if let Some(old) = old.filter(|_| !zoomed) {
                    if (frame.w - old.w).abs() > DRAG_THRESHOLD
                        || (frame.h - old.h).abs() > DRAG_THRESHOLD
                    {
                        m.drags.insert(wid, (frame, m.now_ms + SNAP_BACK_MS));
                    } else if let Some(d) = m.drags.get_mut(&wid) {
                        *d = (frame, m.now_ms + SNAP_BACK_MS);
                    }
                }
            }
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

fn focus_changed(
    m: &mut Model,
    wid: WindowId,
    frontmost: bool,
    activated: bool,
    fx: &mut Vec<Effect>,
) {
    let Some(w) = m.windows.get(&wid) else { return };
    let ws = w.workspace.clone();
    if m.scratch_hidden.contains(&wid) {
        if !frontmost {
            return;
        }
        m.os_focus = Some(wid);
        // Cmd-Tab or a Dock click on a hidden scratchpad brings it here;
        // late reports for the window we just hid are ignored.
        let suppressed = m.now_ms < m.suppress_follow_until;
        if let Some(name) = m.scratchpad_of(wid).map(str::to_string) {
            if activated && !suppressed {
                show_scratch(m, &name, wid, false, fx);
                fx.push(Effect::Snapshot);
                return;
            }
        }
        match m.focused_window() {
            Some(f) => focus_fx(m, f, fx),
            None => fx.push(Effect::FocusDesktop),
        }
        return;
    }
    if !frontmost {
        // A background app's own focus moved (often because we moved its
        // windows): remember it for that workspace, change nothing else.
        if let Some(w) = m.ws_mut(&ws) {
            w.focused = Some(wid);
        }
        return;
    }
    // macOS keeps reporting the window that had focus before we hid it;
    // only a report of a *different* window is the user moving focus.
    let repeat = m.os_focus == Some(wid);
    m.os_focus = Some(wid);
    if !m.is_visible(&ws) {
        // Only an app activation (Cmd-Tab, Dock, clicking into it) shows a
        // hidden workspace. Plain focus-change notices also arrive, late,
        // for windows we just hid; following those undid switches.
        let suppressed = m.now_ms < m.suppress_follow_until;
        if !activated || suppressed {
            tracing::info!(%wid, workspace = %ws, repeat, activated, suppressed, "ignored focus report from a hidden workspace");
            match m.focused_window() {
                Some(f) => focus_fx(m, f, fx),
                None => fx.push(Effect::FocusDesktop),
            }
            return;
        }
        // Cmd-Tab or a Dock click on a hidden window: show its workspace.
        tracing::info!(%wid, workspace = %ws, "focus moved to a hidden window: following");
        show_workspace(m, &ws, fx, false);
    }
    let d = m.ws(&ws).map(|w| w.display.clone());
    m.focus_window = Some(wid);
    m.focus_display = d;
    if let Some(w) = m.ws_mut(&ws) {
        w.focused = Some(wid);
        // Another window of a zoomed workspace got focus: show the tiles.
        if w.fullscreen.is_some_and(|f| f != wid) {
            w.fullscreen = None;
            m.layout_dirty = true;
        }
    }
    fx.push(Effect::Snapshot);
}

fn focus_fx(m: &mut Model, wid: WindowId, fx: &mut Vec<Effect>) {
    // Focusing a minimized window would restore it: the user minimized it.
    let Some(w) = m.windows.get(&wid).filter(|w| !w.minimized) else {
        return;
    };
    fx.push(Effect::Focus { pid: w.pid, wid });
    set_focus(m, wid, fx);
}

/// Record `wid` as focused (the effect that focuses it is the caller's).
fn set_focus(m: &mut Model, wid: WindowId, fx: &mut Vec<Effect>) {
    let Some(ws) = m.windows.get(&wid).map(|w| w.workspace.clone()) else {
        return;
    };
    m.os_focus = Some(wid);
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
        .or_else(|| {
            w.floating.iter().rev().copied().find(|f| {
                !m.scratch_hidden.contains(f) && m.windows.get(f).is_some_and(|i| !i.minimized)
            })
        })
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
    let previous = m.visible.insert(d.clone(), ws.into());
    if let Some(prev) = previous.filter(|p| p != ws) {
        tracing::info!(display = %d, from = %prev, to = %ws, "workspace switch");
    }
    m.remembered_visible.insert(d.clone(), ws.into());
    m.focus_display = Some(d);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    m.layout_dirty = true;
    let mut floats = m.ws(ws).unwrap().floating.clone();
    if let Some(z) = m.ws(ws).unwrap().fullscreen {
        floats.insert(0, z);
    }
    for f in floats {
        // Raising a minimized window would restore it.
        if let Some(i) = m
            .windows
            .get(&f)
            .filter(|i| !i.minimized && !m.scratch_hidden.contains(&f))
        {
            fx.push(Effect::Raise { pid: i.pid, wid: f });
        }
    }
    if focus {
        let w = m.ws(ws).unwrap();
        let target = w
            .focused
            .filter(|f| {
                m.windows
                    .get(f)
                    .is_some_and(|i| i.workspace == ws && !i.minimized && m.window_shown(i))
            })
            .or_else(|| first_window(m, ws));
        match target {
            Some(t) => focus_fx(m, t, fx),
            None => {
                // Keyboard focus must not stay on a window we just hid.
                m.focus_window = None;
                fx.push(Effect::FocusDesktop);
            }
        }
    }
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
}

/// Empty workspaces that are not shown are dropped (they come back on demand).
fn gc_workspaces(m: &mut Model) {
    let dead: Vec<WorkspaceId> = m
        .workspaces
        .iter()
        .filter(|w| !m.visible.values().any(|v| *v == w.id))
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

/// Turn a finished edge drag into tile weights. `old` is the tile the
/// window had; each axis whose edge moved resizes against the neighbour
/// across that edge. A window dragged by its title bar keeps its size and
/// never gets here.
fn commit_drag(m: &mut Model, wid: WindowId, frame: Rect) {
    let Some(old) = m.last_sent.get(&wid).map(|s| s.rect) else {
        return;
    };
    let Some(node) = m.window_node(wid) else {
        return;
    };
    let Some(root) = m.ws(&m.windows[&wid].workspace).map(|w| w.root) else {
        return;
    };
    let present: std::collections::HashSet<NodeId> = m
        .tree
        .leaves(root)
        .into_iter()
        .filter(|n| m.tree.leaf(*n).is_some_and(|l| m.leaf_present(l)))
        .collect();
    let mut changed = false;
    for axis in [Orientation::Horizontal, Orientation::Vertical] {
        let (os, ol) = old.span(axis);
        let (ns, nl) = frame.span(axis);
        let (d_start, d_end) = (ns - os, (ns + nl) - (os + ol));
        let t = DRAG_THRESHOLD;
        // Exactly one edge moved; both moving means a move or a stretch.
        let (delta, toward_next) = if d_start.abs() > t && d_end.abs() <= t {
            (-d_start, false)
        } else if d_end.abs() > t && d_start.abs() <= t {
            (d_end, true)
        } else {
            continue;
        };
        changed |= m
            .tree
            .resize_edge(node, axis, delta, ol, toward_next, &present);
    }
    if changed {
        m.layout_dirty = true;
    }
}

fn tick(m: &mut Model, fx: &mut Vec<Effect>) {
    let now = m.now_ms;
    // Finished edge drags become the new layout.
    let due: Vec<(WindowId, Rect)> = m
        .drags
        .iter()
        .filter(|(_, d)| now >= d.1)
        .map(|(w, d)| (*w, d.0))
        .collect();
    for (wid, frame) in due {
        m.drags.remove(&wid);
        let before = undo::capture(m);
        commit_drag(m, wid, frame);
        undo::record(m, before);
        // Whether or not the layout changed, the window goes to its tile.
        if let Some(s) = m.last_sent.get_mut(&wid) {
            s.settled = false;
            s.next_retry_ms = Some(now);
        }
        fx.push(Effect::PersistState);
    }
    // Watchdog: a window of a shown workspace must be on screen. If one is
    // found hidden (an app undid our move, a lost update), send it again.
    if now >= m.next_watchdog_ms {
        m.next_watchdog_ms = now + WATCHDOG_MS;
        let stray: Vec<WindowId> = m
            .windows
            .values()
            .filter(|w| !w.minimized && m.window_shown(w))
            .filter(|w| reconcile::mostly_hidden(m, &w.actual))
            .filter(|w| m.last_sent.get(&w.id).is_none_or(|s| s.settled))
            .map(|w| w.id)
            .collect();
        for w in stray {
            tracing::warn!(wid = %w, title = %m.windows[&w].title, "watchdog: window of a shown workspace is off screen; resending");
            m.last_sent.remove(&w);
            m.layout_dirty = true;
        }
    }
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
        // Stop reserving space; the slot keeps waiting for the app.
        m.pending_slots.retain(|p| p.deadline_ms > now);
        m.layout_dirty = true;
    }
    m.pending_launches.retain(|p| p.deadline_ms > now);
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
    // Focus follows mouse: the pointer has rested over a window.
    if let Some((wid, _)) = m.hover.filter(|h| now >= h.1) {
        m.hover = None;
        let shown = m
            .windows
            .get(&wid)
            .is_some_and(|w| !w.minimized && m.window_shown(w));
        if m.config.settings.focus_follows_mouse
            && m.gesture.is_none()
            && shown
            && m.focus_window != Some(wid)
        {
            focus_fx(m, wid, fx);
        }
    }
}

// ---------------------------------------------------------------- commands

fn command(m: &mut Model, cmd: Command, fx: &mut Vec<Effect>) -> Reply {
    // Rearranging a workspace shows its tiles again.
    if matches!(
        cmd,
        Command::Focus { .. }
            | Command::Move { .. }
            | Command::Resize { .. }
            | Command::ToggleOrientation
            | Command::ToggleLayout
            | Command::Balance
            | Command::Layout { .. }
    ) {
        if let Some(ws) = m.focused_ws_id() {
            if let Some(w) = m.ws_mut(&ws) {
                if w.fullscreen.take().is_some() {
                    m.layout_dirty = true;
                }
            }
        }
    }
    if m.layout_dirty {
        fx.extend(reconcile::refresh_desired(m));
    }
    // Tree-changing commands can be undone: keep the layout from before.
    let undoable = matches!(
        cmd,
        Command::Move { .. }
            | Command::Resize { .. }
            | Command::MoveToWorkspace { .. }
            | Command::ToggleOrientation
            | Command::ToggleLayout
            | Command::ToggleFloat
            | Command::ProfileApply {
                profile: Some(_),
                ..
            }
            | Command::Gesture {
                phase: GesturePhase::Drop { .. }
            }
    );
    let before = undoable.then(|| undo::capture(m));
    let r = match cmd {
        Command::Focus { dir } => cmd_focus(m, dir, fx),
        Command::Move { dir } => cmd_move(m, dir, fx),
        Command::Resize { axis, delta } => cmd_resize(m, axis, delta),
        Command::Workspace { name } => match target_ws(m, &name) {
            Ok(ws) => {
                show_workspace(m, &ws, fx, true);
                Reply::ok()
            }
            Err(e) => Reply::err(e),
        },
        Command::CycleWorkspace { which } => cmd_cycle_workspace(m, which, fx),
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
        Command::Balance => cmd_balance(m, fx),
        Command::Layout { preset } => cmd_layout(m, preset, fx),
        Command::ToggleFloat => cmd_toggle_float(m, fx),
        Command::ToggleFullscreen => cmd_toggle_fullscreen(m, fx),
        Command::SwapWorkspaces => cmd_swap_workspaces(m, fx),
        Command::Undo => cmd_undo(m, fx),
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
        Command::SetSetting { .. }
        | Command::SetScratchpads { .. }
        | Command::Rebind { .. }
        | Command::ResetShortcuts
        | Command::SuspendHotkeys { .. } => Reply::err("config editing is handled by the runtime"),
        Command::ProfileSave { .. }
        | Command::ProfileLoad { .. }
        | Command::ProfileDelete { .. }
        | Command::ProfileList
        | Command::ProfileWhen { .. }
        | Command::ProfileDiff { .. }
        | Command::ProfileUpdate { .. } => Reply::err("profile storage is handled by the runtime"),
        Command::Ping => Reply::data(&"pong"),
        Command::Doctor => Reply::err("doctor is handled by the runtime"),
        Command::Subscribe { .. } => Reply::err("subscribe works only over the control socket"),
        Command::Gesture { phase } => cmd_gesture(m, phase, fx),
        Command::Hover { at } => cmd_hover(m, at),
        Command::FocusWindow { wid } => cmd_focus_window(m, WindowId(wid), fx),
        Command::Scratchpad { name } => cmd_scratchpad(m, &name, fx),
        Command::Summon { app } => cmd_summon(m, &app, fx),
        Command::Palette => {
            fx.push(Effect::UiRequest(UiRequest::Palette));
            Reply::ok()
        }
        Command::Search { query, profiles } => {
            Reply::data(&crate::palette::search(m, &query, &profiles))
        }
    };
    if let Some(before) = before {
        undo::record(m, before);
    }
    gc_workspaces(m);
    r
}

/// Put back the newest layout in the undo ring. One that changes nothing
/// now (its windows all closed) is skipped.
fn cmd_undo(m: &mut Model, fx: &mut Vec<Effect>) -> Reply {
    while let Some(prev) = m.undo.pop_back() {
        let now = undo::capture(m);
        let focus = undo::restore(m, &prev);
        if undo::capture(m) == now {
            continue;
        }
        if let Some(w) = focus {
            focus_fx(m, w, fx);
        } else if m.focused_window().is_none() {
            if let Some(w) = m.focused_ws_id().and_then(|ws| first_window(m, &ws)) {
                focus_fx(m, w, fx);
            }
        }
        m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
        fx.push(Effect::PersistState);
        fx.push(Effect::Snapshot);
        return Reply::ok();
    }
    Reply::err("nothing to undo")
}

/// Equal sizes for the tiles of the focused window's container (the
/// workspace's top level when no tile has focus).
fn cmd_balance(m: &mut Model, fx: &mut Vec<Effect>) -> Reply {
    let Some(ws) = m.focused_ws_id() else {
        return Reply::err("no workspace");
    };
    prune_ghosts(m, &ws);
    let root = m.ws(&ws).unwrap().root;
    let node = m
        .focused_window()
        .and_then(|w| m.tree.find_window(root, w))
        .unwrap_or(root);
    if m.tree.balance(node) {
        m.layout_dirty = true;
        fx.push(Effect::PersistState);
    }
    Reply::ok()
}

/// Arrange the focused workspace in a preset and remember it, so it
/// re-shapes as tiles come and go; `manual` forgets it.
fn cmd_layout(m: &mut Model, preset: LayoutPreset, fx: &mut Vec<Effect>) -> Reply {
    let Some(ws) = m.focused_ws_id() else {
        return Reply::err("no workspace");
    };
    let mode = match preset {
        LayoutPreset::MasterStack { ratio } => {
            let ratio = ratio.unwrap_or(m.config.settings.master_ratio);
            if !MASTER_RATIO_RANGE.contains(&ratio) {
                return Reply::err("master ratio must be between 0.1 and 0.9");
            }
            Some(LayoutMode::MasterStack { ratio })
        }
        LayoutPreset::Columns { n } => {
            if !(1..=crate::workspace::MAX_COLUMNS).contains(&n) {
                return Reply::err(format!(
                    "columns must be 1 to {}",
                    crate::workspace::MAX_COLUMNS
                ));
            }
            Some(LayoutMode::Columns { n })
        }
        LayoutPreset::Manual => None,
    };
    prune_ghosts(m, &ws);
    let i = m.workspaces.iter().position(|w| w.id == ws).unwrap();
    m.workspaces[i].layout = mode;
    if mode.is_some() {
        arrange(m, i, None);
        m.layout_dirty = true;
    }
    fx.push(Effect::PersistState);
    Reply::ok()
}

/// Jump to a window from the palette: show its workspace where it is,
/// end a zoom that hides it, and restore it if minimized (the user asked
/// for this very window).
fn cmd_focus_window(m: &mut Model, wid: WindowId, fx: &mut Vec<Effect>) -> Reply {
    let Some(w) = m.windows.get(&wid) else {
        return Reply::err(format!("no window {wid}"));
    };
    let (pid, ws, minimized) = (w.pid, w.workspace.clone(), w.minimized);
    if m.ws(&ws).is_none() {
        return Reply::err(format!("window {wid} has no workspace"));
    }
    if m.scratch_hidden.contains(&wid) {
        if let Some(name) = m.scratchpad_of(wid).map(str::to_string) {
            show_scratch(m, &name, wid, false, fx);
            fx.push(Effect::Snapshot);
            return Reply::ok();
        }
    }
    if !m.is_visible(&ws) {
        show_workspace(m, &ws, fx, false);
    }
    if let Some(x) = m.ws_mut(&ws) {
        if x.fullscreen.is_some_and(|f| f != wid) {
            x.fullscreen = None;
            m.layout_dirty = true;
        }
    }
    if minimized {
        fx.push(Effect::Restore { pid, wid });
    } else {
        fx.push(Effect::Focus { pid, wid });
    }
    set_focus(m, wid, fx);
    Reply::ok()
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
    let target = match target_ws(m, name) {
        Ok(t) => t,
        Err(e) => return Reply::err(e),
    };
    if src == target {
        return Reply::ok();
    }
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

/// `2` or `left:2` -> workspace id (created if needed).
fn target_ws(m: &mut Model, target: &str) -> Result<WorkspaceId, String> {
    let (home, name) = m
        .resolve_target(target)
        .ok_or_else(|| format!("no display for `{target}`"))?;
    if !valid_name(&name) {
        return Err(format!("`{name}` is not a valid workspace name"));
    }
    Ok(m.ensure_workspace(&home, &name))
}

fn cmd_cycle_workspace(m: &mut Model, which: Cycle, fx: &mut Vec<Effect>) -> Reply {
    let Some(d) = m.focused_display() else {
        return Reply::err("no display");
    };
    let current = m.visible.get(&d).cloned();
    let list: Vec<WorkspaceId> = m
        .workspaces_on(&d)
        .into_iter()
        .filter(|w| {
            Some(&w.id) == current.as_ref()
                || w.floating.iter().any(|f| !m.scratch_hidden.contains(f))
                || m.tree
                    .windows(w.root)
                    .iter()
                    .any(|x| m.windows.contains_key(x))
        })
        .map(|w| w.id.clone())
        .collect();
    if list.len() < 2 {
        return Reply::ok();
    }
    let i = current
        .and_then(|c| list.iter().position(|w| *w == c))
        .unwrap_or(0);
    let n = list.len();
    let next = match which {
        Cycle::Next => (i + 1) % n,
        Cycle::Prev => (i + n - 1) % n,
    };
    let ws = list[next].clone();
    show_workspace(m, &ws, fx, true);
    Reply::ok()
}

/// Give workspace `from` a new home and name, updating every reference.
fn rename_ws(m: &mut Model, from: &str, home: &str, name: &str) -> WorkspaceId {
    let to = ws_id(home, name);
    if let Some(w) = m.workspaces.iter_mut().find(|w| w.id == from) {
        w.id = to.clone();
        w.home = home.to_string();
        w.name = name.to_string();
    }
    for w in m.windows.values_mut().filter(|w| w.workspace == from) {
        w.workspace = to.clone();
    }
    for r in m.placements.iter_mut().filter(|r| r.workspace == from) {
        r.workspace = to.clone();
    }
    for p in m.pending_slots.iter_mut().filter(|p| p.workspace == from) {
        p.workspace = to.clone();
    }
    for (ws, _) in m.expected_floating.values_mut() {
        if *ws == from {
            *ws = to.clone();
        }
    }
    for v in m
        .visible
        .values_mut()
        .chain(m.remembered_visible.values_mut())
    {
        if *v == from {
            *v = to.clone();
        }
    }
    to
}

/// Move the focused display's workspace to the next/previous display. It
/// keeps its name there if free (else takes the next free number); the
/// source display shows another of its workspaces.
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
    let name = m.ws(&ws).unwrap().name.clone();
    let taken: BTreeSet<WorkspaceId> = m
        .workspaces
        .iter()
        .filter(|w| w.home == target)
        .map(|w| WorkspaceId::from(w.name.as_str()))
        .collect();
    let new_name = if taken.contains(name.as_str()) {
        next_free_name(&taken).to_string()
    } else {
        name
    };
    let moved = rename_ws(m, &ws, &target, &new_name);
    m.ws_mut(&moved).unwrap().display = target.clone();
    m.visible.insert(target.clone(), moved.clone());
    m.remembered_visible.insert(target.clone(), moved);
    let replacement = m
        .workspaces_on(&src)
        .into_iter()
        .find(|w| !w.is_guest())
        .map(|w| w.id.clone())
        .unwrap_or_else(|| {
            let taken: BTreeSet<WorkspaceId> = m
                .workspaces
                .iter()
                .filter(|w| w.home == src)
                .map(|w| WorkspaceId::from(w.name.as_str()))
                .collect();
            let n = next_free_name(&taken).to_string();
            m.ensure_workspace(&src, &n)
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

/// Swap what the focused and the next display show. Workspace names stay
/// with their displays; the windows trade places.
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
    let i1 = m.workspaces.iter().position(|w| w.id == w1).unwrap();
    let i2 = m.workspaces.iter().position(|w| w.id == w2).unwrap();
    let (r1, f1, fo1) = (
        m.workspaces[i1].root,
        m.workspaces[i1].floating.clone(),
        m.workspaces[i1].focused,
    );
    let (r2, f2, fo2) = (
        m.workspaces[i2].root,
        m.workspaces[i2].floating.clone(),
        m.workspaces[i2].focused,
    );
    m.workspaces[i1].root = r2;
    m.workspaces[i1].floating = f2;
    m.workspaces[i1].focused = fo2;
    m.workspaces[i2].root = r1;
    m.workspaces[i2].floating = f1;
    m.workspaces[i2].focused = fo1;
    for w in m.windows.values_mut() {
        if w.workspace == w1 {
            w.workspace = w2.clone();
        } else if w.workspace == w2 {
            w.workspace = w1.clone();
        }
    }
    // Focus follows the focused windows to their new display.
    m.focus_display = Some(d2);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    Reply::ok()
}

fn cmd_toggle_fullscreen(m: &mut Model, fx: &mut Vec<Effect>) -> Reply {
    let Some(wid) = m.focused_window() else {
        return Reply::err("no focused window");
    };
    let w = m.windows[&wid].clone();
    if w.mode != WindowMode::Tiled {
        return Reply::err("only tiled windows can be zoomed to fullscreen");
    }
    let Some(ws) = m.ws_mut(&w.workspace) else {
        return Reply::err("no workspace");
    };
    if ws.fullscreen == Some(wid) {
        ws.fullscreen = None;
    } else {
        ws.fullscreen = Some(wid);
        fx.push(Effect::Raise { pid: w.pid, wid });
    }
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
            wi.reason = "floated with toggle-float".into();
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
            wi.reason = "tiled with toggle-float".into();
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
    pub reason: String,
}

#[derive(Serialize)]
pub struct AppSummary {
    pub pid: Pid,
    /// Bundle id, or the executable name for unbundled apps.
    pub id: String,
    pub name: String,
    pub regular: bool,
    pub unresponsive: bool,
    pub windows: usize,
    pub ignored_windows: usize,
}

#[derive(Serialize)]
pub struct WorkspaceSummary {
    pub id: WorkspaceId,
    pub name: String,
    /// Display it belongs to.
    pub home: String,
    /// Display it is on now (differs while `home` is unplugged).
    pub display: String,
    pub visible: bool,
    pub windows: usize,
    pub focused: Option<WindowId>,
    /// Layout preset the workspace keeps, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<LayoutMode>,
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
                    visible: m.window_shown(w),
                    frame: w.actual,
                    minimized: w.minimized,
                    stubborn: w.stubborn,
                    reason: w.reason.clone(),
                })
                .collect();
            Reply::data(&v)
        }
        QueryKind::Displays => Reply::data(&m.displays),
        QueryKind::Apps => {
            let v: Vec<AppSummary> = m
                .apps
                .values()
                .map(|a| AppSummary {
                    pid: a.pid,
                    id: a.bundle_id.clone(),
                    name: a.name.clone(),
                    regular: a.regular,
                    unresponsive: a.unresponsive,
                    windows: m.windows.values().filter(|w| w.pid == a.pid).count(),
                    ignored_windows: m.ignored_by_pid.get(&a.pid).copied().unwrap_or(0),
                })
                .collect();
            Reply::data(&v)
        }
        QueryKind::Workspaces => {
            let v: Vec<WorkspaceSummary> = m
                .workspaces
                .iter()
                .map(|w| WorkspaceSummary {
                    id: w.id.clone(),
                    name: w.name.clone(),
                    home: w.home.clone(),
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
                    layout: w.layout,
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
        QueryKind::Bindings => match m.config.binding_table() {
            Ok(t) => Reply::data(&t),
            Err(e) => Reply::err(e.render()),
        },
        QueryKind::Keys => Reply::data(
            &crate::config::KEYS
                .iter()
                .map(|(n, k)| (n.to_string(), *k))
                .collect::<BTreeMap<String, u16>>(),
        ),
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

/// Every workspace with tiled windows on each connected display (plus the
/// visible one), marking which one each display shows.
pub fn capture(m: &Model, name: &str) -> Profile {
    let mut displays = Vec::new();
    for (i, d) in m.displays.iter().enumerate() {
        let shown = m.visible.get(&d.id);
        for ws in m.workspaces_on(&d.id).into_iter().filter(|w| !w.is_guest()) {
            let ser = m.tree.to_ser(ws.root);
            let mut counts: BTreeMap<String, usize> = BTreeMap::new();
            for w in m.tree.windows(ws.root) {
                if let Some(i) = m.windows.get(&w) {
                    *counts.entry(i.bundle_id.clone()).or_default() += 1;
                }
            }
            let tree = to_slots(m, &ser, &counts);
            let visible = shown == Some(&ws.id);
            if tree.is_none() && !visible {
                continue;
            }
            displays.push(ProfileDisplay {
                display: display_ref_for(m, i),
                workspace: ws.name.clone(),
                tree: tree.unwrap_or_else(SerNode::empty),
                visible,
            });
        }
    }
    Profile {
        name: name.into(),
        version: crate::profile::PROFILE_VERSION,
        when: None,
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
    // Per display: has an entry marked `visible` been applied yet?
    let mut shown: BTreeMap<String, bool> = BTreeMap::new();
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
        if !valid_name(&pd.workspace) {
            missing.push(format!(
                "{} (bad workspace name `{}`)",
                pd.display, pd.workspace
            ));
            continue;
        }
        let ws = m.ensure_workspace(&d, pd.workspace.trim());
        if targets.iter().any(|(w, _)| *w == ws) {
            continue;
        }
        // The entry marked visible (else the first one) is what the display shows.
        let first_for_display = !shown.contains_key(&d);
        if pd.visible || first_for_display {
            if pd.visible || !shown.get(&d).copied().unwrap_or(false) {
                m.visible.insert(d.clone(), ws.clone());
                m.remembered_visible.insert(d.clone(), ws.clone());
            }
            let prev = shown.get(&d).copied().unwrap_or(false);
            shown.insert(d.clone(), prev || pd.visible);
        }
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
        // The profile's own shape replaces any layout preset.
        m.ws_mut(&ws).unwrap().layout = None;
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
                w.slot = Some(slot.clone());
                w.reason = "profile slot".into();
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
                // Not part of the new profile.
                if let Some(x) = m.windows.get_mut(w) {
                    x.slot = None;
                }
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
    // The profile stays active until the user applies another one.
    m.active_profile = Some(p.name.clone());
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

// ---------------------------------------------------------------- gestures

fn cancel_gesture(m: &mut Model, fx: &mut Vec<Effect>) {
    if m.gesture.take().is_some() {
        fx.push(Effect::UiRequest(UiRequest::GestureOverlayHide));
    }
}

/// The window drawn at `at` on a visible workspace: floats first (the
/// focused one first), then a zoomed tile, then tiles.
fn window_at(m: &Model, at: Point) -> Option<WindowId> {
    let shown = |wid: &WindowId| {
        m.windows
            .get(wid)
            .filter(|w| !w.minimized && !m.scratch_hidden.contains(wid))
    };
    for d in &m.displays {
        let Some(ws) = m.visible_ws(&d.id) else {
            continue;
        };
        let mut floats: Vec<WindowId> = ws.floating.iter().rev().copied().collect();
        if let Some(i) = floats.iter().position(|f| Some(*f) == m.focus_window) {
            let f = floats.remove(i);
            floats.insert(0, f);
        }
        for f in floats {
            if let Some(w) = shown(&f) {
                if w.float_frame.unwrap_or(w.actual).contains(at) {
                    return Some(f);
                }
            }
        }
        if let Some(z) = ws.fullscreen.filter(|z| shown(z).is_some()) {
            if d.visible_frame.contains(at) {
                return Some(z);
            }
        }
        for wid in m.tree.windows(ws.root) {
            if shown(&wid).is_some_and(|w| w.mode == WindowMode::Tiled)
                && m.desired.get(&wid).is_some_and(|r| r.contains(at))
            {
                return Some(wid);
            }
        }
    }
    None
}

/// The pointer moved: focus the window under it once it rests there for
/// `HOVER_FOCUS_MS` (each move starts the wait again).
fn cmd_hover(m: &mut Model, at: Point) -> Reply {
    m.hover = None;
    if m.config.settings.focus_follows_mouse && m.gesture.is_none() {
        m.hover = window_at(m, at)
            .filter(|w| m.focus_window != Some(*w))
            .map(|w| (w, m.now_ms + HOVER_FOCUS_MS));
    }
    Reply::ok()
}

fn cmd_gesture(m: &mut Model, phase: GesturePhase, fx: &mut Vec<Effect>) -> Reply {
    match phase {
        GesturePhase::Begin { at, pointer } => {
            cancel_gesture(m, fx);
            if let Some(wid) = window_at(m, at) {
                m.gesture = Some(Gesture { wid, target: None });
                gesture_move(m, pointer, fx);
            }
        }
        GesturePhase::Move { at } => gesture_move(m, at, fx),
        GesturePhase::Drop { at } => {
            let Some(g) = m.gesture.take() else {
                return Reply::ok();
            };
            fx.push(Effect::UiRequest(UiRequest::GestureOverlayHide));
            if !m.windows.contains_key(&g.wid) {
                return Reply::ok();
            }
            if let Some(t) = drop::target_at(m, g.wid, at) {
                drop_window(m, g.wid, &t, fx);
            }
        }
        GesturePhase::Cancel => cancel_gesture(m, fx),
    }
    Reply::ok()
}

fn gesture_move(m: &mut Model, at: Point, fx: &mut Vec<Effect>) {
    let Some(g) = m.gesture.clone() else { return };
    if !m.windows.contains_key(&g.wid) {
        cancel_gesture(m, fx);
        return;
    }
    let target = drop::target_at(m, g.wid, at);
    if target == g.target {
        return;
    }
    let shown = target
        .as_ref()
        .and_then(|t| drop::preview(m, g.wid, t).map(|p| (p, drop::label(m, t))));
    m.gesture = Some(Gesture {
        wid: g.wid,
        target: target.filter(|_| shown.is_some()),
    });
    fx.push(Effect::UiRequest(match shown {
        Some(((frame, target), label)) => UiRequest::GestureOverlay {
            frame,
            target,
            label,
        },
        None => UiRequest::GestureOverlayHide,
    }));
}

/// Carry out a gesture drop: `wid` goes to `target` and is tiled there.
fn drop_window(m: &mut Model, wid: WindowId, target: &DropTarget, fx: &mut Vec<Effect>) {
    let Some(dst) = drop::target_ws(m, target) else {
        return;
    };
    let w = m.windows[&wid].clone();
    let src = w.workspace.clone();
    for ws in [&src, &dst] {
        if let Some(x) = m.ws_mut(ws) {
            if x.fullscreen.take().is_some() {
                m.layout_dirty = true;
            }
        }
    }
    let root = m.ws(&dst).unwrap().root;
    let tiled_node = m.window_node(wid).filter(|_| w.mode == WindowMode::Tiled);
    match (target, tiled_node) {
        (DropTarget::Swap { other }, Some(n)) => {
            // Both stay tiles; across workspaces they trade workspaces too.
            drop::apply(&mut m.tree, root, Source::Leaf(n), target);
            if src != dst {
                m.windows.get_mut(other).unwrap().workspace = src.clone();
                if let Some(x) = m.ws_mut(&src) {
                    if x.focused == Some(wid) {
                        x.focused = Some(*other);
                    }
                }
                if let Some(x) = m.ws_mut(&dst) {
                    if x.focused == Some(*other) {
                        x.focused = None;
                    }
                }
            }
            refresh_record(m, *other);
        }
        (DropTarget::Swap { other }, None) => {
            // A floating window takes the tile; the other window floats
            // where it was.
            detach_window(m, wid, false);
            drop::apply(&mut m.tree, root, Source::New(Leaf::Window(wid)), target);
            if let Some(x) = m.ws_mut(&dst) {
                if x.focused == Some(*other) {
                    x.focused = None;
                }
            }
            let o = m.windows.get_mut(other).unwrap();
            o.mode = WindowMode::Floating;
            o.workspace = src.clone();
            o.float_frame = Some(w.float_frame.unwrap_or(w.actual));
            o.reason = "floated by a gesture swap".into();
            if let Some(x) = m.ws_mut(&src) {
                x.floating.push(*other);
            }
            refresh_record(m, *other);
        }
        _ => {
            detach_window(m, wid, false);
            drop::apply(&mut m.tree, root, Source::New(Leaf::Window(wid)), target);
        }
    }
    let wi = m.windows.get_mut(&wid).unwrap();
    wi.mode = WindowMode::Tiled;
    wi.workspace = dst.clone();
    wi.stubborn = false;
    if w.mode == WindowMode::Floating {
        wi.reason = "tiled by a gesture".into();
    }
    m.ghosts.remove(&wid);
    m.drags.remove(&wid);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    focus_fx(m, wid, fx);
    refresh_record(m, wid);
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
}

// ------------------------------------------------------ scratchpads, summon

/// Centered frame of `size` on display `d`.
fn scratch_frame(sp: &ScratchpadConfig, d: &crate::display::Display) -> Rect {
    let a = d.visible_frame;
    let (w, h) = sp.size_in(a.w, a.h);
    Rect::new(a.center().x - w / 2.0, a.center().y - h / 2.0, w, h).fit_into(&a)
}

/// Does `w` belong to the app the user named: bundle id or app name,
/// ignoring case.
fn app_matches(m: &Model, w: &WindowInfo, app: &str) -> bool {
    w.bundle_id.eq_ignore_ascii_case(app)
        || m.apps
            .get(&w.pid)
            .is_some_and(|a| a.name.eq_ignore_ascii_case(app))
}

/// The window of `app` the user most likely means: the most recently
/// focused standard window, not minimized if possible.
fn best_window_of(m: &Model, app: &str, skip: &dyn Fn(WindowId) -> bool) -> Option<WindowId> {
    m.windows
        .values()
        .filter(|w| w.role == crate::rules::WindowRole::Standard)
        .filter(|w| app_matches(m, w, app) && !skip(w.id))
        .max_by_key(|w| {
            (
                !w.minimized,
                m.focus_seen.get(&w.id).copied().unwrap_or(0),
                w.seq,
            )
        })
        .map(|w| w.id)
}

/// The window to focus in `ws` once `except` leaves: the one focused most
/// recently, else the first.
fn previous_window(m: &Model, ws: &str, except: WindowId) -> Option<WindowId> {
    m.windows
        .values()
        .filter(|w| w.id != except && w.workspace == ws && !w.minimized && m.window_shown(w))
        .filter_map(|w| m.focus_seen.get(&w.id).map(|t| (*t, w.id)))
        .max()
        .map(|(_, w)| w)
        .or_else(|| first_window(m, ws).filter(|w| *w != except))
}

/// Ask the shell to open `bundle`; `scratchpad` names the scratchpad its
/// first new window becomes.
fn launch(m: &mut Model, bundle: &str, scratchpad: Option<String>, fx: &mut Vec<Effect>) {
    let request_id = m.next_request_id;
    m.next_request_id += 1;
    m.pending_launches.push(PendingLaunch {
        request_id,
        bundle_id: bundle.to_string(),
        scratchpad,
        deadline_ms: m.now_ms + PENDING_SLOT_MS,
    });
    fx.push(Effect::UiRequest(UiRequest::LaunchApp {
        request_id,
        bundle_id: bundle.to_string(),
        args: vec![],
    }));
}

/// Scratchpads that are gone from the config let go of their windows
/// (a hidden one comes back where its workspace is).
fn prune_scratchpads(m: &mut Model) {
    let cfg = &m.config;
    let gone: Vec<WindowId> = m
        .scratch
        .iter()
        .filter(|(n, _)| cfg.scratchpad(n).is_none())
        .map(|(_, w)| *w)
        .collect();
    m.scratch.retain(|n, _| cfg.scratchpad(n).is_some());
    m.pending_launches.retain(|p| {
        p.scratchpad
            .as_ref()
            .is_none_or(|n| cfg.scratchpad(n).is_some())
    });
    for w in gone {
        if m.scratch_hidden.remove(&w) {
            m.layout_dirty = true;
        }
    }
}

fn cmd_scratchpad(m: &mut Model, name: &str, fx: &mut Vec<Effect>) -> Reply {
    let Some(sp) = m.config.scratchpad(name).cloned() else {
        return Reply::err(format!(
            "no scratchpad named `{name}` (add a [[scratchpad]] to config.toml)"
        ));
    };
    if m.scratch
        .get(name)
        .is_some_and(|w| !m.windows.contains_key(w))
    {
        m.scratch.remove(name);
    }
    if let Some(wid) = m.scratch.get(name).copied() {
        let w = &m.windows[&wid];
        let here = m
            .ws(&w.workspace)
            .is_some_and(|x| Some(&x.display) == m.focused_display().as_ref());
        let shown = m.window_shown(w) && !w.minimized;
        if shown && here && m.focus_window == Some(wid) {
            hide_scratch(m, wid, fx);
        } else {
            show_scratch(m, name, wid, false, fx);
        }
        return Reply::ok();
    }
    // Not set up yet: adopt the app's window if it has one, else open it.
    let taken: BTreeSet<WindowId> = m.scratch.values().copied().collect();
    if let Some(wid) = best_window_of(m, &sp.app, &|w| taken.contains(&w)) {
        m.scratch.insert(name.to_string(), wid);
        show_scratch(m, name, wid, true, fx);
        return Reply::ok();
    }
    if !m
        .pending_launches
        .iter()
        .any(|p| p.scratchpad.as_deref() == Some(name))
    {
        launch(m, &sp.app, Some(name.to_string()), fx);
    }
    Reply::ok()
}

/// Show scratchpad window `wid` as a float on the focused display's
/// workspace and focus it. It keeps a frame the user gave it on that
/// display unless `recenter`.
fn show_scratch(m: &mut Model, name: &str, wid: WindowId, recenter: bool, fx: &mut Vec<Effect>) {
    let Some(sp) = m.config.scratchpad(name).cloned() else {
        return;
    };
    let Some(d) = m.focused_display() else { return };
    let Some(ws) = m.visible.get(&d).cloned() else {
        return;
    };
    let Some(disp) = m.display(&d).cloned() else {
        return;
    };
    let w = m.windows[&wid].clone();
    let keep = (!recenter && w.mode == WindowMode::Floating)
        .then_some(w.float_frame)
        .flatten()
        .filter(|f| display_for_rect(&m.displays, f).is_some_and(|x| x.id == d));
    let frame = keep.unwrap_or_else(|| scratch_frame(&sp, &disp));
    // Re-add it so it is the newest float of the workspace (raised last).
    detach_window(m, wid, false);
    m.ws_mut(&ws).unwrap().floating.push(wid);
    let wi = m.windows.get_mut(&wid).unwrap();
    wi.workspace = ws;
    wi.mode = WindowMode::Floating;
    wi.float_frame = Some(frame);
    wi.reason = format!("scratchpad {name}");
    m.scratch_hidden.remove(&wid);
    if w.minimized {
        fx.push(Effect::Restore { pid: w.pid, wid });
    } else {
        fx.push(Effect::Focus { pid: w.pid, wid });
    }
    set_focus(m, wid, fx);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    refresh_record(m, wid);
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
}

/// Toggle scratchpad window `wid` off: park it and give focus back to the
/// window that had it before.
fn hide_scratch(m: &mut Model, wid: WindowId, fx: &mut Vec<Effect>) {
    let w = m.windows[&wid].clone();
    if w.mode == WindowMode::Tiled {
        // Tiled meanwhile (toggle-float, a gesture): float it so its tile
        // closes instead of leaving a hole.
        let r = m.desired.get(&wid).copied().unwrap_or(w.actual);
        detach_window(m, wid, false);
        m.ws_mut(&w.workspace).unwrap().floating.push(wid);
        let wi = m.windows.get_mut(&wid).unwrap();
        wi.mode = WindowMode::Floating;
        wi.float_frame = Some(r);
    }
    m.scratch_hidden.insert(wid);
    if let Some(x) = m.ws_mut(&w.workspace) {
        if x.focused == Some(wid) {
            x.focused = None;
        }
    }
    if m.focus_window == Some(wid) {
        m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
        match previous_window(m, &w.workspace, wid) {
            Some(p) => focus_fx(m, p, fx),
            None => {
                m.focus_window = None;
                fx.push(Effect::FocusDesktop);
            }
        }
    }
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
}

/// Bring `app`'s window to the focused workspace (next to the focused
/// tile, or as a float if it floats) and focus it.
fn cmd_summon(m: &mut Model, app: &str, fx: &mut Vec<Effect>) -> Reply {
    let app = app.trim();
    let Some(wid) = best_window_of(m, app, &|_| false) else {
        // Open it: new windows go to the focused display anyway. A running
        // app named by its name is opened by its bundle id.
        let bundle = m
            .apps
            .values()
            .find(|a| a.name.eq_ignore_ascii_case(app) || a.bundle_id.eq_ignore_ascii_case(app))
            .map(|a| a.bundle_id.clone())
            .unwrap_or_else(|| app.to_string());
        if !m.pending_launches.iter().any(|p| p.bundle_id == bundle) {
            launch(m, &bundle, None, fx);
        }
        return Reply::ok();
    };
    if let Some(name) = m.scratchpad_of(wid).map(str::to_string) {
        show_scratch(m, &name, wid, false, fx);
        return Reply::ok();
    }
    let Some(target) = m.focused_ws_id() else {
        return Reply::err("no display");
    };
    let w = m.windows[&wid].clone();
    if w.workspace != target {
        detach_window(m, wid, false);
        match w.mode {
            WindowMode::Floating => {
                m.ws_mut(&target).unwrap().floating.push(wid);
                // Keep its size; bring it onto the target display.
                let area = m
                    .ws(&target)
                    .and_then(|x| m.display(&x.display))
                    .map(|d| d.visible_frame);
                if let Some(a) = area {
                    let f = w.float_frame.unwrap_or(w.actual);
                    let on = display_for_rect(&m.displays, &f)
                        .is_some_and(|d| d.visible_frame.intersection(&a).is_some());
                    if !on || looks_parked(m, &f) {
                        let r =
                            Rect::new(a.center().x - f.w / 2.0, a.center().y - f.h / 2.0, f.w, f.h)
                                .fit_into(&a);
                        m.windows.get_mut(&wid).unwrap().float_frame = Some(r);
                    }
                }
            }
            WindowMode::Tiled => {
                let at = default_insert(m, &target);
                let root = m.ws(&target).unwrap().root;
                if let Some(z) = m.ws_mut(&target) {
                    z.fullscreen = None;
                }
                m.tree.insert_leaf(root, Leaf::Window(wid), at);
            }
        }
        m.windows.get_mut(&wid).unwrap().workspace = target.clone();
        refresh_record(m, wid);
    } else if let Some(z) = m.ws_mut(&target) {
        // Already here, maybe behind a zoomed tile.
        if z.fullscreen.is_some_and(|f| f != wid) {
            z.fullscreen = None;
        }
    }
    if w.minimized {
        fx.push(Effect::Restore { pid: w.pid, wid });
    } else {
        fx.push(Effect::Focus { pid: w.pid, wid });
    }
    set_focus(m, wid, fx);
    m.suppress_follow_until = m.now_ms + FOLLOW_SUPPRESS_MS;
    m.layout_dirty = true;
    fx.push(Effect::PersistState);
    fx.push(Effect::Snapshot);
    Reply::ok()
}
