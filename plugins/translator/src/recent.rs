//! Recent translations: the last texts translated and copied (or left in the pad), with their
//! languages, kept in storage under `recent` as JSON.

use tomakast_plugin::{ListRow, SearchMode, View};

use crate::json::{self, Json};
use crate::langs;

/// The storage key.
pub const KEY: &str = "recent";
/// The view id of the list.
pub const VIEW: &str = "recent";
/// How many are kept.
pub const MAX: usize = 20;
/// The longest text kept, in bytes (cut at a character boundary).
pub const MAX_TEXT: usize = 8 * 1024;
/// The stored value stays under this.
const MAX_VALUE: usize = 200 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Recent {
    pub text: String,
    /// `None`: detected.
    pub from: Option<String>,
    pub to: String,
}

/// The list in `raw`; anything unreadable is an empty list.
pub fn load(raw: &str) -> Vec<Recent> {
    let Some(Json::Arr(items)) = json::parse(raw) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let text = str_of(item.get("text"))?.to_string();
            let to = str_of(item.get("to"))?.to_string();
            let from = str_of(item.get("from")).map(String::from);
            (!text.is_empty()).then_some(Recent { text, from, to })
        })
        .take(MAX)
        .collect()
}

fn str_of(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Str(s)) => Some(s),
        _ => None,
    }
}

/// `list` as stored.
pub fn save(list: &[Recent]) -> String {
    let items: Vec<String> = list
        .iter()
        .map(|r| {
            let from = r.from.as_deref().map_or_else(|| "null".to_string(), quote);
            format!(
                "{{\"text\":{},\"from\":{from},\"to\":{}}}",
                quote(&r.text),
                quote(&r.to)
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// A JSON string.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Puts `entry` first: its text cut to [`MAX_TEXT`], an older entry of the same text
/// dropped, at most [`MAX`] kept, and the oldest dropped while the value is too large.
pub fn remember(list: &mut Vec<Recent>, mut entry: Recent) {
    if entry.text.trim().is_empty() {
        return;
    }
    if entry.text.len() > MAX_TEXT {
        let mut end = MAX_TEXT;
        while !entry.text.is_char_boundary(end) {
            end -= 1;
        }
        entry.text.truncate(end);
    }
    list.retain(|r| r.text != entry.text);
    list.insert(0, entry);
    list.truncate(MAX);
    while list.len() > 1 && save(list).len() >= MAX_VALUE {
        list.pop();
    }
}

/// The list view: one row per entry, newest first, its first line and its languages.
pub fn view(list: &[Recent]) -> Result<View, String> {
    let rows = list
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let first_line: String = r
                .text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(200)
                .collect();
            let from = r
                .from
                .as_deref()
                .map_or_else(|| "Detected".to_string(), langs::name);
            ListRow::new(format!("r{i}"), first_line)?
                .accessory(tomakast_plugin::Accessory::Text(format!(
                    "{from} → {}",
                    langs::name(&r.to)
                )))
                .action("open", "Open in Translator")
        })
        .collect::<Result<Vec<_>, tomakast_plugin::InvalidId>>()?;
    Ok(View::list(VIEW, "Recent Translations")?
        .placeholder("Filter recent translations")
        .search(SearchMode::CoreFilter)
        .rows(rows)
        .into())
}

/// The entry of row `r<i>`.
pub fn row(list: &[Recent], row_id: &str) -> Option<Recent> {
    let i: usize = row_id.strip_prefix('r')?.parse().ok()?;
    list.get(i).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> Recent {
        Recent {
            text: text.into(),
            from: None,
            to: "ru".into(),
        }
    }

    #[test]
    fn newest_first_capped_and_unique() {
        let mut list = Vec::new();
        for n in 0..25 {
            remember(&mut list, entry(&format!("text {n}")));
        }
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0].text, "text 24");
        remember(&mut list, entry("text 20"));
        assert_eq!(list[0].text, "text 20");
        assert_eq!(list.iter().filter(|r| r.text == "text 20").count(), 1);
        remember(&mut list, entry("   "));
        assert_eq!(list[0].text, "text 20", "blank text is not kept");
    }

    #[test]
    fn long_texts_are_cut() {
        let mut list = Vec::new();
        remember(&mut list, entry(&"ж".repeat(20 * 1024)));
        assert!(list[0].text.len() <= MAX_TEXT);
        for n in 0..MAX {
            remember(&mut list, entry(&format!("{n}{}", "x".repeat(MAX_TEXT))));
        }
        assert!(save(&list).len() < MAX_VALUE);
    }

    #[test]
    fn storage_round_trip() {
        let list = vec![
            Recent {
                text: "line \"one\"\n\ttwo \\ ё".into(),
                from: Some("en".into()),
                to: "zh-CN".into(),
            },
            entry("x"),
        ];
        assert_eq!(load(&save(&list)), list);
        assert!(load("garbage").is_empty());
        assert!(load("{}").is_empty());
        assert_eq!(row(&list, "r1"), Some(entry("x")));
        assert_eq!(row(&list, "r9"), None);
        assert_eq!(row(&list, "x1"), None);
    }
}
