//! The core thread and its plumbing.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Once};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{select, unbounded, Receiver, Sender};
use pane_core::config::{Chord, Config, Mods, STARTER_CONFIG};
use pane_core::display::Display;
use pane_core::drift::{self, ProfileDiff};
use pane_core::effect::Effect;
use pane_core::event::Event;
use pane_core::model::Model;
use pane_core::profile::{Profile, ProfileCondition};
use pane_core::proto::{Command, QueryKind, Reply, UiRequest};
use pane_core::reconcile::reconcile;
use pane_core::reducer::{reduce, refresh_placements};
use pane_core::snapshot::Snapshot;
use pane_core::stream::{self, EventKind};
use pane_core::worker::WorkerCmd;
use pane_core::Pid;

use crate::lockfile::{self, LockOutcome};
use crate::paths::Paths;
use crate::persist;
use crate::platform::{AppWorker, HotkeyHandle, Platform};

pub type Sink = Arc<dyn Fn(&str) + Send + Sync>;

pub struct Options {
    pub paths: Paths,
    /// Receives `Snapshot` JSON whenever it changes (at most ~20 Hz).
    pub on_snapshot: Option<Sink>,
    /// Receives `UiRequest` JSON.
    pub on_request: Option<Sink>,
    pub hotkeys: bool,
    pub control_socket: bool,
    /// How often to look for launched/terminated apps.
    pub app_poll: Duration,
    /// How long the displays must stay unchanged before a profile is chosen
    /// for them (`[when]`), so a flickering monitor does not flap profiles.
    pub auto_profile_settle: Duration,
}

impl Options {
    pub fn new(paths: Paths) -> Options {
        Options {
            paths,
            on_snapshot: None,
            on_request: None,
            hotkeys: true,
            control_socket: true,
            app_poll: Duration::from_secs(1),
            auto_profile_settle: Duration::from_secs(5),
        }
    }
}

enum Msg {
    Event(Event),
    Command {
        cmd: Command,
        reply: Option<Sender<Reply>>,
    },
    Displays(Vec<Display>),
    InitialApps(Vec<Pid>),
    AxMissing,
    AxTrusted,
    Shutdown(Sender<()>),
    Subscribe {
        kinds: Vec<EventKind>,
        tx: Sender<String>,
    },
}

/// Lines a subscriber may have queued before it counts as stuck and is
/// dropped.
const SUBSCRIBER_BUFFER: usize = 256;

/// A `pane subscribe` client.
struct Subscriber {
    kinds: Vec<EventKind>,
    tx: Sender<String>,
    /// The snapshot its events were last computed from.
    last: Snapshot,
}

/// Cheap, cloneable handle to a running core.
#[derive(Clone)]
pub struct Handle {
    tx: Sender<Msg>,
    platform: Arc<dyn Platform>,
    stopped: Arc<AtomicBool>,
}

impl Handle {
    /// Run a command and wait for its reply.
    pub fn command(&self, cmd: Command, timeout: Duration) -> Reply {
        let (rtx, rrx) = crossbeam_channel::bounded(1);
        if self
            .tx
            .send(Msg::Command {
                cmd,
                reply: Some(rtx),
            })
            .is_err()
        {
            return Reply::err("Pane is shutting down");
        }
        rrx.recv_timeout(timeout)
            .unwrap_or_else(|_| Reply::err("timed out"))
    }

    /// Fire-and-forget command (hotkeys).
    pub fn command_async(&self, cmd: Command) {
        let _ = self.tx.send(Msg::Command { cmd, reply: None });
    }

    /// Stream events of `kinds` (empty = all but `snapshot`) as JSON lines,
    /// starting with the current state. The channel closes when Pane stops,
    /// or when the receiver falls more than 256 lines behind.
    pub fn subscribe(&self, kinds: Vec<EventKind>) -> Receiver<String> {
        let (tx, rx) = crossbeam_channel::bounded(SUBSCRIBER_BUFFER);
        let _ = self.tx.send(Msg::Subscribe { kinds, tx });
        rx
    }

    pub fn send_event(&self, e: Event) {
        let _ = self.tx.send(Msg::Event(e));
    }

    /// Re-read the displays. Call on the main thread (uses NSScreen).
    pub fn screens_changed(&self) {
        let d = self.platform.displays();
        let _ = self.tx.send(Msg::Displays(d));
    }

    /// Unpark everything, persist, and stop. Bounded by `timeout`.
    pub fn shutdown(&self, timeout: Duration) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return true;
        }
        let (dtx, drx) = crossbeam_channel::bounded(1);
        if self.tx.send(Msg::Shutdown(dtx)).is_err() {
            return true;
        }
        drx.recv_timeout(timeout + Duration::from_millis(500))
            .is_ok()
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}

/// Send `sub` the events from `prev` to `next`. False when the subscriber
/// is gone or its buffer is full (a slow reader must not hold up the core).
fn send_events(sub: &Subscriber, prev: Option<&Snapshot>, next: &Snapshot) -> bool {
    stream::diff(prev, next, &sub.kinds).iter().all(|ev| {
        serde_json::to_string(ev)
            .map(|line| sub.tx.try_send(line).is_ok())
            .unwrap_or(true)
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

const RECONCILE_SPACING_MS: u64 = 16;
const SNAPSHOT_SPACING_MS: u64 = 50;
const PERSIST_DEBOUNCE_MS: u64 = 500;
const DISPLAY_DEBOUNCE_MS: u64 = 300;
const INITIAL_SCAN_MS: u64 = 3000;
const SHUTDOWN_MS: u64 = 3000;

fn install_panic_hook() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current().name().unwrap_or("?").to_string();
            tracing::error!(%thread, "panic: {info}");
            prev(info);
        }));
    });
}

