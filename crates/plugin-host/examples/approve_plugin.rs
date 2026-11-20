//! Dev helper for acceptance runs: approves one plugin folder the way the review view's
//! "Allow" does. The folder is read as the manager reads it, the gate must answer `Review`,
//! and the same `Grant` the review module writes (the reviewed content hash and permissions,
//! decision `Allow`) goes through `PluginDb::set_grant`.
//!
//! Use it only with a throwaway `TOMAKAST_HOME` and the daemon stopped:
//!
//! ```text
//! cargo run --release -p tomakast-plugin-host --example approve_plugin -- \
//!     "$TOMAKAST_HOME/tomakast.db" "$TOMAKAST_HOME/plugins/kitchen-sink"
//! ```

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use tomakast_core::store::Store;
use tomakast_plugin_host::{Gate, Grant, GrantDecision, Package, PluginDb, check};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let (Some(db_file), Some(dir), None) = (args.next(), args.next(), args.next()) else {
        bail!("usage: approve_plugin <tomakast.db> <plugin folder>");
    };
    // The core store creates and migrates the database, as the daemon does on start.
    drop(Store::open(&db_file).context("opening the core store")?);
    let db = PluginDb::open(&db_file)?;
    let package = Package::read(&dir).context("reading the plugin folder")?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
    let review = match check(&db, &package, now) {
        Gate::Review(review) => review,
        Gate::Run(_) => {
            println!("{} is already approved", package.manifest.id);
            return Ok(());
        }
        Gate::Denied => bail!(
            "{} is denied; delete the folder to ask again",
            package.manifest.id
        ),
    };
    db.set_grant(&Grant {
        plugin_id: review.id.clone(),
        content_hash: review.content_hash,
        permissions: review.permissions.clone(),
        name: review.name.clone(),
        author: review.author.clone(),
        widgets: review.widgets.clone(),
        decision: GrantDecision::Allow,
        decided_at: now,
        updated_at: now,
    })?;
    let hash: String = review.content_hash[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    println!(
        "allowed {} ({}), content {hash}…",
        review.id, review.version
    );
    Ok(())
}
