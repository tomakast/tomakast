//! The system's own icon of a file or folder, as Finder draws it, rendered to a PNG file the
//! launcher draws (macOS). Elsewhere there is none: Linux draws a file's type icon from the
//! icon theme ([`crate::icon_theme`]).
//!
//! A file's icon is its type's, asked once per extension; a folder's is the generic folder's,
//! except an app or another bundle, the home folder and the folders right in it or in `/`
//! (Applications, Documents, Downloads…), which have icons of their own; a file without an
//! extension is asked by its path (a program's icon is not a document's). [`FileIcons`]
//! renders each icon once into an [`ICON_PIXELS`] square PNG in the folder its owner gives,
//! after deleting the ones earlier runs left there.

use std::path::{Path, PathBuf};

/// The edge of a rendered icon, in pixels: the grid's 64 pt image on a 2x screen.
pub const ICON_PIXELS: u32 = 128;

/// What the system is asked for the icon of.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IconSource {
    /// The icon of this file or folder itself.
    Path(PathBuf),
    /// The icon of a type: a file extension (`pdf`) or a type identifier (`public.folder`).
    Type(String),
}

/// The type identifier of a plain folder.
const FOLDER_TYPE: &str = "public.folder";

/// What to ask the system for the icon of the file or folder at `path`, under `home`.
pub fn icon_source(path: &Path, is_dir: bool, home: &Path) -> IconSource {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty());
    if !is_dir {
        return match extension {
            Some(ext) => IconSource::Type(ext.to_lowercase()),
            None => IconSource::Path(path.to_path_buf()),
        };
    }
    let parent = path.parent();
    let own_icon = extension.is_some()
        || path == home
        || parent == Some(home)
        || parent == Some(Path::new("/"));
    if own_icon {
        IconSource::Path(path.to_path_buf())
    } else {
        IconSource::Type(FOLDER_TYPE.to_string())
    }
}

/// The name of the PNG of `source`: a hash of it, the same within a process.
fn file_name(source: &IconSource) -> String {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::hash::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{:016x}.png", hasher.finish())
}

