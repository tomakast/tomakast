//! Changing the Home list in `config.toml`. The launcher edits the file's own list in place:
//! it adds, removes, moves or resizes one entry and leaves every other entry, table, comment
//! and blank line as the user wrote them. An entry it cannot read stays where it is and is
//! never counted, moved or changed.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::settings::{HomeEntry, home_elements};
use crate::widget::WidgetSize;

/// The title of the launcher's own action that adds a search tile to Home. No plugin action
/// may have it.
pub const ADD_TO_HOME: &str = "Add to Home";

#[derive(Debug, thiserror::Error)]
pub enum HomeWriteError {
    /// The file is not valid TOML, or its `home` is not a table or its `home.widgets` not a
    /// list; it is left as it is rather than overwritten.
    #[error("config.toml is not valid TOML: {0}")]
    Broken(String),
    /// The file's Home list is not the one the launcher shows (it was edited since it was
    /// read): nothing is written.
    #[error("config.toml changed: try again")]
    Changed,
    /// The edit names an entry past the end of the list.
    #[error("there is no Home entry {0}")]
    NoEntry(usize),
    /// The file's Home is written in a form the launcher cannot change without changing
    /// something else; nothing is written.
    #[error("config.toml's Home cannot be changed as it is written: {0}")]
    Unchangeable(String),
    #[error("writing config.toml: {0}")]
    Io(#[from] std::io::Error),
    /// The launcher was given no `config.toml` to write.
    #[error("there is no config.toml to write Home to")]
    NoConfigFile,
    /// The tile to add is not one of the latest search's tiles (it was answered for an older
    /// query, or is gone).
    #[error("the tile is no longer one of the search's tiles")]
    NotASearchTile,
}

/// One change of the Home list. Indices count the entries the launcher reads, in the file's
/// order; an entry it cannot read is not counted.
#[derive(Clone, Debug, PartialEq)]
pub enum HomeEdit {
    /// Removes entry `i` with the comment lines right above it.
    Remove(usize),
    /// Moves entry `from` so that it ends up at index `to`; the comment lines right above it
    /// move with it.
    Move { from: usize, to: usize },
    /// Sets the size of entry `i`, changing only its `size` value.
    Resize(usize, WidgetSize),
    /// Puts a new entry before entry `i` (above the comment lines right above it), or at the
    /// end of the list when `i` is the number of entries.
    Insert(usize, HomeEntry),
}

/// Adds `entry` at the end of the Home list of `config_file` (a missing file is created) and
/// returns the file's list as the launcher reads it after the write.
pub fn add_home(config_file: &Path, entry: &HomeEntry) -> Result<Vec<HomeEntry>, HomeWriteError> {
    let file = Current::read(config_file)?;
    let at = file.readable().len();
    file.apply(config_file, HomeEdit::Insert(at, entry.clone()))
}

/// Applies `edit` to the Home list of `config_file` and returns the file's list as the
/// launcher reads it after the write. The file's list must be `expected` (the one the
/// launcher shows), else nothing is written and the error is [`HomeWriteError::Changed`].
pub fn edit_home(
    config_file: &Path,
    expected: &[HomeEntry],
    edit: HomeEdit,
) -> Result<Vec<HomeEntry>, HomeWriteError> {
    let file = Current::read(config_file)?;
    if file.readable() != expected {
        return Err(HomeWriteError::Changed);
    }
    file.apply(config_file, edit)
}

/// The file as read: its text and each element of its `home.widgets` as the launcher reads it.
struct Current {
    text: String,
    elements: Vec<Option<HomeEntry>>,
}

impl Current {
    fn read(path: &Path) -> Result<Current, HomeWriteError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let elements = elements_of(&text)?;
        Ok(Current { text, elements })
    }

    fn readable(&self) -> Vec<HomeEntry> {
        self.elements.iter().flatten().cloned().collect()
    }

    /// The positions in `home.widgets` of the entries the launcher reads.
    fn slots(&self) -> Vec<usize> {
        (0..self.elements.len())
            .filter(|&i| self.elements[i].is_some())
            .collect()
    }

