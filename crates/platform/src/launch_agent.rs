//! Launch at login on macOS: a launchd agent, `~/Library/LaunchAgents/<LABEL>.plist`, that
//! starts the launcher when the user logs in and again after it fails. Unlike the systemd
//! unit the file exists only while "Start at login" is on: launchd loads every agent there at
//! login, and an agent that is loaded starts at once. With the switch off nothing restarts the
//! launcher after a crash.
//!
//! The launcher only writes or deletes the file; it never unloads its own agent, which would
//! stop it. `just install` and a bare `tomakast` load it ([`crate::autostart::start_service`]).
//!
//! The text of the file and the decisions are plain Rust, tested on every OS.

use std::path::{Path, PathBuf};

use anyhow::Context as _;

/// The agent's label, and the app bundle's identifier.
pub const LABEL: &str = "app.tomakast";

/// Set in the agent's environment: the daemon runs under launchd. A second daemon started
/// there exits 0, which launchd does not restart, and the log has no colour codes.
pub const AGENT_ENV: &str = "TOMAKAST_LAUNCH_AGENT";

/// Where the agent lives and what it runs.
#[derive(Clone, Debug)]
pub struct AgentPlaces {
    /// `~/Library/LaunchAgents/<LABEL>.plist`.
    pub file: PathBuf,
    /// `~/Library/Logs/tomakast.log`: the daemon's output.
    pub log: PathBuf,
    /// The executable the agent runs.
    pub exe: PathBuf,
}

impl AgentPlaces {
    /// The places under the home folder `home`, for `exe`.
    pub fn under(home: &Path, exe: PathBuf) -> AgentPlaces {
        AgentPlaces {
            file: home
                .join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
            log: home.join("Library/Logs/tomakast.log"),
            exe,
        }
    }
}

/// Whether this process runs under the launcher's agent.
pub fn under_agent() -> bool {
    std::env::var_os(AGENT_ENV).is_some_and(|v| v == "1")
}

/// `text` escaped for an XML string.
fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

fn utf8(path: &Path) -> anyhow::Result<String> {
    path.to_str()
        .map(xml_escape)
        .with_context(|| format!("{} is not UTF-8", path.display()))
}

/// The text of the agent for `places`.
pub fn agent_contents(places: &AgentPlaces) -> anyhow::Result<String> {
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!-- Written by tomakast while "Start at login" is on; edits are lost. -->
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{exe}</string>
		<string>daemon</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<!-- Started again when it fails; `tomakast quit` exits 0 and stays stopped. -->
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<!-- A launcher: never slowed down as a background job. -->
	<key>ProcessType</key>
	<string>Interactive</string>
	<key>LimitLoadToSessionType</key>
	<string>Aqua</string>
	<key>EnvironmentVariables</key>
	<dict>
		<key>{env}</key>
		<string>1</string>
	</dict>
	<key>StandardOutPath</key>
	<string>{log}</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#,
        label = LABEL,
        exe = utf8(&places.exe)?,
        env = AGENT_ENV,
        log = utf8(&places.log)?,
    ))
}

/// What [`apply_agent`] does to the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStep {
    /// Writes it (with the current executable), if it holds anything else.
    Write,
    /// Deletes it, if it is there.
    Remove,
    /// Leaves it alone.
    Keep,
}

/// The step for "Start at login" `enabled` (`None`: leave it as it is, but a file that is
/// there runs the current executable) when the file `exists`.
pub fn agent_step(enabled: Option<bool>, exists: bool) -> AgentStep {
    match enabled {
        Some(true) => AgentStep::Write,
        Some(false) => AgentStep::Remove,
        None if exists => AgentStep::Write,
        None => AgentStep::Keep,
    }
}

/// Applies "Start at login" (`enabled`, `None` to leave it) to the agent file.
pub fn apply_agent(places: &AgentPlaces, enabled: Option<bool>) -> anyhow::Result<()> {
    match agent_step(enabled, places.file.is_file()) {
        AgentStep::Write => {
            crate::autostart::write_if_changed(&places.file, &agent_contents(places)?)?;
        }
        AgentStep::Remove => match std::fs::remove_file(&places.file) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(
                    anyhow::Error::new(e).context(format!("removing {}", places.file.display()))
                );
            }
        },
        AgentStep::Keep => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(dir: &Path) -> AgentPlaces {
        AgentPlaces::under(
            dir,
            dir.join("Applications/tomakast.app/Contents/MacOS/tomakast"),
        )
    }

    #[test]
    fn the_agent_runs_the_daemon_and_restarts_it_after_a_failure() {
        let places = AgentPlaces::under(
            Path::new("/Users/me"),
            "/Users/me/Applications/tomakast.app/Contents/MacOS/tomakast".into(),
        );
        assert_eq!(
            places.file,
            Path::new("/Users/me/Library/LaunchAgents/app.tomakast.plist")
        );
        let text = agent_contents(&places).unwrap();
        for wanted in [
            "<string>app.tomakast</string>",
            "<string>/Users/me/Applications/tomakast.app/Contents/MacOS/tomakast</string>\n\t\t<string>daemon</string>",
            "<key>SuccessfulExit</key>\n\t\t<false/>",
            "<key>TOMAKAST_LAUNCH_AGENT</key>\n\t\t<string>1</string>",
            "<string>/Users/me/Library/Logs/tomakast.log</string>",
        ] {
            assert!(text.contains(wanted), "{wanted:?} missing from\n{text}");
        }
    }

    #[test]
    fn the_agent_escapes_its_paths() {
        let places = AgentPlaces::under(Path::new("/Users/a&b"), "/Apps/<x> \"y\".app/t".into());
        let text = agent_contents(&places).unwrap();
        assert!(text.contains("<string>/Apps/&lt;x&gt; &quot;y&quot;.app/t</string>"));
        assert!(text.contains("/Users/a&amp;b/Library/Logs/tomakast.log"));
        assert!(!text.contains("a&b"));
    }

    #[test]
    fn the_file_is_there_exactly_while_start_at_login_is_on() {
        assert_eq!(agent_step(Some(true), false), AgentStep::Write);
        assert_eq!(agent_step(Some(false), true), AgentStep::Remove);
        // A config that cannot be read leaves the choice alone, but a file there runs this
        // executable.
        assert_eq!(agent_step(None, true), AgentStep::Write);
        assert_eq!(agent_step(None, false), AgentStep::Keep);

        let dir = tempfile::tempdir().unwrap();
        let places = places(dir.path());
        apply_agent(&places, Some(true)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&places.file).unwrap(),
            agent_contents(&places).unwrap()
        );
        apply_agent(&places, None).unwrap();
        assert!(places.file.is_file());
        apply_agent(&places, Some(false)).unwrap();
        assert!(!places.file.exists());
        apply_agent(&places, Some(false)).unwrap();
        apply_agent(&places, None).unwrap();
        assert!(!places.file.exists());
    }
}
