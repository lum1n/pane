# config.toml

`~/Library/Application Support/Pane/config.toml` is created with comments on first launch. **Settings…** in the menu edits `[settings]` and the hotkeys for you. It rewrites only the lines it changes, keeps your comments, and refuses changes that would make the file invalid. To reload it, press `alt-shift-r`, run `pane reload`, or use the menu. Unknown keys are errors; errors show line and column in a notice and in Settings, and Pane keeps the previous config.

```toml
version = 1

[settings]
inner_gap = 6                    # points between tiles
outer_gap = 6                    # points around the edge of each display
default_layout = "tiles"         # or "accordion"
placement_memory_days = 30       # how long to remember where windows were
restore_placements = true        # put relaunched windows back in their tiles
startup_profile = "work"         # default profile: applied at launch when no profile is active
electron_bundle_ids = ["com.microsoft.VSCode"]   # use size/position/size when moving
resize_step = 0.05
auto_float_below = 120           # windows narrower or shorter than this float
float_apps = ["com.apple.calculator"]   # always start floating (bundle id or executable name, globs ok)
tile_apps = ["my-app"]           # always tile, even if Pane would float them
modifier = "alt"                 # behind every default hotkey: "alt", "ctrl-alt", "cmd-ctrl",
                                 # "hyper" (cmd-ctrl-alt-shift), "meh" (ctrl-alt-shift)

# Name displays. Each alias needs exactly one of: position, uuid, match_name.
[[display]]
name = "left"
position = "left"                # left | center | right (by arrangement)
# uuid = "37D8832A-..."          # `pane query displays` shows UUIDs
# match_name = "DELL U2720Q"     # substring of the display name

# Workspaces and their display preferences (most preferred first).
[[workspace]]
name = "1"
display = ["left", "main"]

# Rules match by app (glob on bundle id) and/or title (regex).
# Precedence: ignore > float/tile > automatic floating > tile.
[[rule]]
app = "com.apple.finder"
title = "^Copy"
action = "float"                 # float | tile | ignore
workspace = "3"                  # optional: where new windows go
display = "right"                # optional: visible workspace of that display

[shortcuts]                      # merged over the defaults; "mod" = the modifier above
"mod-h" = "focus left"
"mod-q" = "none"                 # unbind a default
"cmd-ctrl-t" = "toggle-float"    # absolute chords work too
```

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

`pane query windows` shows a `reason` for every window, for example `auto-float: not resizable`. `pane query apps` lists the apps Pane observes. If an app is missing from `pane query apps`, Pane has not seen a window from it.

## Workspaces

Workspaces are per display: each display has its own `1`-`9` (any name works with `workspace NAME`), shown with `mod-1..9` on the display you are on. `[[workspace]]` tables from older configs are accepted and ignored.

Commands that take a workspace accept `NAME` (on the focused display) or `DISPLAY:NAME`, e.g. `pane ws left:2`, `pane ws main:web`, or `pane ws uuid:<UUID>:3`.

## Display references

These forms work in `[[workspace]].display`, `[[rule]].display` and profile files:

- the name of a `[[display]]` alias;
- `main`;
- `left`, `center` or `right`. With three displays these are by position. With two, `center` means the main display. With one, all three resolve to it.
- `uuid:<UUID>`, or a bare UUID;
- `name:<substring>`, or any other text, matched against the display name.

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
| `toggle-fullscreen` | zoom the focused tile to fill its display; ends when another window there is focused or a new tile opens |
| `swap-workspaces` | swap with the next display |
| `reload`, `profile-picker`, `open-editor` | |
| `profile-apply NAME` | |
| `recover`, `recover --all-apps` | |

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
