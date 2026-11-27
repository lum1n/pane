//! `config.toml`: strict, versioned user configuration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::display::{Display, DisplayRef, Position};
use crate::geometry::Orientation;
use crate::proto::Command;
use crate::rules::{Rule, RuleAction, TilePlacement, TilePosition};
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
    /// Apps summoned and dismissed with one key, as a centered float.
    #[serde(default, rename = "scratchpad")]
    pub scratchpads: Vec<ScratchpadConfig>,
    /// Workspace -> name shown in the menu bar icon, the menu and the
    /// palette. Keys are a workspace (`"1"`, on every display) or
    /// `DISPLAY:WORKSPACE` (`"right:3"`), which wins.
    #[serde(default)]
    pub workspace_names: BTreeMap<String, String>,
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
    /// No gaps on a workspace that shows a single tile.
    pub smart_gaps: bool,
    pub default_layout: ContainerLayout,
    pub placement_memory_days: u32,
    pub restore_placements: bool,
    /// No effect: workspaces always return to their own display.
    pub return_workspaces_on_reconnect: bool,
    pub startup_profile: Option<String>,
    /// Apply a profile by itself when the connected displays come to match
    /// its `[when]` block.
    pub auto_profiles: bool,
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
    /// Move windows by holding the modifier and pointing.
    pub gestures: bool,
    /// Modifier(s) for gestures; empty means the same as `modifier`.
    pub gesture_modifier: String,
    /// Width in points of the border drawn around the focused window
    /// (0 turns it off).
    pub border_width: f64,
    /// Border colour: "accent" (the system accent colour) or "#rgb",
    /// "#rrggbb", "#rrggbbaa".
    pub border_color: String,
    /// How much to darken windows that do not have focus (0 = off, 0.8 max).
    pub dim_unfocused: f64,
    /// Move the pointer to the middle of the window Pane focuses from the
    /// keyboard (hotkeys, palette, CLI), also onto another display.
    pub mouse_follows_focus: bool,
    /// Focus the window under the pointer once it has rested there briefly.
    pub focus_follows_mouse: bool,
    /// Draw each window's app icon on its tile in the menu bar icon.
    pub menu_bar_app_icons: bool,
    /// Share of the width `layout master-stack` gives the master tile
    /// (0.1 to 0.9).
    pub master_ratio: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            inner_gap: 6.0,
            outer_gap: 6.0,
            smart_gaps: false,
            default_layout: ContainerLayout::Tiles,
            placement_memory_days: 30,
            restore_placements: true,
            return_workspaces_on_reconnect: true,
            startup_profile: None,
            auto_profiles: true,
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
            gestures: true,
            gesture_modifier: String::new(),
            border_width: 2.0,
            border_color: "accent".into(),
            dim_unfocused: 0.0,
            mouse_follows_focus: false,
            focus_follows_mouse: false,
            menu_bar_app_icons: true,
            master_ratio: 0.6,
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
    /// Overrides `[settings] inner_gap` on this display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inner_gap: Option<f64>,
    /// Overrides `[settings] outer_gap` on this display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outer_gap: Option<f64>,
}

impl DisplayAlias {
    /// The display this alias stands for.
    pub fn display_ref(&self) -> Option<DisplayRef> {
        if let Some(u) = &self.uuid {
            return Some(DisplayRef::Uuid(u.clone()));
        }
        if let Some(n) = &self.match_name {
            return Some(DisplayRef::Name(n.clone()));
        }
        self.position.map(DisplayRef::Position)
    }
}

/// Gaps in effect on one display.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gaps {
    pub inner: f64,
    pub outer: f64,
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
    /// Where the tile goes when a matching window is first tiled.
    #[serde(default)]
    pub position: Option<TilePosition>,
    /// How the new tile splits the focused one (with `position = "next"`).
    #[serde(default)]
    pub orientation: Option<Orientation>,
    /// Layout of the split the new tile lands in.
    #[serde(default)]
    pub layout: Option<ContainerLayout>,
    /// Share of that split, between 0 and 1.
    #[serde(default)]
    pub weight: Option<f64>,
}

impl RuleConfig {
    pub fn placement(&self) -> TilePlacement {
        TilePlacement {
            position: self.position,
            orientation: self.orientation,
            layout: self.layout,
            weight: self.weight,
        }
    }
}

