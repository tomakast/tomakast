//! Calc's sheets in the plugin's `storage`, which is the truth: every call reads it again, so
//! a reloaded plugin finds what the last one saved.
//!
//! Keys (all calc's own):
//! - `sheet:<id>`: the text, cut to [`MAX_SAVED_BYTES`];
//! - `sheet-edited:<id>`: when it was last edited, in unix seconds (WASI wall clock);
//! - `sheets`: the ids, most recently edited first, comma-separated;
//! - `current`: the sheet the Calculator opens (the one last edited, or opened from the
//!   history);
//! - `next-id`: the next id, a base-36 counter;
//! - `rates:fiat` and `rates:crypto`: the exchange rates last fetched (see
//!   [`rates`](crate::rates)), written through [`Sheets::set_background`].
//!
//! A new sheet only takes an id. It is listed in `sheets` by its first save of text that is
//! not blank, so a sheet the user never types in leaves nothing behind (Ruling 13). Making
//! room drops the oldest sheet that is neither the one being saved nor the current one, an
//! empty one first, and every drop is told in a toast that names the sheet.
//!
//! When edits are saved (Ruling 8). Storage takes at most 20 writes per second, failed ones
//! included. Calc counts its own writes and allows itself [`WRITE_BUDGET`] per second, so a
//! failed `set` means a full quota, never the rate. Every `text-changed` is saved at once
//! while the budget, less a [`RESERVE`] of writes kept for `closed` and the actions, allows:
//! a save writes the text, and `sheet-edited` and `sheets` only when they change, so that is
//! about 12 edits per second. Faster than that, the newest text waits in memory for the
//! next event, and `closed`, every action (which may use the reserve) and every `search`
//! save it. A plugin that breaks in that gap loses that fraction of a second. A `search`
//! makes only saves that need no sheet dropped: picking one reads every sheet, which is not
//! for its 100 ms deadline. A save that needs a drop waits for the next call that is not a
//! search.
//!
//! Rate writes count against the same budget, never use the reserve, and are at most one per
//! feed per refresh (about 7 an hour). When they find no room, or the quota is full, the rates
//! stay in memory: a rate write never drops a sheet and never shows a toast.
//!
//! A storage read error makes the calls that would write after it write nothing: an edit
//! stays pending, an action fails with a toast. Writing from a list that could not be read
//! would drop sheets from it.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tomakast_plugin::{Effect, Toast, log, storage};

use crate::views::{self, Entry, is_sheet_id};

/// At most this many sheets are kept; listing one more drops the oldest that is not current.
pub const MAX_SHEETS: usize = 100;
/// A sheet longer than this is still computed, but saved cut to it.
pub const MAX_SAVED_BYTES: usize = 64 * 1024;
/// Writes calc allows itself per [`WRITE_WINDOW`], below the host's 20.
pub const WRITE_BUDGET: usize = 16;
pub const WRITE_WINDOW: Duration = Duration::from_secs(1);
/// The most writes one save of a listed sheet makes: the text, `sheet-edited`, `sheets` and
/// `current`.
pub const SAVE_WRITES: usize = 4;
/// The writes of the first save of a sheet at [`MAX_SHEETS`]: a whole save and the two
/// removes of the sheet it drops.
const LISTING_WRITES: usize = SAVE_WRITES + 2;
/// Writes a `text-changed` leaves for `closed` and the actions: one whole save. Not the
/// first save of a sheet at [`MAX_SHEETS`], which happens once per sheet: when the budget is
/// short, that one waits for the next call, with nothing of it written.
pub const RESERVE: usize = SAVE_WRITES;
/// Sheets one write may drop to fit the quota.
const MAX_DROPS: usize = 3;
/// A dropped sheet is named in its toast by at most this many characters of its first line.
const NAME_CHARS: usize = 40;
/// Errors of the host's storage that mean the quota is full (Ruling 7).
const QUOTA_PREFIX: &str = "quota: ";

