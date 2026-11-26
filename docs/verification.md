# Verifying on the Mac (3 displays)

Run these after `make run`, with `make logs` open in a terminal. `pane` refers to `build/Pane.app/Contents/Helpers/pane`.

## M0 Skeleton
- [ ] `make app` succeeds; `codesign -dv build/Pane.app` shows the "Pane Dev" authority.
- [ ] The Accessibility prompt appears once, and not again after `make app && make run`.
- [ ] `pane query displays` shows 3 UUIDs with top-left frames matching System Settings > Displays.
- [ ] `pane query windows` lists every window on all 3 displays, each with the right display and frame.

## M1 Tiling on one display
- [ ] 4 Terminal windows tile; closing one relayouts; `alt-s` toggles the split; `alt-ctrl-h/l` resizes.
- [ ] Get Info and Save dialogs float.
- [ ] VS Code and Slack land correctly on the first try, or after a retry (logged at debug level).
- [ ] `kill -STOP <pid>` of one app: the others keep tiling and a notice says the app is not responding. `kill -CONT` recovers.
- [ ] System Settings (large minimum size) causes no retry storm: its neighbour shrinks instead.

## M2 Workspaces, parking, recovery
- [ ] `alt-2` hides workspace 1, leaving only a 1 pt sliver at the bottom-right; `alt-1` restores the exact frames.
- [ ] Quit from the menu restores all windows; so does `kill -TERM $(pgrep -x Pane)`.
- [ ] `kill -9`, then relaunch: all windows come back with a "Restored N windows" notice.
- [ ] Mission Control shows parked windows on their own display.

## M3 Three displays
- [ ] Workspaces 1-3 are on the left, 4-6 centre, 7-9 right.
- [ ] Unplug right: 7-9 are hidden on the centre display, whose visible workspace stays the same; nothing is lost. Replug: 7-9 return, showing the one that was visible before.
- [ ] Repeat with the left display, then unplug two; then sleep and wake.
- [ ] `alt-l` from the centre's rightmost tile focuses the (empty) right display; `alt-shift-.` moves a workspace; `alt-return` swaps.

## M4 Persistence and identity
- [ ] With 6 windows in nested splits across 3 displays, quit and relaunch Pane: identical.
- [ ] Quit Safari (2 windows with distinct titles) and relaunch it: both return to their tiles and weights.
- [ ] Reboot: the layout comes back for apps that relaunch automatically.

## M5 Menu bar icon
- [ ] 3 boxes match the physical arrangement and update within 100 ms; the focused tile and display are highlighted.
- [ ] Legible in dark and light menu bars; idle CPU stays under 0.5% (Activity Monitor).

## M6 Profiles
- [ ] Profile Editor > Capture Current shows all 3 displays correctly.
- [ ] Drag Mail onto a tile edge and save as "work". Quit Mail and apply: Mail launches into its tile within 30 s.
- [ ] Applying with everything running launches nothing and closes nothing; a missing app gives a notice.
- [ ] Save as Startup: the profile applies at launch.
