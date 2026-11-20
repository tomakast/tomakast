//! Image files that plugins name as icons (`Icon::PluginFile`), loaded with bounds.
//!
//! The plugin host checked the path when the icon arrived (inside the plugin folder, no
//! symbolic link, a regular file of at most 1 MiB), but the UI opens the file later. So that
//! a file swapped in between cannot make the launcher follow a link, block on a FIFO or
//! decode a huge canvas, the UI reads it here, never through `img(path)`:
//! - the file is opened without following a symbolic link (`O_NOFOLLOW`) and without
//!   blocking (`O_NONBLOCK`), and must be a regular file;
//! - at most [`MAX_PLUGIN_IMAGE_BYTES`] are read;
//! - only PNG and JPEG are accepted (static formats; GIF and WebP can hold many frames), and
//!   the dimensions declared in the header must be 1..=[`MAX_PLUGIN_IMAGE_SIDE`] on both
//!   sides, checked before anything decodes the image.
//!
//! Anything else shows as no icon. Loaded images are kept per path while the file's size and
//! modification time stay the same.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::{Image, ImageFormat};

/// Largest plugin image file, in bytes.
pub const MAX_PLUGIN_IMAGE_BYTES: u64 = 1024 * 1024;
/// Largest width and height a plugin image may declare, in pixels.
pub const MAX_PLUGIN_IMAGE_SIDE: u32 = 1024;

/// The image at `path`, if it follows the rules above; cached.
pub(crate) fn cached(path: &Path) -> Option<Arc<Image>> {
    type Cache = HashMap<PathBuf, (Stamp, Option<Arc<Image>>)>;
    thread_local! {
        static CACHE: RefCell<Cache> = RefCell::new(HashMap::new());
    }
    let stamp = Stamp::of(path)?;
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((seen, image)) = cache.get(path)
            && *seen == stamp
        {
            return image.clone();
        }
        let image = match load(path) {
            Ok(image) => Some(image),
            Err(problem) => {
                tracing::warn!(path = %path.display(), "plugin icon not shown: {problem}");
                None
            }
        };
        cache.insert(path.to_owned(), (stamp, image.clone()));
        image
    })
}

/// What identifies a version of a file for the cache: its size and modification time, read
/// without following a link.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Stamp(u64, Option<SystemTime>);

impl Stamp {
    fn of(path: &Path) -> Option<Stamp> {
        let meta = std::fs::symlink_metadata(path).ok()?;
        Some(Stamp(meta.len(), meta.modified().ok()))
    }
}

/// Reads and checks the image at `path` (see the module docs).
pub fn load(path: &Path) -> Result<Arc<Image>, String> {
    let file = open_no_follow(path).map_err(|e| format!("cannot open: {e}"))?;
    let meta = file.metadata().map_err(|e| format!("no metadata: {e}"))?;
    if !meta.is_file() {
        return Err("not a regular file".into());
    }
    if meta.len() > MAX_PLUGIN_IMAGE_BYTES {
        return Err(format!("larger than {MAX_PLUGIN_IMAGE_BYTES} bytes"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PLUGIN_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read: {e}"))?;
    if bytes.len() as u64 > MAX_PLUGIN_IMAGE_BYTES {
        return Err(format!("larger than {MAX_PLUGIN_IMAGE_BYTES} bytes"));
    }
    let (format, width, height) = dimensions(&bytes)?;
    let side = 1..=MAX_PLUGIN_IMAGE_SIDE;
    if !side.contains(&width) || !side.contains(&height) {
        return Err(format!(
            "declares {width}x{height} pixels, at most {MAX_PLUGIN_IMAGE_SIDE}x{MAX_PLUGIN_IMAGE_SIDE}"
        ));
    }
    Ok(Arc::new(Image::from_bytes(format, bytes)))
}

fn open_no_follow(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(path)
}

/// The format and the width and height an image declares in its header: PNG (`IHDR`) or
/// JPEG (the first start-of-frame marker). Anything else is refused.
pub fn dimensions(bytes: &[u8]) -> Result<(ImageFormat, u32, u32), String> {
    let (format, width, height) = tomakast_raster::bitmap::header(bytes)?;
    let format = match format {
        tomakast_core::ImageFormat::Png => ImageFormat::Png,
        _ => ImageFormat::Jpeg,
    };
    Ok((format, width, height))
}
