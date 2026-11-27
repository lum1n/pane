//! Desired frames -> `SetFrame` effects, with retries and parking.

use crate::display::{park_rect, Display};
use crate::effect::Effect;
use crate::geometry::{Rect, Size};
use crate::layout::{self, LayoutInput};
use crate::model::{Model, SentFrame, WindowMode, MIN_SIZE_CONFIRM_MS, RETRY_BACKOFF_MS};
use crate::proto::UiRequest;
use crate::state::ParkedEntry;
use crate::tree::{Leaf, NodeId, Tree};
use crate::WindowId;

const MAX_OVERFLOW_PASSES: usize = 8;

/// Lay out the workspace tree at `root` (in `tree`, which may be a scratch
/// copy of `m.tree`) on `display`, with that display's gaps. `also_present`
/// counts as a tiled window even if it is floating now (a gesture preview).
pub fn layout_root(
    m: &Model,
    tree: &Tree,
    root: NodeId,
    display: &Display,
    also_present: Option<WindowId>,
) -> layout::LayoutOutput {
    let gaps = m.gaps(display);
    let present =
        |l: &Leaf| (also_present.is_some() && l.window() == also_present) || m.leaf_present(l);
    let min = |l: &Leaf| m.leaf_min_size(l);
    layout::compute(
        tree,
        root,
        display.visible_frame,
        &LayoutInput {
            inner_gap: gaps.inner,
            outer_gap: gaps.outer,
            smart_gaps: m.config.settings.smart_gaps,
            present: &present,
            min_size: &min,
        },
    )
}

/// Recompute `m.desired` for every window. Floats the newest window of any
/// container whose minimum sizes cannot fit; returns notices about that.
pub fn refresh_desired(m: &mut Model) -> Vec<Effect> {
    let mut fx = Vec::new();
    m.layout_dirty = false;
    repair(m);
    for _ in 0..MAX_OVERFLOW_PASSES {
        let mut desired = std::collections::BTreeMap::new();
        let mut slot_frames = Vec::new();
        let mut overflow: Vec<WindowId> = Vec::new();
        for d in &m.displays {
            let Some(ws) = m.visible.get(&d.id).and_then(|w| m.ws(w)) else {
                continue;
            };
            let out = layout_root(m, &m.tree, ws.root, d, None);
            for (node, rect) in out.frames {
                match m.tree.leaf(node) {
                    Some(Leaf::Window(w)) => {
                        desired.insert(*w, rect);
                    }
                    Some(Leaf::Slot(s)) => slot_frames.push((ws.id.clone(), s.clone(), rect)),
                    None => {}
                }
            }
            // A zoomed tile fills the display's visible frame (no gaps).
            if let Some(z) = ws.fullscreen.filter(|z| {
                m.windows
                    .get(z)
                    .is_some_and(|w| w.mode == WindowMode::Tiled && !w.minimized)
            }) {
                desired.insert(z, d.visible_frame);
            }
            overflow.extend(
                out.overflow
                    .iter()
                    .filter_map(|n| m.tree.leaf(*n).and_then(|l| l.window())),
            );
        }
        if let Some(newest) = overflow
            .iter()
            .filter_map(|w| m.windows.get(w))
            .max_by_key(|w| w.seq)
            .map(|w| w.id)
        {
            float_window(m, newest);
            let name = m.windows[&newest].title.clone();
            fx.push(Effect::UiRequest(UiRequest::info(format!(
                "Not enough room to tile \"{name}\"; floating it"
            ))));
            continue;
        }
        // Floating windows keep their frame; hidden workspaces get parked.
        for w in m.windows.values() {
            if w.minimized {
                continue;
            }
            let Some(ws) = m.ws(&w.workspace) else {
                continue;
            };
            let Some(disp) = m.display(&ws.display) else {
                continue;
            };
            let visible = m.window_shown(w);
            if visible {
                if w.mode == WindowMode::Floating {
                    desired.insert(w.id, w.float_frame.unwrap_or(w.actual));
                }
            } else if m.shutting_down {
                let r = restore_rect(m, w.id).fit_into(&disp.visible_frame);
                desired.insert(w.id, r);
            } else {
                let size = park_size(m, w.id);
                desired.insert(w.id, park_rect(disp, size, &m.displays));
            }
        }
        m.desired = desired;
        m.slot_frames = slot_frames;
        return fx;
    }
    fx
}

