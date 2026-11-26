//! Crash-safe file writes.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use pane_core::config::ScratchpadConfig;
use pane_core::profile::Profile;
use pane_core::state::{ParkedEntry, ParkedFile, State, STATE_VERSION};

/// Write to a temp file, fsync, rename over `path`, fsync the directory.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let tmp = path.with_extension("tmp");
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)?;
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

pub fn read_state(path: &Path) -> Option<State> {
    let text = fs::read_to_string(path).ok()?;
    match serde_json::from_str::<State>(&text) {
        Ok(s) if s.version == STATE_VERSION => Some(s),
        Ok(s) => {
            tracing::warn!(
                version = s.version,
                "ignoring state.json with another version"
            );
            None
        }
        Err(e) => {
            tracing::warn!("ignoring unreadable state.json: {e}");
            let _ = fs::rename(path, path.with_extension("json.bad"));
            None
        }
    }
}

pub fn write_state(path: &Path, s: &State) -> std::io::Result<()> {
    write_atomic(
        path,
        &serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?,
    )
}

pub fn read_parked(path: &Path) -> Vec<ParkedEntry> {
    fs::read_to_string(path)
        .ok()
        .and_then(|t| serde_json::from_str::<ParkedFile>(&t).ok())
        .map(|f| f.entries)
        .unwrap_or_default()
}

pub fn write_parked(path: &Path, entries: &[ParkedEntry]) -> std::io::Result<()> {
    if entries.is_empty() {
        return match fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let f = ParkedFile {
        version: 1,
        entries: entries.to_vec(),
    };
    write_atomic(
        path,
        &serde_json::to_vec_pretty(&f).map_err(std::io::Error::other)?,
    )
}

/// Every readable profile in `dir`, sorted by name.
pub fn read_profiles(dir: &Path) -> Vec<Profile> {
    let mut v: Vec<Profile> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter_map(|p| read_profile(&p).ok())
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

pub fn read_profile(path: &Path) -> Result<Profile, String> {
    let t = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Profile::from_toml(&t)
}

pub fn write_profile(path: &Path, p: &Profile) -> Result<(), String> {
    write_atomic(path, p.to_toml()?.as_bytes()).map_err(|e| e.to_string())
}

/// Edit one table of config.toml as text, keeping comments and layout:
/// drop the active `key = value` lines of `[table]` for which `remove(key)`
/// is true, then insert `lines` right after the table header (creating the
/// table at the end if it does not exist).
pub fn edit_table(
    text: &str,
    table: &str,
    remove: impl Fn(&str) -> bool,
    lines: &[String],
) -> String {
    let header = format!("[{table}]");
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            inside = t == header;
        }
        if inside && !t.starts_with('#') && !t.starts_with('[') {
            if let Some((k, _)) = t.split_once('=') {
                let k = k.trim().trim_matches('"').trim_matches('\'');
                if remove(k) {
                    continue;
                }
            }
        }
        out.push(l.to_string());
    }
    if !lines.is_empty() {
        match out.iter().position(|l| l.trim() == header) {
            Some(i) => {
                for (j, line) in lines.iter().enumerate() {
                    out.insert(i + 1 + j, line.clone());
                }
            }
            None => {
                out.push(String::new());
                out.push(header);
                out.extend(lines.iter().cloned());
            }
        }
    }
    out.join("\n") + "\n"
}

/// Set (or clear) `startup_profile` in config.toml.
pub fn set_startup_profile(config_text: &str, name: Option<&str>) -> String {
    let lines: Vec<String> = name
        .map(|n| format!("startup_profile = {}", toml_string(n)))
        .into_iter()
        .collect();
    edit_table(config_text, "settings", |k| k == "startup_profile", &lines)
}

/// Set a `[settings]` key to a TOML literal, or remove it.
pub fn set_setting(config_text: &str, key: &str, literal: Option<&str>) -> String {
    let lines: Vec<String> = literal
        .map(|v| format!("{key} = {v}"))
        .into_iter()
        .collect();
    edit_table(config_text, "settings", |k| k == key, &lines)
}

