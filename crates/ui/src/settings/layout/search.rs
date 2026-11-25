//! Search over the settings: which pages, and which of their groups, a query keeps. A row
//! matches when the query is in its title or one of its keywords (the key name among them), a
//! group when its title or one of its rows matches, a page when one of its groups does. A page
//! whose title matches is kept whole, groups or none. Matching ignores case. A group that
//! opens and closes is opened by a query that matches its title or some of its rows; then only
//! those rows are drawn.

/// The text of a page that search reads.
pub(crate) struct PageText {
    pub(crate) title: String,
    pub(crate) groups: Vec<GroupText>,
}

/// The text of a group that search reads.
pub(crate) struct GroupText {
    pub(crate) title: Option<String>,
    pub(crate) rows: Vec<RowText>,
}

/// The text of a row that search reads.
pub(crate) struct RowText {
    pub(crate) title: Option<String>,
    pub(crate) keywords: Vec<String>,
}

/// For each of `pages`, the indices of its groups `query` keeps, or `None` when it keeps
/// nothing of the page; an empty query keeps everything.
pub(crate) fn filter(pages: &[PageText], query: &str) -> Vec<Option<Vec<usize>>> {
    let query = normalized(query);
    let has = |text: &str| text.to_lowercase().contains(&query);
    pages
        .iter()
        .map(|page| {
            // The empty query is in every title.
            let whole = has(&page.title);
            let groups: Vec<usize> = page
                .groups
                .iter()
                .enumerate()
                .filter(|(_, group)| {
                    whole
                        || group.title.as_deref().is_some_and(has)
                        || group.rows.iter().any(|row| row_matches(row, &has))
                })
                .map(|(ix, _)| ix)
                .collect();
            (whole || !groups.is_empty()).then_some(groups)
        })
        .collect()
}

/// For each group of each of `pages`, the rows `query` opens it on (see [`revealed_rows`]);
/// with an empty query, no page has any.
pub(crate) type Revealed = Vec<Vec<Option<Vec<usize>>>>;

pub(crate) fn revealed(pages: &[PageText], query: &str) -> Revealed {
    if normalized(query).is_empty() {
        return Vec::new();
    }
    pages
        .iter()
        .map(|page| {
            page.groups
                .iter()
                .map(|group| revealed_rows(group, query))
                .collect()
        })
        .collect()
}

/// The rows of a collapsible group that `query` opens it on: all of them when it matches the
/// group's title, those that match otherwise. `None` when the query is empty or matches
/// nothing of the group: the group stays as it is.
pub(crate) fn revealed_rows(group: &GroupText, query: &str) -> Option<Vec<usize>> {
    let query = normalized(query);
    if query.is_empty() {
        return None;
    }
    let has = |text: &str| text.to_lowercase().contains(&query);
    let whole = group.title.as_deref().is_some_and(has);
    let rows: Vec<usize> = group
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| whole || row_matches(row, &has))
        .map(|(ix, _)| ix)
        .collect();
    (!rows.is_empty()).then_some(rows)
}

fn normalized(query: &str) -> String {
    query.trim().to_lowercase()
}

fn row_matches(row: &RowText, has: &impl Fn(&str) -> bool) -> bool {
    row.title.as_deref().is_some_and(has) || row.keywords.iter().any(|keyword| has(keyword))
}

#[cfg(test)]
mod tests {
    use super::{GroupText, PageText, RowText, filter, revealed_rows};

    fn row(title: &str, keywords: &[&str]) -> RowText {
        RowText {
            title: Some(title.to_string()),
            keywords: keywords.iter().map(|k| k.to_string()).collect(),
        }
    }

    fn group(title: Option<&str>, rows: Vec<RowText>) -> GroupText {
        GroupText {
            title: title.map(str::to_string),
            rows,
        }
    }

