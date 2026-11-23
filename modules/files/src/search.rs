//! Search over the index: names matched with the core's matcher (so layouts and scripts work
//! as for apps), ranked with the module's own signals.
//!
//! Every name carries a 64-bit set of its characters (and one of its transliteration
//! skeleton's). A name is scored only when its set holds every letter and digit of the query,
//! of one of the query's layout readings, or (in the skeleton set) of the query's skeleton.
//! Fuzzy matching needs each of those characters somewhere in the text, so the check never
//! drops a match. The scan runs on every core, one matcher per thread.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use nucleo_matcher::chars::{graphemes, normalize, to_lower_case};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use tomakast_core::{
    Matcher, Pattern, ReadingKind, Scripts, TextKeys, is_hidden_char, skeleton, skeleton_alts,
};

use crate::index::{Entry, EntryId, FileIndex};
use crate::names::{display_name, display_parent};

/// What lifts a result besides its match.
pub struct Signals<'a> {
    /// Paths opened through the launcher, as the module records them.
    pub opened: &'a HashMap<PathBuf, Opened>,
    /// Paths in KDE's recently-used.xbel (Linux), else empty.
    pub recent: &'a HashSet<PathBuf>,
    /// `~` of the parent part of a query (`proj/read`), as in subtitles.
    pub home: &'a Path,
    /// Seconds since the Unix epoch.
    pub now: i64,
}

/// How often and when a path was last opened through the launcher.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opened {
    pub count: u32,
    pub last: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Found {
    pub id: EntryId,
    /// The match score scaled by the signals (see [`ranked`]); only for ordering the module's
    /// own results, not on the scale of match scores.
    pub score: u32,
}

const DAY: i64 = 86_400;
/// Opened through the launcher: the strongest lift, more with each open (up to
/// `OPENED_MAX_COUNT`) and with a recent last open.
const OPENED_BONUS: u32 = 60;
const OPENED_PER_OPEN: u32 = 6;
const OPENED_MAX_COUNT: u32 = 10;
const OPENED_RECENCY: [(i64, u32); 3] = [(DAY, 30), (7 * DAY, 20), (30 * DAY, 10)];
/// In KDE's recently used list.
const RECENT_LIST_BONUS: u32 = 40;
/// Modified lately.
const MODIFIED_RECENCY: [(i64, u32); 4] = [(DAY, 20), (7 * DAY, 14), (30 * DAY, 8), (365 * DAY, 3)];
/// Shallower: this much at depth 0, one less per level.
const DEPTH_BONUS: u32 = 10;
/// The parent part of a `proj/read` query adds this share of its score.
const PARENT_SHARE: u32 = 4;
/// Lifts scale the match score: each point of lift adds 1/`LIFT_SCALE` of it. All lifts
/// together (at most about 220) add up to about half, so they order comparable matches and
/// let habit win among them, but never lift a scattered match over a clearly better name.
const LIFT_SCALE: u32 = 400;
/// Entries per parallel task.
const CHUNK: usize = 2048;

thread_local! {
    static MATCHER: RefCell<Matcher> = RefCell::new(Matcher::new());
}

/// Keys that hold nothing: what a name's keys amount to when the query cannot use them.
static NO_KEYS: LazyLock<TextKeys> = LazyLock::new(|| TextKeys::new(""));

/// The match data of one name, kept in the index.
#[derive(Default)]
pub(crate) struct NameKeys {
    /// The characters of the name, as the matcher compares them.
    name_mask: u64,
    /// The characters of the name's transliteration skeletons; all bits for a name in
    /// several scripts.
    skel_mask: u64,
    scripts: Scripts,
}

impl NameKeys {
    /// The keys, and the core's keys when the name has a script other than Latin (the index
    /// keeps those aside). A Latin name's core keys matter only to a query in another script
    /// and are rebuilt for the names such a query reaches.
    pub(crate) fn new(name: &OsStr) -> (NameKeys, Option<TextKeys>) {
        let text = match_text(name);
        let scripts = Scripts::of(&text);
        let skel_mask = if scripts == Scripts::default() {
            0
        } else if scripts.single().is_some() {
            skeleton_alts(&text)
                .iter()
                .fold(haystack_mask(&skeleton(&text)), |m, a| m | haystack_mask(a))
        } else {
            u64::MAX
        };
        let text_keys = (scripts != Scripts::default() && scripts != Scripts::LATIN)
            .then(|| TextKeys::new(&text));
        let keys = NameKeys {
            name_mask: haystack_mask(&text),
            skel_mask,
            scripts,
        };
        (keys, text_keys)
    }

