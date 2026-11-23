//! The desktop's list of recently used files (`recently-used.xbel`, KDE and GTK), a small
//! lift in the module's ranking.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

/// The paths of the recently used list, read again when the file changes.
pub struct Recent {
    file: Option<PathBuf>,
    /// The modification time of the file when it was last read; `None` while it is missing.
    stamp: Option<SystemTime>,
    paths: HashSet<PathBuf>,
}

impl Recent {
    pub fn new(file: Option<PathBuf>) -> Recent {
        Recent {
            file,
            stamp: None,
            paths: HashSet::new(),
        }
    }

    /// Reads the file again if its modification time changed: one `stat` otherwise.
    pub fn refresh(&mut self) {
        let Some(file) = &self.file else {
            return;
        };
        let meta = std::fs::metadata(file).ok();
        let stamp = meta.as_ref().and_then(|m| m.modified().ok());
        if stamp == self.stamp {
            return;
        }
        self.stamp = stamp;
        self.paths = match meta {
            Some(meta) if meta.len() > MAX_BYTES => {
                tracing::warn!(file = %file.display(), "files: the recent list is too large, ignored");
                HashSet::new()
            }
            Some(_) => std::fs::read(file)
                .map(|bytes| xbel_paths(&String::from_utf8_lossy(&bytes)))
                .unwrap_or_default(),
            None => HashSet::new(),
        };
    }

    pub fn paths(&self) -> &HashSet<PathBuf> {
        &self.paths
    }
}

/// The local paths of the `<bookmark href="file:///…">` elements of an XBEL document.
///
/// A plain scan, not an XML parser: the file is machine-written and flat, and only one
/// attribute of one element is needed. Whatever does not look like such an element (another
/// element, a remote URI, an unterminated attribute) is skipped; nothing fails.
pub fn xbel_paths(text: &str) -> HashSet<PathBuf> {
    const TAG: &str = "<bookmark";
    let mut paths = HashSet::new();
    let mut rest = text;
    while let Some(i) = rest.find(TAG) {
        rest = &rest[i + TAG.len()..];
        // `<bookmark:application …>` and the like are other elements.
        if !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        if let Some(path) = attribute(tag, "href").and_then(|href| file_path(&unescape(href))) {
            paths.insert(path);
        }
    }
    paths
}

/// Larger recent lists are not read (they hold a few hundred entries).
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// The value of attribute `name` in the inside of a start tag, as written (escaped).
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(i) = tag[from..].find(name) {
        let at = from + i;
        from = at + name.len();
        if !tag[..at].ends_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let Some(after) = tag[from..].trim_start().strip_prefix('=') else {
            continue;
        };
        let after = after.trim_start();
        let quote = after.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let value = &after[1..];
        return value.find(quote).map(|end| &value[..end]);
    }
    None
}

/// XML's five named entities and numeric character references; anything else stays.
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let decoded = rest.find(';').and_then(|end| {
            let c = match &rest[1..end] {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                num => {
                    let code = match num.strip_prefix("#x").or_else(|| num.strip_prefix("#X")) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => num.strip_prefix('#').and_then(|d| d.parse().ok()),
                    };
                    char::from_u32(code?)?
                }
            };
            Some((c, end))
        });
        match decoded {
            Some((c, end)) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The path of a local `file:` URI (`file:///p` or `file://localhost/p`), percent escapes
/// decoded to bytes; a `%` not followed by two hex digits stays as it is.
fn file_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let path = match rest.strip_prefix("localhost") {
        Some(path) => path,
        None => rest,
    };
    if !path.starts_with('/') {
        return None;
    }
    let raw = path.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if raw[i] == b'%'
            && let (Some(hi), Some(lo)) = (
                raw.get(i + 1).copied().and_then(hex),
                raw.get(i + 2).copied().and_then(hex),
            )
        {
            bytes.push((hi * 16 + lo) as u8);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
    }
    #[cfg(not(unix))]
    {
        String::from_utf8(bytes).ok().map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    const XBEL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<xbel version="1.0" xmlns:bookmark="http://www.freedesktop.org/standards/desktop-bookmarks">
  <bookmark href="file:///home/u/docs/fire.txt" added="2026-11-01T10:00:00Z">
    <info><metadata owner="http://freedesktop.org"><bookmark:applications>
      <bookmark:application name="kate" exec="&apos;kate %u&apos;" href="file:///not/this"/>
    </bookmark:applications></metadata></info>
  </bookmark>
  <bookmark  href='file:///home/u/Tom%20%26%20Jerry.mp4'/>
  <bookmark href="file:///home/u/a&amp;b%E9.txt"/>
  <bookmark href="file://localhost/home/u/local.txt"/>
  <bookmark href="https://example.com/page"/>
  <bookmark href="file://otherhost/home/u/remote.txt"/>
  <bookmark href="file:///home/u/broken%"/>
  <bookmark href="file:///home/u/unterminated
"#;

    #[test]
    fn recent_paths_from_xbel() {
        let paths = xbel_paths(XBEL);
        let mut expected = vec![
            PathBuf::from("/home/u/docs/fire.txt"),
            PathBuf::from("/home/u/Tom & Jerry.mp4"),
            PathBuf::from("/home/u/local.txt"),
            PathBuf::from("/home/u/broken%"),
        ];
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            // Percent escapes are bytes, not necessarily UTF-8.
            expected.push(PathBuf::from(std::ffi::OsStr::from_bytes(
                b"/home/u/a&b\xe9.txt",
            )));
        }
        let expected: HashSet<PathBuf> = expected.into_iter().collect();
        #[cfg(not(unix))]
        let paths: HashSet<PathBuf> = paths
            .into_iter()
            .filter(|p| !p.to_string_lossy().contains("a&b"))
            .collect();
        assert_eq!(paths, expected);
    }

    #[test]
    fn recent_follows_the_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("recently-used.xbel");
        let mut recent = Recent::new(Some(file.clone()));
        recent.refresh();
        assert!(recent.paths().is_empty());

        fs::write(&file, r#"<bookmark href="file:///a.txt"/>"#).unwrap();
        recent.refresh();
        assert_eq!(recent.paths().len(), 1);

        fs::write(
            &file,
            r#"<bookmark href="file:///a.txt"/><bookmark href="file:///b"/>"#,
        )
        .unwrap();
        // A later modification time, whatever the file system's resolution.
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later)
            .unwrap();
        recent.refresh();
        assert_eq!(recent.paths().len(), 2);

        fs::remove_file(&file).unwrap();
        recent.refresh();
        assert!(recent.paths().is_empty());
    }
}
