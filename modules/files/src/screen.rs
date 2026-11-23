//! The "Search Files" screen: a list the module filters itself, drawn as rows or as a grid.

use std::fmt::Display;

use chrono::{DateTime, Datelike, TimeZone};
use tomakast_core::{Accessory, Action, Icon, ListLayout, ListRow, ListView, SearchMode, ViewId};

use crate::state::Layout;

/// The screen's view id.
pub const VIEW_ID: &str = "files/search";
/// At most this many files on the screen.
pub const LIMIT: usize = 100;
/// Tiles per row in the grid.
const GRID_COLUMNS: u8 = 5;
/// The action that switches between the list and the grid.
pub const TOGGLE: &str = "toggle-layout";

/// The screen with `rows` (at most [`LIMIT`]) for `query` at `revision`; `busy` while there
/// is no index yet (the busy line shows under the header).
pub fn view(
    query: &str,
    revision: u64,
    mut rows: Vec<ListRow>,
    layout: Layout,
    busy: bool,
) -> ListView {
    rows.truncate(LIMIT);
    ListView {
        id: ViewId(VIEW_ID.to_string()),
        title: "Search Files".to_string(),
        placeholder: "Search files…".to_string(),
        search: SearchMode::Module,
        query: query.to_string(),
        revision,
        rows,
        show_detail: false,
        layout: match layout {
            Layout::List => ListLayout::Rows,
            Layout::Grid => ListLayout::Grid {
                columns: GRID_COLUMNS,
            },
        },
        footer_action: Some(TOGGLE.to_string()),
        busy,
    }
}

/// The actions of a file or folder row on the screen: those of every file row, then the
/// layout toggle.
pub(crate) fn row_actions(layout: Layout) -> Vec<Action> {
    let mut actions = crate::path_actions();
    actions.push(Action {
        id: TOGGLE.to_string(),
        title: match layout {
            Layout::List => "Show as Grid",
            Layout::Grid => "Show as List",
        }
        .to_string(),
        shortcut: Some("ctrl+g".to_string()),
    });
    actions
}

/// What a row shows of a file or folder.
pub(crate) struct Shown {
    /// The local part of the item id.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// The type icon; in the grid, the thumbnail when there is one.
    pub icon: Icon,
    /// Empty when the time is unknown: then the row shows no date.
    pub modified: String,
}

pub(crate) fn row(shown: Shown, layout: Layout) -> ListRow {
    ListRow {
        id: shown.id,
        section: None,
        title: shown.title,
        subtitle: Some(shown.subtitle),
        icon: Some(shown.icon),
        // No date when the time is unknown.
        accessories: if shown.modified.is_empty() {
            Vec::new()
        } else {
            vec![Accessory::Text(shown.modified)]
        },
        detail_markdown: None,
        actions: row_actions(layout),
    }
}

