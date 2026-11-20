//! Plugin storage: key-value data and approval grants, in the core database file.

use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use tomakast_core::store::SCHEMA_VERSION;

use crate::manifest::Permissions;

pub const MAX_KEY_BYTES: usize = 256;
pub const MAX_VALUE_BYTES: usize = 256 * 1024;
pub const MAX_PLUGIN_BYTES: u64 = 1024 * 1024;
/// Counted per stored row on top of key and value bytes, so many tiny entries cannot dodge the
/// quota.
pub const ROW_OVERHEAD_BYTES: u64 = 32;

#[derive(Debug, thiserror::Error)]
pub enum KvError {
    #[error("key is longer than {MAX_KEY_BYTES} bytes")]
    KeyTooLong,
    #[error("value is larger than {MAX_VALUE_BYTES} bytes")]
    ValueTooLarge,
    /// Ruling 7: the message starts with `quota: ` (a stable prefix the WIT `storage`
    /// docs promise plugins); no other storage error starts so.
    #[error("quota: storage quota of {MAX_PLUGIN_BYTES} bytes exceeded")]
    QuotaExceeded,
    #[error(transparent)]
    Db(#[from] rusqlite::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantDecision {
    Allow,
    Deny,
}

/// A widget the user approved: its id and the title the prompt showed. The size is not part
/// of it, so a resize asks nothing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WidgetGrant {
    pub id: String,
    pub title: String,
}

/// How `Grant.widgets` is stored: a TOML `[[widgets]]` array.
#[derive(Serialize, Deserialize)]
struct StoredWidgets {
    #[serde(default)]
    widgets: Vec<WidgetGrant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub plugin_id: String,
    pub content_hash: [u8; 32],
    pub permissions: Permissions,
    /// The plugin's name when the user decided. Empty for a grant stored before names were
    /// recorded: no manifest name is empty, so such a grant never matches (Ruling AJ).
    pub name: String,
    /// The plugin's author when the user decided.
    pub author: Option<String>,
    /// The widgets the user approved. Empty for a grant stored before widgets (NULL column).
    pub widgets: Vec<WidgetGrant>,
    pub decision: GrantDecision,
    pub decided_at: i64,
    pub updated_at: i64,
}

pub struct PluginDb {
    conn: Connection,
}

fn row_bytes(key_len: usize, value_len: usize) -> u64 {
    key_len as u64 + value_len as u64 + ROW_OVERHEAD_BYTES
}

/// The one place quota usage is computed (used by `kv_set` and `kv_usage`), optionally leaving
/// out the row for `except_key` (the one about to be replaced).
fn usage(conn: &Connection, plugin: &str, except_key: Option<&str>) -> Result<u64, KvError> {
    let n: i64 = conn.query_row(
        "SELECT COALESCE(SUM(LENGTH(CAST(key AS BLOB)) + LENGTH(value) + ?3), 0)
         FROM plugin_kv WHERE plugin_id = ?1 AND (?2 IS NULL OR key <> ?2)",
        params![plugin, except_key, ROW_OVERHEAD_BYTES as i64],
        |r| r.get(0),
    )?;
    Ok(n as u64)
}

fn decision_str(d: GrantDecision) -> &'static str {
    match d {
        GrantDecision::Allow => "allow",
        GrantDecision::Deny => "deny",
    }
}

impl PluginDb {
    /// Opens its own WAL connection to the database file the core `Store` initialised.
    pub fn open(path: &Path) -> anyhow::Result<PluginDb> {
        // No CREATE: the core `Store` owns creation and migration.
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .with_context(|| format!("opening database {}", path.display()))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("enabling WAL")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        anyhow::ensure!(
            version == SCHEMA_VERSION,
            "database schema is v{version}, plugins need v{SCHEMA_VERSION}; open the core store first"
        );
        Ok(PluginDb { conn })
    }

