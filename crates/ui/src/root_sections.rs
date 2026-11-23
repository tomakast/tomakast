//! The sections of the root list: which section labels and hits it shows, in
//! which order. Pure, so it is unit-tested apart from the view.

use tomakast_core::ItemKind;

use crate::theme::SUGGESTIONS_MAX;

/// One line of the root list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RootRow {
    /// A section label; never selectable.
    Label(&'static str),
    /// Index into the hits the rows were built from.
    Hit(usize),
}

impl RootRow {
    /// The hit this row shows; `None` for a label.
    pub(crate) fn hit(self) -> Option<usize> {
        match self {
            RootRow::Hit(ix) => Some(ix),
            RootRow::Label(_) => None,
        }
    }
}

/// The root list for `hits` (kind, title, whether the item has usage; in rank
/// order).
///
/// A typed query is one "Results" section in rank order. The empty query
/// shows "Suggestions": up to
/// [`SUGGESTIONS_MAX`] hits with any usage, by rank; then one section per
/// kind for the other hits, by lowercase title. No hit is listed twice, and
/// empty sections are left out.
pub(crate) fn root_rows(hits: &[(ItemKind, &str, bool)], query_empty: bool) -> Vec<RootRow> {
    let mut rows = Vec::with_capacity(hits.len() + 3);
    if !query_empty {
        if !hits.is_empty() {
            rows.push(RootRow::Label(RESULTS));
            rows.extend((0..hits.len()).map(RootRow::Hit));
        }
        return rows;
    }
    let mut suggested = vec![false; hits.len()];
    let suggestions = hits
        .iter()
        .enumerate()
        .filter(|(_, (_, _, used))| *used)
        .take(SUGGESTIONS_MAX);
    for (ix, _) in suggestions {
        if rows.is_empty() {
            rows.push(RootRow::Label(SUGGESTIONS));
        }
        suggested[ix] = true;
        rows.push(RootRow::Hit(ix));
    }
    // The rest by kind section, then by title; the lowercase title is
    // computed once per hit, not once per comparison.
    let mut rest: Vec<((u8, &'static str), String, usize)> = hits
        .iter()
        .enumerate()
        .filter(|(ix, _)| !suggested[*ix])
        .map(|(ix, (kind, title, _))| (kind_section(*kind), title.to_lowercase(), ix))
        .collect();
    rest.sort_unstable();
    let mut section = None;
    for ((_, label), _, ix) in rest {
        if section != Some(label) {
            section = Some(label);
            rows.push(RootRow::Label(label));
        }
        rows.push(RootRow::Hit(ix));
    }
    rows
}

const RESULTS: &str = "Results";
const SUGGESTIONS: &str = "Suggestions";
const APPLICATIONS: &str = "Applications";
const COMMANDS: &str = "Commands";
const FILES: &str = "Files";

/// The section of the empty query's list that holds items of `kind`: its
/// place among the kind sections and its label.
fn kind_section(kind: ItemKind) -> (u8, &'static str) {
    match kind {
        ItemKind::Application => (0, APPLICATIONS),
        ItemKind::Command | ItemKind::Fallback => (1, COMMANDS),
        ItemKind::File | ItemKind::Folder => (2, FILES),
    }
}

/// The kind of a root item, as its row shows it on the right.
pub(crate) fn kind_label(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Application => "Application",
        ItemKind::Command | ItemKind::Fallback => "Command",
        ItemKind::File => "File",
        ItemKind::Folder => "Folder",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ItemKind::{Application as App, Command as Cmd};
    use RootRow::{Hit, Label};

    #[test]
    fn path_kinds_have_labels() {
        assert_eq!(kind_label(ItemKind::File), "File");
        assert_eq!(kind_label(ItemKind::Folder), "Folder");
        assert_eq!(kind_section(ItemKind::File), (2, "Files"));
        assert_eq!(kind_section(ItemKind::Folder), (2, "Files"));
    }

    #[test]
    fn typed_query_is_one_results_section() {
        let hits = [
            (Cmd, "Zeta", true),
            (App, "Alpha", true),
            (App, "Beta", true),
        ];

        assert_eq!(
            root_rows(&hits, false),
            [Label("Results"), Hit(0), Hit(1), Hit(2)]
        );
    }

    #[test]
    fn empty_query_suggestions_then_kinds() {
        let hits = [
            (App, "Firefox", true),
            (Cmd, "Demo: List", true),
            (App, "Dolphin", false),
            (Cmd, "Demo: Form", false),
        ];

        assert_eq!(
            root_rows(&hits, true),
            [
                Label("Suggestions"),
                Hit(0),
                Hit(1),
                Label("Applications"),
                Hit(2),
                Label("Commands"),
                Hit(3),
            ]
        );
    }

    #[test]
    fn kind_sections_are_alphabetical_ignoring_case() {
        let hits = [
            (Cmd, "zoo", false),
            (App, "beta", false),
            (Cmd, "Alpha", false),
            (App, "Alpha", false),
        ];

        assert_eq!(
            root_rows(&hits, true),
            [
                Label("Applications"),
                Hit(3),
                Hit(1),
                Label("Commands"),
                Hit(2),
                Hit(0),
            ]
        );
    }

    #[test]
    fn suggestions_capped_at_8() {
        let titles = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        let hits: Vec<_> = titles
            .iter()
            .enumerate()
            .map(|(ix, title)| {
                let kind = if ix % 2 == 0 { App } else { Cmd };
                (kind, *title, true)
            })
            .collect();

        let rows = root_rows(&hits, true);

        let mut expected = vec![Label("Suggestions")];
        expected.extend((0..8).map(Hit));
        expected.extend([Label("Applications"), Hit(8), Label("Commands"), Hit(9)]);
        assert_eq!(rows, expected);
    }

    #[test]
    fn no_item_twice() {
        let hits: Vec<_> = (0..20)
            .map(|ix| {
                let kind = if ix % 3 == 0 { Cmd } else { App };
                (kind, "same", ix % 2 == 0)
            })
            .collect();

        for query_empty in [true, false] {
            let mut listed: Vec<usize> = root_rows(&hits, query_empty)
                .into_iter()
                .filter_map(|row| match row {
                    Hit(ix) => Some(ix),
                    Label(_) => None,
                })
                .collect();
            listed.sort_unstable();
            assert_eq!(listed, (0..20).collect::<Vec<_>>());
        }
    }

    #[test]
    fn dynamic_hit_on_empty_query_is_not_a_suggestion() {
        // A dynamic hit has no usage: the caller passes `false`.
        let hits = [(Cmd, "2 + 2", false), (App, "Firefox", true)];

        assert_eq!(
            root_rows(&hits, true),
            [Label("Suggestions"), Hit(1), Label("Commands"), Hit(0),]
        );
    }

    #[test]
    fn empty_sections_omitted() {
        assert_eq!(root_rows(&[], true), []);
        assert_eq!(root_rows(&[], false), []);
        // No usage: no "Suggestions"; no commands: no "Commands".
        assert_eq!(
            root_rows(&[(App, "Files", false)], true),
            [Label("Applications"), Hit(0)]
        );
        // Usage only: no kind sections.
        assert_eq!(
            root_rows(&[(Cmd, "Demo", true)], true),
            [Label("Suggestions"), Hit(0)]
        );
    }

    #[test]
    fn kind_labels() {
        assert_eq!(kind_label(App), "Application");
        assert_eq!(kind_label(Cmd), "Command");
    }
}