    pub(crate) fn scripts(&self) -> Scripts {
        self.scripts
    }
}

/// The text a name is matched as: its display name, borrowed when the name is clean.
fn match_text(name: &OsStr) -> Cow<'_, str> {
    match name.to_str() {
        Some(s) if !s.chars().any(is_hidden_char) => Cow::Borrowed(s),
        _ => Cow::Owned(display_name(name)),
    }
}

/// One bit per letter and digit, the rest folded into the remaining 28 bits.
fn bit(c: char) -> u64 {
    let i = match c {
        'a'..='z' => c as u32 - 'a' as u32,
        '0'..='9' => 26 + (c as u32 - '0' as u32),
        _ => 36 + c as u32 % 28,
    };
    1 << i
}

/// Every character of a text as the matcher may compare it: lowercased, and lowercased after
/// the Latin normalization (on unless the query holds a character it would change).
fn haystack_mask(text: &str) -> u64 {
    text.chars().fold(0, |m, c| {
        m | bit(to_lower_case(c)) | bit(to_lower_case(normalize(c)))
    })
}

/// The characters a match must contain: the letters and digits of the pattern as the matcher
/// keeps them (lowercased, the first of each grapheme). Punctuation and spaces are left out,
/// since whitespace splits words and `\ ` escapes a space.
fn needle_mask(text: &str) -> u64 {
    graphemes(text)
        .filter(|c| c.is_alphanumeric())
        .fold(0, |m, c| m | bit(to_lower_case(c)))
}

/// A parsed query with the characters each of its readings needs.
struct Query {
    pattern: Pattern,
    typed: u64,
    layouts: Vec<(Scripts, u64)>,
    layout_scripts: Scripts,
    /// The query's script and the characters of its skeleton.
    skeleton: Option<(Scripts, u64)>,
}

impl Query {
    fn new(text: &str, scripts: Scripts, m: &mut Matcher) -> Query {
        let pattern = Pattern::across(text, scripts, m);
        let layouts: Vec<(Scripts, u64)> = pattern
            .readings()
            .iter()
            .filter(|r| r.kind == ReadingKind::Layout)
            .map(|r| (r.script, needle_mask(&r.text)))
            .collect();
        let layout_scripts = layouts
            .iter()
            .fold(Scripts::default(), |acc, (s, _)| acc.union(*s));
        // The core reads a skeleton only for a query in one script.
        let skeleton = Scripts::of(text)
            .single()
            .map(|script| (script, needle_mask(&skeleton(text))));
        Query {
            typed: needle_mask(text),
            pattern,
            layouts,
            layout_scripts,
            skeleton,
        }
    }

    /// Whether some reading of the query may match the name: false only when none can.
    fn reaches(&self, k: &NameKeys) -> bool {
        let covers = |need: u64, have: u64| need & !have == 0;
        covers(self.typed, k.name_mask)
            || self
                .layouts
                .iter()
                .any(|(s, need)| k.scripts.contains(*s) && covers(*need, k.name_mask))
            || self.skeleton.is_some_and(|(script, need)| {
                k.scripts.union(script) != script && covers(need, k.skel_mask)
            })
    }

    /// Whether the name's keys can change its score: a layout reading in the name's script,
    /// or a skeleton the name may have in another script.
    fn needs_keys(&self, k: &NameKeys) -> bool {
        k.scripts.contains(self.layout_scripts)
            || self
                .skeleton
                .is_some_and(|(script, _)| k.scripts.union(script) != script)
    }

