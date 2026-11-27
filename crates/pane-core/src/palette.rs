//! The window and command palette: everything you can jump to or run,
//! ranked against what you typed.
//!
//! The shell sends `search` on every keystroke and shows the items in
//! order; picking one sends its `command` back unchanged.

use serde::{Deserialize, Serialize};

use crate::model::{Model, WindowMode};
use crate::proto::Command;
use crate::WindowId;

/// Most items a search returns.
pub const MAX_ITEMS: usize = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    Window,
    Workspace,
    Profile,
    Command,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaletteItem {
    pub kind: ItemKind,
    pub title: String,
    pub subtitle: String,
    /// App of a window item (the shell shows its icon).
    #[serde(default)]
    pub bundle_id: Option<String>,
    /// Chord bound to a command item, e.g. `alt-shift-f`.
    #[serde(default)]
    pub shortcut: Option<String>,
    /// What picking the item runs.
    pub command: Command,
    /// The focused window, the shown workspace, the active profile.
    #[serde(default)]
    pub current: bool,
}

/// Rank everything against `query`. An empty query lists windows, most
/// recently focused first (the focused one last, so Enter goes back to the
/// previous window), then workspaces. A query starting with `>` searches
/// only commands and profiles.
pub fn search(m: &Model, query: &str, profiles: &[String]) -> Vec<PaletteItem> {
    let (commands_only, query) = match query.trim_start().strip_prefix('>') {
        Some(rest) => (true, rest),
        None => (false, query),
    };
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut all: Vec<(PaletteItem, u64)> = Vec::new();
    if !commands_only {
        all.extend(window_items(m));
        all.extend(workspace_items(m).into_iter().map(|i| (i, 0)));
    }
    if commands_only || !terms.is_empty() {
        all.extend(profile_items(m, profiles).into_iter().map(|i| (i, 0)));
        all.extend(command_items(m, commands_only).into_iter().map(|i| (i, 0)));
    }
    if terms.is_empty() {
        // Windows by recency, the focused one at the end of them.
        all.sort_by(|(a, ra), (b, rb)| {
            a.kind
                .cmp(&b.kind)
                .then(a.current.cmp(&b.current))
                .then(rb.cmp(ra))
        });
        all.truncate(MAX_ITEMS);
        return all.into_iter().map(|(i, _)| i).collect();
    }
    let mut scored: Vec<(i64, PaletteItem, u64)> = all
        .into_iter()
        .filter_map(|(item, recency)| score_item(&terms, &item).map(|s| (s, item, recency)))
        .collect();
    scored.sort_by(|(sa, a, ra), (sb, b, rb)| {
        sb.cmp(sa)
            .then(a.kind.cmp(&b.kind))
            .then(rb.cmp(ra))
            .then(a.title.cmp(&b.title))
    });
    scored.truncate(MAX_ITEMS);
    scored.into_iter().map(|(_, i, _)| i).collect()
}

/// Every term must match the title or the subtitle; title hits count double.
fn score_item(terms: &[String], item: &PaletteItem) -> Option<i64> {
    let title = item.title.to_lowercase();
    let rest = format!(
        "{} {}",
        item.subtitle,
        item.shortcut.as_deref().unwrap_or("")
    )
    .to_lowercase();
    let mut total = 0;
    for t in terms {
        let a = fuzzy(t, &title).map(|s| s * 2);
        let b = fuzzy(t, &rest);
        total += a.max(b)?;
    }
    Some(total)
}

/// Subsequence match of `needle` in `hay` (both lowercase). Higher is
/// better: hits at word starts and runs of consecutive characters score
/// more, gaps cost a little. `None` if not all of `needle` is found.
pub fn fuzzy(needle: &str, hay: &str) -> Option<i64> {
    let n: Vec<char> = needle.chars().collect();
    let h: Vec<char> = hay.chars().collect();
    if n.is_empty() {
        return Some(0);
    }
    // Try each place the first character occurs; keep the best greedy run.
    (0..h.len())
        .filter(|&i| h[i] == n[0])
        .filter_map(|start| greedy(&n, &h, start))
        .max()
}

