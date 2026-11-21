//! The threat model of the plugin host, end to end: the kitchen-sink plugin, driven through a
//! core as the daemon drives it, tries what a malicious plugin would try. Each test asserts the
//! denial and that the host keeps running (the plugin still answers, or only it stops).
//!
//! Denials tested elsewhere and not repeated here:
//! - deadlines, memory, traps, blocking WASI calls, oversized answers, storage and preference
//!   isolation: `tests/plugin.rs` (`infinite_loop_in_search_traps_and_marks_broken`,
//!   `memory_hog_traps_at_64_mib`, `blocking_wasi_call_is_refused`,
//!   `oversized_return_value_breaks_the_plugin`, `storage_without_permission_errors`, …);
//! - kind and source on the init and search paths: `init_commands_are_indexed_with_source`,
//!   `search_roundtrip` (`tests/plugin.rs`);
//! - a symlink to a file inside the folder, a missing file, a folder as icon:
//!   `unresolvable_plugin_file_icon_is_dropped` (`tests/plugin.rs`);
//! - every id field with control, format or oversized characters, effect by effect:
//!   `caps::tests::ids_have_no_control_or_format_characters`;
//! - permissions and quotas of effects, timers, HTTP and `open`: `tests/effects.rs`,
//!   `tests/timers.rs`, `tests/http.rs`, `tests/open.rs`;
//! - approval before instantiation: `tests/approval.rs`, `tests/manager.rs`;
//! - HTML and images in the markdown renderer itself: `crates/ui/tests/markdown.rs`;
//! - every SVG rule (scripts, `foreignObject`, `url(`, `@import`, element counts) and the image
//!   caps: `widget_caps::tests`, and end to end in `tests/widgets.rs`.

mod support;

use std::collections::BTreeMap;
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

use support::harness::*;
use tomakast_core::testing::FakeModule;
use tomakast_core::{CoreUpdate, Effect, Item, ItemId, ItemKind, ToastKind, View, ViewEvent};
use tomakast_plugin_host::widget_caps::check_svg;
use tomakast_ui::markdown::{self, Block};

/// The probe `action` (see the kitchen-sink crate doc) and the metadata it answers.
fn probe(h: &mut Harness, action: &str) -> BTreeMap<String, String> {
    let effects = h
        .run("kitchen-sink/indexed", action)
        .unwrap_or_else(|e| panic!("probe {action} failed: {e}"));
    metadata(&effects)
}

/// The host keeps running and so does the plugin: its command still answers.
fn assert_alive(h: &mut Harness) {
    let effects = h
        .run("kitchen-sink/hello", "run")
        .expect("the plugin still runs");
    assert!(
        matches!(&effects[..], [Effect::Toast(t)] if t.title == "Kitchen Sink: hello"),
        "{effects:?}"
    );
}

fn is_err(value: &str) -> bool {
    value.starts_with("Err(")
}

/// Ruling AJ: only a plugin holding the `search` permission is asked with what the user
/// types; without it the query never reaches the guest.
#[test]
fn search_needs_the_search_permission() {
    let mut h = harness_with(|p| p.manifest.permissions.search = false, BTreeMap::new());
    let answer = h.search("echo:typed");
    assert!(
        answer.as_ref().is_none_or(Vec::is_empty),
        "the plugin saw the query: {answer:?}"
    );
    // With the permission the same query is answered (the harness default).
    let mut h = harness();
    assert_eq!(h.search("echo:typed").expect("an answer").len(), 1);
}

#[test]
fn wasi_has_no_filesystem() {
    let mut h = harness();
    // There is something to read: the host itself can.
    assert!(std::fs::read("/etc/passwd").is_ok() || !cfg!(unix));
    let seen = probe(&mut h, "wasi-fs");
    for key in ["passwd", "dot", "root", "write"] {
        assert!(is_err(&seen[key]), "{key}: {}", seen[key]);
    }
    assert_alive(&mut h);
}

