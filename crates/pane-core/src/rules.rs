//! Window rules and automatic floating.

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::display::DisplayRef;
use crate::geometry::{Direction, Orientation};
use crate::tree::ContainerLayout;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Tile,
    Float,
    Ignore,
}

/// AX role classification of a window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowRole {
    #[default]
    Standard,
    Dialog,
    Sheet,
    Floating,
    Popover,
    Other,
}

/// Where a rule puts a window's tile when the window is first tiled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TilePosition {
    /// Next to the focused tile, like any new window.
    Next,
    /// A full-height column at the left edge of the workspace.
    Left,
    /// A full-height column at the right edge of the workspace.
    Right,
    /// A full-width row at the top of the workspace.
    #[serde(alias = "up")]
    Top,
    /// A full-width row at the bottom of the workspace.
    #[serde(alias = "down")]
    Bottom,
}

impl TilePosition {
    /// The workspace edge, or `None` for [`TilePosition::Next`].
    pub fn edge(self) -> Option<Direction> {
        match self {
            TilePosition::Next => None,
            TilePosition::Left => Some(Direction::Left),
            TilePosition::Right => Some(Direction::Right),
            TilePosition::Top => Some(Direction::Up),
            TilePosition::Bottom => Some(Direction::Down),
        }
    }
}

/// The layout part of a rule: applied once, when a matching window is
/// first tiled (not when it comes back to a remembered tile).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TilePlacement {
    pub position: Option<TilePosition>,
    /// How the new tile splits the focused one (only with `position` next).
    pub orientation: Option<Orientation>,
    /// Layout of the split the new tile lands in.
    pub layout: Option<ContainerLayout>,
    /// Share of that split, between 0 and 1.
    pub weight: Option<f64>,
}

impl TilePlacement {
    pub fn is_empty(&self) -> bool {
        *self == TilePlacement::default()
    }

