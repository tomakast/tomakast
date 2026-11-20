//! Plugin `open`: the scheme and path rules, checked before the opener runs. A recording opener
//! stands in for the platform one, so no test opens anything on the desktop.

mod support;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use support::harness::*;
use tomakast_core::ViewEvent;
use tomakast_plugin_host::Opener;

const SINK: &str = "kitchen-sink/indexed";

/// What reached the opener, in order.
#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl Opener for Recorder {
    fn open_url(&self, url: &str) -> anyhow::Result<()> {
        self.0.lock().unwrap().push(format!("url {url}"));
        Ok(())
    }

    fn open_path(&self, path: &Path) -> anyhow::Result<()> {
        self.0
            .lock()
            .unwrap()
            .push(format!("path {}", path.display()));
        Ok(())
    }
}

impl Recorder {
    fn take(&self) -> Vec<String> {
        std::mem::take(&mut self.0.lock().unwrap())
    }
}

fn opened_with(open_permission: bool) -> (Harness, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let opener = Arc::clone(&recorder);
    let h = harness_built(
        |p| p.manifest.permissions.open = open_permission,
        BTreeMap::new(),
        move |plugin| plugin.with_opener(opener),
    );
    (h, recorder)
}

fn open(h: &mut Harness, action: &str) -> String {
    let effects = h.run(SINK, action).expect("run answers");
    metadata(&effects)["result"].clone()
}

#[test]
fn open_url_rejects_file_and_javascript_schemes() {
    let (mut h, opened) = opened_with(true);
    for url in [
        "file:///etc/passwd",
        "javascript:alert(1)",
        "JavaScript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "ftp://example.com/x",
        "smb://server/share",
        "https://user:pw@example.com/",
        "https://example.com@evil.example/",
        "https://example.com/a b",
        "https://example.com/\nx",
        "not a url",
        "",
    ] {
        let refused = open(&mut h, &format!("open-url:{url}"));
        assert!(refused.starts_with("Err"), "{url:?}: {refused}");
    }
    assert_eq!(opened.take(), Vec::<String>::new());

    // The allowed twins reach the opener, in their parsed form.
    for url in [
        "https://example.com/x?q=1",
        "HTTP://Example.COM/x",
        "mailto:someone@example.com",
        "mailto:someone@example.com?subject=Hi&attach=/home/u/.ssh/id_ed25519&body=x",
    ] {
        assert_eq!(open(&mut h, &format!("open-url:{url}")), "Ok(())", "{url}");
    }
    assert_eq!(
        opened.take(),
        [
            "url https://example.com/x?q=1",
            "url http://example.com/x",
            "url mailto:someone@example.com",
            // Ruling AC: no attachment field reaches the mail client.
            "url mailto:someone@example.com?subject=Hi&body=x",
        ]
    );

    // Without the permission nothing opens.
    let (mut bare, opened) = opened_with(false);
    let refused = open(&mut bare, "open-url:https://example.com/");
    assert!(
        refused.starts_with("Err") && refused.contains("permission"),
        "{refused}"
    );
    assert!(opened.take().is_empty());
}

#[cfg(target_os = "linux")]
#[test]
fn open_path_rejects_executables_and_desktop_files() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let file = |name: &str, mode: u32| -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"data").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        path
    };
    let plain = file("notes.txt", 0o644);
    let script = file("run.sh", 0o755);
    let owner_exec = file("tool", 0o744);
    let desktop = file("app.desktop", 0o644);
    let upper = file("APP.DESKTOP", 0o644);
    // Ruling AA: no exec bit, but refused on every OS by extension.
    let exe = file("setup.exe", 0o644);
    let jar = file("tool.JAR", 0o644);
    symlink(&desktop, dir.join("innocent.txt")).unwrap();
    symlink(&script, dir.join("readme.txt")).unwrap();
    symlink(&plain, dir.join("evil.desktop")).unwrap();
    std::fs::create_dir(dir.join("folder")).unwrap();
    let fifo = std::ffi::CString::new(dir.join("pipe").to_str().unwrap()).unwrap();
    // SAFETY: a valid NUL-terminated path; mkfifo only creates the node.
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);

    let (mut h, opened) = opened_with(true);
    let refused_paths = [
        script.clone(),
        owner_exec,
        desktop,
        upper,
        exe,
        jar,
        dir.join("innocent.txt"),
        dir.join("readme.txt"),
        dir.join("evil.desktop"),
        dir.join("pipe"),
        dir.join("missing.txt"),
        PathBuf::from("notes.txt"),
        PathBuf::from(""),
    ];
    for path in &refused_paths {
        let refused = open(&mut h, &format!("open-path:{}", path.display()));
        // Finding 4: one generic refusal, whatever the reason (no filesystem oracle).
        assert_eq!(
            refused,
            "Err(\"this path may not be opened\")",
            "{}",
            path.display()
        );
    }
    assert_eq!(opened.take(), Vec::<String>::new());

    // The allowed twins: a plain file (also through `..`) and a folder, canonical.
    let detour = dir.join("folder/../notes.txt");
    for path in [&plain, &detour, &dir.join("folder")] {
        let result = open(&mut h, &format!("open-path:{}", path.display()));
        assert_eq!(result, "Ok(())", "{}", path.display());
    }
    assert_eq!(
        opened.take(),
        [
            format!("path {}", plain.display()),
            format!("path {}", plain.display()),
            format!("path {}", dir.join("folder").display()),
        ]
    );

    let (mut bare, opened) = opened_with(false);
    let refused = open(&mut bare, &format!("open-path:{}", plain.display()));
    assert!(
        refused.starts_with("Err") && refused.contains("permission"),
        "{refused}"
    );
    assert!(opened.take().is_empty());
}

