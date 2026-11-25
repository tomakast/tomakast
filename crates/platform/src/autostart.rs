//! Launch at login.
//!
//! Linux: a systemd user service, `tomakast.service`, tied to the graphical session. It starts
//! the launcher at login and again after a crash, and its log is the journal's
//! (`journalctl --user -u tomakast`). The launcher owns the unit file: every apply writes it
//! with the current executable, and "Start at login" enables or disables it. Without a
//! systemd user manager, or in a session that never reaches `graphical-session.target` (some
//! Hyprland, Sway or niri setups), an XDG autostart `.desktop` file starts the launcher
//! instead, as earlier versions did; nothing restarts it then.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};

/// The name of the launcher's systemd user unit.
pub const UNIT: &str = "tomakast.service";

/// What starts the launcher at login.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    /// The systemd user service; `systemctl --user start tomakast.service` runs the launcher.
    Systemd,
    /// An XDG autostart entry, which the desktop runs at login.
    Xdg,
    /// Nothing: not implemented on this OS yet.
    Unsupported,
}

/// What systemd says about the user's session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Probe {
    /// No systemd, or no user manager to talk to.
    NoUserManager,
    /// The user manager runs and `graphical-session.target` is active.
    SessionTarget,
    /// The user manager runs; `graphical-session.target` is not active.
    NoSessionTarget,
}

/// One change [`apply_in`] makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Writes the unit with the current executable; a changed file is followed by
    /// `systemctl --user daemon-reload`.
    WriteUnit,
    /// `systemctl --user enable tomakast.service`.
    Enable,
    /// `systemctl --user disable tomakast.service`.
    Disable,
    /// Writes the XDG autostart entry.
    WriteXdg,
    /// Removes the XDG autostart entry, if there is one.
    RemoveXdg,
}

/// The backend and the steps for `probe`. `graphical`: this process runs in a graphical
/// session. `enabled`: "Start at login", or `None` to leave it as it is (the config could not
/// be loaded: its defaults would undo the user's choice); the unit is written all the same.
pub fn plan(probe: Probe, graphical: bool, enabled: Option<bool>) -> (Backend, Vec<Step>) {
    let backend = match probe {
        Probe::NoUserManager => Backend::Xdg,
        Probe::SessionTarget => Backend::Systemd,
        // In a graphical session the target would be active if the desktop used it, so the
        // unit would never start at login. From a text console or SSH it tells nothing.
        Probe::NoSessionTarget if graphical => Backend::Xdg,
        Probe::NoSessionTarget => Backend::Systemd,
    };
    let user_manager = probe != Probe::NoUserManager;
    let mut steps = Vec::new();
    if user_manager {
        steps.push(Step::WriteUnit);
    }
    if let Some(on) = enabled {
        match backend {
            Backend::Systemd => {
                steps.push(if on { Step::Enable } else { Step::Disable });
                // Left from an earlier version: it would start a second daemon.
                steps.push(Step::RemoveXdg);
            }
            _ => {
                if user_manager {
                    steps.push(Step::Disable);
                }
                steps.push(if on { Step::WriteXdg } else { Step::RemoveXdg });
            }
        }
    }
    (backend, steps)
}

/// The contents of `tomakast.service` for the executable `exe`.
pub fn unit_contents(exe: &Path) -> anyhow::Result<String> {
    Ok(format!(
        "\
# Written by tomakast on each start; edits are lost. \"Start at login\" enables it.
[Unit]
Description=tomakast launcher
PartOf=graphical-session.target
After=graphical-session.target

[Service]
ExecStart={exe} daemon
Restart=on-failure
RestartSec=1
# Another daemon is running already (one started by hand): starting again would not help.
RestartPreventExitStatus=3
# The apps the launcher started run in this unit too: only the launcher stops with it.
KillMode=process
OOMPolicy=continue

[Install]
WantedBy=graphical-session.target
",
        exe = exec_path(exe)?
    ))
}

/// `path` as the first word of `ExecStart=`: quoted, with systemd's specifiers (`%`) and
/// variables (`$`) escaped.
fn exec_path(path: &Path) -> anyhow::Result<String> {
    let path = path
        .to_str()
        .with_context(|| format!("{} is not UTF-8", path.display()))?;
    if path.contains(['\n', '\r']) {
        bail!("{path:?} contains a line break");
    }
    let mut out = String::from("\"");
    for c in path.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '%' => out.push_str("%%"),
            '$' => out.push_str("$$"),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(out)
}

