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

All shortcuts are built on one **modifier** (`mod`, Option by default). Change it, or any single shortcut, in **Settings > Hotkeys**: tick the modifier keys you want, or click a shortcut and press a new one. Moving adds Shift and resizing adds Control; if Shift or Control is already part of the modifier, the next free key is used instead.

| Keys | Action |
|---|---|
| `mod-h/j/k/l` | focus left/down/up/right (crosses displays, including empty ones) |
| `mod-shift-h/j/k/l` | move window (at the edge: to the next display) |
| `mod-ctrl-h/l`, `mod-ctrl-k/j` | narrower/wider, shorter/taller by 5% |
| `mod-1..9` | show workspace |
| `mod-shift-1..9` | move window to workspace |
| `mod-,` / `mod-.` | focus previous/next display |
| `mod-shift-,` / `mod-shift-.` | move workspace to previous/next display |
| `mod-s` | toggle split direction |
| `mod-a` | tiles / accordion |
| `mod-f` | float / tile |
| `mod-return` | swap workspaces with the next display |
| `mod-shift-r` | reload config |
| `mod-shift-p` | profile picker |

The `pane` CLI talks to the running app over a Unix socket:

```sh
pane ws 3                 pane query windows|displays|workspaces|tree|snapshot
pane do "focus left"      pane profile list|apply NAME|capture NAME|delete NAME
pane reload               pane recover [--all-apps]
```

Settings (gaps, behaviour, startup profile, all hotkeys) are in **Settings…** in the menu. They are stored in `~/Library/Application Support/Pane/config.toml`, which you can also edit by hand; see [docs/config.md](docs/config.md). Displays, workspaces and window rules are edited in the file.

## Recovery

Windows are never lost:

- Quitting Pane (menu, `kill -TERM`, logout) puts every window back on screen first.
- If Pane crashes, the next launch restores everything listed in `parked.json`.
- **Restore All Windows** in the menu, or `pane recover --all-apps`, brings hidden windows back. The CLI also works when Pane isn't running.

## Layout of the repo

`crates/pane-core` holds the pure model (tree, layout, workspaces, displays, identity, profiles, reducer); it is tested on any OS. `pane-macos` is the AX, display and hotkey glue. `pane-agent` is the runtime. `pane-ffi` is the C ABI for the Swift shell, and `pane-cli` is the `pane` command. The menu bar app is in `shell/` (SwiftPM, no Xcode project). See [docs/architecture.md](docs/architecture.md).

On Linux, `make test` and `make check-mac` (which type-checks the macOS code against `aarch64-apple-darwin`) verify everything except the Swift shell.
