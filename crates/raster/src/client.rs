//! The launcher's side of the helper: starts it on the first job, sends one job at a time,
//! and stops it when a job runs past its time, when it breaks the protocol, or after it
//! has used half its CPU limit. The next job starts a new one.
//!
//! A helper that ran a plugin's bytes is not trusted. Its pipes are read and written by
//! threads of its own, so that a helper that never reads, or never answers, holds the
//! client no longer than the job's limit, counted from before the job is written. Exactly
//! one reply is read per job sent, and it must name that job: a reply while no job waits
//! for one, a second reply, or a reply to another job stops the helper, unread. The
//! client so holds at most one reply.

use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::protocol::{self, Reply, VERSION};
use crate::{Job, Pixels};

/// The helper's address space, set before it starts (unix).
pub const MEMORY_LIMIT: u64 = 512 << 20;
/// The helper's CPU time in all, set before it starts (unix): a backstop, since the client
/// starts a new helper once one has used [`RESTART_AFTER_CPU`].
pub const CPU_LIMIT: Duration = Duration::from_secs(60);
pub const RESTART_AFTER_CPU: Duration = Duration::from_secs(30);
/// How long a new helper may take to load its fonts and say it is ready.
pub const STARTUP: Duration = Duration::from_secs(10);

/// The only variables of the launcher's environment a helper gets: those its font lookup
/// reads, so that it finds the user's fonts as the launcher would.
pub const HELPER_ENV: [&str; 6] = [
    "HOME",
    "USERPROFILE",
    "SYSTEMROOT",
    "FONTCONFIG_FILE",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
];

/// What a helper's pipe threads hear.
enum Heard {
    /// It is ready, speaking this protocol version.
    Ready(u8),
    Reply(Reply),
    /// It stopped: its output ended, or its input broke.
    Gone(String),
    /// It broke the protocol; it was stopped.
    Broke(String),
}

/// The process, shared with the reader thread, which stops it when it breaks the protocol.
type Process = Arc<Mutex<Child>>;

struct Helper {
    process: Process,
    /// To the writer thread: one job at a time.
    jobs: SyncSender<(u64, Job)>,
    /// From the writer thread: whether it wrote each job whole.
    wrote: Receiver<(u64, bool)>,
    heard: Receiver<Heard>,
    /// The job whose reply the reader may read next, if one waits.
    turn: Arc<Mutex<Option<u64>>>,
}

/// What one attempt at a job gave.
enum Attempt {
    Done(Result<Pixels, String>),
    /// The helper stopped before it took the job: it may run on a new one.
    NotTaken(String),
}

/// The helper at `program`, started when a job needs it.
pub struct Client {
    program: PathBuf,
    helper: Option<Helper>,
    next_job: u64,
    /// Replies read from every helper so far.
    replies: Arc<AtomicUsize>,
    /// Why no helper at `program` can do any job (it speaks another version).
    unusable: Option<String>,
}

impl Client {
    pub fn new(program: PathBuf) -> Client {
        Client {
            program,
            helper: None,
            next_job: 0,
            replies: Arc::default(),
            unusable: None,
        }
    }

    /// Does `job` in the helper. `running(true)` marks when the job's time starts, just
    /// before it is written (a new helper's start is not the job's), and `running(false)`
    /// that it stopped without an outcome, while the job moves to a new helper. A job that
    /// runs longer than `limit` stops the helper and is refused; so is one whose helper
    /// stops or breaks the protocol. A job the helper stopped before taking (it may have
    /// been killed while idle, by the OOM killer say) runs once more on a new helper.
    pub fn run(
        &mut self,
        job: &Job,
        limit: Duration,
        mut running: impl FnMut(bool),
    ) -> Result<Pixels, String> {
        match self.attempt(job, limit, &mut running)? {
            Attempt::Done(outcome) => outcome,
            Attempt::NotTaken(_) => {
                running(false);
                match self.attempt(job, limit, &mut running)? {
                    Attempt::Done(outcome) => outcome,
                    Attempt::NotTaken(why) => Err(format!("the raster helper stopped ({why})")),
                }
            }
        }
    }

    /// How many replies the client has read from its helpers: never more than the jobs it
    /// sent.
    #[doc(hidden)]
    pub fn replies_read(&self) -> usize {
        self.replies.load(Ordering::SeqCst)
    }

