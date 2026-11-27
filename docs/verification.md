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

## Profiles that follow the displays
- [ ] Apply "office" with all displays connected and tick **Apply “office” on These Displays Automatically**: `profiles/office.toml` gets a `[when]` with every display's UUID; the menu marks "office (these displays)".
- [ ] Unplug down to the laptop, apply "laptop" and tick the same item. Plug the monitors back in: about 5 s after the last display appears, "office" applies with a notice. Unplug: "laptop" applies.
- [ ] Apply a third profile by hand on the office desk: it stays, also after `make stop` and reopening Pane.
- [ ] Sleep and wake the Mac on the office desk (monitors wake late): no profile switch.
- [ ] Quit Pane, change desk, start Pane: the matching profile applies at launch (instead of `startup_profile`).
- [ ] Save the desk again under the same name, or edit the profile in the Profile Editor and save: the `[when]` stays.

## Profile drift
- [ ] Apply a saved profile: no dot on the menu bar icon. Swap two tiles of different apps, or resize one by mouse: a dot appears top right, and the Profiles submenu title ends in `•`.
- [ ] Undo the change by re-applying the profile: the dot goes away.
- [ ] Open apps in a workspace the profile does not mention: no dot.
- [ ] **Update “NAME” from Current Desk…** opens the Profile Editor with the saved profile and the desk side by side; gone apps are outlined red on the left, new ones green on the right; unchanged rows are dimmed.
- [ ] **Update Profile** saves (the dot goes away, `profiles/NAME.toml` has the new layout, title regexes and `[when]` are kept); **Edit Before Saving** shows the result in the editor without saving; **Cancel** goes back.
- [ ] **Compare with Desk** in the editor works for any saved profile, also one that is not active.
- [ ] Right after launch (windows still being found) no dot flashes.

## Palette
- [ ] `mod-/` opens the palette centred on the focused display, above the focus border and dimming; typing works without Pane taking focus from the app you were in.
- [ ] Escape, or clicking elsewhere, closes it and leaves focus where it was.
- [ ] Empty query: the previously focused window is first; Enter goes back to it.
- [ ] Typing part of an app name or title finds windows on all 3 displays, including hidden workspaces; Enter shows that workspace on its own display and focuses the window.
- [ ] A minimized window is restored when picked; a window behind a zoomed tile ends the zoom.
- [ ] `>` lists commands with their shortcuts (⌥⇧F etc.); Enter runs one. Profiles apply.
- [ ] Arrow keys, Page Up/Down and clicking a row work; app icons show.

## Undo
- [ ] Move, resize (keys and edge drag), swap with a gesture, toggle split direction and float: each `mod-z` puts back one step, windows land in their old tiles.
- [ ] `mod-shift-3` then `mod-z`: the window comes back to its tile on the workspace you are on and has focus.
- [ ] Apply a profile that shows another workspace, then `mod-z`: the previous workspace and tiles come back and the menu no longer shows the profile as active.
- [ ] `mod-z` with nothing to undo shows an error notice and changes nothing.

