//! Widget images: the PNG, JPEG and SVG bytes a plugin sends (`set-widget-image`, or with a
//! frame), as GPUI images for the image nodes and canvases of its tiles.
//!
//! Nothing decodes in the launcher's process: a miss queues a job for the raster helper
//! process ([`tomakast_raster`]), through a client thread ([`worker`]), and reads `None`
//! (draws nothing) until it is done; then the launcher window is drawn again. A plugin has
//! at most one job queued or running at a time. An image is decoded on its first paint
//! only, so nothing is decoded while the launcher is hidden. The cache is kept while it is
//! hidden, so the next showing draws its images at once; only the looks for the jobs'
//! answers stop ([`stop_polls`]).
//! - A PNG or JPEG must declare 1..=1024 pixels on both sides in its header, and is decoded
//!   with the same limits; it is kept at its own size.
//! - An SVG ([`svg`]) is parsed with no external reference and checked against a cost model
//!   before it renders. It is rasterized at the device size it shows at, never larger than
//!   its box, and kept by that size and fit: at a new scale it is rasterized again, and a
//!   raster of another size is never stretched in its place. One that takes longer than
//!   [`raster_deadline`] in CPU time, or is still running past [`STALLED_FACTOR`] times
//!   that (the helper is then stopped), is refused from then on; so is one whose helper
//!   crashes or hits its memory limit.
//!
//! The helper is `tomakast-raster`, found next to the launcher's executable unless
//! [`set_raster_helper`] names another.
//!
//! GPUI's own SVG renderer is never used: it reads `href` paths from disk.
//!
//! The cache is per thread (the UI thread's), like [`crate::plugin_image::cached`]. It keeps
//! at most [`MAX_CACHED_BYTES_PER_PLUGIN`] of pixels per plugin and [`MAX_CACHED_BYTES`] in
//! all, making room by evicting the images least recently drawn, never one drawn within
//! [`IN_USE`]. A replaced image's rasters go at the next tick ([`sweep`]), and so do those
//! of a plugin gone.

mod worker;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
#[cfg(not(test))]
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::{App, AsyncApp, DevicePixels, Pixels, RenderImage, Size, Task, size};
use tomakast_core::{
    Core, Draw, Frame, ImageFit, ImageFormat, NodeKind, SOFT_RENDER_DEADLINE, WidgetImage,
    WidgetKey, WidgetSize,
};
use tomakast_raster::Job;

use super::DEBUG_BUDGET_FACTOR;
use crate::LauncherController;
use crate::plugin_image::{self, MAX_PLUGIN_IMAGE_SIDE};
use crate::stack::Launcher as _;
use worker::{Inbox, Started};

/// Pixels' bytes the cache keeps per plugin, and in all.
pub const MAX_CACHED_BYTES_PER_PLUGIN: usize = 32 << 20;
pub const MAX_CACHED_BYTES: usize = 128 << 20;
/// An image drawn within this long is never evicted to make room for another.
pub const IN_USE: Duration = Duration::from_secs(1);
/// Rasters of one SVG kept at once, at as many sizes.
pub const MAX_SVG_SIZES: usize = 4;
/// An SVG still running this many times its deadline (wall time, from when it started) is
/// refused without waiting for it: its CPU time may lag behind. Its plugin starts no other
/// job until it returns.
pub const STALLED_FACTOR: u32 = 2;

/// A job still running this many times [`stall_limit`] holds its raster thread past the
/// client's own limit: another thread starts for the jobs behind it. A backstop: the client
/// answers at its limit.
pub const STUCK_FACTOR: u32 = 2;

/// How long a PNG or JPEG may take to decode, in wall time, before its helper is stopped
/// and it is refused.
pub const BITMAP_LIMIT: Duration = Duration::from_secs(2);

/// How long an SVG may take to parse, check and rasterize, in the helper's CPU time, before
/// it is refused for good: the render deadline, [`DEBUG_BUDGET_FACTOR`] times as long in a
/// debug build, as the canvas paints.
pub fn raster_deadline() -> Duration {
    let factor = if cfg!(debug_assertions) {
        DEBUG_BUDGET_FACTOR
    } else {
        1
    };
    SOFT_RENDER_DEADLINE * factor
}

/// The raster helper: `tomakast-raster` next to the launcher's executable, unless
/// [`set_raster_helper`] named another.
#[cfg(not(test))]
static HELPER: LazyLock<Mutex<tomakast_raster::Client>> =
    LazyLock::new(|| Mutex::new(tomakast_raster::Client::new(raster_helper())));
static HELPER_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// The raster helper's executable: the one [`set_raster_helper`] named, else
/// `tomakast-raster` next to the launcher's executable.
pub fn raster_helper() -> PathBuf {
    let named = HELPER_PATH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    named.unwrap_or_else(|| {
        let name = format!("tomakast-raster{}", std::env::consts::EXE_SUFFIX);
        std::env::current_exe()
            .map(|exe| exe.with_file_name(&name))
            .unwrap_or_else(|_| PathBuf::from(name))
    })
}