/// The modification time `mtime` as the screen shows it, at `now` (both Unix seconds) in
/// `tz`: "Today 14:05", "Yesterday", "12 Nov" this year, "12 Nov 2024" before (or after).
/// Nothing for 0: the index holds 0 when the time could not be read.
pub fn modified_label<Tz: TimeZone>(mtime: i64, now: i64, tz: &Tz) -> String
where
    Tz::Offset: Display,
{
    if mtime == 0 {
        return String::new();
    }
    let (Some(at), Some(now)) = (
        DateTime::from_timestamp(mtime, 0),
        DateTime::from_timestamp(now, 0),
    ) else {
        return String::new();
    };
    let (at, now) = (at.with_timezone(tz), now.with_timezone(tz));
    let (day, today) = (at.date_naive(), now.date_naive());
    if day == today {
        at.format("Today %H:%M").to_string()
    } else if today.pred_opt() == Some(day) {
        "Yesterday".to_string()
    } else if day.year() == today.year() {
        at.format("%-d %b").to_string()
    } else {
        at.format("%-d %b %Y").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    #[test]
    fn modified_date_labels() {
        // Three hours east of UTC: a time after local midnight is still the day before in
        // UTC, so the offset has to be applied.
        let tz = FixedOffset::east_opt(3 * 3600).unwrap();
        let at = |y, mo, d, h, mi| tz.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap().timestamp();
        let now = at(2026, 11, 22, 14, 5);
        let cases = [
            (at(2026, 11, 22, 14, 5), "Today 14:05"),
            (at(2026, 11, 22, 0, 30), "Today 00:30"),
            (at(2026, 11, 21, 23, 59), "Yesterday"),
            (at(2026, 11, 21, 0, 0), "Yesterday"),
            (at(2026, 11, 20, 23, 59), "20 Nov"),
            (at(2026, 11, 12, 9, 0), "12 Nov"),
            (at(2026, 1, 3, 9, 0), "3 Jan"),
            (at(2024, 11, 12, 9, 0), "12 Nov 2024"),
            (at(2027, 2, 1, 9, 0), "1 Feb 2027"),
        ];
        for (mtime, label) in cases {
            assert_eq!(modified_label(mtime, now, &tz), label, "{mtime}");
        }
        // Across a new year, yesterday is still "Yesterday", and last year has its year.
        let new_year = at(2027, 1, 1, 10, 0);
        assert_eq!(
            modified_label(at(2026, 12, 31, 22, 0), new_year, &tz),
            "Yesterday"
        );
        assert_eq!(
            modified_label(at(2026, 12, 30, 22, 0), new_year, &tz),
            "30 Dec 2026"
        );
    }

    #[test]
    fn unknown_modified_time_shows_no_date() {
        let tz = FixedOffset::east_opt(0).unwrap();
        assert_eq!(modified_label(0, 1_800_000_000, &tz), "");
        let shown = Shown {
            id: "0".repeat(32),
            title: "old.txt".into(),
            subtitle: "~".into(),
            icon: Icon::Named("text-plain".into()),
            modified: String::new(),
        };
        assert!(row(shown, Layout::List).accessories.is_empty());
    }

    #[test]
    fn screen_view_shape() {
        let shown = |n: usize| Shown {
            id: format!("{n:032x}"),
            title: format!("{n}.txt"),
            subtitle: "~".into(),
            icon: Icon::Named("text-plain".into()),
            modified: "Today 14:05".into(),
        };
        let rows: Vec<ListRow> = (0..LIMIT + 5)
            .map(|n| row(shown(n), Layout::List))
            .collect();
        let list = view("fire", 7, rows, Layout::List, false);
        assert_eq!(list.id, ViewId(VIEW_ID.into()));
        assert_eq!(list.title, "Search Files");
        assert_eq!(list.placeholder, "Search files…");
        assert_eq!(list.search, SearchMode::Module);
        assert_eq!(list.query, "fire");
        assert_eq!(list.revision, 7);
        assert_eq!(list.layout, ListLayout::Rows);
        assert!(!list.show_detail);
        assert_eq!(list.rows.len(), LIMIT);
        let first = &list.rows[0];
        assert_eq!(first.id, format!("{:032x}", 0));
        assert_eq!(first.title, "0.txt");
        assert_eq!(first.subtitle.as_deref(), Some("~"));
        assert_eq!(first.icon, Some(Icon::Named("text-plain".into())));
        assert_eq!(first.accessories, [Accessory::Text("Today 14:05".into())]);
        assert_eq!(first.section, None);

        let grid = view(
            "",
            0,
            vec![row(shown(1), Layout::Grid)],
            Layout::Grid,
            false,
        );
        assert_eq!(grid.layout, ListLayout::Grid { columns: 5 });
    }

    #[test]
    fn rows_carry_the_layout_toggle() {
        for (layout, title) in [
            (Layout::List, "Show as Grid"),
            (Layout::Grid, "Show as List"),
        ] {
            let actions = row_actions(layout);
            let ids: Vec<&str> = actions.iter().map(|a| a.id.as_str()).collect();
            let mut expected = vec!["open", "reveal", "copy-path"];
            if cfg!(target_os = "linux") {
                expected.push("open-with");
            }
            expected.push(TOGGLE);
            assert_eq!(ids, expected);
            let toggle = actions.last().unwrap();
            assert_eq!(toggle.title, title);
            assert_eq!(toggle.shortcut.as_deref(), Some("ctrl+g"));
        }
    }
}