/// Enforce the model's invariants, whatever sequence of events led here:
/// every window is in an existing workspace; a tiled window has exactly one
/// leaf, in its own workspace's tree; a floating window is in exactly its
/// own workspace's floating list. Returns how many windows were fixed.
pub fn repair(m: &mut Model) -> usize {
    let mut fixed = 0;
    let wids: Vec<WindowId> = m.windows.keys().copied().collect();
    for wid in wids {
        let w = m.windows[&wid].clone();
        // Leaves and floating entries for this window, anywhere.
        let mut leaves: Vec<(crate::WorkspaceId, crate::tree::NodeId)> = Vec::new();
        for ws in &m.workspaces {
            if let Some(n) = m.tree.find_window(ws.root, wid) {
                leaves.push((ws.id.clone(), n));
            }
        }
        let mut ws_id = w.workspace.clone();
        if m.ws(&ws_id).is_none() {
            // Dangling workspace: adopt the one holding its leaf, else the
            // visible workspace of the display it is on.
            ws_id = leaves.first().map(|l| l.0.clone()).unwrap_or_else(|| {
                let on = if crate::reducer::looks_parked(m, &w.actual) {
                    None
                } else {
                    crate::display::display_for_rect(&m.displays, &w.actual).map(|d| d.id.clone())
                };
                on.or_else(|| m.focused_display())
                    .and_then(|d| m.visible.get(&d).cloned())
                    .or_else(|| m.workspaces.first().map(|x| x.id.clone()))
                    .unwrap_or_default()
            });
            if m.ws(&ws_id).is_none() {
                continue;
            }
            m.windows.get_mut(&wid).unwrap().workspace = ws_id.clone();
            fixed += 1;
        }
        match w.mode {
            WindowMode::Tiled => {
                let mut kept = false;
                for (lws, n) in &leaves {
                    if *lws == ws_id && !kept {
                        kept = true;
                    } else {
                        m.tree.remove(*n);
                        fixed += 1;
                    }
                }
                if !kept {
                    let root = m.ws(&ws_id).unwrap().root;
                    m.tree
                        .insert_leaf(root, Leaf::Window(wid), crate::tree::InsertAt::End);
                    fixed += 1;
                }
                for ws in &mut m.workspaces {
                    let before = ws.floating.len();
                    ws.floating.retain(|f| *f != wid);
                    fixed += before - ws.floating.len();
                }
            }
            WindowMode::Floating => {
                for (_, n) in &leaves {
                    m.tree.remove(*n);
                    fixed += 1;
                }
                for ws in &mut m.workspaces {
                    let mine = ws.id == ws_id;
                    let count = ws.floating.iter().filter(|f| **f == wid).count();
                    if mine && count == 0 {
                        ws.floating.push(wid);
                        fixed += 1;
                    } else if (!mine && count > 0) || count > 1 {
                        ws.floating.retain(|f| *f != wid);
                        if mine {
                            ws.floating.push(wid);
                        }
                        fixed += 1;
                    }
                }
            }
        }
    }
    fixed
}

/// Shows at most a thin strip (60pt) on every display.
pub fn mostly_hidden(m: &Model, r: &Rect) -> bool {
    m.displays.iter().all(|d| {
        d.frame
            .intersection(r)
            .is_none_or(|i| i.w <= 60.0 || i.h <= 60.0)
    })
}