    /// Sends `job` to a running helper, and waits for its one reply, up to `limit`.
    fn attempt(
        &mut self,
        job: &Job,
        limit: Duration,
        running: &mut impl FnMut(bool),
    ) -> Result<Attempt, String> {
        self.next_job += 1;
        let id = self.next_job;
        let helper = self.helper()?;
        *lock(&helper.turn) = Some(id);
        running(true);
        let deadline = Instant::now() + limit;
        if let Err(e) = helper.jobs.try_send((id, job.clone())) {
            // Full: it answered a job before it read it all.
            let why = match e {
                TrySendError::Full(_) => "it answered a job it had not read",
                TrySendError::Disconnected(_) => "its input broke",
            };
            self.stop();
            return Ok(Attempt::NotTaken(why.into()));
        }
        let heard = helper.heard.recv_timeout(limit);
        Ok(match heard {
            Ok(Heard::Reply(reply)) => {
                if reply.helper_cpu > RESTART_AFTER_CPU {
                    self.stop();
                }
                Attempt::Done(reply.outcome)
            }
            Ok(Heard::Broke(why)) => {
                self.stop();
                Attempt::Done(Err(format!(
                    "the raster helper broke the protocol ({why}): stopped"
                )))
            }
            Ok(Heard::Ready(_)) => {
                self.stop();
                Attempt::Done(Err("the raster helper said it was ready again".into()))
            }
            Ok(Heard::Gone(why)) => self.gone(id, why, deadline),
            Err(RecvTimeoutError::Disconnected) => {
                self.gone(id, "its pipes closed".into(), deadline)
            }
            Err(RecvTimeoutError::Timeout) => {
                self.stop();
                Attempt::Done(Err(format!(
                    "still rasterizing after {} ms: stopped",
                    limit.as_millis()
                )))
            }
        })
    }

    /// The helper stopped during job `id`: whether it had taken the job whole, as its
    /// writer thread tells once the stopped helper's pipe breaks.
    fn gone(&mut self, id: u64, why: String, deadline: Instant) -> Attempt {
        let Some(helper) = self.helper.take() else {
            return Attempt::Done(Err(format!("the raster helper stopped ({why})")));
        };
        kill(&helper.process);
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .max(Duration::from_millis(100));
        let taken = loop {
            match helper.wrote.recv_timeout(wait) {
                Ok((wrote, whole)) if wrote == id => break whole,
                Ok(_) => continue,
                // It never finished writing: it never took the job.
                Err(_) => break false,
            }
        };
        reap(&helper.process);
        if taken {
            Attempt::Done(Err(format!("the raster helper stopped ({why})")))
        } else {
            Attempt::NotTaken(why)
        }
    }

    /// The helper, started if it is not running. One that stopped or spoke while idle is
    /// stopped and replaced first.
    fn helper(&mut self) -> Result<&mut Helper, String> {
        if let Some(why) = &self.unusable {
            return Err(why.clone());
        }
        if let Some(helper) = &self.helper {
            while helper.wrote.try_recv().is_ok() {}
            let spoke = helper.heard.try_recv().is_ok();
            let stopped = !matches!(lock(&helper.process).try_wait(), Ok(None));
            if spoke || stopped {
                self.stop();
            }
        }
        if self.helper.is_none() {
            self.helper = Some(match start(&self.program, &self.replies) {
                Ok(helper) => helper,
                Err(Start::Failed(why)) => return Err(why),
                Err(Start::Unusable(why)) => {
                    tracing::error!("{why}; plugin images are not shown");
                    self.unusable = Some(why.clone());
                    return Err(why);
                }
            });
        }
        Ok(self.helper.as_mut().expect("just started"))
    }

