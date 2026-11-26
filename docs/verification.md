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
- [ ] `mod-2` on any display switches only that display; each display has its own 1-9; `mod-[`/`mod-]` cycle.
- [ ] Switching to an empty workspace and typing does not type into a hidden window; switching is never undone a moment later.
- [ ] No hidden window peeks out on a neighbouring display.
- [ ] Switch back and forth between workspaces many times, opening apps in between: windows are always in their tiles, never in a corner.
- [ ] Minimize a window, switch away and back: it stays minimized.
- [ ] Unplug right: its workspaces are hidden on the centre display (listed in the menu as "<display>: N"), whose visible workspace stays the same; nothing is lost. Replug: they return, showing the one that was visible before.
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
- [ ] Add a second workspace tab to a display (+), put apps in both, mark one with ★; apply: both workspaces are filled and the ★ one is shown.
- [ ] Drag Mail onto a tile edge and save as "work". Quit Mail and apply: Mail launches into its tile within 30 s.
- [ ] Applying with everything running launches nothing and closes nothing; a missing app gives a notice.
- [ ] Save as Startup: the profile applies at launch.
- [ ] Apply a profile, then switch workspaces, open and close windows, quit and reopen apps, restart Pane: the layout stays the profile's (the menu shows "Profile: name" with a checkmark) until you apply another.
- [ ] An app in the profile that is not running leaves no gap; when it opens, its window goes to its slot.
- [ ] No window ever ends up parked in the corner of a display while its workspace is shown.

## Palette
- [ ] `mod-/` opens the palette centred on the focused display, above the focus border and dimming; typing works without Pane taking focus from the app you were in.
- [ ] Escape, or clicking elsewhere, closes it and leaves focus where it was.
- [ ] Empty query: the previously focused window is first; Enter goes back to it.
- [ ] Typing part of an app name or title finds windows on all 3 displays, including hidden workspaces; Enter shows that workspace on its own display and focuses the window.
- [ ] A minimized window is restored when picked; a window behind a zoomed tile ends the zoom.
- [ ] `>` lists commands with their shortcuts (⌥⇧F etc.); Enter runs one. Profiles apply.
- [ ] Arrow keys, Page Up/Down and clicking a row work; app icons show.

## Scratchpad and summon
- [ ] With `[[scratchpad]] app = "com.googlecode.iterm2" key = "mod-grave"` and iTerm not running, Option-` opens iTerm and its first window appears as a centered float at the configured size on the focused display, focused.
- [ ] Option-` again hides it and focus returns to the window you were in; Option-` brings it back. Nothing flashes in the park corner while it is shown.
- [ ] With iTerm already running, Option-` takes its window (a tiled one leaves its tile, the others fill the space).
- [ ] Focus another display, Option-`: it appears centered on that display.
- [ ] Hidden, Cmd-Tab to iTerm shows it; the focus border follows it.
- [ ] Hidden, the menu bar icon does not draw it; `pane query windows` shows `"visible": false`.
- [ ] Quit Pane while it is hidden: the window comes back on screen.
- [ ] `pane do "summon Safari"` from another workspace moves a Safari window next to the focused tile without switching workspace; with Safari not running it opens.