/// Replace every `[[scratchpad]]` table with `list`, appended at the end.
/// Comments and blank lines after a removed table stay (they belong to
/// what follows it).
pub fn set_scratchpads(config_text: &str, list: &[ScratchpadConfig]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    for l in config_text.lines() {
        let t = l.trim();
        if t.starts_with('[') {
            inside = t == "[[scratchpad]]";
            if inside {
                continue;
            }
        }
        if inside && !t.is_empty() && !t.starts_with('#') {
            continue;
        }
        out.push(l.to_string());
    }
    while out.last().is_some_and(|l| l.trim().is_empty()) {
        out.pop();
    }
    for sp in list {
        out.push(String::new());
        out.push("[[scratchpad]]".into());
        if let Some(n) = &sp.name {
            out.push(format!("name = {}", toml_string(n)));
        }
        out.push(format!("app = {}", toml_string(&sp.app)));
        if let Some(k) = &sp.key {
            out.push(format!("key = {}", toml_string(k)));
        }
        if let Some(z) = &sp.size {
            out.push(format!("size = {}", toml_string(z)));
        }
    }
    out.join("\n") + "\n"
}

pub fn toml_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pane_core::config::{Config, STARTER_CONFIG};

    #[test]
    fn scratchpads_are_replaced_and_config_stays_valid() {
        let sp = |app: &str, key: Option<&str>| ScratchpadConfig {
            name: None,
            app: app.into(),
            key: key.map(Into::into),
            size: Some("50%x50%".into()),
        };
        let t = set_scratchpads(STARTER_CONFIG, &[sp("a", Some("mod-grave")), sp("b", None)]);
        let c = Config::parse(&t).unwrap();
        assert_eq!(c.scratchpads.len(), 2);
        assert_eq!(c.scratchpads[0].key.as_deref(), Some("mod-grave"));
        let t2 = set_scratchpads(&t, &[sp("c", None)]);
        let c2 = Config::parse(&t2).unwrap();
        assert_eq!(c2.scratchpads.len(), 1);
        assert_eq!(c2.scratchpads[0].app, "c");
        let t3 = set_scratchpads(&t2, &[]);
        assert!(Config::parse(&t3).unwrap().scratchpads.is_empty());
        assert!(!t3.contains("[[scratchpad]]\napp"));
    }

    #[test]
    fn startup_profile_edit_keeps_config_valid() {
        let t = set_startup_profile(STARTER_CONFIG, Some("work"));
        let c = Config::parse(&t).unwrap();
        assert_eq!(c.settings.startup_profile.as_deref(), Some("work"));
        let t2 = set_startup_profile(&t, Some("home"));
        assert_eq!(
            Config::parse(&t2)
                .unwrap()
                .settings
                .startup_profile
                .as_deref(),
            Some("home")
        );
        let t3 = set_startup_profile(&t2, None);
        assert_eq!(Config::parse(&t3).unwrap().settings.startup_profile, None);
        let t4 = set_startup_profile("version = 1\n", Some("x"));
        assert_eq!(
            Config::parse(&t4)
                .unwrap()
                .settings
                .startup_profile
                .as_deref(),
            Some("x")
        );
        let t5 = set_startup_profile(
            "[settings]\ninner_gap = 2\n[[workspace]]\nname = \"1\"\n",
            Some("y"),
        );
        assert_eq!(
            Config::parse(&t5)
                .unwrap()
                .settings
                .startup_profile
                .as_deref(),
            Some("y")
        );
    }

    #[test]
    fn edit_table_keeps_comments() {
        let t = set_setting(STARTER_CONFIG, "inner_gap", Some("10"));
        assert!(t.contains("# Pane configuration"));
        let c = Config::parse(&t).unwrap();
        assert_eq!(c.settings.inner_gap, 10.0);
        assert_eq!(t.matches("inner_gap").count(), 1);
        let t = edit_table(
            &t,
            "shortcuts",
            |_| false,
            &["\"mod-x\" = \"reload\"".into()],
        );
        assert!(Config::parse(&t).unwrap().shortcuts.contains_key("mod-x"));
        let t = edit_table(
            "version = 1\n",
            "shortcuts",
            |_| false,
            &["\"alt-x\" = \"reload\"".into()],
        );
        assert!(Config::parse(&t).unwrap().shortcuts.contains_key("alt-x"));
    }

    #[test]
    fn atomic_write_and_parked_roundtrip() {
        let dir = std::env::temp_dir().join(format!("pane-test-{}", std::process::id()));
        let p = dir.join("parked.json");
        let e = ParkedEntry {
            pid: 1,
            wid: pane_core::WindowId(2),
            bundle_id: "a".into(),
            title: "t".into(),
            original: pane_core::Rect::new(1.0, 2.0, 3.0, 4.0),
        };
        write_parked(&p, std::slice::from_ref(&e)).unwrap();
        assert_eq!(read_parked(&p), vec![e]);
        write_parked(&p, &[]).unwrap();
        assert!(!p.exists());
        let _ = fs::remove_dir_all(dir);
    }
}
