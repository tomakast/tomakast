//! Log output of `scan`. Kept in its own test binary: `tracing` caches
//! per-callsite interest process-wide, so scans on other test threads
//! (without a subscriber) could hide these events from the scoped one.
#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tomakast_module_apps::{ScanOptions, scan};

/// Collects formatted log output in memory.
#[derive(Clone, Default)]
struct LogBuffer(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for LogBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn scan_warns_about_each_broken_entry() {
    let opts = ScanOptions {
        dirs: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/broken")],
        locales: vec![],
        current_desktops: vec!["KDE".to_string()],
        path_env: Some(OsString::new()),
        icon_theme: "hicolor".to_string(),
    };
    let log = LogBuffer::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || scan(&opts));
    let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    let warned: Vec<&str> = text.lines().filter(|l| l.contains(" WARN ")).collect();
    for file in [
        "invalid-utf8.desktop",
        "noname.desktop",
        "noexec.desktop",
        "quote.desktop",
    ] {
        assert!(
            warned.iter().any(|l| l.contains(file)),
            "no warning for {file} in:\n{text}"
        );
    }
    assert!(
        !text.contains("good.desktop"),
        "good entry was warned about:\n{text}"
    );
}
