# Pane: macOS tiling window manager, implementation plan

## Context

Pane is a new tiling window manager for macOS (14+, Apple silicon), inspired by Kit Langton's Spaceman (github.com/anomalyco/spaceman, Rust core + ObjC shim, MIT) but with better window handling. The repo is empty. No animations, blazing fast, rock solid, three displays as the primary test case, menu bar icon that draws the live setup, and graphical saving of whole-desk "profiles" (which app in which pane on which screen).

Decisions already taken with the user:
- Window-first split tree (i3/Spaceman style: containers with H/V orientation, windows as leaves, weights).
- Emulated per-display workspaces: hidden windows are parked off-screen (AeroSpace/Spaceman approach). No native Spaces, no SIP changes.
- Rust core + Swift/ObjC shell. Public Accessibility API plus the one private call `_AXUIElementGetWindow`. Not App Store eligible, same as both predecessors.
- Hotkeys in v1, trackpad gestures in a later phase.
- Code is written on Linux; build/run/verification is done by the user on their Mac with 3 displays.

What "better than Spaceman" means (user-selected):
1. Stable window identity across relaunch/reboot (Spaceman only restores workspace+float per bundle id; same-size windows can swap).
2. Never fight apps: per-app AX worker threads (AeroSpace #131 lesson), retries with backoff, min-size-aware layout.
3. Predictable new-window placement via rules; dialogs/sheets/pickers auto-float.
4. Deterministic workspace reassignment when displays disconnect/reconnect; parked windows always recoverable.

Research notes are in Lapp: "Research: Spaceman and AeroSpace landscape".

## Architecture summary

- **One process, two languages.** `Pane.app` is a Swift (AppKit + SwiftUI) shell linking a Rust static library. Rust owns windows, layout, workspaces, parking, persistence, hotkeys, CLI socket. Swift owns only what needs `NSApplication`: status item + icon, menus, profile editor, settings, permission prompt, `NSWorkspace`/`NSScreen` notifications, `NSWorkspace.openApplication`.
- **FFI = C ABI with JSON payloads** (serde on Rust side, Codable on Swift side), defined once in `pane-core::proto` and shared with the CLI socket protocol.
- **Headless `pane-agentd` binary** from M0 so milestones M0-M4 are testable without Swift. Also keeps a future split into a separate agent process cheap.
- **`pane-core` is pure** (no macOS deps): all tree/layout/assignment/identity/profile logic is unit-tested on Linux.

Why Swift shell + in-process Rust rather than objc2-only UI or a separate agent process: SwiftUI drag/drop and canvas are far less code than objc2 AppKit; one TCC Accessibility identity; synchronous snapshot access for the icon. Crash risk is mitigated by panic hook + recovery file + `catch_unwind` around the reducer.

## Repo layout

```
Cargo.toml                 workspace: crates/*
rust-toolchain.toml
Makefile                   app | agent | test | check-mac | install | run | clean
scripts/build-app.sh       cargo + swift build + bundle + codesign
scripts/make-dev-cert.sh   self-signed "Pane Dev" codesign identity (stable TCC trust)
scripts/ci-linux.sh        cargo test + cargo check --target aarch64-apple-darwin
crates/
  pane-core/   geometry, tree, layout, workspace, display, assign, identity, rules,
               config, profile, state, model, event, effect, reducer, reconcile,
               snapshot, proto; tests/ (3-display scenarios, relaunch matching)
  pane-macos/  ax (AXUIElement wrapper), ax_private (_AXUIElementGetWindow), observer,
               app_worker (per-app thread), display (CGDisplay + reconfigure cb),
               hotkeys (CGEventTap), window_probe (classify windows)
  pane-agent/  runtime (core thread), workers (pid -> AppWorker), parking, recovery,
               persist, control (unix socket), bin/pane-agentd.rs
  pane-ffi/    staticlib, extern "C", cbindgen -> include/pane.h
  pane-cli/    `pane` binary: pane ws 3 | pane query tree | pane recover ...
shell/         Swift package (Mac only)
  Package.swift
  Sources/CPane/{module.modulemap, include/pane.h}
  Sources/PaneShell/ main, AppDelegate, Permissions, Bridge/{PaneCore,SystemEvents},
                     StatusItem/{StatusIconRenderer,StatusMenu},
                     Editor/{ProfileEditorWindow,DeskCanvasView,TileTreeView,AppPaletteView,DragDrop},
                     Settings/SettingsView
  Resources/Info.plist, AppIcon.icns
docs/architecture.md, docs/config.md
```

Crate deps: `pane-core`: serde, serde_json, toml, slotmap, regex, thiserror, smol_str. `pane-macos`: core-foundation, core-graphics, objc2 + objc2-foundation + objc2-app-kit (NSRunningApplication, NSScreen.visibleFrame only), libc, crossbeam-channel; hand-written `extern "C"` for the dozen AX functions used instead of a half-maintained accessibility crate. `pane-agent`: tracing, tracing-appender, signal-hook. `pane-ffi`: cbindgen. `pane-cli`: clap.

### FFI surface (C ABI, UTF-8 JSON, Rust-allocated strings freed via `pane_free`)

Swift -> Rust: `pane_start(support_dir, on_snapshot, on_request, ctx)`, `pane_command(json) -> json` (sync `proto::Command` -> `proto::Reply`, same as CLI), `pane_event(json)` (`AppLaunched`, `AppTerminated`, `ScreensChanged`, `WillSleep`, `DidWake`, `AppOpenResult`), `pane_shutdown()` (bounded 3s: unpark all, flush), `pane_free`.

Rust -> Swift callbacks (core thread, Swift hops to main): `on_snapshot(json)` (`proto::Snapshot`, emitted only when its hash changes, max ~20Hz), `on_request(json)` (`LaunchApp{request_id,bundle_id}`, `Notice`, `OpenEditor`, `AxPermissionMissing`).

## Core domain model (`pane-core`)

- `DisplayId` = CGDisplay UUID string. `Display { id, name, frame, visible_frame, is_main, scale, position_rank }`. `DisplayRef = Main | Uuid | Name | Position(Left|Center|Right) | Any`.
- `Workspace { id: WorkspaceId, prefs: Vec<DisplayRef>, display, root: NodeId, floating, focused, last_display }`.
- Tree in a `SlotMap<NodeId, Node>`; `Node = Container{parent, orientation, layout: Tiles|Accordion, children, weights} | Leaf::Window{window} | Leaf::Slot{AppSlot}` (slots used only by profiles so editor and live tree share one implementation).
- `WindowId(CGWindowID)`, `WindowInfo { id, pid, bundle_id, title, role_class, resizable, min_size, actual, ordinal, document, state: Tiled|Floating|Parked|Stubborn }`.
- `Model` = displays, workspaces, visible-per-display, tree, windows, apps, placements, config, focus, last_sent frames, pending_slots.
- Pure functions: `tree::insert_leaf/remove_leaf/move_leaf/swap/set_orientation/resize(relative)`, `layout::compute(tree, root, area, gaps, min_sizes) -> Vec<(WindowId, Rect)>` (clamps to min sizes, redistributes; marks overflow so the reducer floats the newest offender with a notice instead of fighting), `layout::neighbor(dir)`.

### Window identity across relaunch

`WindowFingerprint { bundle_id, title, title_norm, document, ordinal, size }`, `PlacementRecord { fp, workspace, anchor: TreeAnchor{path}, weight, floating, seen_at }`.
Scoring: document equal 100, title exact 80, normalized title 55, ordinal 20, size within 10% 10, recency tie-break. Accept at >= 55, or >= 20 when one candidate and one window. When an app launches N windows at once, do global greedy assignment over all (window, record) pairs so titles do not cross-match. Unmatched windows fall to rules and still get a record. Records expire after `placement_memory_days` (30). Apply by inserting at `anchor.path` if it still exists, else append to root.

### Rules and auto-float

`Rule { app: Glob, title: Regex, action: Tile|Float|Ignore, workspace, display }`. Auto-float when role is not Standard, not resizable, modal, tiny, no close button, or owner is not a regular-activation app. Precedence: explicit ignore > explicit float/tile > auto-float > default tile.

### Profiles

`Profile { name, version, displays: [ProfileDisplay { display: DisplayRef, workspace, tree: SlotTree }] }`, `AppSlot { bundle_id, title: Option<Regex>, launch_args }`. `profile::capture(model)` and `profile::plan_apply(model, profile) -> ApplyPlan` are pure. Stored as `profiles/<name>.toml`.

### Persisted files (`~/Library/Application Support/Pane/`)

| File | Content | When |
|---|---|---|
| `config.toml` | user config | user edits; reload via hotkey/CLI/file watcher |
| `state.json` | workspaces + trees + weights, visible-per-display, placements, focus | debounced 500ms, atomic rename |
| `parked.json` | parked windows with original frames | synchronously before each park batch |
| `run.lock` | pid + start time | on start; removed on clean exit |
| `profiles/*.toml` | profiles | editor save |
| `logs/pane.log` | tracing, rotated | continuous |

## Runtime

Threads:
- **Core thread** owns `Model`, drains an `Event` channel, runs `reducer::reduce(&mut Model, Event) -> Vec<Effect>`, dispatches effects. Never touches AX. `catch_unwind` per event.
- **Per-app worker** (one thread per pid, own CFRunLoop): owns that app's AX elements and `AXObserver`; command queue `SetFrame{wid,rect,gen} | Raise | Focus | Query | Probe | Quit`; replies `FrameApplied{wid,gen,ok,actual}`. `AXUIElementSetMessagingTimeout` 1s. Core marks an app `Unresponsive` after 2s without reply and stops sending it frames until it emits an event. A hung app blocks only its own thread.
- **Hotkey thread**: `CGEventTap` session tap, keyDown, swallows matched chords, re-enables on timeout disable. Display reconfigure callback registered on the same run loop, debounced 300ms.
- **Control thread**: unix socket JSON lines.
- **Swift main**: NSWorkspace launch/terminate, screen-parameter change, sleep/wake -> `pane_event`.

Events: AppLaunched/Terminated, WindowCreated/Destroyed/Moved/Resized, TitleChanged, FocusChanged, WindowMinimized, FrameApplied, AppUnresponsive/Responsive, DisplaysChanged, Hotkey, Command, Tick, SleepWake, Shutdown. Effects: Observe/Unobserve, SetFrame, Raise, Focus, WriteParked, PersistState, Snapshot, UiRequest, Reply.

Observer notifications per app element: WindowCreated, UIElementDestroyed, WindowMoved, WindowResized, TitleChanged, FocusedWindowChanged, Miniaturized/Deminiaturized, ApplicationActivated/Shown. On `Observe{pid}` the worker enumerates `AXWindows`, probes each (window id, role/subrole, settable size, minimized, title, document) and emits `WindowCreated`.

### Reconcile loop

Runs after draining the channel, at most once per 16ms. Computes desired frames for every window: visible workspace per display via `layout::compute` on visible_frame minus outer gap; floats keep their frame; other workspaces' windows -> park rect. Emits `SetFrame` only where desired differs from `last_sent` by > 0.5pt, never for Stubborn/Unresponsive. Feedback: if `actual.size` exceeds desired by > 2pt, record `min_size` and relayout (neighbor absorbs). Otherwise retry with backoff [50,150,400,1000,2500] ms, generation-tagged; after that mark Stubborn, float, notify. Chromium quirk: set-size, set-position, set-size when the first attempt lands off by > 1pt. Ignore `WindowMoved` matching our own last set frame within 1pt/150ms. v1 snaps user-dragged tiled windows back; drag-to-swap later.

### Parking

Park rect origin = `(display.frame.max_x - 1, display.frame.max_y - 1)`, size unchanged, on the window's own display (so separate-Spaces mode does not teleport it). Not minimized. Dock clamping: read back actual, treat any origin within 4pt of the target as parked. Order: `WriteParked` (fsync + rename) then `SetFrame`s.

### Recovery guarantees

- Clean quit / SIGTERM / SIGINT / SIGHUP / `applicationWillTerminate`: unpark all (parallel across workers, bounded 3s), persist, clear `parked.json`, remove `run.lock`.
- Panic: `set_hook` runs `recovery::emergency_restore()` from a mirror `Arc<Mutex<Vec<(AxHandle, Rect)>>>` with raw AX calls and 500ms timeouts, then aborts.
- Previous run died (stale `run.lock` + non-empty `parked.json`): on start restore by CGWindowID if pid alive, else by bundle_id + title, else leave to the identity matcher. Notice "Restored N windows".
- Menu item and `pane recover [--all-apps]` pull any window at a park corner back on screen.

## Display handling

- `assign(displays, workspaces, prev) -> Assignment` is pure and total: each workspace takes the first resolvable `DisplayRef` in its prefs, else Main; sticky unless a higher-priority pref display (re)appears; every display always shows exactly one workspace (auto-create if none). Unit tests for {L,C,R}, {C}, {L,C}, {C,R}, {L,R}, every reconnect order, and a replaced display UUID in the same position.
- Disconnect: workspaces from the vanished display move to fallback with `last_display` remembered, their windows re-parked there; the fallback display's visible workspace does not change. Reconnect: they return, and the previously visible workspace on that display (persisted per DisplayId) is shown again. Second reconcile pass 1s later. Sleep: persist; wake: treat as DisplaysChanged.
- Coordinates: core uses global top-left points only (AX and CGDisplayBounds). Convert NSScreen (bottom-left) in `pane-macos::display`: `top = primary.height - (y + h)`. Use NSScreen only for visibleFrame insets; match to CGDisplay via `NSScreenNumber`.
- Config `[[display]]` aliases match by `uuid`, `match_name`, or `position = left|center|right`.

## Menu bar icon

`StatusIconRenderer.render(snapshot) -> NSImage` template, 18pt high, width ~`10 * displays + 2`. Scale the union of display frames into the box, draw each display as a 0.5pt rounded rect, fill the visible workspace's tiles (normalized rects computed by Rust) at 50% alpha, focused tile 100%, focused display gets a 1pt bottom bar, small dots for non-visible workspaces with windows. Rust emits snapshots only on hash change; Swift coalesces 50ms on main. Menu: per display -> workspace list with checkmarks; Profiles submenu (apply/save/edit); Restore all windows; Reload config; Open editor; Settings; Quit.

## Profile editor (SwiftUI)

Left: app palette (running apps + /Applications scan, icons, search). Center: `DeskCanvasView` draws the real display arrangement from the snapshot; each display shows its `SlotTree` as nested rounded rects with app icon + name, workspace picker per display. Right: inspector (title regex, weight, orientation, layout). Drag palette -> tile with 5 drop zones (center replace, N/S/E/W split, i3 style); tile -> tile swap; dividers adjust weights; context menu remove/orientation/accordion. All tree edits go through `pane_command(ProfileEdit)` so there is one tree implementation; undo = stack of Profile values. Buttons: Capture current, Apply, Save, Save as startup.

Apply semantics (Rust): set each display's visible workspace and replace its tree with slots; match running windows to slots (bundle id + title regex + same workspace, greedy global); unmatched slots -> `UiRequest::LaunchApp` -> `NSWorkspace.openApplication`, slot becomes `PendingSlot` (30s deadline) that claims the next `WindowCreated` of that bundle, bypassing rules; unclaimed windows in that workspace are appended normally; nothing is ever closed. Optional startup profile applies after the first full scan.

## Hotkeys and config

Defaults (Option-based): `alt-h/j/k/l` focus (crosses displays, including empty ones); `alt-shift-h/j/k/l` move window; `alt-ctrl-h/j/k/l` resize +/- 5%; `alt-1..9` show workspace; `alt-shift-1..9` move window to workspace; `alt-,`/`alt-.` focus prev/next display; `alt-shift-,`/`.` move workspace to prev/next display; `alt-s` toggle orientation; `alt-a` tiles/accordion; `alt-f` float; `alt-return` swap workspaces between focused and next display; `alt-shift-r` reload; `alt-shift-p` profile picker.

`config.toml` v1, strict (`deny_unknown_fields`), versioned:

```toml
version = 1
[settings]
inner_gap = 6
outer_gap = 6
default_layout = "tiles"
placement_memory_days = 30
restore_placements = true
return_workspaces_on_reconnect = true
startup_profile = "work"
electron_bundle_ids = ["com.microsoft.VSCode"]

[[display]]
name = "left"
position = "left"          # or uuid = "..." / match_name = "DELL U2720Q"

[[workspace]]
name = "1"
display = ["left", "main"]

[[rule]]
app = "com.apple.finder"   # glob on bundle id
title = "^Copy"            # regex
action = "float"           # float | tile | ignore
workspace = "3"
display = "right"

[shortcuts]
"alt-h" = "focus left"
```

Parse errors carry line/column and surface as a Notice and in Settings. v1 Settings UI only shows config path, errors, Open in editor, Reload.

## Build and dev loop

- `make app` -> `scripts/build-app.sh`: `cargo build --release --target aarch64-apple-darwin -p pane-ffi -p pane-cli -p pane-agent` (cbindgen writes `pane.h`), `swift build -c release --package-path shell` (system-library target `CPane`, linker flags for `libpane_ffi.a`, AppKit, ApplicationServices; no Xcode project), assemble `build/Pane.app` (Info.plist: `com.vegard.pane`, `LSUIElement`, `NSAccessibilityUsageDescription`, `LSMinimumSystemVersion 14.0`; MacOS/ contains Pane, pane, pane-agentd), `codesign --options runtime` nested then bundle.
- Signing: ad-hoc changes cdhash per build and TCC re-prompts. `scripts/make-dev-cert.sh` creates a self-signed "Pane Dev" identity once; `CODESIGN_IDENTITY` defaults to it. Fallback: `tccutil reset Accessibility com.vegard.pane`.
- `make install`, `make run` (kill + relaunch), `make agent` (headless only).
- On Linux: `cargo test -p pane-core -p pane-cli` fully; `cargo check --target aarch64-apple-darwin --workspace` after `rustup target add aarch64-apple-darwin` (check does not link; verify in M0; gate any crate needing `xcrun` behind `cfg(target_os = "macos")`). Swift shell cannot be built on Linux. CI: ubuntu job runs `scripts/ci-linux.sh`; `macos-14` job runs `make app` + `cargo test --workspace`.

## Milestones and verification (user runs on the Mac, 3 displays)

**M0 Skeleton**: workspace compiles; core geometry/tree/layout + tests; AX wrapper + `_AXUIElementGetWindow`; `pane-agentd` prompts for AX, enumerates and logs windows; socket + `pane query windows|displays`; empty Swift status item; `make app` yields signed app.
Check: build succeeds; AX prompt once, not again after rebuild; `pane query windows` lists every window on all 3 displays with correct display and frame; `pane query displays` shows 3 UUIDs with top-left frames matching System Settings.

**M1 Tiling, one display**: workers, observers, reducer, reconcile, hotkeys, auto-float, rules, config.
Check: 4 Terminal windows tile; close one relayouts; `alt-s` toggles; resize steps; Get Info and Save dialogs float; VS Code and Slack placed on first try or after a logged retry; `kill -STOP` an app: others keep tiling, app marked unresponsive; `kill -CONT` recovers; System Settings (large min size) causes no retry storm.

**M2 Workspaces, parking, recovery**.
Check: `alt-2` hides workspace 1 (1px sliver bottom-right only), `alt-1` restores exact frames; menu quit restores all; `kill -9`: next start restores all with notice; `kill -TERM` restores before exit; Mission Control shows parked windows on their own display.

**M3 Three displays**: assign, position refs, reconfigure debounce, cross-display focus/move, move/swap workspace.
Check: workspaces 1-3 left, 4-6 center, 7-9 right per config; unplug right: 7-9 hidden on center, center's visible workspace unchanged, nothing lost; replug: 7-9 return with the previously visible one shown; repeat with left; unplug two; sleep/wake; `alt-l` from center's rightmost tile focuses the empty right display; `alt-shift-.` moves a workspace; `alt-return` swaps.

**M4 Persistence and identity**.
Check: 6 windows in nested splits across 3 displays; quit and relaunch Pane: identical; quit Safari (2 windows, distinct titles) and relaunch it: both return to old tiles and weights; reboot: layout restored for auto-relaunched apps; new unknown windows follow rules.

**M5 Menu bar icon**.
Check: 3 boxes matching physical arrangement; updates within 100ms; focused tile and display highlighted; legible in dark and light; idle CPU < 0.5%.

**M6 Profiles editor + apply**.
Check: Capture shows all 3 displays correctly; drag Mail onto a tile edge, save "work"; quit Mail, apply: Mail launches into its tile within 30s; apply with everything running: no launches, nothing closed; missing app gives a notice; startup profile applies at launch.

**M7 Later**: trackpad gestures (CGEventTap gesture events or private MultitouchSupport), drag-to-swap outline, shortcut editor GUI, per-display gaps.

## Risks and open questions

- AX thread-safety is undocumented; AeroSpace ships per-app threads successfully. All AX calls for one app stay on one thread. Fallback: single AX thread with per-app timeouts behind the same worker API.
- `_AXUIElementGetWindow` is private. Fallback: match AX windows to `CGWindowListCopyWindowInfo` by pid + frame + title.
- Parking artifacts (1px sliver, Mission Control thumbnails, Dock clamping) vary by macOS build; test on the user's exact version. Recommend "Displays have separate Spaces" ON.
- Native fullscreen windows and other native Spaces are invisible to AX; documented limitation.
- Self-signed cert keeping TCC trust across rebuilds must be verified in M0.
- CGEventTap may need Input Monitoring on some setups; fallback Carbon `RegisterEventHotKey` from Swift.
- Browser windows change titles constantly; identity relies on ordinal + size; consider per-app `identity = "ordinal"` config later.
- `AXEnhancedUserInterface` affects Electron apps globally; keep off by default, use the double-set workaround.
- Open for the user: numbered vs named workspaces (plan supports both), whether workspaces of an unplugged display stay hotkey-reachable (plan: yes), whether a startup profile launches apps automatically at login (plan: ask once).
