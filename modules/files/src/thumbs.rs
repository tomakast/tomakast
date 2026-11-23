//! Thumbnails of the grid: read from the freedesktop cache other apps share
//! (`$XDG_CACHE_HOME/thumbnails/{large,normal}`, named by the MD5 of the file's URI, valid
//! while their `Thumb::MTime` is the file's modification time), and made in the background
//! for images that have none, into `large` (256 px), so other apps reuse them (Linux).
//!
//! Elsewhere there are no thumbnails yet: `cached` finds none and `request` does nothing.
//!
//! The file's bytes may come from anywhere (a download), so they are decoded and shrunk in
//! the `tomakast-raster` helper process, under its memory and CPU limits, never in the
//! daemon: a decoder that panics, hangs or runs out of memory takes only the helper down.
//! The daemon reads the file and writes the small result as a PNG.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Once, PoisonError};
use std::time::{Duration, Instant};

/// At most this many requests wait; more are dropped (the screen never shows more rows).
const QUEUE_CAP: usize = crate::screen::LIMIT;
/// Finished thumbnails are reported at most this often, together.
const BATCH_EVERY: Duration = Duration::from_millis(250);
/// Files asked for are remembered (so a failure is not tried again on every keystroke) up to
/// this many; then the memory starts afresh.
const TRIED_CAP: usize = 10_000;
/// Images over this many bytes on disk get no thumbnail.
pub const MAX_BYTES: u64 = tomakast_raster::protocol::MAX_THUMBNAIL_BYTES as u64;
/// The edge of a `large` thumbnail.
const SIZE: u32 = 256;
/// How long the helper may take for one thumbnail.
const JOB_LIMIT: Duration = Duration::from_secs(5);

/// Decodes an image file's bytes into the pixels of its thumbnail, at most [`SIZE`] on a
/// side.
pub(crate) trait Decode: Send {
    fn thumbnail(&mut self, bytes: Arc<[u8]>) -> Result<tomakast_raster::Pixels, String>;
}

/// In the `tomakast-raster` helper next to the executable, started on the first thumbnail.
struct InHelper(tomakast_raster::Client);

impl Decode for InHelper {
    fn thumbnail(&mut self, bytes: Arc<[u8]>) -> Result<tomakast_raster::Pixels, String> {
        let job = tomakast_raster::Job::Thumbnail {
            bytes,
            max_side: SIZE,
        };
        self.0.run(&job, JOB_LIMIT, |_| {})
    }
}

/// In this process, for tests: no helper binary is needed.
#[cfg(test)]
pub(crate) struct InProcess;

#[cfg(test)]
impl Decode for InProcess {
    fn thumbnail(&mut self, bytes: Arc<[u8]>) -> Result<tomakast_raster::Pixels, String> {
        tomakast_raster::run(&tomakast_raster::Job::Thumbnail {
            bytes,
            max_side: SIZE,
        })
    }
}

/// The helper process; in unit tests, this process.
fn default_decoder() -> Box<dyn Decode> {
    #[cfg(test)]
    {
        Box::new(InProcess)
    }
    #[cfg(not(test))]
    {
        Box::new(InHelper(tomakast_raster::Client::new(
            tomakast_raster::helper_next_to_executable(),
        )))
    }
}

/// The cache file name of `path`: the MD5 of its URI, in hex, and `.png`.
pub fn thumbnail_name(path: &Path) -> String {
    use md5::{Digest, Md5};
    format!(
        "{:x}.png",
        Md5::digest(crate::names::file_uri(path).as_bytes())
    )
}

/// An image type the module makes thumbnails of, by the name's extension: PNG, JPEG, WebP,
/// GIF. Others (videos, PDFs) only get one another app made.
fn supported(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    ["png", "jpg", "jpeg", "webp", "gif"]
        .iter()
        .any(|known| ext.eq_ignore_ascii_case(known))
}