/// Names the raster helper's executable; before the first image only. By default it is
/// `tomakast-raster` next to the launcher's executable.
pub fn set_raster_helper(program: PathBuf) {
    *HELPER_PATH
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(program);
}

/// Does `job` in the raster helper, starting `started` just before the job is written (and
/// clearing it while the job moves to a new helper); refused past `limit`. Unit tests do it
/// in their own process.
fn run_job(
    job: &Job,
    limit: Duration,
    started: &Started,
) -> Result<tomakast_raster::Pixels, String> {
    #[cfg(test)]
    {
        let _ = limit;
        // As a helper does before it says it is ready.
        tomakast_raster::svg::load_fonts();
        started.start();
        tomakast_raster::run(job)
    }
    #[cfg(not(test))]
    {
        HELPER
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .run(job, limit, |running| {
                if running {
                    started.start();
                } else {
                    started.stop();
                }
            })
    }
}

/// `pixels` from the helper as a GPUI image, if they are `expected` big (or, for a bitmap,
/// at most 1024 on a side).
fn render_image(
    pixels: tomakast_raster::Pixels,
    expected: Option<(u32, u32)>,
) -> Result<Done, String> {
    let size = (pixels.width, pixels.height);
    let fits = match expected {
        Some(expected) => size == expected,
        None => {
            let side = 1..=MAX_PLUGIN_IMAGE_SIDE;
            side.contains(&size.0) && side.contains(&size.1)
        }
    };
    if !fits {
        return Err(format!(
            "an image of {}x{} pixels from the helper",
            size.0, size.1
        ));
    }
    let mut buffer = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba)
        .ok_or("pixels of the wrong length from the helper")?;
    // RGBA to GPUI's BGRA.
    for pixel in buffer.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(Done {
        image: Arc::new(RenderImage::new([image::Frame::new(buffer)])),
    })
}

/// How often the UI thread looks for a job's answer while it runs.
pub const ANSWER_POLL: Duration = Duration::from_millis(4);

/// Image `id` of `provider` at `version`.
type ImageRef = (String, String, u64);

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    image: ImageRef,
    /// For an SVG, the raster's size and how the document fits it.
    raster: Option<(i32, i32, u8)>,
}

enum Slot {
    Pending { job: u64 },
    Ready(Arc<RenderImage>),
}

/// A plugin's job, queued or running.
struct Running {
    id: u64,
    key: Key,
    started: Started,
    /// Refused when it runs past [`stall_limit`].
    timed: bool,
    /// It ran past it: its image is refused, and the job is left to return.
    stalled: bool,
    /// It ran past [`STUCK_FACTOR`] times that: another raster thread took the jobs behind
    /// it.
    stuck: bool,
}

struct Entry {
    slot: Slot,
    /// Its pixels' bytes, counted from when it is asked for.
    bytes: usize,
    used: Instant,
}