fn greedy(n: &[char], h: &[char], start: usize) -> Option<i64> {
    let mut score = 0i64;
    let mut hi = start;
    let mut prev: Option<usize> = None;
    for &c in n {
        while hi < h.len() && h[hi] != c {
            hi += 1;
        }
        if hi == h.len() {
            return None;
        }
        score += 1;
        let word_start = hi == 0 || !h[hi - 1].is_alphanumeric();
        if word_start {
            score += 10;
        }
        if hi == 0 {
            score += 5;
        }
        match prev {
            Some(p) if p + 1 == hi => score += 8,
            Some(p) => score -= ((hi - p - 1) as i64).min(5),
            None => {}
        }
        prev = Some(hi);
        hi += 1;
    }
    Some(score)
}

fn app_name(m: &Model, pid: crate::Pid, bundle_id: &str) -> String {
    m.apps
        .get(&pid)
        .map(|a| a.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            bundle_id
                .rsplit('.')
                .next()
                .unwrap_or(bundle_id)
                .to_string()
        })
}

fn display_name(m: &Model, id: &str) -> String {
    m.display(id)
        .map(|d| d.name.clone())
        .or_else(|| m.display_names.get(id).cloned())
        .unwrap_or_else(|| "Unplugged display".into())
}

fn window_items(m: &Model) -> Vec<(PaletteItem, u64)> {
    let focused = m.focused_window();
    m.windows
        .values()
        .map(|w| {
            let app = app_name(m, w.pid, &w.bundle_id);
            let title = if w.title.trim().is_empty() {
                app.clone()
            } else {
                w.title.clone()
            };
            let mut sub = vec![app];
            if let Some(ws) = m.ws(&w.workspace) {
                sub.push(format!(
                    "{} · workspace {}",
                    display_name(m, &ws.display),
                    ws.name
                ));
            }
            if w.minimized {
                sub.push("minimized".into());
            } else if w.mode == WindowMode::Floating {
                sub.push("floating".into());
            }
            let item = PaletteItem {
                kind: ItemKind::Window,
                title,
                subtitle: sub.join(" · "),
                bundle_id: Some(w.bundle_id.clone()),
                shortcut: None,
                command: Command::FocusWindow { wid: w.id.0 },
                current: focused == Some(w.id),
            };
            (item, recency(m, w.id))
        })
        .collect()
}

fn recency(m: &Model, wid: WindowId) -> u64 {
    m.focus_seen.get(&wid).copied().unwrap_or(0)
}

fn workspace_items(m: &Model) -> Vec<PaletteItem> {
    crate::snapshot::build(m)
        .workspaces
        .into_iter()
        .map(|w| {
            let windows = match w.windows {
                0 => "empty".to_string(),
                1 => "1 window".to_string(),
                n => format!("{n} windows"),
            };
            PaletteItem {
                kind: ItemKind::Workspace,
                title: format!("Workspace {}", w.label),
                subtitle: format!("{} · {windows}", display_name(m, &w.display)),
                bundle_id: None,
                shortcut: None,
                command: Command::Workspace { name: w.target },
                current: w.visible,
            }
        })
        .collect()
}

fn profile_items(m: &Model, profiles: &[String]) -> Vec<PaletteItem> {
    profiles
        .iter()
        .map(|p| {
            let current = m.active_profile.as_deref() == Some(p.as_str());
            PaletteItem {
                kind: ItemKind::Profile,
                title: p.clone(),
                subtitle: if current {
                    "Apply profile · active"
                } else {
                    "Apply profile"
                }
                .into(),
                bundle_id: None,
                shortcut: None,
                command: Command::ProfileApply {
                    name: Some(p.clone()),
                    profile: None,
                },
                current,
            }
        })
        .collect()
}

/// Commands that make sense without a shortcut, besides the bound ones.
const EXTRA_COMMANDS: &[&str] = &[
    "open-editor",
    "recover",
    "toggle-fullscreen",
    "reload",
    "balance",
    "layout master-stack",
    "layout columns 2",
    "layout columns 3",
    "layout manual",
];

