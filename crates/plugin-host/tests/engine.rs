//! The engine, the compile child and the precompiled cache.

mod support;

use std::fs;
use std::process::Command;

use support::{compile_exe, guest, sha256};
use tomakast_plugin_host::compile::child_args;
use tomakast_plugin_host::{Cache, HostEngine, Package, compile_in_child, engine_config};
use wasmtime::Engine;
use wasmtime::component::Component;

fn kitchen_sink() -> (Vec<u8>, [u8; 32], std::path::PathBuf) {
    let wasm_path = guest("kitchen-sink");
    let wasm = fs::read(&wasm_path).unwrap();
    let hash = sha256(&wasm);
    (wasm, hash, wasm_path.parent().unwrap().to_owned())
}

/// Names in `dir` that end in `.tmp`.
fn temp_files(dir: &std::path::Path) -> Vec<String> {
    entries(dir)
        .into_iter()
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(dir)
        .map(|rd| {
            rd.map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn cache_entry_is_written_by_child_and_deserialized() {
    let (wasm, hash, plugin_dir) = kitchen_sink();
    // The harness folder is a real plugin package: named by id, with its manifest.
    let package = Package::read(&plugin_dir).unwrap();
    assert_eq!(package.manifest.id, "kitchen-sink");
    assert_eq!(package.wasm_hash, hash);
    let data = tempfile::tempdir().unwrap();
    let cache = Cache::new(data.path().join("cache/plugins"));
    let engine = HostEngine::new().unwrap();

    let entry = cache.path_for(&hash, engine.engine());
    assert!(!entry.exists());
    let component = engine.load(hash, &wasm, &cache, &compile_exe()).unwrap();
    assert!(component.component_type().exports(engine.engine()).count() > 0);

    // Exactly the entry is left behind: no temp input, no `.tmp` output.
    let name = entry.file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(entries(cache.dir()), vec![name]);

    // A second load is a hit: the entry is deserialized as it is, not rewritten.
    let modified = fs::metadata(&entry).unwrap().modified().unwrap();
    let bogus_exe = data.path().join("no-such-exe");
    engine.load(hash, &wasm, &cache, &bogus_exe).unwrap();
    assert_eq!(fs::metadata(&entry).unwrap().modified().unwrap(), modified);
}

#[test]
fn child_refuses_hash_mismatch() {
    let (wasm, hash, _) = kitchen_sink();
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("input.wasm");
    fs::write(&input, &wasm).unwrap();
    let out = dir.path().join("out.cwasm");
    let mut wrong = hash;
    wrong[0] ^= 1;

    // The child itself refuses a valid component whose hash does not match.
    let status = Command::new(compile_exe())
        .args(child_args(&input, &wrong, &out))
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success());
    assert!(!out.exists());
    assert_eq!(temp_files(dir.path()), Vec::<String>::new());

    // Through the daemon side the error carries the child's message.
    let err = compile_in_child(&compile_exe(), &wasm, wrong, &out).unwrap_err();
    assert!(format!("{err:#}").contains("sha256"), "{err:#}");
    assert!(!out.exists());
    assert_eq!(temp_files(dir.path()), Vec::<String>::new());

    // The same input with the right hash compiles.
    compile_in_child(&compile_exe(), &wasm, hash, &out).unwrap();
    assert!(out.is_file());
}

#[test]
fn concurrent_loads_of_one_plugin_all_succeed() {
    let (wasm, hash, _) = kitchen_sink();
    let data = tempfile::tempdir().unwrap();
    let cache = Cache::new(data.path().to_owned());
    let engine = HostEngine::new().unwrap();
    let start = std::sync::Barrier::new(4);
    std::thread::scope(|scope| {
        let loads: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    start.wait();
                    engine.load(hash, &wasm, &cache, &compile_exe())
                })
            })
            .collect();
        for load in loads {
            load.join().unwrap().unwrap();
        }
    });
    let entry = cache.path_for(&hash, engine.engine());
    let name = entry.file_name().unwrap().to_str().unwrap().to_owned();
    assert_eq!(entries(cache.dir()), vec![name]);
}

/// A cache hit takes no lock: it does not wait for another plugin's slow compile.
#[cfg(unix)]
#[test]
fn cache_hit_does_not_wait_for_another_compile() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let (wasm, hash, _) = kitchen_sink();
    let data = tempfile::tempdir().unwrap();
    let cache = Cache::new(data.path().join("cache"));
    let engine = HostEngine::new().unwrap();
    engine.load(hash, &wasm, &cache, &compile_exe()).unwrap();

    // Another plugin whose compile child takes 1.5 s (and then fails).
    let slow = data.path().join("slow-child");
    fs::write(&slow, "#!/bin/sh\nsleep 1.5\nexit 1\n").unwrap();
    fs::set_permissions(&slow, fs::Permissions::from_mode(0o755)).unwrap();
    let other: &[u8] = b"\0asm\x0d\x00\x01\x00";

    std::thread::scope(|scope| {
        let compiling = scope.spawn(|| engine.load(sha256(other), other, &cache, &slow));
        std::thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
        engine.load(hash, &wasm, &cache, &compile_exe()).unwrap();
        let hit = start.elapsed();
        assert!(hit < Duration::from_millis(500), "cache hit took {hit:?}");
        assert!(compiling.join().unwrap().is_err());
    });
}