const SHEETS: &str = "sheets";
const CURRENT: &str = "current";
const NEXT_ID: &str = "next-id";

/// What the user is told when a change cannot be made now.
pub const BUSY: &str = "Too many changes at once. Try again in a moment.";
pub const FULL: &str = "Calculator storage is full. Delete sheets in History to make room.";
pub const UNREADABLE: &str = "Calculator storage could not be read.";
pub const UNWRITABLE: &str = "Calculator storage could not be written.";

fn text_key(id: &str) -> String {
    format!("sheet:{id}")
}

fn edited_key(id: &str) -> String {
    format!("sheet-edited:{id}")
}

fn is_quota(error: &str) -> bool {
    error.starts_with(QUOTA_PREFIX)
}

/// The message for a failed write; the host's own text goes to the log only.
fn write_error(what: &str, error: &str) -> String {
    log::warn(&format!("writing {what} failed: {error}"));
    if is_quota(error) { FULL } else { UNWRITABLE }.into()
}

/// Why a sheet was dropped.
#[derive(Clone, Copy, PartialEq)]
enum Why {
    /// [`MAX_SHEETS`] were listed already.
    Cap,
    /// The storage quota is full.
    Quota,
}

/// Whether `text` holds nothing but white space: a sheet that is not listed yet, or a
/// sheet to drop first.
fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// How a write that may make room went.
enum Fit {
    /// Written (perhaps after dropping sheets).
    Done,
    /// The quota is full and no sheet may be dropped.
    Full,
    /// The write budget is spent for now.
    Later,
    /// The quota is full, and this call ([`flush_quietly`](Sheets::flush_quietly)) drops
    /// nothing.
    NeedsRoom,
    /// Another error, logged.
    Failed,
}

/// The plugin's sheets: what storage holds, and what is not saved yet.
#[derive(Default)]
pub struct Sheets {
    /// Edits not saved yet, by sheet id.
    pending: BTreeMap<String, String>,
    /// The last text the user typed in each open sheet, for "Copy Last Result" (the saved
    /// text may be cut).
    latest: HashMap<String, String>,
    /// Calc's recent writes.
    writes: WriteWindow,
    /// Writes the current call leaves unused ([`RESERVE`] for a `text-changed`).
    keep: usize,
    /// Whether the current call is a search, which drops no sheet to make room.
    quiet: bool,
    /// Sheets whose save needs a drop: searches do not try them again (a failed write
    /// counts against the budget) until a call that may drop has.
    needs_room: HashSet<String>,
    /// The `sheet-edited` value last written per sheet.
    edited: HashMap<String, i64>,
    /// Sheets the user was told about: storage full, too long to save in full, deleted.
    warned_full: HashSet<String>,
    warned_long: HashSet<String>,
    warned_deleted: HashSet<String>,
    /// Sheets deleted from the history by this instance. A pad of one may still be open
    /// below; its edits are not saved, so the sheet does not come back.
    deleted: HashSet<String>,
    /// Sheets dropped to make room that the user was not told about yet: the name, and why.
    dropped: Vec<(String, Why)>,
    /// Warnings of a call that cannot show them (`search`), for the next one that can.
    queued: Vec<Effect>,
}

thread_local! {
    static STATE: RefCell<Sheets> = RefCell::default();
}

