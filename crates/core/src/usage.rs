//! Usage model: records of past launches and the ranking weights built on them.

use std::collections::HashMap;

use chrono::{Local, Offset, Utc};

use crate::index::ItemId;

const SECS_PER_DAY: i64 = 86_400;
const MINUTES_PER_DAY: u16 = 1440;

/// A point in time with its local-time-zone day and minute-of-day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalNow {
    pub unix: i64,
    /// Days since 1970-01-01 in the local time zone.
    pub local_day: i32,
    /// Minute of the local day, `0..1440`.
    pub local_minute: u16,
}

impl LocalNow {
    pub fn now() -> LocalNow {
        let offset = Local::now().offset().fix().local_minus_utc();
        LocalNow::from_unix_with_offset(Utc::now().timestamp(), offset)
    }

    pub fn from_unix_with_offset(unix: i64, offset_secs: i32) -> LocalNow {
        let local = unix + i64::from(offset_secs);
        LocalNow {
            unix,
            local_day: local.div_euclid(SECS_PER_DAY) as i32,
            local_minute: (local.rem_euclid(SECS_PER_DAY) / 60) as u16,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct UsageRecord {
    pub id: String,
    pub item: ItemId,
    /// The query typed when the item was launched, lowercase.
    pub query: String,
    pub at: LocalNow,
}

impl UsageRecord {
    pub fn new(item: ItemId, query: &str, at: LocalNow) -> UsageRecord {
        UsageRecord {
            id: uuid::Uuid::new_v4().to_string(),
            item,
            query: query.to_lowercase(),
            at,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UsageStats {
    pub frecency: f64,
    pub today: u32,
    pub time_of_day: u32,
    pub query_hits: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RankWeights {
    pub half_life_days: f64,
    pub time_window_minutes: u16,
    pub w_frecency: f64,
    pub w_today: f64,
    pub w_time_of_day: f64,
    pub w_query: f64,
    pub usage_scale: f64,
}

impl Default for RankWeights {
    fn default() -> Self {
        RankWeights {
            half_life_days: 14.0,
            time_window_minutes: 90,
            w_frecency: 1.0,
            w_today: 2.0,
            w_time_of_day: 1.5,
            w_query: 4.0,
            usage_scale: 30.0,
        }
    }
}

pub fn usage_value(s: &UsageStats, w: &RankWeights) -> f64 {
    w.w_frecency * s.frecency
        + w.w_today * f64::from(s.today)
        + w.w_time_of_day * f64::from(s.time_of_day)
        + w.w_query * f64::from(s.query_hits)
}

pub fn combined_score(match_score: u32, usage: f64, w: &RankWeights) -> f64 {
    f64::from(match_score) + w.usage_scale * usage.ln_1p()
}

#[derive(Default)]
pub struct UsageModel {
    records: Vec<UsageRecord>,
}

impl UsageModel {
    pub fn new(records: Vec<UsageRecord>) -> UsageModel {
        UsageModel { records }
    }

    pub fn push(&mut self, r: UsageRecord) {
        self.records.push(r);
    }

    /// Aggregates the signals per item in a single pass over the records.
    pub fn stats(
        &self,
        query: &str,
        now: LocalNow,
        w: &RankWeights,
    ) -> HashMap<ItemId, UsageStats> {
        let q = query.to_lowercase();
        let half_life_secs = w.half_life_days * SECS_PER_DAY as f64;
        let mut out: HashMap<ItemId, UsageStats> = HashMap::new();
        for r in &self.records {
            let s = out.entry(r.item.clone()).or_default();
            s.frecency += 0.5_f64.powf((now.unix - r.at.unix) as f64 / half_life_secs);
            if r.at.local_day == now.local_day {
                s.today += 1;
            }
            if minute_distance(r.at.local_minute, now.local_minute) <= w.time_window_minutes {
                s.time_of_day += 1;
            }
            if !q.is_empty() && r.query.starts_with(&q) {
                s.query_hits += 1;
            }
        }
        out
    }
}

fn minute_distance(a: u16, b: u16) -> u16 {
    let d = a.abs_diff(b);
    d.min(MINUTES_PER_DAY - d)
}
