//! Locations of the config file and the data directory.

use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Context;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paths {
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub db_file: PathBuf,
    /// Installed plugins, one folder each: `<data_dir>/plugins`.
    pub plugins_dir: PathBuf,
    /// Precompiled plugin components, written only by the compile child:
    /// `<data_dir>/cache/plugins`.
    pub plugin_cache_dir: PathBuf,
}

impl Paths {
    /// Detects paths from the process environment (`TOMAKAST_HOME` or XDG/platform dirs).
    pub fn detect() -> anyhow::Result<Paths> {
        Paths::detect_with(|k| std::env::var_os(k))
    }

    /// Like [`Paths::detect`] with an injectable environment lookup.
    pub fn detect_with(env: impl Fn(&str) -> Option<OsString>) -> anyhow::Result<Paths> {
        if let Some(home) = env("TOMAKAST_HOME").filter(|h| !h.is_empty()) {
            let home = PathBuf::from(home);
            return Ok(Paths::with(home.join("config.toml"), home));
        }
        let dirs = directories::ProjectDirs::from("", "", "tomakast")
            .context("cannot determine the user's home directory")?;
        Ok(Paths::with(
            dirs.config_dir().join("config.toml"),
            dirs.data_dir().to_path_buf(),
        ))
    }

    /// The paths for `config_file` and the data directory `data_dir`.
    pub fn with(config_file: PathBuf, data_dir: PathBuf) -> Paths {
        Paths {
            config_file,
            db_file: data_dir.join("tomakast.db"),
            plugins_dir: data_dir.join("plugins"),
            plugin_cache_dir: data_dir.join("cache").join("plugins"),
            data_dir,
        }
    }

    /// Creates the config and data directories.
    pub fn ensure_dirs(&self) -> anyhow::Result<()> {
        let dirs = [self.config_file.parent(), Some(self.data_dir.as_path())];
        for dir in dirs.into_iter().flatten() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(())
    }
}