/// What a job gives back.
pub(super) struct Done {
    image: Arc<RenderImage>,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<Key, Entry>,
    /// Images refused for good, at that version.
    refused: HashSet<ImageRef>,
    /// Images that did not fit the budget, logged once.
    crowded: HashSet<ImageRef>,
    /// The job each plugin has queued or running, stalled ones included.
    jobs: HashMap<String, Running>,
    /// Where the jobs write what they gave.
    inbox: Inbox,
    /// The tasks looking for the jobs' answers while the launcher shows, by job.
    polls: HashMap<u64, Task<()>>,
    next_job: u64,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

#[cfg(test)]
thread_local! {
    /// How many image headers [`bitmap`] has read.
    static HEADER_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many times a poll task has woken.
    static POLL_WAKES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many times a poll found something to draw.
    static REDRAWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many jobs were queued.
    static SUBMITTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// PNG or JPEG `image` of `provider` at `version`, decoded; `None` while it decodes, and for
/// an SVG or an image refused.
pub fn bitmap(
    provider: &str,
    image: &WidgetImage,
    version: u64,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    if image.format == ImageFormat::Svg {
        return None;
    }
    let key = Key {
        image: (provider.to_owned(), image.id.clone(), version),
        raster: None,
    };
    // The header alone, on a miss only, to count the pixels before anything decodes.
    let cost = || {
        #[cfg(test)]
        HEADER_READS.with(|reads| reads.set(reads.get() + 1));
        match plugin_image::dimensions(&image.bytes) {
            Ok((_, width, height)) => pixel_bytes(width, height),
            Err(_) => 0,
        }
    };
    let job = Job::Bitmap(image.bytes.clone());
    lookup(key, cost, false, cx, move |started| {
        render_image(run_job(&job, BITMAP_LIMIT, started)?, None)
    })
}

/// SVG `image` of `provider` at `version`, rasterized at `device` with `fit`; `None` while
/// it rasterizes, for an image refused, and for a raster of any other size or fit.
pub fn svg(
    provider: &str,
    image: &WidgetImage,
    version: u64,
    device: Size<DevicePixels>,
    fit: ImageFit,
    cx: &mut App,
) -> Option<Arc<RenderImage>> {
    if image.format != ImageFormat::Svg || device.width.0 < 1 || device.height.0 < 1 {
        return None;
    }
    let fit_tag = match fit {
        ImageFit::Contain => 0,
        ImageFit::Cover => 1,
        ImageFit::Fill => 2,
    };
    let key = Key {
        image: (provider.to_owned(), image.id.clone(), version),
        raster: Some((device.width.0, device.height.0, fit_tag)),
    };
    let cost = || pixel_bytes(device.width.0 as u32, device.height.0 as u32);
    let size = (device.width.0 as u32, device.height.0 as u32);
    let job = Job::Svg {
        bytes: image.bytes.clone(),
        device: size,
        fit,
    };
    lookup(key, cost, true, cx, move |started| {
        let pixels = run_job(&job, stall_limit(), started)?;
        let deadline = raster_deadline();
        if pixels.cpu > deadline {
            return Err(format!(
                "took {} ms to rasterize, more than {} ms",
                pixels.cpu.as_millis(),
                deadline.as_millis()
            ));
        }
        render_image(pixels, Some(size))
    })
}

fn pixel_bytes(width: u32, height: u32) -> usize {
    (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4)
}

/// How long an SVG job may run, in wall time from when it starts, before it is refused.
fn stall_limit() -> Duration {
    raster_deadline() * STALLED_FACTOR
}

/// The image of `key`: kept, or `None` while `work` makes it on a raster thread. `cost` is
/// its pixels' bytes, asked on a miss only; `timed` work is refused once it runs past
/// [`stall_limit`].
fn lookup(
    key: Key,
    cost: impl FnOnce() -> usize,
    timed: bool,
    cx: &mut App,
    work: impl FnOnce(&Started) -> Result<Done, String> + Send + 'static,
) -> Option<Arc<RenderImage>> {
    let now = Instant::now();
    let (found, job, released) = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        // What finished while nobody looked: since the last poll, or while hidden.
        cache.apply();
        let mut released = Vec::new();
        if cache.refused.contains(&key.image) {
            return (None, None, released);
        }
        if let Some(entry) = cache.entries.get_mut(&key) {
            entry.used = now;
            let (found, job) = match &entry.slot {
                Slot::Ready(image) => (Some(image.clone()), None),
                Slot::Pending { job } => (None, Some(*job)),
            };
            // Still decoding since before the launcher hid: looked for again.
            let poll = job.filter(|job| {
                !cache.polls.contains_key(job) && cache.jobs.values().any(|r| r.id == *job)
            });
            return (found, poll, released);
        }
        // One job at a time per plugin, until it returns, stalled or not. A job asked
        // before the launcher hid is looked for again (its poll too): its image may be
        // this one, kept as a new one would be.
        let mut adopted = None;
        if let Some(running) = cache.jobs.get(&key.image.0) {
            let (id, same) = (running.id, running.key == key);
            let poll = (!cache.polls.contains_key(&id)).then_some(id);
            if !same {
                return (None, poll, released);
            }
            adopted = Some((id, poll));
        }
        let unpolled = adopted.and_then(|(_, poll)| poll);
        let siblings = cache
            .entries
            .keys()
            .filter(|k| k.image == key.image)
            .count();
        if key.raster.is_some() && siblings >= MAX_SVG_SIZES {
            match cache.evict_least_used(now, |k| k.image == key.image) {
                Some(image) => released.extend(image),
                None => return (None, unpolled, released),
            }
        }
        let bytes = cost();
        if !cache.make_room(&key.image.0, bytes, now, &mut released) {
            if cache.crowded.insert(key.image.clone()) {
                tracing::warn!(
                    module = %key.image.0,
                    image = %key.image.1,
                    "widget image not shown for now: the image cache is full"
                );
            }
            return (None, unpolled, released);
        }
        if let Some((id, poll)) = adopted {
            cache.entries.insert(
                key.clone(),
                Entry {
                    slot: Slot::Pending { job: id },
                    bytes,
                    used: now,
                },
            );
            return (None, poll, released);
        }
        let id = cache.next_job;
        cache.next_job += 1;
        let limit = timed.then(stall_limit);
        let started = worker::submit(id, Box::new(work), limit, &cache.inbox);
        #[cfg(test)]
        SUBMITTED.with(|submitted| submitted.set(submitted.get() + 1));
        cache.jobs.insert(
            key.image.0.clone(),
            Running {
                id,
                key: key.clone(),
                started,
                timed,
                stalled: false,
                stuck: false,
            },
        );
        cache.entries.insert(
            key.clone(),
            Entry {
                slot: Slot::Pending { job: id },
                bytes,
                used: now,
            },
        );
        (None, Some(id), released)
    });
    release(released, cx);
    if let Some(id) = job {
        let poll = cx.spawn(async move |cx| poll(id, cx).await);
        CACHE.with(|cache| cache.borrow_mut().polls.insert(id, poll));
    }
    found
}

