//! Loading a cold component must not bring Cranelift into the daemon. Its own test binary:
//! RSS is per process, and other tests would allocate concurrently. Linux only (`VmRSS`).
#![cfg(target_os = "linux")]

mod support;

use support::{compile_exe, guest, sha256};
use tomakast_plugin_host::{Cache, HostEngine};

/// `VmRSS` of this process in KiB.
fn rss_kib() -> u64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap();
    let line = status.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
    line.split_whitespace().nth(1).unwrap().parse().unwrap()
}

#[test]
fn daemon_rss_does_not_include_cranelift() {
    let wasm = std::fs::read(guest("kitchen-sink")).unwrap();
    let hash = sha256(&wasm);
    let data = tempfile::tempdir().unwrap();
    let cache = Cache::new(data.path().to_owned());
    let host = HostEngine::new().unwrap();

    let before = rss_kib();
    let component = host.load(hash, &wasm, &cache, &compile_exe()).unwrap();
    let after = rss_kib();
    let grown = after.saturating_sub(before);
    eprintln!("RSS before load {before} KiB, after {after} KiB (+{grown} KiB)");
    assert!(
        grown < 8 * 1024,
        "load grew RSS by {grown} KiB: compiled in-process?"
    );
    drop(component);

    // The control: compiling in this process does show up, so the bound above means something.
    let _compiled = wasmtime::component::Component::new(host.engine(), &wasm).unwrap();
    let compiled = rss_kib().saturating_sub(after);
    eprintln!("in-process Component::new: +{compiled} KiB");
    assert!(
        compiled >= 8 * 1024,
        "in-process compile grew RSS by only {compiled} KiB"
    );
}
