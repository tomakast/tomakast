//! Reading a plugin folder: `plugin.toml` + `plugin.wasm`, hashed as one unit.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::manifest::{Manifest, ManifestError};

/// Largest accepted `plugin.wasm`.
pub const MAX_WASM_BYTES: u64 = 32 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// Reads at most `max` bytes of a regular file inside `dir`. A symlink is followed only when
/// it resolves to a regular file inside the (canonical) folder; anything else is refused.
fn read_capped(dir: &Path, file: &'static str, max: u64) -> Result<Vec<u8>, PackageError> {
    let io = |source| PackageError::Io { file, source };
    let refuse = |msg: &str| PackageError::Io {
        file,
        source: std::io::Error::other(msg.to_owned()),
    };
    let path = dir.join(file);
    let link = std::fs::symlink_metadata(&path).map_err(io)?;
    if link.file_type().is_symlink() {
        let root = dir.canonicalize().map_err(io)?;
        let target = path.canonicalize().map_err(io)?;
        if !target.starts_with(&root) {
            return Err(refuse("symlink points outside the plugin folder"));
        }
    }
    if !std::fs::metadata(&path).map_err(io)?.is_file() {
        return Err(refuse("not a regular file"));
    }
    let mut buf = Vec::new();
    File::open(&path)
        .map_err(io)?
        .take(max + 1)
        .read_to_end(&mut buf)
        .map_err(io)?;
    if buf.len() as u64 > max {
        return Err(PackageError::TooLarge { file, max });
    }
    Ok(buf)
}

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("cannot read {file}: {source}")]
    Io {
        file: &'static str,
        source: std::io::Error,
    },
    #[error("{file} is larger than {max} bytes")]
    TooLarge { file: &'static str, max: u64 },
    #[error("plugin folder has no usable name")]
    NoFolderName,
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

pub struct Package {
    /// The plugin folder the files were read from; `plugin-file` icons resolve against it.
    pub dir: PathBuf,
    pub manifest: Manifest,
    pub wasm: Arc<[u8]>,
    /// `sha256(plugin.toml ‖ 0x00 ‖ plugin.wasm)`; approval is bound to this.
    pub content_hash: [u8; 32],
    /// `sha256(plugin.wasm)`; keys the compile cache.
    pub wasm_hash: [u8; 32],
}

impl Package {
    /// Reads both files once; the returned bytes are what gets hashed, compiled and run.
    pub fn read(dir: &Path) -> Result<Package, PackageError> {
        let folder = dir
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or(PackageError::NoFolderName)?;
        let toml_bytes = read_capped(dir, "plugin.toml", MAX_MANIFEST_BYTES)?;
        let wasm = read_capped(dir, "plugin.wasm", MAX_WASM_BYTES)?;
        let text = std::str::from_utf8(&toml_bytes)
            .map_err(|e| ManifestError::Syntax(format!("plugin.toml is not UTF-8: {e}")))?;
        let manifest = Manifest::parse(text, folder)?;

        let mut hasher = Sha256::new();
        hasher.update(&toml_bytes);
        hasher.update([0u8]);
        hasher.update(&wasm);
        Ok(Package {
            dir: dir.to_owned(),
            manifest,
            content_hash: hasher.finalize().into(),
            wasm_hash: Sha256::digest(&wasm).into(),
            wasm: Arc::from(wasm),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    const TOML: &str = "id = \"demo-plugin\"\nname = \"X\"\nversion = \"1\"\napi = \"0.1.0\"\n";

    fn folder(toml: &str, wasm: Option<&[u8]>) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("demo-plugin");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("plugin.toml"), toml).unwrap();
        if let Some(w) = wasm {
            fs::write(dir.join("plugin.wasm"), w).unwrap();
        }
        root
    }

    fn read(root: &tempfile::TempDir) -> Result<Package, PackageError> {
        Package::read(&root.path().join("demo-plugin"))
    }

    #[test]
    fn package_reads_and_hashes() {
        let root = folder(TOML, Some(b"\0asm-bytes"));
        let p = read(&root).unwrap();
        assert_eq!(p.manifest.id, "demo-plugin");
        assert_eq!(&*p.wasm, b"\0asm-bytes");
        let mut h = Sha256::new();
        h.update(TOML.as_bytes());
        h.update([0u8]);
        h.update(b"\0asm-bytes");
        assert_eq!(p.content_hash, <[u8; 32]>::from(h.finalize()));
        assert_eq!(
            p.wasm_hash,
            <[u8; 32]>::from(Sha256::digest(b"\0asm-bytes"))
        );
    }

    #[test]
    fn package_hash_covers_both_files() {
        let base = read(&folder(TOML, Some(b"wasm-1"))).unwrap().content_hash;
        let wasm_changed = read(&folder(TOML, Some(b"wasm-2"))).unwrap().content_hash;
        let toml_changed = read(&folder(&format!("{TOML}author = \"A\"\n"), Some(b"wasm-1")))
            .unwrap()
            .content_hash;
        assert_ne!(base, wasm_changed);
        assert_ne!(base, toml_changed);
        assert_ne!(wasm_changed, toml_changed);
    }

    #[test]
    fn package_hash_separates_the_two_files() {
        // Same concatenated bytes, different split between toml and wasm.
        let a = read(&folder(TOML, Some(b"\nrest"))).unwrap();
        let b = read(&folder(&format!("{TOML}\n"), Some(b"rest"))).unwrap();
        assert_ne!(a.content_hash, b.content_hash);
        assert_eq!(b.wasm_hash, {
            use sha2::{Digest, Sha256};
            <[u8; 32]>::from(Sha256::digest(b"rest"))
        });
    }

    #[test]
    fn package_rejects_non_regular_files() {
        let root = folder(TOML, Some(b"x"));
        let dir = root.path().join("demo-plugin");
        fs::remove_file(dir.join("plugin.wasm")).unwrap();
        fs::create_dir(dir.join("plugin.wasm")).unwrap();
        assert!(matches!(
            Package::read(&dir),
            Err(PackageError::Io {
                file: "plugin.wasm",
                ..
            })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn package_symlinks_must_stay_inside_the_folder() {
        use std::os::unix::fs::symlink;
        let root = folder(TOML, Some(b"inside"));
        let dir = root.path().join("demo-plugin");
        // Inside: allowed.
        fs::rename(dir.join("plugin.wasm"), dir.join("real.wasm")).unwrap();
        symlink(dir.join("real.wasm"), dir.join("plugin.wasm")).unwrap();
        assert_eq!(&*Package::read(&dir).unwrap().wasm, b"inside");
        // Outside: refused.
        let outside = root.path().join("outside.wasm");
        fs::write(&outside, b"outside").unwrap();
        fs::remove_file(dir.join("plugin.wasm")).unwrap();
        symlink(&outside, dir.join("plugin.wasm")).unwrap();
        assert!(matches!(
            Package::read(&dir),
            Err(PackageError::Io {
                file: "plugin.wasm",
                ..
            })
        ));
    }

    #[test]
    fn package_rejects_missing_wasm() {
        let root = folder(TOML, None);
        assert!(matches!(
            read(&root),
            Err(PackageError::Io {
                file: "plugin.wasm",
                ..
            })
        ));
    }

    #[test]
    fn package_rejects_missing_manifest() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("demo-plugin");
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("plugin.wasm"), b"x").unwrap();
        assert!(matches!(
            Package::read(&dir),
            Err(PackageError::Io {
                file: "plugin.toml",
                ..
            })
        ));
    }

    #[test]
    fn package_rejects_wasm_over_32_mib() {
        let at_limit = vec![0u8; MAX_WASM_BYTES as usize];
        assert!(read(&folder(TOML, Some(&at_limit))).is_ok());
        let over = vec![0u8; MAX_WASM_BYTES as usize + 1];
        assert!(matches!(
            read(&folder(TOML, Some(&over))),
            Err(PackageError::TooLarge {
                file: "plugin.wasm",
                ..
            })
        ));
    }

    #[test]
    fn package_rejects_id_not_matching_folder() {
        let root = folder(&TOML.replace("demo-plugin", "other"), Some(b"x"));
        assert!(matches!(
            read(&root),
            Err(PackageError::Manifest(
                ManifestError::IdFolderMismatch { .. }
            ))
        ));
    }
}