    /// Writes the file with `edit` applied; returns the list read back.
    fn apply(self, path: &Path, edit: HomeEdit) -> Result<Vec<HomeEntry>, HomeWriteError> {
        let mut list = self.readable();
        let len = list.len();
        match &edit {
            HomeEdit::Remove(i) | HomeEdit::Resize(i, _) if *i >= len => {
                return Err(HomeWriteError::NoEntry(*i));
            }
            HomeEdit::Move { from, to } if *from >= len || *to >= len => {
                return Err(HomeWriteError::NoEntry((*from).max(*to)));
            }
            HomeEdit::Insert(i, _) if *i > len => return Err(HomeWriteError::NoEntry(*i)),
            _ => {}
        }
        match &edit {
            HomeEdit::Remove(i) => {
                list.remove(*i);
            }
            HomeEdit::Move { from, to } => {
                let moved = list.remove(*from);
                list.insert(*to, moved);
            }
            HomeEdit::Resize(i, size) => list[*i].size = Some(*size),
            HomeEdit::Insert(i, entry) => list.insert(*i, entry.clone()),
        }
        if matches!(edit, HomeEdit::Move { from, to } if from == to) {
            return Ok(list);
        }

        let text = self.edited(edit)?;
        // Read back before writing: the file must hold the intended list, and the entries the
        // launcher cannot read must all still be there.
        let after = elements_of(&text)
            .map_err(|e| HomeWriteError::Unchangeable(format!("the edit did not read: {e}")))?;
        let readable: Vec<HomeEntry> = after.iter().flatten().cloned().collect();
        let unread =
            |elements: &[Option<HomeEntry>]| elements.iter().filter(|e| e.is_none()).count();
        if readable != list || unread(&after) != unread(&self.elements) {
            return Err(HomeWriteError::Unchangeable(
                "the edit read back differently".into(),
            ));
        }
        write_atomically(path, text.as_bytes())?;
        Ok(list)
    }

    /// The file's text with `edit` applied (its index already checked).
    fn edited(&self, edit: HomeEdit) -> Result<String, HomeWriteError> {
        let mut doc: DocumentMut = self
            .text
            .parse()
            .map_err(|e: toml_edit::TomlError| HomeWriteError::Broken(e.to_string()))?;
        let slots = self.slots();
        let home_is_header = match doc.get("home") {
            None => true,
            Some(Item::Table(t)) => !t.is_dotted(),
            Some(_) => false,
        };
        let unexpected = || HomeWriteError::Unchangeable("unexpected form of `home`".into());

        let Some(home) = doc.get_mut("home") else {
            // No Home yet: a new `[[home.widgets]]` at the end of the file.
            let HomeEdit::Insert(_, entry) = edit else {
                return Err(unexpected());
            };
            return Ok(appended(&self.text, &entry));
        };
        let home = home.as_table_like_mut().ok_or_else(unexpected)?;
        if !home.contains_key("widgets") {
            let HomeEdit::Insert(_, entry) = edit else {
                return Err(unexpected());
            };
            if home_is_header {
                return Ok(appended(&self.text, &entry));
            }
            // An inline or dotted `home` holds an inline list.
            let mut list = Array::new();
            list.push(inline_entry(&entry));
            home.insert("widgets", Item::Value(Value::Array(list)));
            return Ok(doc.to_string());
        }
        let removed = match (home.get_mut("widgets"), edit) {
            // The end of a list of tables is the end of the file.
            (Some(Item::ArrayOfTables(_)), HomeEdit::Insert(i, entry)) if i == slots.len() => {
                return Ok(appended(&self.text, &entry));
            }
            (Some(Item::ArrayOfTables(tables)), edit) => {
                if tables.len() != self.elements.len() {
                    return Err(unexpected());
                }
                edit_tables(tables, &slots, edit)
            }
            (Some(Item::Value(Value::Array(values))), edit) => {
                if values.len() != self.elements.len() {
                    return Err(unexpected());
                }
                edit_values(values, &slots, edit)?;
                None
            }
            _ => return Err(unexpected()),
        };
        if let Some((position, place)) = removed {
            hand_on(&mut doc, position, &place);
        }
        Ok(doc.to_string())
    }
}

/// Each element of the `home.widgets` of `text` as the launcher reads it.
fn elements_of(text: &str) -> Result<Vec<Option<HomeEntry>>, HomeWriteError> {
    let file: toml::Table =
        toml::from_str(text).map_err(|e| HomeWriteError::Broken(e.to_string()))?;
    home_elements(&file).map_err(|key| {
        HomeWriteError::Broken(match key {
            "home" => "`home` is not a table".to_string(),
            key => format!("`{key}` is not a list"),
        })
    })
}

