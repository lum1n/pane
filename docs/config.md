# config.toml

`~/Library/Application Support/Pane/config.toml` is created with comments on first launch. To reload it, press `alt-shift-r`, run `pane reload`, or use the menu. Unknown keys are errors; errors show line and column in a notice and in Settings, and Pane keeps the previous config.

```toml
version = 1

[settings]
inner_gap = 6                    # points between tiles
outer_gap = 6                    # points around the edge of each display
default_layout = "tiles"         # or "accordion"
placement_memory_days = 30       # how long to remember where windows were
restore_placements = true        # put relaunched windows back in their tiles
return_workspaces_on_reconnect = true
startup_profile = "work"         # applied once the first scan finishes
electron_bundle_ids = ["com.microsoft.VSCode"]   # use size/position/size when moving
resize_step = 0.05
auto_float_below = 120           # windows narrower or shorter than this float

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

[shortcuts]                      # merged over the defaults
"alt-h" = "focus left"
"alt-q" = "none"                 # unbind a default
```

## Display references

These forms work in `[[workspace]].display`, `[[rule]].display` and profile files:

- the name of a `[[display]]` alias;
- `main`;
- `left`, `center` or `right`. With three displays these are by position. With two, `center` means the main display. With one, all three resolve to it.
- `uuid:<UUID>`, or a bare UUID;
- `name:<substring>`, or any other text, matched against the display name.

## Shortcut commands

Modifiers are `cmd`, `alt`, `ctrl` and `shift`. Keys are `a`-`z`, `0`-`9`, `return`, `tab`, `space`, `escape`, `comma`, `period`, `slash`, `semicolon`, `quote`, `leftbracket`, `rightbracket`, `minus`, `equal`, `backslash`, `grave`, the arrow keys (`left`, `right`, `up`, `down`) and `f1`-`f12`.

| Command | Meaning |
|---|---|
| `focus left\|right\|up\|down` | focus the neighbour, crossing displays |
| `move left\|right\|up\|down` | move the window; at the edge, to the next display |
| `resize width\|height ±N` | grow or shrink by N percent |
| `workspace NAME` | show a workspace (created if new) |
| `move-to-workspace NAME` | move the focused window |
| `focus-display next\|prev` | |
| `move-workspace-to-display next\|prev` | |
| `toggle-orientation`, `toggle-layout`, `toggle-float` | |
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
