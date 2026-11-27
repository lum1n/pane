# Plain title-bar drag drops a tiled window like a gesture (Proposal #3 remainder)

## Context
The Lapp todo "Proposal #3 remainder" in project `pane` is still open. Dragging a tiled window by its title bar, with no modifier held, snaps it back 300 ms after the last `AXWindowMoved` (`reducer.rs` `frame_changed`, `SNAP_BACK_MS`). The modifier-hold gesture already has drop targets, a preview and an overlay (`drop::target_at`, `drop::preview`, `drop_window`, `UiRequest::GestureOverlay`). This change reuses all of that so a plain title-bar drag shows the same overlay and swaps, splits or moves the window on release. The other open todo, trackpad swipes, needs private MultitouchSupport APIs and can't be tested here, so it is skipped.

The core can't see the mouse during a drag today. The listen-only pointer tap in `crates/pane-macos/src/hotkeys.rs` doesn't subscribe to `LeftMouseDragged` or `LeftMouseUp`, and the 300 ms quiet time ends the drag early whenever the user pauses.

## Approach

### 1. Pointer input (pane-macos)
- `ffi.rs`: add `kCGEventLeftMouseUp = 2` and `kCGEventLeftMouseDragged = 6`.
- `hotkeys.rs`: add both to `mouse_mask`. In `mouse_cb`:
  - `LeftMouseDragged`: send `Command::Drag { phase: DragPhase::Move { at } }`, throttled to every 4 pt like `hover`, using a new `last_drag: Cell<Option<Point>>` on `TapState`.
  - `LeftMouseUp`: send `Command::Drag { phase: DragPhase::Release { at } }` and reset `last_drag`.
  - Sending is gated by a new `drag: Arc<AtomicBool>` with a `Hotkeys::set_drag(bool)` method, mirroring `set_hover`.
- `crates/pane-agent/src/platform.rs` and `runtime.rs`: add `set_drag` to the `Platform` trait, with a no-op in the fake platform. The runtime calls it where it calls `set_hover`, from the new setting. Add `Drag` to the runtime fast path that skips dispatch for no-op `Hover` commands.

### 2. Protocol and setting (pane-core)
- `proto.rs`: add `Command::Drag { phase: DragPhase }` (JSON only, like `Gesture` and `Hover`), with `DragPhase { Move { at: Point }, Release { at: Point } }`. Add a wire-format test.
- `config.rs`: add `[settings] title_drag: bool`, default `true`. It works without the `gestures` setting. `docs/config.md` documents it, and Settings > General gets a toggle (Swift, `SettingsView.swift`, checked with `swiftc -parse` only).

### 3. Reducer (pane-core `reducer.rs`, `model.rs`)
- Model: give `Gesture` a field `plain: bool`, true for a title-bar drag. Add `m.pointer: Option<(Point, u64)>`, the last drag pointer and when it arrived. Because the drag lives in `m.gesture`, the existing hover suppression and overlay code keep working.
- `cmd_drag(Move{at})`: store `m.pointer`. If `m.gesture` is a plain drag, call `gesture_move(at)`. The overlay appears when the target changes, as it does now.
- `frame_changed`, tiled branch: when `title_drag` is on, the window is visible and not zoomed, the size is within `DRAG_THRESHOLD` of `last_sent` and the origin moved by more than `DRAG_THRESHOLD`, it is a move. If `m.pointer` is recent (≤ 500 ms) and no gesture is running:
  - set `m.gesture = Some(Gesture { wid, target: None, plain: true })` and call `gesture_move(pointer)`;
  - hold the snap-back by keeping `s.settled = false` with `next_retry_ms = None`. Release decides the outcome, and a long fallback (see below) covers a lost mouse-up.

  If the pointer is unknown (no tap or Input Monitoring), keep today's 300 ms snap-back.
- `cmd_drag(Release{at})` while a plain gesture runs:
  1. take the gesture and emit `GestureOverlayHide`;
  2. if `target_at(m, wid, at)` is `Some`, wrap `drop_window` in `undo::capture` / `undo::record`, as `tick` does around `commit_drag`;
  3. if it is `None` (the window's own tile, or nowhere), set `next_retry_ms = Some(now)` so the window snaps back right away.
- Safety net: with a plain gesture active, `tick` cancels it and snaps the window back once no `Drag` event has arrived for 2 s. This covers a lost mouse-up and the tap being disabled.
- Cancel the plain drag (hide the overlay, snap back) when the window closes, floats or is minimized. `cancel_gesture` already handles closes for gestures; check that it also covers the plain case.
- A modifier `Gesture Begin` while a plain drag is active is ignored. That can't really happen anyway, because mouse-down blocks the tracker.
- Floating windows are unchanged: a dragged float keeps its new position. Plain drag targets apply to tiled windows only.

### 4. Tests (`crates/pane-core/tests/scenarios.rs`)
Add a helper `Sim::title_drag(wid, from, to)`. It sends `Drag Move` and `WindowFrameChanged` frames together (same size, moving origin), then `Drag Release`, following the `drag_to` and `gesture` helpers. New tests:
- `title_drag_swaps_tiles`: the overlay appears and the frames end up swapped.
- `title_drag_splits_and_moves_to_another_display`.
- `title_drag_released_over_own_tile_snaps_back`.
- `title_drag_pause_does_not_snap_back`: tick 400 ms mid-drag and expect no `SetFrame`.
- `title_drag_without_pointer_keeps_old_snap_back`: the existing `dragged_tiled_window_snaps_back_after_the_drag` must still pass.
- `title_drag_is_undoable`.
- `title_drag_lost_mouse_up_snaps_back_after_timeout`.
- `title_drag_off_setting_snaps_back`.

The edge-resize tests (`dragging_a_tile_edge_resizes_against_its_neighbour_only` and the others) must keep passing.

### 5. Docs
- `README.md`: add plain title-bar drag to the gesture section next to the drop table, and mention it in the undo line.
- `docs/architecture.md`: pointer flow (`LeftMouseDragged`/`Up` → `Command::Drag`).
- `docs/config.md`: `title_drag`.
- `docs/verification.md`: a Mac checklist item for title-bar drag (swap, split, other display, pause, release over own tile, undo).

## Critical files
`crates/pane-macos/src/{ffi.rs,hotkeys.rs}`, `crates/pane-agent/src/{platform.rs,runtime.rs}`, `crates/pane-core/src/{proto.rs,config.rs,model.rs,reducer.rs}`, `crates/pane-core/tests/scenarios.rs`, `shell/Sources/PaneShell/SettingsView.swift`, docs. Reused unchanged: `drop.rs` (`target_at`, `preview`), `gesture_move`, `drop_window`, `undo::{capture,record}`, and the Swift `GestureOverlay`.

## Verification
- `scripts/ci-linux.sh` runs fmt, clippy, tests and the aarch64-apple-darwin check.
- `swiftc -parse` on the changed Swift.
- The behaviour on a real Mac stays unverified. The new checklist item in `docs/verification.md` covers it, and the PR says so.

## Delivery
Branch `title-drag-drop` off the current worktree branch (main plus b250466), commit, push and open a PR with `gh pr create` (body via the `pr` skill). Then tick the todo in Lapp, update the Follow-ups note if anything is left, and `append_log`.
