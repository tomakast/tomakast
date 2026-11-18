//! SQLite store for usage history.

use std::path::Path;

use anyhow::Context;
use rusqlite::{Connection, params};

use crate::index::ItemId;
use crate::usage::{LocalNow, UsageRecord};

const SCHEMA_V1: &str = "
CREATE TABLE usage(
  id TEXT PRIMARY KEY, item_id TEXT NOT NULL, query TEXT NOT NULL,
  at_unix INTEGER NOT NULL, local_day INTEGER NOT NULL, local_minute INTEGER NOT NULL,
  modified_at INTEGER NOT NULL);
CREATE INDEX usage_at ON usage(at_unix);
";

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Store> {
        let conn = Connection::open(path)
            .with_context(|| format!("opening database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("enabling WAL")?;
        Store::init(conn)
    }

    pub fn open_in_memory() -> anyhow::Result<Store> {
        Store::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> anyhow::Result<Store> {
        let tx = conn.transaction()?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 0 {
            tx.execute_batch(SCHEMA_V1)?;
            tx.pragma_update(None, "user_version", 1)?;
        }
        tx.commit()?;
        Ok(Store { conn })
    }

    pub fn record_usage(&self, r: &UsageRecord) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO usage(id, item_id, query, at_unix, local_day, local_minute, modified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                r.id,
                r.item.as_str(),
                r.query,
                r.at.unix,
                r.at.local_day,
                r.at.local_minute,
                r.at.unix,
            ],
        )?;
        Ok(())
    }

    /// Records with `at_unix >= since_unix`, ascending by `at_unix`.
    /// A row with a malformed item id is skipped with a warning.
    pub fn load_usage(&self, since_unix: i64) -> anyhow::Result<Vec<UsageRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, item_id, query, at_unix, local_day, local_minute
             FROM usage WHERE at_unix >= ?1 ORDER BY at_unix ASC, rowid ASC",
        )?;
        let rows = stmt.query_map([since_unix], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i32>(4)?,
                row.get::<_, u16>(5)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, item_id, query, unix, local_day, local_minute) = row?;
            // One bad row must not cost the whole history.
            let Some(item) = ItemId::parse(&item_id) else {
                tracing::warn!(
                    id,
                    item_id,
                    "skipping usage record with a malformed item id"
                );
                continue;
            };
            out.push(UsageRecord {
                id,
                item,
                query,
                at: LocalNow {
                    unix,
                    local_day,
                    local_minute,
                },
            });
        }
        Ok(out)
    }
}
