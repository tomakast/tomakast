//! Ids, titles and URIs of files: whatever bytes a path holds, the id is valid and the text
//! is printable.

use std::borrow::Cow;
use std::ffi::OsStr;
use std::path::Path;

use md5::{Digest, Md5};
use tomakast_core::{ItemId, is_hidden_char};

/// The provider id of the module.
pub const PROVIDER: &str = "files";

/// The path's raw bytes: exact on Unix, UTF-8 (lossy) elsewhere.
pub fn path_bytes(path: &Path) -> Cow<'_, [u8]> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        Cow::Borrowed(path.as_os_str().as_bytes())
    }
    #[cfg(not(unix))]
    {
        match path.to_string_lossy() {
            Cow::Borrowed(s) => Cow::Borrowed(s.as_bytes()),
            Cow::Owned(s) => Cow::Owned(s.into_bytes()),
        }
    }
}

/// `files/<md5 hex of the path's bytes>`: 38 bytes whatever the path is.
pub fn item_id(path: &Path) -> ItemId {
    let digest = Md5::digest(path_bytes(path));
    ItemId::new(PROVIDER, &format!("{digest:x}"))
}

/// A name as text: invalid UTF-8 and control or format characters (newline, bidi overrides)
/// become U+FFFD.
pub fn display_name(name: &OsStr) -> String {
    name.to_string_lossy()
        .chars()
        .map(|c| if is_hidden_char(c) { '\u{fffd}' } else { c })
        .collect()
}

/// The folder a path lies in, as text: `~/x/y` under `home`, the absolute path elsewhere.
pub fn display_parent(path: &Path, home: &Path) -> String {
    let Some(parent) = path.parent() else {
        return display_name(path.as_os_str());
    };
    match parent.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", display_name(rest.as_os_str())),
        Err(_) => display_name(parent.as_os_str()),
    }
}

/// A full path as text: `~/docs/fire.txt` under `home`, the absolute path elsewhere.
pub fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => format!("~/{}", display_name(rest.as_os_str())),
        Err(_) => display_name(path.as_os_str()),
    }
}

/// `file://` and the path escaped like glib's `g_filename_to_uri` (the form the thumbnail
/// cache hashes): letters, digits and `-_.!~*'()/:@&=+$,` stay, every other byte is `%XX`.
pub fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for &byte in path_bytes(path).iter() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')'
            | b'/'
            | b':'
            | b'@'
            | b'&'
            | b'='
            | b'+'
            | b'$'
            | b',' => uri.push(byte as char),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn item_id_is_stable_and_valid() {
        let a = item_id(Path::new("/home/u/a.txt"));
        assert_eq!(a, item_id(Path::new("/home/u/a.txt")));
        assert_ne!(a, item_id(Path::new("/home/u/b.txt")));
        assert!(a.is_valid());
        assert_eq!(a.as_str().len(), 38);
        assert!(a.as_str().starts_with("files/"));
    }

    #[test]
    fn id_and_title_of_hostile_names() {
        let long = PathBuf::from(format!("/home/u/{}", "x".repeat(300)));
        assert!(item_id(&long).is_valid());
        assert_eq!(item_id(&long).as_str().len(), 38);

        let nasty = ["a\nb", "a\u{202e}txt.exe"];
        for n in nasty {
            let p = Path::new("/home/u").join(n);
            assert!(item_id(&p).is_valid());
            let shown = display_name(OsStr::new(n));
            assert!(!shown.chars().any(is_hidden_char), "{shown:?}");
            assert!(shown.contains('\u{fffd}'));
        }

        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let raw = OsStr::from_bytes(b"caf\xe9\xff.txt");
            let p = Path::new("/home/u").join(raw);
            assert!(item_id(&p).is_valid());
            let shown = display_name(raw);
            assert!(shown.starts_with("caf"));
            assert!(!shown.chars().any(is_hidden_char));
            assert!(shown.contains('\u{fffd}'));
            // The id follows the real bytes, not the lossy text.
            let other = Path::new("/home/u").join(OsStr::from_bytes(b"caf\xe8\xff.txt"));
            assert_ne!(item_id(&p), item_id(&other));
        }
    }

    #[test]
    fn display_parent_uses_tilde_under_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            display_parent(Path::new("/home/u/x/y/f.txt"), home),
            "~/x/y"
        );
        assert_eq!(display_parent(Path::new("/home/u/f.txt"), home), "~");
        assert_eq!(display_parent(Path::new("/data/z/f.txt"), home), "/data/z");
        assert_eq!(display_parent(Path::new("/f.txt"), home), "/");
    }

    #[test]
    fn display_path_uses_tilde_under_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            display_path(Path::new("/home/u/x/f.txt"), home),
            "~/x/f.txt"
        );
        assert_eq!(display_path(Path::new("/home/u"), home), "~");
        assert_eq!(display_path(Path::new("/data/f.txt"), home), "/data/f.txt");
        assert_eq!(
            display_path(Path::new("/home/u/a\nb"), home),
            "~/a\u{fffd}b"
        );
    }

    #[test]
    fn file_uri_matches_glib() {
        for (path, uri) in [
            (
                "/home/u/Screenshot (1).png",
                "file:///home/u/Screenshot%20(1).png",
            ),
            ("/a/a+b,c.jpg", "file:///a/a+b,c.jpg"),
            ("/a/x;y#z?.txt", "file:///a/x%3By%23z%3F.txt"),
            ("/a/100%.txt", "file:///a/100%25.txt"),
            ("/a/é", "file:///a/%C3%A9"),
        ] {
            assert_eq!(file_uri(Path::new(path)), uri);
        }
    }
}
