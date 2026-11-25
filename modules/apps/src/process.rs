//! Starting programs that outlive the launcher (Unix).

use std::process::{Command, Stdio};

/// Spawns `cmd` with no terminal and in a process group of its own, so it outlives the
/// launcher.
pub(crate) fn spawn_detached(mut cmd: Command) -> std::io::Result<std::process::Child> {
    use std::os::unix::process::CommandExt;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
}

/// Waits for the child on a throwaway thread so it never stays a zombie.
pub(crate) fn reap(mut child: std::process::Child) {
    let spawned = std::thread::Builder::new()
        .name("apps-reap".into())
        .spawn(move || {
            let _ = child.wait();
        });
    if let Err(err) = spawned {
        tracing::warn!(%err, "could not start reaper thread");
    }
}