/// A thumbnail cache rooted at `$XDG_CACHE_HOME/thumbnails`, with one background thread
/// (started on the first request) that makes missing thumbnails of images. Dropping it stops
/// the thread after the thumbnail in hand.
pub struct Thumbs {
    root: PathBuf,
    shared: Arc<Shared>,
    batch_every: Duration,
    /// Handed to the thread when it starts.
    decoder: Mutex<Option<Box<dyn Decode>>>,
    thread: Once,
}

type Done = Box<dyn FnOnce() + Send>;

struct Job {
    path: PathBuf,
    mtime: i64,
    done: Done,
}

struct Shared {
    queue: Mutex<Queue>,
    wake: Condvar,
}

#[derive(Default)]
struct Queue {
    jobs: VecDeque<Job>,
    /// Asked for and not cancelled: waiting, in hand, made or failed.
    tried: HashSet<(PathBuf, i64)>,
    stop: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Thumbs {
    pub fn new(cache_root: PathBuf) -> Thumbs {
        Thumbs::with_batch_every(cache_root, BATCH_EVERY)
    }

    /// `new` with how often finished thumbnails are reported.
    pub(crate) fn with_batch_every(cache_root: PathBuf, every: Duration) -> Thumbs {
        Thumbs::with_decoder(cache_root, every, default_decoder())
    }

    /// `new` with how often finished thumbnails are reported, and how images decode.
    pub(crate) fn with_decoder(
        cache_root: PathBuf,
        every: Duration,
        decoder: Box<dyn Decode>,
    ) -> Thumbs {
        Thumbs {
            root: cache_root,
            shared: Arc::new(Shared {
                queue: Mutex::new(Queue::default()),
                wake: Condvar::new(),
            }),
            batch_every: every,
            decoder: Mutex::new(Some(decoder)),
            thread: Once::new(),
        }
    }

    /// A valid cached thumbnail (large first, then normal). `mtime` 0 (unknown) never
    /// matches.
    pub fn cached(&self, path: &Path, mtime: i64) -> Option<PathBuf> {
        cached_in(&self.root, path, mtime)
    }

    /// Queues generation for a supported image without one; `done` runs on the queue thread
    /// once it is made, batched with the others finished in the same 250 ms. Nothing is
    /// queued for another type, an unknown `mtime` (0), a full queue, or a file already
    /// asked for at this `mtime` (waiting, made or failed): a failure is not tried again.
    pub fn request(&self, path: &Path, mtime: i64, done: impl FnOnce() + Send + 'static) {
        // Under the cache itself (indexed when hidden folders are): no thumbnails of
        // thumbnails.
        if !cfg!(target_os = "linux")
            || mtime == 0
            || !supported(path)
            || path.starts_with(&self.root)
        {
            return;
        }
        {
            let mut queue = self.shared.lock();
            let key = (path.to_path_buf(), mtime);
            if queue.stop || queue.jobs.len() >= QUEUE_CAP || queue.tried.contains(&key) {
                return;
            }
            if queue.tried.len() >= TRIED_CAP {
                let waiting = queue
                    .jobs
                    .iter()
                    .map(|j| (j.path.clone(), j.mtime))
                    .collect();
                queue.tried = waiting;
            }
            queue.tried.insert(key);
            queue.jobs.push_back(Job {
                path: path.to_path_buf(),
                mtime,
                done: Box::new(done),
            });
        }
        self.start();
        self.shared.wake.notify_one();
    }

    /// Drops the requests not started yet (the rows they were for are gone); they may be
    /// asked for again.
    pub fn cancel_pending(&self) {
        let mut queue = self.shared.lock();
        let dropped: Vec<Job> = queue.jobs.drain(..).collect();
        for job in &dropped {
            queue.tried.remove(&(job.path.clone(), job.mtime));
        }
        drop(queue);
        // The callbacks are dropped off the lock.
        drop(dropped);
    }

    /// Files asked for and not cancelled.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn requested(&self) -> usize {
        self.shared.lock().tried.len()
    }

    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn queued(&self) -> usize {
        self.shared.lock().jobs.len()
    }