/// Looks every [`ANSWER_POLL`] for what job `id` gave, or for it stalling, while the
/// launcher shows ([`stop_polls`] drops this task), until it returns (stalled or not: its
/// plugin's next image waits for it); draws the launcher again when an image comes or
/// goes. Polled, not awaited: a GPUI task woken from a raster thread would be woken off
/// the UI thread, which GPUI's test scheduler refuses.
async fn poll(id: u64, cx: &mut AsyncApp) {
    loop {
        cx.background_executor().timer(ANSWER_POLL).await;
        let running = cx.update(|cx| {
            #[cfg(test)]
            POLL_WAKES.with(|wakes| wakes.set(wakes.get() + 1));
            let (changed, running) = CACHE.with(|cache| {
                let mut cache = cache.borrow_mut();
                let changed = cache.apply();
                let running = cache.jobs.values().any(|job| job.id == id);
                (changed, running)
            });
            if changed {
                #[cfg(test)]
                REDRAWS.with(|redraws| redraws.set(redraws.get() + 1));
                redraw_launcher(cx);
            }
            running
        });
        if !running {
            return;
        }
    }
}

impl Cache {
    /// Applies what the jobs gave, and refuses an image whose job runs past
    /// [`stall_limit`] (its plugin's slot stays taken until the job returns, and its poll
    /// goes on); whether an image came or went.
    fn apply(&mut self) -> bool {
        let mut changed = false;
        for (id, outcome) in self.inbox.take() {
            let Some(provider) = self
                .jobs
                .iter()
                .find(|(_, job)| job.id == id)
                .map(|(provider, _)| provider.clone())
            else {
                continue;
            };
            let Some(job) = self.jobs.remove(&provider) else {
                continue;
            };
            if let Some(poll) = self.polls.remove(&id) {
                // Ends on its own: it may be the task running this.
                poll.detach();
            }
            // Its plugin's next image may start now, if one waits.
            changed = true;
            match outcome {
                Ok(done) => {
                    if let Some(entry) = self.entries.get_mut(&job.key)
                        && matches!(entry.slot, Slot::Pending { job } if job == id)
                    {
                        entry.slot = Slot::Ready(done.image);
                    }
                }
                Err(problem) => self.refuse(&job.key, &problem),
            }
        }
        let limit = stall_limit();
        let stalled: Vec<Key> = self
            .jobs
            .values_mut()
            .filter(|job| {
                job.timed
                    && !job.stalled
                    && job.started.running().is_some_and(|running| running > limit)
            })
            .map(|job| {
                job.stalled = true;
                job.key.clone()
            })
            .collect();
        for key in stalled {
            let problem = format!("still rasterizing after {} ms", limit.as_millis());
            self.refuse(&key, &problem);
            changed = true;
        }
        for job in self.jobs.values_mut() {
            let stuck = job
                .started
                .running()
                .is_some_and(|running| running > limit * STUCK_FACTOR);
            if job.stalled && !job.stuck && stuck {
                job.stuck = true;
                worker::stuck();
            }
        }
        changed
    }

    /// Refuses `key`'s image from now on, logged once, whether or not it is still in the
    /// cache (the launcher may have hidden meanwhile).
    fn refuse(&mut self, key: &Key, problem: &str) {
        if matches!(
            self.entries.get(key),
            Some(Entry {
                slot: Slot::Pending { .. },
                ..
            })
        ) {
            self.entries.remove(key);
        }
        if self.refused.insert(key.image.clone()) {
            let (module, image, _) = &key.image;
            tracing::warn!(%module, %image, "widget image not shown: {problem}");
        }
    }

    /// Forgets every image `stale` names; returns the rasters to drop.
    fn forget(&mut self, stale: impl Fn(&ImageRef) -> bool) -> Vec<Arc<RenderImage>> {
        let mut released = Vec::new();
        self.entries.retain(|key, entry| {
            let keep = !stale(&key.image);
            if !keep && let Slot::Ready(image) = &entry.slot {
                released.push(image.clone());
            }
            keep
        });
        self.refused.retain(|image| !stale(image));
        self.crowded.retain(|image| !stale(image));
        released
    }

    /// Evicts the raster of `which` drawn longest ago, if not within [`IN_USE`]: `Some` with
    /// it, or `None` when every one is in use (or pending).
    fn evict_least_used(
        &mut self,
        now: Instant,
        which: impl Fn(&Key) -> bool,
    ) -> Option<Option<Arc<RenderImage>>> {
        let key = self
            .entries
            .iter()
            .filter(|(key, entry)| {
                which(key)
                    && matches!(entry.slot, Slot::Ready(_))
                    && now.duration_since(entry.used) >= IN_USE
            })
            .min_by_key(|(_, entry)| entry.used)
            .map(|(key, _)| key.clone())?;
        let entry = self.entries.remove(&key)?;
        Some(match entry.slot {
            Slot::Ready(image) => Some(image),
            Slot::Pending { .. } => None,
        })
    }

