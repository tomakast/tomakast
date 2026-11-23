//! The walked index of the covered folders: one compact entry per file or folder, mutated in
//! place by `upsert` and `remove` so a small change never rebuilds it.

use std::collections::HashMap;
use std::collections::hash_map::{DefaultHasher, Entry as MapEntry};
use std::ffi::OsStr;
use std::fs::Metadata;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use ignore::WalkBuilder;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;
use tomakast_core::{Scripts, TextKeys};

use crate::rules::Rules;
use crate::search::NameKeys;

pub type EntryId = u32;

/// The parent of a root entry (one with no indexed folder above it).
const NO_PARENT: EntryId = EntryId::MAX;

/// One file or folder. A root's name is its full path; every other name is one component.
/// Names live end to end in `FileIndex::names` (one allocation for all of them, not one each).
/// `keys`, the match data of the name, are filled on insert and dropped with the entry in
/// `remove`, so they stay in step with every change.
pub(crate) struct Entry {
    pub(crate) keys: NameKeys,
    pub(crate) mtime: i64,
    name_at: u32,
    name_len: u32,
    parent: EntryId,
    pub(crate) depth: u16,
    pub(crate) is_dir: bool,
    pub(crate) alive: bool,
}

impl Entry {
    pub(crate) fn parent(&self) -> Option<EntryId> {
        (self.parent != NO_PARENT).then_some(self.parent)
    }
}

/// The bytes a name is stored as.
fn os_bytes(s: &OsStr) -> &[u8] {
    s.as_encoded_bytes()
}

/// A name back from its stored bytes.
fn os_str(bytes: &[u8]) -> &OsStr {
    // SAFETY: every slice passed here is the whole `as_encoded_bytes()` of an `OsStr` of this
    // process (a name in the arena, a path in a walk batch).
    unsafe { OsStr::from_encoded_bytes_unchecked(bytes) }
}

/// What the rest of the module sees of an entry.
pub struct EntryView<'a> {
    pub name: &'a OsStr,
    pub is_dir: bool,
    /// Modification time in whole seconds since the Unix epoch.
    pub mtime: i64,
    /// 0 for a root.
    pub depth: u16,
}

/// A path found by the walker, before it is inserted: the path, whether it is a folder, and
/// its modification time.
pub(crate) type Found = (PathBuf, bool, i64);

/// What `upsert` of a path amounts to, worked out without changing the index (see
/// [`FileIndex::plan`]).
pub(crate) enum Upsert {
    /// Outside the roots, or a folder above it vanished meanwhile.
    Nothing,
    /// The path is gone, or left out by the rules.
    Remove,
    /// Entries to insert in this order, parents first: the folders above the path the index
    /// lacks, the path, and everything below it when it is a folder.
    Insert(Vec<Found>),
}

#[derive(Default)]
pub struct FileIndex {
    entries: Vec<Entry>,
    /// Every name's bytes, end to end. A removed entry's bytes stay until the next walk.
    names: Vec<u8>,
    /// The core's match keys of the names in a script other than Latin, by id.
    texts: HashMap<EntryId, TextKeys>,
    /// (parent, name) to entry, keyed by a 32-bit hash with linear probing on a clash; the
    /// candidate's parent and name are always verified. Dead entries keep their slot (so a
    /// probe chain never breaks) and are overwritten by the next insert that reaches them.
    by_name: HashMap<u32, EntryId>,
    /// Folders only: path to id.
    dirs: HashMap<PathBuf, EntryId>,
    /// Folders only: their children, dead ones included (filtered by `alive` when read).
    children: HashMap<EntryId, Vec<EntryId>>,
    roots: Vec<EntryId>,
    alive: usize,
    /// Every script seen in a name since the walk (not narrowed by `remove`).
    scripts: Scripts,
    global_ignore: OnceLock<Gitignore>,
}

/// The start of a snapshot file, its format version and the size of its head.
const SNAPSHOT_MAGIC: &[u8] = b"TKFILES\0";
const SNAPSHOT_VERSION: u32 = 1;
const SNAPSHOT_HEAD: usize = 8 + 4 + 8 + 4;

/// Little-endian reads off the front of a byte slice; `None` past its end.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
}

fn name_key(parent: EntryId, name: &OsStr) -> u32 {
    let mut h = DefaultHasher::new();
    parent.hash(&mut h);
    name.hash(&mut h);
    h.finish() as u32
}

impl FileIndex {
    /// Walks every root of `rules`; the match keys are then computed on all cores.
    pub fn walk(rules: &Rules) -> FileIndex {
        let mut index = FileIndex::default();
        let Some((first, rest)) = rules.roots().split_first() else {
            return index;
        };
        let mut builder = walker(first, rules);
        for root in rest {
            builder.add(root);
        }
        // One walking thread, depth first: the parallel walker keeps far more directories
        // (and their ignore matchers) in flight, and glibc keeps that memory resident in its
        // per-thread arenas after the walk (150 MB and more instead of 50 MB for 450k entries
        // in the bench). Paths are gathered as bytes in one buffer, not one allocation each.
        let mut batch = Batch::default();
        for entry in builder.build().flatten() {
            batch.push(&entry);
        }
        let mut order: Vec<u32> = (0..batch.found.len() as u32).collect();
        let bytes = |&f: &u32| batch.bytes(f as usize);
        // A parent's path sorts before its descendants' (it is their prefix), and a path
        // under two nested roots is kept once.
        order.par_sort_unstable_by(|a, b| bytes(a).cmp(bytes(b)));
        order.dedup_by(|a, b| bytes(a) == bytes(b));
        index.entries.reserve_exact(order.len());
        index.by_name.reserve(order.len());
        // Last components, plus the full paths the roots store.
        let roots_len: usize = rules
            .roots()
            .iter()
            .map(|r| os_bytes(r.as_os_str()).len())
            .sum();
        let names_len: usize = order.iter().map(|f| shown_len(bytes(f))).sum();
        index.names.reserve_exact(names_len + roots_len);
        for f in &order {
            let walked = &batch.found[*f as usize];
            let path = Path::new(os_str(bytes(f)));
            index.insert_keyed(path, walked.is_dir, walked.mtime, NameKeys::default());
        }
        drop(order);
        drop(batch);
        index.fill_keys();
        index
    }