/// `text` with a new `[[home.widgets]]` table of `entry` at its end, after a blank line.
fn appended(text: &str, entry: &HomeEntry) -> String {
    let mut out = text.to_string();
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.ends_with("\n\n") {
            out.push('\n');
        }
    }
    let mut list = ArrayOfTables::new();
    list.push(table_entry(entry));
    let mut home = Table::new();
    home.set_implicit(true);
    home.insert("widgets", Item::ArrayOfTables(list));
    let mut doc = DocumentMut::new();
    doc.insert("home", Item::Table(home));
    out.push_str(&doc.to_string());
    out
}

fn table_entry(entry: &HomeEntry) -> Table {
    let mut table = Table::new();
    fill(&mut table, entry);
    table
}

fn inline_entry(entry: &HomeEntry) -> InlineTable {
    let mut table = InlineTable::new();
    fill(&mut table, entry);
    table.fmt();
    table
}

fn fill(table: &mut dyn TableLike, entry: &HomeEntry) {
    table.insert("widget", toml_edit::value(entry.widget.to_string()));
    if let Some(size) = entry.size {
        table.insert("size", toml_edit::value(size.as_str()));
    }
    for (key, val) in entry.params.iter() {
        table.insert(key, toml_edit::value(val.as_str()));
    }
}

/// Sets the `size` of `table`, keeping the old value's spacing and comment.
fn set_size(table: &mut dyn TableLike, size: WidgetSize) {
    match table.get_mut("size") {
        Some(Item::Value(old)) => {
            let decor = old.decor().clone();
            *old = Value::from(size.as_str());
            *old.decor_mut() = decor;
        }
        _ => {
            table.insert("size", toml_edit::value(size.as_str()));
        }
    }
}

/// The lines before a table header or a list element, as written.
fn prefix_of(decor: &toml_edit::Decor) -> String {
    decor
        .prefix()
        .and_then(|p| p.as_str())
        .unwrap_or_default()
        .to_string()
}

/// The lines before a table header or a list element, in three parts. The comment lines
/// right above the element (no blank line between) are the element's own and go where it
/// goes; what comes before them (blank lines, comments parted from it by a blank line, the
/// end of the line before) and its indentation belong to its place in the file.
struct Prefix {
    place: String,
    comments: String,
    indent: String,
}

impl Prefix {
    /// Splits `prefix`; `mid_line`: it starts after a list's comma, so its first line is the
    /// end of the line before.
    fn split(prefix: &str, mid_line: bool) -> Prefix {
        let mut lines: Vec<&str> = prefix.split_inclusive('\n').collect();
        let indent = match lines.last() {
            Some(last) if !last.ends_with('\n') => lines.pop().unwrap_or_default(),
            _ => "",
        };
        let first = usize::from(mid_line).min(lines.len());
        let mut own = lines.len();
        while own > first && lines[own - 1].trim_start().starts_with('#') {
            own -= 1;
        }
        Prefix {
            place: lines[..own].concat(),
            comments: lines[own..].concat(),
            indent: indent.to_string(),
        }
    }

    fn of_table(table: &Table) -> Prefix {
        Prefix::split(&prefix_of(table.decor()), false)
    }

    fn of_value(value: &Value) -> Prefix {
        Prefix::split(&prefix_of(value.decor()), true)
    }

    /// The lines before an element with these comments that moves into `place`.
    fn moved_into(&self, place: &Prefix) -> String {
        format!("{}{}{}", place.place, self.comments, place.indent)
    }

    /// These lines after `before`, the place of a removed element above, is put in front:
    /// blank lines of this place give way to it, comments stay.
    fn after(&self, before: &str) -> String {
        let place = if self.place.trim().is_empty() {
            ""
        } else {
            &self.place
        };
        format!("{before}{place}{}{}", self.comments, self.indent)
    }
}

/// Whether `place` holds a comment, which must not be lost with the element below it.
fn holds_comment(place: &str) -> bool {
    !place.trim().is_empty()
}