/// The answer of `systemctl --user is-active graphical-session.target` (its stdout).
pub fn probe_from(is_active: &str) -> Probe {
    match is_active.trim() {
        // No user manager answered: the error went to stderr.
        "" => Probe::NoUserManager,
        "active" | "activating" | "reloading" | "refreshing" => Probe::SessionTarget,
        _ => Probe::NoSessionTarget,
    }
}

/// Talks to the user's systemd.
pub trait Systemctl {
    fn probe(&mut self) -> Probe;
    /// `systemctl --user <args>`.
    fn run(&mut self, args: &[&str]) -> anyhow::Result<()>;
}

/// Where [`apply_in`] writes.
#[derive(Clone, Debug)]
pub struct Places {
    /// `$XDG_CONFIG_HOME/systemd/user`.
    pub unit_dir: PathBuf,
    /// `$XDG_CONFIG_HOME/autostart`.
    pub autostart_dir: PathBuf,
    /// The executable the unit and the autostart entry run.
    pub exe: PathBuf,
}

/// Applies "Start at login" (`enabled`, `None` to leave it) as [`plan`] says.
pub fn apply_in(
    places: &Places,
    systemctl: &mut dyn Systemctl,
    graphical: bool,
    enabled: Option<bool>,
) -> anyhow::Result<Backend> {
    let (backend, steps) = plan(systemctl.probe(), graphical, enabled);
    for step in steps {
        match step {
            Step::WriteUnit => {
                let file = places.unit_dir.join(UNIT);
                if write_if_changed(&file, &unit_contents(&places.exe)?)? {
                    systemctl.run(&["daemon-reload"])?;
                }
            }
            Step::Enable => systemctl.run(&["enable", UNIT])?,
            Step::Disable => systemctl.run(&["disable", UNIT])?,
            Step::WriteXdg => set_autostart_in(&places.autostart_dir, true, &places.exe)?,
            Step::RemoveXdg => set_autostart_in(&places.autostart_dir, false, &places.exe)?,
        }
    }
    Ok(backend)
}

