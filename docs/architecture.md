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
| `pane-cli` | `pane` | any, with offline recovery on macOS |

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

The core thread never touches AX. A hung app blocks only its own worker. The core marks an app unresponsive after 2 s without a reply and leaves its windows alone until the app emits an event again.

## Key mechanisms

- **Tree** (`tree.rs`): every workspace's root lives in one arena. Inner containers always have two or more children, and containers that end up with the same orientation as their parent are flattened. `move_leaf` implements i3 semantics: swap with a sibling, enter a sibling container, pop out of a nested container, or wrap the root. A result of `AtEdge` makes the reducer move the window to the neighbouring display.
- **Layout** (`layout.rs`): weights are split along each container's axis, raised to learned minimum sizes, and the difference is taken from neighbours. When minimum sizes cannot fit, the newest window floats with a notice instead of being fought over.
- **Reconcile** (`reconcile.rs`): computes desired frames for all windows (visible tiles, floats, park rects for hidden workspaces) and sends only what changed by more than 0.5 pt. Retries use backoff of 50, 150, 400, 1000 and 2500 ms. A window that keeps refusing becomes "stubborn" and floats. A window that comes back larger than asked records a minimum size and triggers a relayout. `WriteParked` always precedes the frames that park.
- **Assignment** (`assign.rs`): total and deterministic. Workspaces keep their display; a workspace evicted by a disconnect remembers its home and returns when the home reconnects; each display re-shows the workspace it last showed.
- **Identity** (`identity.rs`): records keep a fingerprint per window (document, title, normalized title, ordinal, size). A relaunching app's windows are matched globally and greedily (document 100, exact title 80, normalized title 55, ordinal 20, size 10; accepted at 55, or at 20 for a lone window and record). When an app quits, its tiled windows stay in the tree as **ghost leaves** that take no space. A matched window takes its ghost's exact place and weight. Ghosts are pruned after 10 minutes or when you rearrange that workspace.
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
