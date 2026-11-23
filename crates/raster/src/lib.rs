//! Plugin images decoded and rasterized in a helper process: PNG, JPEG and SVG bytes a
//! plugin sent, as straight RGBA pixels.
//!
//! Static checks cannot bound all that a renderer may spend on a document (layers of
//! opacity, pattern tiles, filters), so the work runs in a process of its own: the
//! `tomakast-raster` helper, which runs [`serve`]. The launcher talks to it through a
//! [`Client`], one job at a time. The helper starts with limits on its memory and CPU time,
//! and the client stops it when a job runs past its time; a helper that stops, for any
//! reason, takes only its job with it, and the next job starts a new one.
//!
//! The checks of [`svg`] and [`bitmap`] still run first, in the helper, and refuse what
//! they can tell is too costly with a clear reason.

pub mod bitmap;
mod client;
pub mod protocol;
pub mod svg;
pub mod thumbnail;

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::LazyLock;
use std::time::Duration;

use tomakast_core::ImageFit;

pub use client::{CPU_LIMIT, Client, HELPER_ENV, MEMORY_LIMIT, RESTART_AFTER_CPU, STARTUP};

/// A raster's width and height, in device pixels.
pub type Device = (u32, u32);

/// An image's pixels: straight (not premultiplied) RGBA, row by row, with the CPU time its
/// work took in the helper's thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub cpu: Duration,
}

/// One job for the helper.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    /// A PNG or JPEG, decoded at its own size.
    Bitmap(Arc<[u8]>),
    /// An SVG document, rasterized at `device` with `fit`.
    Svg {
        bytes: Arc<[u8]>,
        device: Device,
        fit: ImageFit,
    },
    /// A file's image (PNG, JPEG, WebP or GIF, at most [`protocol::MAX_THUMBNAIL_BYTES`]),
    /// its first frame shrunk to fit `max_side` (never enlarged), turned as its EXIF says.
    Thumbnail { bytes: Arc<[u8]>, max_side: u32 },
    /// For tests: sleeps this long, then answers a pixel.
    Sleep(Duration),
    /// For tests: holds this many MiB, then answers a pixel.
    Allocate(u32),
    /// For tests: panics.
    Panic,
}

/// Does `job`, in this process. A panic is answered as an error.
pub fn run(job: &Job) -> Result<Pixels, String> {
    catch_unwind(AssertUnwindSafe(|| run_unguarded(job))).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        Err(format!("its work panicked: {message}"))
    })
}

fn run_unguarded(job: &Job) -> Result<Pixels, String> {
    let pixel = || Pixels {
        width: 1,
        height: 1,
        rgba: vec![0, 0, 0, 255],
        cpu: Duration::ZERO,
    };
    match job {
        Job::Bitmap(bytes) => bitmap::decode(bytes),
        Job::Svg { bytes, device, fit } => svg::rasterize(bytes, *device, *fit),
        Job::Thumbnail { bytes, max_side } => thumbnail::thumbnail(bytes, *max_side),
        Job::Sleep(long) => {
            std::thread::sleep(*long);
            Ok(pixel())
        }
        Job::Allocate(mib) => {
            let held = vec![1u8; (*mib as usize) << 20];
            std::hint::black_box(&held);
            Ok(pixel())
        }
        Job::Panic => panic!("a job that panics"),
    }
}

/// `tomakast-raster` next to the running executable, where the launcher is installed with
/// its helper.
pub fn helper_next_to_executable() -> std::path::PathBuf {
    let name = format!("tomakast-raster{}", std::env::consts::EXE_SUFFIX);
    std::env::current_exe()
        .map(|exe| exe.with_file_name(&name))
        .unwrap_or_else(|_| std::path::PathBuf::from(name))
}

/// The helper's loop: loads the fonts, says it is ready, then does each job read from
/// stdin and writes its answer to stdout, until stdin closes or a request is malformed.
pub fn serve() {
    svg::load_fonts();
    let mut input = std::io::BufReader::new(std::io::stdin().lock());
    let mut output = std::io::BufWriter::new(std::io::stdout().lock());
    if protocol::write_ready(&mut output).is_err() {
        return;
    }
    // Panics are answered, not printed twice.
    std::panic::set_hook(Box::new(|_| {}));
    while let Ok(Some((id, job))) = protocol::read_job(&mut input) {
        let outcome = run(&job);
        if protocol::write_reply(&mut output, id, &outcome, process_cpu()).is_err() {
            return;
        }
    }
}

/// The CPU time this process has used.
#[cfg(unix)]
pub fn process_cpu() -> Duration {
    // SAFETY: an all-zero `rusage` is valid; `getrusage` writes one through our pointer.
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: as above.
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) } != 0 {
        return Duration::ZERO;
    }
    let time = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
    time(usage.ru_utime) + time(usage.ru_stime)
}

#[cfg(not(unix))]
pub fn process_cpu() -> Duration {
    Duration::ZERO
}

/// The CPU time of the calling thread since [`ThreadClock::start`]; the wall time where
/// that clock is missing.
pub(crate) struct ThreadClock(Duration);

impl ThreadClock {
    pub(crate) fn start() -> ThreadClock {
        ThreadClock(thread_time())
    }

    pub(crate) fn elapsed(&self) -> Duration {
        thread_time().saturating_sub(self.0)
    }
}

/// Wall time since the first call, for platforms without a thread clock.
fn wall_time() -> Duration {
    static EPOCH: LazyLock<std::time::Instant> = LazyLock::new(std::time::Instant::now);
    EPOCH.elapsed()
}

#[cfg(unix)]
fn thread_time() -> Duration {
    let mut now = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `clock_gettime` writes one `timespec` through a valid pointer to ours.
    let read = unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut now) };
    if read != 0 {
        return wall_time();
    }
    Duration::new(now.tv_sec as u64, now.tv_nsec as u32)
}

#[cfg(not(unix))]
fn thread_time() -> Duration {
    wall_time()
}
