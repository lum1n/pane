# Gestures: move and tile windows by holding the modifier and pointing

## Context

Today a window can only be moved between tiles with `mod-shift-h/j/k/l`, one i3 step at a time, and a tiled window dragged by its title bar snaps back after 300 ms. The user wants a pointer gesture:

- **Hold the Pane modifier** (`[settings] modifier`, Option by default). No click is needed.
- **Move the mouse** to a region. While you move, an **overlay on screen shows exactly where the window will go and how it will tile.**
- **Let go of the modifier** to drop the window there.

Drop regions are tile-relative and work across displays. The window that moves is the one under the pointer when the modifier went down.

| Pointer over | Result | Overlay label |
|---|---|---|
| Centre of another tile (middle 40%) | Swap the two windows | "Swap with Safari" |
| Edge band of another tile (nearest edge outside the centre) | Split that tile and insert on that side | "Left of Mail" |
| Outer band of a display's tiling area (48 pt) | Full-height column or full-width row on that side (root edge) | "Left edge" |
| A display whose shown workspace has no tiles | Fill that display | "Fill Studio Display" |
| Own tile, or nothing valid | No-op; the overlay is hidden | — |

More behaviour:
- A floating window you drop becomes tiled at the target.
- A zoomed (fullscreen) tile on the source or target workspace is un-zoomed when you drop.
- **Escape** cancels (Escape is swallowed while a gesture is active).
- Pane has no animations, so the overlay jumps from one frame to the next.

### Telling a gesture apart from hotkeys and normal Option use
The gesture uses the hotkey modifier, so a small state machine in the tap thread decides when it is a gesture:
- **Armed** when the held modifiers exactly equal the gesture mods (the four bits shift/ctrl/alt/cmd; caps lock is ignored). It records where the pointer was.
- **Active** once the pointer moves more than 24 pt with no key or mouse button pressed. Only then is `Begin` sent.
- **Blocked** until the modifier is released, if any key is pressed (a hotkey or an Option character, such as `@` on a Norwegian layout) or a mouse button goes down (Option-click, Option-drag in Finder). Those events pass through untouched.
- **Active, then a key:** Escape cancels and is swallowed. Any other key cancels and passes through, so hotkeys still work.
- **Active, then a mouse button:** cancels and passes through.
- **Active, then a gesture modifier is released:** `Drop`. Adding extra modifiers is ignored.
- **Robustness:** every mouse-moved event carries the modifier flags. If they show the modifier is no longer held (a lost flagsChanged), the gesture drops. If the tap is re-enabled after a timeout, the tracker resets (sends Cancel).
- **No window under the start point:** the core ignores `Begin`, so holding Option while hovering a menu does nothing.

## Settings
In `crates/pane-core/src/config.rs` `Settings`:
- `gestures: bool` (default `true`)
- `gesture_modifier: String` (default `""`, meaning the same as `modifier`; otherwise parsed with `Mods::parse`)

Validate both in the same place `modifier()` is validated (`config.rs:726`). Add both to the commented default config text (`config.rs:~766`), to `docs/config.md`, and to Settings > Hotkeys as a "Move windows by holding the modifier and pointing" toggle (`SettingsView.swift`, using the existing `set-setting` command).

## Implementation

### 1. Pure gesture tracker: `crates/pane-core/src/gesture.rs` (new)
- `GestureTracker::new(mods: Option<u8>)` and `set_mods`.
- Input methods: `on_flags(mods, at)`, `on_key_down(is_escape) -> KeyVerdict {Pass, Swallow}`, `on_mouse_down()`, `on_mouse_moved(mods, at)`, `reset()`.
- Each input returns `Option<GesturePhase>`.
- `GesturePhase { Begin{at}, Move{at}, Drop{at}, Cancel }`, with Serde and `at: Point` in global top-left coordinates (the same as AX and the model).
- `Move` is sent only after the pointer has moved 4 pt or more since the last one.
- Pure code, so it is unit tested on Linux.