fn float_window(m: &mut Model, wid: WindowId) {
    let Some(w) = m.windows.get(&wid).cloned() else {
        return;
    };
    if let Some(n) = m.window_node(wid) {
        m.tree.remove(n);
    }
    let area = m
        .ws(&w.workspace)
        .and_then(|ws| m.display(&ws.display))
        .map(|d| d.visible_frame);
    let size = w.min_size.unwrap_or(w.actual.size());
    let frame = match area {
        Some(a) => Rect::new(
            a.center().x - size.w / 2.0,
            a.center().y - size.h / 2.0,
            size.w,
            size.h,
        )
        .fit_into(&a),
        None => w.actual,
    };
    if let Some(ws) = m.ws_mut(&w.workspace) {
        ws.floating.push(wid);
    }
    let wm = m.windows.get_mut(&wid).unwrap();
    wm.mode = WindowMode::Floating;
    wm.float_frame = Some(frame);
    if wm.stubborn {
        wm.reason = "refused to take its tile".into();
    } else {
        wm.reason = "not enough room for its minimum size".into();
    }
}

/// Size to keep while parked: the last on-screen size.
fn park_size(m: &Model, wid: WindowId) -> Size {
    if let Some(p) = m.parked.get(&wid) {
        return p.original.size();
    }
    let w = &m.windows[&wid];
    m.desired
        .get(&wid)
        .filter(|r| !crate::reducer::looks_parked(m, r))
        .map(|r| r.size())
        .unwrap_or(w.actual.size())
}

/// Where a parked window goes back to when Pane quits.
fn restore_rect(m: &Model, wid: WindowId) -> Rect {
    let w = &m.windows[&wid];
    if w.mode == WindowMode::Floating {
        if let Some(f) = w.float_frame {
            return f;
        }
    }
    m.parked.get(&wid).map(|p| p.original).unwrap_or(w.actual)
}