/// Start the runtime. Call from the main thread: the initial display scan
/// uses NSScreen.
pub fn start(platform: Arc<dyn Platform>, opts: Options) -> Result<Handle, String> {
    install_panic_hook();
    let paths = opts.paths.clone();
    paths
        .ensure()
        .map_err(|e| format!("cannot create {}: {e}", paths.root.display()))?;

    let crashed = match lockfile::acquire(&paths.lock(), |p| platform.is_alive(p))
        .map_err(|e| e.to_string())?
    {
        LockOutcome::Busy { pid } => return Err(format!("Pane is already running (pid {pid})")),
        LockOutcome::Stale { pid } => {
            tracing::warn!(pid, "previous run did not exit cleanly");
            true
        }
        LockOutcome::Clean => false,
    };
    let leftovers = if crashed {
        persist::read_parked(&paths.parked())
    } else {
        vec![]
    };

    if !paths.config().exists() {
        let _ = persist::write_atomic(&paths.config(), STARTER_CONFIG.as_bytes());
    }
    let (config, config_err) =
        match std::fs::read_to_string(paths.config()).map(|t| Config::parse(&t)) {
            Ok(Ok(c)) => (c, None),
            Ok(Err(e)) => (Config::default(), Some(e)),
            Err(_) => (Config::default(), None),
        };
    let now = now_ms();
    let model = match persist::read_state(&paths.state()) {
        Some(s) => Model::from_state(config, &s, now),
        None => Model::new(config, now),
    };

    let (tx, rx) = unbounded::<Msg>();
    let (ev_tx, ev_rx) = unbounded::<Event>();
    let stopped = Arc::new(AtomicBool::new(false));
    let handle = Handle {
        tx: tx.clone(),
        platform: platform.clone(),
        stopped: stopped.clone(),
    };

    if !leftovers.is_empty() {
        let _ = tx.send(Msg::Event(Event::RestoreParked { entries: leftovers }));
    }
    if let Some(error) = config_err {
        let _ = tx.send(Msg::Event(Event::ConfigFailed { error }));
    }
    let _ = tx.send(Msg::Displays(platform.displays()));

    let hotkey_tx = if opts.hotkeys {
        let (ctx, crx) = unbounded::<Command>();
        let h = handle.clone();
        std::thread::Builder::new()
            .name("pane-hotkey-fwd".into())
            .spawn(move || {
                for cmd in crx {
                    h.command_async(cmd);
                }
            })
            .map_err(|e| e.to_string())?;
        Some(ctx)
    } else {
        None
    };

    let mut core = Core {
        m: model,
        platform: platform.clone(),
        paths: paths.clone(),
        on_snapshot: opts.on_snapshot.clone(),
        on_request: opts.on_request.clone(),
        subscribers: Vec::new(),
        workers: HashMap::new(),
        ev_tx,
        pending: HashMap::new(),
        next_token: 1,
        hotkey_tx,
        hotkeys: None,
        persist_due: None,
        snapshot_due: None,
        last_snapshot_hash: 0,
        last_snapshot_at: 0,
        displays_due: None,
        first_displays: true,
        initial: None,
        scanned_early: HashSet::new(),
        profiles: vec![],
        profile_whens: vec![],
        active_saved: None,
        auto_due: None,
        auto_settle_ms: opts.auto_profile_settle.as_millis() as u64,
        trusted: false,
        last_reconcile: 0,
        reconcile_due: true,
        shutdown: None,
        stopped: stopped.clone(),
    };
    std::thread::Builder::new()
        .name("pane-core".into())
        .spawn(move || {
            let r = catch_unwind(AssertUnwindSafe(|| core.run(rx, ev_rx)));
            if r.is_err() {
                // The core is gone: put every parked window back, then die.
                let n = core_platform_restore(&platform);
                tracing::error!(restored = n, "core thread crashed; aborting");
                std::process::abort();
            }
        })
        .map_err(|e| e.to_string())?;

    spawn_app_poller(handle.clone(), opts.app_poll)?;
    if opts.control_socket {
        if let Err(e) = crate::control::serve(&paths.socket(), handle.clone()) {
            tracing::warn!("control socket unavailable: {e}");
        }
    }
    Ok(handle)
}

fn core_platform_restore(p: &Arc<dyn Platform>) -> usize {
    p.emergency_restore()
}