    /// `kept` are the core keys the index keeps for the name, if any.
    fn score<'k>(
        &self,
        m: &mut Matcher,
        text: &str,
        k: &NameKeys,
        kept: impl FnOnce() -> Option<&'k TextKeys>,
    ) -> Option<u32> {
        if !self.needs_keys(k) {
            return m.score_text(&self.pattern, text, &NO_KEYS);
        }
        match kept() {
            Some(keys) => m.score_text(&self.pattern, text, keys),
            None => m.score_text(&self.pattern, text, &TextKeys::new(text)),
        }
    }
}

/// `proj/read`: the part before the last separator (when it holds more than spaces) and the
/// part after it.
fn split(query: &str) -> (Option<&str>, &str) {
    let is_sep = |c: char| c == '/' || (cfg!(windows) && c == '\\');
    match query.rfind(is_sep) {
        Some(i) => {
            let parent = &query[..i];
            let parent = (!parent.trim().is_empty()).then_some(parent);
            (parent, &query[i + 1..])
        }
        None => (None, query),
    }
}

/// The best `limit` entries whose name matches `query`, best first. A blank query lists
/// [`recent`].
pub fn search(index: &FileIndex, query: &str, limit: usize, signals: &Signals) -> Vec<Found> {
    if limit == 0 {
        return Vec::new();
    }
    let (parent_part, name_part) = split(query);
    if parent_part.is_none() && name_part.trim().is_empty() {
        return recent(index, limit, signals);
    }
    let scripts = index.scripts();
    let (name_q, parent_q) = MATCHER.with_borrow_mut(|m| {
        let name_q = Query::new(name_part, scripts, m);
        let parent_q = parent_part.map(|p| Pattern::across(p, scripts, m));
        (name_q, parent_q)
    });
    let lifts = lifts(index, signals);
    let hits: Vec<(u32, EntryId)> = index
        .entries()
        .par_chunks(CHUNK)
        .enumerate()
        .flat_map_iter(|(c, chunk)| {
            MATCHER.with_borrow_mut(|m| {
                let mut parents: HashMap<EntryId, Option<u32>> = HashMap::new();
                let mut out = Vec::new();
                for (i, e) in chunk.iter().enumerate() {
                    if !e.alive || !name_q.reaches(&e.keys) {
                        continue;
                    }
                    let id = (c * CHUNK + i) as EntryId;
                    let text = match_text(index.name(e));
                    let kept = || index.text_keys(id);
                    let Some(mut score) = name_q.score(m, &text, &e.keys, kept) else {
                        continue;
                    };
                    if let Some(pq) = &parent_q {
                        let parent_score =
                            |m: &mut Matcher| parent_score(m, pq, index, id, signals.home);
                        let s = match e.parent() {
                            Some(p) => *parents.entry(p).or_insert_with(|| parent_score(m)),
                            None => parent_score(m),
                        };
                        let Some(s) = s else { continue };
                        score += s / PARENT_SHARE;
                    }
                    out.push((ranked(score, lift(e, id, &lifts, signals.now)), id));
                }
                out
            })
        })
        .collect();
    best(hits, limit)
        .into_iter()
        .map(|(score, id)| Found { id, score })
        .collect()
}

/// The score of the folder an entry lies in, shown as in subtitles (`~/x/y`).
fn parent_score(
    m: &mut Matcher,
    pattern: &Pattern,
    index: &FileIndex,
    id: EntryId,
    home: &Path,
) -> Option<u32> {
    let text = display_parent(&index.path(id), home);
    m.score_text(pattern, &text, &TextKeys::new(&text))
}

/// The `limit` highest by score, then by id; sorted.
fn best<K: Ord + Copy>(mut hits: Vec<(K, EntryId)>, limit: usize) -> Vec<(K, EntryId)> {
    let order = |a: &(K, EntryId), b: &(K, EntryId)| b.0.cmp(&a.0).then(a.1.cmp(&b.1));
    if hits.len() > limit {
        hits.select_nth_unstable_by(limit - 1, order);
        hits.truncate(limit);
    }
    hits.sort_unstable_by(order);
    hits
}

