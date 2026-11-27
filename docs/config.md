# config.toml

`~/Library/Application Support/Pane/config.toml` is created with comments on first launch. **Settings…** in the menu edits `[settings]` and the hotkeys for you. It rewrites only the lines it changes, keeps your comments, and refuses changes that would make the file invalid. To reload it, press `alt-shift-r`, run `pane reload`, or use the menu. Unknown keys are errors; errors show line and column in a notice and in Settings, and Pane keeps the previous config.

```toml
version = 1

[settings]
inner_gap = 6                    # points between tiles
outer_gap = 6                    # points around the edge of each display
smart_gaps = false               # no gaps on a workspace that shows a single tile
default_layout = "tiles"         # or "accordion"
placement_memory_days = 30       # how long to remember where windows were
restore_placements = true        # put relaunched windows back in their tiles
startup_profile = "work"         # default profile: applied at launch when no profile is active
auto_profiles = true             # apply a profile by itself when the displays match its [when]
electron_bundle_ids = ["com.microsoft.VSCode"]   # use size/position/size when moving
resize_step = 0.05
auto_float_below = 120           # windows narrower or shorter than this float
float_apps = ["com.apple.calculator"]   # always start floating (bundle id or executable name, globs ok)
tile_apps = ["my-app"]           # always tile, even if Pane would float them
modifier = "alt"                 # behind every default hotkey: "alt", "ctrl-alt", "cmd-ctrl",
                                 # "hyper" (cmd-ctrl-alt-shift), "meh" (ctrl-alt-shift)
gestures = true                  # hold the modifier and point to move windows
gesture_modifier = ""            # keys for gestures; empty = same as `modifier`
border_width = 2                 # border around the focused window, points; 0 = off
border_color = "accent"          # "accent" (system accent) or "#rrggbb" / "#rrggbbaa"
dim_unfocused = 0                # darken unfocused windows, 0 (off) to 0.8
mouse_follows_focus = false      # move the pointer to the window you focus from the keyboard
focus_follows_mouse = false      # focus the window the pointer rests over (150 ms)
title_drag = true                # drag a tiled window's title bar to swap/split/move it; off: it snaps back
menu_bar_app_icons = true        # app icons on the tiles of the menu bar icon
master_ratio = 0.6               # width share of the large tile in `layout master-stack`, 0.1 to 0.9

# Name displays. Each alias needs exactly one of: position, uuid, match_name.
[[display]]
name = "left"
position = "left"                # left | center | right (by arrangement)
# uuid = "37D8832A-..."          # `pane query displays` shows UUIDs
# match_name = "DELL U2720Q"     # substring of the display name
# inner_gap = 0                  # optional: overrides [settings] on this display
# outer_gap = 0                  # optional: overrides [settings] on this display

# Workspaces and their display preferences (most preferred first).
[[workspace]]
name = "1"
display = ["left", "main"]

# Names for workspaces, shown in the menu bar icon, the menu and the palette.
[workspace_names]
"1" = "web"                      # workspace 1 of every display
"right:3" = "chat"               # workspace 3 of the right display (wins over "3")

# Rules match by app (glob on bundle id) and/or title (regex).
# Precedence: ignore > float/tile > automatic floating > tile.
[[rule]]
app = "com.apple.finder"
title = "^Copy"
action = "float"                 # float | tile | ignore
workspace = "3"                  # optional: where new windows go
display = "right"                # optional: visible workspace of that display

# Layout rules: where a matching window's tile goes when it is first tiled.
[[rule]]
app = "com.mitchellh.ghostty"
position = "right"               # next (default) | left | right | top | bottom
weight = 0.3                     # optional: share of the split, between 0 and 1
# orientation = "vertical"       # with position = "next": horizontal (beside) | vertical (below)
# layout = "accordion"           # tiles | accordion: layout of the split the tile lands in

# A scratchpad: one window of `app`, toggled as a centered float on the
# focused display. Opens the app if it is not running.
[[scratchpad]]
app = "com.googlecode.iterm2"    # bundle id (or executable name)
key = "mod-grave"                # optional; [shortcuts] can override it
size = "60%x50%"                 # optional, default 60%x60%; points work too: "1200x800"
name = "term"                    # optional, for `scratchpad term`; defaults to app

[shortcuts]                      # merged over the defaults; "mod" = the modifier above
"mod-h" = "focus left"
"mod-q" = "none"                 # unbind a default
"cmd-ctrl-t" = "toggle-float"    # absolute chords work too
```

## Gaps

`inner_gap` is the space between tiles, `outer_gap` the space between the tiles and the edges of the display's visible frame (below the menu bar, beside the Dock). Both are points, 0 to 200.