#[test]
fn load_refuses_bytes_that_do_not_match_the_hash() {
    let (wasm, hash, _) = kitchen_sink();
    let data = tempfile::tempdir().unwrap();
    let cache = Cache::new(data.path().to_owned());
    let engine = HostEngine::new().unwrap();
    engine.load(hash, &wasm, &cache, &compile_exe()).unwrap();

    // A cache hit for `hash` must not serve other bytes.
    let mut other = wasm.clone();
    other.push(0);
    assert!(engine.load(hash, &other, &cache, &compile_exe()).is_err());
}

#[test]
fn load_never_reads_outside_cache_dir() {
    let (wasm, hash, plugin_dir) = kitchen_sink();
    let data = tempfile::tempdir().unwrap();
    let cache = Cache::new(data.path().join("cache"));
    let engine = HostEngine::new().unwrap();
    let entry = cache.path_for(&hash, engine.engine());
    let entry_name = entry.file_name().unwrap();

    // Decoys in the plugin folder, under every name a loader might look for. The one named
    // like the cache entry is a valid precompiled component, so following it would work.
    let garbage = b"not a precompiled component";
    let valid = Engine::new(&engine_config())
        .unwrap()
        .precompile_component(&wasm)
        .unwrap();
    for name in ["plugin.cwasm", "plugin.wasm.cwasm"] {
        fs::write(plugin_dir.join(name), garbage).unwrap();
    }
    fs::write(plugin_dir.join(entry_name), &valid).unwrap();

    // A cache entry that is a symlink into the plugin folder is not followed.
    fs::create_dir_all(cache.dir()).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(plugin_dir.join(entry_name), &entry).unwrap();

    let component = engine.load(hash, &wasm, &cache, &compile_exe()).unwrap();
    assert!(component.component_type().exports(engine.engine()).count() > 0);

    // The child compiled the source wasm into a regular file in the cache dir; the decoys
    // are untouched.
    let meta = fs::symlink_metadata(&entry).unwrap();
    assert!(meta.file_type().is_file(), "entry is a regular file");
    for name in ["plugin.cwasm", "plugin.wasm.cwasm"] {
        assert_eq!(fs::read(plugin_dir.join(name)).unwrap(), garbage);
    }
    assert_eq!(fs::read(plugin_dir.join(entry_name)).unwrap(), valid);
}

#[test]
fn corrupt_cache_entry_is_recompiled_once() {
    let (wasm, hash, _) = kitchen_sink();
    let data = tempfile::tempdir().unwrap();
    let cache = Cache::new(data.path().to_owned());
    let engine = HostEngine::new().unwrap();
    let entry = cache.path_for(&hash, engine.engine());
    fs::write(&entry, b"garbage").unwrap();

    engine.load(hash, &wasm, &cache, &compile_exe()).unwrap();
    assert_ne!(fs::read(&entry).unwrap(), b"garbage");

    // When the child cannot fix it either, load fails instead of retrying.
    fs::write(&entry, b"garbage").unwrap();
    let bogus_exe = data.path().join("no-such-exe");
    assert!(engine.load(hash, &wasm, &cache, &bogus_exe).is_err());
}

#[test]
fn cache_key_changes_with_engine_config() {
    let (wasm, hash, _) = kitchen_sink();
    let data = tempfile::tempdir().unwrap();
    let cache = Cache::new(data.path().to_owned());
    let host = HostEngine::new().unwrap();
    host.load(hash, &wasm, &cache, &compile_exe()).unwrap();
    let entry = cache.path_for(&hash, host.engine());
    assert!(entry.is_file());

    // Same config: same key.
    let same = Engine::new(&engine_config()).unwrap();
    assert_eq!(cache.path_for(&hash, &same), entry);

    // Another config: another key, so the existing entry is a miss for it, and indeed it
    // would not load there.
    let mut config = engine_config();
    config.epoch_interruption(false);
    let other = Engine::new(&config).unwrap();
    let other_entry = cache.path_for(&hash, &other);
    assert_ne!(other_entry, entry);
    assert!(!other_entry.exists());
    assert!(unsafe { Component::deserialize_file(&other, &entry) }.is_err());
}
