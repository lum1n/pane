# Pane

A tiling window manager for macOS 14+ on Apple silicon. It has a window-first split tree (i3 style), per-display workspaces, no animations, and a menu bar icon that draws your live desk. Profiles save "which app in which pane on which screen" and restore it, launching apps that aren't running.

Pane uses the public Accessibility API plus one private call (`_AXUIElementGetWindow`). It needs no SIP changes and does not use native Spaces. Hidden workspaces are parked one point inside the bottom-right corner of their display, and there are several ways to get them back (see [Recovery](#recovery)).

## Build and run (on the Mac)

```sh
rustup target add aarch64-apple-darwin
scripts/make-dev-cert.sh      # once: stable signing identity, so Accessibility stays granted across rebuilds
make app                      # build/Pane.app
make run                      # (re)launch it
make install                  # copy to /Applications; `make link-cli` puts `pane` on PATH
```

On first launch, macOS asks for Accessibility access. Tiling starts as soon as you grant it. `make agent` builds the headless `pane-agentd` (everything except the UI), which is handy for debugging from a terminal.

## Use

Every display has its own workspaces 1-9. They stay on their display: unplug a monitor and its workspaces wait, hidden, on the nearest remaining display (reachable from the menu), then go back when it returns. A new monitor plugged in at the same spot takes them over.

All shortcuts are built on one **modifier** (`mod`, Option by default). Change it, or any single shortcut, in **Settings > Hotkeys**: tick the modifier keys you want, or click a shortcut and press a new one. Moving adds Shift and resizing adds Control; if Shift or Control is already part of the modifier, the next free key is used instead.

| Keys | Action |
|---|---|
| `mod-h/j/k/l` | focus left/down/up/right (crosses displays, including empty ones) |
| `mod-shift-h/j/k/l` | move window (at the edge: to the next display) |
| `mod-ctrl-h/l`, `mod-ctrl-k/j` | narrower/wider, shorter/taller by 5% |
| `mod-1..9` | show workspace 1-9 of the display you are on |
| `mod-shift-1..9` | move window to that workspace (same display) |
| `mod-[` / `mod-]` | previous/next workspace on this display |
| `mod-,` / `mod-.` | focus previous/next display |
| `mod-shift-,` / `mod-shift-.` | move this workspace to the previous/next display |
| `mod-s` | toggle split direction |
| `mod-a` | tiles / accordion |
| `mod-f` | float / tile |
| `mod-shift-f` | fullscreen: zoom the focused tile to fill the display (again to restore) |
| `mod-return` | swap what this display and the next one show |
| `mod-shift-r` | reload config |
| `mod-shift-p` | profile picker |
| `mod-/` | palette: go to any window, workspace, profile or command |

You can also resize with the mouse: drag the edge a tiled window shares with a neighbour. When you let go, the tile keeps its new size and only that neighbour gives or takes space. Dragging an outer screen edge snaps the window back.

To move a window with the mouse, hold the modifier (Option by default) over it and point somewhere else; no click needed. An outline shows where the window will go and how it will tile. Release the modifier to drop it there, or press Escape to cancel:

| Point at | The window |
|---|---|
| the middle of another tile | swaps places with it |
| near an edge of another tile | splits that tile and goes on that side |
| the outer edge of a screen (48 pt band) | becomes a full-height column or full-width row on that side |
| a screen with no tiles | fills that screen |

Floating windows you drop become tiles. Typing, clicking or a hotkey while you hold the modifier is not a gesture. Turn it off in Settings > Hotkeys or with `gestures = false`; `gesture_modifier` gives gestures their own keys.

`mod-/` (or **Go to Window…** in the menu) opens the palette. Type part of an app name or window title to find a window on any display or workspace, including hidden and minimized ones; Enter shows its workspace and focuses it. With nothing typed, the window you were in before comes first, so `mod-/` then Enter goes back. Workspaces and profiles are listed too; start with `>` to search only commands (each with its shortcut). `pane search TEXT` returns the same list as JSON.

A **scratchpad** is an app you call up with one key and dismiss with the same key: a terminal, notes, music. Add one to `config.toml`:

```toml
[[scratchpad]]
app = "com.googlecode.iterm2"
key = "mod-grave"
size = "60%x50%"
```

The key shows the app's window as a centered float on the display you are working on (opening the app if it is not running, or taking its existing window), and again hides it, giving focus back to where you were. On another display it comes to that display. Cmd-Tab to the app also brings it back. `summon APP` (bind it in `[shortcuts]`, e.g. `"mod-shift-t" = "summon Terminal"`) brings an app's window to the workspace you are on, next to the focused tile, without switching workspace.

You can also manage scratchpads in **Settings → Scratchpads**, toggle them from the menu bar icon's **Scratchpads** submenu, and summon any running app from its **Summon** submenu (the palette offers `Summon APP` after `>`).

Profiles can follow your displays. Apply a profile, then choose **Apply “NAME” on These Displays Automatically** in the menu's Profiles submenu (or run `pane profile auto NAME`). From then on, Pane applies that profile by itself whenever exactly those displays are connected. Plug in at the office and get the office layout; unplug and get the laptop one. It waits until the displays have settled for 5 seconds. A profile you pick by hand stays until the displays change. See [docs/config.md](docs/config.md#profiles-that-follow-the-displays).

Layout presets arrange a workspace for you. `layout master-stack` puts one large tile on the left (60% wide, `master_ratio`) and stacks the rest on the right; `layout columns 3` deals the tiles into three equal columns. The workspace keeps that shape as windows open and close, until `layout manual`. `balance` gives the tiles of the focused container equal sizes again. Run them from the palette (`>layout`, `>balance`) or bind them in `[shortcuts]`. See [docs/config.md](docs/config.md#layout-presets).

The focused window gets a thin border in the accent colour. Settings > General > Focus changes its width and colour (`border_width = 0` turns it off) and can dim the other windows (`dim_unfocused`).

The `pane` CLI talks to the running app over a Unix socket:

```sh
pane ws 3                 pane query windows|displays|workspaces|tree|snapshot
pane do "focus left"      pane profile list|apply|capture|delete|auto|manual NAME
pane reload               pane recover [--all-apps]
pane search saf           pane do "focus-window 1234"
pane do "scratchpad com.googlecode.iterm2"    pane do "summon Safari"
pane subscribe [focus|workspace-shown|workspaces|displays|profile] [--snapshot]
```

### Scripting with `pane subscribe`

`pane subscribe` prints one JSON line each time something changes: focus, the workspace a display shows, the workspace list, displays, the active profile. It starts with the current state, so a widget can draw at once, and runs until Pane quits. Name kinds to get only those; `--snapshot` adds the whole menu bar snapshot on every change. The events are listed in [docs/config.md](docs/config.md#events-pane-subscribe).

A SketchyBar item that shows the workspace on the focused display:

```sh
pane subscribe workspace-shown focus | while read -r ev; do
  ws=$(jq -r 'select(.event == "focus") | .workspace // empty' <<<"$ev")
  [ -n "$ws" ] && sketchybar --set pane_ws label="$ws"
done
```

Turn on Do Not Disturb while workspace 3 is shown:

```sh
pane subscribe workspace-shown | jq --unbuffered -r '.workspace' | while read -r ws; do
  if [ "$ws" = 3 ]; then shortcuts run "DND on"; else shortcuts run "DND off"; fi
done
```

Settings (gaps, behaviour, startup profile, apps that always float or tile, all hotkeys) are in **Settings…** in the menu. They are stored in `~/Library/Application Support/Pane/config.toml`, which you can also edit by hand; see [docs/config.md](docs/config.md). Displays, workspaces and window rules are edited in the file.

## Recovery

Windows are never lost:

- Quitting Pane (menu, `kill -TERM`, logout) puts every window back on screen first.
- If Pane crashes, the next launch restores everything listed in `parked.json`.
- **Restore All Windows** in the menu, or `pane recover --all-apps`, brings hidden windows back. The CLI also works when Pane isn't running.

## Layout of the repo

`crates/pane-core` holds the pure model (tree, layout, workspaces, displays, identity, profiles, reducer); it is tested on any OS. `pane-macos` is the AX, display and hotkey glue. `pane-agent` is the runtime. `pane-ffi` is the C ABI for the Swift shell, and `pane-cli` is the `pane` command. The menu bar app is in `shell/` (SwiftPM, no Xcode project). See [docs/architecture.md](docs/architecture.md).

On Linux, `make test` and `make check-mac` (which type-checks the macOS code against `aarch64-apple-darwin`) verify everything except the Swift shell.