`inner_gap` and `outer_gap` in a `[[display]]` alias override `[settings]` on the display that alias resolves to, so a laptop screen can have none and a 4K monitor more:

```toml
[[display]]
name = "laptop"
match_name = "Built-in"
inner_gap = 0
outer_gap = 0

[[display]]
name = "4k"
uuid = "37D8832A-..."
outer_gap = 12                   # inner_gap stays as in [settings]
```

Each gap is overridden on its own; if several aliases resolve to the same display, the first one that sets a gap wins for that gap. Aliases whose display is not connected do nothing.

With `smart_gaps = true`, a workspace that shows a single tile gets no gaps at all: the window fills the visible frame. Several windows in one accordion count as one tile. As soon as a second tile appears, the display's gaps come back.

## Hotkey layers

Defaults come in three layers, all built from `modifier`:

- **base** (`mod`): focus, workspaces, display focus, toggles.
- **move** (`mod` + Shift): move window, move to workspace, move workspace to display, fullscreen, reload, profile picker.
- **resize** (`mod` + Control): resize.

If the modifier already includes Shift or Control, those layers take the next free key, in the order Control, Option, Command, Shift. The two layers never collide. With `hyper` every modifier is already in use, so only the base layer gets defaults; bind the rest in `[shortcuts]` or in Settings.

Shortcuts set in Settings are written relative to the modifier (`"mod-cmd-y"`) when they include it, so they move with it if you change the modifier later.

## Which windows tile

Pane tiles standard, resizable windows of regular (Dock) apps. It floats:
- dialogs, sheets and panels;
- windows that cannot be resized;
- modal windows;
- windows of menu-bar or accessory apps;
- windows smaller than `auto_float_below`.

`float_apps`, `tile_apps` and `[[rule]]`s override this (rules first). Apps started from a terminal, such as `cargo run`, have no bundle id; Pane names them by their executable, e.g. `my-app`.

### Layout rules

`position`, `orientation`, `layout` and `weight` in a `[[rule]]` decide where a matching window's tile goes the first time it is tiled:

- `position = "left"`, `"right"`, `"top"` or `"bottom"` puts it at that edge of the workspace, as a full-height column or full-width row next to everything else, like dragging a window to a display edge.
- `position = "next"` (the default) puts it next to the focused tile. `orientation = "horizontal"` places it beside that tile and `"vertical"` below it, splitting the tile if needed. An edge position sets the orientation itself, so the two can't be combined.
- `weight` is the tile's share of the split it lands in: `0.3` makes a terminal column take 30% of the width.
- `layout` sets the layout of that split, `tiles` or `accordion`. With an edge position the split is the whole workspace.

Each of these comes from the first matching rule that sets it, so a rule for one title can override a rule for the whole app. They apply only when Pane makes a new tile: a window going back to a remembered tile, a profile slot, or the layout saved across a restart keeps that place. Floating windows ignore them.

`pane query windows` shows a `reason` for every window, for example `auto-float: not resizable`. `pane query apps` lists the apps Pane observes. If an app is missing from `pane query apps`, Pane has not seen a window from it.

## Scratchpads

A scratchpad's window floats on the workspace it was last shown on. Hidden, it is parked like a window of a hidden workspace, whichever workspace is shown, and it is not counted when cycling workspaces. Pressing the key while it is shown but not focused, or shown on another display, brings it to the focused display and focuses it instead of hiding it. It keeps a position you dragged it to on the same display; on another display it is centered again at `size`. Cmd-Tab or a Dock click on the app shows it. Pane remembers which window is the scratchpad across restarts; when that window closes, the next press opens a new one. Removing the `[[scratchpad]]` lets go of the window (shown again if it was hidden).

## Workspaces

Workspaces are per display: each display has its own `1`-`9` (any name works with `workspace NAME`), shown with `mod-1..9` on the display you are on. `[[workspace]]` tables from older configs are accepted and ignored.

`[workspace_names]` gives workspaces a name to show next to their number: in the menu bar icon (the workspace shown on the focused display, beside the desk), in the menu (`2 · web`) and in the palette, where you can search by it (`Workspace 2 · web`; windows say `workspace 2 (web)`). A key is a workspace (`"1"`, on every display) or `DISPLAY:WORKSPACE` with any display reference below (`"right:3"`, `"uuid:<UUID>:2"`); the display form wins. Names are 1-32 characters; the icon shows the first 16. Workspaces keep their number for commands, so `mod-3` and `workspace 3` still work.

Commands that take a workspace accept `NAME` (on the focused display) or `DISPLAY:NAME`, e.g. `pane ws left:2`, `pane ws main:web`, or `pane ws uuid:<UUID>:3`.

## Menu bar icon