    /// Stops the helper, if one runs. Its pipe threads end as its pipes close.
    fn stop(&mut self) {
        if let Some(helper) = self.helper.take() {
            kill(&helper.process);
            reap(&helper.process);
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn kill(process: &Process) {
    let _ = lock(process).kill();
}

fn reap(process: &Process) {
    let _ = lock(process).wait();
}

/// Why a helper did not start.
enum Start {
    Failed(String),
    /// It speaks another protocol version: no helper at its path can do any job.
    Unusable(String),
}

/// Starts the helper at `program`, with its limits, no environment but [`HELPER_ENV`] and
/// `/` as its directory, and waits until it is ready.
fn start(program: &Path, replies: &Arc<AtomicUsize>) -> Result<Helper, Start> {
    let failed = |e: &dyn std::fmt::Display| {
        Start::Failed(format!(
            "the raster helper {} did not start: {e}",
            program.display()
        ))
    };
    // Absolute: its directory is not the launcher's.
    let program = std::path::absolute(program).map_err(|e| failed(&e))?;
    let mut command = Command::new(&program);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .env_clear();
    for key in HELPER_ENV {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    #[cfg(unix)]
    {
        command.current_dir("/");
        limit(&mut command);
    }
    let mut child = command.spawn().map_err(|e| failed(&e))?;
    let input = BufWriter::new(child.stdin.take().expect("piped"));
    let output = BufReader::new(child.stdout.take().expect("piped"));
    let process: Process = Arc::new(Mutex::new(child));

    // Capacity 1: one reply, or one word that it stopped, at a time.
    let (tell, heard) = mpsc::sync_channel(1);
    let (jobs, queue) = mpsc::sync_channel::<(u64, Job)>(1);
    let (wrote_tell, wrote) = mpsc::sync_channel(1);
    let turn = Arc::new(Mutex::new(None));
    let reader = {
        let (tell, turn, process, replies) =
            (tell.clone(), turn.clone(), process.clone(), replies.clone());
        std::thread::Builder::new()
            .name("tomakast-raster-reader".into())
            .spawn(move || read(output, &tell, &turn, &process, &replies))
    };
    let writer = std::thread::Builder::new()
        .name("tomakast-raster-writer".into())
        .spawn(move || write(input, queue, &wrote_tell, &tell));
    if let Err(e) = reader.and(writer) {
        kill(&process);
        reap(&process);
        return Err(Start::Failed(format!(
            "the raster helper's pipe threads did not start: {e}"
        )));
    }
    let helper = Helper {
        process,
        jobs,
        wrote,
        heard,
        turn,
    };
    let stopped = |why: Start| {
        kill(&helper.process);
        reap(&helper.process);
        why
    };
    match helper.heard.recv_timeout(STARTUP) {
        Ok(Heard::Ready(VERSION)) => Ok(helper),
        Ok(Heard::Ready(other)) => {
            let heard = if other == b'R' {
                "1".to_owned()
            } else {
                other.to_string()
            };
            Err(stopped(Start::Unusable(format!(
                "the raster helper {} speaks protocol version {heard}, not {VERSION}: it is \
                 not the one built with this launcher",
                program.display()
            ))))
        }
        Ok(Heard::Gone(why) | Heard::Broke(why)) => Err(stopped(Start::Failed(format!(
            "the raster helper stopped as it started ({why})"
        )))),
        Ok(Heard::Reply(_)) | Err(_) => Err(stopped(Start::Failed(
            "the raster helper was not ready in time".into(),
        ))),
    }
}

/// The reader thread: the ready byte, then for each reply its first byte, and the rest
/// only if a job waits for it ([`Helper::turn`]) and the reply names that job; anything
/// else stops the helper. Ends with the helper's output, or when nobody listens.
fn read(
    mut output: BufReader<std::process::ChildStdout>,
    tell: &SyncSender<Heard>,
    turn: &Mutex<Option<u64>>,
    process: &Process,
    replies: &AtomicUsize,
) {
    let broke = |why: &str| {
        kill(process);
        let _ = tell.send(Heard::Broke(why.into()));
    };
    match protocol::read_ready(&mut output) {
        Ok(version) => {
            if tell.send(Heard::Ready(version)).is_err() || version != VERSION {
                return;
            }
        }
        Err(e) => {
            let _ = tell.send(Heard::Gone(e.to_string()));
            return;
        }
    }
    loop {
        let mut status = [0];
        if let Err(e) = std::io::Read::read_exact(&mut output, &mut status) {
            let _ = tell.send(Heard::Gone(e.to_string()));
            return;
        }
        let Some(expected) = lock(turn).take() else {
            return broke("a reply while no job waited for one");
        };
        let reply = match protocol::read_reply_after(status[0], &mut output) {
            Ok(reply) => reply,
            Err(e) => return broke(&format!("a malformed reply: {e}")),
        };
        replies.fetch_add(1, Ordering::SeqCst);
        if reply.job != expected {
            return broke("a reply to another job");
        }
        if tell.send(Heard::Reply(reply)).is_err() {
            return;
        }
    }
}

/// The writer thread: writes each job, and tells whether it wrote it whole. Ends when the
/// helper's input breaks, or when the client lets go of the helper.
fn write(
    mut input: BufWriter<std::process::ChildStdin>,
    queue: Receiver<(u64, Job)>,
    wrote: &SyncSender<(u64, bool)>,
    tell: &SyncSender<Heard>,
) {
    for (id, job) in queue {
        let written = protocol::write_job(&mut input, id, &job);
        if wrote.send((id, written.is_ok())).is_err() {
            return;
        }
        if let Err(e) = written {
            let _ = tell.send(Heard::Gone(format!("it took no job: {e}")));
            return;
        }
    }
}

/// Limits the helper's address space and CPU time before it runs; on Linux, it is also
/// killed when the thread that started it ends (the client's threads live as long as the
/// launcher), so that a helper never outlives a launcher killed outright.
#[cfg(unix)]
fn limit(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    let cpu = CPU_LIMIT.as_secs() as libc::rlim_t;
    let memory = MEMORY_LIMIT as libc::rlim_t;
    #[cfg(target_os = "linux")]
    // SAFETY: `getpid` has no preconditions.
    let launcher = unsafe { libc::getpid() };
    // SAFETY: between fork and exec, only `setrlimit`, `prctl` and `getppid` run: they are
    // async-signal-safe and touch only the stack values given. Nothing allocates.
    unsafe {
        command.pre_exec(move || {
            let set = |resource, value: libc::rlim_t| {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            };
            set(libc::RLIMIT_AS, memory)?;
            set(libc::RLIMIT_CPU, cpu)?;
            #[cfg(target_os = "linux")]
            {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The launcher died before the signal was asked for.
                if libc::getppid() != launcher {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }
            }
            Ok(())
        });
    }
}
