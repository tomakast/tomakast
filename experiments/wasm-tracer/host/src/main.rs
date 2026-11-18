//! Prints every measurement of the WASM tracer bullet. See `docs/experiments/2026-11-wasm-tracer.md`.
//!
//! Run: `cargo run --release --manifest-path experiments/wasm-tracer/host/Cargo.toml`
//! (after building the guest for `wasm32-wasip2`).

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tracer_host::{
    Plugin, PluginOptions, RECURSIVE_WIT, SearchItem, Ticker, guest_bytes, make_engine,
    make_linker, median_p95, parse_wit, rss_kib,
};
use wasmtime::Result;
use wasmtime::component::Component;

const CALLS: usize = 1000;
const WARMUP: usize = 50;
const COMPILE_RUNS: usize = 5;
const SIZES: [usize; 3] = [1, 10, 100];

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("rss-child") {
        let instances: usize = args[3].parse()?;
        let load = match args[2].as_str() {
            "new" => Load::Compile,
            path => Load::Deserialize(PathBuf::from(path)),
        };
        return rss_child(&load, instances);
    }

    let bytes = guest_bytes()?;
    println!("guest component: {} bytes", bytes.len());

    compile_times(&bytes)?;
    call_latency(&bytes)?;
    worker_thread_latency(&bytes)?;
    rss(&bytes)?;
    epoch_interrupt(&bytes)?;
    memory_limit(&bytes)?;
    recursion();
    Ok(())
}

fn ms(d: Duration) -> String {
    format!("{:.3} ms", d.as_secs_f64() * 1e3)
}

fn us(d: Duration) -> String {
    format!("{:.1} µs", d.as_secs_f64() * 1e6)
}

fn summary(label: &str, samples: &mut [Duration], unit: fn(Duration) -> String) {
    let min = *samples.iter().min().expect("samples");
    let max = *samples.iter().max().expect("samples");
    let (median, p95) = median_p95(samples);
    println!(
        "  {label:<44} median {:>11}  p95 {:>11}  min {:>11}  max {:>11}  (n={})",
        unit(median),
        unit(p95),
        unit(min),
        unit(max),
        samples.len()
    );
}

/// Cold `Component::new` vs `precompile_component` + `Component::deserialize`, each on a fresh engine.
fn compile_times(bytes: &[u8]) -> Result<()> {
    println!("\n== compile (fresh Engine per run, cache off) ==");
    for epoch in [false, true] {
        let tag = if epoch { "epoch on" } else { "epoch off" };
        let mut cold = Vec::new();
        let mut precompile = Vec::new();
        let mut deserialize = Vec::new();
        let mut cwasm_len = 0;
        for _ in 0..COMPILE_RUNS {
            let engine = make_engine(epoch);
            let start = Instant::now();
            Component::new(&engine, bytes)?;
            cold.push(start.elapsed());

            let engine = make_engine(epoch);
            let start = Instant::now();
            let cwasm = engine.precompile_component(bytes)?;
            precompile.push(start.elapsed());
            cwasm_len = cwasm.len();

            let engine = make_engine(epoch);
            let start = Instant::now();
            // SAFETY: the bytes were produced just above by `precompile_component` with the same config.
            unsafe { Component::deserialize(&engine, &cwasm)? };
            deserialize.push(start.elapsed());
        }
        summary(&format!("Component::new ({tag})"), &mut cold, ms);
        summary(
            &format!("Engine::precompile_component ({tag})"),
            &mut precompile,
            ms,
        );
        summary(
            &format!("Component::deserialize ({tag})"),
            &mut deserialize,
            ms,
        );
        println!("  precompiled artifact ({tag}): {cwasm_len} bytes");
    }
    Ok(())
}

fn query(n: usize) -> String {
    format!("{n} firefox")
}

fn measure_search(plugin: &mut Plugin, n: usize) -> Result<Vec<Duration>> {
    let q = query(n);
    for _ in 0..WARMUP {
        plugin.search(&q)?;
    }
    let mut samples = Vec::with_capacity(CALLS);
    for _ in 0..CALLS {
        let start = Instant::now();
        let items = plugin.search(&q)?;
        samples.push(start.elapsed());
        assert_eq!(items.len(), n);
    }
    Ok(samples)
}

