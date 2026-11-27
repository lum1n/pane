# Architecture

## One process, two languages

`Pane.app` is a Swift (AppKit + SwiftUI) shell that links the Rust static library `libpane_ffi.a`.

- **Rust** owns windows, layout, workspaces, parking, persistence, hotkeys and the CLI socket.
- **Swift** owns only what needs `NSApplication`: the status item and its icon, menus, the profile editor, settings, the permission explanation, `NSWorkspace` and `NSScreen` notifications, and launching apps for profiles.

The boundary is a C ABI with JSON payloads (`crates/pane-ffi/include/pane.h`). The same `proto::Command` / `proto::Reply` types serve the CLI socket.

`pane-agentd` is the same runtime without the UI. It is useful for debugging, and it keeps a future split into a separate agent process cheap.

## Crates

| Crate | Role | Platform |
|---|---|---|
| `pane-core` | Tree, layout, assignment, identity, rules, config, profiles, model, reducer, reconcile, snapshot, protocol | any (pure) |
| `pane-macos` | `AxElement`, observers, per-app workers, displays, CGEventTap hotkeys, recovery mirror | macOS only (empty elsewhere) |
| `pane-agent` | Core thread, worker registry, persistence, `run.lock`, control socket, `pane-agentd` | any, via the `Platform` trait |
| `pane-ffi` | `pane_start`, `pane_command`, `pane_event`, `pane_shutdown`, `pane_free` | staticlib |
| `pane-cli` | `pane` | any, with offline recovery on macOS and an offline `pane doctor` |

## Data flow

```
 workers (one thread per app, own CFRunLoop + AXObserver) ──Event──┐
 app poller (CGWindowList, 1 s) / NSWorkspace (Swift) ──Event──────┤
 hotkeys (CGEventTap thread) / socket / FFI ──Command──────────────┤
 displays (NSScreen on main) ──Displays (debounced 300 ms)─────────┤
                                                                   ▼
                          core thread: reduce(&mut Model, Event) -> Vec<Effect>
                                       reconcile(&mut Model)    -> Vec<Effect>  (≤ every 16 ms)
                                                                   │
   SetFrame/Raise/Focus -> worker   WriteParked -> parked.json (sync)   PersistState -> state.json (500 ms)
   Snapshot -> Swift icon (on change, ≤ 20 Hz)   UiRequest -> Swift   Reply -> waiting client
```

`pane subscribe` hangs off the snapshot path. Each socket client that subscribes registers a bounded channel (256 lines) with the core. Whenever a snapshot is due, the core diffs it against the last one that client saw (`stream::diff`) and `try_send`s the resulting event lines; a client that is gone or full is dropped, so a slow reader never holds up the core. Snapshots are built for subscribers even when there is no Swift sink (headless `pane-agentd`). Control connections set `SO_NOSIGPIPE`, since inside the app nothing ignores SIGPIPE and a write to a closed subscriber would otherwise kill Pane.

Gestures take the same path: a listen-only tap on the hotkey thread watches the pointer and modifier keys, `gesture::GestureTracker` turns them into `Command::Gesture` phases, and the core answers with `UiRequest::GestureOverlay` (the frame the window will get, from `drop::preview`) for Swift to draw. A plain title-bar drag reuses that overlay: the same tap sends `Command::Drag` (`DragPhase::Move` every 4 pt while the left button is held, `Release` on mouse-up; gated by `title_drag`). The core keeps the last pointer in `Model.pointer`. When a visible tiled window then moves without changing size and the pointer is fresh (500 ms), `frame_changed` starts a `Gesture { plain: true }`, holds the window's snap-back, and updates the overlay as the pointer moves. `Release` drops it with `drop_window` (recorded for undo), or sends it back to its tile when there is no target. With no pointer news for 5 s, `tick` gives up and snaps it back, and the resync pass leaves the held window alone. Without pointer events the old 300 ms snap-back applies.

The core thread never touches AX. A hung app blocks only its own worker. The core marks an app unresponsive after 2 s without a reply and leaves its windows alone until the app emits an event again.

## Key mechanisms