The icon draws every display in its arrangement with the visible workspace's tiles inside it; the focused tile is solid, dots under a display count its hidden workspaces with windows, and a bar marks the focused display. With `menu_bar_app_icons = true` (the default; **Settings > General > Menu Bar**) each tile big enough for one shows its app's icon, and the icon is drawn in colour instead of as a monochrome template.

Click a window's tile to focus that window (the same as `focus-window ID`). Clicking anywhere else, on the focused tile, or with the right button or Control opens the menu.

## Display references

These forms work in `[[workspace]].display`, `[[rule]].display` and profile files:

- the name of a `[[display]]` alias;
- `main`;
- `left`, `center` or `right`. With three displays these are by position. With two, `center` means the main display. With one, all three resolve to it.
- `uuid:<UUID>`, or a bare UUID;
- `name:<substring>`, or any other text, matched against the display name.

## Profiles that follow the displays

A profile with a `[when]` block applies by itself when the connected displays come to match it, such as three monitors at the office and the laptop alone on the train. **Apply “NAME” on These Displays Automatically** in the menu's Profiles submenu writes one for the active profile, and so does `pane profile auto NAME`. You can also write one in `profiles/NAME.toml` yourself:

```toml
name = "office"

[when]
displays = ["uuid:37D8832A-...", "uuid:5A1B...", "name:built-in"]
exact = true                     # default: no other display connected; false = at least these
```

`displays` takes the display references above. Positions (`left`, `main`) match whatever is connected, so use UUIDs, names or `[[display]]` aliases. `pane query displays` lists the UUIDs.

- Pane waits until the displays have not changed for 5 seconds, so a monitor that flickers or wakes slowly doesn't switch profiles back and forth.
- It chooses once per desk. A profile you apply by hand stays until the set of connected displays changes, including across a restart on the same desk.
- When several profiles match, an exact match beats one with `exact = false`, then the one naming more displays wins, then the active profile, then the first name.
- A matching profile at launch takes the place of `startup_profile`.
- `auto_profiles = false` turns it off.

## Profile drift

Pane compares the active profile's saved file with the desk whenever the desk changes. When a workspace the profile describes no longer matches, the menu bar icon gets a dot and the Profiles menu item a `•`.

- A workspace is matched by its display (resolved, so `left` in the profile and `uuid:…` on the desk are the same display) and its name.
- It differs when an app was added or is gone, when the tiles are arranged differently (order, splits, orientation, accordion), or when a tile's share of its row or column changed by more than 2 percentage points.
- Apps are compared by bundle id, so two windows of one app swapping places, or a changed window title, is not drift.
- Workspaces with windows that the profile does not mention are not drift (applying the profile leaves them alone), but the diff lists them and updating adds them.
- Entries for displays that are not connected cannot be compared and stay as they are.
- Nothing is compared until the startup scan has found the windows.

Updating writes the desk into the profile file: changed workspaces take the desk's tree, keeping each slot's `title` regex and `launch_args` where the same app is still there; workspaces whose apps are all gone are dropped (kept empty if marked to be shown); the name and `[when]` stay. `pane profile diff [NAME]` prints the comparison as JSON (`entries[].change` is `unchanged`, `changed`, `added`, `removed` or `not_connected`; `updated` is the profile updating would save). `pane profile update [NAME]` saves it. Without a name both use the active profile.

## Shortcut commands

Modifiers are `cmd`, `alt`, `ctrl`, `shift`, `hyper`, `meh` and `mod`. Keys are `a`-`z`, `0`-`9`, `return`, `tab`, `space`, `escape`, `comma`, `period`, `slash`, `semicolon`, `quote`, `leftbracket`, `rightbracket`, `minus`, `equal`, `backslash`, `grave`, the arrow keys (`left`, `right`, `up`, `down`) and `f1`-`f12`.