fn spawn_app_poller(handle: Handle, every: Duration) -> Result<(), String> {
    std::thread::Builder::new()
        .name("pane-apps".into())
        .spawn(move || {
            let p = handle.platform.clone();
            if !p.ax_trusted(true) {
                let _ = handle.tx.send(Msg::AxMissing);
                while !p.ax_trusted(false) {
                    if handle.is_stopped() {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
            let _ = handle.tx.send(Msg::AxTrusted);
            let mut known: HashMap<Pid, ()> = HashMap::new();
            let mut first = true;
            while !handle.is_stopped() {
                let apps = p.running_apps();
                let now: HashSet<Pid> = apps.iter().map(|a| a.pid).collect();
                for a in &apps {
                    if known.insert(a.pid, ()).is_none() {
                        handle.send_event(Event::AppLaunched {
                            pid: a.pid,
                            bundle_id: a.bundle_id.clone(),
                            name: a.name.clone(),
                            regular: a.regular,
                        });
                    }
                }
                let gone: Vec<Pid> = known
                    .keys()
                    .copied()
                    .filter(|pid| !now.contains(pid) && !p.is_alive(*pid))
                    .collect();
                for pid in gone {
                    known.remove(&pid);
                    handle.send_event(Event::AppTerminated { pid });
                }
                if first {
                    let _ = handle
                        .tx
                        .send(Msg::InitialApps(apps.iter().map(|a| a.pid).collect()));
                    first = false;
                }
                std::thread::sleep(every);
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

struct Core {
    m: Model,
    platform: Arc<dyn Platform>,
    paths: Paths,
    on_snapshot: Option<Sink>,
    on_request: Option<Sink>,
    subscribers: Vec<Subscriber>,
    workers: HashMap<Pid, Box<dyn AppWorker>>,
    ev_tx: Sender<Event>,
    pending: HashMap<u64, Sender<Reply>>,
    next_token: u64,
    hotkey_tx: Option<Sender<Command>>,
    hotkeys: Option<Box<dyn HotkeyHandle>>,
    persist_due: Option<u64>,
    snapshot_due: Option<u64>,
    last_snapshot_hash: u64,
    last_snapshot_at: u64,
    displays_due: Option<(u64, Vec<Display>)>,
    first_displays: bool,
    /// Apps whose first enumeration we wait for before `InitialScanDone`.
    initial: Option<(HashSet<Pid>, u64)>,
    /// Apps whose first enumeration came before the app list did.
    scanned_early: HashSet<Pid>,
    profiles: Vec<String>,
    /// Each saved profile's `[when]`, for the snapshot and the menu.
    profile_whens: Vec<(String, ProfileCondition)>,
    /// The active profile's saved file, by name (`None` inside when it is
    /// not saved), for the drift dot. Reloaded when the name changes or a
    /// profile is saved or deleted.
    active_saved: Option<(String, Option<Profile>)>,
    /// When to choose a profile for the displays (after they settle).
    auto_due: Option<u64>,
    auto_settle_ms: u64,
    trusted: bool,
    last_reconcile: u64,
    reconcile_due: bool,
    shutdown: Option<(Sender<()>, u64)>,
    stopped: Arc<AtomicBool>,
}

impl Core {
    fn run(&mut self, rx: Receiver<Msg>, ev_rx: Receiver<Event>) {
        self.refresh_profiles();
        loop {
            let now = now_ms();
            let wake = self.next_wake(now);
            let timeout = Duration::from_millis(wake.saturating_sub(now).min(1000));
            select! {
                recv(rx) -> msg => match msg {
                    Ok(m) => self.on_msg(m),
                    Err(_) => return,
                },
                recv(ev_rx) -> ev => if let Ok(e) = ev { self.on_event(e) },
                default(timeout) => {},
            }
            for _ in 0..512 {
                let mut any = false;
                if let Ok(m) = rx.try_recv() {
                    self.on_msg(m);
                    any = true;
                }
                if let Ok(e) = ev_rx.try_recv() {
                    self.on_event(e);
                    any = true;
                }
                if !any {
                    break;
                }
            }
            self.timers();
            self.reconcile_if_due();
            if self.finish_shutdown_if_ready() {
                return;
            }
        }
    }

    fn next_wake(&self, now: u64) -> u64 {
        let mut t = now + 1000;
        let mut at = |x: u64| t = t.min(x);
        if let Some(d) = self.m.next_deadline() {
            at(d);
        }
        if let Some(d) = self.persist_due {
            at(d);
        }
        if let Some(d) = self.snapshot_due {
            at(d);
        }
        if let Some((d, _)) = &self.displays_due {
            at(*d);
        }
        if let Some((_, d)) = &self.initial {
            at(*d);
        }
        if let Some(d) = self.auto_due {
            at(d);
        }
        if self.reconcile_due {
            at(self.last_reconcile + RECONCILE_SPACING_MS);
        }
        if let Some((_, d)) = &self.shutdown {
            at(*d);
        }
        t
    }

    fn reduce(&mut self, e: Event) {
        let now = now_ms();
        // Pointer moves arrive many times a second and rarely change
        // anything: don't schedule a reconcile for each.
        let hover = matches!(
            &e,
            Event::Command {
                cmd: Command::Hover { .. } | Command::Drag { .. },
                ..
            }
        );
        let m = &mut self.m;
        match catch_unwind(AssertUnwindSafe(|| reduce(m, e, now))) {
            Ok(fx) if hover && fx.is_empty() && !self.m.layout_dirty => return,
            Ok(fx) => self.dispatch(fx),
            Err(_) => {
                // Resynchronise: forget what was sent so the next reconcile
                // re-establishes every frame.
                self.m.last_sent.clear();
                self.m.layout_dirty = true;
                self.request(UiRequest::warn(
                    "Internal error; see the log. Pane recovered.",
                ));
            }
        }
        self.reconcile_due = true;
    }

    fn on_event(&mut self, e: Event) {
        if let Event::WindowsDiscovered {
            pid, initial: true, ..
        } = &e
        {
            match &mut self.initial {
                Some((set, _)) => {
                    set.remove(pid);
                }
                // Before the app list arrives: don't wait for it again.
                None if !self.m.initial_scan_done => {
                    self.scanned_early.insert(*pid);
                }
                None => {}
            }
        }
        self.reduce(e);
    }

    fn on_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Event(e) => self.on_event(e),
            Msg::Command { cmd, reply } => self.command(cmd, reply),
            Msg::Displays(d) => {
                if self.first_displays {
                    self.first_displays = false;
                    self.reduce(Event::DisplaysChanged { displays: d });
                } else {
                    self.displays_due = Some((now_ms() + DISPLAY_DEBOUNCE_MS, d));
                }
            }
            Msg::InitialApps(pids) => {
                let early = std::mem::take(&mut self.scanned_early);
                let set: HashSet<Pid> = pids
                    .into_iter()
                    .filter(|p| self.workers.contains_key(p) && !early.contains(p))
                    .collect();
                self.initial = Some((set, now_ms() + INITIAL_SCAN_MS));
            }
            Msg::AxMissing => {
                self.trusted = false;
                self.request(UiRequest::AxPermissionMissing);
                self.snapshot_due = Some(now_ms());
            }
            Msg::AxTrusted => {
                self.trusted = true;
                self.start_hotkeys();
                self.snapshot_due = Some(now_ms());
            }
            Msg::Shutdown(done) => {
                tracing::info!("shutting down: restoring parked windows");
                self.reduce(Event::Shutdown);
                self.last_reconcile = 0;
                self.reconcile_if_due();
                self.shutdown = Some((done, now_ms() + SHUTDOWN_MS));
            }
            Msg::Subscribe { kinds, tx } => {
                let snap = self.snapshot();
                let sub = Subscriber {
                    kinds,
                    tx,
                    last: snap.clone(),
                };
                if send_events(&sub, None, &snap) {
                    self.subscribers.push(sub);
                }
            }
        }
    }

    fn start_hotkeys(&mut self) {
        if self.hotkeys.is_some() {
            return;
        }
        let Some(tx) = self.hotkey_tx.clone() else {
            return;
        };
        let bindings = self.m.config.bindings().unwrap_or_default();
        let gesture = self.m.config.gesture_mods().ok().flatten();
        match self.platform.start_hotkeys(&bindings, gesture, tx) {
            Ok(h) => {
                h.set_hover(self.m.config.settings.focus_follows_mouse);
                h.set_drag(self.m.config.settings.title_drag);
                h.set_swipes(self.m.config.settings.trackpad_swipes);
                self.hotkeys = Some(h);
            }
            Err(e) => {
                tracing::warn!("hotkeys: {e}");
                self.request(UiRequest::warn(format!("Hotkeys unavailable: {e}")));
            }
        }
    }

    fn timers(&mut self) {
        let now = now_ms();
        if self.m.next_deadline().is_some_and(|d| d <= now) {
            self.reduce(Event::Tick);
        }
        if self.displays_due.as_ref().is_some_and(|(d, _)| *d <= now) {
            let (_, displays) = self.displays_due.take().unwrap();
            tracing::info!(count = displays.len(), "displays changed");
            self.reduce(Event::DisplaysChanged { displays });
            // Before the first scan the scan's end chooses instead.
            if self.m.initial_scan_done {
                self.auto_due = Some(now + self.auto_settle_ms);
            }
        }
        if self.auto_due.is_some_and(|d| d <= now) {
            self.auto_due = None;
            self.auto_profile();
        }
        if let Some((set, deadline)) = &self.initial {
            if set.is_empty() || *deadline <= now {
                self.initial = None;
                self.reduce(Event::InitialScanDone);
                // A profile meant for these displays comes first. The
                // default profile applies only when none is active: an
                // active profile (kept in state.json) stays until the user
                // applies another one.
                let auto = self.auto_profile();
                let startup = self.m.config.settings.startup_profile.clone();
                if let Some(name) = startup.filter(|_| !auto && self.m.active_profile.is_none()) {
                    self.command(
                        Command::ProfileApply {
                            name: Some(name),
                            profile: None,
                        },
                        None,
                    );
                }
            }
        }
        if self.persist_due.is_some_and(|d| d <= now) {
            self.persist_due = None;
            self.persist();
        }
        if self.snapshot_due.is_some_and(|d| d <= now) {
            self.snapshot_due = None;
            self.emit_snapshot();
        }
    }

    fn reconcile_if_due(&mut self) {
        let now = now_ms();
        if !self.reconcile_due || now < self.last_reconcile + RECONCILE_SPACING_MS {
            return;
        }
        self.reconcile_due = false;
        self.last_reconcile = now;
        self.m.now_ms = now;
        let m = &mut self.m;
        match catch_unwind(AssertUnwindSafe(|| reconcile(m))) {
            Ok(fx) => self.dispatch(fx),
            Err(_) => {
                self.m.last_sent.clear();
                self.m.layout_dirty = true;
            }
        }
    }

    fn dispatch(&mut self, fx: Vec<Effect>) {
        for f in fx {
            match f {
                Effect::Observe { pid } => {
                    if !self.workers.contains_key(&pid) {
                        match self.platform.spawn_worker(pid, self.ev_tx.clone()) {
                            Some(w) => {
                                self.workers.insert(pid, w);
                            }
                            None => tracing::debug!(pid, "no worker for app"),
                        }
                    }
                }
                Effect::Unobserve { pid } => {
                    if let Some(w) = self.workers.remove(&pid) {
                        w.send(WorkerCmd::Quit);
                    }
                }
                Effect::SetFrame {
                    pid,
                    wid,
                    rect,
                    gen,
                    double_set,
                    restore_to,
                } => {
                    if let Some(w) = self.workers.get(&pid) {
                        w.send(WorkerCmd::SetFrame {
                            wid,
                            rect,
                            gen,
                            double_set,
                            restore_to,
                        });
                    }
                }
                Effect::FocusDesktop => self.platform.focus_desktop(),
                Effect::WarpPointer { rect } => self.platform.warp_pointer(rect),
                Effect::Raise { pid, wid } => {
                    if let Some(w) = self.workers.get(&pid) {
                        w.send(WorkerCmd::Raise { wid });
                    }
                }
                Effect::Focus { pid, wid } => {
                    if let Some(w) = self.workers.get(&pid) {
                        w.send(WorkerCmd::Focus { wid });
                    }
                }
                Effect::Restore { pid, wid } => {
                    if let Some(w) = self.workers.get(&pid) {
                        w.send(WorkerCmd::Restore { wid });
                    }
                }
                Effect::WriteParked { entries } => {
                    if let Err(e) = persist::write_parked(&self.paths.parked(), &entries) {
                        tracing::error!("writing parked.json: {e}");
                    }
                }
                Effect::PersistState => {
                    self.persist_due
                        .get_or_insert(now_ms() + PERSIST_DEBOUNCE_MS);
                }
                Effect::Snapshot => {
                    let at = (self.last_snapshot_at + SNAPSHOT_SPACING_MS).max(now_ms());
                    self.snapshot_due = Some(self.snapshot_due.map_or(at, |d| d.min(at)));
                }
                Effect::UiRequest(r) => self.request(r),
                Effect::Reply { token, reply } => {
                    if let Some(tx) = self.pending.remove(&token) {
                        let _ = tx.send(reply);
                    }
                }
                Effect::RecoverSweep { all_apps } => {
                    let displays: Vec<_> = self
                        .m
                        .displays
                        .iter()
                        .map(|d| (d.frame, d.visible_frame))
                        .collect();
                    let known: Vec<_> = self
                        .m
                        .parked
                        .values()
                        .map(|p| (p.wid, p.original))
                        .collect();
                    for w in self.workers.values() {
                        w.send(WorkerCmd::Recover {
                            displays: displays.clone(),
                            known: known.clone(),
                        });
                    }
                    if all_apps {
                        let p = self.platform.clone();
                        std::thread::spawn(move || {
                            let n = p.sweep_all(&known);
                            tracing::info!(n, "swept all apps");
                        });
                    }
                }
            }
        }
    }

    fn request(&self, r: UiRequest) {
        // Overlay updates follow the pointer: keep them out of the info log.
        if matches!(
            r,
            UiRequest::GestureOverlay { .. } | UiRequest::GestureOverlayHide
        ) {
            tracing::debug!("ui request: {r:?}");
        }
        match (&self.on_request, serde_json::to_string(&r)) {
            (Some(sink), Ok(json)) => sink(&json),
            (None, _) => match &r {
                UiRequest::Notice { message, .. } => tracing::info!("notice: {message}"),
                UiRequest::GestureOverlay { .. } | UiRequest::GestureOverlayHide => {}
                other => tracing::info!("ui request: {other:?}"),
            },
            _ => {}
        }
    }

    fn snapshot(&self) -> Snapshot {
        let mut s = pane_core::snapshot::build(&self.m);
        s.profiles = self.profiles.clone();
        s.desk_profiles = self
            .profile_whens
            .iter()
            .filter(|(_, c)| c.score(&self.m.displays, &self.m.config).is_some())
            .map(|(n, _)| n.clone())
            .collect();
        s.profile_drift = self.active_drifted();
        s.ax_trusted = self.trusted;
        s
    }

    /// Does the desk differ from the saved active profile? Not before the
    /// first scan, while windows are still being found.
    fn active_drifted(&self) -> bool {
        match (&self.active_saved, &self.m.active_profile) {
            (Some((n, Some(p))), Some(active)) if n == active && self.m.initial_scan_done => {
                let live = pane_core::reducer::capture(&self.m, n);
                drift::diff(p, &live, &self.m.displays, &self.m.config).drifted
            }
            _ => false,
        }
    }

    /// Load the active profile's saved file if the active name changed.
    fn sync_active_saved(&mut self) {
        let want = self.m.active_profile.as_ref();
        if self.active_saved.as_ref().map(|(n, _)| n) == want {
            return;
        }
        self.active_saved = want.map(|n| {
            let p = persist::read_profile(&self.paths.profile(n))
                .ok()
                .filter(|p| p.name == *n);
            (n.clone(), p)
        });
    }

    /// How the desk differs from saved profile `name` (default: the active one).
    fn profile_diff(&self, name: Option<&str>) -> Result<ProfileDiff, String> {
        let name = name
            .map(str::to_string)
            .or_else(|| self.m.active_profile.clone())
            .ok_or("no profile is active")?;
        let saved = persist::read_profile(&self.paths.profile(&name))
            .map_err(|_| format!("no profile named `{name}`"))?;
        let live = pane_core::reducer::capture(&self.m, &name);
        Ok(drift::diff(&saved, &live, &self.m.displays, &self.m.config))
    }

    /// Save profile `name` (default: the active one) as updated from the desk.
    fn profile_update(&mut self, name: Option<&str>) -> Reply {
        let d = match self.profile_diff(name) {
            Ok(d) => d,
            Err(e) => return Reply::err(e),
        };
        let r = persist::write_profile(&self.paths.profile(&d.name), &d.updated);
        self.refresh_profiles();
        match r {
            Ok(()) => Reply::data(&d.updated),
            Err(e) => Reply::err(e),
        }
    }

    fn emit_snapshot(&mut self) {
        self.last_snapshot_at = now_ms();
        if self.on_snapshot.is_none() && self.subscribers.is_empty() {
            return;
        }
        self.sync_active_saved();
        let snap = self.snapshot();
        if let Some(sink) = &self.on_snapshot {
            if let Ok(json) = serde_json::to_string(&snap) {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                json.hash(&mut h);
                let hash = h.finish();
                if hash != self.last_snapshot_hash {
                    self.last_snapshot_hash = hash;
                    sink(&json);
                }
            }
        }
        // Each subscriber diffs against what it last saw (it may have
        // joined between two snapshots). Gone or stuck ones are dropped.
        self.subscribers.retain_mut(|sub| {
            if sub.last == snap {
                return true;
            }
            let prev = std::mem::replace(&mut sub.last, snap.clone());
            send_events(sub, Some(&prev), &snap)
        });
    }

    fn persist(&mut self) {
        refresh_placements(&mut self.m);
        if let Err(e) = persist::write_state(&self.paths.state(), &self.m.to_state()) {
            tracing::error!("writing state.json: {e}");
        }
    }

    fn load_config(&mut self) -> Reply {
        match std::fs::read_to_string(self.paths.config()) {
            Ok(text) => match Config::parse(&text) {
                Ok(c) => {
                    let bindings = c.bindings().unwrap_or_default();
                    let gesture = c.gesture_mods().ok().flatten();
                    let hover = c.settings.focus_follows_mouse;
                    let drag = c.settings.title_drag;
                    let swipes = c.settings.trackpad_swipes;
                    self.reduce(Event::ConfigLoaded {
                        config: Box::new(c),
                    });
                    if let Some(h) = &self.hotkeys {
                        h.update(&bindings, gesture);
                        h.set_hover(hover);
                        h.set_drag(drag);
                        h.set_swipes(swipes);
                    }
                    Reply::ok()
                }
                Err(e) => {
                    let msg = e.render();
                    self.reduce(Event::ConfigFailed { error: e });
                    Reply::err(msg)
                }
            },
            Err(e) => Reply::err(format!("{}: {e}", self.paths.config().display())),
        }
    }

    /// Rewrite config.toml with `f`, refusing results that do not parse,
    /// then reload.
    fn edit_config(&mut self, f: impl FnOnce(&str) -> Result<String, String>) -> Reply {
        let text = std::fs::read_to_string(self.paths.config()).unwrap_or_default();
        let new = match f(&text) {
            Ok(t) => t,
            Err(e) => return Reply::err(e),
        };
        if let Err(e) = Config::parse(&new) {
            return Reply::err(e.render());
        }
        if let Err(e) = persist::write_atomic(&self.paths.config(), new.as_bytes()) {
            return Reply::err(e.to_string());
        }
        self.load_config()
    }

    fn set_setting(&mut self, key: &str, value: &serde_json::Value) -> Reply {
        let known =
            serde_json::to_value(pane_core::config::Settings::default()).unwrap_or_default();
        if known.get(key).is_none() {
            return Reply::err(format!("unknown setting `{key}`"));
        }
        let literal = match toml_literal(value) {
            Ok(l) => l,
            Err(e) => return Reply::err(e),
        };
        let key = key.to_string();
        self.edit_config(move |t| Ok(persist::set_setting(t, &key, literal.as_deref())))
    }

    fn rebind(&mut self, command: &str, chord: Option<&str>) -> Reply {
        let command = command.trim().to_string();
        let chord = chord.map(str::to_string);
        self.edit_config(move |text| {
            let cfg = Config::parse(text)
                .map_err(|e| format!("fix config.toml first: {}", e.render()))?;
            rebind_text(text, &cfg, &command, chord.as_deref())
        })
    }

    fn command(&mut self, cmd: Command, reply: Option<Sender<Reply>>) {
        let local = match cmd {
            Command::Reload => Some(self.load_config()),
            Command::ProfileList => Some(Reply::data(&self.profiles)),
            Command::Doctor => Some(Reply::data(&crate::doctor::Live::from_model(
                &self.m,
                self.platform.ax_trusted(false),
                self.platform.input_monitoring(),
                self.hotkey_tx.as_ref().map(|_| self.hotkeys.is_some()),
            ))),
            Command::ProfileWhen { ref name, here } => Some(self.profile_when(name, here)),
            Command::ProfileDiff { ref name } => Some(match self.profile_diff(name.as_deref()) {
                Ok(d) => Reply::data(&d),
                Err(e) => Reply::err(e),
            }),
            Command::ProfileUpdate { ref name } => Some(self.profile_update(name.as_deref())),
            // Re-capturing a saved profile keeps its `[when]`.
            Command::ProfileCapture { ref name } => {
                let name = name.as_deref().unwrap_or("Captured");
                let mut p = pane_core::reducer::capture(&self.m, name);
                p.when = persist::read_profile(&self.paths.profile(name))
                    .ok()
                    .and_then(|old| old.when);
                Some(Reply::data(&p))
            }
            Command::ProfileLoad { ref name } => {
                Some(match persist::read_profile(&self.paths.profile(name)) {
                    Ok(p) => Reply::data(&p),
                    Err(e) => Reply::err(e),
                })
            }
            Command::ProfileSave {
                ref profile,
                startup,
            } => {
                let r = persist::write_profile(&self.paths.profile(&profile.name), profile);
                self.refresh_profiles();
                Some(match r {
                    Ok(()) if startup => {
                        let text = std::fs::read_to_string(self.paths.config()).unwrap_or_default();
                        let edited = persist::set_startup_profile(&text, Some(&profile.name));
                        match persist::write_atomic(&self.paths.config(), edited.as_bytes()) {
                            Ok(()) => self.load_config(),
                            Err(e) => Reply::err(e.to_string()),
                        }
                    }
                    Ok(()) => Reply::ok(),
                    Err(e) => Reply::err(e),
                })
            }
            Command::ProfileDelete { ref name } => {
                let r = std::fs::remove_file(self.paths.profile(name));
                self.refresh_profiles();
                Some(
                    r.map(|_| Reply::ok())
                        .unwrap_or_else(|e| Reply::err(e.to_string())),
                )
            }
            Command::ProfileApply {
                name: Some(ref name),
                profile: None,
            } => match persist::read_profile(&self.paths.profile(name)) {
                Ok(p) => {
                    self.forward(
                        Command::ProfileApply {
                            name: None,
                            profile: Some(p),
                        },
                        reply,
                    );
                    return;
                }
                Err(e) => Some(Reply::err(e)),
            },
            Command::Query {
                what: QueryKind::Snapshot,
            } => {
                self.sync_active_saved();
                Some(Reply::data(&self.snapshot()))
            }
            // The core does not know the saved profiles; add them.
            Command::Search { ref query, .. } => {
                let cmd = Command::Search {
                    query: query.clone(),
                    profiles: self.profiles.clone(),
                };
                self.forward(cmd, reply);
                return;
            }
            Command::SetSetting { ref key, ref value } => Some(self.set_setting(key, value)),
            Command::SetScratchpads { ref scratchpads } => {
                let list = scratchpads.clone();
                Some(self.edit_config(move |t| Ok(persist::set_scratchpads(t, &list))))
            }
            Command::Rebind {
                ref command,
                ref chord,
            } => Some(self.rebind(command, chord.as_deref())),
            Command::ResetShortcuts => {
                Some(self.edit_config(|t| Ok(persist::edit_table(t, "shortcuts", |_| true, &[]))))
            }
            Command::SuspendHotkeys { suspended } => {
                if let Some(h) = &self.hotkeys {
                    h.set_suspended(suspended);
                }
                Some(Reply::ok())
            }
            _ => None,
        };
        match local {
            Some(r) => match reply {
                Some(tx) => {
                    let _ = tx.send(r);
                }
                None => {
                    if let Reply::Error { message } = r {
                        self.request(UiRequest::warn(message));
                    }
                }
            },
            None => self.forward(cmd, reply),
        }
    }

    /// Re-read the saved profiles' names and `[when]` blocks.
    fn refresh_profiles(&mut self) {
        let all = persist::read_profiles(&self.paths.profiles());
        self.profiles = all.iter().map(|p| p.name.clone()).collect();
        self.profile_whens = all
            .into_iter()
            .filter_map(|p| Some((p.name, p.when?)))
            .collect();
        self.active_saved = None;
        self.snapshot_due = Some(now_ms());
    }

    /// Choose the profile meant for the connected displays, once per desk:
    /// only when the display set differs from the one last acted on, so a
    /// profile the user picked by hand stays until the desk changes (also
    /// across restarts). Returns whether a profile is being applied.
    fn auto_profile(&mut self) -> bool {
        if !self.m.config.settings.auto_profiles || self.m.displays.is_empty() {
            return false;
        }
        let key = pane_core::profile::topology_key(&self.m.displays);
        if self.m.auto_topology.as_deref() == Some(key.as_str()) {
            return false;
        }
        self.m.auto_topology = Some(key);
        self.persist_due
            .get_or_insert(now_ms() + PERSIST_DEBOUNCE_MS);
        if self.profile_whens.is_empty() {
            return false;
        }
        let all = persist::read_profiles(&self.paths.profiles());
        let Some(p) = pane_core::profile::pick_for_displays(
            &all,
            &self.m.displays,
            &self.m.config,
            self.m.active_profile.as_deref(),
        ) else {
            tracing::info!("displays changed; no profile is meant for them");
            return false;
        };
        if self.m.active_profile.as_deref() == Some(p.name.as_str()) {
            return false;
        }
        tracing::info!(profile = %p.name, "displays changed; applying their profile");
        self.request(UiRequest::info(format!("Applied profile “{}”", p.name)));
        self.command(
            Command::ProfileApply {
                name: None,
                profile: Some(p.clone()),
            },
            None,
        );
        true
    }

    fn profile_when(&mut self, name: &str, here: bool) -> Reply {
        let path = self.paths.profile(name);
        let mut p = match persist::read_profile(&path) {
            Ok(p) => p,
            Err(_) => return Reply::err(format!("no profile named `{name}`")),
        };
        if here {
            if self.m.displays.is_empty() {
                return Reply::err("no displays connected");
            }
            p.when = Some(ProfileCondition::for_displays(&self.m.displays));
            // This desk now has its profile; don't choose again until it changes.
            self.m.auto_topology = Some(pane_core::profile::topology_key(&self.m.displays));
            self.persist_due
                .get_or_insert(now_ms() + PERSIST_DEBOUNCE_MS);
        } else {
            p.when = None;
        }
        let r = persist::write_profile(&path, &p);
        self.refresh_profiles();
        match r {
            Ok(()) => Reply::data(&p.when),
            Err(e) => Reply::err(e),
        }
    }

    fn forward(&mut self, cmd: Command, reply: Option<Sender<Reply>>) {
        let token = reply.map(|tx| {
            let t = self.next_token;
            self.next_token += 1;
            self.pending.insert(t, tx);
            t
        });
        self.reduce(Event::Command { cmd, token });
    }

    fn finish_shutdown_if_ready(&mut self) -> bool {
        let Some((_, deadline)) = &self.shutdown else {
            return false;
        };
        let outstanding = self.m.outstanding_sets();
        if outstanding > 0 && now_ms() < *deadline {
            return false;
        }
        if outstanding > 0 {
            tracing::warn!(
                outstanding,
                "shutdown deadline reached with frames unconfirmed"
            );
        }
        let (done, _) = self.shutdown.take().unwrap();
        self.persist();
        let _ = persist::write_parked(&self.paths.parked(), &[]);
        for (_, w) in self.workers.drain() {
            w.send(WorkerCmd::Quit);
        }
        let _ = std::fs::remove_file(self.paths.socket());
        lockfile::release(&self.paths.lock());
        self.stopped.store(true, Ordering::Release);
        let _ = done.send(());
        tracing::info!("clean exit");
        true
    }
}

/// JSON value from Settings -> TOML literal (`None` removes the key).
fn toml_literal(v: &serde_json::Value) -> Result<Option<String>, String> {
    use serde_json::Value as J;
    Ok(match v {
        J::Null => None,
        J::Bool(b) => Some(b.to_string()),
        J::Number(n) => Some(n.to_string()),
        J::String(s) => Some(persist::toml_string(s)),
        J::Array(items) => {
            let parts: Result<Vec<String>, String> = items
                .iter()
                .map(|i| match i {
                    J::String(s) => Ok(persist::toml_string(s)),
                    J::Number(n) => Ok(n.to_string()),
                    J::Bool(b) => Ok(b.to_string()),
                    _ => Err("only flat lists are supported".to_string()),
                })
                .collect();
            Some(format!("[{}]", parts?.join(", ")))
        }
        J::Object(_) => return Err("tables are not supported here".into()),
    })
}

/// Write a chord the way `[shortcuts]` keys are written: relative to the
/// modifier (`mod-shift-x`) when it contains it, so it follows later
/// modifier changes.
fn chord_key(c: &Chord, base: &Mods) -> String {
    let m = Mods {
        cmd: c.cmd,
        alt: c.alt,
        ctrl: c.ctrl,
        shift: c.shift,
    };
    if m.contains(*base) {
        let extra = Mods {
            cmd: m.cmd && !base.cmd,
            alt: m.alt && !base.alt,
            ctrl: m.ctrl && !base.ctrl,
            shift: m.shift && !base.shift,
        };
        let e = extra.display();
        if e.is_empty() {
            format!("mod-{}", c.key)
        } else {
            format!("mod-{e}-{}", c.key)
        }
    } else {
        c.display()
    }
}

/// New config text in which `command` is bound to exactly `chord` (or
/// nothing): its other overrides go, its default chords are set to "none",
/// and whatever held `chord` before is replaced.
pub fn rebind_text(
    text: &str,
    cfg: &Config,
    command: &str,
    chord: Option<&str>,
) -> Result<String, String> {
    let base = cfg.modifier().map_err(|e| e.render())?;
    let cmd = Command::parse(command)?;
    let new = chord.map(|c| Chord::parse_with(c, &base)).transpose()?;
    let defaults: Vec<Chord> = cfg
        .default_bindings(&base)
        .into_iter()
        .filter(|(_, t)| Command::parse(t).ok().as_ref() == Some(&cmd))
        .filter_map(|(c, _)| Chord::parse_with(&c, &base).ok())
        .collect();
    let mut remove: HashSet<String> = HashSet::new();
    let mut kept_on: Vec<Chord> = Vec::new();
    for (k, v) in &cfg.shortcuts {
        let kc = Chord::parse_with(k, &base).ok();
        let is_none = v.trim() == "none";
        let binds_cmd = !is_none && Command::parse(v).ok().as_ref() == Some(&cmd);
        let on_new = new.is_some() && kc == new;
        let unbinds_default = is_none && kc.as_ref().is_some_and(|c| defaults.contains(c));
        if binds_cmd || on_new || unbinds_default {
            remove.insert(k.clone());
        } else if let Some(c) = kc {
            kept_on.push(c);
        }
    }
    let mut lines: Vec<String> = Vec::new();
    for d in &defaults {
        if Some(d) != new.as_ref() && !kept_on.contains(d) {
            lines.push(format!(
                "{} = \"none\"",
                persist::toml_string(&chord_key(d, &base))
            ));
        }
    }
    if let Some(c) = &new {
        lines.push(format!(
            "{} = {}",
            persist::toml_string(&chord_key(c, &base)),
            persist::toml_string(command)
        ));
    }
    Ok(persist::edit_table(
        text,
        "shortcuts",
        |k| remove.contains(k),
        &lines,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pane_core::config::STARTER_CONFIG;

    fn chord_of(cfg: &Config, cmd: &str) -> Vec<String> {
        cfg.binding_table()
            .unwrap()
            .into_iter()
            .find(|r| r.command == cmd)
            .unwrap()
            .chords
    }

    #[test]
    fn rebind_moves_and_clears() {
        let c = Config::parse(STARTER_CONFIG).unwrap();
        let t = rebind_text(STARTER_CONFIG, &c, "focus left", Some("cmd-alt-y")).unwrap();
        let c = Config::parse(&t).unwrap();
        assert_eq!(chord_of(&c, "focus left"), vec!["cmd-alt-y"]);
        assert!(t.contains("\"mod-h\" = \"none\""), "{t}");
        assert!(t.contains("\"mod-cmd-y\" = \"focus left\""), "{t}");
        // Rebinding again replaces, does not pile up.
        let t2 = rebind_text(&t, &c, "focus left", Some("alt-h")).unwrap();
        let c2 = Config::parse(&t2).unwrap();
        assert_eq!(chord_of(&c2, "focus left"), vec!["alt-h"]);
        assert!(c2.shortcuts.len() <= 1, "{:?}", c2.shortcuts);
        // Clearing leaves it unbound.
        let t3 = rebind_text(&t2, &c2, "focus left", None).unwrap();
        assert!(chord_of(&Config::parse(&t3).unwrap(), "focus left").is_empty());
        // Overrides written with `mod` follow a modifier change.
        let t4 = persist::set_setting(&t, "modifier", Some("\"ctrl-alt\""));
        assert_eq!(
            chord_of(&Config::parse(&t4).unwrap(), "focus left"),
            vec!["cmd-ctrl-alt-y"]
        );
    }

    #[test]
    fn rebinding_a_scratchpad_unbinds_its_key() {
        let text = format!("{STARTER_CONFIG}\n[[scratchpad]]\napp = \"x\"\nkey = \"mod-grave\"\n");
        // The scratchpad table comes after [shortcuts]; edit_table must cope.
        let c = Config::parse(&text).unwrap();
        assert_eq!(chord_of(&c, "scratchpad x"), vec!["alt-grave"]);
        let t = rebind_text(&text, &c, "scratchpad x", Some("alt-t")).unwrap();
        let c = Config::parse(&t).unwrap();
        assert_eq!(chord_of(&c, "scratchpad x"), vec!["alt-t"]);
    }

    #[test]
    fn stealing_a_chord_replaces_its_owner() {
        let c = Config::parse(STARTER_CONFIG).unwrap();
        let t = rebind_text(STARTER_CONFIG, &c, "recover", Some("alt-h")).unwrap();
        let c = Config::parse(&t).unwrap();
        assert_eq!(chord_of(&c, "recover"), vec!["alt-h"]);
        assert!(chord_of(&c, "focus left").is_empty());
    }

    #[test]
    fn literals() {
        assert_eq!(
            toml_literal(&serde_json::json!(6)).unwrap().as_deref(),
            Some("6")
        );
        assert_eq!(
            toml_literal(&serde_json::json!("a\"b")).unwrap().as_deref(),
            Some("\"a\\\"b\"")
        );
        assert_eq!(
            toml_literal(&serde_json::json!(["x"])).unwrap().as_deref(),
            Some("[\"x\"]")
        );
        assert_eq!(toml_literal(&serde_json::Value::Null).unwrap(), None);
    }
}
