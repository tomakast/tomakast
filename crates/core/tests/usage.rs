use tomakast_core::{ItemId, LocalNow, RankWeights, UsageModel, UsageRecord, combined_score};

fn item() -> ItemId {
    ItemId::new("apps", "firefox")
}

fn now() -> LocalNow {
    LocalNow::from_unix_with_offset(1_700_000_000, 0)
}

fn record(query: &str, at: LocalNow) -> UsageRecord {
    UsageRecord::new(item(), query, at)
}

#[test]
fn frecency_halves_after_half_life() {
    let w = RankWeights::default();
    let now = now();
    let at = LocalNow::from_unix_with_offset(now.unix - 14 * 86400, 0);
    let model = UsageModel::new(vec![record("", at)]);
    let stats = model.stats("", now, &w);
    assert!((stats[&item()].frecency - 0.5).abs() < 1e-9);
}

#[test]
fn today_counts_only_same_local_day() {
    let w = RankWeights::default();
    let now = now();
    let same_day = LocalNow {
        unix: now.unix - 60,
        local_day: now.local_day,
        local_minute: 10,
    };
    let prev_day = LocalNow {
        unix: now.unix - 86400,
        local_day: now.local_day - 1,
        local_minute: 10,
    };
    let model = UsageModel::new(vec![record("", same_day), record("", prev_day)]);
    assert_eq!(model.stats("", now, &w)[&item()].today, 1);
}

#[test]
fn time_of_day_window_wraps_midnight() {
    let w = RankWeights::default();
    let now = LocalNow {
        unix: 1_700_000_000,
        local_day: 100,
        local_minute: 5,
    };
    let near = LocalNow {
        unix: 1_600_000_000,
        local_day: 50,
        local_minute: 1430,
    };
    let far = LocalNow {
        unix: 1_600_000_000,
        local_day: 50,
        local_minute: 200,
    };
    let model = UsageModel::new(vec![record("", near)]);
    assert_eq!(model.stats("", now, &w)[&item()].time_of_day, 1);
    let model = UsageModel::new(vec![record("", far)]);
    assert_eq!(model.stats("", now, &w)[&item()].time_of_day, 0);
}

#[test]
fn query_hits_use_prefix_of_recorded_query() {
    let w = RankWeights::default();
    let now = now();
    let model = UsageModel::new(vec![record("FIR", now)]);
    assert_eq!(model.stats("fi", now, &w)[&item()].query_hits, 1);
    assert_eq!(model.stats("fo", now, &w)[&item()].query_hits, 0);
    assert_eq!(model.stats("", now, &w)[&item()].query_hits, 0);
}

#[test]
fn zero_usage_keeps_match_score() {
    assert_eq!(combined_score(120, 0.0, &RankWeights::default()), 120.0);
}

#[test]
fn from_unix_with_offset_computes_local_minute_and_day() {
    let a = LocalNow::from_unix_with_offset(0, 3 * 3600);
    assert_eq!((a.local_minute, a.local_day), (180, 0));
    let b = LocalNow::from_unix_with_offset(0, -3600);
    assert_eq!((b.local_minute, b.local_day), (1380, -1));
}