- **Tree** (`tree.rs`): every workspace's root lives in one arena. Inner containers always have two or more children, and containers that end up with the same orientation as their parent are flattened. `move_leaf` implements i3 semantics: swap with a sibling, enter a sibling container, pop out of a nested container, or wrap the root. A result of `AtEdge` makes the reducer move the window to the neighbouring display.
- **Layout presets** (`Workspace.layout`, `reducer::keep_layouts`): a workspace may keep a `LayoutMode` (master-stack with its ratio, or N columns). `reduce` counts the tiles of those workspaces before and after every event; when the count changed, the tree is rebuilt in that shape from its leaves in visual order (`Tree::arrange_master_stack`, `Tree::arrange_columns`), so ghost leaves and slots keep their places in the order. The master's width from before the event is kept, so a resize survives. Manual rearranging stays until the next change in count. Applying a profile to the workspace clears its preset.
- **Layout** (`layout.rs`): weights are split along each container's axis, raised to learned minimum sizes, and the difference is taken from neighbours. When minimum sizes cannot fit, the newest window floats with a notice instead of being fought over.
- **Reconcile** (`reconcile.rs`): computes desired frames for all windows (visible tiles, floats, park rects for hidden workspaces) and sends only what changed by more than 0.5 pt. Retries use backoff of 50, 150, 400, 1000 and 2500 ms. A window that keeps refusing becomes "stubborn" and floats. A window that comes back larger than asked is checked again after at least 500 ms; if it still has the same size, that is recorded as its minimum and triggers a relayout (a single read-back may be a late resize). Tiling a floating window by hand forgets the recorded minimum. `WriteParked` always precedes the frames that park.
- **Workspaces** (`workspace.rs`, `assign.rs`): every workspace belongs to a home display (its id is `<display UUID>/<name>`). Assignment is total and deterministic:
  - A workspace is on its home display when that display is connected.
  - Otherwise it is a hidden guest on the remaining display nearest to where its home last was.
  - A display that appears where a missing one was, and owns nothing, adopts that display's workspaces.
  - Every display re-shows the workspace it last showed.