/// Runs `f` on the plugin's sheets.
pub fn with<R>(f: impl FnOnce(&mut Sheets) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

impl Sheets {
    /// The sheet ids, most recently edited first.
    pub fn order(&self) -> Result<Vec<String>, String> {
        let stored = get(SHEETS)?.unwrap_or_default();
        let mut seen = HashSet::new();
        Ok(stored
            .split(',')
            .filter(|id| is_sheet_id(id) && seen.insert(*id))
            .map(str::to_owned)
            .collect())
    }

    fn current(&self) -> Result<Option<String>, String> {
        Ok(get(CURRENT)?.filter(|id| is_sheet_id(id)))
    }

    /// The text of sheet `id` as the user last typed it.
    pub fn text(&self, id: &str) -> Result<String, String> {
        if let Some(text) = self.latest.get(id).or_else(|| self.pending.get(id)) {
            return Ok(text.clone());
        }
        Ok(get(&text_key(id))?.unwrap_or_default())
    }

    /// Every sheet, newest first, for the history.
    pub fn entries(&self) -> Result<Vec<Entry>, String> {
        self.order()?
            .into_iter()
            .map(|id| {
                Ok(Entry {
                    text: self.text(&id)?,
                    edited: get(&edited_key(&id))?.and_then(|t| t.parse().ok()),
                    id,
                })
            })
            .collect()
    }

    /// The sheet the Calculator opens: the current one, else the newest (made current),
    /// else a new one.
    pub fn open_current(&mut self) -> Result<String, String> {
        let order = self.order()?;
        if let Some(current) = self.current()?.filter(|c| order.contains(c)) {
            return Ok(current);
        }
        match order.first() {
            Some(newest) => {
                self.make_current(newest)?;
                Ok(newest.clone())
            }
            None => self.create(),
        }
    }

    pub fn make_current(&mut self, id: &str) -> Result<(), String> {
        if self.current()?.as_deref() == Some(id) {
            return Ok(());
        }
        if !self.room(1) {
            return Err(BUSY.into());
        }
        self.set(CURRENT, id)
            .map_err(|e| write_error("the current sheet", &e))
    }

    /// Starts a new, empty sheet: takes the next id and nothing else. The sheet is listed,
    /// and made current, by its first save of text that is not blank.
    pub fn create(&mut self) -> Result<String, String> {
        let order = self.order()?;
        let counted = get(NEXT_ID)?.and_then(|n| u64::from_str_radix(&n, 36).ok());
        // Never reuse an id, even when `next-id` was lost: not a listed one, not one this
        // instance deleted (its stale pads must not save into the new sheet), and not one
        // whose text waits or is open.
        let after_known = order
            .iter()
            .chain(&self.deleted)
            .chain(self.pending.keys())
            .chain(self.latest.keys())
            .filter_map(|id| u64::from_str_radix(id, 36).ok())
            .max()
            .map(|n| n + 1);
        let n = counted.max(after_known).unwrap_or(0);
        let id = base36(n);
        if !self.room(1) {
            return Err(BUSY.into());
        }
        // Without the new count, the next sheet would take this id too.
        self.set(NEXT_ID, &base36(n + 1))
            .map_err(|e| write_error("the sheet counter", &e))?;
        // Not reachable through the ids above; stated so that a new sheet is always saved.
        self.deleted.remove(&id);
        self.warned_deleted.remove(&id);
        Ok(id)
    }

    /// Deletes sheet `id`. When it was current, the newest one left is.
    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        let mut order = self.order()?;
        order.retain(|other| other != id);
        let was_current = self.current()?.as_deref() == Some(id);
        if !self.room(3 + usize::from(was_current)) {
            return Err(BUSY.into());
        }
        self.pending.remove(id);
        self.latest.remove(id);
        self.drop_keys(id)
            .map_err(|e| write_error("the sheet", &e))?;
        self.set(SHEETS, &order.join(","))
            .map_err(|e| write_error("the sheet list", &e))?;
        // Deleted once the list no longer names it: a failed delete leaves the sheet listed,
        // and its open pad still saves.
        self.deleted.insert(id.to_owned());
        if was_current {
            match order.first() {
                Some(next) => self.set(CURRENT, next),
                None => self.remove(CURRENT),
            }
            .map_err(|e| write_error("the current sheet", &e))?;
        }
        Ok(())
    }

    /// The user edited sheet `id` to `text`: saved now unless that would leave less than
    /// [`RESERVE`] of the write budget. Returns the warnings a save gives.
    pub fn edited(&mut self, id: &str, text: String) -> Vec<Effect> {
        if self.deleted.contains(id) {
            // A stale pad of a deleted sheet: computed, never saved; said once.
            self.latest.insert(id.to_owned(), text);
            if !self.warned_deleted.insert(id.to_owned()) {
                return Vec::new();
            }
            return vec![Effect::toast(
                Toast::error("This sheet was deleted").message("Its edits are not saved."),
            )];
        }
        self.latest.insert(id.to_owned(), text.clone());
        self.pending.insert(id.to_owned(), text);
        self.keep = RESERVE;
        let warnings = self.flush();
        self.keep = 0;
        warnings
    }

    /// Sheet `id` left the screen: everything unsaved is saved.
    pub fn closed(&mut self, id: &str) -> Vec<Effect> {
        let warnings = self.flush();
        self.latest.remove(id);
        warnings
    }

    /// Saves every pending edit it can; what it cannot stays pending for the next call.
    /// Returns the warnings, the ones held for this call first. Sheets it drops are not in
    /// them: the call names them with [`drop_notice`](Self::drop_notice), after its own
    /// effects.
    pub fn flush(&mut self) -> Vec<Effect> {
        let mut warnings = std::mem::take(&mut self.queued);
        if !self.quiet {
            self.needs_room.clear();
        }
        if !self.pending.is_empty()
            && let (Ok(mut order), Ok(mut current)) = (self.order(), self.current())
        {
            // On a read error (logged by `get`) the edits wait for a call that can read.
            let pending: Vec<(String, String)> = self
                .pending
                .iter()
                .filter(|(id, _)| !(self.quiet && self.needs_room.contains(*id)))
                .map(|(id, text)| (id.clone(), text.clone()))
                .collect();
            for (id, text) in pending {
                if self.save(&id, &text, &mut order, &mut current, &mut warnings) {
                    self.pending.remove(&id);
                }
            }
        }
        warnings
    }

    /// [`flush`](Self::flush) for `search`, which has 100 ms: it makes only the saves that
    /// need no sheet dropped (picking one reads every sheet); the others wait for the next
    /// call that is not a search. Its warnings are held for the next call that can show them.
    pub fn flush_quietly(&mut self) {
        if self.pending.keys().all(|id| self.needs_room.contains(id)) {
            return;
        }
        self.quiet = true;
        self.queued = self.flush();
        self.quiet = false;
    }

    /// Writes sheet `id`: its text, then, when they change, when it was edited, its place
    /// first in `sheets`, and `current` (the Calculator reopens the sheet last edited). A
    /// sheet that is not listed yet is listed first, and only for text that is not blank:
    /// blank text of one is not written. Whether all of it was written. `order` and
    /// `current` are the stored values, kept up to date.
    fn save(
        &mut self,
        id: &str,
        text: &str,
        order: &mut Vec<String>,
        current: &mut Option<String>,
        warnings: &mut Vec<Effect>,
    ) -> bool {
        if !order.iter().any(|other| other == id) {
            if is_blank(text) {
                return true;
            }
            if !self.list(id, order, current.as_deref(), warnings) {
                return false;
            }
        }
        let stored = views::truncate(text, MAX_SAVED_BYTES);
        if stored.len() < text.len() && self.warned_long.insert(id.to_owned()) {
            warnings.push(Effect::toast(
                Toast::error("Sheet too long to save in full").message(format!(
                    "Only its first {} KiB are kept.",
                    MAX_SAVED_BYTES / 1024
                )),
            ));
        }
        let fit = self.fit(order, &[Some(id), current.as_deref()], false, |s, _| {
            s.set(&text_key(id), stored)
        });
        match fit {
            Fit::Done => {}
            Fit::Full => {
                self.warn_unsaved(id, warnings);
                return false;
            }
            Fit::NeedsRoom => {
                self.needs_room.insert(id.to_owned());
                return false;
            }
            Fit::Later | Fit::Failed => return false,
        }
        let now = unix_now();
        if self.edited.get(id) != Some(&now) {
            if !self.room(1) || self.set(&edited_key(id), &now.to_string()).is_err() {
                return false;
            }
            self.edited.insert(id.to_owned(), now);
        }
        if order.first().map(String::as_str) != Some(id) {
            let mut moved = order.clone();
            moved.retain(|other| other != id);
            moved.insert(0, id.to_owned());
            if !self.room(1) || self.set(SHEETS, &moved.join(",")).is_err() {
                return false;
            }
            *order = moved;
        }
        if current.as_deref() != Some(id) {
            if !self.room(1) || self.set(CURRENT, id).is_err() {
                return false;
            }
            *current = Some(id.to_owned());
        }
        true
    }

    /// Lists sheet `id` first in `sheets`, before any of it is written, so that no text is
    /// stored that the history does not show. With [`MAX_SHEETS`] listed, one sheet is
    /// dropped first; a full quota drops more. Whether it is listed; `order` is kept up to
    /// date.
    fn list(
        &mut self,
        id: &str,
        order: &mut Vec<String>,
        current: Option<&str>,
        warnings: &mut Vec<Effect>,
    ) -> bool {
        let keep = [Some(id), current];
        let at_cap = order.len() >= MAX_SHEETS;
        if at_cap && self.quiet {
            self.needs_room.insert(id.to_owned());
            return false;
        }
        // The whole first save, up front: it is not left half done for want of writes.
        if !self.room(if at_cap { LISTING_WRITES } else { SAVE_WRITES }) {
            return false;
        }
        let mut capped = false;
        if at_cap {
            match self.victim(order, &keep) {
                Ok(Some(victim)) => {
                    if let Err(e) = self.drop_sheet(&victim, Why::Cap) {
                        log::warn(&format!("dropping sheet {victim} failed: {e}"));
                        return false;
                    }
                    order.retain(|other| *other != victim);
                    capped = true;
                }
                Ok(None) => {}
                Err(_) => return false,
            }
        }
        let fit = self.fit(order, &keep, true, |s, order| {
            let mut ids = vec![id];
            ids.extend(order.iter().map(String::as_str));
            s.set(SHEETS, &ids.join(","))
        });
        match fit {
            Fit::Done => {
                order.insert(0, id.to_owned());
                true
            }
            failed => {
                // The cap's drop is done: the list must not name it any more.
                if capped && self.set(SHEETS, &order.join(",")).is_err() {
                    log::warn("writing the sheet list failed");
                }
                match failed {
                    Fit::Full => self.warn_unsaved(id, warnings),
                    Fit::NeedsRoom => {
                        self.needs_room.insert(id.to_owned());
                    }
                    _ => {}
                }
                false
            }
        }
    }

    /// Runs `write`; while it fails on the quota, drops the sheet [`victim`](Self::victim)
    /// picks of `order` and `keep` (at most [`MAX_DROPS`]) and runs it again with the shorter
    /// `order`. When sheets were dropped, `sheets` is rewritten, unless `write` writes it
    /// itself. A quiet call drops nothing: a write that does not fit is
    /// [`NeedsRoom`](Fit::NeedsRoom).
    fn fit(
        &mut self,
        order: &mut Vec<String>,
        keep: &[Option<&str>],
        writes_sheets: bool,
        mut write: impl FnMut(&mut Self, &[String]) -> Result<(), String>,
    ) -> Fit {
        if !self.room(1) {
            return Fit::Later;
        }
        match write(self, order) {
            Ok(()) => return Fit::Done,
            Err(e) if is_quota(&e) && self.quiet => return Fit::NeedsRoom,
            Err(e) if is_quota(&e) => {}
            Err(e) => {
                log::warn(&format!("a storage write failed: {e}"));
                return Fit::Failed;
            }
        }
        let mut dropped = 0;
        // Two removes and a retry per sheet, and the new `sheets` at the end.
        let outcome = loop {
            if dropped == MAX_DROPS || !self.room(4) {
                break Fit::Later;
            }
            let victim = match self.victim(order, keep) {
                Ok(Some(victim)) => victim,
                Ok(None) => break Fit::Full,
                Err(_) => break Fit::Failed,
            };
            if let Err(e) = self.drop_sheet(&victim, Why::Quota) {
                log::warn(&format!("dropping sheet {victim} failed: {e}"));
                break Fit::Failed;
            }
            order.retain(|other| *other != victim);
            dropped += 1;
            match write(self, order) {
                Ok(()) => break Fit::Done,
                Err(e) if is_quota(&e) => {}
                Err(e) => {
                    log::warn(&format!("a storage write failed: {e}"));
                    break Fit::Failed;
                }
            }
        };
        let listed = writes_sheets && matches!(outcome, Fit::Done);
        if dropped > 0 && !listed && self.set(SHEETS, &order.join(",")).is_err() {
            log::warn("writing the sheet list failed");
        }
        outcome
    }

    /// Tells the user, once per sheet, that storage is full and sheet `id` is not saved.
    fn warn_unsaved(&mut self, id: &str, warnings: &mut Vec<Effect>) {
        if !self.warned_full.insert(id.to_owned()) {
            return;
        }
        warnings.push(Effect::toast(
            Toast::error("Calculator storage is full")
                .message("This sheet is not saved. Delete sheets in History to make room."),
        ));
    }

    /// The sheet to drop to make room: of the sheets of `order` that are none of `keep`, the
    /// oldest blank one, else the oldest.
    fn victim(&self, order: &[String], keep: &[Option<&str>]) -> Result<Option<String>, String> {
        let others = || {
            order
                .iter()
                .rev()
                .filter(|id| !keep.contains(&Some(id.as_str())))
        };
        for id in others() {
            if is_blank(&self.text(id)?) {
                return Ok(Some(id.clone()));
            }
        }
        Ok(others().next().cloned())
    }

    /// Drops sheet `id` to make room (two writes); the user is told by
    /// [`drop_notice`](Self::drop_notice).
    fn drop_sheet(&mut self, id: &str, why: Why) -> Result<(), String> {
        let name = match self.text(id) {
            Ok(text) => views::sheet_name(&text, NAME_CHARS),
            Err(_) => "a sheet".to_owned(),
        };
        self.drop_keys(id)?;
        self.dropped.push((name, why));
        Ok(())
    }

    /// One toast naming the sheets dropped since the last one, if any. Every call that
    /// returns effects puts it last: a toast replaces the one before it, and the names are
    /// told only once. Until then they are kept, so drops of several calls that could not
    /// tell them (searches) are named together.
    pub fn drop_notice(&mut self) -> Option<Effect> {
        if self.dropped.is_empty() {
            return None;
        }
        let dropped = std::mem::take(&mut self.dropped);
        let names: Vec<&str> = dropped.iter().map(|(name, _)| name.as_str()).collect();
        let (title, verb) = match names.len() {
            1 => ("Deleted an old sheet".to_owned(), "was"),
            n => (format!("Deleted {n} old sheets"), "were"),
        };
        let mut reasons = Vec::new();
        if dropped.iter().any(|(_, why)| *why == Why::Cap) {
            reasons.push(format!("Calculator keeps at most {MAX_SHEETS} sheets."));
        }
        if dropped.iter().any(|(_, why)| *why == Why::Quota) {
            reasons.push("Calculator storage is full.".to_owned());
        }
        let message = format!(
            "{} {verb} deleted to make room. {}",
            capitalized(&names.join(", ")),
            reasons.join(" ")
        );
        Some(Effect::toast(Toast::error(title).message(message)))
    }

    /// Removes the keys of sheet `id` (two writes).
    fn drop_keys(&mut self, id: &str) -> Result<(), String> {
        self.pending.remove(id);
        self.latest.remove(id);
        self.edited.remove(id);
        self.remove(&text_key(id))?;
        self.remove(&edited_key(id))
    }

    /// Whether `n` more writes, and the writes this call keeps, stay within
    /// [`WRITE_BUDGET`].
    fn room(&mut self, n: usize) -> bool {
        self.writes.room(n, self.keep, Instant::now())
    }

    /// One write that is not a sheet's (a rate feed): only when the budget less [`RESERVE`]
    /// has room. `Ok(false)`: no room now. A failed `set` is returned as its error (logged by
    /// the caller); it never drops a sheet.
    pub fn set_background(&mut self, key: &str, value: &str) -> Result<bool, String> {
        if !self.writes.room(1, RESERVE, Instant::now()) {
            return Ok(false);
        }
        self.set(key, value).map(|()| true)
    }

    fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        self.writes.note(Instant::now());
        storage::set(key, value)
    }

    fn remove(&mut self, key: &str) -> Result<(), String> {
        self.writes.note(Instant::now());
        storage::remove(key)
    }
}