    /// The alive entries as bytes, parents first, for [`from_snapshot`](Self::from_snapshot):
    /// `fingerprint` names the coverage they were walked under.
    pub fn to_snapshot(&self, fingerprint: u64) -> Vec<u8> {
        let mut out = Vec::with_capacity(SNAPSHOT_HEAD + self.alive * 24 + self.names.len());
        out.extend_from_slice(SNAPSHOT_MAGIC);
        out.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        out.extend_from_slice(&fingerprint.to_le_bytes());
        let count_at = out.len();
        out.extend_from_slice(&0u32.to_le_bytes());
        // Old id to position in the snapshot; a parent always has a smaller id than its
        // children (it is inserted first), so one pass in id order sees it first.
        let mut position = vec![NO_PARENT; self.entries.len()];
        let mut count: u32 = 0;
        for (id, e) in self.entries.iter().enumerate() {
            if !e.alive {
                continue;
            }
            let parent = match e.parent {
                NO_PARENT => NO_PARENT,
                p => match position[p as usize] {
                    // Its folder is gone: the entry is dropped with it.
                    NO_PARENT => continue,
                    at => at,
                },
            };
            let name = os_bytes(self.raw_name(e));
            out.extend_from_slice(&parent.to_le_bytes());
            out.push(u8::from(e.is_dir));
            out.extend_from_slice(&e.mtime.to_le_bytes());
            out.extend_from_slice(&(name.len() as u32).to_le_bytes());
            out.extend_from_slice(name);
            position[id] = count;
            count += 1;
        }
        out[count_at..count_at + 4].copy_from_slice(&count.to_le_bytes());
        out
    }

    /// An index read back from [`to_snapshot`](Self::to_snapshot)'s bytes; `None` when they
    /// are of another version or coverage, or damaged.
    pub fn from_snapshot(bytes: &[u8], fingerprint: u64) -> Option<FileIndex> {
        let mut r = Reader(bytes);
        if r.take(SNAPSHOT_MAGIC.len())? != SNAPSHOT_MAGIC
            || r.u32()? != SNAPSHOT_VERSION
            || r.u64()? != fingerprint
        {
            return None;
        }
        let count = r.u32()? as usize;
        let mut index = FileIndex::default();
        // At most what the bytes can hold: a damaged count must not reserve gigabytes.
        let room = count.min(r.0.len() / 17);
        index.entries.reserve_exact(room);
        index.by_name.reserve(room);
        // The folders' paths by snapshot position, for their children's paths.
        let mut dir_paths: Vec<Option<PathBuf>> = Vec::with_capacity(room);
        for _ in 0..count {
            let parent = r.u32()?;
            let is_dir = r.u8()? != 0;
            let mtime = r.i64()?;
            let len = r.u32()? as usize;
            let name = os_str(r.take(len)?);
            let path = match parent {
                NO_PARENT => PathBuf::from(name),
                p => dir_paths.get(p as usize)?.as_ref()?.join(name),
            };
            // A name that does not name one entry under its parent: not ours.
            if parent != NO_PARENT && path.parent().and_then(|p| index.dirs.get(p)).is_none() {
                return None;
            }
            index.insert_keyed(&path, is_dir, mtime, NameKeys::default())?;
            dir_paths.push(is_dir.then_some(path));
        }
        if !r.0.is_empty() {
            return None;
        }
        drop(dir_paths);
        index.fill_keys();
        Some(index)
    }

    /// Computes every alive entry's match keys, on all cores.
    fn fill_keys(&mut self) {
        let names = &self.names;
        let texts: Vec<(EntryId, TextKeys)> = self
            .entries
            .par_iter_mut()
            .enumerate()
            .with_min_len(4096)
            .filter(|(_, e)| e.alive)
            .filter_map(|(id, e)| {
                let (keys, text) = NameKeys::new(shown(e, names));
                e.keys = keys;
                Some((id as EntryId, text?))
            })
            .collect();
        self.texts.extend(texts);
        self.scripts = self
            .entries
            .iter()
            .fold(self.scripts, |acc, e| acc.union(e.keys.scripts()));
    }

    /// The name as stored: a root's full path.
    fn raw_name(&self, e: &Entry) -> &OsStr {
        raw(e, &self.names)
    }

    /// The name as shown: a root's last component.
    pub(crate) fn name(&self, e: &Entry) -> &OsStr {
        shown(e, &self.names)
    }

    /// The core's match keys of a name in a script other than Latin.
    pub(crate) fn text_keys(&self, id: EntryId) -> Option<&TextKeys> {
        self.texts.get(&id)
    }