    /// Makes room for `bytes` more of `provider`'s, adding the rasters it evicts to
    /// `released`; whether it could.
    fn make_room(
        &mut self,
        provider: &str,
        bytes: usize,
        now: Instant,
        released: &mut Vec<Arc<RenderImage>>,
    ) -> bool {
        if bytes > MAX_CACHED_BYTES_PER_PLUGIN {
            return false;
        }
        loop {
            let own: usize = self
                .entries
                .iter()
                .filter(|(key, _)| key.image.0 == provider)
                .map(|(_, entry)| entry.bytes)
                .sum();
            let all: usize = self.entries.values().map(|entry| entry.bytes).sum();
            let evicted = if own + bytes > MAX_CACHED_BYTES_PER_PLUGIN {
                self.evict_least_used(now, |key| key.image.0 == provider)
            } else if all + bytes > MAX_CACHED_BYTES {
                self.evict_least_used(now, |_| true)
            } else {
                return true;
            };
            match evicted {
                Some(image) => released.extend(image),
                None => return false,
            }
        }
    }
}

/// Removes `released` from every window's sprite atlas, once the window drawing now is done.
fn release(released: Vec<Arc<RenderImage>>, cx: &mut App) {
    if released.is_empty() {
        return;
    }
    cx.defer(move |cx| {
        for image in released {
            cx.drop_image(image, None);
        }
    });
}

/// Draws the launcher window again, if it is open.
fn redraw_launcher(cx: &mut App) {
    let Some(target) = cx
        .try_global::<LauncherController>()
        .and_then(|launcher| launcher.window())
    else {
        return;
    };
    // An error means the window closed meanwhile.
    let _ = target.handle.update(cx, |_, window, _| window.refresh());
}

/// Forgets the images whose version `current` no longer gives (replaced, or their plugin
/// gone), and drops their rasters.
pub(crate) fn sweep(current: impl Fn(&str, &str) -> Option<u64>, cx: &mut App) {
    let released = CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.forget(|(provider, id, version)| current(provider, id) != Some(*version))
    });
    release(released, cx);
}

/// Stops looking for the jobs' answers: the launcher hid, and nothing draws its images
/// until it shows again. The decoded images stay (the cache's caps bound them), so the next
/// showing draws them at once. The jobs still running keep their plugins' slots, and what
/// they give is applied on the next look, after the next show, which looks again for an
/// image still decoding.
pub(crate) fn stop_polls() {
    let polls = CACHE.with(|cache| std::mem::take(&mut cache.borrow_mut().polls));
    // Dropped outside the borrow: a dropped task is cancelled.
    drop(polls);
}

/// The images the frames of the tiles drawn now name, with their versions, as the core had
/// them at the last tick.
#[derive(Clone, Default)]
pub(crate) struct FrameImages(Rc<ByProvider>);

/// Images with their versions, by provider, then by id.
type ByProvider = HashMap<String, HashMap<String, (u64, WidgetImage)>>;

impl FrameImages {
    /// The images `frames` name, by their tile's provider.
    pub(crate) fn of<'a>(
        core: &Core,
        frames: impl IntoIterator<Item = (&'a WidgetKey, &'a Frame)>,
    ) -> FrameImages {
        let mut images = ByProvider::new();
        for (key, frame) in frames {
            let provider = images.entry(key.provider.clone()).or_default();
            let named = frame.nodes.iter().flat_map(|node| {
                let (image, commands) = match &node.kind {
                    NodeKind::Image(image) => (Some(image.image.as_str()), &[][..]),
                    NodeKind::Canvas(canvas) => (None, canvas.commands.as_slice()),
                    _ => (None, &[][..]),
                };
                image
                    .into_iter()
                    .chain(commands.iter().filter_map(|command| match command {
                        Draw::Image(image) | Draw::Svg(image) => Some(image.image.as_str()),
                        _ => None,
                    }))
            });
            for id in named {
                if provider.contains_key(id) {
                    continue;
                }
                if let Some(image) = core.widget_image(&key.provider, id) {
                    provider.insert(id.to_owned(), image);
                }
            }
        }
        FrameImages(Rc::new(images))
    }

    /// Image `id` of `provider`, with its version.
    pub(crate) fn get(&self, provider: &str, id: &str) -> Option<&(u64, WidgetImage)> {
        self.0.get(provider)?.get(id)
    }
}

/// The largest box an SVG is rasterized for: a double tile.
pub(crate) fn max_raster() -> Size<Pixels> {
    size(
        Pixels::from(super::layout::tile_width(
            WidgetSize::Double,
            super::VIEWPORT,
        )),
        Pixels::from(crate::theme::STRIP_TILE_HEIGHT),
    )
}

