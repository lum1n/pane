//! Displays and references to them.

use serde::{Deserialize, Serialize};

use crate::geometry::Rect;
use crate::DisplayId;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Display {
    pub id: DisplayId,
    pub name: String,
    /// Global top-left points.
    pub frame: Rect,
    /// Frame minus menu bar and Dock.
    pub visible_frame: Rect,
    pub is_main: bool,
    #[serde(default = "one")]
    pub scale: f64,
    /// 0 = leftmost. Filled in by [`rank_displays`].
    #[serde(default)]
    pub position_rank: usize,
}

fn one() -> f64 {
    1.0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Position {
    Left,
    Center,
    Right,
}

/// A way to name a display that survives reconnects.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayRef {
    Main,
    Uuid(String),
    /// Case-insensitive substring of the display name.
    Name(String),
    Position(Position),
    Any,
}

impl DisplayRef {
    /// Parse the short form used in config and profiles:
    /// `main`, `any`, `left|center|right`, `uuid:<UUID>`, `name:<substring>`,
    /// a bare UUID, or anything else as a name substring.
    pub fn parse(s: &str) -> DisplayRef {
        let t = s.trim();
        match t.to_ascii_lowercase().as_str() {
            "main" | "primary" => DisplayRef::Main,
            "any" => DisplayRef::Any,
            "left" => DisplayRef::Position(Position::Left),
            "center" | "centre" | "middle" => DisplayRef::Position(Position::Center),
            "right" => DisplayRef::Position(Position::Right),
            _ => {
                if let Some(u) = t.strip_prefix("uuid:") {
                    DisplayRef::Uuid(u.to_string())
                } else if let Some(n) = t.strip_prefix("name:") {
                    DisplayRef::Name(n.to_string())
                } else if looks_like_uuid(t) {
                    DisplayRef::Uuid(t.to_string())
                } else {
                    DisplayRef::Name(t.to_string())
                }
            }
        }
    }

    pub fn to_short(&self) -> String {
        match self {
            DisplayRef::Main => "main".into(),
            DisplayRef::Any => "any".into(),
            DisplayRef::Position(Position::Left) => "left".into(),
            DisplayRef::Position(Position::Center) => "center".into(),
            DisplayRef::Position(Position::Right) => "right".into(),
            DisplayRef::Uuid(u) => format!("uuid:{u}"),
            DisplayRef::Name(n) => format!("name:{n}"),
        }
    }

    /// Resolve against the connected displays (which must be ranked).
    ///
    /// Positions: `left` is the leftmost and `right` the rightmost display.
    /// `center` is the middle one with three or more displays, and the main
    /// display with fewer, so a {left, center} desk still has a "center".
    pub fn resolve<'a>(&self, displays: &'a [Display]) -> Option<&'a Display> {
        if displays.is_empty() {
            return None;
        }
        let by_rank = |r: usize| displays.iter().find(|d| d.position_rank == r);
        let main = || displays.iter().find(|d| d.is_main).or(displays.first());
        match self {
            DisplayRef::Main => main(),
            DisplayRef::Any => main(),
            DisplayRef::Uuid(u) => displays.iter().find(|d| d.id.eq_ignore_ascii_case(u)),
            DisplayRef::Name(n) => {
                let n = n.to_lowercase();
                displays.iter().find(|d| d.name.to_lowercase().contains(&n))
            }
            DisplayRef::Position(Position::Left) => by_rank(0),
            DisplayRef::Position(Position::Right) => by_rank(displays.len() - 1),
            DisplayRef::Position(Position::Center) => {
                if displays.len() >= 3 {
                    by_rank(displays.len() / 2)
                } else {
                    main()
                }
            }
        }
    }
}

fn looks_like_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
        && s.matches('-').count() == 4
}