    /// Every entry, dead ones included; an entry's id is its position.
    pub(crate) fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The scripts of the names, for the query's readings.
    pub(crate) fn scripts(&self) -> Scripts {
        self.scripts
    }

    /// The id of an indexed path.
    pub fn id_of(&self, path: &Path) -> Option<EntryId> {
        self.find(path)
    }

    /// Alive entries.
    pub fn len(&self) -> usize {
        self.alive
    }

    pub fn is_empty(&self) -> bool {
        self.alive == 0
    }

    pub fn path(&self, id: EntryId) -> PathBuf {
        let mut names = Vec::new();
        let mut cur = id;
        loop {
            let e = &self.entries[cur as usize];
            names.push(self.raw_name(e));
            if e.parent == NO_PARENT {
                break;
            }
            cur = e.parent;
        }
        let mut path = PathBuf::new();
        for name in names.into_iter().rev() {
            path.push(name);
        }
        path
    }

    pub fn entry(&self, id: EntryId) -> EntryView<'_> {
        let e = &self.entries[id as usize];
        EntryView {
            name: self.name(e),
            is_dir: e.is_dir,
            mtime: e.mtime,
            depth: e.depth,
        }
    }

    /// Alive entries, parents before their children for a freshly walked index.
    pub fn ids(&self) -> impl Iterator<Item = EntryId> + '_ {
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.alive)
            .map(|(i, _)| i as EntryId)
    }

    /// Every indexed folder, roots included: what the watcher watches.
    pub(crate) fn folders(&self) -> impl ExactSizeIterator<Item = &Path> {
        self.dirs.keys().map(PathBuf::as_path)
    }

    pub(crate) fn is_folder(&self, path: &Path) -> bool {
        self.dirs.contains_key(path)
    }

    /// The indexed folders at and below `path` (none when `path` is not an indexed folder).
    pub(crate) fn folders_under(&self, path: &Path) -> Vec<PathBuf> {
        let Some(&top) = self.dirs.get(path) else {
            return Vec::new();
        };
        let mut found = vec![path.to_path_buf()];
        let mut stack = vec![top];
        while let Some(dir) = stack.pop() {
            for &kid in self.children.get(&dir).into_iter().flatten() {
                let e = &self.entries[kid as usize];
                if e.alive && e.is_dir {
                    found.push(self.path(kid));
                    stack.push(kid);
                }
            }
        }
        found
    }

    /// The paths of an indexed folder's entries (none when `path` is not an indexed folder).
    pub(crate) fn children_of(&self, path: &Path) -> Vec<PathBuf> {
        let Some(dir) = self.dirs.get(path) else {
            return Vec::new();
        };
        self.children
            .get(dir)
            .into_iter()
            .flatten()
            .filter(|&&kid| self.entries[kid as usize].alive)
            .map(|&kid| path.join(self.raw_name(&self.entries[kid as usize])))
            .collect()
    }

    /// Adds or refreshes one path, and walks it when it is a folder. Only that path is
    /// checked (never its folder re-read), against the same rules as `walk`: the name and
    /// prefix exclusions on every link below its root, and the ignore files of every ancestor.
    pub fn upsert(&mut self, path: &Path, rules: &Rules) {
        match self.plan(path, rules) {
            Upsert::Nothing => {}
            Upsert::Remove => self.remove(path),
            Upsert::Insert(found) => self.insert_found(&found),
        }
    }

    /// What `upsert` would do, with all its reading of the disk (a folder's whole subtree
    /// included) done here, so the caller can apply it in short steps.
    pub(crate) fn plan(&self, path: &Path, rules: &Rules) -> Upsert {
        // The innermost root, as `walk` treats a nested root as a root of its own.
        let Some(root) = rules
            .roots()
            .iter()
            .filter(|r| path.starts_with(r))
            .max_by_key(|r| r.components().count())
        else {
            return Upsert::Nothing;
        };
        let Ok(meta) = std::fs::symlink_metadata(path) else {
            return Upsert::Remove;
        };
        if path != root && !self.allowed(path, root, meta.is_dir(), rules) {
            return Upsert::Remove;
        }
        // The folders above the path that the index does not hold yet, down from the root.
        let mut missing = Vec::new();
        // A root has no folders above it to add.
        if path != root {
            for ancestor in path.ancestors().skip(1) {
                if self.find(ancestor).is_some() {
                    break;
                }
                missing.push(ancestor);
                if ancestor == root {
                    break;
                }
            }
        }
        let mut found = Vec::with_capacity(missing.len() + 1);
        for ancestor in missing.into_iter().rev() {
            let Ok(m) = std::fs::symlink_metadata(ancestor) else {
                return Upsert::Nothing;
            };
            found.push((ancestor.to_path_buf(), m.is_dir(), mtime_secs(&m)));
        }
        found.push((path.to_path_buf(), meta.is_dir(), mtime_secs(&meta)));
        if meta.is_dir() {
            found.extend(walk_contents(path, rules));
        }
        Upsert::Insert(found)
    }

    /// Inserts (or refreshes) entries in order; each one's folder must be indexed or come
    /// earlier in `found`.
    pub(crate) fn insert_found(&mut self, found: &[Found]) {
        for (path, is_dir, mtime) in found {
            self.insert(path, *is_dir, *mtime);
        }
    }

    /// Marks the entry and its subtree dead.
    pub fn remove(&mut self, path: &Path) {
        let Some(id) = self.find(path) else {
            return;
        };
        if self.entries[id as usize].parent == NO_PARENT {
            self.roots.retain(|&r| r != id);
        }
        // Collect the subtree first: paths are read from names, which are cleared below.
        let mut doomed = vec![id];
        let mut next = 0;
        while next < doomed.len() {
            if let Some(kids) = self.children.remove(&doomed[next]) {
                let entries = &self.entries;
                doomed.extend(kids.into_iter().filter(|&k| entries[k as usize].alive));
            }
            next += 1;
        }
        for &d in &doomed {
            if self.entries[d as usize].is_dir {
                let path = self.path(d);
                self.dirs.remove(&path);
            }
        }
        for d in doomed {
            let e = &mut self.entries[d as usize];
            e.alive = false;
            e.name_len = 0;
            e.keys = NameKeys::default();
            self.texts.remove(&d);
            self.alive -= 1;
        }
    }

    /// Whether the walker would show `path` (below `root`): not excluded by name or prefix on
    /// any link, not ignored by the ignore files of any ancestor or the global one.
    ///
    /// As the walker's `require_git`: `.gitignore`, `.git/info/exclude` and the global
    /// gitignore apply only when a folder above `path` is a repository, and then only the
    /// files up to the innermost repository's top folder; `.ignore` files apply everywhere.
    fn allowed(&self, path: &Path, root: &Path, is_dir: bool, rules: &Rules) -> bool {
        for link in path.ancestors().take_while(|p| *p != root) {
            if rules.excludes(link, link == path && is_dir || link != path) {
                return false;
            }
        }
        let dirs: Vec<(&Path, bool)> = path
            .ancestors()
            .skip(1)
            .map(|dir| (dir, is_repository(dir)))
            .collect();
        let in_repository = rules.gitignore() && dirs.iter().any(|&(_, repository)| repository);
        // Still inside the innermost repository, going up.
        let mut git_files = in_repository;
        // Deepest folder first; the first file with an opinion decides.
        for (dir, repository) in dirs {
            let mut files = vec![dir.join(".ignore")];
            if git_files {
                files.push(dir.join(".gitignore"));
                files.push(dir.join(".git/info/exclude"));
            }
            git_files &= !repository;
            for file in files {
                if !file.is_file() {
                    continue;
                }
                let mut b = GitignoreBuilder::new(dir);
                if b.add(&file).is_some() {
                    continue;
                }
                let Ok(ig) = b.build() else { continue };
                let m = ig.matched_path_or_any_parents(path, is_dir);
                if m.is_ignore() {
                    return false;
                }
                if m.is_whitelist() {
                    return true;
                }
            }
        }
        if !in_repository || !rules.global_ignore() {
            return true;
        }
        let global = self.global_ignore.get_or_init(|| Gitignore::global().0);
        match path.strip_prefix(root) {
            Ok(rel) if !rel.as_os_str().is_empty() => {
                !global.matched_path_or_any_parents(rel, is_dir).is_ignore()
            }
            _ => true,
        }
    }

    /// The (parent, name) an entry of `path` is stored under.
    fn slot_of<'a>(&self, path: &'a Path) -> (EntryId, &'a OsStr) {
        let parent = path.parent().and_then(|p| self.dirs.get(p)).copied();
        match (parent, path.file_name()) {
            (Some(p), Some(name)) => (p, name),
            _ => (NO_PARENT, path.as_os_str()),
        }
    }

    fn find(&self, path: &Path) -> Option<EntryId> {
        let (parent, name) = self.slot_of(path);
        let mut key = name_key(parent, name);
        loop {
            let id = *self.by_name.get(&key)?;
            let e = &self.entries[id as usize];
            if e.alive && e.parent == parent && self.raw_name(e) == name {
                return Some(id);
            }
            key = key.wrapping_add(1);
        }
    }

    /// Adds the path, or refreshes it when indexed; a changed kind replaces the entry.
    fn insert(&mut self, path: &Path, is_dir: bool, mtime: i64) {
        if let Some(id) = self.find(path) {
            let e = &mut self.entries[id as usize];
            if e.is_dir == is_dir {
                e.mtime = mtime;
                return;
            }
            self.remove(path);
        }
        self.insert_new(path, is_dir, mtime);
    }

    /// Adds a path known to be absent; its folder, if indexed, is already in `dirs`.
    fn insert_new(&mut self, path: &Path, is_dir: bool, mtime: i64) {
        let (keys, text) = NameKeys::new(shown_name(path));
        if let Some(id) = self.insert_keyed(path, is_dir, mtime, keys)
            && let Some(text) = text
        {
            self.texts.insert(id, text);
        }
    }

    /// `insert_new` with the name's keys given; `None` when the names no longer fit.
    fn insert_keyed(
        &mut self,
        path: &Path,
        is_dir: bool,
        mtime: i64,
        keys: NameKeys,
    ) -> Option<EntryId> {
        let (parent, name) = self.slot_of(path);
        let name = os_bytes(name);
        let (Ok(name_at), Ok(name_len)) =
            (u32::try_from(self.names.len()), u32::try_from(name.len()))
        else {
            tracing::warn!("files: the index's names are full; not indexing more");
            return None;
        };
        if name_at.checked_add(name_len).is_none() {
            tracing::warn!("files: the index's names are full; not indexing more");
            return None;
        }
        let depth = match parent {
            NO_PARENT => 0,
            p => self.entries[p as usize].depth.saturating_add(1),
        };
        let id = self.entries.len() as EntryId;
        let mut key = name_key(parent, os_str(name));
        loop {
            match self.by_name.entry(key) {
                MapEntry::Vacant(v) => {
                    v.insert(id);
                    break;
                }
                MapEntry::Occupied(mut o) => {
                    if !self.entries[*o.get() as usize].alive {
                        o.insert(id);
                        break;
                    }
                    key = key.wrapping_add(1);
                }
            }
        }
        self.scripts = self.scripts.union(keys.scripts());
        self.names.extend_from_slice(name);
        self.entries.push(Entry {
            keys,
            mtime,
            name_at,
            name_len,
            parent,
            depth,
            is_dir,
            alive: true,
        });
        self.alive += 1;
        if parent == NO_PARENT {
            self.roots.push(id);
        } else {
            self.children.entry(parent).or_default().push(id);
        }
        if is_dir {
            self.dirs.insert(path.to_path_buf(), id);
        }
        Some(id)
    }
}