    pub fn kv_get(&self, plugin: &str, key: &str) -> Result<Option<Vec<u8>>, KvError> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
                params![plugin, key],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// Stores `value`, enforcing the per-item caps and the plugin's total quota (key and value
    /// bytes of all its entries; an overwrite replaces the old entry in the sum).
    pub fn kv_set(&self, plugin: &str, key: &str, value: &[u8], now: i64) -> Result<(), KvError> {
        if key.len() > MAX_KEY_BYTES {
            return Err(KvError::KeyTooLong);
        }
        if value.len() > MAX_VALUE_BYTES {
            return Err(KvError::ValueTooLarge);
        }
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let others = usage(&tx, plugin, Some(key))?;
        if others + row_bytes(key.len(), value.len()) > MAX_PLUGIN_BYTES {
            return Err(KvError::QuotaExceeded);
        }
        tx.execute(
            "INSERT INTO plugin_kv(plugin_id, key, value, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(plugin_id, key) DO UPDATE SET value = ?3, updated_at = ?4",
            params![plugin, key, value, now],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Returns whether an entry was removed.
    pub fn kv_remove(&self, plugin: &str, key: &str) -> Result<bool, KvError> {
        let n = self.conn.execute(
            "DELETE FROM plugin_kv WHERE plugin_id = ?1 AND key = ?2",
            params![plugin, key],
        )?;
        Ok(n > 0)
    }

    /// Bytes charged to `plugin`: key + value + a fixed overhead per row.
    pub fn kv_usage(&self, plugin: &str) -> Result<u64, KvError> {
        usage(&self.conn, plugin, None)
    }

    pub fn grant(&self, plugin: &str) -> anyhow::Result<Option<Grant>> {
        let row = self
            .conn
            .query_row(
                "SELECT content_hash, permissions, decision, decided_at, updated_at, name, author, widgets
                 FROM plugin_grants WHERE plugin_id = ?1",
                [plugin],
                |r| {
                    Ok((
                        (
                            r.get::<_, Vec<u8>>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ),
                        (r.get::<_, i64>(3)?, r.get::<_, i64>(4)?),
                        (
                            r.get::<_, Option<String>>(5)?,
                            r.get::<_, Option<String>>(6)?,
                        ),
                        r.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            (hash, permissions, decision),
            (decided_at, updated_at),
            (name, author),
            widgets,
        )) = row
        else {
            return Ok(None);
        };
        let content_hash: [u8; 32] = hash
            .try_into()
            .map_err(|_| anyhow::anyhow!("stored grant for {plugin:?} has a malformed hash"))?;
        let decision = match decision.as_str() {
            "allow" => GrantDecision::Allow,
            "deny" => GrantDecision::Deny,
            other => anyhow::bail!("stored grant for {plugin:?} has decision {other:?}"),
        };
        let permissions = toml::from_str(&permissions)
            .with_context(|| format!("stored grant for {plugin:?} has malformed permissions"))?;
        let widgets = match widgets {
            None => Vec::new(),
            Some(text) => {
                toml::from_str::<StoredWidgets>(&text)
                    .with_context(|| format!("stored grant for {plugin:?} has malformed widgets"))?
                    .widgets
            }
        };
        Ok(Some(Grant {
            plugin_id: plugin.to_owned(),
            content_hash,
            permissions,
            name: name.unwrap_or_default(),
            author,
            widgets,
            decision,
            decided_at,
            updated_at,
        }))
    }

    /// Uninstalls `plugin`: deletes its grant and its key-value data in one transaction;
    /// returns whether there was a grant. The plugin manager calls it when a plugin folder is
    /// removed while it runs, so a folder re-created later is reviewed again even if its
    /// permissions are no larger (Ruling AG), and a later plugin with the same id does not
    /// inherit the data (Ruling AR).
    pub fn forget_plugin(&self, plugin: &str) -> anyhow::Result<bool> {
        let tx = Transaction::new_unchecked(&self.conn, TransactionBehavior::Immediate)?;
        let grants = tx.execute("DELETE FROM plugin_grants WHERE plugin_id = ?1", [plugin])?;
        tx.execute("DELETE FROM plugin_kv WHERE plugin_id = ?1", [plugin])?;
        tx.commit()?;
        Ok(grants > 0)
    }

    pub fn set_grant(&self, g: &Grant) -> anyhow::Result<()> {
        let permissions = toml::to_string(&g.permissions)?;
        let widgets = toml::to_string(&StoredWidgets {
            widgets: g.widgets.clone(),
        })?;
        self.conn.execute(
            "INSERT OR REPLACE INTO plugin_grants
               (plugin_id, content_hash, permissions, decision, decided_at, updated_at, name, author, widgets)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                g.plugin_id,
                g.content_hash.as_slice(),
                permissions,
                decision_str(g.decision),
                g.decided_at,
                g.updated_at,
                g.name,
                g.author,
                widgets
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::DomainPattern;
    use tomakast_core::store::Store;

    /// A database file initialised by the core store (current schema), plus the plugin handle.
    fn open() -> (tempfile::TempDir, PluginDb) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tomakast.db");
        let _store = Store::open(&path).unwrap();
        let db = PluginDb::open(&path).unwrap();
        (dir, db)
    }

    #[test]
    fn kv_roundtrip_and_isolation() {
        let (_d, db) = open();
        assert_eq!(db.kv_get("a", "k").unwrap(), None);
        db.kv_set("a", "k", b"v1", 1).unwrap();
        db.kv_set("b", "k", b"other", 1).unwrap();
        assert_eq!(db.kv_get("a", "k").unwrap().as_deref(), Some(&b"v1"[..]));
        db.kv_set("a", "k", b"v2", 2).unwrap();
        assert_eq!(db.kv_get("a", "k").unwrap().as_deref(), Some(&b"v2"[..]));
        assert_eq!(db.kv_get("b", "k").unwrap().as_deref(), Some(&b"other"[..]));
        assert!(db.kv_remove("a", "k").unwrap());
        assert!(!db.kv_remove("a", "k").unwrap());
        assert_eq!(db.kv_get("a", "k").unwrap(), None);
        assert_eq!(db.kv_get("b", "k").unwrap().as_deref(), Some(&b"other"[..]));
    }

    #[test]
    fn kv_quota_accounting() {
        const O: u64 = ROW_OVERHEAD_BYTES;
        let (_d, db) = open();
        assert_eq!(db.kv_usage("p").unwrap(), 0);
        db.kv_set("p", "key", b"12345", 1).unwrap();
        assert_eq!(db.kv_usage("p").unwrap(), 3 + 5 + O);
        // Overwrite is accounted by replacement, not addition.
        db.kv_set("p", "key", b"1", 2).unwrap();
        assert_eq!(db.kv_usage("p").unwrap(), 3 + 1 + O);
        // Other plugins do not count.
        db.kv_set("q", "key", &[0u8; 100], 1).unwrap();
        assert_eq!(db.kv_usage("p").unwrap(), 3 + 1 + O);
        db.kv_remove("p", "key").unwrap();
        assert_eq!(db.kv_usage("p").unwrap(), 0);

        // Per-item caps.
        assert!(matches!(
            db.kv_set("p", &"k".repeat(MAX_KEY_BYTES + 1), b"", 1),
            Err(KvError::KeyTooLong)
        ));
        assert!(matches!(
            db.kv_set("p", "k", &vec![0u8; MAX_VALUE_BYTES + 1], 1),
            Err(KvError::ValueTooLarge)
        ));
        // Total cap.
        let big = vec![0u8; MAX_VALUE_BYTES];
        for i in 0..3 {
            db.kv_set("p", &format!("k{i}"), &big, 1).unwrap();
        }
        let used = db.kv_usage("p").unwrap();
        assert_eq!(used, 3 * (MAX_VALUE_BYTES as u64 + 2 + O));
        assert!(matches!(
            db.kv_set("p", "k3", &big, 1),
            Err(KvError::QuotaExceeded)
        ));
        assert_eq!(db.kv_usage("p").unwrap(), used);
        // Overwriting in place still works at the edge.
        db.kv_set("p", "k0", &big, 2).unwrap();
        assert_eq!(db.kv_usage("p").unwrap(), used);
        // Exactly the limit is allowed; one byte more is not.
        let rest = (MAX_PLUGIN_BYTES - used - 2 - O) as usize;
        assert!(matches!(
            db.kv_set("p", "k3", &vec![0u8; rest + 1], 1),
            Err(KvError::QuotaExceeded)
        ));
        db.kv_set("p", "k3", &vec![0u8; rest], 1).unwrap();
        assert_eq!(db.kv_usage("p").unwrap(), MAX_PLUGIN_BYTES);
        assert!(matches!(
            db.kv_set("p", "x", b"", 1),
            Err(KvError::QuotaExceeded)
        ));
    }

    #[test]
    fn open_requires_an_existing_current_database() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("none.db");
        assert!(PluginDb::open(&missing).is_err());
        assert!(!missing.exists(), "open must not create the file");
        // An empty (v0) file created by someone else is refused.
        let empty = dir.path().join("empty.db");
        drop(Connection::open(&empty).unwrap());
        assert!(PluginDb::open(&empty).is_err());
    }

    #[test]
    fn grant_roundtrip_keeps_widgets() {
        let (_d, db) = open();
        assert_eq!(db.grant("p").unwrap(), None);
        let mut g = Grant {
            plugin_id: "p".into(),
            content_hash: [7; 32],
            permissions: Permissions {
                storage: true,
                http: vec![DomainPattern::parse("*.example.org").unwrap()],
                ..Permissions::default()
            },
            name: "P".into(),
            author: Some("Someone".into()),
            widgets: vec![
                WidgetGrant {
                    id: "clock".into(),
                    title: "Clock".into(),
                },
                WidgetGrant {
                    id: "weather".into(),
                    title: "Weather \"now\"".into(),
                },
            ],
            decision: GrantDecision::Allow,
            decided_at: 10,
            updated_at: 10,
        };
        db.set_grant(&g).unwrap();
        assert_eq!(db.grant("p").unwrap(), Some(g.clone()));
        // Replacing keeps one row per plugin.
        g.content_hash = [8; 32];
        g.decision = GrantDecision::Deny;
        g.updated_at = 20;
        g.author = None;
        g.widgets.clear();
        db.set_grant(&g).unwrap();
        assert_eq!(db.grant("p").unwrap(), Some(g));
        assert_eq!(db.grant("other").unwrap(), None);
    }

    #[test]
    fn a_grant_without_a_name_reads_as_an_empty_name() {
        // A row written before names were recorded (schema v2): never matches a manifest.
        let (_d, db) = open();
        db.conn
            .execute(
                "INSERT INTO plugin_grants(plugin_id, content_hash, permissions, decision, decided_at, updated_at)
                 VALUES ('p', ?1, '', 'allow', 1, 1)",
                [[1u8; 32].as_slice()],
            )
            .unwrap();
        let g = db.grant("p").unwrap().unwrap();
        assert_eq!((g.name.as_str(), g.author), ("", None));
    }

    #[test]
    fn a_grant_with_a_null_widgets_column_reads_as_no_widgets() {
        let (_d, db) = open();
        db.conn
            .execute(
                "INSERT INTO plugin_grants(plugin_id, content_hash, permissions, decision, decided_at, updated_at, name)
                 VALUES ('p', ?1, '', 'allow', 1, 1, 'P')",
                [[1u8; 32].as_slice()],
            )
            .unwrap();
        assert_eq!(db.grant("p").unwrap().unwrap().widgets, vec![]);
    }

    #[test]
    fn forget_plugin_deletes_only_that_plugin() {
        let (_d, db) = open();
        let g = |id: &str| Grant {
            plugin_id: id.into(),
            content_hash: [1; 32],
            permissions: Permissions::default(),
            name: "P".into(),
            author: None,
            widgets: Vec::new(),
            decision: GrantDecision::Allow,
            decided_at: 1,
            updated_at: 1,
        };
        db.set_grant(&g("p")).unwrap();
        db.set_grant(&g("q")).unwrap();
        db.kv_set("p", "k", b"v", 1).unwrap();
        db.kv_set("q", "k", b"v", 1).unwrap();
        assert!(db.forget_plugin("p").unwrap());
        assert_eq!(db.grant("p").unwrap(), None);
        assert_eq!(db.kv_usage("p").unwrap(), 0);
        assert_eq!(db.grant("q").unwrap(), Some(g("q")));
        assert_eq!(db.kv_get("q", "k").unwrap().as_deref(), Some(&b"v"[..]));
        assert!(!db.forget_plugin("p").unwrap(), "nothing left to remove");
    }

    #[test]
    fn second_connection_sees_writes_while_store_is_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tomakast.db");
        let store = Store::open(&path).unwrap();
        let a = PluginDb::open(&path).unwrap();
        a.kv_set("p", "k", b"v", 1).unwrap();
        // Writes through the core store's connection do not block the plugin connection.
        store
            .record_usage(&tomakast_core::usage::UsageRecord {
                id: "u".into(),
                item: tomakast_core::index::ItemId::parse("apps/x").unwrap(),
                query: "q".into(),
                at: tomakast_core::usage::LocalNow {
                    unix: 1,
                    local_day: 1,
                    local_minute: 1,
                },
            })
            .unwrap();
        let b = PluginDb::open(&path).unwrap();
        assert_eq!(b.kv_get("p", "k").unwrap().as_deref(), Some(&b"v"[..]));
    }
}
