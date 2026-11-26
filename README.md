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

| Keys | Action |
|---|---|
| `alt-h/j/k/l` | focus left/down/up/right (crosses displays, including empty ones) |
| `alt-shift-h/j/k/l` | move window (at the edge: to the next display) |
| `alt-ctrl-h/l`, `alt-ctrl-k/j` | narrower/wider, shorter/taller by 5% |
| `alt-1..9` | show workspace |
| `alt-shift-1..9` | move window to workspace |
| `alt-,` / `alt-.` | focus previous/next display |
| `alt-shift-,` / `alt-shift-.` | move workspace to previous/next display |
| `alt-s` | toggle split direction |
| `alt-a` | tiles / accordion |
| `alt-f` | float / tile |
| `alt-return` | swap workspaces with the next display |
| `alt-shift-r` | reload config |
| `alt-shift-p` | profile picker |

The `pane` CLI talks to the running app over a Unix socket:

```sh
pane ws 3                 pane query windows|displays|workspaces|tree|snapshot
pane do "focus left"      pane profile list|apply NAME|capture NAME|delete NAME
pane reload               pane recover [--all-apps]
```

Configuration is in `~/Library/Application Support/Pane/config.toml`; see [docs/config.md](docs/config.md).

## Recovery

Windows are never lost:

- Quitting Pane (menu, `kill -TERM`, logout) puts every window back on screen first.
- If Pane crashes, the next launch restores everything listed in `parked.json`.
- **Restore All Windows** in the menu, or `pane recover --all-apps`, brings hidden windows back. The CLI also works when Pane isn't running.

## Layout of the repo

`crates/pane-core` holds the pure model (tree, layout, workspaces, displays, identity, profiles, reducer); it is tested on any OS. `pane-macos` is the AX, display and hotkey glue. `pane-agent` is the runtime. `pane-ffi` is the C ABI for the Swift shell, and `pane-cli` is the `pane` command. The menu bar app is in `shell/` (SwiftPM, no Xcode project). See [docs/architecture.md](docs/architecture.md).

On Linux, `make test` and `make check-mac` (which type-checks the macOS code against `aarch64-apple-darwin`) verify everything except the Swift shell.