fn raw<'a>(e: &Entry, names: &'a [u8]) -> &'a OsStr {
    let at = e.name_at as usize;
    os_str(&names[at..at + e.name_len as usize])
}

fn shown<'a>(e: &Entry, names: &'a [u8]) -> &'a OsStr {
    let name = raw(e, names);
    if e.parent == NO_PARENT {
        shown_name(Path::new(name))
    } else {
        name
    }
}

/// The bytes of a path's last component (what its entry stores, unless it is a root).
fn shown_len(path: &[u8]) -> usize {
    os_bytes(shown_name(Path::new(os_str(path)))).len()
}

/// What a walk found: the paths' bytes end to end, and per path where its bytes start and
/// what it is.
#[derive(Default)]
struct Batch {
    paths: Vec<u8>,
    found: Vec<Walked>,
}

struct Walked {
    at: usize,
    len: usize,
    is_dir: bool,
    mtime: i64,
}

impl Batch {
    fn push(&mut self, entry: &ignore::DirEntry) {
        let (is_dir, mtime) = kind_of(entry);
        let bytes = os_bytes(entry.path().as_os_str());
        self.found.push(Walked {
            at: self.paths.len(),
            len: bytes.len(),
            is_dir,
            mtime,
        });
        self.paths.extend_from_slice(bytes);
    }