/// The lifts of the opened and recently listed entries, by id.
fn lifts(index: &FileIndex, signals: &Signals) -> HashMap<EntryId, u32> {
    let mut lifts: HashMap<EntryId, u32> = HashMap::new();
    for (path, opened) in signals.opened {
        if let Some(id) = index.id_of(path) {
            let count = opened.count.min(OPENED_MAX_COUNT);
            *lifts.entry(id).or_default() += OPENED_BONUS
                + OPENED_PER_OPEN * count
                + age_bonus(signals.now - opened.last, &OPENED_RECENCY);
        }
    }
    for path in signals.recent {
        if let Some(id) = index.id_of(path) {
            *lifts.entry(id).or_default() += RECENT_LIST_BONUS;
        }
    }
    lifts
}

/// The ranking score of a match: the match score grown by its lifts, in proportion. One is
/// added so a blank name part (`proj/`, score 0) is still ordered by its lifts.
fn ranked(score: u32, lift: u32) -> u32 {
    (score + 1).saturating_mul(LIFT_SCALE + lift)
}

fn lift(e: &Entry, id: EntryId, lifts: &HashMap<EntryId, u32>, now: i64) -> u32 {
    let listed = if lifts.is_empty() {
        0
    } else {
        lifts.get(&id).copied().unwrap_or(0)
    };
    listed
        + age_bonus(now - e.mtime, &MODIFIED_RECENCY)
        + DEPTH_BONUS.saturating_sub(u32::from(e.depth))
}

/// The bonus of the first step the age (seconds, a future time counting as now) is within.
fn age_bonus(age: i64, steps: &[(i64, u32)]) -> u32 {
    steps
        .iter()
        .find(|(within, _)| age <= *within)
        .map_or(0, |(_, bonus)| *bonus)
}