## Scratchpad and summon
- [ ] With `[[scratchpad]] app = "com.googlecode.iterm2" key = "mod-grave"` and iTerm not running, Option-` opens iTerm and its first window appears as a centered float at the configured size on the focused display, focused.
- [ ] Option-` again hides it and focus returns to the window you were in; Option-` brings it back. Nothing flashes in the park corner while it is shown.
- [ ] With iTerm already running, Option-` takes its window (a tiled one leaves its tile, the others fill the space).
- [ ] Focus another display, Option-`: it appears centered on that display.
- [ ] Hidden, Cmd-Tab to iTerm shows it; the focus border follows it.
- [ ] Hidden, the menu bar icon does not draw it; `pane query windows` shows `"visible": false`.
- [ ] Quit Pane while it is hidden: the window comes back on screen.
- [ ] Menu bar icon → Scratchpads lists each scratchpad, checked while shown; clicking toggles it. With none configured it says so.
- [ ] Menu bar icon → Summon lists running apps; picking one moves its window next to the focused tile without switching workspace.
- [ ] Settings → Scratchpads: click +, pick a running app in the sheet, set a key and size, Save; `config.toml` gets a `[[scratchpad]]` (comments elsewhere intact) and the key works at once. Select it and click −: the table is gone and the window is let go.
- [ ] A bad size (`big`) shows the config error in the tab and leaves `config.toml` unchanged.
- [ ] `pane do "summon Safari"` from another workspace moves a Safari window next to the focused tile without switching workspace; with Safari not running it opens.

## Layout presets
- [ ] Three windows on a workspace, `pane do "layout master-stack"`: the first is on the left at 60% of the width, the other two stacked on the right.
- [ ] Open a fourth window: it joins the stack; the master keeps its width. Close the master: the next window becomes the master.
- [ ] Widen the master with `mod-ctrl-l`, open a window: the master stays that wide.
- [ ] `pane do "layout master-stack 70"`: the master becomes 70% wide.
- [ ] Restart Pane: the workspace still re-shapes when a window opens. `pane query workspaces` shows its `layout`.
- [ ] `pane do "layout columns 3"` with five windows: three equal columns (two, two, one). A sixth window makes it two, two, two.
- [ ] `pane do "layout manual"`, open a window: it opens next to the focused tile and nothing else moves.
- [ ] Resize a tile, then `pane do "balance"`: the tiles of that split are equal again; other splits keep their sizes.
- [ ] Palette, `>layout`: "Layout: master and stack", "Layout: 2 columns", "Layout: 3 columns", "Layout: manual (stop re-arranging)"; `>balance`: "Balance tile sizes". Each runs.

## Event stream
- [ ] `pane subscribe | jq -c .` in a terminal: prints `displays`, one `workspace-shown` per display, `workspaces`, `focus` and `profile` at once.
- [ ] Switch workspace with a hotkey: a `workspace-shown` with the new `workspace` and `previous`, then `workspaces` and `focus`.
- [ ] Click into another window, and on another display: a `focus` with its `app`, `title`, `display` and `workspace`. Switching to an empty workspace gives `"window": null`.
- [ ] Change a focused Safari tab: a `focus` with the new `title`.
- [ ] Unplug a display: `displays` without it and `workspaces` with its workspaces as guests. Plug it back in: `displays` and its `workspace-shown`.
- [ ] Apply a profile from the menu: `profile` with its name.
- [ ] `pane subscribe focus --snapshot` prints only `focus` and `snapshot` lines.
- [ ] Ctrl-C the subscriber, then switch workspaces: Pane keeps working (no crash from SIGPIPE).
- [ ] Quit Pane: the subscriber exits with status 0.

## Launch at login and updates
- [ ] `make install`, open /Applications/Pane.app, Settings > General > Startup and Updates: tick **Open Pane at login**. Pane is listed in System Settings > General > Login Items; log out and in: Pane starts.
- [ ] If macOS shows "needs approval", the orange line and **Open Login Items** appear; allow Pane there and come back to Settings: the toggle is on and the line is gone.
- [ ] Untick it: Pane leaves Login Items.
- [ ] **Check for Updates…** in the menu with no newer release: "You're up to date". Offline: "Couldn't check for updates" with the reason.
- [ ] Set `CFBundleShortVersionString` in the built app's Info.plist below the latest release tag (or `defaults delete com.vegard.pane lastUpdateCheck` with an older build) and relaunch: a notice says the new version is available once, the menu starts with "Pane X Is Available…", and that item opens the release page.
- [ ] Untick **Check for updates automatically**: no check at launch (`defaults read com.vegard.pane checkForUpdates` is 0).

## Title-bar drag
- [ ] Drag a tiled window by its title bar (no modifier) over the middle of another tile: the outline appears; let go and the two swap. `mod-z` swaps them back.
- [ ] Drag near an edge of another tile, to a screen edge, and onto another display: the outline matches where the window lands on release each time.
- [ ] Pause for a few seconds mid-drag with the button held: the window does not jump back to its tile.
- [ ] Drag out and back over the window's own tile and let go: the outline goes away and the window returns to its tile.
- [ ] Untick **Move tiled windows by dragging their title bar** in Settings > Shortcuts: a dragged tile snaps back with no outline, without restarting. Dragging a floating window still just moves it. Resizing by an edge still works.

## Mouse follows focus, focus follows mouse
- [ ] With `mouse_follows_focus = true`, `mod-l` to the next tile puts the pointer in its middle; `mod-l` onto the next display (also an empty one) moves the pointer there. The pointer does not stutter after a warp.
- [ ] The pointer already over the window you focus does not move. Clicking a window or Cmd-Tab never moves the pointer.
- [ ] `mod-2` to a workspace with windows puts the pointer on the window that gets focus, at its new place.
- [ ] With `focus_follows_mouse = true`, resting the pointer on another tile focuses it (border follows) after a moment; sweeping across tiles quickly does not focus the ones passed over.
- [ ] Switching workspace under a still pointer does not change focus. Holding Option and pointing (gesture) does not focus anything.
- [ ] Both toggles in Settings > Focus take effect without restarting.

## Trackpad swipes
- [ ] First set System Settings > Trackpad > More Gestures > "Swipe between full-screen apps" to four fingers or off, and check Mission Control / App Exposé are not on four fingers up.
- [ ] Tick **Switch workspaces with three-finger swipes** in Settings > Shortcuts (or `trackpad_swipes = true`): with the pointer over the centre display, a three-finger swipe left shows its next workspace and right the previous one, even while another display has focus. One swipe moves one workspace, however far the fingers travel.
- [ ] A four-finger swipe up opens the palette.
- [ ] Two-finger scrolling, pinch to zoom, rotating, and resting three fingers do nothing. Three-finger drag (Accessibility) still drags windows; if it also switches workspaces, note it here.
- [ ] Untick the toggle: swipes do nothing, without restarting.

## Doctor
- [ ] With Pane running and permissions granted, `pane doctor` shows Accessibility and Input Monitoring for Pane (no "this terminal"), `Hotkeys: running`, the displays as in `pane query displays`, `run.lock: held by pid` Pane's pid, and exits 0.
- [ ] Turn Pane off in Privacy & Security > Accessibility: `pane doctor` says NOT granted and exits 1.
- [ ] `kill -STOP` an app's pid, wait for the notice: it is listed as not responding. `kill -CONT` and it goes away.
- [ ] A window that refuses its tile (fixed-size app) is listed under its app with `refused to take its tile`.
- [ ] Break `config.toml` (`gaps = 999`) without reloading: `Config: ERROR` with the message. Reload, fix the file, run again: "fixed on disk, but Pane still runs on defaults".
- [ ] Quit Pane: `pane doctor` says not running, shows the terminal's permissions, the displays, and `run.lock: none`.
- [ ] `kill -9` Pane: `run.lock: STALE` with the parked window count; starting Pane clears it.

## UI polish (macOS 26)
Build with Xcode 26 on PATH (`make app && make run`); check in light and dark mode, and again with Increase Contrast and Reduce Transparency on.
- [ ] **Settings**: the toolbar tabs (General, Layout, Focus, Apps, Scratchpads, Shortcuts) switch, the window title follows the tab, and the window takes each pane's height.
- [ ] Every control round-trips to `config.toml`: change a gap with the stepper, check the file, change it back. The status capsule at the bottom shows the result and goes away after about 4 s.
- [ ] Shortcuts: click a keycap, press a new chord; it records ("Type shortcut…" with an accent outline) and Esc cancels. Hovering a row shows Reset and Clear; the row's context menu has the same; double-clicking a row records. The search field filters. **Reset All…** asks first.
- [ ] Apps: + offers running apps (with icons), Choose Application… and Enter Bundle ID or Glob… (sheet). Selecting a row and − removes it. An empty list shows "No Apps".
- [ ] Scratchpads: + opens the sheet; Save writes `[[scratchpad]]`. Double-click or Edit… edits; an invalid entry keeps the sheet open with the error.
- [ ] General: the Accessibility capsule is green when granted, orange with **Open Settings** when not. Check Now shows a spinner while checking.
- [ ] **Profile editor**: the toolbar shows Profiles, New, Capture, Compare, Undo/Redo, Add Display, Save (menu: Save as Startup Profile) and a glass **Apply**. ⌘S saves, ⌘Z/⇧⌘Z undo and redo.
- [ ] Rename by editing the window title, and with Profiles > Rename…; Open lists the new name after saving.
- [ ] Sidebar search filters apps, with "Running" and "Applications" sections. Dragging an app onto a tile highlights the half it will take (edge) or the whole tile (middle) and drops there. Dragging tiles still swaps and moves.
- [ ] The inspector button shows and hides the inspector. With a tile selected: header with icon, Matching, Workspace (rename, Show after applying), Layout (segmented split direction, Accordion), Remove Tile. With nothing selected: "No Tile Selected".
- [ ] A profile with no displays shows "No Displays" with New Profile and Capture Current Desk.
- [ ] Compare with the desk drifted: rows with tinted badges, and Cancel / Edit Before Saving / glass Update Profile in the toolbar. With no drift: "No Differences".
- [ ] **Palette** (`mod-/`): a glass panel with a magnifier. It takes typing without activating Pane; arrows, Enter and Esc work; the selection is a rounded accent rect, readable in light and dark. Shortcuts show as keycaps, the current item has a "Current" tag, and a query with no results shows "No matches".
- [ ] **HUD**: Reload with a broken config: a glass capsule with a warning symbol fades in and out. With Reduce Motion on, it appears and disappears at once.
- [ ] **Gesture overlay**: the preview has 12 pt corners and the label sits in a glass pill; nothing animates. The focus border's corners match the window's.
- [ ] **Menu**: displays are section headers, workspace window counts are badges, every action has a symbol, and **About Pane** opens the About panel. **Save Current Desk as Profile…**: Save stays disabled until a name is typed.
- [ ] **Onboarding**: `tccutil reset Accessibility com.vegard.pane`, launch: the welcome window opens. **Open System Settings**, turn Pane on: the window flips to "You're all set" with **Done** within a second. The menu's **Grant Accessibility Access…** opens the same window.