/// Sort displays left to right (then top to bottom) and set `position_rank`.
pub fn rank_displays(displays: &mut [Display]) {
    displays.sort_by(|a, b| {
        let ca = a.frame.center();
        let cb = b.frame.center();
        ca.x.total_cmp(&cb.x)
            .then(ca.y.total_cmp(&cb.y))
            .then(a.id.cmp(&b.id))
    });
    for (i, d) in displays.iter_mut().enumerate() {
        d.position_rank = i;
    }
}

/// The display containing the most of `rect` (or nearest to its centre).
pub fn display_for_rect<'a>(displays: &'a [Display], rect: &Rect) -> Option<&'a Display> {
    let best = displays
        .iter()
        .map(|d| {
            (
                d,
                d.frame.intersection(rect).map(|r| r.area()).unwrap_or(0.0),
            )
        })
        .max_by(|a, b| a.1.total_cmp(&b.1));
    match best {
        Some((d, a)) if a > 0.0 => Some(d),
        _ => {
            let c = rect.center();
            displays.iter().min_by(|a, b| {
                let da = (a.frame.center().x - c.x).powi(2) + (a.frame.center().y - c.y).powi(2);
                let db = (b.frame.center().x - c.x).powi(2) + (b.frame.center().y - c.y).powi(2);
                da.total_cmp(&db)
            })
        }
    }
}

/// Where a window of `size` is parked on `display`: its top-left corner one
/// point inside the display's bottom-right corner.
pub fn park_rect(display: &Display, size: crate::Size) -> Rect {
    Rect::new(
        display.frame.max_x() - 1.0,
        display.frame.max_y() - 1.0,
        size.w,
        size.h,
    )
}

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    pub fn disp(id: &str, x: f64, main: bool) -> Display {
        Display {
            id: id.to_string(),
            name: format!("Display {id}"),
            frame: Rect::new(x, 0.0, 1920.0, 1080.0),
            visible_frame: Rect::new(x, 25.0, 1920.0, 1055.0),
            is_main: main,
            scale: 2.0,
            position_rank: 0,
        }
    }

    /// Left, centre (main) and right displays.
    pub fn three() -> Vec<Display> {
        let mut v = vec![
            disp("L", -1920.0, false),
            disp("C", 0.0, true),
            disp("R", 1920.0, false),
        ];
        rank_displays(&mut v);
        v
    }

    pub fn subset(ids: &[&str]) -> Vec<Display> {
        let mut v: Vec<Display> = three()
            .into_iter()
            .filter(|d| ids.contains(&d.id.as_str()))
            .collect();
        rank_displays(&mut v);
        v
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn positions_resolve() {
        let d = three();
        let id = |r: DisplayRef, ds: &[Display]| r.resolve(ds).map(|d| d.id.clone());
        assert_eq!(id(DisplayRef::parse("left"), &d).as_deref(), Some("L"));
        assert_eq!(id(DisplayRef::parse("center"), &d).as_deref(), Some("C"));
        assert_eq!(id(DisplayRef::parse("right"), &d).as_deref(), Some("R"));
        let lc = subset(&["L", "C"]);
        assert_eq!(id(DisplayRef::parse("center"), &lc).as_deref(), Some("C"));
        assert_eq!(id(DisplayRef::parse("right"), &lc).as_deref(), Some("C"));
        let lr = subset(&["L", "R"]);
        assert_eq!(id(DisplayRef::parse("right"), &lr).as_deref(), Some("R"));
    }

    #[test]
    fn parse_forms() {
        assert_eq!(
            DisplayRef::parse("uuid:abc"),
            DisplayRef::Uuid("abc".into())
        );
        assert_eq!(
            DisplayRef::parse("37D8832A-2D66-02CA-B9F7-8F30A301B230"),
            DisplayRef::Uuid("37D8832A-2D66-02CA-B9F7-8F30A301B230".into())
        );
        assert_eq!(
            DisplayRef::parse("DELL U2720Q"),
            DisplayRef::Name("DELL U2720Q".into())
        );
    }
}
