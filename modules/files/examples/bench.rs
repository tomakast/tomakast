//! Times the file index and file search over the home folder. Read-only: it walks `$HOME`,
//! mutates only the in-memory index, and prints counts and timings, never file names.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tomakast_module_files::index::FileIndex;
use tomakast_module_files::rules::{Coverage, Rules};
use tomakast_module_files::search::{Signals, search};

const QUERIES: &[&str] = &[
    "fire",
    "doc",
    "readme",
    "main.rs",
    "proj/read",
    "ашку",
    "otchet",
    "png",
    "config",
    "cargo",
    "invoice",
    "screen",
    "notes",
    "todo",
    "zzzqx",
    ".pdf",
    "photo",
    "downloads",
    "rust",
    "kde",
    "a/e",
    "doc/re",
    // No letter: the root list does not answer it, the screen does (timed here as both).
    "10/2",
];
const RUNS: usize = 30;
const BURST: usize = 5_000;
/// The rows of the "Search Files" screen.
const LIMIT: usize = 100;
/// The files and folders of the root list.
const ROOT_LIMIT: usize = 3;

/// Resident memory in MB (Linux; 0 elsewhere).
fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find(|l| l.starts_with("VmRSS:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|kb| kb.parse::<f64>().ok())
        .map_or(0.0, |kb| kb / 1024.0)
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Walks on a thread of its own, as the module does, and times it.
fn walk(rules: &Rules) -> (FileIndex, Duration) {
    let rules = rules.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        let index = FileIndex::walk(&rules);
        (index, started.elapsed())
    })
    .join()
    .expect("the walk panicked")
}

fn main() {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        eprintln!("HOME is not set");
        return;
    };
    let rules = Rules::new(&Coverage::default(), &home);
    let before = rss_mb();
    let (mut index, took) = walk(&rules);
    let after = rss_mb();
    println!("walk (cold or warm, as run): {:.0} ms", ms(took));
    println!("entries: {}", index.len());
    println!("resident memory: {after:.1} MB after the walk ({before:.1} MB before)");

    // A burst of changes: 5,000 files dropped, then put back.
    let files: Vec<PathBuf> = {
        let all: Vec<_> = index.ids().filter(|&id| !index.entry(id).is_dir).collect();
        let step = (all.len() / BURST).max(1);
        all.iter()
            .step_by(step)
            .take(BURST)
            .map(|&id| index.path(id))
            .collect()
    };
    let started = Instant::now();
    for p in &files {
        index.remove(p);
    }
    let removes = started.elapsed();
    let started = Instant::now();
    for p in &files {
        index.upsert(p, &rules);
    }
    let upserts = started.elapsed();
    println!(
        "burst of {}: removes {:.0} ms, upserts {:.0} ms; entries after: {}",
        files.len(),
        ms(removes),
        ms(upserts),
        index.len()
    );
    println!("resident memory after the burst: {:.1} MB", rss_mb());

    // A re-walk builds a second index while the first still answers searches.
    let (again, took) = walk(&rules);
    let both = rss_mb();
    drop(index);
    let index = again;
    println!(
        "re-walk: {:.0} ms; resident memory {both:.1} MB with both indexes, {:.1} MB after \
         dropping the old one",
        ms(took),
        rss_mb()
    );

    // The saved index: written after a walk, read back at the next start.
    let started = Instant::now();
    let bytes = index.to_snapshot(1);
    let written = started.elapsed();
    let started = Instant::now();
    let back = FileIndex::from_snapshot(&bytes, 1).expect("the snapshot reads back");
    let read = started.elapsed();
    assert_eq!(back.len(), index.len());
    println!(
        "saved index: {:.1} MB, built in {:.0} ms, read back in {:.0} ms (no disk: the file \
         read itself is extra)",
        bytes.len() as f64 / 1e6,
        ms(written),
        ms(read)
    );
    drop((bytes, back));

    let opened = HashMap::new();
    let recent = HashSet::new();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let signals = Signals {
        opened: &opened,
        recent: &recent,
        home: Path::new(&home),
        now,
    };
    println!(
        "{:<12} {:>11} {:>11} {:>11} {:>11} {:>8} {:>8}",
        "query", "root med", "root p95", "screen med", "screen p95", "shown", "matches"
    );
    // Median and 95th percentile in ms of `RUNS` searches for `query` with `limit` results.
    let time = |query: &str, limit: usize| {
        let mut times: Vec<Duration> = (0..RUNS)
            .map(|_| {
                let started = Instant::now();
                std::hint::black_box(search(&index, query, limit, &signals));
                started.elapsed()
            })
            .collect();
        times.sort_unstable();
        (
            ms(times[RUNS / 2]),
            ms(times[(RUNS * 95).div_ceil(100) - 1]),
        )
    };
    let mut worst: f64 = 0.0;
    for query in QUERIES {
        let (root_median, root_p95) = time(query, ROOT_LIMIT);
        let (median, p95) = time(query, LIMIT);
        worst = worst.max(p95).max(root_p95);
        let shown = search(&index, query, LIMIT, &signals).len();
        let matches = search(&index, query, usize::MAX, &signals).len();
        println!(
            "{query:<12} {root_median:>11.2} {root_p95:>11.2} {median:>11.2} {p95:>11.2} \
             {shown:>8} {matches:>8}"
        );
    }
    println!("worst p95: {worst:.2} ms");
    println!("resident memory at the end: {:.1} MB", rss_mb());
}