/// `with_summon` adds a `summon APP` item per app with a window; plain
/// searches leave them out so the window itself stays the top hit.
fn command_items(m: &Model, with_summon: bool) -> Vec<PaletteItem> {
    let mut rows: Vec<(String, Option<String>)> = match m.config.binding_table() {
        Ok(t) => t
            .into_iter()
            .map(|b| (b.command, b.chords.into_iter().next()))
            .collect(),
        Err(_) => vec![],
    };
    let scratchpads = m
        .config
        .scratchpads
        .iter()
        .map(|s| format!("scratchpad {}", s.name()));
    for c in EXTRA_COMMANDS
        .iter()
        .map(|c| c.to_string())
        .chain(scratchpads)
    {
        if !rows.iter().any(|(t, _)| *t == c) {
            rows.push((c, None));
        }
    }
    let mut apps: Vec<String> = m
        .windows
        .values()
        .filter(|_| with_summon)
        .map(|w| app_name(m, w.pid, &w.bundle_id))
        .collect();
    apps.sort();
    apps.dedup();
    rows.extend(apps.into_iter().map(|a| (format!("summon {a}"), None)));
    rows.into_iter()
        .filter_map(|(text, chord)| {
            let command = Command::parse(&text).ok()?;
            if matches!(command, Command::Palette) {
                return None;
            }
            Some(PaletteItem {
                kind: ItemKind::Command,
                title: humanize(&text),
                subtitle: "Command".into(),
                bundle_id: None,
                shortcut: chord,
                command,
                current: false,
            })
        })
        .collect()
}

/// `move-to-workspace 3` -> `Move to workspace 3`.
fn humanize(text: &str) -> String {
    let named = match text.split_whitespace().next().unwrap_or("") {
        "open-editor" => Some("Open profile editor".to_string()),
        "recover" => Some("Recover hidden windows".to_string()),
        "reload" => Some("Reload config".to_string()),
        "profile-picker" => Some("Show profile picker".to_string()),
        "balance" => Some("Balance tile sizes".to_string()),
        "layout" => {
            let mut words = text.split_whitespace().skip(1);
            Some(match (words.next(), words.next()) {
                (Some("master-stack" | "master"), None) => "Layout: master and stack".to_string(),
                (Some("master-stack" | "master"), Some(pct)) => {
                    format!("Layout: master and stack ({pct}%)")
                }
                (Some("columns"), Some(n)) => format!("Layout: {n} columns"),
                (Some("manual" | "off"), _) => "Layout: manual (stop re-arranging)".to_string(),
                _ => "Layout".to_string(),
            })
        }
        "scratchpad" => Some(format!(
            "Toggle scratchpad {}",
            text.split_whitespace().nth(1).unwrap_or("")
        )),
        "summon" => Some(format!(
            "Summon {}",
            text.split_once(' ').map_or("", |(_, a)| a)
        )),
        _ => None,
    };
    if let Some(n) = named {
        return n;
    }
    let s = text.replace('-', " ");
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_prefers_word_starts_and_runs() {
        assert!(fuzzy("xyz", "safari").is_none());
        assert!(fuzzy("sf", "safari").is_some());
        let starts = fuzzy("tw", "two words").unwrap();
        let middle = fuzzy("tw", "between").unwrap();
        assert!(starts > middle, "{starts} vs {middle}");
        let run = fuzzy("term", "terminal").unwrap();
        let scattered = fuzzy("term", "the error message").unwrap();
        assert!(run > scattered, "{run} vs {scattered}");
        // The best starting point wins, not the first one.
        let a = fuzzy("chat", "c chat").unwrap();
        assert!(a >= fuzzy("chat", "chat").unwrap() - 5);
    }

    #[test]
    fn humanizes_commands() {
        assert_eq!(humanize("move-to-workspace 3"), "Move to workspace 3");
        assert_eq!(humanize("toggle-float"), "Toggle float");
        assert_eq!(humanize("open-editor"), "Open profile editor");
        assert_eq!(humanize("balance"), "Balance tile sizes");
        assert_eq!(humanize("layout master-stack"), "Layout: master and stack");
        assert_eq!(humanize("layout columns 3"), "Layout: 3 columns");
        assert_eq!(
            humanize("summon Visual Studio Code"),
            "Summon Visual Studio Code"
        );
    }
}