#[test]
fn wasi_has_no_env_or_args() {
    let mut h = harness();
    // There is something to leak: the test process has an environment and arguments.
    assert!(std::env::vars_os().next().is_some());
    assert!(std::env::args_os().next().is_some());
    let seen = probe(&mut h, "wasi-env");
    assert_eq!(seen["env-count"], "0", "{}", seen["env-names"]);
    assert_eq!(seen["args"], "[]");
    // No preopened directory either: the guest's working directory is the empty root.
    assert_eq!(seen["cwd"], "Ok(\"/\")");
    assert_alive(&mut h);
}

#[test]
fn wasi_has_no_sockets() {
    let mut h = harness();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let answer = h.run("kitchen-sink/indexed", &format!("wasi-tcp:{port}"));
    // Whatever the guest saw: nothing ever reached the listener, so its error is not merely
    // a closed port.
    let deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((_, peer)) => panic!("the plugin connected from {peer}"),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(e) => panic!("accept: {e}"),
        }
    }
    let seen = metadata(&answer.expect("the probe answers"));
    assert!(is_err(&seen["tcp"]), "tcp: {}", seen["tcp"]);
    assert!(is_err(&seen["lookup"]), "lookup: {}", seen["lookup"]);
    assert_alive(&mut h);
}

/// Set in the child run of [`wasi_stdout_is_not_inherited`]: the text the plugin writes.
const STDOUT_CHILD: &str = "TOMAKAST_THREAT_STDOUT_CANARY";
const CHILD_DONE: &str = "stdout probe done";

/// The guest writes a canary to stdout and stderr. This test binary is run again as a child
/// with its output captured, so a host that inherited stdio would show the canary there.
#[test]
fn wasi_stdout_is_not_inherited() {
    if let Ok(canary) = std::env::var(STDOUT_CHILD) {
        let mut h = harness();
        let seen = probe(&mut h, &format!("wasi-stdout:{canary}"));
        // The writes succeed into a sink, so a guest cannot tell; nothing reaches the host.
        assert_eq!(seen["stdout"], "Ok(())");
        assert_eq!(seen["stderr"], "Ok(())");
        assert_alive(&mut h);
        println!("{CHILD_DONE}");
        return;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let canary = format!("tk-leak-{}-{nanos}", std::process::id());
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "wasi_stdout_is_not_inherited",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(STDOUT_CHILD, &canary)
        .output()
        .expect("run the test binary as a child");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success() && stdout.contains(CHILD_DONE),
        "the child run failed:\n{stdout}\n{stderr}"
    );
    assert!(
        !stdout.contains(&canary),
        "the plugin's stdout reached the host"
    );
    assert!(
        !stderr.contains(&canary),
        "the plugin's stderr reached the host"
    );
}

#[test]
fn plugin_file_icon_cannot_escape_folder() {
    // Paths that leave the folder by name reject the whole answer, with a denial toast. A
    // fresh plugin each, because the denial toast is shown at most once per 10 s.
    for path in ["../x.png", "icons/../../x.png", "/etc/passwd"] {
        let mut h = harness();
        let answer = h.search(&format!("icon:{path}"));
        assert!(
            answer.as_ref().is_none_or(|items| items.is_empty()),
            "{path}: {answer:?}"
        );
        let toast = h.toast();
        assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
        let message = toast.message.unwrap_or_default();
        assert!(
            message.contains("inside the plugin folder"),
            "{path}: {message}"
        );
        assert_alive(&mut h);
    }
    // A symbolic link inside the folder to a real file outside it, or through a linked
    // folder, passes the name check and is caught after canonicalization: the item stays,
    // without an icon.
    #[cfg(unix)]
    {
        let mut h = harness();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("x.png"), b"\x89PNG").unwrap();
        let icons = loaded().2.join("icons");
        let file_link = icons.join("escape.png");
        let dir_link = icons.join("escape-dir");
        let _ = std::fs::remove_file(&file_link);
        let _ = std::fs::remove_file(&dir_link);
        std::os::unix::fs::symlink(outside.path().join("x.png"), &file_link).unwrap();
        std::os::unix::fs::symlink(outside.path(), &dir_link).unwrap();
        for path in ["icons/escape.png", "icons/escape-dir/x.png"] {
            let items = h.search(&format!("icon:{path}")).expect("an answer");
            assert_eq!(items.len(), 1, "{path}: the item is kept");
            assert_eq!(items[0].icon, None, "{path}");
        }
        std::fs::remove_file(&file_link).unwrap();
        std::fs::remove_file(&dir_link).unwrap();
        assert_alive(&mut h);
    }
}