/// Emit SetFrames for every window whose desired frame was not sent yet,
/// plus due retries. `WriteParked` always precedes newly parking frames.
pub fn reconcile(m: &mut Model) -> Vec<Effect> {
    let mut fx = Vec::new();
    if m.layout_dirty {
        fx.extend(refresh_desired(m));
    }
    let now = m.now_ms;

    // Parking journal.
    if !m.shutting_down {
        let mut changed = false;
        let hidden: Vec<WindowId> = m
            .windows
            .values()
            .filter(|w| !w.minimized && m.ws(&w.workspace).is_some() && !m.window_shown(w))
            .map(|w| w.id)
            .collect();
        for wid in &hidden {
            if !m.parked.contains_key(wid) {
                let w = &m.windows[wid];
                let original = if crate::reducer::looks_parked(m, &w.actual) {
                    w.float_frame
                        .or_else(|| m.desired.get(wid).copied())
                        .unwrap_or(w.actual)
                } else {
                    w.actual
                };
                let entry = ParkedEntry {
                    pid: w.pid,
                    wid: *wid,
                    bundle_id: w.bundle_id.clone(),
                    title: w.title.clone(),
                    original,
                };
                m.parked.insert(*wid, entry);
                changed = true;
            }
        }
        let before = m.parked.len();
        m.parked.retain(|w, _| hidden.contains(w));
        changed |= before != m.parked.len();
        if changed {
            fx.push(Effect::WriteParked {
                entries: m.parked.values().cloned().collect(),
            });
        }
    }

    let targets: Vec<(WindowId, Rect)> = m.desired.iter().map(|(w, r)| (*w, *r)).collect();
    for (wid, rect) in targets {
        let Some(w) = m.windows.get(&wid) else {
            continue;
        };
        if w.minimized {
            continue;
        }
        let target_parked = m.parked.contains_key(&wid) && !m.shutting_down;
        // A stubborn window refuses its tile, but it must still be hidden
        // and shown with its workspace.
        if w.stubborn
            && m.last_sent
                .get(&wid)
                .is_some_and(|s| s.parked == target_parked)
        {
            continue;
        }
        let Some(app) = m.apps.get(&w.pid) else {
            continue;
        };
        if app.unresponsive {
            continue;
        }
        let (pid, double_set) = (w.pid, w.double_set);
        let restore_to = m
            .parked
            .get(&wid)
            .filter(|_| !m.shutting_down)
            .map(|p| p.original);
        let send = match m.last_sent.get(&wid) {
            None => {
                if w.actual.approx_eq(&rect, 0.5) {
                    let gen = m.next_gen();
                    m.last_sent.insert(
                        wid,
                        SentFrame {
                            rect,
                            gen,
                            at_ms: 0,
                            attempts: 0,
                            next_retry_ms: None,
                            settled: true,
                            parked: m.parked.contains_key(&wid),
                            oversize: None,
                            burst: false,
                        },
                    );
                    false
                } else {
                    true
                }
            }
            Some(s) if !s.rect.approx_eq(&rect, 0.5) => true,
            Some(s) => !s.settled && s.next_retry_ms.is_some_and(|t| now >= t),
        };
        if !send {
            continue;
        }
        let gen = m.next_gen();
        let (attempts, oversize) = match m.last_sent.get(&wid) {
            Some(s) if s.rect.approx_eq(&rect, 0.5) => (s.attempts, s.oversize),
            _ => (0, None),
        };
        let was_parked = m.last_sent.get(&wid).map(|s| s.parked);
        // Replacing a different frame the app has not answered yet.
        let burst = m.last_sent.get(&wid).is_some_and(|s| {
            !s.settled
                && !s.rect.approx_eq(&rect, 0.5)
                && now.saturating_sub(s.at_ms) < MIN_SIZE_CONFIRM_MS
        });
        if was_parked != Some(target_parked) {
            let title = &m.windows[&wid].title;
            if target_parked {
                tracing::info!(%wid, %title, ?rect, "hiding window");
            } else {
                tracing::info!(%wid, %title, ?rect, "showing window");
            }
        }
        m.last_sent.insert(
            wid,
            SentFrame {
                rect,
                gen,
                at_ms: now,
                attempts,
                next_retry_ms: None,
                settled: false,
                parked: target_parked,
                oversize,
                burst,
            },
        );
        if let Some(a) = m.apps.get_mut(&pid) {
            a.awaiting_since.get_or_insert(now);
        }
        fx.push(Effect::SetFrame {
            pid,
            wid,
            rect,
            gen,
            double_set,
            restore_to,
        });
    }
    fx
}