    fn start(&self) {
        self.thread.call_once(|| {
            let shared = self.shared.clone();
            let root = self.root.clone();
            let every = self.batch_every;
            let taken = self
                .decoder
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take();
            let Some(mut decoder) = taken else {
                return;
            };
            let spawned = std::thread::Builder::new()
                .name("files-thumbs".into())
                .spawn(move || run(&shared, &root, every, decoder.as_mut()));
            if let Err(err) = spawned {
                tracing::warn!(%err, "files: cannot start the thumbnail thread; no thumbnails");
            }
        });
    }
}

impl Drop for Thumbs {
    fn drop(&mut self) {
        let mut queue = self.shared.lock();
        queue.stop = true;
        queue.jobs.clear();
        drop(queue);
        self.shared.wake.notify_all();
    }
}

/// The thread: makes the queued thumbnails one by one and runs the `done` of those finished
/// at most once per `every`, all together; the last batch waits only for the rest of
/// `every`.
fn run(shared: &Shared, root: &Path, every: Duration, decoder: &mut dyn Decode) {
    let mut finished: Vec<Done> = Vec::new();
    let mut flushed: Option<Instant> = None;
    loop {
        if !finished.is_empty() && flushed.is_none_or(|at| at.elapsed() >= every) {
            for done in finished.drain(..) {
                done();
            }
            flushed = Some(Instant::now());
        }
        let job = {
            let mut queue = shared.lock();
            loop {
                if queue.stop {
                    return;
                }
                if let Some(job) = queue.jobs.pop_front() {
                    break Some(job);
                }
                // Something finished is waiting for its batch: sleep only until it is due.
                let due = flushed
                    .filter(|_| !finished.is_empty())
                    .map(|at| at + every);
                match due {
                    None => {
                        queue = shared
                            .wake
                            .wait(queue)
                            .unwrap_or_else(PoisonError::into_inner);
                    }
                    Some(due) => {
                        let now = Instant::now();
                        if now >= due {
                            break None;
                        }
                        queue = shared
                            .wake
                            .wait_timeout(queue, due - now)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                    }
                }
            }
        };
        let Some(job) = job else {
            continue;
        };
        if make(root, &job.path, job.mtime, decoder) {
            finished.push(job.done);
        }
    }
}

/// Makes the thumbnail of `path` unless a valid one is there already; whether there is one
/// now. A failure (the helper refused, stopped or ran out of time) is not tried again: the
/// file stays in the queue's `tried`.
fn make(root: &Path, path: &Path, mtime: i64, decoder: &mut dyn Decode) -> bool {
    if cached_in(root, path, mtime).is_some() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        let out = root.join("large").join(thumbnail_name(path));
        match generate(path, mtime, &out, decoder) {
            Ok(()) => true,
            Err(err) => {
                tracing::debug!(%err, path = %path.display(), "files: no thumbnail");
                false
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = decoder;
        false
    }
}

/// The valid thumbnail of `path` under `root`: `large`, else `normal`.
fn cached_in(root: &Path, path: &Path, mtime: i64) -> Option<PathBuf> {
    if !cfg!(target_os = "linux") || mtime == 0 {
        return None;
    }
    let name = thumbnail_name(path);
    ["large", "normal"]
        .iter()
        .map(|size| root.join(size).join(&name))
        .find(|thumb| stored_mtime(thumb) == Some(mtime))
}

/// The `Thumb::MTime` of the PNG at `thumb`, read from its text chunks (`tEXt`, or an
/// uncompressed `iTXt`); the image data is skipped, not read.
fn stored_mtime(thumb: &Path) -> Option<i64> {
    use std::io::{BufReader, Read};
    /// Text chunks longer than this are not the modification time.
    const MAX_TEXT: u32 = 4096;
    /// Thumbnails have a handful of chunks; a file with more is not looked through.
    const MAX_CHUNKS: usize = 1000;

    let mut file = BufReader::new(std::fs::File::open(thumb).ok()?);
    let mut signature = [0u8; 8];
    file.read_exact(&mut signature).ok()?;
    if signature != *b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    for _ in 0..MAX_CHUNKS {
        let mut header = [0u8; 8];
        file.read_exact(&mut header).ok()?;
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let kind = &header[4..8];
        if kind == b"IEND" {
            return None;
        }
        if (kind == b"tEXt" || kind == b"iTXt") && len <= MAX_TEXT {
            let mut data = vec![0u8; len as usize];
            file.read_exact(&mut data).ok()?;
            if let Some(value) = text_value(kind == b"iTXt", &data, b"Thumb::MTime") {
                return std::str::from_utf8(value).ok()?.trim().parse().ok();
            }
        } else {
            file.seek_relative(i64::from(len)).ok()?;
        }
        // The CRC.
        file.seek_relative(4).ok()?;
    }
    None
}

/// The text of a `tEXt` (or uncompressed `iTXt`) chunk's `data` when its keyword is `key`.
fn text_value<'a>(itxt: bool, data: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let at = data.iter().position(|&b| b == 0)?;
    if &data[..at] != key {
        return None;
    }
    let rest = &data[at + 1..];
    if !itxt {
        return Some(rest);
    }
    // Compression flag, method, then the language tag and the translated keyword.
    let (&compressed, rest) = rest.split_first()?;
    if compressed != 0 {
        return None;
    }
    let rest = rest.get(1..)?;
    let lang = rest.iter().position(|&b| b == 0)?;
    let rest = &rest[lang + 1..];
    let translated = rest.iter().position(|&b| b == 0)?;
    Some(&rest[translated + 1..])
}

