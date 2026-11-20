//! Hash-bound approval: whether a plugin package may run, and the link policy that follows
//! from what loaded plugins may do.
//!
//! A user decision is stored per plugin id with the content hash (`sha256(plugin.toml ‖ 0x00 ‖
//! plugin.wasm)`) and the permission set it was made for. [`decide`] applies the rules to a
//! package; [`check`] is the one gate the plugin manager runs before anything of a plugin is
//! compiled or instantiated.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use tomakast_core::LinkPolicy;

use crate::db::{Grant, GrantDecision, PluginDb, WidgetGrant};
use crate::manifest::Permissions;
use crate::package::Package;

/// What may happen to a plugin package.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Approved: the plugin may load.
    Run,
    /// The user has to decide first.
    Ask,
    /// The user denied this exact content.
    Denied,
}

/// The approval rules, for `package` and the stored `grant` of its id:
///
/// - no grant: `Ask`;
/// - `allow` with the same hash: `Run`;
/// - `allow` with a different hash: `Run` when the package's permissions are a subset of the
///   approved ones, its name and author are the approved ones and every widget (id and title)
///   was approved (the caller records the new hash, see [`check`]); `Ask` when the set grew,
///   the name or author changed (Ruling AJ: the name labels every row of the plugin, so a new
///   one is not taken silently) or a widget is new or renamed;
/// - `deny` with the same hash: `Denied`; with a different hash: `Ask`.
///
/// A grant stored for another plugin id counts as no grant.
///
/// An in-place update whose permissions are no larger runs silently (Ruling AG). A plugin
/// folder that is removed and later re-created is reviewed again because the plugin manager
/// deletes the grant when the folder goes away ([`PluginDb::forget_plugin`]), not because of
/// these rules.
///
/// Denied content stays denied: there is no way to review the same content again in plan
/// 2b-2a (Ruling AF; follow-up R-1 in `docs/superpowers/plans/2026-11-18-plan-2b2a-followups.md`).
pub fn decide(grant: Option<&Grant>, package: &Package) -> Decision {
    let Some(grant) = grant.filter(|g| g.plugin_id == package.manifest.id) else {
        return Decision::Ask;
    };
    let same_content = grant.content_hash == package.content_hash;
    match grant.decision {
        GrantDecision::Allow
            if widgets_granted(grant, package)
                && (same_content
                    || (package
                        .manifest
                        .permissions
                        .is_subset_of(&grant.permissions)
                        && same_identity(grant, package))) =>
        {
            Decision::Run
        }
        GrantDecision::Allow => Decision::Ask,
        GrantDecision::Deny if same_content => Decision::Denied,
        GrantDecision::Deny => Decision::Ask,
    }
}

/// Whether every widget of `package` (id and title) is one the user approved. A removed
/// widget or a changed size asks nothing.
fn widgets_granted(grant: &Grant, package: &Package) -> bool {
    package.manifest.widgets.iter().all(|w| {
        grant
            .widgets
            .iter()
            .any(|g| g.id == w.id && g.title == w.title)
    })
}

/// Whether `package` has the name and author `grant` was decided for. A grant stored before
/// names were recorded has an empty name and never matches.
fn same_identity(grant: &Grant, package: &Package) -> bool {
    grant.name == package.manifest.name && grant.author == package.manifest.author
}

/// Why a plugin is up for review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewReason {
    /// Never approved: no grant, or the user denied an earlier content of it.
    New,
    /// Approved before, and this content asks for more permissions.
    MorePermissions,
    /// Approved before, and this content asks for no more permissions but has another name or
    /// author (Ruling AJ).
    Renamed,
    /// Approved before, and this content asks for no more permissions and keeps its name and
    /// author, but adds a widget or renames one.
    NewWidgets,
}

/// A plugin waiting for the user's decision: what the review prompt shows, and what an
/// approval binds to (this content hash and this permission set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewState {
    pub id: String,
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    /// The plugin folder.
    pub dir: PathBuf,
    pub permissions: Permissions,
    /// The widgets of the manifest: what an approval records.
    pub widgets: Vec<WidgetGrant>,
    pub content_hash: [u8; 32],
    pub reason: ReviewReason,
}

impl ReviewState {
    fn new(package: &Package, reason: ReviewReason) -> ReviewState {
        let m = &package.manifest;
        ReviewState {
            id: m.id.clone(),
            name: m.name.clone(),
            version: m.version.clone(),
            author: m.author.clone(),
            dir: package.dir.clone(),
            permissions: m.permissions.clone(),
            widgets: m
                .widgets
                .iter()
                .map(|w| WidgetGrant {
                    id: w.id.clone(),
                    title: w.title.clone(),
                })
                .collect(),
            content_hash: package.content_hash,
            reason,
        }
    }
}

