//! Which screen the launcher opens on: the screens the desktop can name for each
//! `[launcher] screen` choice, by output name (the Wayland output's name, `DP-1`).
//!
//! - KDE Plasma: a KWin script reports the screen under the pointer, the active window's and
//!   the first one whenever one changes; a show reads the last report.
//! - Hyprland: asked at show over its socket (`j/cursorpos`, `j/monitors`, `j/activewindow`).
//! - Sway, niri: the focused window's output, asked at show; they put a window on the focused
//!   output themselves, so that is all they offer.
//! - Elsewhere (GNOME, X11, macOS, Windows) nothing is offered and the desktop decides.

#[cfg(target_os = "linux")]
use std::io::{Read as _, Write as _};
#[cfg(target_os = "linux")]
use std::os::unix::net::UnixStream;
use std::sync::OnceLock;
#[cfg(target_os = "linux")]
use std::time::Duration;

use tomakast_core::ScreenChoice;

/// How long a desktop has to answer a question at show.
#[cfg(target_os = "linux")]
const ANSWER_TIMEOUT: Duration = Duration::from_millis(200);

/// The desktops that can name a screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Desktop {
    Kde,
    Hyprland,
    Sway,
    Niri,
    /// One that cannot (or a platform other than Wayland on Linux).
    Other,
}

/// The running desktop, told once from the environment.
pub fn desktop() -> Desktop {
    static DESKTOP: OnceLock<Desktop> = OnceLock::new();
    *DESKTOP.get_or_init(|| detect(&|name| std::env::var(name).ok()))
}

/// The desktop the environment `var` describes.
fn detect(var: &dyn Fn(&str) -> Option<String>) -> Desktop {
    let set = |name: &str| var(name).is_some_and(|value| !value.is_empty());
    if !cfg!(target_os = "linux") || !set("WAYLAND_DISPLAY") {
        return Desktop::Other;
    }
    if set("HYPRLAND_INSTANCE_SIGNATURE") {
        Desktop::Hyprland
    } else if set("SWAYSOCK") {
        Desktop::Sway
    } else if set("NIRI_SOCKET") {
        Desktop::Niri
    } else if var("XDG_CURRENT_DESKTOP")
        .is_some_and(|desktops| desktops.split(':').any(|d| d.eq_ignore_ascii_case("KDE")))
    {
        Desktop::Kde
    } else {
        Desktop::Other
    }
}

/// The choices `desktop` can honour, in the schema's order.
fn choices_of(desktop: Desktop) -> &'static [ScreenChoice] {
    use ScreenChoice::*;
    match desktop {
        Desktop::Kde => &[Mouse, ActiveWindow, Primary],
        Desktop::Hyprland => &[Mouse, ActiveWindow],
        Desktop::Sway | Desktop::Niri => &[ActiveWindow],
        Desktop::Other => &[],
    }
}

/// The choices the running desktop can honour, in the schema's order; empty where it can
/// name none (the launcher then opens where the desktop puts it).
pub fn screen_choices() -> &'static [ScreenChoice] {
    choices_of(desktop())
}

/// The output `choice` names now; `None`: the desktop decides (it cannot tell, or the choice
/// is not one it offers).
pub fn screen_for(choice: ScreenChoice) -> Option<String> {
    if !screen_choices().contains(&choice) {
        return None;
    }
    #[cfg(target_os = "linux")]
    let screen = match desktop() {
        Desktop::Kde => kde::reported(choice),
        Desktop::Hyprland => hyprland::screen(choice),
        Desktop::Sway => sway::active_output(),
        Desktop::Niri => niri::active_output(),
        Desktop::Other => None,
    };
    #[cfg(not(target_os = "linux"))]
    let screen = None;
    tracing::debug!(?choice, ?screen, "screen for the launcher");
    screen
}