/// A worker reports the outcome of a SetFrame.
pub fn on_frame_applied(
    m: &mut Model,
    wid: WindowId,
    gen: u64,
    ok: bool,
    actual: Option<Rect>,
    fx: &mut Vec<Effect>,
) {
    let Some(w) = m.windows.get_mut(&wid) else {
        return;
    };
    if let Some(a) = actual {
        w.actual = a;
    }
    let pid = w.pid;
    if let Some(app) = m.apps.get_mut(&pid) {
        app.awaiting_since = None;
    }
    let parked = m.parked.contains_key(&wid) && !m.shutting_down;
    let Some(s) = m.last_sent.get(&wid).cloned() else {
        return;
    };
    if s.gen != gen {
        return;
    }
    let a = actual.unwrap_or(m.windows[&wid].actual);
    let d = s.rect;
    let landed = if parked {
        // macOS may nudge a window it finds too far off-screen; anything
        // that leaves at most a thin strip visible counts as hidden.
        a.origin_near(&d, 4.0) || mostly_hidden(m, &a)
    } else {
        a.origin_near(&d, 1.0) && (a.w - d.w).abs() <= 2.0 && (a.h - d.h).abs() <= 2.0
    };
    // Trust where the window actually is over the AX status: apps (and
    // macOS for off-screen moves) report errors for moves that did happen.
    // Only without a read-back does the status decide.
    if landed && (actual.is_some() || ok) {
        if !ok {
            tracing::debug!(%wid, "AX reported an error but the frame landed");
        }
        let e = m.last_sent.get_mut(&wid).unwrap();
        e.settled = true;
        e.next_retry_ms = None;
        return;
    }
    // Larger than asked: the app may have a minimum size, or it applies the
    // resize late and this is still its old size. Once a retry reads the
    // same size, remember it and let the layout give the space to a
    // neighbour. Until then it is retried like any other miss. A frame sent
    // in a burst (another frame went to the window just before it) cannot
    // start that confirmation: the app may still be working through the
    // earlier frame, and its in-between size can read back the same twice.
    let too_big = !parked && (a.w > d.w + 2.0 || a.h > d.h + 2.0);
    let confirmed = s
        .oversize
        .is_some_and(|o| (a.w - o.w).abs() <= 2.0 && (a.h - o.h).abs() <= 2.0);
    if too_big && confirmed {
        let w = m.windows.get_mut(&wid).unwrap();
        let prev = w.min_size.unwrap_or(Size::ZERO);
        let learned = Size::new(
            if a.w > d.w + 2.0 { a.w } else { 0.0 },
            if a.h > d.h + 2.0 { a.h } else { 0.0 },
        );
        w.min_size = Some(prev.max(learned));
        let e = m.last_sent.get_mut(&wid).unwrap();
        e.settled = true;
        e.next_retry_ms = None;
        m.layout_dirty = true;
        return;
    }
    // Bringing a window back from hiding: some apps apply moves late, so the
    // read-back can still show it hidden. Keep trying (never give up on
    // showing a window whose workspace is shown).
    if !parked && mostly_hidden(m, &a) {
        tracing::info!(%wid, ?a, target = ?d, attempts = s.attempts, "window still hidden after unpark; retrying");
        let e = m.last_sent.get_mut(&wid).unwrap();
        e.attempts = (s.attempts + 1).min(RETRY_BACKOFF_MS.len() as u32);
        e.next_retry_ms = Some(m.now_ms + RETRY_BACKOFF_MS[(e.attempts - 1) as usize].min(1000));
        return;
    }
    let w = m.windows.get_mut(&wid).unwrap();
    if s.attempts == 0 && !w.double_set {
        w.double_set = true;
    }
    let attempts = s.attempts + 1;
    if attempts as usize > RETRY_BACKOFF_MS.len() && parked {
        // Never give up on a window over hiding it: settle where it is.
        let e = m.last_sent.get_mut(&wid).unwrap();
        e.settled = true;
        e.next_retry_ms = None;
        return;
    }
    if attempts as usize > RETRY_BACKOFF_MS.len() && actual.is_none() {
        // We cannot even read where it is: do not punish the window for it.
        tracing::warn!(%wid, "no frame read-back after retries; leaving it");
        let e = m.last_sent.get_mut(&wid).unwrap();
        e.settled = true;
        e.next_retry_ms = None;
        return;
    }
    if attempts as usize > RETRY_BACKOFF_MS.len() {
        tracing::warn!(%wid, title = %w.title, ?a, target = ?d, "window refuses its frame; floating it");
        w.stubborn = true;
        let title = w.title.clone();
        let tiled = w.mode == WindowMode::Tiled;
        let e = m.last_sent.get_mut(&wid).unwrap();
        e.settled = true;
        e.next_retry_ms = None;
        if tiled {
            float_window(m, wid);
            m.layout_dirty = true;
        }
        fx.push(Effect::UiRequest(UiRequest::warn(format!(
            "\"{title}\" refuses to move; leaving it floating"
        ))));
        return;
    }
    let e = m.last_sent.get_mut(&wid).unwrap();
    e.attempts = attempts;
    e.oversize = (too_big && !s.burst).then(|| a.size());
    let mut wait = RETRY_BACKOFF_MS[(attempts - 1) as usize];
    if too_big {
        wait = wait.max(MIN_SIZE_CONFIRM_MS);
    }
    e.next_retry_ms = Some(m.now_ms + wait);
}
