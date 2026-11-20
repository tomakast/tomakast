//! SQLite store for usage and query history.

use std::path::Path;

use anyhow::Context;
use rusqlite::{Connection, params};

use crate::history::HistoryEntry;
use crate::index::ItemId;
use crate::usage::{LocalNow, UsageRecord};

const SCHEMA_V1: &str = "
CREATE TABLE usage(
  id TEXT PRIMARY KEY, item_id TEXT NOT NULL, query TEXT NOT NULL,
  at_unix INTEGER NOT NULL, local_day INTEGER NOT NULL, local_minute INTEGER NOT NULL,
  modified_at INTEGER NOT NULL);
CREATE INDEX usage_at ON usage(at_unix);
";

/// Plugin storage (key-value data and approval grants). The plugin host reads and writes these
/// tables through its own connection; the core only owns the schema.
const SCHEMA_V2: &str = "
CREATE TABLE plugin_kv(
  plugin_id TEXT NOT NULL, key TEXT NOT NULL, value BLOB NOT NULL, updated_at INTEGER NOT NULL,
  PRIMARY KEY(plugin_id, key));
CREATE TABLE plugin_grants(
  plugin_id TEXT PRIMARY KEY, content_hash BLOB NOT NULL, permissions TEXT NOT NULL,
  decision TEXT NOT NULL CHECK(decision IN ('allow','deny')),
  decided_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
";

/// The name and author a plugin had when the user decided (Ruling AJ: an update that changes
/// either asks again). NULL in a row written before v3, which the plugin host treats as
/// changed.
const SCHEMA_V3: &str = "
ALTER TABLE plugin_grants ADD COLUMN name TEXT;
ALTER TABLE plugin_grants ADD COLUMN author TEXT;
";

/// The launcher's query history. Starts empty: nothing is seeded from `usage`.
const SCHEMA_V4: &str = "
CREATE TABLE query_history(
  id TEXT PRIMARY KEY, query TEXT NOT NULL UNIQUE,
  at_unix INTEGER NOT NULL, modified_at INTEGER NOT NULL);
CREATE INDEX query_history_at ON query_history(at_unix);
";

/// The widgets (id and title) a plugin had when the user decided, as TOML. NULL in a row
/// written before v5: no widgets were approved.
const SCHEMA_V5: &str = "
ALTER TABLE plugin_grants ADD COLUMN widgets TEXT;
";

/// The schema version a database has after `Store::open`.
pub const SCHEMA_VERSION: i64 = 5;

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
        if version > SCHEMA_VERSION {
            anyhow::bail!(
                "database schema v{version} is newer than this build (v{SCHEMA_VERSION})"
            );
        }
        if version < 1 {
            tx.execute_batch(SCHEMA_V1)?;
        }
        if version < 2 {
            tx.execute_batch(SCHEMA_V2)?;
        }
        if version < 3 {
            tx.execute_batch(SCHEMA_V3)?;
        }
        if version < 4 {
            tx.execute_batch(SCHEMA_V4)?;
        }
        if version < 5 {
            tx.execute_batch(SCHEMA_V5)?;
        }
        if version < SCHEMA_VERSION {
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
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

    /// Records `query` as used at `at_unix` under `id` and returns the row's id, which stays the
    /// existing one on a reuse. The REPLACE is deliberate: the row gets a new rowid, the same-second tiebreak.
    /// The length rule is the caller's.
    pub fn upsert_query(&self, id: &str, query: &str, at_unix: i64) -> anyhow::Result<String> {
        Ok(self.conn.query_row(
            "INSERT OR REPLACE INTO query_history(id, query, at_unix, modified_at)
             VALUES (COALESCE((SELECT id FROM query_history WHERE query = ?2), ?1), ?2, ?3, ?3)
             RETURNING id",
            params![id, query, at_unix],
            |r| r.get(0),
        )?)
    }

    /// The newest `limit` entries, newest first.
    pub fn load_query_history(&self, limit: usize) -> anyhow::Result<Vec<HistoryEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, query, at_unix FROM query_history
             ORDER BY at_unix DESC, rowid DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(HistoryEntry {
                id: r.get(0)?,
                query: r.get(1)?,
                at_unix: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether a row with this id existed.
    pub fn delete_query(&self, id: &str) -> anyhow::Result<bool> {
        Ok(self
            .conn
            .execute("DELETE FROM query_history WHERE id = ?1", [id])?
            > 0)
    }

    /// The number of rows deleted.
    pub fn clear_query_history(&self) -> anyhow::Result<usize> {
        Ok(self.conn.execute("DELETE FROM query_history", [])?)
    }

    /// Deletes all but the newest `keep` rows; returns the number deleted.
    pub fn prune_query_history(&self, keep: usize) -> anyhow::Result<usize> {
        Ok(self.conn.execute(
            "DELETE FROM query_history WHERE id NOT IN
             (SELECT id FROM query_history ORDER BY at_unix DESC, rowid DESC LIMIT ?1)",
            [keep as i64],
        )?)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        stmt.query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn store_migrates_v1_to_v5_keeping_usage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO usage(id, item_id, query, at_unix, local_day, local_minute, modified_at)
                 VALUES ('u1', 'apps/firefox', 'fi', 100, 5, 60, 100)",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let rows = store.load_usage(0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "u1");
        assert_eq!(rows[0].item.as_str(), "apps/firefox");
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 5);
        assert!(store.load_query_history(1000).unwrap().is_empty());
        let names = tables(&store.conn);
        assert!(names.contains(&"plugin_kv".to_owned()), "{names:?}");
        assert!(names.contains(&"plugin_grants".to_owned()), "{names:?}");
        // A second open is a no-op.
        drop(store);
        let again = Store::open(&path).unwrap();
        assert_eq!(again.load_usage(0).unwrap().len(), 1);
    }

    #[test]
    fn store_migrates_v2_to_v3_keeping_grants() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.execute_batch(SCHEMA_V2).unwrap();
            conn.pragma_update(None, "user_version", 2).unwrap();
            conn.execute(
                "INSERT INTO plugin_grants(plugin_id, content_hash, permissions, decision, decided_at, updated_at)
                 VALUES ('p', x'00', '', 'allow', 1, 1)",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let (name, author): (Option<String>, Option<String>) = store
            .conn
            .query_row(
                "SELECT name, author FROM plugin_grants WHERE plugin_id = 'p'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((name, author), (None, None), "an old grant knows no name");
    }

    #[test]
    fn fresh_store_is_v5() {
        let store = Store::open_in_memory().unwrap();
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 5);
        assert!(tables(&store.conn).contains(&"plugin_grants".to_owned()));
        assert!(tables(&store.conn).contains(&"query_history".to_owned()));
    }

    #[test]
    fn store_migrates_v3_to_v4_with_an_empty_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA_V1).unwrap();
            conn.execute_batch(SCHEMA_V2).unwrap();
            conn.execute_batch(SCHEMA_V3).unwrap();
            conn.pragma_update(None, "user_version", 3).unwrap();
            for (id, query, at) in [("u1", "Fi", 100), ("u2", "files", 200)] {
                conn.execute(
                    "INSERT INTO usage(id, item_id, query, at_unix, local_day, local_minute, modified_at)
                     VALUES (?1, ?2, ?3, ?4, 5, 60, ?4)",
                    params![id, if id == "u1" { "apps/firefox" } else { "apps/files" }, query, at],
                )
                .unwrap();
            }
        }
        let store = Store::open(&path).unwrap();
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 5);
        assert_eq!(store.load_usage(0).unwrap().len(), 2);
        assert!(store.load_query_history(1000).unwrap().is_empty());
    }

    #[test]
    fn migration_adds_the_widgets_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            let conn = Connection::open(&path).unwrap();
            for schema in [SCHEMA_V1, SCHEMA_V2, SCHEMA_V3, SCHEMA_V4] {
                conn.execute_batch(schema).unwrap();
            }
            conn.pragma_update(None, "user_version", 4).unwrap();
            conn.execute(
                "INSERT INTO plugin_grants(plugin_id, content_hash, permissions, decision, decided_at, updated_at, name)
                 VALUES ('p', x'00', 'storage = true', 'allow', 7, 8, 'P')",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let version: i64 = store
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 5);
        let (permissions, name, decided_at, widgets): (String, String, i64, Option<String>) = store
            .conn
            .query_row(
                "SELECT permissions, name, decided_at, widgets FROM plugin_grants WHERE plugin_id = 'p'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            (permissions.as_str(), name.as_str(), decided_at, widgets),
            ("storage = true", "P", 7, None),
            "the grant is intact and approved no widgets"
        );
    }

    #[test]
    fn grants_reject_unknown_decisions() {
        let store = Store::open_in_memory().unwrap();
        let insert = |decision: &str| {
            store.conn.execute(
                "INSERT OR REPLACE INTO plugin_grants(plugin_id, content_hash, permissions, decision, decided_at, updated_at)
                 VALUES ('p', x'00', '', ?1, 0, 0)",
                [decision],
            )
        };
        assert!(insert("allow").is_ok());
        assert!(insert("deny").is_ok());
        assert!(insert("maybe").is_err());
    }
}
