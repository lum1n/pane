//! Where Pane keeps its files.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    /// `$PANE_SUPPORT_DIR`, else `~/Library/Application Support/Pane`
    /// (`~/.local/share/pane` off macOS, for development).
    pub fn default_root() -> PathBuf {
        if let Ok(d) = std::env::var("PANE_SUPPORT_DIR") {
            return PathBuf::from(d);
        }
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        if cfg!(target_os = "macos") {
            PathBuf::from(home).join("Library/Application Support/Pane")
        } else {
            PathBuf::from(home).join(".local/share/pane")
        }
    }

    pub fn new(root: impl AsRef<Path>) -> Paths {
        Paths {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.profiles())?;
        std::fs::create_dir_all(self.logs())
    }

    pub fn config(&self) -> PathBuf {
        self.root.join("config.toml")
    }
    pub fn state(&self) -> PathBuf {
        self.root.join("state.json")
    }
    pub fn parked(&self) -> PathBuf {
        self.root.join("parked.json")
    }
    pub fn lock(&self) -> PathBuf {
        self.root.join("run.lock")
    }
    pub fn socket(&self) -> PathBuf {
        self.root.join("pane.sock")
    }
    pub fn profiles(&self) -> PathBuf {
        self.root.join("profiles")
    }
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn profile(&self, name: &str) -> PathBuf {
        self.profiles().join(format!(
            "{}.toml",
            pane_core::profile::Profile::file_stem(name)
        ))
    }
}
