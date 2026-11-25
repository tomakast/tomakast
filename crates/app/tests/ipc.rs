use std::sync::mpsc;
use std::time::Duration;

use tomakast_app::ipc::{Request, ServeError, SocketLocation, resolve_socket, send_at, serve_at};
use tomakast_core::ItemId;

fn unique_name() -> String {
    format!("tomakast-test-{}", uuid::Uuid::new_v4())
}

/// A unique socket location under a private temp dir (no process-environment mutation).
fn location(dir: &tempfile::TempDir) -> SocketLocation {
    let home = dir.path().to_path_buf();
    resolve_socket(&unique_name(), |k| {
        (k == "TOMAKAST_HOME").then(|| home.clone().into_os_string())
    })
    .unwrap()
}

#[test]
fn request_roundtrip() {
    let id = ItemId::parse("apps/x.desktop").unwrap();
    for req in [Request::Toggle, Request::Ping, Request::Run(id)] {
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);
    }
    assert_eq!(Request::Toggle.encode(), "toggle");
    assert_eq!(Request::Ping.encode(), "ping");
    assert_eq!(
        Request::Run(ItemId::parse("apps/x.desktop").unwrap()).encode(),
        "run apps/x.desktop"
    );
}

#[test]
fn settings_request_round_trips() {
    for req in [
        Request::Settings(None),
        Request::Settings(Some("plugin:weather".into())),
    ] {
        assert_eq!(Request::decode(&req.encode()).unwrap(), req);
    }
    assert_eq!(Request::Settings(None).encode(), "settings");
    assert_eq!(
        Request::decode("settings ").unwrap(),
        Request::Settings(None)
    );
    assert_eq!(
        Request::Settings(Some("plugin:weather".into())).encode(),
        "settings plugin:weather"
    );
}

#[test]
fn decode_rejects_garbage() {
    for line in ["", "run", "run nosl", "jump x"] {
        assert!(
            Request::decode(line).is_err(),
            "{line:?} should be rejected"
        );
    }
}

#[test]
fn send_reaches_handler() {
    let dir = tempfile::tempdir().unwrap();
    let loc = location(&dir);
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let _server = serve_at(loc.clone(), move |r| tx.lock().unwrap().send(r).unwrap()).unwrap();
    send_at(&loc, &Request::Toggle).unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Request::Toggle
    );
    let id = ItemId::parse("apps/y.desktop").unwrap();
    send_at(&loc, &Request::Run(id.clone())).unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Request::Run(id)
    );
}

#[test]
fn ping_does_not_reach_handler() {
    let dir = tempfile::tempdir().unwrap();
    let loc = location(&dir);
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let _server = serve_at(loc.clone(), move |r| tx.lock().unwrap().send(r).unwrap()).unwrap();
    send_at(&loc, &Request::Ping).unwrap();
    assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
}

#[test]
fn second_server_reports_already_running() {
    let dir = tempfile::tempdir().unwrap();
    let loc = location(&dir);
    let _first = serve_at(loc.clone(), |_| {}).unwrap();
    assert!(matches!(
        serve_at(loc.clone(), |_| {}),
        Err(ServeError::AlreadyRunning)
    ));
}

#[test]
fn send_without_server_errors() {
    let dir = tempfile::tempdir().unwrap();
    let loc = location(&dir);
    assert!(send_at(&loc, &Request::Toggle).is_err());
}

#[cfg(unix)]
#[test]
fn stale_socket_file_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let loc = location(&dir);
    let SocketLocation::File(path) = &loc else {
        unreachable!()
    };
    std::fs::write(path, b"").unwrap();
    let _server = serve_at(loc.clone(), |_| {}).unwrap();
    send_at(&loc, &Request::Ping).unwrap();
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn resolve_prefers_tomakast_home_over_xdg_runtime_dir() {
    use std::ffi::OsString;
    let env = |k: &str| match k {
        "TOMAKAST_HOME" => Some(OsString::from("/h")),
        "XDG_RUNTIME_DIR" => Some(OsString::from("/run/u")),
        _ => None,
    };
    assert_eq!(
        resolve_socket("s", env).unwrap(),
        SocketLocation::File("/h/s".into())
    );
    let env = |k: &str| (k == "XDG_RUNTIME_DIR").then(|| OsString::from("/run/u"));
    assert_eq!(
        resolve_socket("s", env).unwrap(),
        SocketLocation::File("/run/u/s".into())
    );
    assert!(resolve_socket("s", |_| None).is_err());
}

#[cfg(unix)]
#[test]
fn stalled_client_does_not_wedge_server() {
    use interprocess::local_socket::{GenericFilePath, Stream, ToFsName, traits::Stream as _};
    let dir = tempfile::tempdir().unwrap();
    let loc = location(&dir);
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let _server = serve_at(loc.clone(), move |r| tx.lock().unwrap().send(r).unwrap()).unwrap();
    let SocketLocation::File(path) = &loc else {
        unreachable!()
    };
    // Connects and never sends a line; stays open for the whole test.
    let _stalled =
        Stream::connect(path.as_path().to_fs_name::<GenericFilePath>().unwrap()).unwrap();
    let started = std::time::Instant::now();
    send_at(&loc, &Request::Toggle).unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Request::Toggle
    );
}

#[test]
fn panicking_handler_does_not_kill_server() {
    let dir = tempfile::tempdir().unwrap();
    let loc = location(&dir);
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let _server = serve_at(loc.clone(), move |r| {
        if r == Request::Toggle {
            panic!("boom");
        }
        tx.lock().unwrap().send(r).unwrap()
    })
    .unwrap();
    assert!(send_at(&loc, &Request::Toggle).is_err());
    let id = ItemId::parse("apps/z.desktop").unwrap();
    send_at(&loc, &Request::Run(id.clone())).unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        Request::Run(id)
    );
}