/// Writes `contents` to `file` unless it holds them already; `true` when it wrote.
fn write_if_changed(file: &Path, contents: &str) -> anyhow::Result<bool> {
    if std::fs::read_to_string(file).is_ok_and(|old| old == contents) {
        return Ok(false);
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    std::fs::write(file, contents).with_context(|| format!("writing {}", file.display()))?;
    Ok(true)
}

/// Writes (`enabled`) or removes the XDG autostart entry inside `dir`
/// (`$XDG_CONFIG_HOME/autostart`).
#[cfg(target_os = "linux")]
pub fn set_autostart_in(dir: &Path, enabled: bool, exe: &Path) -> anyhow::Result<()> {
    let file = dir.join("tomakast.desktop");
    if enabled {
        std::fs::create_dir_all(dir)?;
        let contents = format!(
            "[Desktop Entry]\nType=Application\nName=tomakast\nExec=\"{}\" daemon\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(&file, contents)?;
    } else {
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn set_autostart_in(_dir: &Path, _enabled: bool, _exe: &Path) -> anyhow::Result<()> {
    tracing::warn!("autostart is not implemented on this OS yet");
    Ok(())
}

/// Applies "Start at login" (`enabled`, `None` to leave it as it is) for the current
/// executable; see the module docs.
pub fn set_autostart(enabled: Option<bool>) -> anyhow::Result<Backend> {
    #[cfg(target_os = "linux")]
    {
        apply_in(
            &linux::places()?,
            &mut linux::System,
            linux::graphical(),
            enabled,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = enabled;
        tracing::warn!("autostart is not implemented on this OS yet");
        Ok(Backend::Unsupported)
    }
}

/// Starts the launcher's systemd user service when it is how the launcher runs here (the
/// unit is installed and the session reaches its target). `false` when it is not.
pub fn start_service() -> anyhow::Result<bool> {
    #[cfg(target_os = "linux")]
    {
        let mut systemctl = linux::System;
        let (backend, _) = plan(systemctl.probe(), linux::graphical(), None);
        if backend != Backend::Systemd || !linux::places()?.unit_dir.join(UNIT).is_file() {
            return Ok(false);
        }
        systemctl.run(&["start", UNIT])?;
        Ok(true)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(false)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};

    use anyhow::{Context as _, bail};

    use super::{Places, Probe, Systemctl, probe_from};

    pub(super) struct System;

    impl Systemctl for System {
        fn probe(&mut self) -> Probe {
            // Not booted with systemd: no need to ask.
            if !Path::new("/run/systemd/system").exists() {
                return Probe::NoUserManager;
            }
            let out = Command::new("systemctl")
                .args(["--user", "is-active", "graphical-session.target"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output();
            match out {
                Ok(out) => probe_from(&String::from_utf8_lossy(&out.stdout)),
                Err(_) => Probe::NoUserManager,
            }
        }

        fn run(&mut self, args: &[&str]) -> anyhow::Result<()> {
            let out = Command::new("systemctl")
                .args(["--user", "--quiet"])
                .args(args)
                .stdin(Stdio::null())
                .output()
                .context("running systemctl")?;
            if !out.status.success() {
                bail!(
                    "systemctl --user {}: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&out.stderr).trim()
                );
            }
            Ok(())
        }
    }

    /// The user's config dir: `$XDG_CONFIG_HOME`, else `~/.config`.
    fn config_dir() -> anyhow::Result<PathBuf> {
        match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(dir) => Ok(PathBuf::from(dir)),
            None => Ok(PathBuf::from(
                std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?,
            )
            .join(".config")),
        }
    }

    pub(super) fn places() -> anyhow::Result<Places> {
        let config = config_dir()?;
        Ok(Places {
            unit_dir: config.join("systemd").join("user"),
            autostart_dir: config.join("autostart"),
            exe: std::env::current_exe().context("finding the launcher's executable")?,
        })
    }

    /// This process runs in a graphical session.
    pub(super) fn graphical() -> bool {
        ["WAYLAND_DISPLAY", "DISPLAY"]
            .iter()
            .any(|k| std::env::var_os(k).is_some_and(|v| !v.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_that_reaches_its_target_runs_the_service() {
        use Step::*;
        assert_eq!(
            plan(Probe::SessionTarget, true, Some(true)),
            (Backend::Systemd, vec![WriteUnit, Enable, RemoveXdg])
        );
        assert_eq!(
            plan(Probe::SessionTarget, true, Some(false)),
            (Backend::Systemd, vec![WriteUnit, Disable, RemoveXdg])
        );
    }

    #[test]
    fn a_broken_config_still_writes_the_unit_and_leaves_the_rest() {
        assert_eq!(
            plan(Probe::SessionTarget, true, None),
            (Backend::Systemd, vec![Step::WriteUnit])
        );
        assert_eq!(
            plan(Probe::NoSessionTarget, true, None),
            (Backend::Xdg, vec![Step::WriteUnit])
        );
        assert_eq!(
            plan(Probe::NoUserManager, true, None),
            (Backend::Xdg, vec![])
        );
    }

    #[test]
    fn without_systemd_the_xdg_entry_is_used() {
        assert_eq!(
            plan(Probe::NoUserManager, true, Some(true)),
            (Backend::Xdg, vec![Step::WriteXdg])
        );
        assert_eq!(
            plan(Probe::NoUserManager, false, Some(false)),
            (Backend::Xdg, vec![Step::RemoveXdg])
        );
    }

    #[test]
    fn a_graphical_session_without_the_target_falls_back_to_xdg() {
        use Step::*;
        // The unit would never start at login there: it stays written, but disabled.
        assert_eq!(
            plan(Probe::NoSessionTarget, true, Some(true)),
            (Backend::Xdg, vec![WriteUnit, Disable, WriteXdg])
        );
        assert_eq!(
            plan(Probe::NoSessionTarget, true, Some(false)),
            (Backend::Xdg, vec![WriteUnit, Disable, RemoveXdg])
        );
        // From a text console the target says nothing about the desktop.
        assert_eq!(
            plan(Probe::NoSessionTarget, false, Some(true)),
            (Backend::Systemd, vec![WriteUnit, Enable, RemoveXdg])
        );
    }

    #[test]
    fn probes_read_is_active() {
        assert_eq!(probe_from(""), Probe::NoUserManager);
        assert_eq!(probe_from("active\n"), Probe::SessionTarget);
        assert_eq!(probe_from("activating\n"), Probe::SessionTarget);
        assert_eq!(probe_from("inactive\n"), Probe::NoSessionTarget);
        assert_eq!(probe_from("failed\n"), Probe::NoSessionTarget);
    }

    #[test]
    fn the_unit_runs_the_daemon_in_the_graphical_session() {
        let unit = unit_contents(Path::new("/home/u/.local/bin/tomakast")).unwrap();
        for line in [
            "ExecStart=\"/home/u/.local/bin/tomakast\" daemon",
            "Restart=on-failure",
            "RestartSec=1",
            "RestartPreventExitStatus=3",
            "KillMode=process",
            "OOMPolicy=continue",
            "PartOf=graphical-session.target",
            "After=graphical-session.target",
            "WantedBy=graphical-session.target",
        ] {
            assert!(unit.lines().any(|l| l == line), "{line} missing in\n{unit}");
        }
    }

    #[test]
    fn the_unit_escapes_the_executable_path() {
        let unit = unit_contents(Path::new(r#"/opt/a b/100%/$x/q"u\v"#)).unwrap();
        assert!(
            unit.contains(r#"ExecStart="/opt/a b/100%%/$$x/q\"u\\v" daemon"#),
            "{unit}"
        );
        assert!(unit_contents(Path::new("/opt/a\nb")).is_err());
    }

    /// Records the systemctl calls; answers `probe`.
    #[cfg(target_os = "linux")]
    struct Fake {
        probe: Probe,
        calls: Vec<String>,
    }

    #[cfg(target_os = "linux")]
    impl Systemctl for Fake {
        fn probe(&mut self) -> Probe {
            self.probe
        }
        fn run(&mut self, args: &[&str]) -> anyhow::Result<()> {
            self.calls.push(args.join(" "));
            Ok(())
        }
    }

    #[cfg(target_os = "linux")]
    fn places(dir: &Path) -> Places {
        Places {
            unit_dir: dir.join("systemd/user"),
            autostart_dir: dir.join("autostart"),
            exe: "/bin/tomakast".into(),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn applying_writes_the_unit_reloads_once_and_removes_the_old_entry() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        set_autostart_in(&places.autostart_dir, true, &places.exe).unwrap();
        let mut systemctl = Fake {
            probe: Probe::SessionTarget,
            calls: vec![],
        };
        let backend = apply_in(&places, &mut systemctl, true, Some(true)).unwrap();
        assert_eq!(backend, Backend::Systemd);
        assert_eq!(
            systemctl.calls,
            ["daemon-reload", "enable tomakast.service"]
        );
        let unit = std::fs::read_to_string(places.unit_dir.join(UNIT)).unwrap();
        assert_eq!(unit, unit_contents(&places.exe).unwrap());
        assert!(!places.autostart_dir.join("tomakast.desktop").exists());

        // The same unit again: no reload.
        systemctl.calls.clear();
        apply_in(&places, &mut systemctl, true, Some(false)).unwrap();
        assert_eq!(systemctl.calls, ["disable tomakast.service"]);

        // Another executable: written again and reloaded.
        systemctl.calls.clear();
        let moved = Places {
            exe: "/usr/bin/tomakast".into(),
            ..places.clone()
        };
        apply_in(&moved, &mut systemctl, true, None).unwrap();
        assert_eq!(systemctl.calls, ["daemon-reload"]);
        let unit = std::fs::read_to_string(places.unit_dir.join(UNIT)).unwrap();
        assert!(unit.contains("ExecStart=\"/usr/bin/tomakast\" daemon"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn applying_without_the_target_writes_the_xdg_entry() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let mut systemctl = Fake {
            probe: Probe::NoSessionTarget,
            calls: vec![],
        };
        let backend = apply_in(&places, &mut systemctl, true, Some(true)).unwrap();
        assert_eq!(backend, Backend::Xdg);
        assert_eq!(
            systemctl.calls,
            ["daemon-reload", "disable tomakast.service"]
        );
        let entry = std::fs::read_to_string(places.autostart_dir.join("tomakast.desktop")).unwrap();
        assert!(entry.contains("Exec=\"/bin/tomakast\" daemon"), "{entry}");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn applying_without_systemd_touches_no_unit() {
        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        let mut systemctl = Fake {
            probe: Probe::NoUserManager,
            calls: vec![],
        };
        apply_in(&places, &mut systemctl, true, Some(true)).unwrap();
        assert!(systemctl.calls.is_empty());
        assert!(!places.unit_dir.exists());
        assert!(places.autostart_dir.join("tomakast.desktop").is_file());
    }
}