- **Focus following**: Pane shows a hidden window's workspace only on an *app activation* (Cmd-Tab, a Dock click, clicking into the app), and never within 1 s of a switch. Plain focus-change notices, which apps also send late for windows Pane just hid, are ignored, so a switch is never undone. An empty workspace gives the desktop keyboard focus, and minimized windows are never focused or raised (that would restore them).
- **Pointer and focus** (`mouse_follows_focus`, `focus_follows_mouse`): `reduce` compares the focused window and display before and after every non-query command except `Gesture` and `Hover`; when they change it emits `Effect::WarpPointer` with the window's desired frame (laid out first if needed) or the display's visible frame, and the platform warps unless the pointer is already inside. For focus follows mouse the pointer tap sends `Command::Hover` every 4 pt of travel (never while the gesture modifier is held); the reducer keeps `Model.hover` = (window under the pointer, now + 150 ms), and `tick` focuses it when due. Any keyboard command clears a pending hover. The runtime skips the reconcile for hovers that change nothing.
- **Watchdog**: every 2 s, any window of a shown workspace that is found off screen is sent to its tile again. Unhiding a window is retried until it lands; it never gives up and never marks the window "stubborn".
- **Logging**: workspace switches, hiding and showing windows, ignored focus reports and watchdog repairs are logged at info level (`make logs`).
- **Parking corner**: a hidden window is parked at the bottom corner (right, else left) that overlaps the other displays least, so it does not appear on a neighbouring display.
- **Identity** (`identity.rs`): records keep a fingerprint per window (document, title, normalized title, ordinal, size). A relaunching app's windows are matched globally and greedily (document 100, exact title 80, normalized title 55, ordinal 20, size 10; accepted at 55, or at 20 for a lone window and record). When an app quits, its tiled windows stay in the tree as **ghost leaves** that take no space. A matched window takes its ghost's exact place and weight. Ghosts are pruned after 10 minutes or when you rearrange that workspace. A window opened later in an app that is already running (it has other windows, or launched more than 10 s ago), such as a Settings window or a new browser profile window, never goes back to a remembered place or an idle profile slot on a hidden workspace: it opens on the workspace you are on, because nothing would ever show the hidden one (the app is already frontmost, so there is no activation to follow).
- **Active profile**: applying a profile makes it active until the user applies another; this is kept in `state.json`, and the default (`startup_profile`) applies at launch only when none is active. Profile slots are never deleted. A slot whose app is not running takes no space; the app's next window, whenever it opens, fills it. A window that filled a slot gives the slot back when it closes or its app quits, and after a restart those windows come back as slots, so window ids may change.
- **Repair**: before every layout pass, `reconcile::repair` enforces the invariants. Every window is in an existing workspace; a tiled window has exactly one leaf, in its own workspace; a floating window is in its own workspace's floating list. A window can never end up hidden in a workspace nobody can reach. Workspaces are only created for displays Pane knows, and placement records naming an unknown display fall back to the window's own display.
- **Profiles that follow the displays**: the runtime (which owns the profile files) chooses a profile when the display set settles, 5 s after the last debounced change, and at the end of the startup scan. `profile::pick_for_displays` scores each `[when]` against the connected displays. `Model.auto_topology` (persisted) records the display set it last acted on, so it chooses only when that set changes, and a profile picked by hand survives until then.
- **Profile drift** (`drift.rs`): the runtime keeps the active profile's saved file (reloaded when the active name changes or a profile is saved or deleted) and, for each snapshot, diffs it against `reducer::capture` of the desk: entries are matched by resolved display and workspace name, slots by bundle id, and weights compared as shares with a tolerance. `Snapshot.profile_drift` drives the dot on the menu bar icon. `ProfileDiff.updated` is the profile "Update from Current Desk" saves.
- **Undo** (`undo.rs`): `Model.undo` is a ring of the last 20 layouts (every workspace's tree, floating list and focus, each window's workspace, mode, float frame and slot, the visible workspaces and the active profile; runtime only). Tree-changing commands and committed edge drags push the layout from before them, only when they changed it. `undo` restores the newest one into the live tree, skipping windows that are gone and appending windows opened since, and reconcile moves the windows. A layout that would change nothing is dropped and the next one is tried.
- **Profiles** (`profile.rs` plus `reducer::apply_profile`): editor operations run on the same `Tree` code. Applying a profile replaces the workspace trees with slots, then matches running windows to slots: same workspace and title first, then relaxed. Each unmatched slot asks Swift to launch its app; the slot keeps its space for 30 s and claims that app's next window. Nothing is ever closed.

## Recovery guarantees

| Situation | What happens |
|---|---|
| Clean quit, SIGTERM/INT/HUP, `applicationWillTerminate` | Reconcile with `shutting_down` sends every parked window to its saved frame. The runtime waits up to 3 s for confirmations, then writes state, clears `parked.json` and removes `run.lock`. |
| Panic in the reducer | Caught per event; the core forgets `last_sent` and re-establishes every frame. |
| Panic elsewhere on the core thread | The AX mirror (`pane-macos::recovery`) restores parked windows with raw AX calls and 500 ms timeouts, then the process aborts. |
| `kill -9` / power loss | The next start sees a stale `run.lock`, reads `parked.json`, restores matching windows and shows "Restored N windows". |
| Anything else | **Restore All Windows** / `pane recover --all-apps`. The CLI works without the app, using `parked.json` plus a sweep of every display's park corner. |

## Deviations from the plan

- The CLI and agent sit in `Contents/Helpers/` rather than `Contents/MacOS/`, because `pane` and `Pane` collide on case-insensitive APFS.
- `pane.h` is written by hand and kept in sync; `make header` regenerates it with cbindgen if it is installed. This avoids a cbindgen build dependency.
- Apps are discovered by polling `CGWindowList` every second (NSWorkspace notifications in the app make it immediate). This works the same in the app and in the headless agent.
- With no `[[workspace]]` in the config, workspaces 1-3 prefer the left display, 4-6 the centre and 7-9 the right.