/// The empty-query list: entries opened through the launcher (latest open first), then those
/// in KDE's recent list (latest modified first), then the latest modified files.
pub fn recent(index: &FileIndex, limit: usize, signals: &Signals) -> Vec<Found> {
    if limit == 0 {
        return Vec::new();
    }
    let mut seen: HashSet<EntryId> = HashSet::new();
    let mut ids: Vec<EntryId> = Vec::new();
    let mut take = |ranked: Vec<(i64, EntryId)>, ids: &mut Vec<EntryId>| {
        for (_, id) in best(ranked, usize::MAX) {
            if ids.len() < limit && seen.insert(id) {
                ids.push(id);
            }
        }
    };
    let opened = signals
        .opened
        .iter()
        .filter_map(|(path, o)| Some((o.last, index.id_of(path)?)))
        .collect();
    take(opened, &mut ids);
    let listed = signals
        .recent
        .iter()
        .filter_map(|path| {
            let id = index.id_of(path)?;
            Some((index.entry(id).mtime, id))
        })
        .collect();
    take(listed, &mut ids);
    if ids.len() < limit {
        // Enough newest files to fill the list after skipping those already listed.
        let want = limit.saturating_add(ids.len());
        let newest: Vec<(i64, EntryId)> = index
            .entries()
            .par_chunks(CHUNK)
            .enumerate()
            .flat_map_iter(|(c, chunk)| {
                let files = chunk
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| e.alive && !e.is_dir)
                    .map(|(i, e)| (e.mtime, (c * CHUNK + i) as EntryId))
                    .collect();
                best(files, want)
            })
            .collect();
        take(best(newest, want), &mut ids);
    }
    let n = ids.len() as u32;
    ids.into_iter()
        .enumerate()
        .map(|(i, id)| Found {
            id,
            score: n - i as u32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::names::display_name;
    use crate::rules::{Coverage, Rules};
    use std::fs;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;
    use tomakast_core::{Matcher, Pattern, Scripts, TextKeys};

    const NOW: i64 = 1_800_000_000;

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    /// Creates the file with the given modification time (seconds since the epoch).
    fn touch_at(p: &Path, mtime: i64) {
        touch(p);
        let f = fs::File::options().write(true).open(p).unwrap();
        f.set_modified(UNIX_EPOCH + Duration::from_secs(mtime as u64))
            .unwrap();
    }

    fn rules_for(root: &Path) -> Rules {
        let c = Coverage {
            roots: vec![root.to_str().unwrap().to_string()],
            exclude: vec![],
            hidden: false,
            gitignore: true,
            global_ignore: false,
        };
        Rules::new(&c, Path::new("/nonexistent-home"))
    }

    fn index_of(root: &Path) -> FileIndex {
        FileIndex::walk(&rules_for(root))
    }

    struct Sig {
        opened: HashMap<PathBuf, Opened>,
        recent: HashSet<PathBuf>,
        home: PathBuf,
    }

    impl Sig {
        fn new(home: &Path) -> Sig {
            Sig {
                opened: HashMap::new(),
                recent: HashSet::new(),
                home: home.to_path_buf(),
            }
        }

        fn get(&self) -> Signals<'_> {
            Signals {
                opened: &self.opened,
                recent: &self.recent,
                home: &self.home,
                now: NOW,
            }
        }
    }

    fn paths(idx: &FileIndex, found: &[Found]) -> Vec<PathBuf> {
        found.iter().map(|f| idx.path(f.id)).collect()
    }

    fn top(idx: &FileIndex, query: &str, sig: &Sig) -> PathBuf {
        let found = search(idx, query, 10, &sig.get());
        assert!(!found.is_empty(), "nothing for {query:?}");
        idx.path(found[0].id)
    }

    #[test]
    fn finds_by_fuzzy_name() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join("proj/src/main.rs"));
        touch(&tmp.path().join("notes.md"));
        touch(&tmp.path().join("a.txt"));
        let idx = index_of(tmp.path());
        let sig = Sig::new(tmp.path());
        assert_eq!(top(&idx, "mnrs", &sig), tmp.path().join("proj/src/main.rs"));
        assert!(search(&idx, "zzzqx", 10, &sig.get()).is_empty());
    }

    #[test]
    fn finds_across_layouts_and_scripts() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join("fire.txt"));
        touch(&tmp.path().join("Отчёт.pdf"));
        touch(&tmp.path().join("other.txt"));
        let idx = index_of(tmp.path());
        let sig = Sig::new(tmp.path());
        assert_eq!(top(&idx, "ашку", &sig), tmp.path().join("fire.txt"));
        assert_eq!(top(&idx, "otchet", &sig), tmp.path().join("Отчёт.pdf"));
    }

    #[test]
    fn separator_matches_the_parent() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join("proj/x/README.md"));
        touch(&tmp.path().join("other/README.md"));
        let idx = index_of(tmp.path());
        let sig = Sig::new(tmp.path());
        let found = paths(&idx, &search(&idx, "proj/read", 10, &sig.get()));
        assert_eq!(found, vec![tmp.path().join("proj/x/README.md")]);
        // Without the separator both match.
        assert_eq!(search(&idx, "read", 10, &sig.get()).len(), 2);
        // An empty name part lists what lies under a matching parent.
        let under = paths(&idx, &search(&idx, "proj/", 10, &sig.get()));
        assert!(under.contains(&tmp.path().join("proj/x")), "{under:?}");
        assert!(!under.contains(&tmp.path().join("other/README.md")));
    }

    #[cfg(unix)]
    #[test]
    fn finds_a_name_that_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let tmp = TempDir::new().unwrap();
        let raw = tmp
            .path()
            .join(std::ffi::OsStr::from_bytes(b"caf\xe9\xff report.txt"));
        touch(&raw);
        touch(&tmp.path().join("other.txt"));
        let idx = index_of(tmp.path());
        let sig = Sig::new(tmp.path());
        // Found through its lossy text, opened by its real bytes.
        assert_eq!(top(&idx, "caf report", &sig), raw);
    }

    #[test]
    fn opened_beats_unopened_equal_match() {
        let tmp = TempDir::new().unwrap();
        touch_at(&tmp.path().join("a/report.txt"), NOW - 100);
        touch_at(&tmp.path().join("b/report.txt"), NOW - 100);
        let idx = index_of(tmp.path());
        for opened in ["a/report.txt", "b/report.txt"] {
            let mut sig = Sig::new(tmp.path());
            let p = tmp.path().join(opened);
            sig.opened.insert(
                p.clone(),
                Opened {
                    count: 1,
                    last: NOW - 50,
                },
            );
            assert_eq!(top(&idx, "report", &sig), p);
        }
    }

    #[test]
    fn recent_list_beats_plain_equal_match() {
        let tmp = TempDir::new().unwrap();
        touch_at(&tmp.path().join("a/report.txt"), NOW - 100);
        touch_at(&tmp.path().join("b/report.txt"), NOW - 100);
        let idx = index_of(tmp.path());
        for listed in ["a/report.txt", "b/report.txt"] {
            let mut sig = Sig::new(tmp.path());
            let p = tmp.path().join(listed);
            sig.recent.insert(p.clone());
            assert_eq!(top(&idx, "report", &sig), p);
        }
    }

    #[test]
    fn opened_beats_recent_list() {
        let tmp = TempDir::new().unwrap();
        touch_at(&tmp.path().join("a/report.txt"), NOW - 100);
        touch_at(&tmp.path().join("b/report.txt"), NOW - 100);
        let idx = index_of(tmp.path());
        let mut sig = Sig::new(tmp.path());
        let opened = tmp.path().join("b/report.txt");
        sig.opened.insert(
            opened.clone(),
            Opened {
                count: 1,
                last: NOW - 50,
            },
        );
        sig.recent.insert(tmp.path().join("a/report.txt"));
        assert_eq!(top(&idx, "report", &sig), opened);
    }

    #[test]
    fn recently_modified_beats_older_equal_match() {
        let tmp = TempDir::new().unwrap();
        touch_at(&tmp.path().join("a/report.txt"), NOW - 400 * 86_400);
        touch_at(&tmp.path().join("b/report.txt"), NOW - 3_600);
        let idx = index_of(tmp.path());
        let sig = Sig::new(tmp.path());
        assert_eq!(top(&idx, "report", &sig), tmp.path().join("b/report.txt"));
    }

    #[test]
    fn shallower_beats_deeper_equal_match() {
        let tmp = TempDir::new().unwrap();
        // The deeper file sorts first, so it has the lower id and would win a tie.
        touch_at(&tmp.path().join("a/b/c/report.txt"), NOW - 100);
        touch_at(&tmp.path().join("z/report.txt"), NOW - 100);
        let idx = index_of(tmp.path());
        let sig = Sig::new(tmp.path());
        assert_eq!(top(&idx, "report", &sig), tmp.path().join("z/report.txt"));
    }

    #[test]
    fn exact_name_beats_scattered_kde_listed_match() {
        // The listed file is also fresher and shallower: still the better name leads.
        let tmp = TempDir::new().unwrap();
        let exact = tmp.path().join("a/b/notes.md");
        let scattered = tmp.path().join("node_types.ts");
        touch_at(&exact, NOW - 400 * 86_400);
        touch_at(&scattered, NOW - 60);
        let idx = index_of(tmp.path());
        let mut sig = Sig::new(tmp.path());
        sig.recent.insert(scattered.clone());
        let found = paths(&idx, &search(&idx, "notes", 10, &sig.get()));
        assert_eq!(found, vec![exact, scattered]);
    }

    #[test]
    fn exact_name_beats_scattered_once_opened_match() {
        let tmp = TempDir::new().unwrap();
        let exact = tmp.path().join("a/b/notes.md");
        let scattered = tmp.path().join("unmounted_files.txt");
        touch_at(&exact, NOW - 400 * 86_400);
        touch_at(&scattered, NOW - 60);
        let idx = index_of(tmp.path());
        let mut sig = Sig::new(tmp.path());
        sig.opened.insert(
            scattered.clone(),
            Opened {
                count: 1,
                last: NOW - 60,
            },
        );
        let found = paths(&idx, &search(&idx, "notes", 10, &sig.get()));
        assert_eq!(found, vec![exact, scattered]);
    }

    #[test]
    fn habit_wins_between_comparable_matches() {
        // `rep_ort.txt` matches `report` a little worse than `report.txt`; opened, it leads.
        let tmp = TempDir::new().unwrap();
        let opened = tmp.path().join("rep_ort.txt");
        touch_at(&opened, NOW - 100);
        touch_at(&tmp.path().join("report.txt"), NOW - 100);
        let idx = index_of(tmp.path());
        let mut sig = Sig::new(tmp.path());
        sig.opened.insert(
            opened.clone(),
            Opened {
                count: 1,
                last: NOW - 60,
            },
        );
        assert_eq!(top(&idx, "report", &sig), opened);
    }

    #[test]
    fn limit_is_respected() {
        let tmp = TempDir::new().unwrap();
        for i in 0..10 {
            touch(&tmp.path().join(format!("file{i}.txt")));
        }
        let idx = index_of(tmp.path());
        let sig = Sig::new(tmp.path());
        assert_eq!(search(&idx, "file", 3, &sig.get()).len(), 3);
        assert_eq!(search(&idx, "file", 100, &sig.get()).len(), 10);
        assert!(search(&idx, "file", 0, &sig.get()).is_empty());
        assert_eq!(recent(&idx, 3, &sig.get()).len(), 3);
    }

    #[test]
    fn search_follows_upsert_and_remove() {
        let tmp = TempDir::new().unwrap();
        touch(&tmp.path().join("old/keep.txt"));
        let rules = rules_for(tmp.path());
        let mut idx = FileIndex::walk(&rules);
        let sig = Sig::new(tmp.path());
        let new = tmp.path().join("old/Отчёт.pdf");
        touch(&new);
        idx.upsert(&new, &rules);
        assert_eq!(top(&idx, "otchet", &sig), new);
        assert_eq!(top(&idx, "jnx`n", &sig), new);
        idx.remove(&tmp.path().join("old"));
        assert!(search(&idx, "otchet", 10, &sig.get()).is_empty());
        assert!(search(&idx, "keep", 10, &sig.get()).is_empty());
    }

    #[test]
    fn recent_orders_opened_then_kde_then_modified() {
        let tmp = TempDir::new().unwrap();
        let t = tmp.path();
        touch_at(&t.join("opened-old.txt"), NOW - 10);
        touch_at(&t.join("opened-new.txt"), NOW - 10_000);
        touch_at(&t.join("kde-a.txt"), NOW - 500);
        touch_at(&t.join("kde-b.txt"), NOW - 50);
        touch_at(&t.join("plain-new.txt"), NOW - 5);
        touch_at(&t.join("plain-old.txt"), NOW - 5_000);
        let idx = index_of(t);
        let mut sig = Sig::new(t);
        sig.opened.insert(
            t.join("opened-old.txt"),
            Opened {
                count: 9,
                last: NOW - 300,
            },
        );
        sig.opened.insert(
            t.join("opened-new.txt"),
            Opened {
                count: 1,
                last: NOW - 200,
            },
        );
        // Paths that are not indexed are skipped.
        sig.opened.insert(
            t.join("gone.txt"),
            Opened {
                count: 1,
                last: NOW,
            },
        );
        sig.recent.insert(t.join("kde-a.txt"));
        sig.recent.insert(t.join("kde-b.txt"));
        sig.recent.insert(t.join("opened-new.txt"));
        let found = recent(&idx, 100, &sig.get());
        let names: Vec<PathBuf> = paths(&idx, &found);
        let want: Vec<PathBuf> = [
            "opened-new.txt",
            "opened-old.txt",
            "kde-b.txt",
            "kde-a.txt",
            "plain-new.txt",
            "plain-old.txt",
        ]
        .iter()
        .map(|n| t.join(n))
        .collect();
        // Folders are not listed by modification time; the root is a folder.
        assert_eq!(names, want);
        assert!(found.windows(2).all(|w| w[0].score > w[1].score));
    }

    /// A deterministic generator of names in Latin, Cyrillic, other scripts and mixed.
    fn generated_names(n: usize) -> Vec<String> {
        const PIECES: &[&str] = &[
            "report",
            "Fire",
            "main",
            "notes",
            "Cache",
            "photo",
            "README",
            "invoice",
            "Café",
            "Cafe\u{301}",
            "naïve",
            "résumé",
            "schuka",
            "Chrome",
            "jolka",
            "TODO",
            "config",
            "Отчёт",
            "отчет",
            "Ёлка",
            "е\u{308}лка",
            "щука",
            "Хром",
            "файл",
            "Настройки",
            "йогурт",
            "Привет",
            "ЗАМЕТКИ",
            "Скриншот",
            "ფაილი",
            "Բարեւ",
            "Ελλάδα",
            "日本",
            "2024",
            "v1.2",
            "x",
            "Q",
        ];
        const SEPS: &[&str] = &[" ", "_", "-", ".", ""];
        const EXTS: &[&str] = &[".txt", ".pdf", ".md", "", ".tar.gz"];
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = |m: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % m as u64) as usize
        };
        // A few fixed ones: a space inside a name (for `main\ notes`), letters only in
        // their accented form (`Ré`), a Cyrillic name with no plain `е`.
        let mut out: Vec<String> = ["main notes final.md", "Ré.txt", "ЁЖ.md", "Зелёный чай.txt"]
            .map(String::from)
            .to_vec();
        let mut seen: HashSet<String> = out.iter().cloned().collect();
        while out.len() < n {
            let parts = 1 + next(3);
            let mut name = String::new();
            for i in 0..parts {
                if i > 0 {
                    name.push_str(SEPS[next(SEPS.len())]);
                }
                name.push_str(PIECES[next(PIECES.len())]);
            }
            name.push_str(EXTS[next(EXTS.len())]);
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
        out
    }

    const QUERIES: &[&str] = &[
        "fire",
        "rpt",
        "main",
        "mnrs",
        "notes",
        "cache",
        "kesh",
        "photo",
        "foto",
        "readme",
        "rdm",
        "invoice",
        "cafe",
        "café",
        "CAFÉ",
        "naive",
        "resume",
        "otchet",
        "отчет",
        "отчёт",
        "jnxtn",
        "ашку",
        "ёлка",
        "elka",
        "yolka",
        "щука",
        "shchuka",
        "schuka",
        "хром",
        "chrome",
        "hrom",
        "файл",
        "fajl",
        "fail",
        "настройки",
        "nastrojki",
        "nastroyki",
        "ЗАМЕТКИ",
        "zametki",
        "pfvtnrb",
        "faili",
        "barev",
        "2024",
        "v1",
        ".pdf",
        "fire notes",
        "отчет 2024",
        "main\\ notes",
        "Report",
        "e",
        "скрин",
        "re",
        "ёж",
        "еж",
        "zelyonyj",
        "zelenyj",
    ];

    #[test]
    fn prefilter_never_drops_a_match() {
        let tmp = TempDir::new().unwrap();
        let names = generated_names(2000);
        for (i, name) in names.iter().enumerate() {
            touch(&tmp.path().join(format!("d{}", i % 10)).join(name));
        }
        let idx = index_of(tmp.path());
        assert_eq!(idx.len(), 2000 + 10 + 1);
        let sig = Sig::new(tmp.path());

        // The oracle: every entry, scored with keys built from scratch.
        let texts: Vec<(EntryId, String)> = idx
            .ids()
            .map(|id| (id, display_name(idx.entry(id).name)))
            .collect();
        let scripts = texts
            .iter()
            .fold(Scripts::default(), |acc, (_, t)| acc.union(Scripts::of(t)));
        let mut m = Matcher::new();
        let no_lifts = HashMap::new();
        let mut matched_queries = 0;
        for query in QUERIES {
            let pattern = Pattern::across(query, scripts, &mut m);
            // Every match with its ranking score: the match score and the entry's own lifts.
            let mut want: Vec<(EntryId, u32)> = texts
                .iter()
                .filter_map(|(id, t)| {
                    let score = m.score_text(&pattern, t, &TextKeys::new(t))?;
                    let e = &idx.entries()[*id as usize];
                    Some((*id, ranked(score, lift(e, *id, &no_lifts, NOW))))
                })
                .collect();
            want.sort_unstable();
            let mut got: Vec<(EntryId, u32)> = search(&idx, query, usize::MAX, &sig.get())
                .iter()
                .map(|f| (f.id, f.score))
                .collect();
            got.sort_unstable();
            assert_eq!(got, want, "query {query:?}");
            matched_queries += usize::from(!want.is_empty());
        }
        assert_eq!(matched_queries, QUERIES.len());
    }
}
