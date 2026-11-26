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
    /// Ignored: workspaces are per display now. Still accepted so configs
    /// from earlier versions load.
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
    /// No effect: workspaces always return to their own display.
    pub return_workspaces_on_reconnect: bool,
    pub startup_profile: Option<String>,
    /// Apps that need the size/position/size double set.
    pub electron_bundle_ids: Vec<String>,
    /// Resize step as a fraction of the container (0.05 = 5%).
    pub resize_step: f64,
    /// Windows smaller than this (either side, points) float automatically.
    pub auto_float_below: f64,
    /// Modifier(s) behind every default hotkey, e.g. "alt", "ctrl-alt",
    /// "cmd-ctrl", "hyper" (cmd-ctrl-alt-shift) or "meh" (ctrl-alt-shift).
    /// Also what `mod` means in `[shortcuts]`.
    pub modifier: String,
    /// Apps whose windows always start floating: bundle ids (or executable
    /// names for unbundled apps), `*` globs allowed.
    pub float_apps: Vec<String>,
    /// Apps whose windows always tile, overriding automatic floating (for
    /// apps with unusual windows). Explicit `[[rule]]`s still win.
    pub tile_apps: Vec<String>,
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
            modifier: "alt".into(),
            float_apps: vec![],
            tile_apps: vec![],
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

/// A set of modifier keys.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct Mods {
    pub cmd: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
}

impl Mods {
    pub const ALT: Mods = Mods {
        cmd: false,
        alt: true,
        ctrl: false,
        shift: false,
    };
    pub const CTRL: Mods = Mods {
        cmd: false,
        alt: false,
        ctrl: true,
        shift: false,
    };
    pub const SHIFT: Mods = Mods {
        cmd: false,
        alt: false,
        ctrl: false,
        shift: true,
    };
    pub const CMD: Mods = Mods {
        cmd: true,
        alt: false,
        ctrl: false,
        shift: false,
    };

    /// Add one modifier token (`cmd`, `alt`, `ctrl`, `shift`, `hyper`,
    /// `meh`, or `mod` = `base`). Returns false for an unknown token.
    fn add_token(&mut self, t: &str, base: &Mods) -> bool {
        match t {
            "cmd" | "command" | "super" => self.cmd = true,
            "alt" | "opt" | "option" => self.alt = true,
            "ctrl" | "control" => self.ctrl = true,
            "shift" => self.shift = true,
            "hyper" => {
                *self = self.union(Mods {
                    cmd: true,
                    alt: true,
                    ctrl: true,
                    shift: true,
                })
            }
            "meh" => {
                *self = self.union(Mods {
                    cmd: false,
                    alt: true,
                    ctrl: true,
                    shift: true,
                })
            }
            "mod" => *self = self.union(*base),
            _ => return false,
        }
        true
    }

    /// Parse a modifier setting such as `ctrl-alt` or `hyper`. At least one
    /// of cmd, ctrl and alt is required: shift alone would eat typing.
    pub fn parse(s: &str) -> Result<Mods, String> {
        let mut m = Mods::default();
        for t in s
            .trim()
            .to_ascii_lowercase()
            .split(['-', '+', ' '])
            .filter(|t| !t.is_empty())
        {
            if t == "mod" || !m.add_token(t, &Mods::default()) {
                return Err(format!(
                    "unknown modifier `{t}` in `{s}` (use cmd, ctrl, alt, shift, hyper, meh)"
                ));
            }
        }
        if !(m.cmd || m.ctrl || m.alt) {
            return Err(format!(
                "modifier `{s}` needs at least one of cmd, ctrl, alt"
            ));
        }
        Ok(m)
    }

    pub fn union(self, o: Mods) -> Mods {
        Mods {
            cmd: self.cmd || o.cmd,
            alt: self.alt || o.alt,
            ctrl: self.ctrl || o.ctrl,
            shift: self.shift || o.shift,
        }
    }

    pub fn contains(self, o: Mods) -> bool {
        self.union(o) == self
    }