    fn bytes(&self, f: usize) -> &[u8] {
        let w = &self.found[f];
        &self.paths[w.at..w.at + w.len]
    }
}

/// The last component of a path, or the whole path when it has none.
fn shown_name(path: &Path) -> &OsStr {
    path.file_name().unwrap_or(path.as_os_str())
}

/// The folder's contents (not the folder itself), parents first, with the walker settings of
/// `walk`.
fn walk_contents(dir: &Path, rules: &Rules) -> Vec<Found> {
    let mut found = Vec::new();
    for entry in walker(dir, rules).build().flatten() {
        if entry.depth() == 0 {
            continue;
        }
        found.push(found_of(&entry));
    }
    found.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    found
}

fn found_of(entry: &ignore::DirEntry) -> Found {
    let (is_dir, mtime) = kind_of(entry);
    (entry.path().to_path_buf(), is_dir, mtime)
}

/// Whether the entry is a folder, and its modification time.
fn kind_of(entry: &ignore::DirEntry) -> (bool, i64) {
    let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
    let mtime = entry.metadata().map_or(0, |m| mtime_secs(&m));
    (is_dir, mtime)
}

pub(crate) fn mtime_secs(meta: &Metadata) -> i64 {
    match meta.modified().map(|t| t.duration_since(UNIX_EPOCH)) {
        Ok(Ok(d)) => d.as_secs() as i64,
        Ok(Err(e)) => -(e.duration().as_secs() as i64),
        Err(_) => 0,
    }
}

/// Whether `dir` is the top folder of a repository, as the walker tells: a `.git` folder or
/// file (a worktree, a submodule), or a `.jj` folder.
fn is_repository(dir: &Path) -> bool {
    dir.join(".git").exists() || dir.join(".jj").exists()
}