| Command | Meaning |
|---|---|
| `focus left\|right\|up\|down` | focus the neighbour, crossing displays |
| `move left\|right\|up\|down` | move the window; at the edge, to the next display |
| `resize width\|height ±N` | grow or shrink by N percent |
| `workspace NAME` | show workspace NAME of the focused display (or `DISPLAY:NAME`) |
| `move-to-workspace NAME` | move the focused window there |
| `cycle-workspace next\|prev` | next/previous workspace on the focused display |
| `focus-display next\|prev` | |
| `move-workspace-to-display next\|prev` | |
| `toggle-orientation`, `toggle-layout`, `toggle-float` | |
| `balance` | give the tiles next to the focused one (its container) equal sizes |
| `layout master-stack [PERCENT]` | one large tile on the left, the rest stacked on the right; PERCENT (10-90) is the large tile's width, default `master_ratio`. Kept as windows come and go |
| `layout columns N` | N equal columns (1-12), each stacking its tiles. Kept as windows come and go |
| `layout manual` | stop keeping a layout; the tiles stay where they are |
| `toggle-fullscreen` | zoom the focused tile to fill its display; ends when another window there is focused or a new tile opens |
| `swap-workspaces` | swap with the next display |
| `undo` | put back the layout from before the last move, resize, gesture or edge drag, `move-to-workspace`, toggle (orientation, layout, float) or `profile-apply`; again to go further back (20 steps). Windows opened since stay, closed ones are left out (`mod-z`) |
| `reload`, `profile-picker`, `open-editor` | |
| `palette` | open the window and command palette (`mod-/`) |
| `focus-window ID` | show the window's workspace and focus it, restoring it if minimized (`pane query windows` lists ids) |
| `scratchpad NAME` | show the `[[scratchpad]]` NAME on the focused display (opening the app, or taking its window, the first time); hide it if it is shown there and focused |
| `summon APP` | bring a window of APP (bundle id or app name, any case) to the focused workspace, next to the focused tile, and focus it; opens APP if it has no window |
| `profile-apply NAME` | |
| `profile-diff [NAME]`, `profile-update [NAME]` | compare the desk with saved profile NAME (default: the active one), or save the desk into it; see [Profile drift](#profile-drift) |
| `profile-when here\|off NAME` | `here`: apply NAME by itself whenever exactly the displays connected now are connected (writes its `[when]`); `off`: only by hand |
| `recover`, `recover --all-apps` | |

## Layout presets

`layout master-stack` and `layout columns N` rearrange the focused workspace's tiles, in their current order, and the workspace remembers the layout (in `state.json`, so it survives a restart). Whenever the number of tiles there changes (a window opens, closes, or moves in or out), Pane rearranges them again:

- **master-stack**: the first tile is the master, on the left at `master_ratio` of the width (or the percentage given); the others share a column on the right. When the master closes, the next tile takes its place. If you resize the master, it keeps that width when windows come and go.
- **columns N**: the tiles are dealt into N equal columns, earlier columns taking one more when they don't divide evenly. With N or fewer tiles, each gets its own column.

Between those changes you can still move, split and resize by hand; the next window that opens or closes puts the layout back. `layout manual` turns this off, and applying a profile to the workspace replaces the layout with the profile's own. `pane query workspaces` shows each workspace's `layout`.

`balance` doesn't need a preset: it evens out the container of the focused tile (the workspace's top level when nothing is focused), leaving the other splits alone.

None of these has a default shortcut. Bind them in `[shortcuts]` (for example `"mod-e" = "balance"`, `"mod-m" = "layout master-stack"`) or run them from the palette (`mod-/`, then `>layout`).

## Events (`pane subscribe`)

`pane subscribe [KIND…] [--snapshot]` (on the socket: `{"cmd":"subscribe","events":["focus"]}`) answers `{"status":"ok"}`, then writes one JSON object per line, tagged by `event`. It starts with the current state, one event of each kind asked for, then sends an event whenever that part changes (at most every 50 ms). No kinds means all of them except `snapshot`. It works only over the socket, not as a shortcut.

| `event` | When | Fields |
|---|---|---|
| `focus` | the focused window, its title, or its display/workspace changes | `window` (id; null when the desktop has focus), `app` (bundle id), `title`, `display` (UUID), `workspace` |
| `workspace-shown` | a display shows another workspace (or its `[workspace_names]` name changes), or a display appears | `display`, `display_name`, `workspace`, `workspace_title` (its name, or null), `previous` |
| `workspaces` | the workspace list changes (also window counts and guests) | `workspaces`: `id`, `name`, `display`, `home`, `guest`, `label`, `title` (its `[workspace_names]` name, or null), `target`, `visible`, `windows` |
| `displays` | a display is connected, unplugged or focused | `displays`: `id`, `name`, `is_main`, `rank`, `focused`; `focused` (UUID) |
| `profile` | another profile becomes active | `active` |
| `snapshot` | anything in `pane query snapshot` changes | `snapshot` |

A client that reads too slowly (256 lines behind) is disconnected; Pane does not wait for it.

## Files

| File | Content |
|---|---|
| `config.toml` | this file |
| `state.json` | workspaces, trees, weights, visible workspace per display, window placements (written 500 ms after changes, atomically) |
| `parked.json` | hidden windows and their original frames (written before every park) |
| `run.lock` | pid of the running instance; a leftover means the last run crashed |
| `profiles/*.toml` | saved profiles |
| `logs/pane.*.log` | logs, rotated daily; `RUST_LOG=debug` for more |
| `pane.sock` | CLI socket |
