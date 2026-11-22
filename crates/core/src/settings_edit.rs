//! Writing the `[hotkeys]` table back into `config.toml`, keeping the user's comments,
//! order and layout.

use std::path::Path;

use anyhow::Context as _;
use toml_edit::{DocumentMut, Item, Table, value};

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
    Settings::from_toml(text)?;
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
