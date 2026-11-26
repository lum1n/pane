//! The command/reply protocol shared by the CLI socket and the FFI.
//!
//! Wire format: one JSON object per line. Commands are tagged by `cmd`,
//! replies by `status`.

use serde::{Deserialize, Serialize};

use crate::geometry::{Direction, Orientation};
use crate::profile::{Profile, ProfileEdit};
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
    Workspace {
        name: String,
    },
    MoveToWorkspace {
        name: String,
    },
    FocusDisplay {
        which: Cycle,
    },
    MoveWorkspaceToDisplay {
        which: Cycle,
    },
    ToggleOrientation,
    ToggleLayout,
    ToggleFloat,
    /// Swap the visible workspaces of the focused and the next display.
    SwapWorkspaces,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QueryKind {
    Windows,
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
            "focus-display" => Command::FocusDisplay { which: cycle(1)? },
            "move-workspace-to-display" => Command::MoveWorkspaceToDisplay { which: cycle(1)? },
            "toggle-orientation" => Command::ToggleOrientation,
            "toggle-layout" => Command::ToggleLayout,
            "toggle-float" => Command::ToggleFloat,
            "swap-workspaces" => Command::SwapWorkspaces,
            "reload" => Command::Reload,
            "profile-picker" => Command::ProfilePicker,
            "open-editor" => Command::OpenEditor,
            "recover" => Command::Recover {
                all_apps: parts.get(1) == Some(&"--all-apps"),
            },
            "profile-apply" => Command::ProfileApply {
                name: Some(arg(1)?.to_string()),
                profile: None,
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
                | Command::ProfileList
                | Command::ProfileLoad { .. }
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
    AxPermissionMissing,
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
            serde_json::to_string(&Reply::ok()).unwrap(),
            r#"{"status":"ok"}"#
        );
    }
}