    /// General (one untitled group) and Weather (Preferences, Commands).
    fn pages() -> Vec<PageText> {
        vec![
            PageText {
                title: "General".into(),
                groups: vec![group(
                    None,
                    vec![
                        row("Launcher hotkey", &["shortcut", "launcher.hotkey"]),
                        row("Start at login", &["autostart"]),
                    ],
                )],
            },
            PageText {
                title: "Weather".into(),
                groups: vec![
                    group(
                        Some("Preferences"),
                        vec![row("Units", &["units"]), row("Wind", &["wind"])],
                    ),
                    group(Some("Commands"), vec![row("Forecast", &["forecast"])]),
                ],
            },
        ]
    }

    #[test]
    fn an_empty_query_keeps_everything() {
        assert_eq!(filter(&pages(), ""), [Some(vec![0]), Some(vec![0, 1])]);
    }

    #[test]
    fn a_row_title_match_keeps_its_group() {
        assert_eq!(filter(&pages(), "Forecast"), [None, Some(vec![1])]);
    }

    #[test]
    fn a_group_title_match_keeps_the_group() {
        assert_eq!(filter(&pages(), "Preferences"), [None, Some(vec![0])]);
    }

    #[test]
    fn a_key_name_matches() {
        let pages = vec![PageText {
            title: "Location".into(),
            groups: vec![
                group(None, vec![row("City", &["location.city"])]),
                group(
                    None,
                    vec![row("Look up location by Wi-Fi", &["location.wifi_lookup"])],
                ),
            ],
        }];
        assert_eq!(filter(&pages, "wifi_l"), [Some(vec![1])]);
    }

    #[test]
    fn a_page_title_match_keeps_the_whole_page() {
        assert_eq!(filter(&pages(), "weath"), [None, Some(vec![0, 1])]);
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(filter(&pages(), "START AT"), [Some(vec![0]), None]);
        assert_eq!(filter(&pages(), "units"), filter(&pages(), "UnItS"));
    }

    #[test]
    fn no_match_leaves_every_page_empty() {
        assert_eq!(filter(&pages(), "zzz"), [None, None]);
    }

    #[test]
    fn a_page_without_groups_stays_on_an_empty_query_or_a_title_match() {
        let pages = vec![PageText {
            title: "About".into(),
            groups: Vec::new(),
        }];
        assert_eq!(filter(&pages, ""), [Some(vec![])]);
        assert_eq!(filter(&pages, "abo"), [Some(vec![])]);
        assert_eq!(filter(&pages, "zzz"), [None]);
    }

    #[test]
    fn a_collapsible_group_opens_on_its_matching_rows() {
        let apps = group(
            Some("Applications"),
            vec![
                row("Firefox", &["apps/firefox"]),
                row("Files", &["apps/files"]),
                row("KeePassXC", &["apps/keepassxc"]),
            ],
        );
        assert_eq!(revealed_rows(&apps, ""), None);
        assert_eq!(revealed_rows(&apps, "fire"), Some(vec![0]));
        assert_eq!(revealed_rows(&apps, "FI"), Some(vec![0, 1]));
        assert_eq!(revealed_rows(&apps, "applic"), Some(vec![0, 1, 2]));
        assert_eq!(revealed_rows(&apps, "zzz"), None);
    }

    /// A query that is in the page's title ("Hotkeys") still opens a group on its rows that
    /// match it; one that matches none of them leaves the group as it is.
    #[test]
    fn a_query_in_the_page_title_still_opens_matching_rows() {
        let apps = group(
            Some("Applications"),
            vec![
                row("Firefox", &["apps/firefox"]),
                row("KeyCastr", &["apps/keycastr"]),
            ],
        );
        // "key" is in "Hotkeys" and in "KeyCastr".
        assert_eq!(revealed_rows(&apps, "key"), Some(vec![1]));
        assert_eq!(revealed_rows(&apps, "hot"), None);
        let pages = vec![PageText {
            title: "Hotkeys".into(),
            groups: vec![apps],
        }];
        assert_eq!(filter(&pages, "key"), [Some(vec![0])]);
    }
}