/// `search` latency with one store and one reused instance, with and without epoch interruption.
fn call_latency(bytes: &[u8]) -> Result<()> {
    println!("\n== search() latency, same thread, one Store + reused instance ==");
    for epoch in [false, true] {
        let engine = make_engine(epoch);
        let component = Component::new(&engine, bytes)?;
        let linker = make_linker(&engine);
        let _ticker = epoch.then(|| Ticker::start(&engine, Duration::from_millis(1)));
        let options = PluginOptions {
            epoch_deadline_ticks: epoch.then_some(100),
            ..PluginOptions::default()
        };

        let mut instantiate = Vec::new();
        for _ in 0..20 {
            let start = Instant::now();
            Plugin::new(&engine, &component, &linker, options)?;
            instantiate.push(start.elapsed());
        }
        let tag = if epoch {
            "epoch on, 1 ms tick"
        } else {
            "epoch off"
        };
        summary(
            &format!("new Store + instantiate ({tag})"),
            &mut instantiate,
            us,
        );

        let mut plugin = Plugin::new(&engine, &component, &linker, options)?;
        for n in SIZES {
            let mut samples = measure_search(&mut plugin, n)?;
            summary(&format!("search -> {n} items ({tag})"), &mut samples, us);
        }
    }
    Ok(())
}

/// Round trip of `search` through a worker thread that owns the store (the proposed design).
fn worker_thread_latency(bytes: &[u8]) -> Result<()> {
    println!("\n== search() round trip via a worker thread that owns the Store (epoch on) ==");
    let engine = make_engine(true);
    let component = Component::new(&engine, bytes)?;
    let linker = make_linker(&engine);
    let _ticker = Ticker::start(&engine, Duration::from_millis(1));

    let (request_tx, request_rx) = mpsc::channel::<String>();
    let (reply_tx, reply_rx) = mpsc::channel::<Vec<SearchItem>>();
    let worker = {
        let (engine, component, linker) = (engine.clone(), component.clone(), linker.clone());
        std::thread::spawn(move || -> Result<()> {
            let options = PluginOptions {
                epoch_deadline_ticks: Some(100),
                ..PluginOptions::default()
            };
            let mut plugin = Plugin::new(&engine, &component, &linker, options)?;
            for q in request_rx {
                reply_tx
                    .send(plugin.search(&q)?)
                    .expect("reply channel open");
            }
            Ok(())
        })
    };

    for n in SIZES {
        let q = query(n);
        for _ in 0..WARMUP {
            request_tx.send(q.clone()).expect("worker alive");
            reply_rx.recv().expect("worker replies");
        }
        let mut samples = Vec::with_capacity(CALLS);
        for _ in 0..CALLS {
            let start = Instant::now();
            request_tx.send(q.clone()).expect("worker alive");
            let items = reply_rx.recv().expect("worker replies");
            samples.push(start.elapsed());
            assert_eq!(items.len(), n);
        }
        summary(&format!("round trip -> {n} items"), &mut samples, us);
    }
    drop(request_tx);
    worker.join().expect("worker did not panic")?;
    Ok(())
}

fn cwasm_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/tracer_guest.epoch.cwasm")
}

/// How the RSS child gets its component.
enum Load {
    Deserialize(PathBuf),
    Compile,
}