/// Ruling X: `open` needs a user action. Called while handling a timer it fails, with the
/// permission; from `run` it works.
#[test]
fn open_from_a_timer_needs_a_user_action() {
    let (mut h, opened) = opened_with(true);
    let started = open(&mut h, "timer-reply:20:open-url:https://example.com/");
    assert_eq!(started, "Ok(1)");
    let answer = next_detail(&mut h, "open", Duration::from_secs(5)).expect("the timer fires");
    assert!(answer["result"].contains("user action"), "{answer:?}");
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("notes.txt");
    std::fs::write(&file, b"x").unwrap();
    let started = open(
        &mut h,
        &format!("timer-reply:20:open-path:{}", file.display()),
    );
    assert_eq!(started, "Ok(2)");
    let answer = next_detail(&mut h, "open", Duration::from_secs(5)).expect("the timer fires");
    assert!(answer["result"].contains("user action"), "{answer:?}");
    assert!(opened.take().is_empty(), "opened from a timer");

    // The twin: the same calls from `run`.
    assert_eq!(open(&mut h, "open-url:https://example.com/"), "Ok(())");
    assert_eq!(
        open(&mut h, &format!("open-path:{}", file.display())),
        "Ok(())"
    );
    assert_eq!(opened.take().len(), 2);
}

/// Ruling AL: a view event is a user action only when it is a chosen action or a submitted
/// form. `closed` (also sent when the launcher hides), `query-changed` and `text-changed`
/// cannot open anything; a chosen action can.
#[test]
fn open_needs_a_chosen_action_not_any_view_event() {
    let (mut h, opened) = opened_with(true);
    let view = "reply:open-url:https://example.com/";
    let result = |h: &mut Harness, event: ViewEvent| {
        let effects = h.view_event(view, event).expect("the event is answered");
        metadata(&effects)["result"].clone()
    };
    for event in [
        ViewEvent::Closed,
        ViewEvent::QueryChanged {
            revision: 1,
            text: "a".into(),
        },
        ViewEvent::TextChanged {
            revision: 1,
            text: "a".into(),
        },
    ] {
        let answer = result(&mut h, event.clone());
        assert!(answer.contains("user action"), "{event:?}: {answer}");
    }
    assert!(opened.take().is_empty(), "opened without a user action");

    // The twins: a chosen action and a submitted form open.
    let chosen = ViewEvent::ActionChosen {
        row_id: view.into(),
        action_id: "go".into(),
    };
    assert_eq!(result(&mut h, chosen), "Ok(())");
    let submitted = ViewEvent::FormSubmitted { values: Vec::new() };
    assert_eq!(result(&mut h, submitted), "Ok(())");
    assert_eq!(opened.take().len(), 2);
}

/// Ruling AL: one call opens at most once, URL and path counted together; the next call may
/// open again.
#[test]
fn one_open_per_call() {
    let (mut h, opened) = opened_with(true);
    let effects = h.run(SINK, "open-twice:https://example.com/").unwrap();
    let seen = metadata(&effects);
    assert_eq!(seen["result"], "Ok(())");
    assert!(seen["second"].contains("at most one"), "{seen:?}");
    assert!(seen["third"].contains("at most one"), "{seen:?}");
    assert_eq!(opened.take(), ["url https://example.com/"]);
    // The next call may open again.
    assert_eq!(open(&mut h, "open-url:https://example.com/"), "Ok(())");
    assert_eq!(opened.take().len(), 1);
}