/// Starts following the screens where the desktop tells them as they change (KDE); a no-op
/// elsewhere. `instance` tells this daemon's KWin script from another daemon's (its home).
pub fn start_tracking(instance: &str) {
    #[cfg(target_os = "linux")]
    if desktop() == Desktop::Kde {
        kde::start(instance);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = instance;
}

/// A socket to `path`, with the answer timeout.
#[cfg(target_os = "linux")]
fn connect(path: &str) -> Option<UnixStream> {
    let stream = UnixStream::connect(path)
        .inspect_err(|e| tracing::debug!(%path, error = %e, "desktop socket"))
        .ok()?;
    stream.set_read_timeout(Some(ANSWER_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(ANSWER_TIMEOUT)).ok()?;
    Some(stream)
}

#[cfg(target_os = "linux")]
mod hyprland {
    use super::*;
    use serde_json::Value;

    /// The output for `choice`, asked over the instance's socket.
    pub(super) fn screen(choice: ScreenChoice) -> Option<String> {
        let monitors = parse(&ask("j/monitors")?)?;
        match choice {
            ScreenChoice::Mouse => {
                let at = cursor(&parse(&ask("j/cursorpos")?)?)?;
                monitor_at(&monitors, at)
            }
            ScreenChoice::ActiveWindow => {
                active_monitor(&parse(&ask("j/activewindow")?)?, &monitors)
            }
            ScreenChoice::Primary => None,
        }
    }

    fn parse(text: &str) -> Option<Value> {
        serde_json::from_str(text)
            .inspect_err(|e| tracing::debug!(error = %e, "Hyprland answer"))
            .ok()
    }

    /// Hyprland's answer to `request`.
    fn ask(request: &str) -> Option<String> {
        let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let paths = [
            format!("{runtime}/hypr/{signature}/.socket.sock"),
            format!("/tmp/hypr/{signature}/.socket.sock"),
        ];
        let mut stream = paths.iter().find_map(|path| connect(path))?;
        stream.write_all(request.as_bytes()).ok()?;
        let mut answer = String::new();
        stream.read_to_string(&mut answer).ok()?;
        Some(answer)
    }

    /// The pointer, in the layout's logical pixels.
    pub(super) fn cursor(answer: &Value) -> Option<(f64, f64)> {
        Some((answer["x"].as_f64()?, answer["y"].as_f64()?))
    }

    /// The monitor whose area holds `at`. Hyprland gives a monitor's size in its own pixels:
    /// its area in the layout is that over its scale, its sides swapped when turned a quarter.
    pub(super) fn monitor_at(monitors: &Value, (x, y): (f64, f64)) -> Option<String> {
        monitors.as_array()?.iter().find_map(|monitor| {
            let left = monitor["x"].as_f64()?;
            let top = monitor["y"].as_f64()?;
            let scale = monitor["scale"].as_f64().filter(|s| *s > 0.).unwrap_or(1.);
            let (mut width, mut height) = (
                monitor["width"].as_f64()? / scale,
                monitor["height"].as_f64()? / scale,
            );
            if monitor["transform"].as_u64().is_some_and(|t| t % 2 == 1) {
                std::mem::swap(&mut width, &mut height);
            }
            let inside = (left..left + width).contains(&x) && (top..top + height).contains(&y);
            inside.then(|| monitor["name"].as_str().map(str::to_string))?
        })
    }

    /// The name of the monitor of the active window; `None` with no active window (`{}`).
    pub(super) fn active_monitor(window: &Value, monitors: &Value) -> Option<String> {
        let id = window["monitor"].as_i64()?;
        monitors
            .as_array()?
            .iter()
            .find(|m| m["id"].as_i64() == Some(id))
            .and_then(|m| m["name"].as_str())
            .map(str::to_string)
    }
}

#[cfg(target_os = "linux")]
mod sway {
    use super::*;
    use serde_json::Value;

    /// The i3 IPC message type that lists the workspaces.
    const GET_WORKSPACES: u32 = 1;
    const MAGIC: &[u8] = b"i3-ipc";

    /// The output of the focused workspace: the focused window's, or the focused output
    /// when it has none.
    pub(super) fn active_output() -> Option<String> {
        let path = std::env::var("SWAYSOCK").ok()?;
        let mut stream = connect(&path)?;
        let mut request = MAGIC.to_vec();
        request.extend(0u32.to_ne_bytes());
        request.extend(GET_WORKSPACES.to_ne_bytes());
        stream.write_all(&request).ok()?;
        let mut header = [0u8; 14];
        stream.read_exact(&mut header).ok()?;
        if &header[..6] != MAGIC {
            return None;
        }
        let len = u32::from_ne_bytes(header[6..10].try_into().ok()?) as usize;
        let mut body = vec![0u8; len];
        stream.read_exact(&mut body).ok()?;
        focused_output(&serde_json::from_slice(&body).ok()?)
    }

    pub(super) fn focused_output(workspaces: &Value) -> Option<String> {
        workspaces
            .as_array()?
            .iter()
            .find(|w| w["focused"].as_bool() == Some(true))
            .and_then(|w| w["output"].as_str())
            .map(str::to_string)
    }
}

#[cfg(target_os = "linux")]
mod niri {
    use super::*;
    use serde_json::Value;

    /// The output of the focused window's workspace; `None` with no focused window.
    pub(super) fn active_output() -> Option<String> {
        let window = ask("\"FocusedWindow\"")?;
        let workspaces = ask("\"Workspaces\"")?;
        window_output(&window, &workspaces)
    }

    /// niri's answer to `request`, one JSON line each way.
    fn ask(request: &str) -> Option<Value> {
        let path = std::env::var("NIRI_SOCKET").ok()?;
        let mut stream = connect(&path)?;
        stream.write_all(format!("{request}\n").as_bytes()).ok()?;
        stream.shutdown(std::net::Shutdown::Write).ok()?;
        let mut answer = String::new();
        stream.read_to_string(&mut answer).ok()?;
        serde_json::from_str(answer.lines().next()?).ok()
    }

    pub(super) fn window_output(window: &Value, workspaces: &Value) -> Option<String> {
        let workspace = window["Ok"]["FocusedWindow"]["workspace_id"].as_u64()?;
        workspaces["Ok"]["Workspaces"]
            .as_array()?
            .iter()
            .find(|w| w["id"].as_u64() == Some(workspace))
            .and_then(|w| w["output"].as_str())
            .map(str::to_string)
    }
}

#[cfg(target_os = "linux")]
mod kde {
    use std::sync::Mutex;

    use super::*;
    use zbus::blocking::{Connection, MessageIterator, connection};

    /// Where the script reports to, in this daemon.
    const PATH: &str = "/dev/tomakast/Screens";
    const INTERFACE: &str = "dev.tomakast.Screens";

    /// The last report: the screen under the pointer, the active window's, the first one.
    static REPORT: Mutex<Option<[String; 3]>> = Mutex::new(None);

    pub(super) fn reported(choice: ScreenChoice) -> Option<String> {
        let report = REPORT.lock().ok()?.clone()?;
        let name = match choice {
            ScreenChoice::Mouse => &report[0],
            ScreenChoice::ActiveWindow => &report[1],
            ScreenChoice::Primary => &report[2],
        };
        (!name.is_empty()).then(|| name.clone())
    }

    struct Screens;

    #[zbus::interface(name = "dev.tomakast.Screens")]
    impl Screens {
        fn report(&self, pointer: String, active: String, first: String) {
            tracing::debug!(%pointer, %active, %first, "KWin reports the screens");
            if let Ok(mut report) = REPORT.lock() {
                *report = Some([pointer, active, first]);
            }
        }
    }

    pub(super) fn start(instance: &str) {
        let plugin = plugin_name(instance);
        let spawned = std::thread::Builder::new()
            .name("kwin-screens".into())
            .spawn(move || {
                if let Err(e) = run(&plugin) {
                    tracing::warn!(error = %e, "the screens KWin reports are not followed");
                }
            });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "no thread to follow the screens");
        }
    }

    /// The script's name in KWin for the daemon of `instance`: one per home, so two daemons
    /// (a dev run and the installed one) keep their own.
    pub(super) fn plugin_name(instance: &str) -> String {
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        instance.hash(&mut hasher);
        format!("tomakast-screens-{:016x}", hasher.finish())
    }

    /// Serves the report, loads the script, and loads it again whenever KWin starts again.
    fn run(plugin: &str) -> zbus::Result<()> {
        let conn = connection::Builder::session()?
            .serve_at(PATH, Screens)?
            .build()?;
        let service = conn
            .unique_name()
            .map(|name| name.to_string())
            .ok_or_else(|| zbus::Error::Failure("no unique name".into()))?;
        let script = std::env::temp_dir().join(format!("{plugin}.js"));
        std::fs::write(&script, script_source(&service))
            .map_err(|e| zbus::Error::Failure(format!("writing the script: {e}")))?;
        let script = script.to_string_lossy().into_owned();
        load(&conn, &script, plugin);
        let owners = MessageIterator::for_match_rule(
            "type='signal',sender='org.freedesktop.DBus',interface='org.freedesktop.DBus',\
             member='NameOwnerChanged',arg0='org.kde.KWin'",
            &conn,
            Some(8),
        )?;
        for message in owners {
            let Ok(message) = message else { continue };
            let Ok((_, _, new)) = message.body().deserialize::<(String, String, String)>() else {
                continue;
            };
            if !new.is_empty() {
                tracing::debug!("KWin started again; the screens script is loaded again");
                load(&conn, &script, plugin);
            }
        }
        Ok(())
    }

    /// Unloads a script left under `plugin` (by an earlier daemon), loads `script` and runs it.
    fn load(conn: &Connection, script: &str, plugin: &str) {
        const SCRIPTING: Option<&str> = Some("org.kde.kwin.Scripting");
        let kwin = Some("org.kde.KWin");
        let _ = conn.call_method(kwin, "/Scripting", SCRIPTING, "unloadScript", &(plugin,));
        let loaded = conn
            .call_method(
                kwin,
                "/Scripting",
                SCRIPTING,
                "loadScript",
                &(script, plugin),
            )
            .and_then(|reply| reply.body().deserialize::<i32>());
        let id = match loaded {
            Ok(id) if id >= 0 => id,
            Ok(id) => {
                tracing::warn!(id, "KWin did not load the screens script");
                return;
            }
            Err(e) => {
                tracing::warn!(error = %e, "KWin did not load the screens script");
                return;
            }
        };
        let path = format!("/Scripting/Script{id}");
        if let Err(e) =
            conn.call_method(kwin, path.as_str(), Some("org.kde.kwin.Script"), "run", &())
        {
            tracing::warn!(error = %e, "KWin did not run the screens script");
        }
    }

    /// The KWin script: reports the three screens to `service` when one of them changes.
    pub(super) fn script_source(service: &str) -> String {
        format!(
            r#"// Written by tomakast: tells it which screen is under the pointer, the active
// window's and the first one, when one of them changes.
let last = "";
function name(output) {{ return output ? output.name : ""; }}
function report() {{
    const pointer = name(workspace.screenAt(workspace.cursorPos));
    const active = name(workspace.activeWindow ? workspace.activeWindow.output : null);
    const first = name(workspace.screens.length > 0 ? workspace.screens[0] : null);
    const now = [pointer, active, first].join("\n");
    if (now === last) return;
    last = now;
    callDBus("{service}", "{PATH}", "{INTERFACE}", "Report", pointer, active, first);
}}
let watched = null;
function activated(window) {{
    if (watched) watched.outputChanged.disconnect(report);
    watched = window;
    if (window) window.outputChanged.connect(report);
    report();
}}
workspace.cursorPosChanged.connect(report);
workspace.windowActivated.connect(activated);
workspace.screensChanged.connect(report);
activated(workspace.activeWindow);
"#
        )
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use serde_json::json;

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.to_string())
        }
    }

    #[test]
    fn the_desktop_is_told_from_the_environment() {
        let wayland = ("WAYLAND_DISPLAY", "wayland-0");
        let cases: [(&[(&str, &str)], Desktop); 6] = [
            (&[wayland, ("XDG_CURRENT_DESKTOP", "KDE")], Desktop::Kde),
            (
                &[
                    wayland,
                    ("HYPRLAND_INSTANCE_SIGNATURE", "abc"),
                    ("XDG_CURRENT_DESKTOP", "KDE"),
                ],
                Desktop::Hyprland,
            ),
            (&[wayland, ("SWAYSOCK", "/run/sway.sock")], Desktop::Sway),
            (&[wayland, ("NIRI_SOCKET", "/run/niri.sock")], Desktop::Niri),
            (
                &[wayland, ("XDG_CURRENT_DESKTOP", "ubuntu:GNOME")],
                Desktop::Other,
            ),
            // X11: nothing is offered.
            (
                &[("XDG_CURRENT_DESKTOP", "KDE"), ("DISPLAY", ":0")],
                Desktop::Other,
            ),
        ];
        for (vars, desktop) in cases {
            assert_eq!(detect(&env(vars)), desktop, "{vars:?}");
        }
    }

    #[test]
    fn each_desktop_offers_what_it_can_name() {
        use ScreenChoice::*;
        assert_eq!(choices_of(Desktop::Kde), [Mouse, ActiveWindow, Primary]);
        assert_eq!(choices_of(Desktop::Hyprland), [Mouse, ActiveWindow]);
        assert_eq!(choices_of(Desktop::Sway), [ActiveWindow]);
        assert_eq!(choices_of(Desktop::Niri), [ActiveWindow]);
        assert!(choices_of(Desktop::Other).is_empty());
    }

    /// `hyprctl -j monitors` of two monitors: a 4K one at scale 1.5 and a 1080p one turned a
    /// quarter, right of it.
    fn hyprland_monitors() -> serde_json::Value {
        json!([
            {"id": 0, "name": "DP-1", "x": 0, "y": 0, "width": 3840, "height": 2160,
             "scale": 1.5, "transform": 0, "focused": true},
            {"id": 1, "name": "HDMI-A-1", "x": 2560, "y": 0, "width": 1920, "height": 1080,
             "scale": 1.0, "transform": 1, "focused": false}
        ])
    }

    #[test]
    fn hyprland_the_pointers_monitor() {
        let monitors = hyprland_monitors();
        let at = hyprland::cursor(&json!({"x": 2559, "y": 1439})).unwrap();
        assert_eq!(hyprland::monitor_at(&monitors, at).as_deref(), Some("DP-1"));
        // Turned: 1080 wide, 1920 high.
        assert_eq!(
            hyprland::monitor_at(&monitors, (2600., 1900.)).as_deref(),
            Some("HDMI-A-1")
        );
        assert_eq!(hyprland::monitor_at(&monitors, (3700., 100.)), None);
    }

    #[test]
    fn hyprland_the_active_windows_monitor() {
        let monitors = hyprland_monitors();
        let window = json!({"address": "0x5a", "monitor": 1, "workspace": {"id": 2}});
        assert_eq!(
            hyprland::active_monitor(&window, &monitors).as_deref(),
            Some("HDMI-A-1")
        );
        // No active window.
        assert_eq!(hyprland::active_monitor(&json!({}), &monitors), None);
    }

    #[test]
    fn sway_the_focused_workspaces_output() {
        let workspaces = json!([
            {"num": 1, "name": "1", "focused": false, "visible": true, "output": "eDP-1"},
            {"num": 2, "name": "2", "focused": true, "visible": true, "output": "DP-2"}
        ]);
        assert_eq!(sway::focused_output(&workspaces).as_deref(), Some("DP-2"));
        assert_eq!(sway::focused_output(&json!([])), None);
    }

    #[test]
    fn niri_the_focused_windows_output() {
        let window = json!({"Ok": {"FocusedWindow": {"id": 7, "title": "x", "workspace_id": 3}}});
        let workspaces = json!({"Ok": {"Workspaces": [
            {"id": 1, "idx": 1, "output": "eDP-1", "is_focused": false},
            {"id": 3, "idx": 1, "output": "DP-1", "is_focused": true}
        ]}});
        assert_eq!(
            niri::window_output(&window, &workspaces).as_deref(),
            Some("DP-1")
        );
        let none = json!({"Ok": {"FocusedWindow": null}});
        assert_eq!(niri::window_output(&none, &workspaces), None);
    }

    #[test]
    fn the_kwin_script_reports_to_this_daemon() {
        let source = kde::script_source(":1.42");
        assert!(source.contains(
            r#"callDBus(":1.42", "/dev/tomakast/Screens", "dev.tomakast.Screens", "Report""#
        ));
        assert_ne!(kde::plugin_name("/home/a"), kde::plugin_name("/home/b"));
        assert_eq!(kde::plugin_name("/home/a"), kde::plugin_name("/home/a"));
    }
}