### 2. Drop targets and the tree operation: `crates/pane-core/src/drop.rs` (new)
- **Types:**
  - `DropTarget { Swap{other: WindowId}, Split{leaf: NodeId, dir: Direction}, DisplayEdge{display, dir}, Fill{display} }`
  - `Source { Leaf(NodeId), New(Leaf) }`. `New` is for a floating window.
- **`fn target_at(m: &Model, wid, at) -> Option<DropTarget>`:**
  - Finds the display under `at` (reuse `display_for_rect`-style lookup).
  - Its tiling area (`m.tiling_area`) decides the outer band.
  - It then finds which present tile contains `at`, from a layout pass over that display's visible workspace. Use `layout::compute` with the same `LayoutInput` as `reconcile::refresh_desired`. Factor out a small helper so both share it.
  - It returns `None` over the window's own tile.
- **`fn apply(tree: &mut Tree, root_of_target: NodeId, src: Source, target: &DropTarget) -> NodeId`:**
  - Pure tree work. It returns the window's new leaf.
  - Swap uses `Tree::swap_leaves` (`tree.rs:566`).
  - Split removes the source leaf and calls `insert_leaf(.., InsertAt::Split{target, dir})` (`tree.rs:284`/`313`).
  - DisplayEdge uses `InsertAt::Split{target: root, dir}`, which `insert_split` already handles for roots by wrapping the root's children.
  - Fill uses `InsertAt::End`.
- **`fn preview(m: &Model, wid, target) -> Option<Rect>`:**
  - Clones `m.tree` (`Tree` is `Clone`) and runs `apply` on the clone.
  - Runs `layout::compute` for the target workspace and returns the new leaf's rect.
  - The overlay therefore shows the exact frame the window will get, including gaps, weights and minimum sizes.

### 3. Protocol and reducer
- **Protocol** (`proto.rs`):
  - Add `Command::Gesture { phase: GesturePhase }`. It is not a query, and `Command::parse` does not support it, because the tap sends JSON-free values. The CLI can still send it as raw JSON for testing.
  - Add `UiRequest::GestureOverlay { frame: Rect, target: Option<Rect>, label: String }` and `UiRequest::GestureOverlayHide`.
