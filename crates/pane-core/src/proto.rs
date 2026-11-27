//! The command/reply protocol shared by the CLI socket and the FFI.
//!
//! Wire format: one JSON object per line. Commands are tagged by `cmd`,
//! replies by `status`.

use serde::{Deserialize, Serialize};

use crate::config::ScratchpadConfig;
use crate::geometry::{Direction, Orientation, Point, Rect};
use crate::gesture::{DragPhase, GesturePhase};
use crate::profile::{Profile, ProfileEdit};
use crate::stream::EventKind;
use crate::swipe::Swipe;
use crate::workspace::Cycle;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Command {
    Focus {
        dir: Direction,
    },
    Move {
        dir: Direction,
    },
    /// Grow (`delta > 0`) or shrink the focused window by `delta` (fraction).
    Resize {
        axis: Orientation,
        delta: f64,
    },
    /// Show workspace `name` of the focused display, or `<display>:<name>`.
    Workspace {
        name: String,
    },
    /// Move the focused window to workspace `name` (same target syntax).
    MoveToWorkspace {
        name: String,
    },
    /// Show the next/previous workspace on the focused display (own
    /// workspaces with windows, the visible one, and guests).
    CycleWorkspace {
        which: Cycle,
    },
    FocusDisplay {
        which: Cycle,
    },
    MoveWorkspaceToDisplay {
        which: Cycle,
    },
    ToggleOrientation,
    ToggleLayout,
    /// Give the tiles of the focused container equal sizes.
    Balance,
    /// Arrange the focused workspace's tiles in a preset and keep that
    /// shape as tiles come and go (`manual` stops it).
    Layout {
        preset: LayoutPreset,
    },
    ToggleFloat,
    /// Zoom the focused tile to fill its display (again to restore).
    ToggleFullscreen,
    /// Swap the visible workspaces of the focused and the next display.
    SwapWorkspaces,
    /// Put back the layout from before the last tree-changing command.
    Undo,
    Reload,
    ProfilePicker,
    OpenEditor,
    Recover {
        #[serde(default)]
        all_apps: bool,
    },
    Query {
        what: QueryKind,
    },
    ProfileCapture {
        #[serde(default)]
        name: Option<String>,
    },
    /// Apply a profile given inline or by name (the runtime loads it).
    ProfileApply {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        profile: Option<Profile>,
    },
    ProfileSave {
        profile: Profile,
        #[serde(default)]
        startup: bool,
    },
    ProfileLoad {
        name: String,
    },
    ProfileDelete {
        name: String,
    },
    ProfileList,
    /// Make profile `name` apply by itself on the displays connected now
    /// (`here: true`), or never by itself (`here: false`). Handled by the
    /// runtime, which edits the profile's `[when]`.
    ProfileWhen {
        name: String,
        here: bool,
    },
    /// How the live desk differs from saved profile `name` (default: the
    /// active one): a `drift::ProfileDiff`. Handled by the runtime.
    ProfileDiff {
        #[serde(default)]
        name: Option<String>,
    },
    /// Save profile `name` (default: the active one) as updated from the
    /// live desk (`ProfileDiff.updated`). Handled by the runtime.
    ProfileUpdate {
        #[serde(default)]
        name: Option<String>,
    },
    /// Apply an editor operation to a profile and return the result.
    ProfileEdit {
        profile: Profile,
        edit: ProfileEdit,
    },
    /// Set a `[settings]` value in config.toml (JSON null removes it) and reload.
    SetSetting {
        key: String,
        value: serde_json::Value,
    },
    /// Replace every `[[scratchpad]]` in config.toml with these and reload.
    SetScratchpads {
        scratchpads: Vec<ScratchpadConfig>,
    },
    /// Bind `command` to `chord` instead of whatever it had; `chord: None`
    /// leaves it unbound. Handled by the runtime (edits `[shortcuts]`).
    Rebind {
        command: String,
        #[serde(default)]
        chord: Option<String>,
    },
    /// Drop every `[shortcuts]` override (back to the defaults).
    ResetShortcuts,
    /// Pause hotkeys (while Settings records a new shortcut).
    SuspendHotkeys {
        suspended: bool,
    },
    Ping,
    /// Health report (`pane doctor`): permissions, displays, troubled apps,
    /// the config error. Handled by the runtime.
    Doctor,
    /// Focus a window wherever it is: show its workspace (on its display)
    /// first, and restore it if minimized.
    FocusWindow {
        wid: u32,
    },
    /// Show the scratchpad `name` (`[[scratchpad]]`) on the focused display,
    /// or hide it if it is shown and focused. Opens the app if needed.
    Scratchpad {
        name: String,
    },
    /// Bring a window of `app` (bundle id or app name) to the focused
    /// workspace, next to the focused tile, and focus it. Opens the app if
    /// it has no window.
    Summon {
        app: String,
    },
    /// Open the window and command palette.
    Palette,
    /// Rank windows, workspaces, profiles and commands against `query`
    /// (`palette::search`). The runtime fills in `profiles`.
    Search {
        #[serde(default)]
        query: String,
        #[serde(default)]
        profiles: Vec<String>,
    },
    /// Input from the gesture tracker (hold the modifier and point). Not in
    /// the text form; send JSON to test it.
    Gesture {
        phase: GesturePhase,
    },
    /// The pointer moved to `at` with no button held (focus follows
    /// mouse). Not in the text form.
    Hover {
        at: Point,
    },
    /// The pointer with the left button held, and its release (title-bar
    /// drag of a tiled window). Not in the text form.
    Drag {
        phase: DragPhase,
    },
    /// A trackpad swipe with the pointer at `at`: three fingers left/right
    /// cycle the workspaces of the display under the pointer, four fingers
    /// up open the palette. Not in the text form.
    Swipe {
        swipe: Swipe,
        at: Point,
    },
    /// Stream events (`stream::StreamEvent`) as JSON lines after the
    /// `{"status":"ok"}` reply. Only works over the control socket. An empty
    /// list means every kind except `snapshot`.
    Subscribe {
        #[serde(default)]
        events: Vec<EventKind>,
    },
}