/// Hands `place`, the lines of a removed table's place at `position`, to the table printed
/// next in the file, or to the end of the file.
fn hand_on(doc: &mut DocumentMut, position: Option<isize>, place: &str) {
    if !holds_comment(place) {
        return;
    }
    let next = position.and_then(|p| next_position(doc.as_table(), p));
    if let Some(table) = next.and_then(|n| table_at(doc.as_table_mut(), n)) {
        let prefix = Prefix::of_table(table).after(place);
        table.decor_mut().set_prefix(prefix);
        return;
    }
    let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
    let trailing = if trailing.trim().is_empty() {
        ""
    } else {
        &trailing
    };
    let place = place.trim_end_matches('\n');
    doc.set_trailing(format!("{place}\n{trailing}"));
}

/// Whether `table` prints a header of its own.
fn has_header(table: &Table) -> bool {
    !table.is_dotted() && !table.is_implicit()
}

/// The smallest place in the file after `after` of a table under `table` that has a header.
fn next_position(table: &Table, after: isize) -> Option<isize> {
    let mut next: Option<isize> = None;
    let mut keep = |p: Option<isize>| {
        if let Some(p) = p.filter(|&p| p > after) {
            next = Some(next.map_or(p, |n| n.min(p)));
        }
    };
    for (_, item) in table.iter() {
        let tables: Vec<(&Table, bool)> = match item {
            Item::Table(t) => vec![(t, has_header(t))],
            Item::ArrayOfTables(list) => list.iter().map(|t| (t, true)).collect(),
            _ => continue,
        };
        for (t, header) in tables {
            if header {
                keep(t.position());
            }
            keep(next_position(t, after));
        }
    }
    next
}

