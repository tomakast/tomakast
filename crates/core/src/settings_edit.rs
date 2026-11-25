//! Writing single keys and the `[hotkeys]` table back into `config.toml`, keeping the user's
//! comments, order and layout.

use std::path::Path;

use anyhow::Context as _;
use toml_edit::{DocumentMut, Item, Table, TableLike, Value, value};

use crate::index::ItemId;
use crate::settings::{Hotkey, Settings};

/// `text` with every `[hotkeys]` entry of `item` removed (keys compared as parsed hotkeys,
/// values as item ids), then `"<hotkey>" = "<item>"` appended for `Some`. Errors when `text`
/// is not valid settings: a broken file is never rewritten.
pub fn set_item_hotkey(
    text: &str,
    item: &ItemId,
    hotkey: Option<&Hotkey>,
) -> anyhow::Result<String> {
    Settings::parse(text)?;
    let mut doc: DocumentMut = text.parse().context("invalid TOML")?;
    if !doc.contains_key("hotkeys") {
        let mut table = Table::new();
        table.set_implicit(false);
        doc.insert("hotkeys", Item::Table(table));
    }
    let table = doc["hotkeys"]
        .as_table_like_mut()
        .context("[hotkeys] is not a table")?;
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    let stale: Vec<String> = table
        .iter()
        .filter(|(_, v)| v.as_str().and_then(ItemId::parse).as_ref() == Some(item))
        .map(|(k, _)| k.to_string())
        .collect();
    // The comment lines above a key belong to the key: moved to the next entry kept, so
    // removing the entry does not take the user's comments with it.
    let mut carried = String::new();
    for key in &keys {
        let prefix = table
            .key(key)
            .and_then(|k| k.leaf_decor().prefix())
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        if stale.contains(key) {
            carried.push_str(&prefix);
            table.remove(key);
        } else if !carried.is_empty() {
            if let Some(mut k) = table.key_mut(key) {
                k.leaf_decor_mut().set_prefix(format!("{carried}{prefix}"));
            }
            carried.clear();
        }
    }
    if let Some(hotkey) = hotkey {
        let key = hotkey.to_string();
        table.insert(&key, value(item.as_str()));
        if !carried.is_empty()
            && let Some(mut k) = table.key_mut(&key)
        {
            k.leaf_decor_mut().set_prefix(std::mem::take(&mut carried));
        }
    }
    // Nothing left to carry the comments: they stay right under the table's header.
    let comments: String = carried
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with('#'))
        .map(|line| format!("\n{line}"))
        .collect();
    if !comments.is_empty()
        && let Some(table) = doc["hotkeys"].as_table_mut()
    {
        let suffix = table
            .decor()
            .suffix()
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        table.decor_mut().set_suffix(format!("{suffix}{comments}"));
    }
    Ok(doc.to_string())
}

/// `text` with the key at `path` (one or more segments: a built-in key, an unrecognized one)
/// set to `value`, or removed for `None`. Only that key changes: the file's comments, order
/// and layout stay, and a key is edited in the form the user wrote it (header, dotted or
/// inline). The comment lines above a removed key go to the next key. A table a removal
/// leaves with no keys and no comments is removed; a table the edit creates is a plain
/// `[a]` header. Errors when `text`, or the result, is not valid settings: a broken file is
/// never rewritten and a broken one is never produced.
pub fn set_key(text: &str, path: &[&str], value: Option<Value>) -> anyhow::Result<String> {
    anyhow::ensure!(!path.is_empty(), "no key to set");
    Settings::parse(text)?;
    let mut doc: DocumentMut = text.parse().context("invalid TOML")?;
    let (left_over, _) = edit_in(doc.as_table_mut(), path, value)?;
    if !left_over.is_empty() {
        let trailing = doc.trailing().as_str().unwrap_or_default().to_string();
        doc.set_trailing(format!("{trailing}{}", comment_lines(&left_over)));
    }
    let out = doc.to_string();
    Settings::parse(&out).context("the edit would make config.toml invalid")?;
    Ok(out)
}

/// `text` with the keys that moved in the schema at their new place, or `None` when it has
/// none (or does not parse, which is left for the user to fix). Today: the calculator's
/// `[plugins.calc] timezone` is `[location] timezone`; kept when that is not set yet, dropped
/// when it is.
pub fn migrate(text: &str) -> Option<String> {
    let file: toml::Table = text.parse().ok()?;
    let old = file
        .get("plugins")
        .and_then(|p| p.get("calc"))
        .and_then(|c| c.get("timezone"))?;
    let set = file
        .get("location")
        .and_then(|l| l.get("timezone"))
        .is_some();
    let mut out = text.to_string();
    if let (false, toml::Value::String(zone)) = (set, old)
        && !zone.trim().is_empty()
    {
        out = set_key(
            &out,
            &["location", "timezone"],
            Some(Value::from(zone.trim())),
        )
        .ok()?;
    }
    set_key(&out, &["plugins", "calc", "timezone"], None).ok()
}

