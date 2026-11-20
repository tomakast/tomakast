//! `storage`: the plugin's own key-value store in the core database, under the `storage`
//! permission, the quota of [`PluginDb::kv_set`](crate::db::PluginDb::kv_set), and at most
//! [`MAX_STORAGE_WRITES`] writes per [`STORAGE_WRITE_WINDOW`] (Ruling AO): each write is a
//! commit to the database file the core and the review module write to as well.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::HostState;
use crate::bindings::tomakast::plugin::storage::Host;
use crate::db::KvError;

/// `set` and `remove` calls a plugin may make per [`STORAGE_WRITE_WINDOW`].
pub const MAX_STORAGE_WRITES: usize = 20;
pub const STORAGE_WRITE_WINDOW: Duration = Duration::from_secs(1);

const DENIED: &str = "the storage permission was not granted";
const TOO_MANY: &str = "more than 20 storage writes per second";

impl HostState {
    fn storage_allowed(&self) -> Result<(), String> {
        if self.permissions.storage {
            Ok(())
        } else {
            Err(DENIED.into())
        }
    }

    /// Counts a write against the rate; every `set` and `remove` with the permission counts,
    /// a refused one included (a `set` takes the write lock before its quota check).
    fn storage_write_allowed(&mut self) -> Result<(), String> {
        self.storage_allowed()?;
        if self
            .storage_writes
            .admit(Instant::now(), MAX_STORAGE_WRITES, STORAGE_WRITE_WINDOW)
        {
            Ok(())
        } else {
            Err(TOO_MANY.into())
        }
    }
}

fn message(err: KvError) -> String {
    match err {
        KvError::Db(e) => {
            tracing::warn!(target: "plugin", "plugin storage failed: {e}");
            "storage is not available".into()
        }
        other => other.to_string(),
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl Host for HostState {
    fn get(&mut self, key: String) -> Result<Option<String>, String> {
        self.record_import("storage.get");
        self.storage_allowed()?;
        let value = self.db.kv_get(&self.plugin, &key).map_err(message)?;
        // Values are written from WIT strings, so they are UTF-8.
        value
            .map(|bytes| String::from_utf8(bytes).map_err(|_| "stored value is not UTF-8".into()))
            .transpose()
    }

    fn set(&mut self, key: String, value: String) -> Result<(), String> {
        self.record_import("storage.set");
        self.storage_write_allowed()?;
        self.db
            .kv_set(&self.plugin, &key, value.as_bytes(), unix_now())
            .map_err(message)
    }

    fn remove(&mut self, key: String) -> Result<(), String> {
        self.record_import("storage.remove");
        self.storage_write_allowed()?;
        self.db.kv_remove(&self.plugin, &key).map_err(message)?;
        Ok(())
    }
}
