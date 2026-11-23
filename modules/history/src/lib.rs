//! `history` module: the "Search History" command and its list view. The history itself
//! is core data (`QueryHistory`); this built-in module is its only reader and editor.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::bail;
use tomakast_core::{
    Action, Effect, Item, ItemId, ItemKind, ListLayout, ListRow, ListView, Module, ModuleContext,
    QueryHistory, SearchMode, Toast, ToastKind, View, ViewEvent, ViewId,
};

/// The provider id; the daemon exempts it from recording its own query.
pub const PROVIDER: &str = "history";

const COMMAND: &str = "search-history";
const VIEW: &str = "history";
const TITLE: &str = "Search History";

/// Unix seconds now; injectable for tests.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub struct HistoryModule {
    history: QueryHistory,
    clock: Clock,
}

impl HistoryModule {
    pub fn new(history: QueryHistory) -> HistoryModule {
        HistoryModule::with_clock(history, Arc::new(system_now))
    }

    pub fn with_clock(history: QueryHistory, clock: Clock) -> HistoryModule {
        HistoryModule { history, clock }
    }

    fn list(&self) -> View {
        let now = (self.clock)();
        let rows = self
            .history
            .entries()
            .into_iter()
            .map(|e| ListRow {
                id: e.id,
                section: None,
                title: e.query,
                subtitle: Some(ago(now, e.at_unix)),
                icon: None,
                accessories: vec![],
                detail_markdown: None,
                actions: row_actions(),
            })
            .collect();
        View::List(ListView {
            id: ViewId(VIEW.to_string()),
            title: TITLE.to_string(),
            placeholder: "Filter search history…".to_string(),
            search: SearchMode::CoreFilter,
            query: String::new(),
            revision: 0,
            rows,
            show_detail: false,
            layout: ListLayout::Rows,
            footer_action: None,
            busy: false,
        })
    }

    fn cleared(&self) -> Vec<Effect> {
        let mut view = self.list();
        if let View::List(v) = &mut view {
            v.rows.clear();
        }
        vec![
            Effect::UpdateView(view),
            toast(ToastKind::Success, "Search history cleared", None),
        ]
    }
}

fn system_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn toast(kind: ToastKind, title: &str, message: Option<String>) -> Effect {
    Effect::Toast(Toast {
        kind,
        title: title.to_string(),
        message,
    })
}

fn search_item() -> Item {
    Item {
        id: ItemId::new(PROVIDER, COMMAND),
        title: TITLE.to_string(),
        subtitle: Some("Queries you typed in the launcher".to_string()),
        keywords: ["history", "recent", "queries"].map(String::from).to_vec(),
        icon: None,
        kind: ItemKind::Command,
        source: None,
        actions: vec![Action {
            id: "open".to_string(),
            title: "Open".to_string(),
            shortcut: None,
        }],
    }
}

fn row_actions() -> Vec<Action> {
    let action = |id: &str, title: &str, shortcut: Option<&str>| Action {
        id: id.to_string(),
        title: title.to_string(),
        shortcut: shortcut.map(String::from),
    };
    vec![
        action("search", "Search", None),
        action("delete", "Delete Entry", Some("ctrl+shift+x")),
        // Panel only, on purpose: no shortcut to hit by accident.
        action("clear", "Clear Search History", None),
    ]
}

/// "just now" (< 60 s, or in the future), "{m} min ago" (< 1 h), "{h} h ago" (< 24 h),
/// "{d} d ago".
pub fn ago(now_unix: i64, at_unix: i64) -> String {
    let secs = now_unix - at_unix;
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{} min ago", secs / 60)
    } else if secs < 86_400 {
        format!("{} h ago", secs / 3600)
    } else {
        format!("{} d ago", secs / 86_400)
    }
}

impl Module for HistoryModule {
    fn id(&self) -> &str {
        PROVIDER
    }

    fn start(&mut self, ctx: ModuleContext) {
        ctx.replace_items(vec![search_item()]);
    }

    fn run_action(&mut self, _item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        match action_id {
            "open" => Ok(vec![Effect::PushView(self.list())]),
            other => bail!("unknown action {other:?}"),
        }
    }