/// What `layout` arranges a workspace into.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "layout", rename_all = "kebab-case")]
pub enum LayoutPreset {
    /// One large tile and a stacked column. `ratio` is the master's share
    /// of the width; unset uses `[settings] master_ratio`.
    MasterStack {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ratio: Option<f64>,
    },
    /// `n` equal columns.
    Columns { n: usize },
    /// Stop keeping a preset; the tiles stay where they are.
    Manual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryKind {
    Windows,
    /// Apps Pane observes, with window counts.
    Apps,
    Displays,
    Workspaces,
    Tree,
    Snapshot,
    Config,
    /// Hotkey table (`config::BindingInfo` rows).
    Bindings,
    /// Key names to macOS keycodes.
    Keys,
}

impl Command {
    /// Parse the text form used in `[shortcuts]` and by the CLI.
    pub fn parse(s: &str) -> Result<Command, String> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        let arg = |i: usize| {
            parts
                .get(i)
                .copied()
                .ok_or_else(|| format!("`{s}`: missing argument"))
        };
        let dir = |i: usize| {
            arg(i).and_then(|a| {
                Direction::parse(a).ok_or_else(|| format!("`{s}`: bad direction `{a}`"))
            })
        };
        let cycle = |i: usize| {
            arg(i).and_then(|a| match a {
                "next" => Ok(Cycle::Next),
                "prev" | "previous" => Ok(Cycle::Prev),
                _ => Err(format!("`{s}`: expected next or prev")),
            })
        };
        let cmd = match parts.first().copied().unwrap_or("") {
            "focus" => Command::Focus { dir: dir(1)? },
            "move" => Command::Move { dir: dir(1)? },
            "resize" => {
                let axis = match arg(1)? {
                    "width" | "w" => Orientation::Horizontal,
                    "height" | "h" => Orientation::Vertical,
                    a => return Err(format!("`{s}`: bad axis `{a}`")),
                };
                let pct: f64 = arg(2)?
                    .trim_start_matches('+')
                    .parse()
                    .map_err(|_| format!("`{s}`: bad amount"))?;
                Command::Resize {
                    axis,
                    delta: pct / 100.0,
                }
            }
            "workspace" => Command::Workspace {
                name: arg(1)?.to_string(),
            },
            "move-to-workspace" => Command::MoveToWorkspace {
                name: arg(1)?.to_string(),
            },
            "cycle-workspace" => Command::CycleWorkspace { which: cycle(1)? },
            "focus-display" => Command::FocusDisplay { which: cycle(1)? },
            "move-workspace-to-display" => Command::MoveWorkspaceToDisplay { which: cycle(1)? },
            "toggle-orientation" => Command::ToggleOrientation,
            "toggle-layout" => Command::ToggleLayout,
            "balance" => Command::Balance,
            "layout" => {
                let preset = match arg(1)? {
                    "master-stack" | "master" => LayoutPreset::MasterStack {
                        ratio: match parts.get(2) {
                            None => None,
                            Some(a) => {
                                let pct: f64 = a
                                    .trim_end_matches('%')
                                    .parse()
                                    .map_err(|_| format!("`{s}`: bad master size `{a}`"))?;
                                if !(10.0..=90.0).contains(&pct) {
                                    return Err(format!(
                                        "`{s}`: master size must be 10 to 90 (percent)"
                                    ));
                                }
                                Some(pct / 100.0)
                            }
                        },
                    },
                    "columns" => {
                        let n: usize = arg(2)?
                            .parse()
                            .map_err(|_| format!("`{s}`: bad column count"))?;
                        if !(1..=crate::workspace::MAX_COLUMNS).contains(&n) {
                            return Err(format!(
                                "`{s}`: columns must be 1 to {}",
                                crate::workspace::MAX_COLUMNS
                            ));
                        }
                        LayoutPreset::Columns { n }
                    }
                    "manual" | "off" => LayoutPreset::Manual,
                    a => {
                        return Err(format!(
                            "`{s}`: unknown layout `{a}` (master-stack, columns N, manual)"
                        ))
                    }
                };
                Command::Layout { preset }
            }
            "toggle-float" => Command::ToggleFloat,
            "toggle-fullscreen" => Command::ToggleFullscreen,
            "swap-workspaces" => Command::SwapWorkspaces,
            "undo" => Command::Undo,
            "reload" => Command::Reload,
            "profile-picker" => Command::ProfilePicker,
            "open-editor" => Command::OpenEditor,
            "palette" => Command::Palette,
            "focus-window" => Command::FocusWindow {
                wid: arg(1)?
                    .parse()
                    .map_err(|_| format!("`{s}`: bad window id"))?,
            },
            "scratchpad" => Command::Scratchpad {
                name: arg(1)?.to_string(),
            },
            "summon" => {
                arg(1)?;
                Command::Summon {
                    app: parts[1..].join(" "),
                }
            }
            "search" => Command::Search {
                query: parts[1..].join(" "),
                profiles: vec![],
            },
            "recover" => Command::Recover {
                all_apps: parts.get(1) == Some(&"--all-apps"),
            },
            "profile-apply" => Command::ProfileApply {
                name: Some(arg(1)?.to_string()),
                profile: None,
            },
            "profile-diff" => Command::ProfileDiff {
                name: (parts.len() > 1).then(|| parts[1..].join(" ")),
            },
            "profile-update" => Command::ProfileUpdate {
                name: (parts.len() > 1).then(|| parts[1..].join(" ")),
            },
            "profile-when" => {
                let here = match arg(1)? {
                    "here" => true,
                    "off" => false,
                    _ => return Err(format!("`{s}`: use `profile-when here|off NAME`")),
                };
                arg(2)?;
                Command::ProfileWhen {
                    name: parts[2..].join(" "),
                    here,
                }
            }
            "subscribe" => Command::Subscribe {
                events: parts[1..]
                    .iter()
                    .map(|k| {
                        EventKind::parse(k).ok_or_else(|| format!("`{s}`: unknown event `{k}`"))
                    })
                    .collect::<Result<_, _>>()?,
            },
            other => return Err(format!("unknown command `{other}`")),
        };
        Ok(cmd)
    }