/// The device size to rasterize an SVG at that shows `logical` big at `scale`: never more
/// than `cap` (and [`max_raster`]) at `scale`, shrunk evenly to fit; at least a pixel. `None`
/// for an empty or unbounded size.
pub(crate) fn device_size(
    logical: Size<Pixels>,
    cap: Size<Pixels>,
    scale: f32,
) -> Option<Size<DevicePixels>> {
    let max = max_raster();
    let (width, height) = (
        f32::from(logical.width) * scale,
        f32::from(logical.height) * scale,
    );
    let (max_width, max_height) = (
        f32::from(cap.width.min(max.width)) * scale,
        f32::from(cap.height.min(max.height)) * scale,
    );
    if !(width.is_finite() && height.is_finite() && width > 0. && height > 0.) {
        return None;
    }
    let shrink = (max_width / width).min(max_height / height).clamp(0., 1.);
    let side = |v: f32| DevicePixels((v * shrink).ceil().max(1.) as i32);
    Some(size(side(width), side(height)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::TestAppContext;
    use std::io::Cursor;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]))
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn device(width: i32, height: i32) -> Size<DevicePixels> {
        size(DevicePixels(width), DevicePixels(height))
    }

    const DOC: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 71 24"><circle cx="12" cy="12" r="10" fill="red"/></svg>"#;

    fn image(format: ImageFormat, bytes: &[u8]) -> WidgetImage {
        WidgetImage {
            id: "doc".into(),
            format,
            bytes: Arc::from(bytes),
        }
    }

    /// Runs the app until `done` gives something, the raster threads being real; panics
    /// after 5 s.
    fn until<T>(
        cx: &mut TestAppContext,
        mut done: impl FnMut(&mut TestAppContext) -> Option<T>,
    ) -> T {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            cx.executor().advance_clock(ANSWER_POLL);
            cx.run_until_parked();
            if let Some(found) = done(cx) {
                return found;
            }
            assert!(Instant::now() < deadline, "not done within 5 s");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn refused(provider: &str) -> bool {
        CACHE.with(|cache| {
            cache
                .borrow()
                .refused
                .iter()
                .any(|image| image.0 == provider)
        })
    }

    fn key(provider: &str) -> Key {
        Key {
            image: (provider.into(), "doc".into(), 1),
            raster: None,
        }
    }

    fn done(started: &Started) -> Result<Done, String> {
        started.start();
        let pixel = tomakast_raster::Pixels {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
            cpu: Duration::ZERO,
        };
        render_image(pixel, None)
    }

    /// At a new scale the document is rasterized again, and until then no raster of the
    /// old size stands in for it.
    #[gpui_kit::test]
    fn svg_raster_is_keyed_by_device_size(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let doc = image(ImageFormat::Svg, DOC.as_bytes());
        let raster = |cx: &mut TestAppContext, d| {
            cx.update(|cx| svg("keyed", &doc, 1, d, ImageFit::Fill, cx))
        };
        let (small, large) = (device(710, 240), device(1065, 360));

        assert!(raster(cx, small).is_none(), "rasterized off the UI thread");
        let first = until(cx, |cx| raster(cx, small));
        assert_eq!(first.size(0), small);

        assert!(
            raster(cx, large).is_none(),
            "the scale-2 raster is not stretched"
        );
        let second = until(cx, |cx| raster(cx, large));
        assert_eq!(second.size(0), large);
        assert_ne!(first.id, second.id);
    }

    /// Drawing a decoded image reads nothing of its bytes: the header is read on a miss.
    #[gpui_kit::test]
    fn a_hit_reads_no_header(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let png = image(ImageFormat::Png, &png(4, 4));
        let draw = |cx: &mut TestAppContext| cx.update(|cx| bitmap("hits", &png, 1, cx));
        assert!(draw(cx).is_none());
        until(cx, draw);
        let reads = HEADER_READS.with(|reads| reads.get());

        for _ in 0..100 {
            assert!(draw(cx).is_some());
        }

        assert_eq!(HEADER_READS.with(|reads| reads.get()), reads);
    }

    #[test]
    fn device_sizes_stay_within_the_cap() {
        let at = |w: f32, h: f32| size(Pixels::from(w), Pixels::from(h));
        let tile = at(355., 120.);
        assert_eq!(
            device_size(at(355., 120.), tile, 2.),
            Some(device(710, 240))
        );
        // Shrunk evenly to the cap's height.
        assert_eq!(
            device_size(at(100., 1200.), tile, 1.),
            Some(device(10, 120))
        );
        // Never past a double tile, whatever the cap.
        assert_eq!(
            device_size(at(4000., 120.), at(4000., 120.), 1.),
            Some(device(718, 22))
        );
        assert_eq!(device_size(at(0., 10.), tile, 1.), None);
        assert_eq!(device_size(at(f32::INFINITY, 10.), tile, 1.), None);
    }

    fn versions() -> Vec<u64> {
        CACHE.with(|cache| {
            let mut versions: Vec<u64> = cache
                .borrow()
                .entries
                .keys()
                .map(|key| key.image.2)
                .collect();
            versions.sort();
            versions
        })
    }

    /// The sweep of each tick forgets a version the core replaced, and every image of a
    /// plugin gone; hiding the launcher keeps them, so the next showing draws them at once.
    #[gpui_kit::test]
    fn replaced_images_drop_their_rasters_and_a_hidden_launcher_keeps_them(
        cx: &mut TestAppContext,
    ) {
        let _pool = pool_lock();
        let doc = image(ImageFormat::Svg, DOC.as_bytes());
        let at = device(71, 24);
        let raster = |cx: &mut TestAppContext, v| {
            cx.update(|cx| svg("swept", &doc, v, at, ImageFit::Fill, cx))
        };
        until(cx, |cx| raster(cx, 1));
        until(cx, |cx| raster(cx, 2));
        assert_eq!(versions(), [1, 2]);

        cx.update(|cx| sweep(|_, _| Some(2), cx));
        assert_eq!(versions(), [2]);

        stop_polls();
        assert_eq!(versions(), [2]);
        let submitted = SUBMITTED.with(|n| n.get());
        assert!(raster(cx, 2).is_some(), "drawn at once on the next show");
        assert_eq!(SUBMITTED.with(|n| n.get()), submitted, "not decoded again");

        cx.update(|cx| sweep(|_, _| None, cx));
        assert!(versions().is_empty());
    }

    /// Work that panics refuses its image; the raster thread goes on.
    #[gpui_kit::test]
    fn a_panic_in_the_work_is_a_refusal(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let panics = |_: &Started| -> Result<Done, String> { panic!("a hostile document") };
        assert!(
            cx.update(|cx| lookup(key("panics"), || 0, false, cx, panics))
                .is_none()
        );
        until(cx, |_| refused("panics").then_some(()));

        let fine = until(cx, |cx| {
            cx.update(|cx| lookup(key("after"), || 0, false, cx, done))
        });
        assert_eq!(fine.size(0), device(1, 1));
    }

    /// The raster threads are shared by every test of this process: the tests that submit
    /// jobs run one at a time, so a job held at its gate never holds up another test's.
    static POOL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn pool_lock() -> std::sync::MutexGuard<'static, ()> {
        POOL.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The gate of a [`gated`] job: dropping it (at the end of the test, or as a failing
    /// test unwinds) lets the job return.
    type Gate = std::sync::mpsc::Sender<()>;

    /// Work that runs until its gate opens, and the gate.
    fn gated() -> (
        Gate,
        impl FnOnce(&Started) -> Result<Done, String> + Send + 'static,
    ) {
        let (gate, shut) = std::sync::mpsc::channel::<()>();
        let work = move |started: &Started| {
            started.start();
            // Nothing is ever sent: it returns when the gate goes.
            let _ = shut.recv();
            done(started)
        };
        (gate, work)
    }

    /// Whether job `id`'s outcome waits in the inbox.
    fn in_inbox(id: u64) -> bool {
        CACHE.with(|cache| cache.borrow().inbox.holds(id))
    }

    fn running_job(provider: &str) -> Option<u64> {
        CACHE.with(|cache| cache.borrow().jobs.get(provider).map(|job| job.id))
    }

    /// A raster still running past twice its deadline is refused by the poll, with no
    /// other look, and another thread takes the jobs behind it.
    #[gpui_kit::test]
    fn a_stalled_raster_is_refused(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let (gate, slow) = gated();
        assert!(
            cx.update(|cx| lookup(key("slow"), || 0, true, cx, slow))
                .is_none()
        );
        let held = running_job("slow").expect("the slow job");
        until(cx, |_| refused("slow").then_some(()));

        until(cx, |cx| {
            cx.update(|cx| lookup(key("behind"), || 0, false, cx, done))
        });
        // Done while the slow job is still held: another thread took it.
        assert_eq!(
            running_job("slow"),
            Some(held),
            "the slow job has not returned"
        );
        assert!(!in_inbox(held));

        drop(gate);
        until(cx, |_| running_job("slow").is_none().then_some(()));
    }

    /// A plugin whose job stalled starts no other until that one returns: one plugin holds
    /// one raster thread at most.
    #[gpui_kit::test]
    fn a_plugin_with_a_stalled_job_starts_no_other(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let other = Key {
            image: ("pinned".into(), "other".into(), 1),
            raster: None,
        };
        let (gate, slow) = gated();
        cx.update(|cx| lookup(key("pinned"), || 0, true, cx, slow));
        let slow = running_job("pinned");
        until(cx, |_| refused("pinned").then_some(()));

        assert!(
            cx.update(|cx| lookup(other.clone(), || 0, false, cx, done))
                .is_none()
        );
        assert_eq!(running_job("pinned"), slow, "still the stalled job's slot");

        // Once it returns, the plugin's next job runs.
        drop(gate);
        let fine = until(cx, |cx| {
            cx.update(|cx| lookup(other.clone(), || 0, false, cx, done))
        });
        assert_eq!(fine.size(0), device(1, 1));
    }

    /// The launcher hid while a job ran past its limit: on the next show, it is refused,
    /// and not queued again.
    #[gpui_kit::test]
    fn an_overrun_while_hidden_is_refused_on_the_next_show(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let stall = stall_limit();
        let (gate, slow) = gated();
        cx.update(|cx| lookup(key("hidden"), || 0, true, cx, slow));
        stop_polls();
        // Nothing looks while hidden; the job runs past its limit.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !CACHE.with(|cache| {
            let cache = cache.borrow();
            let running = cache
                .jobs
                .get("hidden")
                .and_then(|job| job.started.running());
            running.is_some_and(|running| running > stall)
        }) {
            assert!(Instant::now() < deadline, "the job ran within 5 s");
            std::thread::sleep(Duration::from_millis(1));
        }
        let next = CACHE.with(|cache| cache.borrow().next_job);

        // The next show looks again.
        assert!(
            cx.update(|cx| lookup(key("hidden"), || 0, true, cx, done))
                .is_none()
        );

        assert!(refused("hidden"));
        assert_eq!(CACHE.with(|cache| cache.borrow().next_job), next);
        // Its job returns, late: the image stays refused.
        drop(gate);
        until(cx, |cx| {
            cx.update(|cx| lookup(key("hidden"), || 0, true, cx, done));
            running_job("hidden").is_none().then_some(())
        });
        assert!(refused("hidden"));
        assert_eq!(CACHE.with(|cache| cache.borrow().next_job), next);
    }

    /// Hiding stops the polls: nothing wakes the UI thread for the jobs still running.
    #[gpui_kit::test]
    fn no_poll_wakes_while_hidden(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let wakes = || POLL_WAKES.with(|wakes| wakes.get());
        let (gate, slow) = gated();
        cx.update(|cx| lookup(key("asleep"), || 0, false, cx, slow));
        for _ in 0..3 {
            cx.executor().advance_clock(ANSWER_POLL);
            cx.run_until_parked();
        }
        assert!(wakes() > 0, "it polls while shown");

        stop_polls();
        let hidden = wakes();
        for _ in 0..100 {
            cx.executor().advance_clock(ANSWER_POLL);
            cx.run_until_parked();
        }

        assert_eq!(wakes(), hidden);
        drop(gate);
        until(cx, |cx| {
            cx.update(|cx| lookup(key("asleep"), || 0, false, cx, done))
        });
    }

    /// The launcher hid while an image's job ran, and showed again with the image on
    /// screen: once the job returns, the window is drawn again, with the image, and no job
    /// is queued twice.
    #[gpui_kit::test]
    fn a_job_that_returns_after_hide_and_show_draws(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let (gate, slow) = gated();
        cx.update(|cx| lookup(key("reshown"), || 0, false, cx, slow));
        stop_polls();
        let submitted = SUBMITTED.with(|n| n.get());
        let redraws = REDRAWS.with(|n| n.get());

        // Shown again: the image is asked for while its job still runs.
        assert!(
            cx.update(|cx| lookup(key("reshown"), || 0, false, cx, done))
                .is_none()
        );
        drop(gate);
        until(cx, |_| (REDRAWS.with(|n| n.get()) > redraws).then_some(()));

        let image = cx.update(|cx| lookup(key("reshown"), || 0, false, cx, done));
        assert!(image.is_some(), "the job's own image");
        assert_eq!(SUBMITTED.with(|n| n.get()), submitted);
    }

    /// A job refused as stalled whose answer comes just after (the helper stopped at the
    /// same limit): its plugin's slot is freed and the window drawn again with no other
    /// look, and no other raster thread starts for it.
    #[gpui_kit::test]
    fn a_stalled_job_that_returns_frees_its_slot_with_no_other_look(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let threads = worker::threads();
        let (gate, slow) = gated();
        cx.update(|cx| lookup(key("freed"), || 0, true, cx, slow));
        until(cx, |_| refused("freed").then_some(()));
        let redraws = REDRAWS.with(|n| n.get());

        // It returns well before it would count as stuck.
        drop(gate);

        until(cx, |_| running_job("freed").is_none().then_some(()));

        assert!(REDRAWS.with(|n| n.get()) > redraws, "drawn again");
        assert_eq!(worker::threads(), threads, "no other raster thread");
    }

    /// An image asked for again while its job still runs, after its entry left the cache
    /// (made room for others), counts against the cache as a new one would: past the
    /// plugin's budget it is not kept.
    #[gpui_kit::test]
    fn an_image_asked_again_while_its_job_runs_counts_against_the_cache(cx: &mut TestAppContext) {
        let _pool = pool_lock();
        let (gate, slow) = gated();
        cx.update(|cx| lookup(key("readopted"), || 0, false, cx, slow));
        stop_polls();
        CACHE.with(|cache| cache.borrow_mut().entries.remove(&key("readopted")));

        let too_big = MAX_CACHED_BYTES_PER_PLUGIN + 1;
        cx.update(|cx| lookup(key("readopted"), || too_big, false, cx, done));

        let kept = CACHE.with(|cache| cache.borrow().entries.contains_key(&key("readopted")));
        assert!(!kept, "an image past the budget is not kept");
        // Its job still frees the slot when it returns.
        drop(gate);
        until(cx, |_| running_job("readopted").is_none().then_some(()));
    }
}
