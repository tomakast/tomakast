//! Throwaway probe of the GlobalShortcuts portal on the user's desktop (item hotkeys plan,
//! Task 1). Run it with `just hotkey-probe` and follow the prompts. It registers as
//! `dev.tomakast.HotkeyProbe`, never as tomakast itself, binds only Ctrl+Alt+Shift+K/L/U/I,
//! and writes what it sees to stdout and to `$XDG_RUNTIME_DIR/tk-hotkey-probe.txt`.

#[cfg(target_os = "linux")]
fn main() {
    probe::main();
}

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
mod probe {
    use std::io::{BufRead as _, Write as _};
    use std::sync::{Mutex, OnceLock};

    use ashpd::desktop::Session;
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt as _;

    const APP_ID: &str = "dev.tomakast.HotkeyProbe";

    static REPORT: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

    fn say(line: &str) {
        println!("{line}");
        if let Some(file) = REPORT.get() {
            let _ = writeln!(file.lock().unwrap(), "{line}");
        }
    }

    fn wait(prompt: &str) {
        say("");
        say(&format!(">>> {prompt}"));
        say(">>> Press Enter here when done.");
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
    }

    fn ask(question: &str) -> String {
        say("");
        say(&format!("??? {question}"));
        say("??? Type the answer and press Enter.");
        let mut line = String::new();
        let _ = std::io::stdin().lock().read_line(&mut line);
        say(&format!("    answer: {}", line.trim()));
        line
    }

    async fn close(session: &Option<Session<GlobalShortcuts>>, name: &str) {
        if let Some(session) = session {
            match session.close().await {
                Ok(()) => say(&format!("[{name}] closed")),
                Err(e) => say(&format!("[{name}] close failed: {e}")),
            }
        }
    }

    fn shortcut(id: &str, trigger: &str) -> NewShortcut {
        NewShortcut::new(id, format!("tomakast probe {id}")).preferred_trigger(trigger)
    }

    /// Creates a session and binds `shortcuts`; prints the outcome. `None` when refused.
    async fn bind(
        portal: &GlobalShortcuts,
        label: &str,
        shortcuts: &[NewShortcut],
    ) -> Option<Session<GlobalShortcuts>> {
        let session = match portal.create_session(Default::default()).await {
            Ok(session) => session,
            Err(e) => {
                say(&format!("[{label}] create_session failed: {e}"));
                return None;
            }
        };
        say(&format!(
            "[{label}] session created; binding {} shortcut(s)…",
            shortcuts.len()
        ));
        let request = match portal
            .bind_shortcuts(&session, shortcuts, None, Default::default())
            .await
        {
            Ok(request) => request,
            Err(e) => {
                say(&format!("[{label}] bind_shortcuts call failed: {e}"));
                let _ = session.close().await;
                return None;
            }
        };
        match request.response() {
            Ok(response) => {
                say(&format!("[{label}] bind ACCEPTED:"));
                for s in response.shortcuts() {
                    say(&format!(
                        "    id={:?} description={:?} trigger_description={:?}",
                        s.id(),
                        s.description(),
                        s.trigger_description()
                    ));
                }
                Some(session)
            }
            Err(e) => {
                say(&format!("[{label}] bind REFUSED: {e}"));
                let _ = session.close().await;
                None
            }
        }
    }

    fn kde_entry() {
        let path = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_default()
            .join(".config/kglobalshortcutsrc");
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let mut inside = false;
        say(&format!("--- {} section [{APP_ID}]:", path.display()));
        for line in text.lines() {
            if line.starts_with('[') {
                inside = line == format!("[{APP_ID}]");
            }
            if inside {
                say(&format!("    {line}"));
            }
        }
        say("--- end");
    }

    async fn listen(listener: GlobalShortcuts, kind: &'static str) {
        if kind == "activated" {
            let Ok(mut events) = listener.receive_activated().await else {
                say("cannot listen to Activated");
                return;
            };
            while let Some(ev) = events.next().await {
                say(&format!(
                    "  ** ACTIVATED id={:?} at={:?} session={}",
                    ev.shortcut_id(),
                    ev.timestamp(),
                    ev.session_handle()
                ));
            }
        } else {
            let Ok(mut events) = listener.receive_shortcuts_changed().await else {
                say("cannot listen to ShortcutsChanged");
                return;
            };
            while let Some(ev) = events.next().await {
                say(&format!(
                    "  ** SHORTCUTS CHANGED session={}",
                    ev.session_handle()
                ));
                for s in ev.shortcuts() {
                    say(&format!(
                        "       id={:?} trigger_description={:?}",
                        s.id(),
                        s.trigger_description()
                    ));
                }
            }
        }
    }

    pub fn main() {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_else(|| "/tmp".into());
        let path = std::path::Path::new(&dir).join("tk-hotkey-probe.txt");
        if let Ok(file) = std::fs::File::create(&path) {
            let _ = REPORT.set(Mutex::new(file));
        }
        say(&format!("report: {}", path.display()));
        async_io::block_on(run());
    }