/// Reads the image at `path`, has `decoder` shrink it to fit 256 px, and writes the result
/// to `out` as a PNG with `Thumb::URI` and `Thumb::MTime`, mode 0600, through a temp file
/// and a rename. Refuses anything but a regular file (a link is not followed), a file over
/// 50 MB, and one whose modification time is no longer `mtime`; the decoder refuses what is
/// not a PNG, JPEG, WebP or GIF, and images over 100 megapixels.
#[cfg(target_os = "linux")]
pub(crate) fn generate(
    path: &Path,
    mtime: i64,
    out: &Path,
    decoder: &mut dyn Decode,
) -> anyhow::Result<()> {
    let bytes = read_image_file(path, mtime)?;
    let pixels = decoder
        .thumbnail(bytes.into())
        .map_err(anyhow::Error::msg)?;
    let (w, h) = (pixels.width, pixels.height);
    anyhow::ensure!(
        (1..=SIZE).contains(&w)
            && (1..=SIZE).contains(&h)
            && pixels.rgba.len() as u64 == u64::from(w) * u64::from(h) * 4,
        "the decoder answered {w}x{h} pixels in {} bytes",
        pixels.rgba.len()
    );
    let png = encode(w, h, &pixels.rgba, &crate::names::file_uri(path), mtime)?;
    write_private(out, &png)
}

/// The bytes of the file at `path`, opened once: not through a link, without waiting (a
/// pipe would hold the thread), and only if it is a regular file of at most 50 MB modified
/// at `mtime`.
#[cfg(target_os = "linux")]
fn read_image_file(path: &Path, mtime: i64) -> anyhow::Result<Vec<u8>> {
    use anyhow::ensure;
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(meta.is_file(), "not a regular file");
    ensure!(meta.len() <= MAX_BYTES, "over 50 MB");
    ensure!(
        crate::index::mtime_secs(&meta) == mtime,
        "changed since it was indexed"
    );
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= MAX_BYTES, "over 50 MB");
    Ok(bytes)
}

/// `width` x `height` straight RGBA pixels as a PNG with the spec's text chunks.
#[cfg(target_os = "linux")]
fn encode(width: u32, height: u32, rgba: &[u8], uri: &str, mtime: i64) -> anyhow::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.add_text_chunk("Thumb::URI".to_string(), uri.to_string())?;
    encoder.add_text_chunk("Thumb::MTime".to_string(), mtime.to_string())?;
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    writer.finish()?;
    Ok(bytes)
}

