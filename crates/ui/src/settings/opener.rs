//! What the Config file row opens the file with: the daemon installs the system's, tests a
//! recorder. Both calls may block (the system answers when it answers), so the window makes
//! them on the background executor.

use std::path::Path;

/// Opens `config.toml`, or shows it in its folder.
pub trait ConfigOpener: Send + Sync {
    /// Opens the file with the application the system has for it.
    fn open(&self, path: &Path) -> anyhow::Result<()>;
    /// Shows the file in the file manager.
    fn reveal(&self, path: &Path) -> anyhow::Result<()>;
}

/// The system's way to open a file and to show it in its folder.
pub struct PlatformOpener;

impl ConfigOpener for PlatformOpener {
    fn open(&self, path: &Path) -> anyhow::Result<()> {
        tomakast_platform::open::open_path(path)
    }

    fn reveal(&self, path: &Path) -> anyhow::Result<()> {
        tomakast_platform::reveal_path(path)
    }
}