/// Calc's recent writes (Ruling 8), apart from what counts them.
#[derive(Default)]
pub struct WriteWindow {
    /// When each write of the last [`WRITE_WINDOW`] was made, oldest first.
    writes: VecDeque<Instant>,
}

impl WriteWindow {
    /// Whether `n` more writes at `now`, and `keep` after them, stay within [`WRITE_BUDGET`].
    pub fn room(&mut self, n: usize, keep: usize, now: Instant) -> bool {
        while self
            .writes
            .front()
            .is_some_and(|at| now.duration_since(*at) >= WRITE_WINDOW)
        {
            self.writes.pop_front();
        }
        self.writes.len() + n + keep <= WRITE_BUDGET
    }

    /// A write made at `now`, failed or not.
    pub fn note(&mut self, now: Instant) {
        self.writes.push_back(now);
    }
}

/// `text` with its first character upper case ("an empty sheet" starts a sentence).
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The stored value of `key`. A read error is logged; the caller gets [`UNREADABLE`].
fn get(key: &str) -> Result<Option<String>, String> {
    storage::get(key).map_err(|e| {
        log::warn(&format!("reading {key} failed: {e}"));
        UNREADABLE.to_owned()
    })
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("ASCII digits")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base36_counts() {
        for (n, want) in [
            (0, "0"),
            (9, "9"),
            (10, "a"),
            (35, "z"),
            (36, "10"),
            (100, "2s"),
        ] {
            assert_eq!(base36(n), want);
            assert_eq!(u64::from_str_radix(want, 36), Ok(n));
        }
    }

    #[test]
    fn blank_and_capitalized() {
        assert!(is_blank("") && is_blank(" \n\t") && !is_blank(" 1 "));
        assert_eq!(capitalized("an empty sheet, “x”"), "An empty sheet, “x”");
        assert_eq!(capitalized("“x”"), "“x”");
        assert_eq!(capitalized(""), "");
    }

    #[test]
    fn background_writes_leave_the_reserve() {
        let t0 = Instant::now();
        let mut window = WriteWindow::default();
        for _ in 0..11 {
            window.note(t0);
        }
        assert!(window.room(1, RESERVE, t0));
        window.note(t0);
        assert!(!window.room(1, RESERVE, t0));
        // A sheet's call, which may use the reserve, still has room.
        assert!(window.room(1, 0, t0));
        assert!(window.room(1, RESERVE, t0 + WRITE_WINDOW));
    }

    #[test]
    fn quota_errors_by_prefix() {
        assert!(is_quota("quota: storage quota of 1048576 bytes exceeded"));
        assert!(!is_quota("more than 20 storage writes per second"));
        assert!(!is_quota("storage is not available (quota: no)"));
    }
}