/// Writes `bytes` to `out` as the spec asks: the folder 0700, the file 0600 from its
/// creation, a temp file in the same folder renamed over `out`.
#[cfg(target_os = "linux")]
fn write_private(out: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};
    static TEMP: AtomicU64 = AtomicU64::new(0);

    let dir = out
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no folder for {}", out.display()))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let name = out.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(
        ".{name}.tomakast-{}-{}",
        std::process::id(),
        TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(bytes)?;
        std::fs::rename(&tmp, out)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(written?)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use md5::{Digest, Md5};
    use tempfile::TempDir;

    use crate::index::mtime_secs;
    use crate::names::file_uri;

    const PATIENCE: Duration = Duration::from_secs(10);

    fn mtime_of(path: &Path) -> i64 {
        mtime_secs(&fs::symlink_metadata(path).unwrap())
    }

    /// A `w`x`h` PNG at `path`.
    fn image(path: &Path, w: u32, h: u32) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]))
            .save(path)
            .unwrap();
    }

    /// A PNG with the given text chunks at `path`, as another app would leave it.
    fn thumbnail(path: &Path, chunks: &[(&str, &str)]) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = fs::File::create(path).unwrap();
        let mut enc = png::Encoder::new(file, 2, 2);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        for (key, value) in chunks {
            enc.add_text_chunk(key.to_string(), value.to_string())
                .unwrap();
        }
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&[0u8; 16]).unwrap();
        w.finish().unwrap();
    }

    fn md5_name(path: &Path) -> String {
        format!("{:x}.png", Md5::digest(file_uri(path).as_bytes()))
    }

    fn files_in(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let Ok(rd) = fs::read_dir(dir) else {
            return found;
        };
        for e in rd {
            let p = e.unwrap().path();
            if p.is_dir() {
                found.extend(files_in(&p));
            } else {
                found.push(p);
            }
        }
        found
    }

    fn wait_for(what: &str, cond: impl Fn() -> bool) {
        let deadline = Instant::now() + PATIENCE;
        while !cond() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn thumbnail_name_follows_the_spec_example() {
        // The example of the freedesktop thumbnail spec.
        assert_eq!(
            thumbnail_name(Path::new("/home/jens/photos/me.png")),
            "c6ee772d9e49320e97ec29a7eb5b1697.png"
        );
    }

    #[test]
    fn cached_thumbnail_found_by_uri_md5() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("thumbnails");
        let file = tmp.path().join("photos/My cat (1).png");
        image(&file, 4, 4);
        let mtime = mtime_of(&file);
        let thumbs = Thumbs::new(root.clone());
        assert_eq!(thumbs.cached(&file, mtime), None);

        let uri = file_uri(&file);
        let normal = root.join("normal").join(md5_name(&file));
        thumbnail(
            &normal,
            &[("Thumb::URI", &uri), ("Thumb::MTime", &mtime.to_string())],
        );
        assert_eq!(thumbs.cached(&file, mtime), Some(normal.clone()));

        // Large comes first.
        let large = root.join("large").join(md5_name(&file));
        thumbnail(
            &large,
            &[("Thumb::URI", &uri), ("Thumb::MTime", &mtime.to_string())],
        );
        assert_eq!(thumbs.cached(&file, mtime), Some(large.clone()));
        // A stale large one falls back to a valid normal one.
        thumbnail(
            &large,
            &[
                ("Thumb::URI", &uri),
                ("Thumb::MTime", &(mtime - 5).to_string()),
            ],
        );
        assert_eq!(thumbs.cached(&file, mtime), Some(normal));
        // An unknown modification time never matches.
        thumbnail(&large, &[("Thumb::URI", &uri), ("Thumb::MTime", "0")]);
        assert_eq!(thumbs.cached(&file, 0), None);
    }

    #[test]
    fn stale_thumbnail_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("thumbnails");
        let file = tmp.path().join("a.png");
        image(&file, 4, 4);
        let mtime = mtime_of(&file);
        let thumbs = Thumbs::new(root.clone());
        let at = root.join("large").join(md5_name(&file));
        let uri = file_uri(&file);
        thumbnail(
            &at,
            &[
                ("Thumb::URI", &uri),
                ("Thumb::MTime", &(mtime + 1).to_string()),
            ],
        );
        assert_eq!(thumbs.cached(&file, mtime), None);
        // No modification time at all, or garbage: not valid either.
        thumbnail(&at, &[("Thumb::URI", &uri)]);
        assert_eq!(thumbs.cached(&file, mtime), None);
        thumbnail(&at, &[("Thumb::MTime", "soon")]);
        assert_eq!(thumbs.cached(&file, mtime), None);
        fs::write(&at, b"not a png").unwrap();
        assert_eq!(thumbs.cached(&file, mtime), None);
    }

    #[test]
    fn generates_png_with_spec_chunks() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("wide.png");
        image(&file, 1000, 500);
        let mtime = mtime_of(&file);
        let out = tmp.path().join("thumbnails/large").join(md5_name(&file));
        generate(&file, mtime, &out, &mut InProcess).unwrap();

        let decoder = png::Decoder::new(std::io::BufReader::new(fs::File::open(&out).unwrap()));
        let reader = decoder.read_info().unwrap();
        let info = reader.info();
        assert_eq!((info.width, info.height), (256, 128));
        let text: Vec<(String, String)> = info
            .uncompressed_latin1_text
            .iter()
            .map(|c| (c.keyword.clone(), c.text.clone()))
            .collect();
        assert!(
            text.contains(&("Thumb::URI".into(), file_uri(&file))),
            "{text:?}"
        );
        assert!(
            text.contains(&("Thumb::MTime".into(), mtime.to_string())),
            "{text:?}"
        );
        let mode = fs::metadata(&out).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // Only the thumbnail is left: the temp file was renamed.
        assert_eq!(
            files_in(&tmp.path().join("thumbnails")),
            std::slice::from_ref(&out)
        );
        // And it is what the cache now answers.
        let thumbs = Thumbs::new(tmp.path().join("thumbnails"));
        assert_eq!(thumbs.cached(&file, mtime), Some(out));

        // A small image is not blown up.
        let small = tmp.path().join("small.png");
        image(&small, 40, 20);
        let out = tmp.path().join("small-thumb.png");
        generate(&small, mtime_of(&small), &out, &mut InProcess).unwrap();
        assert_eq!(image::image_dimensions(&out).unwrap(), (40, 20));
    }

    #[test]
    fn refuses_too_large_images() {
        let tmp = TempDir::new().unwrap();
        let cache = tmp.path().join("thumbnails/large");

        // A PNG whose header says 20000x20000 (400 megapixels), with a valid, small body.
        let huge = tmp.path().join("huge.png");
        let mut enc = png::Encoder::new(fs::File::create(&huge).unwrap(), 20_000, 20_000);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::One);
        let mut writer = enc.write_header().unwrap();
        // One row of zeros is enough for the header to be read; the rest is never decoded.
        writer
            .write_chunk(
                png::chunk::IDAT,
                &[0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
            )
            .unwrap();
        drop(writer);
        let err = generate(
            &huge,
            mtime_of(&huge),
            &cache.join("huge.png"),
            &mut InProcess,
        )
        .unwrap_err();
        assert!(err.to_string().contains("megapixels"), "{err:#}");

        // Over 50 MB on disk (sparse, so cheap to make).
        let big = tmp.path().join("big.png");
        image(&big, 4, 4);
        fs::OpenOptions::new()
            .write(true)
            .open(&big)
            .unwrap()
            .set_len(50 * 1024 * 1024 + 1)
            .unwrap();
        let err =
            generate(&big, mtime_of(&big), &cache.join("big.png"), &mut InProcess).unwrap_err();
        assert!(err.to_string().contains("50 MB"), "{err:#}");

        // Not a regular file (a pipe would block the thread forever).
        #[cfg(unix)]
        {
            let fifo = tmp.path().join("pipe.png");
            let status = std::process::Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap();
            assert!(status.success());
            let err = generate(
                &fifo,
                mtime_of(&fifo),
                &cache.join("pipe.png"),
                &mut InProcess,
            )
            .unwrap_err();
            assert!(err.to_string().contains("regular file"), "{err:#}");
        }

        // Changed since it was indexed.
        let moved = tmp.path().join("moved.png");
        image(&moved, 4, 4);
        let err = generate(
            &moved,
            mtime_of(&moved) - 10,
            &cache.join("moved.png"),
            &mut InProcess,
        )
        .unwrap_err();
        assert!(err.to_string().contains("changed"), "{err:#}");

        assert_eq!(
            files_in(&tmp.path().join("thumbnails")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn skips_unsupported_types() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("thumbnails");
        let pdf = tmp.path().join("paper.pdf");
        fs::write(&pdf, b"%PDF-1.4").unwrap();
        // An image in disguise is still not asked for: the type is read off the name.
        let fake = tmp.path().join("notes.txt");
        image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3]))
            .save_with_format(&fake, image::ImageFormat::Png)
            .unwrap();
        let pic = tmp.path().join("pic.JPG");
        image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]))
            .save_with_format(&pic, image::ImageFormat::Jpeg)
            .unwrap();
        let thumbs = Thumbs::new(root.clone());
        let done = Arc::new(Mutex::new(Vec::new()));
        for path in [&pdf, &fake, &pic] {
            let done = done.clone();
            let name = path.file_name().unwrap().to_owned();
            thumbs.request(path, mtime_of(path), move || {
                done.lock().unwrap().push(name);
            });
        }
        // One thread, in order: once the picture is done, the others were passed over.
        wait_for("the picture", || !done.lock().unwrap().is_empty());
        assert_eq!(*done.lock().unwrap(), ["pic.JPG"]);
        assert_eq!(files_in(&root), [root.join("large").join(md5_name(&pic))]);
        assert!(thumbs.cached(&pic, mtime_of(&pic)).is_some());
        assert_eq!(thumbs.cached(&pdf, mtime_of(&pdf)), None);
    }

    #[test]
    fn a_failed_image_is_not_tried_again() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("thumbnails");
        let broken = tmp.path().join("broken.png");
        fs::write(&broken, b"\x89PNG\r\n\x1a\nnot really").unwrap();
        let good = tmp.path().join("good.png");
        image(&good, 4, 4);
        let thumbs = Thumbs::new(root.clone());
        let calls = Arc::new(Mutex::new(Vec::<&str>::new()));
        let call = |name: &'static str| {
            let calls = calls.clone();
            move || calls.lock().unwrap().push(name)
        };
        thumbs.request(&broken, mtime_of(&broken), call("broken"));
        thumbs.request(&good, mtime_of(&good), call("good"));
        wait_for("the good one", || calls.lock().unwrap().contains(&"good"));
        // Asked again (every rebuild of the grid asks): neither is queued a second time.
        thumbs.request(&broken, mtime_of(&broken), call("broken again"));
        thumbs.request(&good, mtime_of(&good), call("good again"));
        assert_eq!(thumbs.queued(), 0);
        std::thread::sleep(Duration::from_millis(300));
        // A failure calls nothing: there is nothing new to show.
        assert_eq!(*calls.lock().unwrap(), ["good"]);
    }

    #[test]
    fn cancel_drops_what_is_not_started() {
        let tmp = TempDir::new().unwrap();
        let thumbs = Thumbs::new(tmp.path().join("thumbnails"));
        // Many requests at once, then the cancel.
        let calls = Arc::new(Mutex::new(0usize));
        let files: Vec<PathBuf> = (0..50)
            .map(|n| {
                let file = tmp.path().join(format!("{n}.png"));
                image(&file, 600, 600);
                file
            })
            .collect();
        for file in &files {
            let calls = calls.clone();
            thumbs.request(file, mtime_of(file), move || *calls.lock().unwrap() += 1);
        }
        thumbs.cancel_pending();
        assert_eq!(thumbs.queued(), 0);
        std::thread::sleep(Duration::from_millis(500));
        // The thread may have taken a few before the cancel (a busy machine can hold the
        // test back between requests), never all of them.
        assert!(*calls.lock().unwrap() < 10, "{}", calls.lock().unwrap());
        // Cancelled requests can be made again.
        let file = tmp.path().join("49.png");
        let again = Arc::new(Mutex::new(false));
        let flag = again.clone();
        thumbs.request(&file, mtime_of(&file), move || *flag.lock().unwrap() = true);
        wait_for("the request made again", || *again.lock().unwrap());
    }

    #[test]
    fn queue_is_bounded() {
        let tmp = TempDir::new().unwrap();
        let thumbs = Thumbs::new(tmp.path().join("thumbnails"));
        // The thread is never started: nothing drains the queue while the test fills it.
        thumbs.thread.call_once(|| {});
        for n in 0..QUEUE_CAP + 50 {
            thumbs.request(&tmp.path().join(format!("{n}.png")), 1, || {});
        }
        assert_eq!(thumbs.queued(), QUEUE_CAP);
    }

    #[test]
    fn a_missing_helper_is_a_failure() {
        let tmp = TempDir::new().unwrap();
        let mut helper = InHelper(tomakast_raster::Client::new(
            tmp.path().join("no-such-tomakast-raster"),
        ));
        let file = tmp.path().join("a.png");
        image(&file, 4, 4);
        let out = tmp.path().join("thumbnails/large/a.png");
        let err = generate(&file, mtime_of(&file), &out, &mut helper).unwrap_err();
        assert!(err.to_string().contains("did not start"), "{err:#}");
        assert!(!out.exists());
    }

    #[test]
    fn a_decoder_failure_is_remembered() {
        // The helper refused, stopped or ran out of time: the file is not asked for again.
        struct Failing(Arc<Mutex<usize>>);
        impl Decode for Failing {
            fn thumbnail(&mut self, _: Arc<[u8]>) -> Result<tomakast_raster::Pixels, String> {
                *self.0.lock().unwrap() += 1;
                Err("the raster helper stopped".into())
            }
        }
        let tmp = TempDir::new().unwrap();
        let calls = Arc::new(Mutex::new(0));
        let thumbs = Thumbs::with_decoder(
            tmp.path().join("thumbnails"),
            BATCH_EVERY,
            Box::new(Failing(calls.clone())),
        );
        let file = tmp.path().join("a.png");
        image(&file, 4, 4);
        let done = Arc::new(Mutex::new(false));
        let flag = done.clone();
        thumbs.request(&file, mtime_of(&file), move || *flag.lock().unwrap() = true);
        wait_for("the decoder", || *calls.lock().unwrap() == 1);
        thumbs.request(&file, mtime_of(&file), || {});
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(*calls.lock().unwrap(), 1);
        assert!(!*done.lock().unwrap());
        assert_eq!(
            files_in(&tmp.path().join("thumbnails")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn skips_files_in_the_cache_itself() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("thumbnails");
        let thumbs = Thumbs::new(root.clone());
        thumbs.thread.call_once(|| {});
        // With hidden folders indexed, the cache's own thumbnails are found too.
        let inside = root
            .join("large")
            .join("0123456789abcdef0123456789abcdef.png");
        image(&inside, 4, 4);
        thumbs.request(&inside, mtime_of(&inside), || {});
        assert_eq!(thumbs.requested(), 0);
        // A file beside the cache is asked for.
        let beside = tmp.path().join("thumbnails-of-mine.png");
        image(&beside, 4, 4);
        thumbs.request(&beside, mtime_of(&beside), || {});
        assert_eq!(thumbs.requested(), 1);
    }

    #[test]
    fn finished_thumbnails_are_batched() {
        let tmp = TempDir::new().unwrap();
        let every = Duration::from_millis(250);
        let thumbs = Thumbs::with_batch_every(tmp.path().join("thumbnails"), every);
        let at = Arc::new(Mutex::new(Vec::new()));
        for n in 0..6 {
            let file = tmp.path().join(format!("{n}.png"));
            image(&file, 300, 300);
            let at = at.clone();
            thumbs.request(&file, mtime_of(&file), move || {
                at.lock().unwrap().push(Instant::now())
            });
        }
        wait_for("all six", || at.lock().unwrap().len() == 6);
        // The calls come in bursts at least `every` apart; the last burst is not held back
        // for more thumbnails that never come.
        let at = at.lock().unwrap();
        for pair in at.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                gap < Duration::from_millis(50) || gap >= every - Duration::from_millis(5),
                "{gap:?}"
            );
        }
    }
}