/// Whether `name` is one [`file_name`] gives.
fn is_icon_file(name: &str) -> bool {
    name.strip_suffix(".png")
        .is_some_and(|stem| stem.len() == 16 && stem.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// The system's file icons, rendered as PNGs into one folder, each once.
pub struct FileIcons {
    dir: PathBuf,
    rendered: std::sync::Mutex<std::collections::HashMap<IconSource, Option<PathBuf>>>,
}

impl FileIcons {
    /// Icons rendered into `dir`, the caller's own folder: on macOS the icons earlier runs
    /// left there are deleted now.
    pub fn new(dir: PathBuf) -> FileIcons {
        if cfg!(target_os = "macos") {
            clear(&dir);
        }
        FileIcons {
            dir,
            rendered: Default::default(),
        }
    }

    /// The PNG of the system's icon of the file or folder at `path` (under `home`), rendered
    /// the first time it is asked for; `None` when it cannot be, and on every OS but macOS.
    pub fn icon(&self, path: &Path, is_dir: bool, home: &Path) -> Option<PathBuf> {
        if !cfg!(target_os = "macos") {
            return None;
        }
        let mut rendered = self
            .rendered
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        rendered
            .entry(icon_source(path, is_dir, home))
            .or_insert_with_key(|source| {
                let png = render_png(source)?;
                let file = self.dir.join(file_name(source));
                let written =
                    std::fs::create_dir_all(&self.dir).and_then(|()| std::fs::write(&file, png));
                match written {
                    Ok(()) => Some(file),
                    Err(err) => {
                        tracing::warn!(%err, dir = %self.dir.display(), "cannot write a file icon");
                        None
                    }
                }
            })
            .clone()
    }
}

/// Deletes the icons earlier runs left in `dir`, and nothing else.
fn clear(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_str().is_some_and(is_icon_file) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// The name Finder shows for the file or folder at `path`: an app's in the system's language
/// (`Calculator.app` is "Калькулятор" in Russian), without a hidden extension; `None` on every
/// OS but macOS.
pub fn display_name(path: &Path) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let path = objc2_foundation::NSString::from_str(path.to_str()?);
        let name = objc2::rc::autoreleasepool(|_| {
            objc2_foundation::NSFileManager::defaultManager()
                .displayNameAtPath(&path)
                .to_string()
        });
        (!name.is_empty()).then_some(name)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

/// The PNG bytes of `source`'s icon at [`ICON_PIXELS`].
#[cfg(target_os = "macos")]
fn render_png(source: &IconSource) -> Option<Vec<u8>> {
    use objc2::AllocAnyThread as _;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSWorkspace};
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    objc2::rc::autoreleasepool(|_| {
        let workspace = NSWorkspace::sharedWorkspace();
        let image = match source {
            IconSource::Path(path) => workspace.iconForFile(&NSString::from_str(path.to_str()?)),
            // An extension or a type identifier alike, without touching any file.
            #[allow(deprecated)]
            IconSource::Type(kind) => workspace.iconForFileType(&NSString::from_str(kind)),
        };
        let side = f64::from(ICON_PIXELS);
        let mut rect = NSRect::new(NSPoint::new(0., 0.), NSSize::new(side, side));
        // SAFETY: `rect` outlives the call; no context and no hints.
        let cg_image =
            unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        let bitmap = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg_image);
        // SAFETY: an empty dictionary is a valid property list.
        let png = unsafe {
            bitmap.representationUsingType_properties(
                NSBitmapImageFileType::PNG,
                &NSDictionary::new(),
            )
        }?;
        Some(png.to_vec())
    })
}

#[cfg(not(target_os = "macos"))]
fn render_png(_source: &IconSource) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str, is_dir: bool) -> IconSource {
        icon_source(Path::new(path), is_dir, Path::new("/Users/me"))
    }

    fn by_path(path: &str) -> IconSource {
        IconSource::Path(PathBuf::from(path))
    }

    fn by_type(kind: &str) -> IconSource {
        IconSource::Type(kind.to_string())
    }

    #[test]
    fn files_take_their_types_icon_by_extension() {
        assert_eq!(source("/Users/me/a/Report.PDF", false), by_type("pdf"));
        assert_eq!(source("/Users/me/a/b.tar.gz", false), by_type("gz"));
        // No extension: a program's icon is not a document's, so the file's own.
        assert_eq!(
            source("/Users/me/bin/tool", false),
            by_path("/Users/me/bin/tool")
        );
        assert_eq!(
            source("/Users/me/.zshrc", false),
            by_path("/Users/me/.zshrc")
        );
    }

    #[test]
    fn folders_are_the_folder_icon_unless_they_have_their_own() {
        assert_eq!(
            source("/Users/me/code/proj", true),
            by_type("public.folder")
        );
        // Apps and other bundles.
        assert_eq!(
            source("/Applications/Safari.app", true),
            by_path("/Applications/Safari.app")
        );
        assert_eq!(
            source("/Users/me/x/Lib.framework", true),
            by_path("/Users/me/x/Lib.framework")
        );
        // The home folder, the folders in it (Downloads…) and those in `/` (Applications…).
        assert_eq!(source("/Users/me", true), by_path("/Users/me"));
        assert_eq!(
            source("/Users/me/Downloads", true),
            by_path("/Users/me/Downloads")
        );
        assert_eq!(source("/Applications", true), by_path("/Applications"));
    }

    #[test]
    fn icon_files_are_named_by_a_hash_and_only_those_are_cleared() {
        let name = file_name(&by_type("pdf"));
        assert!(is_icon_file(&name), "{name}");
        assert_eq!(name, file_name(&by_type("pdf")));
        assert_ne!(name, file_name(&by_type("png")));
        for other in ["state.json", "index.bin", "abc.png", "0123456789abcdeg.png"] {
            assert!(!is_icon_file(other), "{other}");
        }
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(&name), b"").unwrap();
        std::fs::write(dir.path().join("keep.png"), b"").unwrap();
        clear(dir.path());
        assert!(!dir.path().join(&name).exists());
        assert!(dir.path().join("keep.png").exists());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn no_icons_outside_macos() {
        let dir = tempfile::tempdir().unwrap();
        let icons = FileIcons::new(dir.path().to_path_buf());
        assert_eq!(
            icons.icon(Path::new("/tmp/a.pdf"), false, Path::new("/tmp")),
            None
        );
    }
}
