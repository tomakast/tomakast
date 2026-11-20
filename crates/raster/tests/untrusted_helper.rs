//! The client against helpers that break the protocol: a helper that ran a plugin's bytes is
//! not trusted. Each fake helper is this test binary itself, linked or copied under the
//! name `fake-<mode>`: started under that name, [`main`] runs the fake instead of the tests
//! (the client gives a helper no argument, no environment and `/` as its directory, so the
//! name is all a fake can go by). A fake notes each start in `<mode>.spawns`, next to it.
//!
//! No test harness (`harness = false`): libtest would write to stdout, which is the fake's
//! pipe. `main` runs the tests one by one, those whose name contains the first argument
//! that is not a flag.

use std::io::{BufReader, BufWriter, Read as _, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tomakast_core::ImageFit;
use tomakast_raster::protocol;
use tomakast_raster::{Client, Job, Pixels};

const TESTS: &[(&str, fn())] = &[
    (
        "a_flood_of_replies_stops_the_helper_and_the_next_job_runs_on_a_new_one",
        a_flood_of_replies_stops_the_helper_and_the_next_job_runs_on_a_new_one,
    ),
    (
        "a_reply_to_another_job_refuses_this_one",
        a_reply_to_another_job_refuses_this_one,
    ),
    (
        "a_helper_that_never_reads_fails_the_job_in_time_and_frees_the_lock",
        a_helper_that_never_reads_fails_the_job_in_time_and_frees_the_lock,
    ),
    (
        "a_helper_that_died_while_idle_is_replaced_before_the_next_job",
        a_helper_that_died_while_idle_is_replaced_before_the_next_job,
    ),
    (
        "a_job_the_helper_never_took_is_tried_once_more",
        a_job_the_helper_never_took_is_tried_once_more,
    ),
    (
        "a_helper_of_another_version_is_refused",
        a_helper_of_another_version_is_refused,
    ),
    (
        "the_helper_gets_no_environment_and_the_root_directory",
        the_helper_gets_no_environment_and_the_root_directory,
    ),
    #[cfg(target_os = "linux")]
    (
        "the_helper_dies_with_the_launcher",
        the_helper_dies_with_the_launcher,
    ),
];

fn main() {
    if let Some(mode) = fake_mode() {
        fake::serve(&mode);
        return;
    }
    let filter = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with('-'))
        .unwrap_or_default();
    let mut failed = Vec::new();
    for (name, test) in TESTS.iter().filter(|(name, _)| name.contains(&filter)) {
        let start = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(test));
        let verdict = if outcome.is_ok() { "ok" } else { "FAILED" };
        println!("test {name} ... {verdict} ({:?})", start.elapsed());
        if outcome.is_err() {
            failed.push(*name);
        }
    }
    if !failed.is_empty() {
        println!("failed: {failed:?}");
        std::process::exit(1);
    }
}

/// The mode this binary runs as, when started as a fake helper.
fn fake_mode() -> Option<String> {
    let me = PathBuf::from(std::env::args_os().next()?);
    let stem = me.file_stem()?.to_str()?;
    stem.strip_prefix("fake-").map(str::to_owned)
}

/// The wire, as the fakes speak it.
mod wire {
    use super::*;

    pub fn ready(output: &mut impl std::io::Write) {
        protocol::write_ready(output).unwrap();
    }

    pub fn read_job(input: &mut impl std::io::Read) -> Option<(u64, Job)> {
        protocol::read_job(input).ok().flatten()
    }

    pub fn reply(output: &mut impl std::io::Write, job: u64, pixels: &Pixels) {
        protocol::write_reply(output, job, &Ok(pixels.clone()), Duration::ZERO).unwrap();
    }
}

mod fake {
    use super::*;

    /// Runs as the fake helper `mode`.
    pub fn serve(mode: &str) {
        let dir = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_owned();
        let spawns = dir.join(format!("{mode}.spawns"));
        let before = std::fs::read_to_string(&spawns).unwrap_or_default();
        let first = before.is_empty();
        std::fs::write(&spawns, format!("{before}{}\n", std::process::id())).unwrap();

        let mut input = BufReader::new(std::io::stdin().lock());
        let mut output = BufWriter::new(std::io::stdout().lock());
        match mode {
            // A launcher of its own, given the helper: sends it a job and waits.
            "launcher" => {
                let helper = std::env::args_os().nth(1).unwrap();
                let mut client = Client::new(helper.into());
                let _ = client.run(&large(), Duration::from_secs(60), |_| {});
                return;
            }
            // Says it is ready, then never reads a job.
            "noread" => {
                wire::ready(&mut output);
                std::thread::sleep(Duration::from_secs(60));
                return;
            }
            // The ready byte of the first helpers.
            "old" => {
                output.write_all(b"R").unwrap();
                output.flush().unwrap();
                std::thread::sleep(Duration::from_secs(60));
                return;
            }
            // The first one stops once a job starts coming, before it took it all.
            "dies-on-take" if first => {
                wire::ready(&mut output);
                let _ = input.read(&mut [0]);
                return;
            }
            "env" => {
                let env: Vec<String> = std::env::vars_os()
                    .map(|(key, _)| key.to_string_lossy().into_owned())
                    .collect();
                let cwd = std::env::current_dir().unwrap();
                let seen = format!("{}\n{}", cwd.display(), env.join("\n"));
                std::fs::write(dir.join("env.seen"), seen).unwrap();
            }
            _ => {}
        }
        wire::ready(&mut output);
        let flood = Pixels {
            width: 4096,
            height: 4096,
            rgba: vec![0; 4096 * 4096 * 4],
            cpu: Duration::ZERO,
        };
        let mut stale = None;
        while let Some((id, job)) = wire::read_job(&mut input) {
            let pixels = answer(&job);
            if let Some(old) = stale.take() {
                wire::reply(&mut output, old, &pixels);
            }
            wire::reply(&mut output, id, &pixels);
            let bitmap = matches!(job, Job::Bitmap(_));
            match mode {
                // Answers a bitmap 40 times, with the largest replies there are.
                "flood" if bitmap => {
                    for _ in 0..40 {
                        wire::reply(&mut output, id, &flood);
                    }
                }
                // Answers a bitmap again, with the next job.
                "stale" if bitmap => stale = Some(id),
                // Stops after a bitmap, as if something killed it.
                "dies-idle" if bitmap => return,
                _ => {}
            }
        }
    }

