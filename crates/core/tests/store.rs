use tomakast_core::store::Store;
use tomakast_core::{ItemId, LocalNow, UsageRecord};

fn record(query: &str, unix: i64) -> UsageRecord {
    UsageRecord::new(
        ItemId::new("apps", "firefox"),
        query,
        LocalNow::from_unix_with_offset(unix, 3 * 3600),
    )
}

#[test]
fn roundtrip_usage_record() {
    let store = Store::open_in_memory().unwrap();
    let r = record("fire", 1_700_000_000);
    store.record_usage(&r).unwrap();
    assert_eq!(store.load_usage(0).unwrap(), vec![r]);
}

#[test]
fn load_usage_filters_by_since() {
    let store = Store::open_in_memory().unwrap();
    let old = record("a", 100);
    let new = record("b", 200);
    store.record_usage(&new).unwrap();
    store.record_usage(&old).unwrap();
    assert_eq!(store.load_usage(150).unwrap(), vec![new]);
    let all = store.load_usage(0).unwrap();
    assert_eq!(
        all.iter().map(|r| r.at.unix).collect::<Vec<_>>(),
        [100, 200]
    );
}

#[test]
fn file_store_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tomakast.db");
    let r = record("x", 500);
    {
        let store = Store::open(&path).unwrap();
        store.record_usage(&r).unwrap();
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.load_usage(0).unwrap(), vec![r]);
}

#[test]
fn reopening_does_not_rerun_migration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tomakast.db");
    drop(Store::open(&path).unwrap());
    drop(Store::open(&path).unwrap());
    let conn = rusqlite::Connection::open(&path).unwrap();
    let v: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(v, 1);
}

#[test]
fn load_usage_skips_rows_with_a_malformed_item_id() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tomakast.db");
    let store = Store::open(&path).unwrap();
    let before = record("a", 100);
    let after = record("b", 300);
    store.record_usage(&before).unwrap();
    store.record_usage(&after).unwrap();
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO usage(id, item_id, query, at_unix, local_day, local_minute, modified_at)
         VALUES ('bad', 'no-slash', 'q', 200, 0, 0, 200)",
        [],
    )
    .unwrap();
    assert_eq!(store.load_usage(0).unwrap(), vec![before, after]);
}