    /// `cmd-ctrl-alt-shift` order, without a key.
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
        parts.join("-")
    }

    /// `self` plus the first modifier from `prefs` it does not have yet.
    fn layer(self, prefs: &[Mods], avoid: Option<Mods>) -> Option<Mods> {
        prefs
            .iter()
            .find(|p| !self.contains(**p) && Some(self.union(**p)) != avoid)
            .map(|p| self.union(*p))
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
    /// Parse with `mod` meaning alt.
    pub fn parse(s: &str) -> Result<Chord, String> {
        Chord::parse_with(s, &Mods::ALT)
    }

    /// Parse `mods-key`, where `mod` in the modifiers stands for `base`.
    pub fn parse_with(s: &str, base: &Mods) -> Result<Chord, String> {
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
        let mut mods = Mods::default();
        for m in mods_part.split('-').filter(|m| !m.is_empty()) {
            if !mods.add_token(m, base) {
                return Err(format!("unknown modifier `{m}` in `{s}`"));
            }
        }
        c.cmd = mods.cmd;
        c.alt = mods.alt;
        c.ctrl = mods.ctrl;
        c.shift = mods.shift;
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

/// Canonical key names and their macOS ANSI virtual keycodes.
pub const KEYS: &[(&str, u16)] = &[
    ("a", 0x00),
    ("s", 0x01),
    ("d", 0x02),
    ("f", 0x03),
    ("h", 0x04),
    ("g", 0x05),
    ("z", 0x06),
    ("x", 0x07),
    ("c", 0x08),
    ("v", 0x09),
    ("b", 0x0B),
    ("q", 0x0C),
    ("w", 0x0D),
    ("e", 0x0E),
    ("r", 0x0F),
    ("y", 0x10),
    ("t", 0x11),
    ("1", 0x12),
    ("2", 0x13),
    ("3", 0x14),
    ("4", 0x15),
    ("6", 0x16),
    ("5", 0x17),
    ("equal", 0x18),
    ("9", 0x19),
    ("7", 0x1A),
    ("minus", 0x1B),
    ("8", 0x1C),
    ("0", 0x1D),
    ("rightbracket", 0x1E),
    ("o", 0x1F),
    ("u", 0x20),
    ("leftbracket", 0x21),
    ("i", 0x22),
    ("p", 0x23),
    ("return", 0x24),
    ("l", 0x25),
    ("j", 0x26),
    ("quote", 0x27),
    ("k", 0x28),
    ("semicolon", 0x29),
    ("backslash", 0x2A),
    ("comma", 0x2B),
    ("slash", 0x2C),
    ("n", 0x2D),
    ("m", 0x2E),
    ("period", 0x2F),
    ("tab", 0x30),
    ("space", 0x31),
    ("grave", 0x32),
    ("backspace", 0x33),
    ("escape", 0x35),
    ("f5", 0x60),
    ("f6", 0x61),
    ("f7", 0x62),
    ("f3", 0x63),
    ("f8", 0x64),
    ("f9", 0x65),
    ("f11", 0x67),
    ("f10", 0x6D),
    ("f12", 0x6F),
    ("f4", 0x76),
    ("f2", 0x78),
    ("f1", 0x7A),
    ("left", 0x7B),
    ("right", 0x7C),
    ("down", 0x7D),
    ("up", 0x7E),
];

/// macOS ANSI virtual keycode for a canonical key name.
pub fn keycode(name: &str) -> Option<u16> {
    let name = if name == "delete" { "backspace" } else { name };
    KEYS.iter().find(|(n, _)| *n == name).map(|(_, k)| *k)
}

/// One row of the hotkey table shown in Settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingInfo {
    /// Command in shortcut syntax, e.g. `focus left`.
    pub command: String,
    /// Chords currently bound to it.
    pub chords: Vec<String>,
    /// Chord the defaults would give it, if any.
    pub default: Option<String>,
}

/// Default bindings built on `base`.
///
/// Three layers: `base` (focus, workspaces, toggles), "move" = `base` plus
/// shift (or the next free modifier), and "resize" = `base` plus ctrl (or
/// the next free one, never the same as move). With all four modifiers in
/// `base` there is nothing left to add, so move and resize get no defaults.
pub fn default_shortcuts(base: &Mods) -> Vec<(String, String)> {
    let mv = base.layer(&[Mods::SHIFT, Mods::CTRL, Mods::ALT, Mods::CMD], None);
    let rs = base.layer(&[Mods::CTRL, Mods::ALT, Mods::CMD, Mods::SHIFT], mv);
    let chord = |m: &Mods, key: &str| format!("{}-{key}", m.display());
    let mut v: Vec<(String, String)> = Vec::new();
    let mut add = |m: Option<Mods>, key: &str, cmd: String| {
        if let Some(m) = m {
            v.push((chord(&m, key), cmd));
        }
    };
    let b = Some(*base);
    for (key, dir) in [("h", "left"), ("j", "down"), ("k", "up"), ("l", "right")] {
        add(b, key, format!("focus {dir}"));
        add(mv, key, format!("move {dir}"));
    }
    add(rs, "h", "resize width -5".into());
    add(rs, "l", "resize width +5".into());
    add(rs, "j", "resize height +5".into());
    add(rs, "k", "resize height -5".into());
    add(b, "comma", "focus-display prev".into());
    add(b, "period", "focus-display next".into());
    add(mv, "comma", "move-workspace-to-display prev".into());
    add(mv, "period", "move-workspace-to-display next".into());
    add(b, "s", "toggle-orientation".into());
    add(b, "a", "toggle-layout".into());
    add(b, "f", "toggle-float".into());
    add(b, "return", "swap-workspaces".into());
    add(b, "leftbracket", "cycle-workspace prev".into());
    add(b, "rightbracket", "cycle-workspace next".into());
    add(mv, "r", "reload".into());
    add(mv, "f", "toggle-fullscreen".into());
    add(mv, "p", "profile-picker".into());
    for i in 1..=9 {
        add(b, &i.to_string(), format!("workspace {i}"));
        add(mv, &i.to_string(), format!("move-to-workspace {i}"));
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
        self.compiled_rules()?;
        self.bindings()?;
        Ok(())
    }

    /// Resolve a display name used in `[[rule]]`, profiles or targets.
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

    /// `[[rule]]`s in order, then one float rule per `float_apps` entry (so
    /// an explicit rule for the same app wins).
    pub fn compiled_rules(&self) -> Result<Vec<Rule>, ConfigError> {
        let list = |apps: &[String], action: RuleAction, name: &'static str| {
            apps.iter()
                .map(move |app| {
                    Rule::new(Some(app), None, Some(action), None, None)
                        .map_err(|e| ConfigError::plain(format!("{name} `{app}`: {e}")))
                })
                .collect::<Vec<_>>()
        };
        let float_apps = list(&self.settings.float_apps, RuleAction::Float, "float_apps")
            .into_iter()
            .chain(list(
                &self.settings.tile_apps,
                RuleAction::Tile,
                "tile_apps",
            ));
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
            .chain(float_apps)
            .collect()
    }

    /// Effective hotkey bindings: defaults overlaid with `[shortcuts]`.
    pub fn bindings(&self) -> Result<Vec<(Chord, Command)>, ConfigError> {
        let base = self.modifier()?;
        let mut map: BTreeMap<Chord, Option<Command>> = BTreeMap::new();
        for (k, v) in default_shortcuts(&base) {
            let chord = Chord::parse_with(&k, &base).map_err(ConfigError::plain)?;
            map.insert(chord, Some(Command::parse(&v).map_err(ConfigError::plain)?));
        }
        for (k, v) in &self.shortcuts {
            let chord = Chord::parse_with(k, &base)
                .map_err(|e| ConfigError::plain(format!("[shortcuts]: {e}")))?;
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

    /// Every bindable action with its current and default chords: the
    /// defaults first (in their order), then commands only `[shortcuts]` binds.
    pub fn binding_table(&self) -> Result<Vec<BindingInfo>, ConfigError> {
        let base = self.modifier()?;
        let current = self.bindings()?;
        let mut rows: Vec<(Command, BindingInfo)> = Vec::new();
        for (chord, text) in default_shortcuts(&base) {
            let cmd = Command::parse(&text).map_err(ConfigError::plain)?;
            let chord = Chord::parse_with(&chord, &base)
                .map_err(ConfigError::plain)?
                .display();
            match rows.iter_mut().find(|(c, _)| *c == cmd) {
                Some((_, r)) => {
                    r.default.get_or_insert(chord);
                }
                None => rows.push((
                    cmd,
                    BindingInfo {
                        command: text,
                        chords: vec![],
                        default: Some(chord),
                    },
                )),
            }
        }
        for text in self.shortcuts.values() {
            if text.trim() == "none" {
                continue;
            }
            if let Ok(cmd) = Command::parse(text) {
                if !rows.iter().any(|(c, _)| *c == cmd) {
                    rows.push((
                        cmd,
                        BindingInfo {
                            command: text.trim().to_string(),
                            chords: vec![],
                            default: None,
                        },
                    ));
                }
            }
        }
        for (chord, cmd) in current {
            if let Some((_, r)) = rows.iter_mut().find(|(c, _)| *c == cmd) {
                r.chords.push(chord.display());
            }
        }
        Ok(rows.into_iter().map(|(_, r)| r).collect())
    }

    /// The `modifier` setting, parsed.
    pub fn modifier(&self) -> Result<Mods, ConfigError> {
        Mods::parse(&self.settings.modifier)
            .map_err(|e| ConfigError::plain(format!("[settings] modifier: {e}")))
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

# The key(s) behind every default hotkey: "alt", "ctrl-alt", "cmd-ctrl",
# "hyper" (cmd-ctrl-alt-shift), "meh" (ctrl-alt-shift), ...
# Moving adds shift and resizing adds ctrl (or the next free modifier).
modifier = "alt"

# Apps whose windows always start floating (bundle ids, or the executable
# name for apps started from a terminal; * globs work).
# float_apps = ["com.apple.calculator", "com.apple.systempreferences"]
# Apps whose windows always tile, even if Pane would float them.
# tile_apps = ["my-app"]

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

# Every display has its own workspaces 1-9: mod-N shows workspace N on the
# display you are working on.

# [[rule]]
# app = "com.apple.finder"   # glob on bundle id
# title = "^Copy"            # regex on window title
# action = "float"           # float | tile | ignore
# workspace = "3"           # workspace 3 of the display the window opens on
# display = "right"          # ...or of this display

[shortcuts]
# "mod" means the modifier above.
# "mod-h" = "focus left"
# "mod-q" = "none"           # unbind a default
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starter_config_parses() {
        let c = Config::parse(STARTER_CONFIG).unwrap();
        assert!(c.workspaces.is_empty());
        // Old configs with [[workspace]] still load.
        assert!(
            Config::parse("[[workspace]]\nname = \"1\"\ndisplay = [\"left\", \"main\"]\n").is_ok()
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
        assert!(Config::parse("[shortcuts]\n\"fn-x\" = \"reload\"\n").is_err());
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
    fn modifier_setting_moves_every_default() {
        let c = Config::parse("[settings]\nmodifier = \"ctrl-alt\"\n").unwrap();
        let b = c.bindings().unwrap();
        let find = |cmd: &str| {
            b.iter()
                .find(|(_, x)| *x == Command::parse(cmd).unwrap())
                .map(|(ch, _)| ch.display())
        };
        assert_eq!(find("focus left").as_deref(), Some("ctrl-alt-h"));
        assert_eq!(find("move left").as_deref(), Some("ctrl-alt-shift-h"));
        // ctrl is taken by the base, so resize adds cmd.
        assert_eq!(find("resize width +5").as_deref(), Some("cmd-ctrl-alt-l"));
        assert!(!b.iter().any(|(ch, _)| ch.display() == "alt-h"));
    }

    #[test]
    fn layers_never_collide() {
        for m in [
            "alt",
            "ctrl",
            "cmd",
            "ctrl-alt",
            "cmd-alt",
            "cmd-ctrl",
            "meh",
            "alt-shift",
            "hyper",
        ] {
            let c = Config::parse(&format!("[settings]\nmodifier = \"{m}\"\n")).unwrap();
            let defaults = default_shortcuts(&c.modifier().unwrap());
            let mut chords: Vec<String> = defaults.iter().map(|d| d.0.clone()).collect();
            chords.sort();
            let n = chords.len();
            chords.dedup();
            assert_eq!(chords.len(), n, "{m}: duplicate default chords");
            assert!(c.bindings().unwrap().len() >= 19, "{m}");
        }
        // Hyper uses every modifier: only the base layer remains.
        let c = Config::parse("[settings]\nmodifier = \"hyper\"\n").unwrap();
        assert!(!c
            .bindings()
            .unwrap()
            .iter()
            .any(|(_, x)| matches!(x, Command::Move { .. })));
    }

    #[test]
    fn mod_token_and_bad_modifier() {
        let c = Config::parse(
            "[settings]\nmodifier = \"cmd-ctrl\"\n[shortcuts]\n\"mod-x\" = \"reload\"\n",
        )
        .unwrap();
        assert!(c
            .bindings()
            .unwrap()
            .iter()
            .any(|(ch, cmd)| ch.display() == "cmd-ctrl-x" && *cmd == Command::Reload));
        assert!(Config::parse("[settings]\nmodifier = \"shift\"\n").is_err());
        assert!(Config::parse("[settings]\nmodifier = \"fn\"\n").is_err());
    }

    #[test]
    fn binding_table_reflects_overrides() {
        let c = Config::parse("[shortcuts]\n\"alt-h\" = \"none\"\n\"cmd-alt-h\" = \"focus left\"\n\"alt-x\" = \"recover\"\n").unwrap();
        let t = c.binding_table().unwrap();
        let row = |cmd: &str| t.iter().find(|r| r.command == cmd).unwrap().clone();
        assert_eq!(row("focus left").chords, vec!["cmd-alt-h"]);
        assert_eq!(row("focus left").default.as_deref(), Some("alt-h"));
        assert_eq!(row("recover").chords, vec!["alt-x"]);
        assert_eq!(row("recover").default, None);
    }

    #[test]
    fn float_apps_become_float_rules() {
        use crate::rules::{decide, WindowRole, WindowTraits};
        let c = Config::parse("[settings]\nfloat_apps = [\"com.apple.calculator\", \"my-*\"]\n[[rule]]\napp = \"my-tool\"\naction = \"tile\"\n").unwrap();
        let rules = c.compiled_rules().unwrap();
        let w = |b: &'static str| WindowTraits {
            bundle_id: b,
            title: "x",
            role: WindowRole::Standard,
            resizable: true,
            modal: false,
            has_close_button: true,
            width: 800.0,
            height: 600.0,
            regular_app: true,
        };
        assert_eq!(
            decide(&rules, &w("com.apple.calculator"), 120.0).action,
            RuleAction::Float
        );
        assert_eq!(
            decide(&rules, &w("my-game"), 120.0).action,
            RuleAction::Float
        );
        assert_eq!(
            decide(&rules, &w("my-tool"), 120.0).action,
            RuleAction::Tile,
            "explicit rule wins"
        );
        assert_eq!(decide(&rules, &w("other"), 120.0).action, RuleAction::Tile);
    }

    #[test]
    fn chord_parse() {
        let c = Chord::parse("alt-shift-,").unwrap();
        assert!(c.alt && c.shift && c.key == "comma");
        assert_eq!(Chord::parse("ctrl--").unwrap().key, "minus");
    }
}
