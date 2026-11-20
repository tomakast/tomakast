//! The CPU time of the calling thread, which the soft render deadline is
//! measured in.

use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// A point in the calling thread's CPU time; [`elapsed`](Self::elapsed) on
/// the same thread only.
#[derive(Clone, Copy)]
pub(crate) struct ThreadTime(Duration);

impl ThreadTime {
    pub(crate) fn now() -> ThreadTime {
        ThreadTime(thread_time())
    }

    /// CPU time the calling thread used since `self`.
    pub(crate) fn elapsed(self) -> Duration {
        thread_time().saturating_sub(self.0)
    }
}

/// Wall time since the first call, for platforms without a thread clock.
fn wall_time() -> Duration {
    static EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);
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