    /// Commands that only read state.
    pub fn is_query(&self) -> bool {
        matches!(
            self,
            Command::Query { .. }
                | Command::Ping
                | Command::Doctor
                | Command::ProfileList
                | Command::ProfileLoad { .. }
                | Command::ProfileDiff { .. }
                | Command::Search { .. }
                | Command::Subscribe { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Reply {
    Ok {
        #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
        data: serde_json::Value,
    },
    Error {
        message: String,
    },
}

impl Reply {
    pub fn ok() -> Reply {
        Reply::Ok {
            data: serde_json::Value::Null,
        }
    }

    pub fn data<T: Serialize>(v: &T) -> Reply {
        match serde_json::to_value(v) {
            Ok(data) => Reply::Ok { data },
            Err(e) => Reply::err(e.to_string()),
        }
    }

    pub fn err(m: impl Into<String>) -> Reply {
        Reply::Error { message: m.into() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

/// Requests from the core to the UI shell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "request", rename_all = "snake_case")]
pub enum UiRequest {
    LaunchApp {
        request_id: u64,
        bundle_id: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Notice {
        level: NoticeLevel,
        message: String,
    },
    OpenEditor,
    ProfilePicker,
    /// Show the window and command palette (toggle if already open).
    Palette,
    AxPermissionMissing,
    /// Show where a window moved by a gesture will go: `frame` is the frame
    /// it will get, `target` the tile being split (if any). Global
    /// top-left points.
    GestureOverlay {
        frame: Rect,
        #[serde(default)]
        target: Option<Rect>,
        label: String,
    },
    GestureOverlayHide,
}

impl UiRequest {
    pub fn info(m: impl Into<String>) -> UiRequest {
        UiRequest::Notice {
            level: NoticeLevel::Info,
            message: m.into(),
        }
    }

    pub fn warn(m: impl Into<String>) -> UiRequest {
        UiRequest::Notice {
            level: NoticeLevel::Warning,
            message: m.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_wire_format() {
        assert_eq!(
            Command::parse("resize width -5").unwrap(),
            Command::Resize {
                axis: Orientation::Horizontal,
                delta: -0.05
            }
        );
        assert_eq!(
            Command::parse("focus-display next").unwrap(),
            Command::FocusDisplay { which: Cycle::Next }
        );
        let j = serde_json::to_string(&Command::Workspace { name: "3".into() }).unwrap();
        assert_eq!(j, r#"{"cmd":"workspace","name":"3"}"#);
        let r: Command = serde_json::from_str(r#"{"cmd":"query","what":"windows"}"#).unwrap();
        assert_eq!(
            r,
            Command::Query {
                what: QueryKind::Windows
            }
        );
        assert_eq!(
            Command::parse("focus-window 42").unwrap(),
            Command::FocusWindow { wid: 42 }
        );
        assert_eq!(
            Command::parse("search web  dev").unwrap(),
            Command::Search {
                query: "web dev".into(),
                profiles: vec![]
            }
        );
        assert!(Command::parse("focus-window x").is_err());
        assert_eq!(
            Command::parse("scratchpad term").unwrap(),
            Command::Scratchpad {
                name: "term".into()
            }
        );
        assert_eq!(
            Command::parse("summon Visual Studio Code").unwrap(),
            Command::Summon {
                app: "Visual Studio Code".into()
            }
        );
        assert!(Command::parse("summon").is_err());
        assert_eq!(
            Command::parse("profile-when here Home office").unwrap(),
            Command::ProfileWhen {
                name: "Home office".into(),
                here: true
            }
        );
        assert_eq!(
            Command::parse("profile-when off work").unwrap(),
            Command::ProfileWhen {
                name: "work".into(),
                here: false
            }
        );
        assert!(Command::parse("profile-when here").is_err());
        assert_eq!(
            Command::parse("profile-diff").unwrap(),
            Command::ProfileDiff { name: None }
        );
        assert_eq!(
            Command::parse("profile-update Home office").unwrap(),
            Command::ProfileUpdate {
                name: Some("Home office".into())
            }
        );
        assert!(Command::parse("profile-diff work").unwrap().is_query());
        assert!(Command::parse("profile-when always work").is_err());
        assert_eq!(
            Command::parse("subscribe").unwrap(),
            Command::Subscribe { events: vec![] }
        );
        assert_eq!(
            Command::parse("subscribe focus workspace-shown").unwrap(),
            Command::Subscribe {
                events: vec![EventKind::Focus, EventKind::WorkspaceShown]
            }
        );
        assert!(Command::parse("subscribe nonsense").is_err());
        assert_eq!(Command::parse("balance").unwrap(), Command::Balance);
        assert_eq!(
            Command::parse("layout master-stack").unwrap(),
            Command::Layout {
                preset: LayoutPreset::MasterStack { ratio: None }
            }
        );
        assert_eq!(
            Command::parse("layout master-stack 65").unwrap(),
            Command::Layout {
                preset: LayoutPreset::MasterStack { ratio: Some(0.65) }
            }
        );
        assert_eq!(
            Command::parse("layout columns 3").unwrap(),
            Command::Layout {
                preset: LayoutPreset::Columns { n: 3 }
            }
        );
        assert_eq!(
            Command::parse("layout manual").unwrap(),
            Command::Layout {
                preset: LayoutPreset::Manual
            }
        );
        assert!(Command::parse("layout").is_err());
        assert!(Command::parse("layout columns").is_err());
        assert!(Command::parse("layout columns 0").is_err());
        assert!(Command::parse("layout master-stack 95").is_err());
        assert!(Command::parse("layout spiral").is_err());
        let j = serde_json::to_string(&Command::Layout {
            preset: LayoutPreset::Columns { n: 2 },
        })
        .unwrap();
        assert_eq!(j, r#"{"cmd":"layout","preset":{"layout":"columns","n":2}}"#);
        let r: Command = serde_json::from_str(r#"{"cmd":"subscribe"}"#).unwrap();
        assert_eq!(r, Command::Subscribe { events: vec![] });
        assert!(r.is_query());
        assert_eq!(
            serde_json::to_string(&Reply::ok()).unwrap(),
            r#"{"status":"ok"}"#
        );
    }
}
