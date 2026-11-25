//! macOS backend: the `.app` bundles of the application folders ([`crate::bundles`]),
//! opened, revealed and handed files through `/usr/bin/open`, never a shell.

use std::path::Path;
use std::process::Command;

use tomakast_core::{Effect, Item};

use crate::bundles::{EXTRA_BUNDLES, application_dirs, bundle_item, find_bundles};
use crate::process::{reap, spawn_detached};
use crate::{AppEntry, ScanOptions};

/// macOS's own opener.
const OPEN: &str = "/usr/bin/open";

pub(crate) fn options_from_env() -> ScanOptions {
    ScanOptions {
        dirs: application_dirs(std::env::home_dir().as_deref()),
        locales: Vec::new(),
        current_desktops: Vec::new(),
        path_env: None,
        icon_theme: String::new(),
    }
}

/// Every bundle, titled as Finder shows it.
pub(crate) fn scan(opts: &ScanOptions) -> Vec<(Item, AppEntry)> {
    find_bundles(&opts.dirs, EXTRA_BUNDLES)
        .into_iter()
        .map(|bundle| {
            let title = tomakast_platform::file_icons::display_name(&bundle);
            bundle_item(&bundle, title)
        })
        .collect()
}

/// Runs `open` with `args`, detached.
fn open(args: &[&std::ffi::OsStr], what: &str) -> anyhow::Result<()> {
    let mut cmd = Command::new(OPEN);
    cmd.args(args);
    let child = spawn_detached(cmd)
        .map_err(|err| anyhow::Error::new(err).context(format!("failed to {what}")))?;
    reap(child);
    Ok(())
}

pub(crate) fn launch(entry: &AppEntry) -> anyhow::Result<()> {
    open(&[entry.desktop_file.as_os_str()], "open the app")
}

pub(crate) fn launch_with_file(entry: &AppEntry, file: &Path) -> anyhow::Result<()> {
    open(
        &[
            "-a".as_ref(),
            entry.desktop_file.as_os_str(),
            file.as_os_str(),
        ],
        "open the file with the app",
    )
}

pub(crate) fn reveal(entry: &AppEntry) -> anyhow::Result<Vec<Effect>> {
    open(
        &["-R".as_ref(), entry.desktop_file.as_os_str()],
        "show the app in Finder",
    )?;
    Ok(vec![Effect::CloseWindow])
}