/// `text` with `key` of `[plugins.<plugin>]` set to `value`, or removed for `None`. A new
/// table is written as a `[plugins.<plugin>]` header.
pub fn set_plugin_preference(
    text: &str,
    plugin: &str,
    key: &str,
    value: Option<Value>,
) -> anyhow::Result<String> {
    set_key(text, &["plugins", plugin, key], value)
}

/// `text` without the `[plugins.<plugin>]` table (and the comment lines above its header).
pub fn remove_plugin_table(text: &str, plugin: &str) -> anyhow::Result<String> {
    set_key(text, &["plugins", plugin], None)
}

/// Sets or removes the key at `path` below `table`. Returns the comment lines of a removed
/// key that no key below `table` could take, and whether a key was actually removed (only
/// then are emptied tables pruned: a no-op leaves the user's empty tables alone).
fn edit_in(
    table: &mut dyn TableLike,
    path: &[&str],
    value: Option<Value>,
) -> anyhow::Result<(String, bool)> {
    let Some((&key, rest)) = path.split_first() else {
        return Ok((String::new(), false));
    };
    if rest.is_empty() {
        return Ok(match value {
            Some(value) => {
                set_leaf(table, key, value);
                (String::new(), false)
            }
            None => {
                let existed = table.contains_key(key);
                (remove_leaf(table, key), existed)
            }
        });
    }
    if !table.contains_key(key) {
        if value.is_none() {
            return Ok((String::new(), false));
        }
        let mut created = Table::new();
        // `[a.b]` is one header: `a` itself prints none.
        created.set_implicit(rest.len() > 1);
        table.insert(key, Item::Table(created));
    }
    let child = table.get_mut(key).context("the key vanished")?;
    let inner = child
        .as_table_like_mut()
        .with_context(|| format!("`{key}` is not a table"))?;
    let (mut left_over, removed) = edit_in(inner, rest, value)?;
    if let Item::Table(t) = child
        && !left_over.is_empty()
    {
        let suffix = t.decor().suffix().and_then(|s| s.as_str()).unwrap_or("");
        let suffix = format!("{suffix}{}", comment_lines(&left_over));
        t.decor_mut().set_suffix(suffix);
        left_over.clear();
    }
    if removed && is_empty_without_comments(child) {
        table.remove(key);
    }
    Ok((left_over, removed))
}

fn set_leaf(table: &mut dyn TableLike, key: &str, value: Value) {
    match table.get_mut(key) {
        Some(Item::Value(old)) => {
            // The value changes; its spacing and the comment after it stay.
            let decor = old.decor().clone();
            *old = value;
            *old.decor_mut() = decor;
        }
        _ => {
            table.insert(key, Item::Value(value));
        }
    }
}

/// Removes `key`. The comment lines above it move to the key after it; returned when there
/// is none.
fn remove_leaf(table: &mut dyn TableLike, key: &str) -> String {
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    let Some(at) = keys.iter().position(|k| k == key) else {
        return String::new();
    };
    let prefix = table
        .key(key)
        .and_then(|k| k.leaf_decor().prefix())
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    table.remove(key);
    if let Some(next) = keys.get(at + 1)
        && let Some(mut k) = table.key_mut(next)
    {
        let own = k
            .leaf_decor()
            .prefix()
            .and_then(|p| p.as_str())
            .unwrap_or("")
            .to_string();
        k.leaf_decor_mut().set_prefix(format!("{prefix}{own}"));
        return String::new();
    }
    prefix
}

/// The comment lines of `text`, each after a line break (to follow a table header).
fn comment_lines(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('#'))
        .map(|line| format!("\n{line}"))
        .collect()
}

/// Whether `item` is a table with no keys and no comment in its header's lines.
fn is_empty_without_comments(item: &Item) -> bool {
    let has_comment = |decor: &toml_edit::Decor| {
        [decor.prefix(), decor.suffix()]
            .into_iter()
            .flatten()
            .any(|part| part.as_str().is_some_and(|s| s.contains('#')))
    };
    match item {
        Item::Table(t) => t.is_empty() && !has_comment(t.decor()),
        Item::Value(Value::InlineTable(t)) => t.is_empty() && !has_comment(t.decor()),
        _ => false,
    }
}

/// Writes `text` to `path` atomically: a temp file in the target's directory, then a rename.
/// A symlinked `path` is resolved first, so the link survives. A missing file is created.
pub fn write_config(path: &Path, text: &str) -> anyhow::Result<()> {
    let target = match std::fs::canonicalize(path) {
        Ok(target) => target,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(e) => return Err(e).with_context(|| format!("resolving {}", path.display())),
    };
    let dir = target
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!(".config.toml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(&target) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    std::fs::rename(&tmp, &target)
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
        .with_context(|| format!("replacing {}", target.display()))
}

/// Makes sure the config file exists, creating its folders and an empty file when it is
/// missing. `Ok(true)` when it was created, `Ok(false)` when it was already there; an existing
/// file is never touched.
pub fn ensure_config_file(path: &Path) -> std::io::Result<bool> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}
