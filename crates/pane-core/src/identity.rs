//! Recognising windows again after an app (or the machine) restarts.

use serde::{Deserialize, Serialize};

use crate::geometry::{Rect, Size};
use crate::{WindowId, WorkspaceId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowFingerprint {
    pub bundle_id: String,
    pub title: String,
    pub title_norm: String,
    #[serde(default)]
    pub document: Option<String>,
    /// Position among the app's windows when they were discovered.
    pub ordinal: u32,
    pub size: Size,
}

impl WindowFingerprint {
    pub fn new(
        bundle_id: &str,
        title: &str,
        document: Option<&str>,
        ordinal: u32,
        size: Size,
    ) -> Self {
        WindowFingerprint {
            bundle_id: bundle_id.to_string(),
            title: title.to_string(),
            title_norm: normalize_title(title),
            document: document.filter(|d| !d.is_empty()).map(str::to_string),
            ordinal,
            size,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacementRecord {
    pub fp: WindowFingerprint,
    pub workspace: WorkspaceId,
    /// Index path of the leaf from the workspace root.
    pub anchor: Vec<usize>,
    pub weight: f64,
    pub floating: bool,
    #[serde(default)]
    pub float_frame: Option<Rect>,
    /// Unix milliseconds.
    pub seen_at: u64,
    /// Window id it had; its leaf may still be in the tree as a ghost.
    #[serde(default)]
    pub last_wid: Option<WindowId>,
    /// Window currently owning the record (not persisted).
    #[serde(skip)]
    pub live: Option<WindowId>,
}

/// Lowercase, drop digits (unread counters, timestamps), bullets and
/// separators, collapse whitespace.
pub fn normalize_title(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut space = false;
    for c in t.chars() {
        if c.is_ascii_digit()
            || matches!(c, '•' | '●' | '(' | ')' | '[' | ']' | '*' | '—' | '–' | '|')
        {
            continue;
        }
        if c.is_whitespace() || c == '-' {
            if !out.is_empty() {
                space = true;
            }
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.extend(c.to_lowercase());
    }
    out
}

pub const ACCEPT: i32 = 55;
pub const ACCEPT_SOLE: i32 = 20;

pub fn score(w: &WindowFingerprint, r: &WindowFingerprint) -> i32 {
    if w.bundle_id != r.bundle_id {
        return i32::MIN;
    }
    let mut s = 0;
    if w.document.is_some() && w.document == r.document {
        s += 100;
    }
    if !w.title.is_empty() && w.title == r.title {
        s += 80;
    } else if !w.title_norm.is_empty() && w.title_norm == r.title_norm {
        s += 55;
    }
    if w.ordinal == r.ordinal {
        s += 20;
    }
    let close = |a: f64, b: f64| b > 0.0 && ((a - b).abs() / b) <= 0.10;
    if close(w.size.w, r.size.w) && close(w.size.h, r.size.h) {
        s += 10;
    }
    s
}

/// Globally greedy assignment of windows to records (both of one or more
/// apps). Returns (window index, record index) pairs.
///
/// Pairs are taken best score first (ties: most recently seen record), so
/// two windows with distinct titles never cross-match. A pair is accepted
/// at [`ACCEPT`], or at [`ACCEPT_SOLE`] when it is the only window and the
/// only record of that app.
pub fn match_windows(
    windows: &[WindowFingerprint],
    records: &[&PlacementRecord],
) -> Vec<(usize, usize)> {
    let mut pairs: Vec<(i32, u64, usize, usize)> = Vec::new();
    for (wi, w) in windows.iter().enumerate() {
        for (ri, r) in records.iter().enumerate() {
            let s = score(w, &r.fp);
            if s >= ACCEPT_SOLE {
                pairs.push((s, r.seen_at, wi, ri));
            }
        }
    }
    pairs.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });
    let count = |bundle: &str| {
        (
            windows.iter().filter(|w| w.bundle_id == bundle).count(),
            records.iter().filter(|r| r.fp.bundle_id == bundle).count(),
        )
    };
    let mut used_w = vec![false; windows.len()];
    let mut used_r = vec![false; records.len()];
    let mut out = Vec::new();
    for (s, _, wi, ri) in pairs {
        if used_w[wi] || used_r[ri] {
            continue;
        }
        let ok = s >= ACCEPT || count(&windows[wi].bundle_id) == (1, 1);
        if ok {
            used_w[wi] = true;
            used_r[ri] = true;
            out.push((wi, ri));
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp(title: &str, ordinal: u32, w: f64) -> WindowFingerprint {
        WindowFingerprint::new(
            "com.apple.Safari",
            title,
            None,
            ordinal,
            Size::new(w, 800.0),
        )
    }

    fn rec(title: &str, ordinal: u32, w: f64, seen: u64) -> PlacementRecord {
        PlacementRecord {
            fp: fp(title, ordinal, w),
            workspace: "1".into(),
            anchor: vec![ordinal as usize],
            weight: 1.0,
            floating: false,
            float_frame: None,
            seen_at: seen,
            last_wid: None,
            live: None,
        }
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_title("(3) Inbox — Mail"), "inbox mail");
        assert_eq!(normalize_title("• main.rs - pane"), "main.rs pane");
    }

    #[test]
    fn distinct_titles_do_not_cross() {
        // Windows reopen in the opposite order with the same size.
        let records = [rec("GitHub", 0, 1000.0, 1), rec("Docs", 1, 1000.0, 1)];
        let refs: Vec<&PlacementRecord> = records.iter().collect();
        let windows = vec![fp("Docs", 0, 1000.0), fp("GitHub", 1, 1000.0)];
        assert_eq!(match_windows(&windows, &refs), vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn sole_window_matches_on_weak_evidence() {
        let records = [rec("Old title", 0, 1000.0, 1)];
        let refs: Vec<&PlacementRecord> = records.iter().collect();
        assert_eq!(
            match_windows(&[fp("Something else", 0, 1400.0)], &refs),
            vec![(0, 0)]
        );
        // With two candidates the weak evidence is not enough.
        let records = [rec("A", 0, 1000.0, 1), rec("B", 1, 1000.0, 1)];
        let refs: Vec<&PlacementRecord> = records.iter().collect();
        assert!(match_windows(&[fp("C", 0, 1400.0)], &refs).is_empty());
    }

    #[test]
    fn changing_titles_use_normalized_form_and_recency() {
        let records = [
            rec("Inbox (3) — Mail", 0, 900.0, 5),
            rec("Inbox (9) — Mail", 0, 900.0, 9),
        ];
        let refs: Vec<&PlacementRecord> = records.iter().collect();
        assert_eq!(
            match_windows(&[fp("Inbox (4) — Mail", 0, 900.0)], &refs),
            vec![(0, 1)]
        );
    }

    #[test]
    fn documents_win() {
        let mut a = rec("x", 3, 500.0, 1);
        a.fp.document = Some("file:///a.txt".into());
        let records = [a];
        let refs: Vec<&PlacementRecord> = records.iter().collect();
        let w = WindowFingerprint::new(
            "com.apple.Safari",
            "y",
            Some("file:///a.txt"),
            0,
            Size::new(1.0, 1.0),
        );
        let other = fp("z", 1, 2.0);
        assert_eq!(match_windows(&[other, w], &refs), vec![(1, 0)]);
    }
}