#[test]
fn markdown_from_plugin_has_no_html_and_no_remote_images() {
    let mut h = harness();
    let effects = h.run("kitchen-sink/indexed", "markdown").unwrap();
    let [Effect::PushView(View::Detail(view))] = &effects[..] else {
        panic!("expected one detail view, got {effects:?}");
    };
    // The host passes the plugin's text on as data; the launcher's renderer makes it inert.
    assert!(view.markdown.contains("<script>alert(1)</script>"));
    let blocks = markdown::parse(&view.markdown);
    let spans: Vec<_> = blocks
        .iter()
        .flat_map(|b| match b {
            Block::Paragraph(spans) | Block::Heading(_, spans) => spans.clone(),
            other => panic!("unexpected block {other:?}"),
        })
        .collect();
    // No span links anywhere: an image or an HTML attribute never becomes a target.
    assert!(spans.iter().all(|s| s.link.is_none()), "{spans:?}");
    let text = markdown::plain_text(&view.markdown);
    for literal in [
        "<script>alert(1)</script>",
        "<img src=\"https://tracker.example/tag.png\">",
        "<iframe src=\"https://evil.example/\"></iframe>",
        "<b>bold</b>",
        // The image is shown as its alt text and URL, never fetched.
        "pixel (https://tracker.example/pixel.png)",
    ] {
        assert!(
            text.contains(literal),
            "{literal:?} not shown as text in {text:?}"
        );
    }
    assert_alive(&mut h);
}

/// An SVG the plugin sends is checked before it is stored: an `<image>` that names a host file
/// (`/etc/hostname`) refuses the whole frame, so no renderer ever gets a path to resolve.
#[test]
fn svg_files_are_never_read() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><image href="/etc/hostname" width="8" height="8"/></svg>"#;
    let err = check_svg(svg).unwrap_err();
    assert!(err.to_string().contains("/etc/hostname"), "{err}");

    let mut h = harness();
    h.home_widgets(&["probe"]);
    h.run("kitchen-sink/indexed", "render:bad-svg:href")
        .unwrap();
    assert_eq!(h.render("probe"), None, "the refused frame is not stored");
    assert!(h.core.widget_image(ID, "bad-svg").is_none());
    assert!(
        h.plugin_events.try_recv().is_err(),
        "the plugin is not broken"
    );
    assert_alive(&mut h);
}

#[test]
fn plugin_cannot_set_item_source_or_kind() {
    // The WIT item has no field for either.
    let wit = std::fs::read_to_string(
        support::workspace_root().join("crates/plugin-api/wit/since_v0.1.0/plugin.wit"),
    )
    .unwrap();
    let start = wit.find("record item {").expect("record item");
    let record = &wit[start..start + wit[start..].find('}').unwrap()];
    let fields: Vec<&str> = record
        .lines()
        .skip(1)
        .filter_map(|l| l.trim().split(':').next())
        .filter(|f| !f.is_empty())
        .collect();
    assert_eq!(
        fields,
        ["id", "title", "subtitle", "keywords", "icon", "actions"]
    );

    // Every way a plugin hands in items gets the host's kind and source: init and search are
    // covered in tests/plugin.rs; here a user-started replace-items and an unsolicited one.
    let mut h = harness();
    h.run("kitchen-sink/indexed", "replace").unwrap();
    let replaced = h.indexed("Replaced").expect("replaced item");
    assert_owned(&replaced);
    let effects = h
        // `replace` took the init item away; the replaced one runs the action instead. The
        // timer fires after init's unsolicited replace-items has left the 1-per-5 s window.
        .run("kitchen-sink/replaced", "timer-reply:5100:replace-many:2")
        .unwrap();
    assert_eq!(metadata(&effects)["result"], "Ok(1)");
    let deadline = Instant::now() + Duration::from_secs(8);
    let item = loop {
        h.settle(Duration::from_millis(20));
        // Both new items are titled "Replaced"; look for one by id.
        let hits = h.core.search("Replaced", now()).hits;
        if let Some(hit) = hits
            .into_iter()
            .find(|hit| hit.item.id.as_str() == "kitchen-sink/r0")
        {
            break hit.item;
        }
        assert!(
            Instant::now() < deadline,
            "the timer's replace-items never arrived: {:?}",
            h.unsolicited
        );
    };
    assert_owned(&item);
    assert_alive(&mut h);
}

