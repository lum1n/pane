//! Getting parked windows back no matter how Pane goes down.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use pane_core::{Rect, WindowId};

use crate::ax::AxElement;

/// Parked windows and where they belong, kept in sync by the workers so a
/// panic hook can restore them with raw AX calls.
#[derive(Default)]
pub struct Mirror {
    inner: Mutex<HashMap<WindowId, (AxElement, Rect)>>,
}

pub fn mirror() -> &'static Mirror {
    static M: OnceLock<Mirror> = OnceLock::new();
    M.get_or_init(Mirror::default)
}

impl Mirror {
    pub fn insert(&self, wid: WindowId, el: AxElement, restore: Rect) {
        if let Ok(mut m) = self.inner.lock() {
            m.insert(wid, (el, restore));
        }
    }

    pub fn remove(&self, wid: WindowId) {
        if let Ok(mut m) = self.inner.lock() {
            m.remove(&wid);
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Put every mirrored window back. Safe to call from a panic hook: uses a
/// poisoned lock if it must, and 500ms AX timeouts.
pub fn emergency_restore() -> usize {
    let guard = match mirror().inner.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    let mut n = 0;
    for (el, rect) in guard.values() {
        el.set_timeout(0.5);
        if el.set_frame(rect, true).is_ok() {
            n += 1;
        }
    }
    n
}

/// Move any window of `pid` whose origin sits at a park corner back on
/// screen. `displays` = (frame, visible_frame); `known` = saved frames.
pub fn sweep_app(pid: i32, displays: &[(Rect, Rect)], known: &[(WindowId, Rect)]) -> usize {
    let app = AxElement::application(pid);
    app.set_timeout(0.5);
    let mut n = 0;
    for w in app.elements("AXWindows") {
        let Some(f) = w.frame() else { continue };
        // Either park corner (see pane_core::display::park_corners).
        let near = |x: f64, y: f64| (f.x - x).abs() <= 4.0 && (f.y - y).abs() <= 4.0;
        let Some((_, vis)) = displays.iter().find(|(frame, _)| {
            let b = frame.max_y() - 1.0;
            near(frame.max_x() - 1.0, b) || near(frame.x - f.w + 1.0, b)
        }) else {
            continue;
        };
        let target = w
            .window_id()
            .and_then(|id| known.iter().find(|(k, _)| *k == id).map(|(_, r)| *r))
            .map(|r| r.fit_into(vis))
            .unwrap_or_else(|| {
                Rect::new(
                    vis.center().x - f.w / 2.0,
                    vis.center().y - f.h / 2.0,
                    f.w,
                    f.h,
                )
                .fit_into(vis)
            });
        if w.set_frame(&target, true).is_ok() {
            n += 1;
        }
    }
    n
}

/// Sweep every app with windows. Used by `pane recover --all-apps` and when
/// no agent is running.
pub fn sweep_all(known: &[(WindowId, Rect)]) -> usize {
    let displays: Vec<(Rect, Rect)> = crate::display::displays()
        .into_iter()
        .map(|d| (d.frame, d.visible_frame))
        .collect();
    crate::apps::window_owner_pids()
        .into_iter()
        .map(|pid| sweep_app(pid, &displays, known))
        .sum()
}
