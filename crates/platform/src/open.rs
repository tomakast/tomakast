//! Opening a URL or a file with the user's default application.
//!
//! These functions check nothing: callers decide what may be opened (the plugin host checks
//! schemes and paths first). Linux runs `xdg-open`, macOS `open`, both detached with no stdio
//! and reaped on a throwaway thread; Windows calls `ShellExecuteW` with the `open` verb.

use std::path::Path;

/// Opens `url` with the default handler for its scheme.
pub fn open_url(url: &str) -> anyhow::Result<()> {
    imp::open(url.as_ref())
}

/// Opens `path` with the default application for its type (a folder in the file manager).
pub fn open_path(path: &Path) -> anyhow::Result<()> {
    imp::open(path.as_os_str())
}

#[cfg(unix)]
mod imp {
    use std::ffi::OsStr;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    use anyhow::Context;

    #[cfg(target_os = "macos")]
    const PROGRAM: &str = "open";
    #[cfg(not(target_os = "macos"))]
    const PROGRAM: &str = "xdg-open";

    pub(super) fn open(target: &OsStr) -> anyhow::Result<()> {
        let child = Command::new(PROGRAM)
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .with_context(|| format!("failed to run {PROGRAM}"))?;
        reap(child);
        Ok(())
    }

    /// Waits for the child on a throwaway thread so it never stays a zombie.
    fn reap(mut child: std::process::Child) {
        let spawned = std::thread::Builder::new()
            .name("open-reap".into())
            .spawn(move || {
                let _ = child.wait();
            });
        if let Err(err) = spawned {
            tracing::warn!(%err, "could not start the reaper thread for {PROGRAM}");
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::ffi::{OsStr, c_void};
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn ShellExecuteW(
            hwnd: *mut c_void,
            operation: *const u16,
            file: *const u16,
            parameters: *const u16,
            directory: *const u16,
            show_cmd: i32,
        ) -> isize;
    }

    const SW_SHOWNORMAL: i32 = 1;

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    pub(super) fn open(target: &OsStr) -> anyhow::Result<()> {
        if target.encode_wide().any(|c| c == 0) {
            anyhow::bail!("the target contains a NUL character");
        }
        let verb = wide(OsStr::new("open"));
        let file = wide(target);
        // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; the other
        // pointers may be null.
        let code = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                file.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        // Values above 32 mean success.
        if code <= 32 {
            anyhow::bail!("ShellExecuteW failed with code {code}");
        }
        Ok(())
    }
}
