//! One thread per application: owns its AX elements and observer, so a
//! hung app blocks nobody but itself.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use crossbeam_channel::{Receiver, Sender};
use pane_core::event::{Event, WindowProbe};
use pane_core::worker::WorkerCmd;
use pane_core::WindowId;

use crate::ax::AxElement;
use crate::observer::{Observer, APP_NOTIFICATIONS, WINDOW_NOTIFICATIONS};
use crate::recovery::mirror;
use crate::runloop::Waker;
use crate::window_probe::{is_fullscreen, probe};

/// Handle the runtime keeps per app.
pub struct Worker {
    tx: Sender<WorkerCmd>,
    waker: Waker,
}

impl Worker {
    pub fn send(&self, cmd: WorkerCmd) {
        if self.tx.send(cmd).is_ok() {
            self.waker.wake();
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Quit is handled by the source; stop() covers a run loop that is
        // already running but busy.
        self.send(WorkerCmd::Quit);
        self.waker.stop();
    }
}

struct State {
    pid: i32,
    app: AxElement,
    observer: Option<Observer>,
    windows: Vec<(WindowId, AxElement)>,
    events: Sender<Event>,
}

/// Spawn the worker; it enumerates the app's windows right away and emits
/// `WindowsDiscovered { initial: true }`.
pub fn spawn(pid: i32, events: Sender<Event>) -> Option<Worker> {
    let (tx, rx) = crossbeam_channel::unbounded::<WorkerCmd>();
    let (wtx, wrx) = crossbeam_channel::bounded::<Waker>(1);
    std::thread::Builder::new()
        .name(format!("pane-app-{pid}"))
        .spawn(move || run(pid, rx, events, wtx))
        .ok()?;
    let waker = wrx.recv_timeout(Duration::from_secs(5)).ok()?;
    Some(Worker { tx, waker })
}

fn run(pid: i32, rx: Receiver<WorkerCmd>, events: Sender<Event>, wtx: Sender<Waker>) {
    let app = AxElement::application(pid);
    app.set_timeout(1.0);
    let st = Rc::new(RefCell::new(State {
        pid,
        app: app.clone(),
        observer: None,
        windows: vec![],
        events,
    }));

    let st_obs = st.clone();
    let observer = Observer::new(pid, move |el, n| on_notification(&st_obs, el, n));
    st.borrow_mut().observer = observer;

    let st_cmd = st.clone();
    let rx_cmd = rx.clone();
    let waker = Waker::install(move || {
        while let Ok(cmd) = rx_cmd.try_recv() {
            if !on_command(&st_cmd, cmd) {
                crate::runloop::Waker::stop_current();
                return;
            }
        }
    });
    if wtx.send(waker).is_err() {
        return;
    }

    // Apps that just launched answer AX with errors for a moment; retry.
    let mut ready = false;
    for _ in 0..20 {
        let s = st.borrow();
        let ok = s.app.attribute("AXWindows").is_ok()
            && s.observer
                .as_ref()
                .is_some_and(|o| APP_NOTIFICATIONS.iter().all(|n| o.add(&s.app, n)));
        drop(s);
        if ok {
            ready = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if !ready {
        tracing::debug!(pid, "app did not become AX-ready; observing anyway");
    }
    enumerate(&st, true);
    crate::runloop::run_current();
    // Tear down on this thread: the observer's run-loop source and the AX
    // elements belong here.
    let mut s = st.borrow_mut();
    if let Some(o) = &s.observer {
        for (_, w) in &s.windows {
            for n in WINDOW_NOTIFICATIONS {
                o.remove(w, n);
            }
        }
        for n in APP_NOTIFICATIONS {
            o.remove(&s.app, n);
        }
    }
    s.observer = None;
    s.windows.clear();
}

fn emit(st: &Rc<RefCell<State>>, e: Event) {
    let _ = st.borrow().events.send(e);
}

fn track(st: &Rc<RefCell<State>>, el: &AxElement) -> Option<WindowProbe> {
    if is_fullscreen(el) {
        return None;
    }
    let p = probe(st.borrow().pid, el)?;
    let mut s = st.borrow_mut();
    if !s.windows.iter().any(|(w, _)| *w == p.wid) {
        if let Some(o) = &s.observer {
            for n in WINDOW_NOTIFICATIONS {
                o.add(el, n);
            }
        }
        s.windows.push((p.wid, el.clone()));
    }
    Some(p)
}

fn enumerate(st: &Rc<RefCell<State>>, initial: bool) {
    let els = st.borrow().app.elements("AXWindows");
    let known: Vec<WindowId> = st.borrow().windows.iter().map(|w| w.0).collect();
    let probes: Vec<WindowProbe> = els.iter().filter_map(|el| track(st, el)).collect();
    // Windows that vanished without a destroyed notification.
    let seen: Vec<WindowId> = probes.iter().map(|p| p.wid).collect();
    let pid = st.borrow().pid;
    for w in known.into_iter().filter(|w| !seen.contains(w)) {
        st.borrow_mut().windows.retain(|x| x.0 != w);
        emit(st, Event::WindowDestroyed { pid, wid: w });
    }
    emit(
        st,
        Event::WindowsDiscovered {
            pid,
            windows: probes,
            initial,
        },
    );
    focused(st);
}

fn find(st: &Rc<RefCell<State>>, el: &AxElement) -> Option<WindowId> {
    st.borrow()
        .windows
        .iter()
        .find(|(_, w)| w == el)
        .map(|(id, _)| *id)
        .or_else(|| el.window_id())
}

fn element(st: &Rc<RefCell<State>>, wid: WindowId) -> Option<AxElement> {
    st.borrow()
        .windows
        .iter()
        .find(|(w, _)| *w == wid)
        .map(|(_, e)| e.clone())
}

fn focused(st: &Rc<RefCell<State>>) {
    let app = st.borrow().app.clone();
    let pid = st.borrow().pid;
    let wid = app.element("AXFocusedWindow").and_then(|el| {
        let id = find(st, &el);
        if id.is_some_and(|id| element(st, id).is_none()) {
            if let Some(p) = track(st, &el) {
                emit(
                    st,
                    Event::WindowsDiscovered {
                        pid,
                        windows: vec![p],
                        initial: false,
                    },
                );
            }
        }
        id
    });
    emit(st, Event::FocusChanged { pid, wid });
}

fn on_notification(st: &Rc<RefCell<State>>, el: AxElement, n: &str) {
    let pid = st.borrow().pid;
    match n {
        "AXWindowCreated" => {
            if let Some(p) = track(st, &el) {
                emit(
                    st,
                    Event::WindowsDiscovered {
                        pid,
                        windows: vec![p],
                        initial: false,
                    },
                );
            }
        }
        "AXUIElementDestroyed" => {
            let hit = st.borrow().windows.iter().position(|(_, w)| *w == el);
            if let Some(i) = hit {
                let (wid, _) = st.borrow_mut().windows.remove(i);
                mirror().remove(wid);
                emit(st, Event::WindowDestroyed { pid, wid });
            }
        }
        "AXWindowMoved" | "AXWindowResized" => {
            if let (Some(wid), Some(frame)) = (find(st, &el), el.frame()) {
                emit(st, Event::WindowFrameChanged { pid, wid, frame });
            }
        }
        "AXTitleChanged" => {
            if let Some(wid) = find(st, &el) {
                emit(
                    st,
                    Event::TitleChanged {
                        pid,
                        wid,
                        title: el.string("AXTitle").unwrap_or_default(),
                    },
                );
            }
        }
        "AXWindowMiniaturized" | "AXWindowDeminiaturized" => {
            if let Some(wid) = find(st, &el) {
                emit(
                    st,
                    Event::WindowMinimized {
                        pid,
                        wid,
                        minimized: n == "AXWindowMiniaturized",
                    },
                );
            }
        }
        "AXFocusedWindowChanged" | "AXApplicationActivated" => focused(st),
        "AXApplicationShown" => enumerate(st, false),
        _ => {}
    }
}

/// Returns false to stop the worker.
fn on_command(st: &Rc<RefCell<State>>, cmd: WorkerCmd) -> bool {
    let pid = st.borrow().pid;
    match cmd {
        WorkerCmd::SetFrame {
            wid,
            rect,
            gen,
            double_set,
            restore_to,
        } => {
            let Some(el) = element(st, wid) else {
                emit(
                    st,
                    Event::FrameApplied {
                        pid,
                        wid,
                        gen,
                        ok: false,
                        actual: None,
                    },
                );
                return true;
            };
            let mut ok = el.set_frame(&rect, double_set).is_ok();
            let mut actual = el.frame();
            // Chromium quirk: the first attempt can land a few points off when
            // crossing displays; one immediate size/position/size fixes it.
            if !double_set && actual.is_some_and(|a| !a.approx_eq(&rect, 1.0)) {
                ok = el.set_frame(&rect, true).is_ok();
                actual = el.frame();
            }
            match restore_to {
                Some(r) => mirror().insert(wid, el.clone(), r),
                None => mirror().remove(wid),
            }
            emit(
                st,
                Event::FrameApplied {
                    pid,
                    wid,
                    gen,
                    ok,
                    actual,
                },
            );
        }
        WorkerCmd::Raise { wid } => {
            if let Some(el) = element(st, wid) {
                let _ = el.perform("AXRaise");
            }
        }
        WorkerCmd::Focus { wid } => {
            if let Some(el) = element(st, wid) {
                let _ = el.set_bool("AXMain", true);
                let _ = el.perform("AXRaise");
                let app = st.borrow().app.clone();
                if app.set_bool("AXFrontmost", true).is_err() {
                    crate::apps::activate(pid);
                }
            }
        }
        WorkerCmd::Enumerate => enumerate(st, false),
        WorkerCmd::Recover { displays, known } => {
            let n = crate::recovery::sweep_app(pid, &displays, &known);
            if n > 0 {
                tracing::info!(pid, n, "recovered windows at park corners");
            }
        }
        WorkerCmd::Quit => return false,
    }
    true
}
