//! Desired frames -> `SetFrame` effects, with retries and parking.

use crate::display::park_rect;
use crate::effect::Effect;
use crate::geometry::{Rect, Size};
use crate::layout::{self, LayoutInput};
use crate::model::{Model, SentFrame, WindowMode, RETRY_BACKOFF_MS};
use crate::proto::UiRequest;
use crate::state::ParkedEntry;
use crate::tree::Leaf;
use crate::WindowId;

const MAX_OVERFLOW_PASSES: usize = 8;

/// Recompute `m.desired` for every window. Floats the newest window of any
/// container whose minimum sizes cannot fit; returns notices about that.
pub fn refresh_desired(m: &mut Model) -> Vec<Effect> {
    let mut fx = Vec::new();
    m.layout_dirty = false;
    for _ in 0..MAX_OVERFLOW_PASSES {
        let mut desired = std::collections::BTreeMap::new();
        let mut slot_frames = Vec::new();
        let mut overflow: Vec<WindowId> = Vec::new();
        for d in &m.displays {
            let Some(ws) = m.visible.get(&d.id).and_then(|w| m.ws(w)) else {
                continue;
            };
            let area = m.tiling_area(d);
            let present = |l: &Leaf| m.leaf_present(l);
            let min = |l: &Leaf| m.leaf_min_size(l);
            let out = layout::compute(
                &m.tree,
                ws.root,
                area,
                &LayoutInput {
                    inner_gap: m.config.settings.inner_gap,
                    present: &present,
                    min_size: &min,
                },
            );
            for (node, rect) in out.frames {
                match m.tree.leaf(node) {
                    Some(Leaf::Window(w)) => {
                        desired.insert(*w, rect);
                    }
                    Some(Leaf::Slot(s)) => slot_frames.push((ws.id.clone(), s.clone(), rect)),
                    None => {}
                }
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
            let visible = m.is_visible(&ws.id);
            if visible {
                if w.mode == WindowMode::Floating {
                    desired.insert(w.id, w.float_frame.unwrap_or(w.actual));
                }
            } else if m.shutting_down {
                let r = restore_rect(m, w.id).fit_into(&disp.visible_frame);
                desired.insert(w.id, r);
            } else {
                let size = park_size(m, w.id);
                desired.insert(w.id, park_rect(disp, size));
            }
        }
        m.desired = desired;
        m.slot_frames = slot_frames;
        return fx;
    }
    fx
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
            .filter(|w| !w.minimized && m.ws(&w.workspace).is_some_and(|ws| !m.is_visible(&ws.id)))
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
        if w.stubborn || w.minimized {
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
        let attempts = match m.last_sent.get(&wid) {
            Some(s) if s.rect.approx_eq(&rect, 0.5) => s.attempts,
            _ => 0,
        };
        m.last_sent.insert(
            wid,
            SentFrame {
                rect,
                gen,
                at_ms: now,
                attempts,
                next_retry_ms: None,
                settled: false,
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
        a.origin_near(&d, 4.0)
    } else {
        a.origin_near(&d, 1.0) && (a.w - d.w).abs() <= 2.0 && (a.h - d.h).abs() <= 2.0
    };
    if ok && landed {
        let e = m.last_sent.get_mut(&wid).unwrap();
        e.settled = true;
        e.next_retry_ms = None;
        return;
    }
    // Larger than asked: the app has a minimum size. Remember it and let
    // the layout give the space to a neighbour instead of retrying.
    if !parked && (a.w > d.w + 2.0 || a.h > d.h + 2.0) {
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
    let w = m.windows.get_mut(&wid).unwrap();
    if s.attempts == 0 && !w.double_set {
        w.double_set = true;
    }
    let attempts = s.attempts + 1;
    if attempts as usize > RETRY_BACKOFF_MS.len() {
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
    e.next_retry_ms = Some(m.now_ms + RETRY_BACKOFF_MS[(attempts - 1) as usize]);
}