    pub fn check(&self) -> Result<(), String> {
        if let Some(w) = self.weight {
            if !(w > 0.0 && w < 1.0) {
                return Err(format!(
                    "weight must be between 0 and 1 (a share of the split), got {w}"
                ));
            }
        }
        if let (Some(p), Some(_)) = (self.position, self.orientation) {
            if p.edge().is_some() {
                return Err(
                    "orientation only works with position = \"next\": an edge position sets it"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

/// `*` and `?` glob, case-insensitive.
#[derive(Clone, Debug)]
pub struct Glob(Regex);

impl Glob {
    pub fn new(pat: &str) -> Result<Glob, regex::Error> {
        let mut re = String::from("(?i)^");
        for c in pat.chars() {
            match c {
                '*' => re.push_str(".*"),
                '?' => re.push('.'),
                c => re.push_str(&regex::escape(&c.to_string())),
            }
        }
        re.push('$');
        Regex::new(&re).map(Glob)
    }

    pub fn matches(&self, s: &str) -> bool {
        self.0.is_match(s)
    }
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub app: Option<Glob>,
    pub title: Option<Regex>,
    pub action: Option<RuleAction>,
    pub workspace: Option<String>,
    pub display: Option<DisplayRef>,
    pub placement: TilePlacement,
}

impl Rule {
    pub fn new(
        app: Option<&str>,
        title: Option<&str>,
        action: Option<RuleAction>,
        workspace: Option<String>,
        display: Option<DisplayRef>,
    ) -> Result<Rule, String> {
        Ok(Rule {
            app: app
                .map(Glob::new)
                .transpose()
                .map_err(|e| format!("bad app glob: {e}"))?,
            title: title
                .map(Regex::new)
                .transpose()
                .map_err(|e| format!("bad title regex: {e}"))?,
            action,
            workspace,
            display,
            placement: TilePlacement::default(),
        })
    }

    pub fn with_placement(mut self, placement: TilePlacement) -> Result<Rule, String> {
        placement.check()?;
        self.placement = placement;
        Ok(self)
    }

    pub fn matches(&self, bundle_id: &str, title: &str) -> bool {
        self.app.as_ref().is_none_or(|g| g.matches(bundle_id))
            && self.title.as_ref().is_none_or(|r| r.is_match(title))
    }
}

/// What the window looks like to the classifier.
#[derive(Clone, Debug, Default)]
pub struct WindowTraits<'a> {
    pub bundle_id: &'a str,
    pub title: &'a str,
    pub role: WindowRole,
    pub resizable: bool,
    pub modal: bool,
    pub has_close_button: bool,
    pub width: f64,
    pub height: f64,
    pub regular_app: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decision {
    pub action: RuleAction,
    /// True when the action came from a rule rather than auto-float/default.
    pub explicit: bool,
    pub workspace: Option<String>,
    pub display: Option<DisplayRef>,
    /// Where a tile goes when the window is first tiled; each field from
    /// the first matching rule that sets it.
    pub placement: TilePlacement,
    /// Human-readable why, shown by `pane query windows`.
    pub reason: String,
}

/// Precedence: explicit ignore > explicit float/tile > auto-float > tile.
/// The first matching rule that names a workspace (or display) wins it.
pub fn decide(rules: &[Rule], w: &WindowTraits, auto_float_below: f64) -> Decision {
    let matching: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.matches(w.bundle_id, w.title))
        .collect();
    let workspace = matching.iter().find_map(|r| r.workspace.clone());
    let display = matching.iter().find_map(|r| r.display.clone());
    let placement = TilePlacement {
        position: matching.iter().find_map(|r| r.placement.position),
        orientation: matching.iter().find_map(|r| r.placement.orientation),
        layout: matching.iter().find_map(|r| r.placement.layout),
        weight: matching.iter().find_map(|r| r.placement.weight),
    };
    let explicit = if matching
        .iter()
        .any(|r| r.action == Some(RuleAction::Ignore))
    {
        Some(RuleAction::Ignore)
    } else {
        matching.iter().find_map(|r| r.action)
    };
    if let Some(action) = explicit {
        let reason = format!(
            "rule: {}",
            match action {
                RuleAction::Tile => "tile",
                RuleAction::Float => "float",
                RuleAction::Ignore => "ignore",
            }
        );
        return Decision {
            action,
            explicit: true,
            workspace,
            display,
            placement,
            reason,
        };
    }
    match float_reason(w, auto_float_below) {
        Some(why) => Decision {
            action: RuleAction::Float,
            explicit: false,
            workspace,
            display,
            placement,
            reason: format!("auto-float: {why}"),
        },
        None => Decision {
            action: RuleAction::Tile,
            explicit: false,
            workspace,
            display,
            placement,
            reason: "standard window".into(),
        },
    }
}

/// Why a window floats automatically, or `None` to tile it.
///
/// A missing close button only counts for non-standard windows: apps with
/// custom (frameless) title bars often have no AXCloseButton.
pub fn float_reason(w: &WindowTraits, below: f64) -> Option<&'static str> {
    if w.role != WindowRole::Standard {
        return Some(match w.role {
            WindowRole::Dialog => "dialog",
            WindowRole::Sheet => "sheet",
            WindowRole::Floating => "floating panel",
            WindowRole::Popover => "popover",
            _ => "not a standard window (unusual AX subrole)",
        });
    }
    if !w.resizable {
        return Some("not resizable");
    }
    if w.modal {
        return Some("modal");
    }
    if !w.regular_app {
        return Some("app is not a regular Dock app (menu bar / accessory)");
    }
    if (w.width > 0.0 && w.width < below) || (w.height > 0.0 && w.height < below) {
        return Some("smaller than auto_float_below");
    }
    None
}

pub fn should_auto_float(w: &WindowTraits, below: f64) -> bool {
    float_reason(w, below).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn std_window<'a>(bundle: &'a str, title: &'a str) -> WindowTraits<'a> {
        WindowTraits {
            bundle_id: bundle,
            title,
            role: WindowRole::Standard,
            resizable: true,
            modal: false,
            has_close_button: true,
            width: 800.0,
            height: 600.0,
            regular_app: true,
        }
    }

    #[test]
    fn glob() {
        let g = Glob::new("com.apple.*").unwrap();
        assert!(g.matches("com.apple.Finder"));
        assert!(!g.matches("org.mozilla.firefox"));
    }

    #[test]
    fn precedence() {
        let rules = vec![
            Rule::new(
                Some("com.apple.finder"),
                Some("^Copy"),
                Some(RuleAction::Float),
                Some("3".into()),
                None,
            )
            .unwrap(),
            Rule::new(Some("com.apple.finder"), None, None, Some("2".into()), None).unwrap(),
            Rule::new(Some("*.zoom.*"), None, Some(RuleAction::Ignore), None, None).unwrap(),
        ];
        let d = decide(
            &rules,
            &std_window("com.apple.finder", "Copy 3 items"),
            120.0,
        );
        assert_eq!(d.action, RuleAction::Float);
        assert_eq!(d.workspace.as_deref(), Some("3"));
        let d = decide(&rules, &std_window("com.apple.finder", "Downloads"), 120.0);
        assert_eq!(d.action, RuleAction::Tile);
        assert_eq!(d.workspace.as_deref(), Some("2"));
        assert_eq!(
            decide(&rules, &std_window("us.zoom.xos", "x"), 120.0).action,
            RuleAction::Ignore
        );
    }

    #[test]
    fn dialogs_auto_float_but_rules_override() {
        let mut w = std_window("com.example", "Save");
        w.role = WindowRole::Dialog;
        assert_eq!(decide(&[], &w, 120.0).action, RuleAction::Float);
        let tile = vec![Rule::new(
            Some("com.example"),
            None,
            Some(RuleAction::Tile),
            None,
            None,
        )
        .unwrap()];
        assert_eq!(decide(&tile, &w, 120.0).action, RuleAction::Tile);
        let mut w = std_window("com.example", "x");
        w.resizable = false;
        assert_eq!(decide(&[], &w, 120.0).action, RuleAction::Float);
        assert_eq!(decide(&[], &w, 120.0).reason, "auto-float: not resizable");
        // Frameless standard windows without a close button still tile.
        let mut w = std_window("com.example", "x");
        w.has_close_button = false;
        assert_eq!(decide(&[], &w, 120.0).action, RuleAction::Tile);
    }

    #[test]
    fn placement_comes_from_the_first_rule_that_sets_each_field() {
        let rules = vec![
            Rule::new(Some("com.apple.Terminal"), Some("^ssh"), None, None, None)
                .unwrap()
                .with_placement(TilePlacement {
                    weight: Some(0.5),
                    ..Default::default()
                })
                .unwrap(),
            Rule::new(Some("com.apple.Terminal"), None, None, None, None)
                .unwrap()
                .with_placement(TilePlacement {
                    position: Some(TilePosition::Right),
                    weight: Some(0.3),
                    ..Default::default()
                })
                .unwrap(),
        ];
        let d = decide(&rules, &std_window("com.apple.Terminal", "ssh box"), 120.0);
        assert_eq!(d.placement.position, Some(TilePosition::Right));
        assert_eq!(d.placement.weight, Some(0.5));
        let d = decide(&rules, &std_window("com.apple.Terminal", "zsh"), 120.0);
        assert_eq!(d.placement.weight, Some(0.3));
        assert!(decide(&rules, &std_window("other", "x"), 120.0)
            .placement
            .is_empty());
    }

    #[test]
    fn placement_check() {
        let bad_weight = TilePlacement {
            weight: Some(1.0),
            ..Default::default()
        };
        assert!(bad_weight.check().is_err());
        let edge_and_orientation = TilePlacement {
            position: Some(TilePosition::Left),
            orientation: Some(Orientation::Vertical),
            ..Default::default()
        };
        assert!(edge_and_orientation.check().is_err());
        let next_and_orientation = TilePlacement {
            position: Some(TilePosition::Next),
            orientation: Some(Orientation::Vertical),
            weight: Some(0.4),
            layout: Some(ContainerLayout::Accordion),
        };
        assert!(next_and_orientation.check().is_ok());
    }
}