/// The table at place `position` under `table` that has a header.
fn table_at(table: &mut Table, position: isize) -> Option<&mut Table> {
    for (_, item) in table.iter_mut() {
        match item {
            Item::Table(t) => {
                if has_header(t) && t.position() == Some(position) {
                    return Some(t);
                }
                if let Some(found) = table_at(t, position) {
                    return Some(found);
                }
            }
            Item::ArrayOfTables(list) => {
                for t in list.iter_mut() {
                    if t.position() == Some(position) {
                        return Some(t);
                    }
                    if let Some(found) = table_at(t, position) {
                        return Some(found);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Applies `edit` to a `[[home.widgets]]` list; `slots` are the positions of the entries the
/// launcher reads. Tables print in the order of their place in the file, so a table that moves
/// takes the place of the one it replaces. A removed table's place (its position and the
/// lines that belong to it) is returned, for [`hand_on`].
fn edit_tables(
    tables: &mut ArrayOfTables,
    slots: &[usize],
    edit: HomeEdit,
) -> Option<(Option<isize>, String)> {
    match edit {
        HomeEdit::Remove(i) => {
            let removed = tables.remove(slots[i]);
            return Some((removed.position(), Prefix::of_table(&removed).place));
        }
        HomeEdit::Resize(i, size) => {
            if let Some(table) = tables.get_mut(slots[i]) {
                set_size(table, size);
            }
        }
        HomeEdit::Move { from, to } => {
            let places: Vec<(Option<isize>, Prefix)> = slots
                .iter()
                .filter_map(|&s| tables.get(s))
                .map(|t| (t.position(), Prefix::of_table(t)))
                .collect();
            let mut order: Vec<Table> = slots
                .iter()
                .filter_map(|&s| tables.get(s).cloned())
                .collect();
            let moved = order.remove(from);
            order.insert(to, moved);
            for ((&slot, (position, place)), mut table) in slots.iter().zip(places).zip(order) {
                let prefix = Prefix::of_table(&table).moved_into(&place);
                table.decor_mut().set_prefix(prefix);
                table.set_position(position);
                if let Some(t) = tables.get_mut(slot) {
                    *t = table;
                }
            }
        }
        HomeEdit::Insert(i, entry) => {
            // Inserting at the end is appending to the file, done by the caller.
            let slot = slots[i];
            let old = tables.get_mut(slot)?;
            let place = Prefix::of_table(old);
            let mut table = table_entry(&entry);
            table
                .decor_mut()
                .set_prefix(format!("{}{}", place.place, place.indent));
            // Same place as the old table: the new one, listed first, prints first.
            table.set_position(old.position());
            old.decor_mut()
                .set_prefix(format!("\n{}{}", place.comments, place.indent));
            tables.insert(slot, table);
        }
    }
    None
}

/// Applies `edit` to an inline `widgets = [...]` list; `slots` are the positions of the
/// entries the launcher reads.
fn edit_values(values: &mut Array, slots: &[usize], edit: HomeEdit) -> Result<(), HomeWriteError> {
    let not_inline = || HomeWriteError::Unchangeable("an entry is not an inline table".into());
    match edit {
        HomeEdit::Remove(i) => {
            let at = slots[i];
            let removed = values.remove(at);
            let gone = Prefix::of_value(&removed);
            match values.get_mut(at) {
                // The new first element takes the place of the old one.
                Some(next) if at == 0 && !holds_comment(&gone.place) => {
                    let own = Prefix::of_value(next);
                    let prefix = if own.comments.is_empty() && !holds_comment(&own.place) {
                        prefix_of(removed.decor())
                    } else {
                        own.after(&gone.place)
                    };
                    next.decor_mut().set_prefix(prefix);
                }
                Some(next) if holds_comment(&gone.place) => {
                    let prefix = Prefix::of_value(next).after(&gone.place);
                    next.decor_mut().set_prefix(prefix);
                }
                None if holds_comment(&gone.place) => {
                    let trailing = values.trailing().as_str().unwrap_or_default().to_string();
                    let trailing = if trailing.trim().is_empty() {
                        String::new()
                    } else {
                        trailing
                    };
                    values.set_trailing(format!("{}{trailing}", gone.place));
                }
                _ => {}
            }
        }
        HomeEdit::Resize(i, size) => match values.get_mut(slots[i]) {
            Some(Value::InlineTable(table)) => set_size(table, size),
            _ => return Err(not_inline()),
        },
        HomeEdit::Move { from, to } => {
            let places: Vec<Prefix> = slots
                .iter()
                .filter_map(|&s| values.get(s))
                .map(Prefix::of_value)
                .collect();
            let mut order: Vec<Value> = slots
                .iter()
                .filter_map(|&s| values.get(s).cloned())
                .collect();
            let moved = order.remove(from);
            order.insert(to, moved);
            for ((&slot, place), mut value) in slots.iter().zip(places).zip(order) {
                let prefix = Prefix::of_value(&value).moved_into(&place);
                value.decor_mut().set_prefix(prefix);
                values.replace_formatted(slot, value);
            }
        }
        HomeEdit::Insert(i, entry) => {
            let mut value = Value::InlineTable(inline_entry(&entry));
            if i == slots.len() {
                // After the last element, spaced like it.
                let last = values.len().checked_sub(1).and_then(|l| values.get(l));
                let prefix = match last.map(|v| prefix_of(v.decor())) {
                    None => String::new(),
                    Some(p) => match p.rfind('\n') {
                        Some(n) => format!("\n{}", &p[n + 1..]),
                        None => " ".to_string(),
                    },
                };
                value.decor_mut().set_prefix(prefix);
                values.push_formatted(value);
            } else {
                let slot = slots[i];
                let Some(old) = values.get_mut(slot) else {
                    return Err(not_inline());
                };
                let place = Prefix::of_value(old);
                value
                    .decor_mut()
                    .set_prefix(format!("{}{}", place.place, place.indent));
                let newline = if place.place.contains('\n') { "\n" } else { "" };
                let mut prefix = format!("{newline}{}{}", place.comments, place.indent);
                if prefix.is_empty() {
                    prefix = " ".to_string();
                }
                old.decor_mut().set_prefix(prefix);
                values.insert_formatted(slot, value);
            }
        }
    }
    Ok(())
}

/// Writes `bytes` to a temporary file next to `path`, then renames it over `path`, so a
/// reader never sees half a file. A symlinked `path` stays a link: the file it points to is
/// written. The file keeps its permissions; a missing one is created with its folder.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let (target, permissions) = match std::fs::canonicalize(path) {
        Ok(target) => {
            let permissions = std::fs::metadata(&target)?.permissions();
            (target, Some(permissions))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            (path.to_path_buf(), None)
        }
        Err(e) => return Err(e),
    };
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp: PathBuf = target.with_file_name(name);
    let written = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}