    fn on_view_event(&mut self, _view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
        let ViewEvent::ActionChosen { row_id, action_id } = event else {
            return Ok(vec![]);
        };
        let failed = |e: anyhow::Error| {
            vec![toast(
                ToastKind::Error,
                "Cannot change the search history",
                Some(format!("{e:#}")),
            )]
        };
        let gone = || {
            vec![
                Effect::UpdateView(self.list()),
                toast(
                    ToastKind::Error,
                    "This entry is no longer in the history",
                    None,
                ),
            ]
        };
        match action_id.as_str() {
            "search" => match self.history.entries().into_iter().find(|e| e.id == row_id) {
                Some(e) => Ok(vec![Effect::SetRootQuery(e.query)]),
                None => Ok(gone()),
            },
            "delete" => match self.history.delete(&row_id) {
                Ok(true) => Ok(vec![Effect::UpdateView(self.list())]),
                Ok(false) => Ok(gone()),
                Err(e) => Ok(failed(e)),
            },
            "clear" => match self.history.clear() {
                Ok(()) => Ok(self.cleared()),
                Err(e) => Ok(failed(e)),
            },
            other => bail!("unknown action {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tomakast_core::store::Store;

    const NOW: i64 = 1_700_000_000;

    fn history() -> QueryHistory {
        QueryHistory::load(Store::open_in_memory().unwrap()).unwrap()
    }

    fn module(h: &QueryHistory, now: i64) -> HistoryModule {
        HistoryModule::with_clock(h.clone(), Arc::new(move || now))
    }

    fn open(m: &mut HistoryModule) -> ListView {
        let effects = m.run_action(&search_item(), "open").unwrap();
        let [Effect::PushView(View::List(v))] = <[Effect; 1]>::try_from(effects).unwrap() else {
            panic!("expected one PushView(List)");
        };
        v
    }

    fn two() -> (QueryHistory, HistoryModule) {
        let h = history();
        h.record("aa", NOW - 3 * 86_400);
        h.record("bb", NOW - 7_200);
        let m = module(&h, NOW);
        (h, m)
    }

    fn choose(m: &mut HistoryModule, row_id: &str, action: &str) -> anyhow::Result<Vec<Effect>> {
        m.on_view_event(
            &ViewId(VIEW.into()),
            ViewEvent::ActionChosen {
                row_id: row_id.into(),
                action_id: action.into(),
            },
        )
    }

    fn id_of(h: &QueryHistory, q: &str) -> String {
        h.entries().into_iter().find(|e| e.query == q).unwrap().id
    }

    fn titles(v: &View) -> Vec<String> {
        let View::List(v) = v else {
            panic!("not a list")
        };
        v.rows.iter().map(|r| r.title.clone()).collect()
    }

    #[test]
    fn ago_formats_minutes_hours_and_days() {
        assert_eq!(ago(1000, 1000), "just now");
        assert_eq!(ago(1059, 1000), "just now");
        assert_eq!(ago(900, 1000), "just now");
        assert_eq!(ago(1060, 1000), "1 min ago");
        assert_eq!(ago(1000 + 3599, 1000), "59 min ago");
        assert_eq!(ago(1000 + 3600, 1000), "1 h ago");
        assert_eq!(ago(1000 + 86_399, 1000), "23 h ago");
        assert_eq!(ago(1000 + 86_400, 1000), "1 d ago");
        assert_eq!(ago(1000 + 3 * 86_400, 1000), "3 d ago");
    }

    #[test]
    fn search_history_is_one_command() {
        let item = search_item();
        assert_eq!(item.id, ItemId::new("history", "search-history"));
        assert_eq!(item.title, "Search History");
        assert_eq!(item.kind, ItemKind::Command);
        assert_eq!(item.actions[0].id, "open");
        assert!(item.id.is_valid());
    }

    #[test]
    fn open_lists_newest_first_with_ago() {
        let (_h, mut m) = two();
        let v = open(&mut m);
        let t: Vec<_> = v.rows.iter().map(|r| r.title.as_str()).collect();
        assert_eq!(t, ["bb", "aa"]);
        let s: Vec<_> = v.rows.iter().map(|r| r.subtitle.as_deref()).collect();
        assert_eq!(s, [Some("2 h ago"), Some("3 d ago")]);
        assert_eq!(v.search, SearchMode::CoreFilter);
        assert_eq!(v.id, ViewId("history".into()));
        assert_eq!(v.title, "Search History");
    }

    #[test]
    fn row_actions_are_search_delete_clear() {
        let (_h, mut m) = two();
        let v = open(&mut m);
        let a = &v.rows[0].actions;
        let triples: Vec<_> = a
            .iter()
            .map(|x| (x.id.as_str(), x.title.as_str(), x.shortcut.as_deref()))
            .collect();
        assert_eq!(
            triples,
            [
                ("search", "Search", None),
                ("delete", "Delete Entry", Some("ctrl+shift+x")),
                ("clear", "Clear Search History", None),
            ]
        );
        assert!(a[1].hotkey().unwrap().is_ok());
    }

    #[test]
    fn search_puts_the_query_in_the_root_field() {
        let (h, mut m) = two();
        let e = choose(&mut m, &id_of(&h, "bb"), "search").unwrap();
        assert_eq!(e, vec![Effect::SetRootQuery("bb".into())]);
    }

    #[test]
    fn delete_entry_removes_it_and_updates_the_list() {
        let (h, mut m) = two();
        let e = choose(&mut m, &id_of(&h, "bb"), "delete").unwrap();
        assert_eq!(e.len(), 1);
        let Effect::UpdateView(v) = &e[0] else {
            panic!("{e:?}")
        };
        assert_eq!(titles(v), ["aa"]);
        let left: Vec<_> = h.entries().into_iter().map(|e| e.query).collect();
        assert_eq!(left, ["aa"]);
    }

    #[test]
    fn clear_empties_history_and_list() {
        let (h, mut m) = two();
        let e = choose(&mut m, &id_of(&h, "bb"), "clear").unwrap();
        assert_eq!(e.len(), 2);
        let Effect::UpdateView(v) = &e[0] else {
            panic!("{e:?}")
        };
        assert!(titles(v).is_empty());
        assert_eq!(
            e[1],
            Effect::Toast(Toast {
                kind: ToastKind::Success,
                title: "Search history cleared".into(),
                message: None,
            })
        );
        assert!(h.entries().is_empty());
    }

    #[test]
    fn a_deleted_row_answers_with_an_error_toast() {
        let (h, mut m) = two();
        let bb = id_of(&h, "bb");
        choose(&mut m, &bb, "delete").unwrap();
        let e = choose(&mut m, &bb, "search").unwrap();
        assert_eq!(e.len(), 2);
        let Effect::UpdateView(v) = &e[0] else {
            panic!("{e:?}")
        };
        assert_eq!(titles(v), ["aa"]);
        assert_eq!(
            e[1],
            Effect::Toast(Toast {
                kind: ToastKind::Error,
                title: "This entry is no longer in the history".into(),
                message: None,
            })
        );
    }

    #[test]
    fn unknown_action_is_an_error() {
        let (h, mut m) = two();
        assert!(choose(&mut m, &id_of(&h, "bb"), "nope").is_err());
    }
}