/// `[[scratchpad]]`: one window of `app`, toggled by `scratchpad NAME`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchpadConfig {
    /// What `scratchpad NAME` calls it; defaults to `app`.
    #[serde(default)]
    pub name: Option<String>,
    /// Bundle id (or executable name for unbundled apps).
    pub app: String,
    /// Chord that toggles it, e.g. "mod-grave".
    #[serde(default)]
    pub key: Option<String>,
    /// "WxH", each side a percentage of the display ("60%") or points.
    #[serde(default)]
    pub size: Option<String>,
}

pub const DEFAULT_SCRATCHPAD_SIZE: &str = "60%x60%";

impl ScratchpadConfig {
    pub fn name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.app)
    }

    /// Width and height for a display whose visible frame is `w` x `h`.
    pub fn size_in(&self, w: f64, h: f64) -> (f64, f64) {
        let (sw, sh) = parse_size(self.size.as_deref().unwrap_or(DEFAULT_SCRATCHPAD_SIZE))
            .unwrap_or((Length::Percent(60.0), Length::Percent(60.0)));
        (sw.of(w).min(w), sh.of(h).min(h))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Length {
    Percent(f64),
    Points(f64),
}

impl Length {
    pub fn of(self, total: f64) -> f64 {
        match self {
            Length::Percent(p) => total * p / 100.0,
            Length::Points(p) => p,
        }
    }
}

/// `60%x50%`, `1200x800`, `50%x700`.
pub fn parse_size(s: &str) -> Result<(Length, Length), String> {
    let side = |t: &str| -> Result<Length, String> {
        let t = t.trim();
        let (num, pct) = match t.strip_suffix('%') {
            Some(n) => (n, true),
            None => (t, false),
        };
        let v: f64 = num
            .trim()
            .parse()
            .map_err(|_| format!("size `{s}`: bad number `{t}`"))?;
        if pct {
            if !(10.0..=100.0).contains(&v) {
                return Err(format!("size `{s}`: percentages go from 10% to 100%"));
            }
            Ok(Length::Percent(v))
        } else {
            if !(100.0..=10000.0).contains(&v) {
                return Err(format!("size `{s}`: points go from 100 to 10000"));
            }
            Ok(Length::Points(v))
        }
    };
    let (w, h) = s
        .split_once(['x', 'X', '*'])
        .ok_or_else(|| format!("size `{s}`: expected WIDTHxHEIGHT, e.g. 60%x50%"))?;
    Ok((side(w)?, side(h)?))
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
    add(b, "slash", "palette".into());
    add(b, "z", "undo".into());
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
            scratchpads: vec![],
            workspace_names: BTreeMap::new(),
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
        let gap_ok = |g: f64| (0.0..=200.0).contains(&g);
        if !gap_ok(s.inner_gap) || !gap_ok(s.outer_gap) {
            return Err(ConfigError::plain("gaps must be between 0 and 200"));
        }
        if !(0.005..=0.5).contains(&s.resize_step) {
            return Err(ConfigError::plain(
                "resize_step must be between 0.005 and 0.5",
            ));
        }
        if !(0.0..=20.0).contains(&s.border_width) {
            return Err(ConfigError::plain("border_width must be between 0 and 20"));
        }
        if !(0.0..=0.8).contains(&s.dim_unfocused) {
            return Err(ConfigError::plain(
                "dim_unfocused must be between 0 and 0.8",
            ));
        }
        if !crate::workspace::MASTER_RATIO_RANGE.contains(&s.master_ratio) {
            return Err(ConfigError::plain(
                "master_ratio must be between 0.1 and 0.9",
            ));
        }
        self.border_rgba()?;
        for d in &self.displays {
            let n =
                d.uuid.is_some() as u8 + d.match_name.is_some() as u8 + d.position.is_some() as u8;
            if n != 1 {
                return Err(ConfigError::plain(format!(
                    "[[display]] `{}` needs exactly one of uuid, match_name, position",
                    d.name
                )));
            }
            if !d.inner_gap.into_iter().chain(d.outer_gap).all(gap_ok) {
                return Err(ConfigError::plain(format!(
                    "[[display]] `{}`: gaps must be between 0 and 200",
                    d.name
                )));
            }
        }
        let mut names: Vec<&str> = Vec::new();
        for (i, sp) in self.scratchpads.iter().enumerate() {
            let what = format!("[[scratchpad]] #{}", i + 1);
            let name = sp.name();
            if sp.app.trim().is_empty() {
                return Err(ConfigError::plain(format!("{what}: app is empty")));
            }
            if name.is_empty() || name.chars().any(char::is_whitespace) {
                return Err(ConfigError::plain(format!(
                    "{what}: name `{name}` must be one word"
                )));
            }
            if names.contains(&name) {
                return Err(ConfigError::plain(format!(
                    "{what}: there is already a scratchpad named `{name}`"
                )));
            }
            names.push(name);
            if let Some(size) = &sp.size {
                parse_size(size).map_err(|e| ConfigError::plain(format!("{what}: {e}")))?;
            }
        }
        for (key, title) in &self.workspace_names {
            let ws = key.rsplit_once(':').map_or(key.as_str(), |(_, w)| w);
            if !crate::workspace::valid_name(ws) || key.starts_with(':') {
                return Err(ConfigError::plain(format!(
                    "[workspace_names] `{key}`: expected a workspace like \"3\" or \"right:3\""
                )));
            }
            let t = title.trim();
            if t.is_empty() || t.chars().count() > 32 {
                return Err(ConfigError::plain(format!(
                    "[workspace_names] `{key}`: names are 1 to 32 characters"
                )));
            }
        }
        self.compiled_rules()?;
        self.bindings()?;
        self.gesture_mods()?;
        Ok(())
    }

    /// Resolve a display name used in `[[rule]]`, profiles or targets.
    pub fn display_ref(&self, s: &str) -> DisplayRef {
        self.displays
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(s.trim()))
            .and_then(DisplayAlias::display_ref)
            .unwrap_or_else(|| DisplayRef::parse(s))
    }

    /// Gaps on `display`: `[settings]`, overridden per side by the first
    /// `[[display]]` alias that resolves to it (among `displays`, the
    /// connected ones) and sets that gap.
    pub fn gaps_for(&self, display: &Display, displays: &[Display]) -> Gaps {
        let mut gaps = Gaps {
            inner: self.settings.inner_gap,
            outer: self.settings.outer_gap,
        };
        let mut inner_set = false;
        let mut outer_set = false;
        for a in &self.displays {
            if a.inner_gap.is_none() && a.outer_gap.is_none() {
                continue;
            }
            let here = a
                .display_ref()
                .and_then(|r| r.resolve(displays))
                .is_some_and(|d| d.id == display.id);
            if !here {
                continue;
            }
            if let (Some(g), false) = (a.inner_gap, inner_set) {
                gaps.inner = g;
                inner_set = true;
            }
            if let (Some(g), false) = (a.outer_gap, outer_set) {
                gaps.outer = g;
                outer_set = true;
            }
        }
        gaps
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
                .and_then(|rule| rule.with_placement(r.placement()))
                .map_err(|e| ConfigError::plain(format!("rule #{}: {e}", i + 1)))
            })
            .chain(float_apps)
            .collect()
    }

    /// The built-in defaults plus each scratchpad's `key`: everything
    /// `[shortcuts]` overrides.
    pub fn default_bindings(&self, base: &Mods) -> Vec<(String, String)> {
        let mut v = default_shortcuts(base);
        for sp in &self.scratchpads {
            if let Some(k) = sp.key.as_ref().filter(|k| !k.trim().is_empty()) {
                v.push((k.trim().to_string(), format!("scratchpad {}", sp.name())));
            }
        }
        v
    }

    pub fn scratchpad(&self, name: &str) -> Option<&ScratchpadConfig> {
        self.scratchpads.iter().find(|s| s.name() == name)
    }

    /// Effective hotkey bindings: defaults overlaid with `[shortcuts]`.
    pub fn bindings(&self) -> Result<Vec<(Chord, Command)>, ConfigError> {
        let base = self.modifier()?;
        let mut map: BTreeMap<Chord, Option<Command>> = BTreeMap::new();
        for (k, v) in self.default_bindings(&base) {
            let chord = Chord::parse_with(&k, &base)
                .map_err(|e| ConfigError::plain(format!("`{k}`: {e}")))?;
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
        for (chord, text) in self.default_bindings(&base) {
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

    /// `border_color` as RGBA in 0..=1, or `None` for the system accent colour.
    pub fn border_rgba(&self) -> Result<Option<[f64; 4]>, ConfigError> {
        parse_color(&self.settings.border_color)
            .map_err(|e| ConfigError::plain(format!("[settings] border_color: {e}")))
    }

    /// Modifiers that start a gesture, or `None` when gestures are off.
    pub fn gesture_mods(&self) -> Result<Option<Mods>, ConfigError> {
        if !self.settings.gestures {
            return Ok(None);
        }
        if self.settings.gesture_modifier.trim().is_empty() {
            return self.modifier().map(Some);
        }
        Mods::parse(&self.settings.gesture_modifier)
            .map(Some)
            .map_err(|e| ConfigError::plain(format!("[settings] gesture_modifier: {e}")))
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

/// "accent" -> `None`; "#rgb", "#rrggbb" or "#rrggbbaa" -> RGBA in 0..=1.
pub fn parse_color(s: &str) -> Result<Option<[f64; 4]>, String> {
    let t = s.trim();
    if t.is_empty() || t.eq_ignore_ascii_case("accent") {
        return Ok(None);
    }
    let bad = || format!("`{s}` is not \"accent\" or a #rrggbb colour");
    let hex = t.strip_prefix('#').ok_or_else(bad)?;
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(bad());
    }
    let digits: Vec<u8> = match hex.len() {
        3 => hex.chars().flat_map(|c| [c, c]).map(|c| c as u8).collect(),
        6 | 8 => hex.bytes().collect(),
        _ => return Err(bad()),
    };
    let byte = |i: usize| {
        let pair = std::str::from_utf8(&digits[i..i + 2]).unwrap_or("00");
        u8::from_str_radix(pair, 16).unwrap_or(0) as f64 / 255.0
    };
    let alpha = if digits.len() == 8 { byte(6) } else { 1.0 };
    Ok(Some([byte(0), byte(2), byte(4), alpha]))
}

/// Commented starter config written on first launch.
pub const STARTER_CONFIG: &str = r#"# Pane configuration. Reload with alt-shift-r or `pane reload`.
version = 1

[settings]
inner_gap = 6
outer_gap = 6
# No gaps on a workspace that shows a single tile.
smart_gaps = false
default_layout = "tiles"
placement_memory_days = 30
restore_placements = true
return_workspaces_on_reconnect = true
# startup_profile = "work"
# Apply a profile by itself when the displays match its [when] block
# (`pane profile-when here NAME` writes one for the current displays).
auto_profiles = true

# The key(s) behind every default hotkey: "alt", "ctrl-alt", "cmd-ctrl",
# "hyper" (cmd-ctrl-alt-shift), "meh" (ctrl-alt-shift), ...
# Moving adds shift and resizing adds ctrl (or the next free modifier).
modifier = "alt"

# Hold the modifier over a window and point somewhere else to move it there;
# release to drop, Escape cancels. Unset gesture_modifier means the same keys
# as `modifier`.
gestures = true
# gesture_modifier = "ctrl-alt"

# A border around the focused window (0 = off) and dimming of the others
# (0 = off, up to 0.8). border_color is "accent" or a hex colour like
# ff8000 or ff800080, written with a leading hash.
border_width = 2
border_color = "accent"
dim_unfocused = 0

# Move the pointer to the window you focus with the keyboard, and focus the
# window the pointer rests over.
mouse_follows_focus = false
focus_follows_mouse = false
# Draw each window's app icon on its tile in the menu bar icon.
menu_bar_app_icons = true
# Width share of the large tile in `layout master-stack` (0.1 to 0.9).
master_ratio = 0.6

# Apps whose windows always start floating (bundle ids, or the executable
# name for apps started from a terminal; * globs work).
# float_apps = ["com.apple.calculator", "com.apple.systempreferences"]
# Apps whose windows always tile, even if Pane would float them.
# tile_apps = ["my-app"]

# Name displays so workspaces and rules can refer to them.
# Match by position (left/center/right), uuid = "..." or match_name = "DELL".
# A [[display]] can override the gaps above for that display (see docs/config.md).
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

# Optional names, shown in the menu bar icon, the menu and the palette.
# "DISPLAY:N" names workspace N of one display only and wins over "N".
# [workspace_names]
# "1" = "web"
# "right:3" = "chat"

# A scratchpad: one key shows the app's window as a centered float on the
# display you are working on (opening the app if needed); again hides it.
# [[scratchpad]]
# app = "com.googlecode.iterm2"
# key = "mod-grave"
# size = "60%x50%"           # percent of the display, or points: 1200x800
# name = "term"              # for `scratchpad term`; defaults to app

# [[rule]]
# app = "com.apple.finder"   # glob on bundle id
# title = "^Copy"            # regex on window title
# action = "float"           # float | tile | ignore
# workspace = "3"           # workspace 3 of the display the window opens on
# display = "right"          # ...or of this display

# Where a matching window's tile goes when it is first tiled:
# [[rule]]
# app = "com.mitchellh.ghostty"
# position = "right"         # next | left | right | top | bottom
# weight = 0.3               # share of the split, 0-1
# orientation = "vertical"   # with position = "next": stack under the focused tile
# layout = "accordion"       # tiles | accordion: layout of the split it lands in

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
    fn workspace_names_validated() {
        let c =
            Config::parse("[workspace_names]\n\"1\" = \"web\"\n\"right:3\" = \"chat\"\n").unwrap();
        assert_eq!(c.workspace_names.len(), 2);
        assert!(Config::parse("[workspace_names]\n\"1\" = \" \"\n").is_err());
        assert!(Config::parse("[workspace_names]\n\"left:\" = \"x\"\n").is_err());
        assert!(Config::parse("[workspace_names]\n\":2\" = \"x\"\n").is_err());
        assert!(Config::parse(&format!(
            "[workspace_names]\n\"2\" = \"{}\"\n",
            "x".repeat(33)
        ))
        .is_err());
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
        assert!(Config::parse("[[rule]]\napp = \"x\"\nweight = 2.0\n").is_err());
        assert!(Config::parse("[[rule]]\napp = \"x\"\nposition = \"middle\"\n").is_err());
        assert!(
            Config::parse("[[rule]]\napp = \"x\"\nposition = \"top\"\norientation = \"h\"\n")
                .is_err()
        );
        let c = Config::parse(
            "[[rule]]\napp = \"x\"\nposition = \"down\"\nweight = 0.3\nlayout = \"accordion\"\n",
        )
        .unwrap();
        let p = c.compiled_rules().unwrap()[0].placement;
        assert_eq!(p.position, Some(TilePosition::Bottom));
        assert_eq!(p.layout, Some(ContainerLayout::Accordion));
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
    fn scratchpads_parse_bind_and_validate() {
        let c = Config::parse(
            "[[scratchpad]]\napp = \"com.googlecode.iterm2\"\nkey = \"mod-grave\"\nsize = \"50%x700\"\nname = \"term\"\n\n[[scratchpad]]\napp = \"com.apple.Notes\"\n",
        )
        .unwrap();
        assert_eq!(c.scratchpads[1].name(), "com.apple.Notes");
        let term = c.scratchpad("term").unwrap();
        assert_eq!(term.size_in(2000.0, 1000.0), (1000.0, 700.0));
        assert_eq!(
            c.scratchpad("com.apple.Notes")
                .unwrap()
                .size_in(1000.0, 500.0),
            (600.0, 300.0)
        );
        let bound = |c: &Config| {
            c.bindings()
                .unwrap()
                .into_iter()
                .find(|(_, cmd)| {
                    *cmd == Command::Scratchpad {
                        name: "term".into(),
                    }
                })
                .map(|(ch, _)| ch.display())
        };
        assert_eq!(bound(&c).as_deref(), Some("alt-grave"));
        let row = c
            .binding_table()
            .unwrap()
            .into_iter()
            .find(|r| r.command == "scratchpad term")
            .unwrap();
        assert_eq!(row.default.as_deref(), Some("alt-grave"));
        // [shortcuts] still wins.
        let c = Config::parse(
            "[[scratchpad]]\napp = \"x\"\nkey = \"mod-grave\"\n[shortcuts]\n\"mod-grave\" = \"none\"\n",
        )
        .unwrap();
        assert!(c
            .bindings()
            .unwrap()
            .iter()
            .all(|(ch, _)| ch.key != "grave"));
        for bad in [
            "[[scratchpad]]\napp = \"x\"\nsize = \"big\"\n",
            "[[scratchpad]]\napp = \"x\"\nsize = \"5%x50%\"\n",
            "[[scratchpad]]\napp = \"x\"\nsize = \"20x500\"\n",
            "[[scratchpad]]\napp = \"x\"\nname = \"two words\"\n",
            "[[scratchpad]]\napp = \"x\"\n[[scratchpad]]\napp = \"x\"\n",
            "[[scratchpad]]\napp = \"x\"\nkey = \"fn-x\"\n",
            "[[scratchpad]]\napp = \"\"\n",
        ] {
            assert!(Config::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn pointer_focus_settings_default_off() {
        let c = Config::default();
        assert!(!c.settings.mouse_follows_focus && !c.settings.focus_follows_mouse);
        let c =
            Config::parse("[settings]\nmouse_follows_focus = true\nfocus_follows_mouse = true\n")
                .unwrap();
        assert!(c.settings.mouse_follows_focus && c.settings.focus_follows_mouse);
        assert!(
            !Config::parse(STARTER_CONFIG)
                .unwrap()
                .settings
                .focus_follows_mouse
        );
    }

    #[test]
    fn border_and_dim_settings() {
        let c = Config::default();
        assert_eq!(c.settings.border_width, 2.0);
        assert_eq!(c.border_rgba().unwrap(), None);
        let c = Config::parse(
            "[settings]\nborder_width = 4\nborder_color = \"#ff8000\"\ndim_unfocused = 0.3\n",
        )
        .unwrap();
        assert_eq!(c.settings.border_width, 4.0);
        assert_eq!(c.settings.dim_unfocused, 0.3);
        let [r, g, b, a] = c.border_rgba().unwrap().unwrap();
        assert_eq!((r, b, a), (1.0, 0.0, 1.0));
        assert!((g - 128.0 / 255.0).abs() < 1e-9);
        assert_eq!(parse_color("#fff").unwrap(), Some([1.0; 4]));
        assert_eq!(parse_color("#00000080").unwrap().unwrap()[3], 128.0 / 255.0);
        for bad in ["red", "#12", "#ggg", "fff"] {
            assert!(parse_color(bad).is_err(), "{bad}");
        }
        assert!(Config::parse("[settings]\nborder_color = \"blue\"\n").is_err());
        assert!(Config::parse("[settings]\nborder_width = 30\n").is_err());
        assert!(Config::parse("[settings]\ndim_unfocused = 0.9\n").is_err());
        assert_eq!(Config::default().settings.master_ratio, 0.6);
        assert!(Config::parse("[settings]\nmaster_ratio = 0.7\n").is_ok());
        assert!(Config::parse("[settings]\nmaster_ratio = 0.95\n").is_err());
    }

    #[test]
    fn per_display_gaps() {
        use crate::display::testutil::three;
        let ds = three();
        let c = Config::parse(
            "[settings]\ninner_gap = 8\nouter_gap = 10\nsmart_gaps = true\n\
             [[display]]\nname = \"laptop\"\nposition = \"left\"\ninner_gap = 0\nouter_gap = 0\n\
             [[display]]\nname = \"big\"\nuuid = \"r\"\nouter_gap = 20\n\
             [[display]]\nname = \"big2\"\nposition = \"right\"\nouter_gap = 99\ninner_gap = 3\n",
        )
        .unwrap();
        assert!(c.settings.smart_gaps);
        let gaps = |id: &str| c.gaps_for(ds.iter().find(|d| d.id == id).unwrap(), &ds);
        assert_eq!(
            gaps("L"),
            Gaps {
                inner: 0.0,
                outer: 0.0
            }
        );
        // No override: [settings].
        assert_eq!(
            gaps("C"),
            Gaps {
                inner: 8.0,
                outer: 10.0
            }
        );
        // The first alias that sets a gap wins, per gap.
        assert_eq!(
            gaps("R"),
            Gaps {
                inner: 3.0,
                outer: 20.0
            }
        );
        assert!(!Config::default().settings.smart_gaps);
        let e = Config::parse("[[display]]\nname = \"x\"\nposition = \"left\"\nouter_gap = 300\n")
            .unwrap_err();
        assert!(e.message.contains("gaps"), "{}", e.message);
        assert!(
            Config::parse("[[display]]\nname = \"x\"\nposition = \"left\"\ninner_gap = -1\n")
                .is_err()
        );
        assert!(Config::parse(STARTER_CONFIG).is_ok());
    }

    #[test]
    fn gesture_modifier_defaults_to_modifier() {
        let c = Config::parse("[settings]\nmodifier = \"ctrl-alt\"\n").unwrap();
        assert_eq!(c.gesture_mods().unwrap(), Some(c.modifier().unwrap()));
        let c = Config::parse("[settings]\ngesture_modifier = \"cmd-alt\"\n").unwrap();
        assert_eq!(c.gesture_mods().unwrap().unwrap().display(), "cmd-alt");
        let c = Config::parse("[settings]\ngestures = false\n").unwrap();
        assert_eq!(c.gesture_mods().unwrap(), None);
        assert!(Config::parse("[settings]\ngesture_modifier = \"shift\"\n").is_err());
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