/// RSS of a clean child process that loads the component, then instantiates N plugins.
fn rss(bytes: &[u8]) -> Result<()> {
    println!("\n== RSS (clean child process per line, epoch on) ==");
    let cwasm = make_engine(true).precompile_component(bytes)?;
    let path = cwasm_path();
    std::fs::write(&path, cwasm)?;
    let exe = std::env::current_exe()?;
    let deserialize = path.to_str().expect("UTF-8 target path").to_string();
    for (load, instances) in [
        (&deserialize, 1),
        (&deserialize, 10),
        (&"new".to_string(), 1),
    ] {
        let output = Command::new(&exe)
            .arg("rss-child")
            .arg(load)
            .arg(instances.to_string())
            .output()?;
        if !output.status.success() {
            wasmtime::bail!(
                "rss child failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        print!("{}", String::from_utf8_lossy(&output.stdout));
    }
    Ok(())
}

fn rss_child(load: &Load, instances: usize) -> Result<()> {
    let start = rss_kib().unwrap_or(0);
    let engine = make_engine(true);
    let (component, how) = match load {
        Load::Deserialize(path) => {
            let bytes = std::fs::read(path)?;
            // SAFETY: the parent wrote this file with `precompile_component` and the same config.
            (
                unsafe { Component::deserialize(&engine, &bytes)? },
                "deserialize",
            )
        }
        Load::Compile => (Component::new(&engine, guest_bytes()?)?, "Component::new"),
    };
    let linker = make_linker(&engine);
    let options = PluginOptions {
        epoch_deadline_ticks: Some(u64::MAX / 2),
        ..PluginOptions::default()
    };
    let loaded = rss_kib().unwrap_or(0);
    let mut plugins = Vec::with_capacity(instances);
    for _ in 0..instances {
        let mut plugin = Plugin::new(&engine, &component, &linker, options)?;
        // Commit the instance's memory with one real call.
        plugin.search(&query(100))?;
        plugins.push(plugin);
    }
    let after = rss_kib().unwrap_or(0);
    println!(
        "  {how:<14} {instances:>2} instance(s): process start {start} KiB, after engine+load {loaded} KiB, \
         after instances {after} KiB -> instances delta {} KiB ({} KiB per instance)",
        after - loaded,
        (after - loaded) / instances as u64
    );
    Ok(())
}

/// How fast epoch interruption stops `spin`, for two tick/deadline pairs.
fn epoch_interrupt(bytes: &[u8]) -> Result<()> {
    println!("\n== epoch interruption of spin() (fresh Store per run) ==");
    let engine = make_engine(true);
    let component = Component::new(&engine, bytes)?;
    let linker = make_linker(&engine);
    for (tick_ms, ticks) in [(1u64, 50u64), (10, 10)] {
        let _ticker = Ticker::start(&engine, Duration::from_millis(tick_ms));
        let options = PluginOptions {
            epoch_deadline_ticks: Some(ticks),
            ..PluginOptions::default()
        };
        let mut samples = Vec::new();
        let mut message = String::new();
        for _ in 0..10 {
            let mut plugin = Plugin::new(&engine, &component, &linker, options)?;
            let (result, elapsed) = plugin.spin();
            let error = result.expect_err("spin must trap");
            message = format!("{error:#}").replace('\n', " | ");
            samples.push(elapsed);
        }
        summary(
            &format!("spin, {tick_ms} ms tick, deadline {ticks} ticks"),
            &mut samples,
            ms,
        );
        println!("  trap: {message}");
    }
    Ok(())
}

/// Whether `StoreLimits` stops `hog` (1 GiB), with an unlimited control run.
fn memory_limit(bytes: &[u8]) -> Result<()> {
    println!("\n== StoreLimits vs hog() (1 GiB) ==");
    let engine = make_engine(false);
    let component = Component::new(&engine, bytes)?;
    let linker = make_linker(&engine);
    for (limit, trap_on_grow_failure) in [
        (Some(64usize << 20), false),
        (Some(64 << 20), true),
        (None, false),
    ] {
        let options = PluginOptions {
            memory_limit: limit,
            trap_on_grow_failure,
            ..PluginOptions::default()
        };
        let mut plugin = Plugin::new(&engine, &component, &linker, options)?;
        let start = Instant::now();
        let result = plugin.hog();
        let elapsed = start.elapsed();
        let label = match limit {
            Some(bytes) => format!(
                "limit {} MiB, trap_on_grow_failure={trap_on_grow_failure}",
                bytes >> 20
            ),
            None => "no limiter".to_string(),
        };
        match result {
            Ok(()) => println!("  {label}: returned normally after {}", ms(elapsed)),
            Err(error) => println!(
                "  {label}: trapped after {}: {}",
                ms(elapsed),
                format!("{error:#}").replace('\n', " | ")
            ),
        }
    }
    Ok(())
}

fn recursion() {
    println!("\n== recursive WIT type ==");
    match parse_wit(RECURSIVE_WIT) {
        Ok(()) => println!("  accepted"),
        Err(error) => println!("  rejected: {}", error.replace('\n', " | ")),
    }
}