/// The outcome of [`check`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    /// The plugin may load. Carries when the user approved it (the grant's `decided_at`).
    Run(i64),
    /// The plugin must not load; hand the review to the `plugins` module
    /// ([`crate::plugins_module::PluginsHandle::set_reviews`]).
    Review(Box<ReviewState>),
    /// The plugin must not load, and nobody is asked.
    Denied,
}

/// The gate before a plugin loads: reads the grant of `package`'s id from `db` and applies
/// [`decide`]. Nothing of a plugin may be compiled or instantiated unless this returns `Run`.
///
/// - An approved update with no new permissions records the new hash and `updated_at = now`;
///   the approved permission set, approved widgets and `decided_at` stay as the user decided
///   them.
/// - A grant row that cannot be read (malformed, or a database error) is a denial.
pub fn check(db: &PluginDb, package: &Package, now: i64) -> Gate {
    let id = &package.manifest.id;
    let grant = match db.grant(id) {
        Ok(grant) => grant,
        Err(e) => {
            tracing::error!(plugin = %id, "cannot read the plugin's approval, denying it: {e:#}");
            return Gate::Denied;
        }
    };
    match decide(grant.as_ref(), package) {
        Decision::Run => {
            // `decide` runs only with a grant.
            let decided_at = grant.as_ref().map_or(0, |g| g.decided_at);
            if let Some(grant) = grant.filter(|g| g.content_hash != package.content_hash) {
                let updated = Grant {
                    content_hash: package.content_hash,
                    updated_at: now,
                    ..grant
                };
                // Not recording it only means the same subset check runs next time.
                if let Err(e) = db.set_grant(&updated) {
                    tracing::warn!(plugin = %id, "cannot record the updated plugin's hash: {e:#}");
                }
            }
            Gate::Run(decided_at)
        }
        Decision::Ask => {
            let reason = match grant {
                Some(g) if g.plugin_id == *id && g.decision == GrantDecision::Allow => {
                    if !package.manifest.permissions.is_subset_of(&g.permissions) {
                        ReviewReason::MorePermissions
                    } else if !same_identity(&g, package) {
                        ReviewReason::Renamed
                    } else {
                        ReviewReason::NewWidgets
                    }
                }
                _ => ReviewReason::New,
            };
            Gate::Review(Box::new(ReviewState::new(package, reason)))
        }
        Decision::Denied => Gate::Denied,
    }
}

/// Whose links open: the native (built-in) providers the policy was built with, and loaded
/// plugins that hold the `open` permission. Every other provider, a plugin that is not loaded
/// included, is denied. It also names the loaded plugins, for the footer of their views
/// (Ruling AN).
///
/// Clones share one table: the plugin manager keeps it current with
/// [`set_plugin`](Self::set_plugin) on every load and reload and
/// [`remove_plugin`](Self::remove_plugin) on every unload, and the launcher asks it on every
/// click.
#[derive(Clone)]
pub struct PluginLinkPolicy {
    /// Ids of the built-in modules; fixed.
    native: Arc<HashSet<String>>,
    /// Loaded plugins that may open links.
    may_open: Arc<RwLock<HashSet<String>>>,
    /// The names of the loaded plugins, by id.
    names: Arc<RwLock<HashMap<String, String>>>,
}

impl PluginLinkPolicy {
    /// A policy that lets the built-in modules `native_ids` open links, and no plugin yet.
    pub fn new(native_ids: HashSet<String>) -> PluginLinkPolicy {
        PluginLinkPolicy {
            native: Arc::new(native_ids),
            may_open: Arc::default(),
            names: Arc::default(),
        }
    }

    /// Records the name and permissions of the loaded plugin `id`.
    pub fn set_plugin(&self, id: &str, name: &str, permissions: &Permissions) {
        if let Ok(mut names) = self.names.write() {
            names.insert(id.to_owned(), name.to_owned());
        }
        let Ok(mut may_open) = self.may_open.write() else {
            return;
        };
        if permissions.open {
            may_open.insert(id.to_owned());
        } else {
            may_open.remove(id);
        }
    }

    /// Forgets the unloaded plugin `id`: its links no longer open.
    pub fn remove_plugin(&self, id: &str) {
        if let Ok(mut may_open) = self.may_open.write() {
            may_open.remove(id);
        }
        if let Ok(mut names) = self.names.write() {
            names.remove(id);
        }
    }
}

impl LinkPolicy for PluginLinkPolicy {
    fn may_open_links(&self, provider: &str) -> bool {
        self.native.contains(provider)
            || self
                .may_open
                .read()
                .is_ok_and(|may_open| may_open.contains(provider))
    }

    /// The name of a plugin, loaded or unloaded since its view was pushed: a provider that
    /// is not built in is a plugin, and one this policy does not know is named by its id.
    fn plugin_name(&self, provider: &str) -> Option<String> {
        if self.native.contains(provider) {
            return None;
        }
        let name = self
            .names
            .read()
            .ok()
            .and_then(|names| names.get(provider).cloned());
        Some(name.unwrap_or_else(|| provider.to_owned()))
    }
}