- **Model** (`model.rs`): `gesture: Option<Gesture { wid, target: Option<DropTarget> }>`. It is runtime-only and not persisted.
- **Reducer** (`reducer.rs`), new `cmd_gesture`:
  - **Begin:** hit-test the visible windows at `at`. Check floats of the visible workspace first, the focused one first, then tiles. If nothing is found, do nothing; otherwise store the gesture.
  - **Move:** recompute `target_at`. Only when it changes, send `GestureOverlay` (with the `preview` rect, the target tile's rect for splits, and the label) or `GestureOverlayHide`.
  - **Drop:**
    - Re-check that the window still exists, then do the model bookkeeping.
    - Detach the window with `detach_window(m, wid, false)` (`reducer.rs:822`). For a cross-workspace Swap, update both windows' `workspace` instead.
    - Then `drop::apply` on the real tree, set `mode = Tiled`, set the workspace and clear the zoom.
    - Then focus the window (`focus_fx`), set `m.layout_dirty = true`, and push `Effect::Snapshot` and `Effect::PersistState`. Follow `cmd_move_to_workspace` (`reducer.rs:1649`) for the cross-workspace bookkeeping.
    - Finally `GestureOverlayHide`.
  - **Cancel:** clear the gesture and hide the overlay.
  - **Shutdown, `DisplaysChanged`, and the window being destroyed:** these cancel a running gesture.
- **Runtime** (`runtime.rs:662` `request`): log `GestureOverlay*` at debug, not info, so moves don't flood `make logs`.

### 4. Event tap (`crates/pane-macos/src/hotkeys.rs`, `ffi.rs`)
- **Second tap on the existing `pane-hotkeys` thread:** a **listen-only** tap (`kCGEventTapOptionListenOnly`) for mouse-moved (5), left/right/other mouse-down (1, 3, 25) and flagsChanged (12). Being listen-only means it can never add cursor lag.
- **Keyboard tap:** stays as it is (active, so it can swallow keys).
- **Shared state:** both taps share the `TapState` (same run loop thread), which now holds the `GestureTracker` behind a `Mutex`.
- **keyDown:** the hotkey lookup runs first. The tracker's verdict is then applied: swallow Escape when the gesture is active.
- **Re-enable:** re-enable logic covers both taps (two stored ports). The tracker resets on a timeout.
- **FFI additions:** `CGEventGetLocation` (returns `CGPoint`) and the new event-type and option constants.
- **Phases:** sent on the existing `Sender<Command>` as `Command::Gesture`.
- **Wiring the gesture mods:** pass them through `Platform::start_hotkeys` and `HotkeyHandle::update` (`crates/pane-agent/src/platform.rs`) as `gesture: Option<Mods>`. `None` means disabled. The runtime computes it from config in `start_hotkeys` and `load_config` (`runtime.rs:491`, `712`). `NullPlatform` ignores it.

### 5. Swift overlay (`shell/Sources/PaneShell/Overlay/GestureOverlay.swift`, new)
- **Panel:** one borderless, non-activating `NSPanel`. Style it like `NoticeHUD.makePanel` (`NoticeHUD.swift:51`), with `ignoresMouseEvents`, `canJoinAllSpaces`, `.stationary`, no shadow, and level `.statusBar`, so it sits above normal windows.
- **Content view:**
  - The **preview frame** as a rounded rect filled with `controlAccentColor` at 25% alpha, with a 2 pt accent border.
  - A small HUD pill with the label, centred in the preview.
  - For splits, a second, faint dashed outline of the target tile, so you can see which tile is being split.
- **Panel frame:** the union of the preview and the target. The view draws both in local coordinates.
- **Coordinates:** global top-left becomes Cocoa with `y = NSScreen.screens[0].frame.maxY - (y + h)`.
- **Requests:** `AppDelegate.handle(request:)` (`AppDelegate.swift:44`) handles `gesture_overlay` and `gesture_overlay_hide`.

### 6. Docs
- README "Use": a paragraph on the gesture next to the edge-drag resize text.
- `docs/config.md`: the two settings.
- `docs/architecture.md`: one line in the data flow (tap → `Command::Gesture` → core → `UiRequest::GestureOverlay` → Swift).
- Lapp: tick off the M7 trackpad/drag item in Follow-ups and note the menu-hover risk below.

## Verification
- **Unit tests (`gesture.rs`):**
  - The arm, activate and drop sequence.
  - A key while armed blocks; a mouse-down blocks; a key while active cancels; Escape is swallowed.
  - Flags lost on a mouse move drop the gesture; extra modifiers are ignored; nothing is sent below the 24 pt threshold.
- **Unit tests (`drop.rs`):**
  - The centre and four edge bands of a tile, the display outer band, an empty display, and `None` over the window's own tile.
  - `apply` for each target, including a root with a different orientation.
- **Scenarios** (`crates/pane-core/tests/scenarios.rs`, using `Sim` and `displays(&["C","R"])`):
  - Swap within a workspace.
  - Split left/right/top/bottom.
  - A display-edge column.
  - A move to the other display (empty, and with tiles).
  - A float dropped and tiled.
  - Cancel changes nothing.
  - A drop on a zoomed workspace un-zooms it.
  - **Invariant:** the `preview` rect sent in `GestureOverlay` equals the window's frame after the drop and `reconcile`.
- **Commands:** `make test` and `make check-mac` (type-checks the tap code for `aarch64-apple-darwin`).
- **On the Mac (`make app && make run`), the user checks:**
  - With Option held, pointing at a tile shows the overlay for its centre and each edge, across displays, and releasing moves the window.
  - `mod-h` and other hotkeys, Option-typed characters (`@`), Option-click and Option-drag in Finder all still behave normally and start no gesture.
  - Escape cancels.
  - Turning the setting off disables the gesture.

## Known risk
Holding Option over an open menu while moving more than 24 pt, with a managed window under the start point, arms a gesture. The overlay makes it visible, and Escape or a drop on the window's own tile does nothing. If this proves annoying, set `gesture_modifier` to e.g. `ctrl-alt`.