    /// A fair answer: an SVG at its device size, anything else as one pixel.
    fn answer(job: &Job) -> Pixels {
        let (width, height) = match job {
            Job::Svg { device, .. } => *device,
            _ => (1, 1),
        };
        Pixels {
            width,
            height,
            rgba: vec![7; (width * height * 4) as usize],
            cpu: Duration::ZERO,
        }
    }
}

/// The fake helpers of one test, in a directory of their own.
struct Fakes {
    dir: tempfile::TempDir,
}

impl Fakes {
    fn new() -> Fakes {
        Fakes {
            dir: tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap(),
        }
    }

    /// The fake helper `mode`.
    fn helper(&self, mode: &str) -> PathBuf {
        let path = self
            .dir
            .path()
            .join(format!("fake-{mode}{}", std::env::consts::EXE_SUFFIX));
        if !path.exists() {
            let me = std::env::current_exe().unwrap();
            if std::fs::hard_link(&me, &path).is_err() {
                std::fs::copy(&me, &path).unwrap();
            }
        }
        path
    }

    /// How many times `mode` started.
    fn spawns(&self, mode: &str) -> usize {
        self.spawned(mode).len()
    }

    /// The pids `mode` started as.
    fn spawned(&self, mode: &str) -> Vec<u32> {
        std::fs::read_to_string(self.dir.path().join(format!("{mode}.spawns")))
            .unwrap_or_default()
            .lines()
            .map(|line| line.parse().unwrap())
            .collect()
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

const LIMIT: Duration = Duration::from_millis(500);

fn run(client: &mut Client, job: &Job) -> Result<Pixels, String> {
    client.run(job, LIMIT, |_| {})
}

fn svg() -> Job {
    Job::Svg {
        bytes: Arc::from(&b"<svg/>"[..]),
        device: (20, 10),
        fit: ImageFit::Fill,
    }
}

fn bitmap() -> Job {
    Job::Bitmap(Arc::from(&b"png"[..]))
}

/// A job too large for a pipe's buffer: its write blocks until the helper reads it.
fn large() -> Job {
    Job::Bitmap(Arc::from(vec![0u8; 2 << 20]))
}

#[cfg(target_os = "linux")]
fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done() {
        assert!(Instant::now() < deadline, "{what}: not within 5 s");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The most memory this process has held, on Linux.
fn peak_memory() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|line| line.starts_with("VmHWM:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib << 10)
}

/// A helper answers one job 40 times with 64 MiB each: it is stopped, the client reads one
/// reply only, and the next job runs on a new helper.
fn a_flood_of_replies_stops_the_helper_and_the_next_job_runs_on_a_new_one() {
    let fakes = Fakes::new();
    let mut client = Client::new(fakes.helper("flood"));
    // Whether the flooded job's own reply is taken depends on what the client reads first;
    // only what follows is certain.
    let _ = run(&mut client, &bitmap());
    std::thread::sleep(Duration::from_millis(500));

    assert_eq!(client.replies_read(), 1, "one reply per job sent");
    if let Some(peak) = peak_memory() {
        assert!(peak < 400 << 20, "held {} MiB", peak >> 20);
    }
    let next = run(&mut client, &svg()).unwrap();
    assert_eq!((next.width, next.height), (20, 10));
    assert_eq!(fakes.spawns("flood"), 2, "a new helper");
    assert_eq!(client.replies_read(), 2);
}

/// A reply that names another job refuses the job being done, and stops the helper.
fn a_reply_to_another_job_refuses_this_one() {
    let fakes = Fakes::new();
    let mut client = Client::new(fakes.helper("stale"));
    run(&mut client, &bitmap()).unwrap();

    let err = run(&mut client, &svg()).unwrap_err();
    assert!(err.contains("another job"), "{err}");

    let next = run(&mut client, &svg()).unwrap();
    assert_eq!((next.width, next.height), (20, 10));
    assert_eq!(fakes.spawns("stale"), 2);
}

/// A helper that never reads its job: the job fails within its limit, while another thread
/// waits for the client.
fn a_helper_that_never_reads_fails_the_job_in_time_and_frees_the_lock() {
    let fakes = Fakes::new();
    let client = Arc::new(Mutex::new(Client::new(fakes.helper("noread"))));

    let start = Instant::now();
    let held = {
        let client = client.clone();
        std::thread::spawn(move || {
            let mut client = client.lock().unwrap();
            let outcome = client.run(&large(), LIMIT, |_| {});
            (outcome, Instant::now())
        })
    };
    std::thread::sleep(Duration::from_millis(50));
    let waited = Instant::now();
    drop(client.lock().unwrap());
    let freed = waited.elapsed();
    let (outcome, done) = held.join().unwrap();

    assert!(outcome.is_err());
    // The limit, a fake's start and a kill.
    let bound = LIMIT + Duration::from_secs(1);
    let failed = done.duration_since(start);
    assert!(failed < bound, "failed after {failed:?}");
    assert!(freed < bound, "waited {freed:?}");
}

/// A helper that stopped between jobs (as the OOM killer would stop it) is replaced before
/// the next job, which renders.
fn a_helper_that_died_while_idle_is_replaced_before_the_next_job() {
    let fakes = Fakes::new();
    let mut client = Client::new(fakes.helper("dies-idle"));
    run(&mut client, &bitmap()).unwrap();
    std::thread::sleep(Duration::from_millis(200));

    let next = run(&mut client, &svg()).unwrap();
    assert_eq!((next.width, next.height), (20, 10));
    assert_eq!(fakes.spawns("dies-idle"), 2);
}

/// A helper that stopped before it took the whole job: the job runs on a new helper once.
fn a_job_the_helper_never_took_is_tried_once_more() {
    let fakes = Fakes::new();
    let mut client = Client::new(fakes.helper("dies-on-take"));
    let mut clock = Vec::new();

    let done = client.run(&large(), LIMIT, |running| clock.push(running));

    assert_eq!(done.map(|pixels| pixels.width), Ok(1));
    assert_eq!(fakes.spawns("dies-on-take"), 2);
    // The new helper's start is not on the job's clock.
    assert_eq!(clock, [true, false, true]);
}

/// A helper of another protocol version (an old `tomakast-raster` next to a new launcher)
/// is refused with a clear reason, and not started again for each job.
fn a_helper_of_another_version_is_refused() {
    let fakes = Fakes::new();
    let mut client = Client::new(fakes.helper("old"));
    let start = Instant::now();

    let err = run(&mut client, &svg()).unwrap_err();
    assert!(err.contains("version"), "{err}");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    let again = run(&mut client, &svg()).unwrap_err();
    assert!(again.contains("version"), "{again}");
    assert_eq!(fakes.spawns("old"), 1);
}

/// The helper starts with none of the launcher's environment but what font lookup reads,
/// in the root directory.
fn the_helper_gets_no_environment_and_the_root_directory() {
    // SAFETY: the tests run one at a time, on this thread.
    unsafe { std::env::set_var("TOMAKAST_SECRET", "launcher only") };
    let fakes = Fakes::new();
    let mut client = Client::new(fakes.helper("env"));
    run(&mut client, &svg()).unwrap();

    let seen = std::fs::read_to_string(fakes.path().join("env.seen")).unwrap();
    let mut lines = seen.lines();
    let cwd = lines.next().unwrap();
    let env: Vec<&str> = lines.collect();
    if cfg!(unix) {
        assert_eq!(cwd, "/");
    }
    let allowed = [
        "HOME",
        "USERPROFILE",
        "SYSTEMROOT",
        "FONTCONFIG_FILE",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
    ];
    for key in &env {
        assert!(
            allowed
                .iter()
                .any(|allowed| key.eq_ignore_ascii_case(allowed)),
            "{key} passed to the helper"
        );
    }
}

/// The launcher killed outright: its helper, stuck in a job, dies with it.
#[cfg(target_os = "linux")]
fn the_helper_dies_with_the_launcher() {
    let fakes = Fakes::new();
    let helper = fakes.helper("noread");
    // A launcher that sends the helper a job it never reads, and waits.
    let launcher = fakes.helper("launcher");
    let mut launcher = std::process::Command::new(&launcher)
        .arg(&helper)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    until("the helper starts", || fakes.spawns("noread") == 1);
    let pid = fakes.spawned("noread")[0];

    launcher.kill().unwrap();
    launcher.wait().unwrap();

    let alive = |pid: u32| {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            !stat
                .rsplit(')')
                .next()
                .unwrap_or("")
                .trim()
                .starts_with('Z')
        })
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while alive(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    let survived = alive(pid);
    if survived {
        // SAFETY: a plain signal to the fake this test started.
        unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    }
    assert!(!survived, "the helper outlived the launcher");
}
