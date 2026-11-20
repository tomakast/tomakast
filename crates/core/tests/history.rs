use tomakast_core::store::Store;
use tomakast_core::usage::UsageRecord;
use tomakast_core::{HistoryEntry, ItemId, LocalNow, QueryHistory};

fn mem() -> QueryHistory {
    QueryHistory::load(Store::open_in_memory().unwrap()).unwrap()
}

fn queries(h: &QueryHistory) -> Vec<String> {
    h.entries().into_iter().map(|e| e.query).collect()
}

fn id_of(h: &QueryHistory, query: &str) -> String {
    let entries: Vec<HistoryEntry> = h.entries();
    entries.into_iter().find(|e| e.query == query).unwrap().id
}

fn file() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    (dir, path)
}

fn reload(path: &std::path::Path) -> QueryHistory {
    QueryHistory::load(Store::open(path).unwrap()).unwrap()
}

#[test]
fn record_skips_blank_and_one_character_queries() {
    let h = mem();
    for q in ["", "a", " a ", "é", "   "] {
        assert!(!h.record(q, 1), "{q:?}");
    }
    assert!(h.record("ab", 1));
    assert_eq!(queries(&h), ["ab"]);
}

#[test]
fn record_trims_and_newest_use_wins_keeping_the_id() {
    let h = mem();
    h.record("  Firefox ", 10);
    let first = id_of(&h, "Firefox");
    h.record("firefox", 11);
    h.record("Firefox", 12);
    assert_eq!(queries(&h), ["Firefox", "firefox"]);
    assert_eq!(id_of(&h, "Firefox"), first);
}

#[test]
fn history_order_survives_a_reload_within_one_second() {
    let (_d, path) = file();
    let h = reload(&path);
    for q in ["aa", "bb", "aa", "cc"] {
        h.record(q, 100);
    }
    assert_eq!(queries(&h), ["cc", "aa", "bb"]);
    assert_eq!(queries(&reload(&path)), ["cc", "aa", "bb"]);
}

#[test]
fn record_keeps_the_newest_1000() {
    let (_d, path) = file();
    let h = reload(&path);
    for i in 0..1001 {
        h.record(&format!("q{i}"), i);
    }
    let check = |h: &QueryHistory| {
        assert_eq!(h.entries().len(), 1000);
        assert_eq!(h.nth_query(0), Some("q1000".into()));
        assert_eq!(h.nth_query(999), Some("q1".into()));
    };
    check(&h);
    check(&reload(&path));
    assert_eq!(
        Store::open(&path)
            .unwrap()
            .load_query_history(2000)
            .unwrap()
            .len(),
        1000
    );
}

#[test]
fn load_prunes_a_store_beyond_the_cap() {
    let (_d, path) = file();
    let store = Store::open(&path).unwrap();
    for i in 0..1005 {
        store
            .upsert_query(&uuid::Uuid::new_v4().to_string(), &format!("q{i}"), i)
            .unwrap();
    }
    assert_eq!(QueryHistory::load(store).unwrap().entries().len(), 1000);
    let again = Store::open(&path).unwrap();
    assert_eq!(again.load_query_history(2000).unwrap().len(), 1000);
}

#[test]
fn nth_query_counts_back_from_the_newest() {
    let h = mem();
    h.record("aa", 1);
    h.record("bb", 2);
    assert_eq!(h.nth_query(0), Some("bb".into()));
    assert_eq!(h.nth_query(1), Some("aa".into()));
    assert_eq!(h.nth_query(2), None);
}

#[test]
fn delete_removes_the_entry_from_memory_and_disk() {
    let (_d, path) = file();
    let h = reload(&path);
    h.record("aa", 1);
    h.record("bb", 2);
    let id = id_of(&h, "aa");
    assert!(h.delete(&id).unwrap());
    assert!(!h.delete(&id).unwrap());
    assert_eq!(queries(&h), ["bb"]);
    assert_eq!(queries(&reload(&path)), ["bb"]);
}

#[test]
fn clear_empties_memory_and_disk() {
    let (_d, path) = file();
    let h = reload(&path);
    h.record("aa", 1);
    h.clear().unwrap();
    assert!(h.entries().is_empty());
    assert!(reload(&path).entries().is_empty());
}

#[test]
fn clear_and_delete_leave_usage_alone() {
    let (_d, path) = file();
    let store = Store::open(&path).unwrap();
    store
        .record_usage(&UsageRecord::new(
            ItemId::new("fake", "a"),
            "aa",
            LocalNow::from_unix_with_offset(1, 0),
        ))
        .unwrap();
    let h = QueryHistory::load(store).unwrap();
    h.record("aa", 1);
    h.record("bb", 2);
    h.delete(&id_of(&h, "aa")).unwrap();
    h.clear().unwrap();
    assert_eq!(Store::open(&path).unwrap().load_usage(0).unwrap().len(), 1);
}

#[test]
fn delete_keeps_memory_when_the_store_fails() {
    let (_d, path) = file();
    let h = reload(&path);
    h.record("aa", 1);
    h.record("bb", 2);
    let id = id_of(&h, "aa");
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch("DROP TABLE query_history").unwrap();
    assert!(h.delete(&id).is_err());
    assert!(h.clear().is_err());
    assert_eq!(queries(&h), ["bb", "aa"]);
}

#[test]
fn failed_writes_still_give_distinct_ids() {
    let (_d, path) = file();
    let h = reload(&path);
    let other = rusqlite::Connection::open(&path).unwrap();
    other.execute_batch("DROP TABLE query_history").unwrap();
    h.record("aa", 1);
    h.record("bb", 2);
    let (a, b) = (id_of(&h, "aa"), id_of(&h, "bb"));
    assert!(!a.is_empty() && !b.is_empty());
    assert_ne!(a, b);
}

#[test]
fn clones_share_one_history() {
    let a = mem();
    let b = a.clone();
    a.record("aa", 1);
    assert_eq!(b.nth_query(0), Some("aa".into()));
}
