//! Deadlines through the epoch ticker. Its own test binary: timing must not compete with the
//! compile children of other tests, which use every core.

mod support;

use std::time::{Duration, Instant};

use support::{compile_exe, guest, instantiate, search, sha256, store};
use tomakast_plugin_host::{Cache, Epoch, HostEngine};
use wasmtime::Trap;

#[test]
fn deadline_counts_from_call_start_after_idle() {
    let wasm = std::fs::read(guest("kitchen-sink")).unwrap();
    let data = tempfile::tempdir().unwrap();
    let host = HostEngine::new().unwrap();
    let component = host
        .load(
            sha256(&wasm),
            &wasm,
            &Cache::new(data.path().to_owned()),
            &compile_exe(),
        )
        .unwrap();

    // A private ticker, so that nothing else keeps it running.
    let epoch = Epoch::new();
    epoch.register(host.engine());
    let mut store = store(host.engine());
    let instance = instantiate(host.engine(), &mut store, &component);

    // One short call with a long (2 s) deadline lets the ticker run, then it parks for a
    // second. Were the next call not armed, that stale 2 s deadline would still apply.
    {
        let _guard = epoch.enter_store(&mut store, Duration::from_secs(2));
        search(&mut store, &instance, "noop").unwrap();
    }
    std::thread::sleep(Duration::from_secs(1));

    let guard = epoch.enter_store(&mut store, Duration::from_millis(100));
    let start = Instant::now();
    let err = search(&mut store, &instance, "spin").unwrap_err();
    let elapsed = start.elapsed();
    drop(guard);
    eprintln!("spin trapped after {elapsed:?}");

    assert_eq!(
        err.downcast_ref::<Trap>(),
        Some(&Trap::Interrupt),
        "{err:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(100) && elapsed <= Duration::from_millis(130),
        "trapped after {elapsed:?}"
    );
}
