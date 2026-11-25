//! Showing a file selected in the user's file manager.
//!
//! Linux asks the `org.freedesktop.FileManager1` service on the session bus and falls back to
//! opening the containing folder; macOS runs `open -R`; Windows runs `explorer /select,`.

use std::path::Path;

/// Shows `path` selected in the file manager.
pub fn reveal_path(path: &Path) -> anyhow::Result<()> {
    imp::reveal(path)
}

/// The `file://` URI of an absolute `path`; bytes outside unreserved characters and `/` are
/// percent-encoded.
#[cfg(target_os = "linux")]
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;

    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(target_os = "linux")]
mod imp {
    use std::path::Path;

    use super::file_uri;

    pub(super) fn reveal(path: &Path) -> anyhow::Result<()> {
        match show_items(path) {
            Ok(()) => Ok(()),
            Err(err) => {
                tracing::debug!(%err, "FileManager1 is unavailable; opening the folder instead");
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new("."));
                crate::open::open_path(parent)
            }
        }
    }

    fn show_items(path: &Path) -> zbus::Result<()> {
        let conn = zbus::blocking::Connection::session()?;
        conn.call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(vec![file_uri(path)], ""),
        )?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::Path;
    use std::process::{Command, Stdio};

    use anyhow::Context;

    pub(super) fn reveal(path: &Path) -> anyhow::Result<()> {
        let mut child = Command::new("open")
            .arg("-R")
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to run open")?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt as _;
    use std::path::Path;
    use std::process::{Command, Stdio};

    use anyhow::Context;

    pub(super) fn reveal(path: &Path) -> anyhow::Result<()> {
        // explorer.exe parses its command line itself: the path is quoted inside the argument
        // (`/select,"C:\a b\c"`), not the argument as a whole, so it is passed raw.
        let mut child = Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("failed to run explorer")?;
        std::thread::spawn(move || {
            let _ = child.wait();
        });
        Ok(())
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::path::Path;

    use super::file_uri;

    #[test]
    fn file_uri_escapes_spaces() {
        assert_eq!(file_uri(Path::new("/a b/c.toml")), "file:///a%20b/c.toml");
    }

    #[test]
    fn file_uri_escapes_non_ascii_bytes() {
        assert_eq!(file_uri(Path::new("/é")), "file:///%C3%A9");
    }
}