    /// The portal registers a host app only when a desktop file of that id exists; without
    /// one, KDE files the shortcuts under whichever app started this process.
    fn desktop_file() -> std::path::PathBuf {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                    .join(".local/share")
            });
        data.join("applications").join(format!("{APP_ID}.desktop"))
    }

    async fn run() {
        let desktop = desktop_file();
        let _ = std::fs::create_dir_all(desktop.parent().unwrap());
        match std::fs::write(
            &desktop,
            "[Desktop Entry]\nType=Application\nName=tomakast hotkey probe\nExec=true\nNoDisplay=true\n",
        ) {
            Ok(()) => say(&format!("wrote {} (removed at the end)", desktop.display())),
            Err(e) => say(&format!("cannot write {}: {e}", desktop.display())),
        }
        // The portal notices a new desktop file a moment after it is written.
        for attempt in 1..=10 {
            let Ok(id) = ashpd::AppID::try_from(APP_ID) else {
                say("bad app id");
                break;
            };
            match ashpd::register_host_app(id).await {
                Ok(()) => {
                    say(&format!("host app registered (attempt {attempt})"));
                    break;
                }
                Err(e) => {
                    say(&format!(
                        "host app registration failed (attempt {attempt}): {e}"
                    ));
                    async_io::Timer::after(std::time::Duration::from_millis(500)).await;
                }
            }
        }
        let portal = match GlobalShortcuts::new().await {
            Ok(portal) => portal,
            Err(e) => {
                say(&format!("GlobalShortcuts portal unavailable: {e}"));
                return;
            }
        };
        say(&format!("portal version {}", portal.version()));

        // Activations and changes, printed as they arrive, each on its own thread.
        for kind in ["activated", "changed"] {
            let listener = match GlobalShortcuts::new().await {
                Ok(listener) => listener,
                Err(e) => {
                    say(&format!("listener proxy failed: {e}"));
                    return;
                }
            };
            std::thread::spawn(move || async_io::block_on(listen(listener, kind)));
        }

        // Round 2 (round 1 is in git history): the item-hotkeys scheme itself — each change
        // binds the whole set in a new session, then the previous session is closed.
        let k = "CTRL+ALT+SHIFT+k";
        let l = "CTRL+ALT+SHIFT+l";
        let u = "CTRL+ALT+SHIFT+u";
        let i = "CTRL+ALT+SHIFT+i";

        say("");
        say("== Step 1: session P binds x1 (Ctrl+Alt+Shift+K) and x2 (Ctrl+Alt+Shift+L)");
        say("   If a KDE dialog appears, press OK.");
        let p = bind(&portal, "P", &[shortcut("x1", k), shortcut("x2", l)]).await;
        ask("Did a KDE dialog appear? (y/n)");
        wait("Press Ctrl+Alt+Shift+K, then Ctrl+Alt+Shift+L");

        say("");
        say(
            "== Step 2: session Q binds x1 (K, unchanged) and x3 (Ctrl+Alt+Shift+U, new); x2 is dropped",
        );
        say("   If a KDE dialog appears, press OK.");
        let q = bind(&portal, "Q", &[shortcut("x1", k), shortcut("x3", u)]).await;
        ask("Did a dialog appear? If yes, which keys did it list (K, U, both)?");
        close(&p, "P").await;
        kde_entry();
        wait("P is closed. Press K, then L (dropped), then U");

        say("");
        say(
            "== Step 3: session R binds x1 (K), x3 (U) and x4 (Ctrl+Alt+Shift+L, new id on x2's old key)",
        );
        say("   If a dialog appears, read what it says about L, then press OK.");
        let r = bind(
            &portal,
            "R",
            &[shortcut("x1", k), shortcut("x3", u), shortcut("x4", l)],
        )
        .await;
        ask("Did a dialog appear? Did it say L is taken or conflicting?");
        close(&q, "Q").await;
        kde_entry();
        wait("Q is closed. Press K, then L, then U");

        say("");
        say("== Step 4: session S binds x1, x3, x4 and x5 (Ctrl+Alt+Shift+I, new)");
        say("   A dialog should appear: press CANCEL.");
        let s = bind(
            &portal,
            "S",
            &[
                shortcut("x1", k),
                shortcut("x3", u),
                shortcut("x4", l),
                shortcut("x5", i),
            ],
        )
        .await;
        ask("Which keys did the dialog list (only I, or all)?");
        wait("Press K, then I");
        close(&s, "S").await;
        wait("S (the cancelled one) is closed. Press K again");

        close(&r, "R").await;
        wait("All closed. Press K once more (should fire nothing)");
        kde_entry();
        say("");
        let _ = std::fs::remove_file(&desktop);
        say(&format!("removed {}", desktop.display()));
        say("Probe finished. To clean up: System Settings › Keyboard › Shortcuts → remove");
        say(
            "the \"dev.tomakast.HotkeyProbe\" entry (or delete its section from ~/.config/kglobalshortcutsrc).",
        );
    }
}