/// The one walker configuration of the module, rooted at `root`.
fn walker(root: &Path, rules: &Rules) -> WalkBuilder {
    let rules = Arc::new(rules.clone());
    let mut b = WalkBuilder::new(root);
    b.hidden(!rules.hidden())
        .git_ignore(rules.gitignore())
        .git_global(rules.global_ignore())
        .git_exclude(rules.gitignore())
        .ignore(true)
        .parents(true)
        // .gitignore, info/exclude and the global gitignore only inside a repository, as
        // `fd` does; `allowed` mirrors it.
        .require_git(true)
        .follow_links(false)
        .same_file_system(true)
        .filter_entry(move |e| {
            e.depth() == 0 || !rules.excludes(e.path(), e.file_type().is_some_and(|t| t.is_dir()))
        });
    b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Coverage;
    use std::collections::BTreeSet;
    use std::fs;
    use tempfile::TempDir;

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    fn rules_for(tmp: &Path, exclude: &[&str], hidden: bool) -> Rules {
        let c = Coverage {
            roots: vec![tmp.to_str().unwrap().to_string()],
            exclude: exclude.iter().map(|s| s.to_string()).collect(),
            hidden,
            gitignore: true,
            global_ignore: false,
        };
        Rules::new(&c, Path::new("/nonexistent-home"))
    }

    fn paths(idx: &FileIndex) -> BTreeSet<PathBuf> {
        idx.ids().map(|id| idx.path(id)).collect()
    }

    fn set(tmp: &Path, rel: &[&str]) -> BTreeSet<PathBuf> {
        let mut s: BTreeSet<PathBuf> = rel.iter().map(|r| tmp.join(r)).collect();
        s.insert(tmp.to_path_buf());
        s
    }

    fn tree(tmp: &Path) {
        touch(&tmp.join("a.txt"));
        touch(&tmp.join(".secret/x"));
        touch(&tmp.join("node_modules/m.js"));
        // A repository: its .gitignore applies.
        fs::create_dir_all(tmp.join("proj/.git")).unwrap();
        touch(&tmp.join("proj/.gitignore"));
        fs::write(tmp.join("proj/.gitignore"), "out/\n").unwrap();
        touch(&tmp.join("proj/out/o.bin"));
        touch(&tmp.join("proj/src/main.rs"));
    }

    #[test]
    fn walk_skips_hidden_ignored_and_builtin_names() {
        let tmp = TempDir::new().unwrap();
        tree(tmp.path());
        let idx = FileIndex::walk(&rules_for(tmp.path(), &[], false));
        assert_eq!(
            paths(&idx),
            set(
                tmp.path(),
                &["a.txt", "proj", "proj/src", "proj/src/main.rs"]
            )
        );
        assert_eq!(idx.len(), 5);
        let root = idx.ids().next().unwrap();
        assert!(idx.entry(root).is_dir);
        assert_eq!(idx.entry(root).depth, 0);
        let main = idx
            .ids()
            .find(|&i| idx.path(i).ends_with("main.rs"))
            .unwrap();
        let v = idx.entry(main);
        assert_eq!(v.name, OsStr::new("main.rs"));
        assert!(!v.is_dir);
        assert_eq!(v.depth, 3);
        assert!(v.mtime > 0);
    }

    #[test]
    fn hidden_true_includes_dot_entries() {
        let tmp = TempDir::new().unwrap();
        tree(tmp.path());
        let idx = FileIndex::walk(&rules_for(tmp.path(), &[], true));
        let p = paths(&idx);
        assert!(p.contains(&tmp.path().join(".secret/x")));
        assert!(p.contains(&tmp.path().join("proj/.gitignore")));
        assert!(!p.contains(&tmp.path().join("node_modules")));
        assert!(!p.contains(&tmp.path().join("proj/out")));
    }

    #[test]
    fn exclude_by_path_and_by_glob() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join("games/doom.wad"));
        touch(&tmp.path().join("disk.iso"));
        touch(&tmp.path().join("keep/x.iso"));
        touch(&tmp.path().join("keep/y.txt"));
        let games = tmp.path().join("games");
        let idx = FileIndex::walk(&rules_for(
            tmp.path(),
            &[games.to_str().unwrap(), "*.iso"],
            false,
        ));
        assert_eq!(paths(&idx), set(tmp.path(), &["keep", "keep/y.txt"]));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_listed_not_followed() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join("real/inner.txt"));
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("link")).unwrap();
        let idx = FileIndex::walk(&rules_for(tmp.path(), &[], false));
        assert_eq!(
            paths(&idx),
            set(tmp.path(), &["real", "real/inner.txt", "link"])
        );
        let link = idx.ids().find(|&i| idx.path(i).ends_with("link")).unwrap();
        assert!(!idx.entry(link).is_dir);
    }

    #[test]
    fn upsert_and_remove_subtree() {
        let tmp = TempDir::new().unwrap();
        tree(tmp.path());
        let rules = rules_for(tmp.path(), &[], false);
        let mut idx = FileIndex::walk(&rules);

        idx.remove(&tmp.path().join("proj"));
        assert_eq!(paths(&idx), set(tmp.path(), &["a.txt"]));
        assert_eq!(idx.len(), 2);

        // The removed folders are gone from the lookup too: the same paths can come back.
        let back = tmp.path().join("proj/src/main.rs");
        idx.upsert(&back, &rules);
        assert_eq!(
            paths(&idx),
            set(
                tmp.path(),
                &["a.txt", "proj", "proj/src", "proj/src/main.rs"]
            )
        );
        idx.remove(&tmp.path().join("proj"));
        assert_eq!(idx.len(), 2);

        let new = tmp.path().join("fresh/deep/new.txt");
        touch(&new);
        idx.upsert(&new, &rules);
        assert_eq!(
            paths(&idx),
            set(
                tmp.path(),
                &["a.txt", "fresh", "fresh/deep", "fresh/deep/new.txt"]
            )
        );
        assert_eq!(idx.len(), 5);

        // Refreshing an indexed path does not duplicate it.
        idx.upsert(&new, &rules);
        assert_eq!(idx.len(), 5);

        // A directory is walked.
        touch(&tmp.path().join("dir2/one.txt"));
        touch(&tmp.path().join("dir2/sub/two.txt"));
        idx.upsert(&tmp.path().join("dir2"), &rules);
        assert!(paths(&idx).contains(&tmp.path().join("dir2/sub/two.txt")));
        assert_eq!(idx.len(), 9);

        // A path that vanished is dropped by upsert.
        fs::remove_file(&new).unwrap();
        idx.upsert(&new, &rules);
        assert!(!paths(&idx).contains(&new));
    }

    #[test]
    fn folders_follow_the_index() {
        let tmp = TempDir::new().unwrap();
        tree(tmp.path());
        let mut idx = FileIndex::walk(&rules_for(tmp.path(), &[], false));
        let all: BTreeSet<PathBuf> = idx.folders().map(Path::to_path_buf).collect();
        assert_eq!(all, set(tmp.path(), &["proj", "proj/src"]));
        assert_eq!(idx.folders().len(), 3);
        assert!(idx.is_folder(&tmp.path().join("proj/src")));
        assert!(!idx.is_folder(&tmp.path().join("a.txt")));
        assert!(!idx.is_folder(&tmp.path().join("proj/out")));

        let under: BTreeSet<PathBuf> = idx
            .folders_under(&tmp.path().join("proj"))
            .into_iter()
            .collect();
        let want: BTreeSet<PathBuf> = ["proj", "proj/src"]
            .iter()
            .map(|r| tmp.path().join(r))
            .collect();
        assert_eq!(under, want);
        assert!(idx.folders_under(&tmp.path().join("a.txt")).is_empty());

        let kids: BTreeSet<PathBuf> = idx
            .children_of(&tmp.path().join("proj"))
            .into_iter()
            .collect();
        assert_eq!(kids, BTreeSet::from([tmp.path().join("proj/src")]));
        assert!(idx.children_of(&tmp.path().join("a.txt")).is_empty());

        idx.remove(&tmp.path().join("proj"));
        assert!(idx.folders_under(&tmp.path().join("proj")).is_empty());
        assert!(idx.children_of(&tmp.path().join("proj")).is_empty());
        assert_eq!(idx.folders().len(), 1);
    }

    #[test]
    fn snapshot_round_trips_and_refuses_another_coverage_or_damage() {
        let tmp = TempDir::new().unwrap();
        tree(tmp.path());
        touch(&tmp.path().join("Отчёт.pdf"));
        let rules = rules_for(tmp.path(), &[], false);
        let mut idx = FileIndex::walk(&rules);
        // A removed entry and a later insert, as the watcher leaves them.
        idx.remove(&tmp.path().join("a.txt"));
        touch(&tmp.path().join("proj/src/lib.rs"));
        idx.upsert(&tmp.path().join("proj/src/lib.rs"), &rules);

        let bytes = idx.to_snapshot(7);
        let back = FileIndex::from_snapshot(&bytes, 7).expect("read back");
        assert_eq!(paths(&back), paths(&idx));
        assert_eq!(back.len(), idx.len());
        assert_eq!(back.scripts(), idx.scripts());
        let report = back.id_of(&tmp.path().join("Отчёт.pdf")).unwrap();
        assert!(back.text_keys(report).is_some(), "match keys are filled");

        assert!(
            FileIndex::from_snapshot(&bytes, 8).is_none(),
            "another coverage"
        );
        assert!(FileIndex::from_snapshot(&bytes[..bytes.len() - 1], 7).is_none());
        assert!(FileIndex::from_snapshot(b"junk", 7).is_none());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(FileIndex::from_snapshot(&longer, 7).is_none());
    }

    #[test]
    fn upsert_respects_gitignore() {
        let tmp = TempDir::new().unwrap();
        tree(tmp.path());
        let rules = rules_for(tmp.path(), &[], false);
        let mut idx = FileIndex::walk(&rules);
        let before = idx.len();
        let ignored = tmp.path().join("proj/out/new.bin");
        touch(&ignored);
        idx.upsert(&ignored, &rules);
        idx.upsert(&tmp.path().join("proj/out"), &rules);
        assert_eq!(idx.len(), before);
        let visible = tmp.path().join("proj/src/ok.rs");
        touch(&visible);
        idx.upsert(&visible, &rules);
        assert_eq!(idx.len(), before + 1);
    }

    /// Whether `path` is in the walked index, and whether `upsert` puts it back after it was
    /// removed: the walk and a watcher event must agree.
    fn seen(rules: &Rules, path: &Path) -> (bool, bool) {
        let mut idx = FileIndex::walk(rules);
        let walked = idx.id_of(path).is_some();
        idx.remove(path);
        idx.upsert(path, rules);
        (walked, idx.id_of(path).is_some())
    }

    #[test]
    fn gitignore_off_shows_ignored_files_but_not_builtin_names_or_dot_ignore() {
        let tmp = TempDir::new().unwrap();
        tree(tmp.path());
        fs::write(tmp.path().join("proj/.ignore"), "skip/\n").unwrap();
        touch(&tmp.path().join("proj/skip/s.txt"));
        let on = rules_for(tmp.path(), &[], false);
        let off = Rules::new(
            &Coverage {
                gitignore: false,
                global_ignore: true,
                ..Coverage {
                    roots: vec![tmp.path().to_str().unwrap().to_string()],
                    ..Coverage::default()
                }
            },
            Path::new("/nonexistent-home"),
        );
        assert!(!off.global_ignore());
        let ignored = tmp.path().join("proj/out/o.bin");
        assert_eq!(seen(&on, &ignored), (false, false));
        assert_eq!(seen(&off, &ignored), (true, true));
        // The built-in names and `.ignore` files still apply.
        assert_eq!(
            seen(&off, &tmp.path().join("node_modules/m.js")),
            (false, false)
        );
        assert_eq!(
            seen(&off, &tmp.path().join("proj/skip/s.txt")),
            (false, false)
        );
    }

    #[test]
    fn gitignore_outside_a_repository_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let notes = tmp.path().join("notes");
        fs::create_dir_all(&notes).unwrap();
        fs::write(notes.join(".gitignore"), "*\n").unwrap();
        touch(&notes.join("a.txt"));
        let rules = rules_for(tmp.path(), &[], false);
        assert_eq!(seen(&rules, &notes.join("a.txt")), (true, true));

        fs::create_dir_all(notes.join(".git")).unwrap();
        assert_eq!(seen(&rules, &notes.join("a.txt")), (false, false));
        // A `.git` file (a worktree or a submodule) marks a repository too.
        fs::remove_dir(notes.join(".git")).unwrap();
        fs::write(notes.join(".git"), "gitdir: /elsewhere\n").unwrap();
        assert_eq!(seen(&rules, &notes.join("a.txt")), (false, false));
        // So does a Jujutsu one, for the walker.
        fs::remove_file(notes.join(".git")).unwrap();
        fs::create_dir_all(notes.join(".jj")).unwrap();
        assert_eq!(seen(&rules, &notes.join("a.txt")), (false, false));
    }

    #[test]
    fn gitignore_above_a_repository_applies_neither_inside_nor_outside_it() {
        let tmp = TempDir::new().unwrap();
        let outer = tmp.path().join("outer");
        fs::create_dir_all(outer.join("repo/.git")).unwrap();
        fs::write(outer.join(".gitignore"), "*.log\n").unwrap();
        fs::write(outer.join("repo/.gitignore"), "*.bak\n").unwrap();
        touch(&outer.join("y.log"));
        touch(&outer.join("repo/x.log"));
        touch(&outer.join("repo/x.bak"));
        let rules = rules_for(tmp.path(), &[], false);
        assert_eq!(seen(&rules, &outer.join("y.log")), (true, true));
        assert_eq!(seen(&rules, &outer.join("repo/x.log")), (true, true));
        assert_eq!(seen(&rules, &outer.join("repo/x.bak")), (false, false));
    }

    #[test]
    fn dot_ignore_applies_everywhere() {
        let tmp = TempDir::new().unwrap();
        let plain = tmp.path().join("plain");
        fs::create_dir_all(&plain).unwrap();
        fs::write(plain.join(".ignore"), "*.tmp\n").unwrap();
        touch(&plain.join("a.tmp"));
        touch(&plain.join("b.txt"));
        // Above a repository, and so past its boundary.
        fs::create_dir_all(plain.join("repo/.git")).unwrap();
        touch(&plain.join("repo/c.tmp"));
        let rules = rules_for(tmp.path(), &[], false);
        assert_eq!(seen(&rules, &plain.join("a.tmp")), (false, false));
        assert_eq!(seen(&rules, &plain.join("b.txt")), (true, true));
        assert_eq!(seen(&rules, &plain.join("repo/c.tmp")), (false, false));
    }

    #[test]
    fn upsert_under_a_nested_hidden_root() {
        let tmp = TempDir::new().unwrap();
        let nested = tmp.path().join(".config/foo");
        touch(&nested.join("old.txt"));
        touch(&tmp.path().join("a.txt"));
        let c = Coverage {
            roots: vec![
                tmp.path().to_str().unwrap().to_string(),
                nested.to_str().unwrap().to_string(),
            ],
            exclude: vec![],
            hidden: false,
            gitignore: true,
            global_ignore: false,
        };
        let rules = Rules::new(&c, Path::new("/nonexistent-home"));
        let mut idx = FileIndex::walk(&rules);
        let new = nested.join("new.txt");
        touch(&new);
        idx.upsert(&new, &rules);
        assert!(paths(&idx).contains(&new));
        assert!(paths(&idx).contains(&nested.join("old.txt")));
        let other = tmp.path().join(".config/other.txt");
        touch(&other);
        idx.upsert(&other, &rules);
        assert!(!paths(&idx).contains(&other));
    }

    #[test]
    fn upsert_many_in_one_folder_is_fast() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("big");
        fs::create_dir_all(&dir).unwrap();
        for i in 0..20_000 {
            fs::write(dir.join(format!("f{i}.dat")), b"").unwrap();
        }
        let rules = rules_for(tmp.path(), &[], false);
        let mut idx = FileIndex::walk(&rules);
        assert_eq!(idx.len(), 20_002);
        let new: Vec<PathBuf> = (0..2_000).map(|i| dir.join(format!("n{i}.dat"))).collect();
        for p in &new {
            fs::write(p, b"").unwrap();
        }
        let started = std::time::Instant::now();
        for p in &new {
            idx.upsert(p, &rules);
        }
        let took = started.elapsed();
        assert_eq!(idx.len(), 22_002);
        assert!(took < std::time::Duration::from_secs(1), "{took:?}");
    }

    #[test]
    fn upsert_of_a_missing_root_adds_only_the_root() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("later");
        let rules = rules_for(&root, &[], false);
        let mut idx = FileIndex::walk(&rules);
        assert_eq!(idx.len(), 0);
        touch(&root.join("f.txt"));
        idx.upsert(&root, &rules);
        assert_eq!(paths(&idx), set(&root, &["f.txt"]));
    }

    #[test]
    fn upsert_outside_roots_is_ignored() {
        let tmp = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        touch(&other.path().join("f.txt"));
        let rules = rules_for(tmp.path(), &[], false);
        let mut idx = FileIndex::walk(&rules);
        idx.upsert(&other.path().join("f.txt"), &rules);
        assert_eq!(idx.len(), 1);
    }
}