fn assert_owned(item: &Item) {
    assert_eq!(item.kind, ItemKind::Command, "{item:?}");
    assert_eq!(item.source.as_deref(), Some(NAME), "{item:?}");
}

#[test]
fn effects_with_control_chars_in_ids_rejected() {
    let mut h = harness();
    // A user-started call answers the rejection as its error (Ruling W), every time.
    for target in ["view", "row", "action", "field", "tile", "item"] {
        let err = h
            .run("kitchen-sink/indexed", &format!("bad-ids:{target}"))
            .expect_err(target);
        assert!(
            err.contains(&format!("{target} id")) && err.contains("control or format"),
            "{target}: {err}"
        );
        // Shown on one line, escaped: the control character never reaches the toast.
        assert!(!err.chars().any(char::is_control), "{target}: {err:?}");
    }
    assert!(h.indexed("Bad id").is_none());
    // An on-event answer is rejected as a whole with a denial toast.
    let effects = h
        .view_event("bad-ids", ViewEvent::Closed)
        .expect("a rejected on-event answer is no call error");
    assert!(effects.is_empty(), "{effects:?}");
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    assert!(
        toast.message.unwrap_or_default().contains("row id"),
        "the denial names the field"
    );
    assert_alive(&mut h);
}

/// A native module keeps answering within 20 ms while the plugin spins until its 100 ms
/// `search` deadline: each module has its own worker.
#[test]
fn a_hung_plugin_does_not_delay_other_modules() {
    let answer = Item {
        id: ItemId::parse("fake/answer").unwrap(),
        title: "Native answer".into(),
        subtitle: None,
        keywords: Vec::new(),
        icon: None,
        kind: ItemKind::Command,
        actions: vec![tomakast_core::Action {
            id: "run".into(),
            title: "Run".into(),
            shortcut: None,
        }],
        source: None,
    };
    let fake = FakeModule::new(Vec::new(), || Ok(vec![])).with_search(vec![answer]);
    let mut h = harness_beside(|_| {}, BTreeMap::new(), |p| p, vec![Box::new(fake)]);
    let native_answer = |h: &mut Harness, query: &str| {
        let start = Instant::now();
        let generation = h.core.search(query, now()).generation;
        loop {
            let ev = tomakast_core::testing::wait_event(&h.rx);
            if let CoreUpdate::Results(snapshot) = h.apply(ev)
                && snapshot.generation == generation
                && snapshot
                    .hits
                    .iter()
                    .any(|hit| hit.item.id.as_str() == "fake/answer")
            {
                return start.elapsed();
            }
        }
    };
    let elapsed = native_answer(&mut h, "spin");
    assert!(
        elapsed < Duration::from_millis(20),
        "the native answer took {elapsed:?} while the plugin spun"
    );
    // Only the plugin stops, at its own deadline.
    let toast = h.toast();
    assert_eq!(toast.kind, ToastKind::Error, "{toast:?}");
    assert!(
        toast
            .message
            .unwrap_or_default()
            .contains("search did not answer within 100 ms")
    );
    h.broken_event();
    // The core and the native module carry on.
    let elapsed = native_answer(&mut h, "after");
    assert!(elapsed < Duration::from_millis(20), "{elapsed:?}");
}
