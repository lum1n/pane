//! `config.toml`: strict, versioned user configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::display::{DisplayRef, Position};
use crate::proto::Command;
use crate::rules::{Rule, RuleAction};
use crate::tree::ContainerLayout;

pub const CONFIG_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default = "version_one")]
    pub version: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default, rename = "display")]
    pub displays: Vec<DisplayAlias>,
    #[serde(default, rename = "workspace")]
    pub workspaces: Vec<WorkspaceConfig>,
    #[serde(default, rename = "rule")]
    pub rules: Vec<RuleConfig>,
    /// Chord -> command, merged over the defaults. `"none"` unbinds.
    #[serde(default)]
    pub shortcuts: BTreeMap<String, String>,
}

fn version_one() -> u32 {
    CONFIG_VERSION
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    pub inner_gap: f64,
    pub outer_gap: f64,
    pub default_layout: ContainerLayout,
    pub placement_memory_days: u32,
    pub restore_placements: bool,
    pub return_workspaces_on_reconnect: bool,
    pub startup_profile: Option<String>,
    /// Apps that need the size/position/size double set.
    pub electron_bundle_ids: Vec<String>,
    /// Resize step as a fraction of the container (0.05 = 5%).
    pub resize_step: f64,
    /// Windows smaller than this (either side, points) float automatically.
    pub auto_float_below: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            inner_gap: 6.0,
            outer_gap: 6.0,
            default_layout: ContainerLayout::Tiles,
            placement_memory_days: 30,
            restore_placements: true,
            return_workspaces_on_reconnect: true,
            startup_profile: None,
            electron_bundle_ids: vec![
                "com.microsoft.VSCode".into(),
                "com.tinyspeck.slackmacgap".into(),
                "com.hnc.Discord".into(),
                "notion.id".into(),
                "com.figma.Desktop".into(),
            ],
            resize_step: 0.05,
            auto_float_below: 120.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DisplayAlias {
    pub name: String,
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub match_name: Option<String>,
    #[serde(default)]
    pub position: Option<Position>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl Default for OneOrMany {
    fn default() -> Self {
        OneOrMany::Many(vec![])
    }
}

impl OneOrMany {
    pub fn to_vec(&self) -> Vec<String> {
        match self {
            OneOrMany::One(s) => vec![s.clone()],
            OneOrMany::Many(v) => v.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub name: String,
    /// Display preferences, most preferred first.
    #[serde(default)]
    pub display: OneOrMany,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleConfig {
    #[serde(default)]
    pub app: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub action: Option<RuleAction>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub display: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{}", self.render())]
pub struct ConfigError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl ConfigError {
    fn plain(message: impl Into<String>) -> Self {
        ConfigError {
            message: message.into(),
            line: None,
            column: None,
        }
    }

    pub fn render(&self) -> String {
        match (self.line, self.column) {
            (Some(l), Some(c)) => format!("line {l}, column {c}: {}", self.message),
            _ => self.message.clone(),
        }
    }
}

/// Modifier set plus key name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Chord {
    pub cmd: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub key: String,
}

impl Chord {
    pub fn parse(s: &str) -> Result<Chord, String> {
        let s = s.trim().to_ascii_lowercase();
        let (mods_part, key) = if let Some(stripped) = s.strip_suffix("--") {
            (stripped, "-".to_string())
        } else {
            match s.rsplit_once('-') {
                Some((m, k)) => (m, k.to_string()),
                None => ("", s.clone()),
            }
        };
        let mut c = Chord {
            cmd: false,
            alt: false,
            ctrl: false,
            shift: false,
            key: canonical_key(&key),
        };
        for m in mods_part.split('-').filter(|m| !m.is_empty()) {
            match m {
                "cmd" | "command" | "super" => c.cmd = true,
                "alt" | "opt" | "option" => c.alt = true,
                "ctrl" | "control" => c.ctrl = true,
                "shift" => c.shift = true,
                other => return Err(format!("unknown modifier `{other}` in `{s}`")),
            }
        }
        if crate::config::keycode(&c.key).is_none() {
            return Err(format!("unknown key `{}` in `{s}`", c.key));
        }
        Ok(c)
    }

    pub fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.cmd {
            parts.push("cmd");
        }
        if self.ctrl {
            parts.push("ctrl");
        }
        if self.alt {
            parts.push("alt");
        }
        if self.shift {
            parts.push("shift");
        }
        parts.push(&self.key);
        parts.join("-")
    }
}

fn canonical_key(k: &str) -> String {
    match k {
        "enter" => "return".into(),
        "esc" => "escape".into(),
        "," => "comma".into(),
        "." => "period".into(),
        "/" => "slash".into(),
        ";" => "semicolon".into(),
        "'" => "quote".into(),
        "[" => "leftbracket".into(),
        "]" => "rightbracket".into(),
        "-" => "minus".into(),
        "=" => "equal".into(),
        "\\" => "backslash".into(),
        "`" => "grave".into(),
        other => other.into(),
    }
}

/// macOS ANSI virtual keycode for a canonical key name.
pub fn keycode(name: &str) -> Option<u16> {
    const LETTERS: [(char, u16); 26] = [
        ('a', 0x00),
        ('b', 0x0B),
        ('c', 0x08),
        ('d', 0x02),
        ('e', 0x0E),
        ('f', 0x03),
        ('g', 0x05),
        ('h', 0x04),
        ('i', 0x22),
        ('j', 0x26),
        ('k', 0x28),
        ('l', 0x25),
        ('m', 0x2E),
        ('n', 0x2D),
        ('o', 0x1F),
        ('p', 0x23),
        ('q', 0x0C),
        ('r', 0x0F),
        ('s', 0x01),
        ('t', 0x11),
        ('u', 0x20),
        ('v', 0x09),
        ('w', 0x0D),
        ('x', 0x07),
        ('y', 0x10),
        ('z', 0x06),
    ];
    const DIGITS: [u16; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
    let mut chars = name.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if let Some((_, k)) = LETTERS.iter().find(|(l, _)| *l == c) {
            return Some(*k);
        }
        if let Some(d) = c.to_digit(10) {
            return Some(DIGITS[d as usize]);
        }
    }
    Some(match name {
        "return" => 0x24,
        "tab" => 0x30,
        "space" => 0x31,
        "backspace" | "delete" => 0x33,
        "escape" => 0x35,
        "comma" => 0x2B,
        "period" => 0x2F,
        "slash" => 0x2C,
        "semicolon" => 0x29,
        "quote" => 0x27,
        "leftbracket" => 0x21,
        "rightbracket" => 0x1E,
        "minus" => 0x1B,
        "equal" => 0x18,
        "backslash" => 0x2A,
        "grave" => 0x32,
        "left" => 0x7B,
        "right" => 0x7C,
        "down" => 0x7D,
        "up" => 0x7E,
        "f1" => 0x7A,
        "f2" => 0x78,
        "f3" => 0x63,
        "f4" => 0x76,
        "f5" => 0x60,
        "f6" => 0x61,
        "f7" => 0x62,
        "f8" => 0x64,
        "f9" => 0x65,
        "f10" => 0x6D,
        "f11" => 0x67,
        "f12" => 0x6F,
        _ => return None,
    })
}

pub fn default_shortcuts() -> Vec<(&'static str, String)> {
    let mut v: Vec<(&'static str, String)> = vec![
        ("alt-h", "focus left".into()),
        ("alt-j", "focus down".into()),
        ("alt-k", "focus up".into()),
        ("alt-l", "focus right".into()),
        ("alt-shift-h", "move left".into()),
        ("alt-shift-j", "move down".into()),
        ("alt-shift-k", "move up".into()),
        ("alt-shift-l", "move right".into()),
        ("alt-ctrl-h", "resize width -5".into()),
        ("alt-ctrl-l", "resize width +5".into()),
        ("alt-ctrl-j", "resize height +5".into()),
        ("alt-ctrl-k", "resize height -5".into()),
        ("alt-comma", "focus-display prev".into()),
        ("alt-period", "focus-display next".into()),
        ("alt-shift-comma", "move-workspace-to-display prev".into()),
        ("alt-shift-period", "move-workspace-to-display next".into()),
        ("alt-s", "toggle-orientation".into()),
        ("alt-a", "toggle-layout".into()),
        ("alt-f", "toggle-float".into()),
        ("alt-return", "swap-workspaces".into()),
        ("alt-shift-r", "reload".into()),
        ("alt-shift-p", "profile-picker".into()),
    ];
    const DIGITS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const SHOW: [&str; 9] = [
        "alt-1", "alt-2", "alt-3", "alt-4", "alt-5", "alt-6", "alt-7", "alt-8", "alt-9",
    ];
    const MOVE: [&str; 9] = [
        "alt-shift-1",
        "alt-shift-2",
        "alt-shift-3",
        "alt-shift-4",
        "alt-shift-5",
        "alt-shift-6",
        "alt-shift-7",
        "alt-shift-8",
        "alt-shift-9",
    ];
    for i in 0..9 {
        v.push((SHOW[i], format!("workspace {}", DIGITS[i])));
        v.push((MOVE[i], format!("move-to-workspace {}", DIGITS[i])));
    }
    v
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: CONFIG_VERSION,
            settings: Settings::default(),
            displays: vec![],
            workspaces: vec![],
            rules: vec![],
            shortcuts: BTreeMap::new(),
        }
    }
}

impl Config {
    pub fn parse(text: &str) -> Result<Config, ConfigError> {
        let cfg: Config = toml::from_str(text).map_err(|e| {
            let (line, column) = match e.span() {
                Some(span) => {
                    let (l, c) = line_col(text, span.start);
                    (Some(l), Some(c))
                }
                None => (None, None),
            };
            ConfigError {
                message: e.message().to_string(),
                line,
                column,
            }
        })?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != CONFIG_VERSION {
            return Err(ConfigError::plain(format!(
                "config version {} is not supported (expected {CONFIG_VERSION})",
                self.version
            )));
        }
        let s = &self.settings;
        if !(0.0..=200.0).contains(&s.inner_gap) || !(0.0..=200.0).contains(&s.outer_gap) {
            return Err(ConfigError::plain("gaps must be between 0 and 200"));
        }
        if !(0.005..=0.5).contains(&s.resize_step) {
            return Err(ConfigError::plain(
                "resize_step must be between 0.005 and 0.5",
            ));
        }
        for d in &self.displays {
            let n =
                d.uuid.is_some() as u8 + d.match_name.is_some() as u8 + d.position.is_some() as u8;
            if n != 1 {
                return Err(ConfigError::plain(format!(
                    "[[display]] `{}` needs exactly one of uuid, match_name, position",
                    d.name
                )));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for w in &self.workspaces {
            if w.name.trim().is_empty() || !seen.insert(w.name.clone()) {
                return Err(ConfigError::plain(format!(
                    "duplicate or empty workspace name `{}`",
                    w.name
                )));
            }
        }
        self.compiled_rules()?;
        self.bindings()?;
        Ok(())
    }

    /// Resolve a display name used in `[[workspace]]` or `[[rule]]`.
    pub fn display_ref(&self, s: &str) -> DisplayRef {
        if let Some(a) = self
            .displays
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(s.trim()))
        {
            if let Some(u) = &a.uuid {
                return DisplayRef::Uuid(u.clone());
            }
            if let Some(n) = &a.match_name {
                return DisplayRef::Name(n.clone());
            }
            if let Some(p) = a.position {
                return DisplayRef::Position(p);
            }
        }
        DisplayRef::parse(s)
    }

    /// Configured workspaces with their display preferences. Without any
    /// `[[workspace]]`, 1-3 prefer the left display, 4-6 the centre and 7-9
    /// the right one (all of them collapse onto one display when only one is
    /// connected).
    pub fn workspace_prefs(&self) -> Vec<(String, Vec<DisplayRef>)> {
        if self.workspaces.is_empty() {
            return (1..=9)
                .map(|i| {
                    let p = match i {
                        1..=3 => Position::Left,
                        4..=6 => Position::Center,
                        _ => Position::Right,
                    };
                    (i.to_string(), vec![DisplayRef::Position(p)])
                })
                .collect();
        }
        self.workspaces
            .iter()
            .map(|w| {
                (
                    w.name.clone(),
                    w.display
                        .to_vec()
                        .iter()
                        .map(|d| self.display_ref(d))
                        .collect(),
                )
            })
            .collect()
    }

    pub fn compiled_rules(&self) -> Result<Vec<Rule>, ConfigError> {
        self.rules
            .iter()
            .enumerate()
            .map(|(i, r)| {
                Rule::new(
                    r.app.as_deref(),
                    r.title.as_deref(),
                    r.action,
                    r.workspace.clone(),
                    r.display.as_ref().map(|d| self.display_ref(d)),
                )
                .map_err(|e| ConfigError::plain(format!("rule #{}: {e}", i + 1)))
            })
            .collect()
    }

    /// Effective hotkey bindings: defaults overlaid with `[shortcuts]`.
    pub fn bindings(&self) -> Result<Vec<(Chord, Command)>, ConfigError> {
        let mut map: BTreeMap<Chord, Option<Command>> = BTreeMap::new();
        for (k, v) in default_shortcuts() {
            let chord = Chord::parse(k).map_err(ConfigError::plain)?;
            map.insert(chord, Some(Command::parse(&v).map_err(ConfigError::plain)?));
        }
        for (k, v) in &self.shortcuts {
            let chord =
                Chord::parse(k).map_err(|e| ConfigError::plain(format!("[shortcuts]: {e}")))?;
            let cmd = if v.trim() == "none" {
                None
            } else {
                Some(
                    Command::parse(v)
                        .map_err(|e| ConfigError::plain(format!("[shortcuts] `{k}`: {e}")))?,
                )
            };
            map.insert(chord, cmd);
        }
        Ok(map
            .into_iter()
            .filter_map(|(c, cmd)| cmd.map(|cmd| (c, cmd)))
            .collect())
    }

    pub fn is_electron(&self, bundle_id: &str) -> bool {
        self.settings
            .electron_bundle_ids
            .iter()
            .any(|b| b == bundle_id)
    }
}

fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let before = &text[..offset.min(text.len())];
    let line = before.matches('\n').count() + 1;
    let col = before
        .rsplit('\n')
        .next()
        .map(|l| l.chars().count())
        .unwrap_or(0)
        + 1;
    (line, col)
}

/// Commented starter config written on first launch.
pub const STARTER_CONFIG: &str = r#"# Pane configuration. Reload with alt-shift-r or `pane reload`.
version = 1

[settings]
inner_gap = 6
outer_gap = 6
default_layout = "tiles"
placement_memory_days = 30
restore_placements = true
return_workspaces_on_reconnect = true
# startup_profile = "work"

# Name displays so workspaces and rules can refer to them.
# Match by position (left/center/right), uuid = "..." or match_name = "DELL".
[[display]]
name = "left"
position = "left"

[[display]]
name = "center"
position = "center"

[[display]]
name = "right"
position = "right"

[[workspace]]
name = "1"
display = ["left", "main"]
[[workspace]]
name = "2"
display = ["left", "main"]
[[workspace]]
name = "3"
display = ["left", "main"]
[[workspace]]
name = "4"
display = ["center", "main"]
[[workspace]]
name = "5"
display = ["center", "main"]
[[workspace]]
name = "6"
display = ["center", "main"]
[[workspace]]
name = "7"
display = ["right", "main"]
[[workspace]]
name = "8"
display = ["right", "main"]
[[workspace]]
name = "9"
display = ["right", "main"]

# [[rule]]
# app = "com.apple.finder"   # glob on bundle id
# title = "^Copy"            # regex on window title
# action = "float"           # float | tile | ignore
# workspace = "3"

[shortcuts]
# "alt-h" = "focus left"
# "alt-q" = "none"           # unbind a default
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_config_parses() {
        let c = Config::parse(STARTER_CONFIG).unwrap();
        assert_eq!(c.workspaces.len(), 9);
        assert_eq!(
            c.workspace_prefs()[0].1,
            vec![DisplayRef::Position(Position::Left), DisplayRef::Main]
        );
        assert!(c.bindings().unwrap().len() > 30);
    }

    #[test]
    fn unknown_field_reports_position() {
        let e = Config::parse("version = 1\n[settings]\ninner_gapp = 3\n").unwrap_err();
        assert_eq!(e.line, Some(3));
        assert!(e.message.contains("inner_gapp"), "{}", e.message);
    }

    #[test]
    fn bad_regex_and_chord_rejected() {
        assert!(Config::parse("[[rule]]\ntitle = \"(\"\n").is_err());
        assert!(Config::parse("[shortcuts]\n\"hyper-x\" = \"reload\"\n").is_err());
        assert!(Config::parse("[shortcuts]\n\"alt-x\" = \"frobnicate\"\n").is_err());
    }

    #[test]
    fn shortcut_override_and_unbind() {
        let c = Config::parse("[shortcuts]\n\"alt-h\" = \"none\"\n\"cmd-alt-x\" = \"reload\"\n")
            .unwrap();
        let b = c.bindings().unwrap();
        assert!(!b.iter().any(|(ch, _)| ch.display() == "alt-h"));
        assert!(b
            .iter()
            .any(|(ch, cmd)| ch.display() == "cmd-alt-x" && *cmd == Command::Reload));
    }

    #[test]
    fn chord_parse() {
        let c = Chord::parse("alt-shift-,").unwrap();
        assert!(c.alt && c.shift && c.key == "comma");
        assert_eq!(Chord::parse("ctrl--").unwrap().key, "minus");
    }
}
