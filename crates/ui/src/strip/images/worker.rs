//! The raster client threads: they hand widget images to the raster helper process and
//! wait for its answer, on threads of their own rather than GPUI's shared background pool.
//!
//! One thread takes the jobs in order, and writes each outcome to its job's [`Inbox`],
//! which the UI thread reads when it looks (nothing wakes it). A job's work marks when it
//! really starts ([`Started::start`]: a new helper first loads its fonts). A job that
//! panics answers with an error; so does one that ran past its limit. A thread found stuck
//! in a job ([`stuck`]) is left to finish on its own, and another starts so the jobs behind
//! it go on, up to [`MAX_THREADS`]; the helper is stopped at the limit, so this is a
//! backstop.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use super::Done;

/// The most raster threads alive at once.
pub const MAX_THREADS: usize = 3;

/// What a job does, off the UI thread, given its start to mark.
pub(super) type Work = Box<dyn FnOnce(&Started) -> Result<Done, String> + Send>;

/// Where jobs write what they gave, by job id; read by the UI thread.
#[derive(Clone, Default)]
pub(super) struct Inbox(Arc<Mutex<Vec<Outcome>>>);

/// A job's id and what it gave.
pub(super) type Outcome = (u64, Result<Done, String>);

impl Inbox {
    /// The outcomes written since the last call.
    pub(super) fn take(&self) -> Vec<Outcome> {
        std::mem::take(
            &mut *self
                .0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// Whether job `id`'s outcome waits to be taken.
    #[cfg(test)]
    pub(super) fn holds(&self, id: u64) -> bool {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .any(|(job, _)| *job == id)
    }

    fn put(&self, id: u64, outcome: Result<Done, String>) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((id, outcome));
    }
}

struct Job {
    id: u64,
    work: Work,
    started: Started,
    /// A job that runs longer is answered with an error, whatever it gave.
    limit: Option<Duration>,
    inbox: Inbox,
}

/// When a job started running, once it has; shared with the UI thread.
#[derive(Clone, Default)]
pub(super) struct Started(Arc<Mutex<Option<Instant>>>);

impl Started {
    /// How long the job has been running; `None` while it waits its turn.
    pub(super) fn running(&self) -> Option<Duration> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .map(|at| at.elapsed())
    }

    /// Marks the job as running from now.
    pub(super) fn start(&self) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Instant::now());
    }

    /// Marks the job as waiting again: it moves to a new helper, which must start first.
    /// (Unit tests do jobs in their own process, with no helper.)
    #[cfg_attr(test, allow(dead_code))]
    pub(super) fn stop(&self) {
        *self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

struct Pool {
    jobs: async_channel::Sender<Job>,
    queue: async_channel::Receiver<Job>,
    threads: AtomicUsize,
}

static POOL: LazyLock<Pool> = LazyLock::new(|| {
    let (jobs, queue) = async_channel::unbounded();
    let pool = Pool {
        jobs,
        queue,
        threads: AtomicUsize::new(0),
    };
    pool.spawn();
    pool
});

impl Pool {
    /// Starts one more thread, unless [`MAX_THREADS`] are alive.
    fn spawn(&self) {
        if self.threads.fetch_add(1, Ordering::SeqCst) >= MAX_THREADS {
            self.threads.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let queue = self.queue.clone();
        let spawned = std::thread::Builder::new()
            .name("tomakast-raster".into())
            .spawn(move || run(queue));
        if let Err(e) = spawned {
            self.threads.fetch_sub(1, Ordering::SeqCst);
            tracing::error!(error = %e, "the raster thread could not start");
        }
    }
}

/// Queues `work` as job `id`, answered into `inbox`, and refused if it runs longer than
/// `limit`; returns when it starts.
pub(super) fn submit(id: u64, work: Work, limit: Option<Duration>, inbox: &Inbox) -> Started {
    let started = Started::default();
    let job = Job {
        id,
        work,
        started: started.clone(),
        limit,
        inbox: inbox.clone(),
    };
    // Unbounded, and the pool keeps a receiver: it never fails.
    let _ = POOL.jobs.send_blocking(job);
    started
}

/// How many raster threads are alive.
#[cfg(test)]
pub(super) fn threads() -> usize {
    POOL.threads.load(Ordering::SeqCst)
}

/// A thread is stuck in a job past its limit: another one takes the jobs behind it.
pub(super) fn stuck() {
    POOL.spawn();
}

fn run(queue: async_channel::Receiver<Job>) {
    while let Ok(job) = queue.recv_blocking() {
        let Job {
            id,
            work,
            started,
            limit,
            inbox,
        } = job;
        let mut outcome =
            catch_unwind(AssertUnwindSafe(|| work(&started))).unwrap_or_else(|panic| {
                let message = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_default();
                Err(format!("its work panicked: {message}"))
            });
        let took = started.running().unwrap_or_default();
        if let Some(limit) = limit
            && took > limit
        {
            outcome = Err(format!(
                "ran for {} ms, past {} ms",
                took.as_millis(),
                limit.as_millis()
            ));
        }
        inbox.put(id, outcome);
    }
}
