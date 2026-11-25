//! The built-in `plugins` module: the review prompt for plugins waiting for approval.
//!
//! The prompt is never shown unprompted. A pending review is only an item in search,
//! `plugins/review-<id>`, which leads the empty query's Suggestions (the provider is pinned,
//! see [`PluginsModule::install`]). Enter on it pushes a detail view whose text the host writes
//! from the manifest.
//!
//! The view's primary (Enter) action is "Not now", which only closes it; "Allow" and "Deny"
//! are in the action panel (Ruling AE), so the keys that open and launch the top suggestion
//! cannot approve a plugin by habit. "Allow" chosen less than [`MIN_READ_TIME`] after the view
//! was shown is ignored.
//!
//! A decision is written to `plugin_grants` for the content hash and permissions that view
//! showed, and only while that content is still pending; then [`PluginEvents::Approved`] or
//! [`PluginEvents::Denied`] goes to the plugin manager.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, bail};
use tomakast_core::{
    Action, Core, DetailView, Effect, Item, ItemId, ItemKind, Module, ModuleContext, Toast,
    ToastKind, View, ViewEvent, ViewId,
};

use crate::approval::{ReviewReason, ReviewState};
use crate::db::{Grant, GrantDecision, PluginDb};
use crate::plugin::PluginEvents;

/// The module's provider id.
pub const PROVIDER: &str = "plugins";

/// How long a review view must have been shown before "Allow" counts.
pub const MIN_READ_TIME: Duration = Duration::from_secs(1);

/// The module's monotonic clock; injectable for tests.
pub type Clock = Arc<dyn Fn() -> Instant + Send + Sync>;

const REVIEW_ACTION: &str = "review";
const NOT_NOW: &str = "not-now";
const ALLOW: &str = "allow";
const DENY: &str = "deny";
/// Bytes of the content hash the prompt shows, in hex.
const HASH_PREFIX_BYTES: usize = 8;

struct Shared {
    /// Set once the module's worker has started it.
    ctx: Option<ModuleContext>,
    reviews: Vec<ReviewState>,
}

impl Shared {
    fn publish(&self) {
        if let Some(ctx) = &self.ctx {
            ctx.replace_items(self.reviews.iter().map(review_item).collect());
        }
    }

    fn is_pending(&self, review: &ReviewState) -> bool {
        self.reviews
            .iter()
            .any(|r| r.id == review.id && r.content_hash == review.content_hash)
    }
}

fn lock(shared: &Mutex<Shared>) -> MutexGuard<'_, Shared> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The plugin manager's handle on the `plugins` module; cheap to clone, `Send`.
#[derive(Clone)]
pub struct PluginsHandle {
    shared: Arc<Mutex<Shared>>,
}

impl PluginsHandle {
    /// Replaces the pending reviews with `reviews`: every plugin the gate answered
    /// [`Gate::Review`](crate::approval::Gate::Review) for, one per plugin id. It only updates
    /// the module's search items, before or after the module has started; it never shows a
    /// view or a toast. An open review whose content is no longer pending can no longer be
    /// decided.
    pub fn set_reviews(&self, reviews: Vec<ReviewState>) {
        let mut shared = lock(&self.shared);
        shared.reviews = reviews;
        shared.publish();
    }
}

/// A review view on screen: what it showed, and since when.
struct OpenReview {
    review: ReviewState,
    shown_at: Instant,
}

pub struct PluginsModule {
    shared: Arc<Mutex<Shared>>,
    db: PluginDb,
    events: mpsc::Sender<PluginEvents>,
    clock: Clock,
    /// The review each open prompt showed, by view id: a decision binds to it, not to the
    /// current review of that plugin.
    open: HashMap<ViewId, OpenReview>,
}

impl PluginsModule {
    /// Registers the module in `core` and pins its provider, so a pending review leads the
    /// empty query's Suggestions. Decisions are written through `db` and reported on `events`.
    pub fn install(
        core: &mut Core,
        db: PluginDb,
        events: mpsc::Sender<PluginEvents>,
    ) -> PluginsHandle {
        PluginsModule::install_with_clock(core, db, events, Arc::new(Instant::now))
    }

    /// [`install`](Self::install) with the clock that times [`MIN_READ_TIME`].
    pub fn install_with_clock(
        core: &mut Core,
        db: PluginDb,
        events: mpsc::Sender<PluginEvents>,
        clock: Clock,
    ) -> PluginsHandle {
        let shared = Arc::new(Mutex::new(Shared {
            ctx: None,
            reviews: Vec::new(),
        }));
        let module = PluginsModule {
            shared: shared.clone(),
            db,
            events,
            clock,
            open: HashMap::new(),
        };
        core.pin_provider(PROVIDER);
        core.register(Box::new(module));
        PluginsHandle { shared }
    }

    /// Answers an action chosen in the review `view`.
    fn choose(&mut self, view: &ViewId, action: &str) -> anyhow::Result<Vec<Effect>> {
        let decision = match action {
            NOT_NOW => return Ok(vec![Effect::PopView]),
            ALLOW => GrantDecision::Allow,
            DENY => GrantDecision::Deny,
            other => bail!("unknown action {other:?}"),
        };
        let Some(open) = self.open.get(view) else {
            return Ok(vec![toast(
                ToastKind::Error,
                "This review is closed",
                Some("Open the plugin's review again to decide."),
            )]);
        };
        // Held from the pending check through the grant write and the retain: a review that
        // `set_reviews` replaces meanwhile cannot be decided on stale content.
        let mut shared = lock(&self.shared);
        if !shared.is_pending(&open.review) {
            drop(shared);
            self.open.remove(view);
            return Ok(vec![
                Effect::PopView,
                toast(
                    ToastKind::Error,
                    "This plugin changed; review it again",
                    None,
                ),
            ]);
        }
        if decision == GrantDecision::Allow
            && (self.clock)().saturating_duration_since(open.shown_at) < MIN_READ_TIME
        {
            return Ok(vec![toast(
                ToastKind::Error,
                "Take a moment to read the permissions",
                None,
            )]);
        }
        let review = &open.review;
        let now = unix_now();
        let grant = Grant {
            plugin_id: review.id.clone(),
            content_hash: review.content_hash,
            permissions: review.permissions.clone(),
            name: review.name.clone(),
            author: review.author.clone(),
            widgets: review.widgets.clone(),
            decision,
            decided_at: now,
            updated_at: now,
        };
        if let Err(e) = self.db.set_grant(&grant) {
            tracing::error!(plugin = %review.id, "cannot store the plugin decision: {e:#}");
            return Ok(vec![toast(
                ToastKind::Error,
                "Cannot save the decision",
                Some(&format!("{e:#}")),
            )]);
        }
        let review = self.open.remove(view).expect("looked up above").review;
        let (id, content_hash) = (review.id.clone(), review.content_hash);
        let (event, title) = match decision {
            GrantDecision::Allow => (
                PluginEvents::Approved { id, content_hash },
                "Plugin allowed",
            ),
            GrantDecision::Deny => (PluginEvents::Denied { id, content_hash }, "Plugin denied"),
        };
        if self.events.send(event).is_err() {
            tracing::warn!(plugin = %review.id, "the plugin manager is gone");
        }
        // This content is decided; a newer review of the same plugin stays.
        shared
            .reviews
            .retain(|r| r.id != review.id || r.content_hash != review.content_hash);
        shared.publish();
        drop(shared);
        Ok(vec![
            Effect::PopView,
            toast(ToastKind::Success, title, Some(&review.name)),
        ])
    }
}

impl Module for PluginsModule {
    fn id(&self) -> &str {
        PROVIDER
    }

    fn start(&mut self, ctx: ModuleContext) {
        let mut shared = lock(&self.shared);
        shared.ctx = Some(ctx);
        shared.publish();
    }

    fn run_action(&mut self, item: &Item, action_id: &str) -> anyhow::Result<Vec<Effect>> {
        if action_id != REVIEW_ACTION {
            bail!("unknown action {action_id:?}");
        }
        let review = lock(&self.shared)
            .reviews
            .iter()
            .find(|r| review_item_id(r) == item.id)
            .cloned()
            .ok_or_else(|| anyhow!("no pending review for {}", item.id.as_str()))?;
        let view = review_view(&review);
        let shown_at = (self.clock)();
        self.open
            .insert(view.id.clone(), OpenReview { review, shown_at });
        Ok(vec![Effect::PushView(View::Detail(view))])
    }

    fn on_view_event(&mut self, view: &ViewId, event: ViewEvent) -> anyhow::Result<Vec<Effect>> {
        match event {
            ViewEvent::ActionChosen { action_id, .. } => self.choose(view, &action_id),
            ViewEvent::Closed => {
                self.open.remove(view);
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn toast(kind: ToastKind, title: &str, message: Option<&str>) -> Effect {
    Effect::Toast(Toast {
        kind,
        title: title.to_owned(),
        message: message.map(str::to_owned),
    })
}

fn review_item_id(review: &ReviewState) -> ItemId {
    ItemId::new(PROVIDER, &format!("review-{}", review.id))
}

fn review_title(review: &ReviewState) -> String {
    format!("Review plugin: {}", review.name)
}

fn review_item(review: &ReviewState) -> Item {
    let subtitle = match review.reason {
        ReviewReason::New => "New plugin needs your approval",
        ReviewReason::MorePermissions => "Plugin update asks for more permissions",
        ReviewReason::Renamed => "Plugin update changed its name or author",
        ReviewReason::NewWidgets => "Plugin update adds or renames widgets",
    };
    Item {
        id: review_item_id(review),
        title: review_title(review),
        subtitle: Some(subtitle.to_owned()),
        keywords: vec!["plugin".into(), "permissions".into(), review.id.clone()],
        icon: None,
        kind: ItemKind::Command,
        actions: vec![Action {
            id: REVIEW_ACTION.into(),
            title: "Review".into(),
            shortcut: None,
        }],
        source: None,
    }
}

fn hash_prefix(hash: &[u8; 32]) -> String {
    hash[..HASH_PREFIX_BYTES]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `text` as literal markdown text: every ASCII punctuation character is backslash-escaped
/// (CommonMark allows that for all of them), so no emphasis, link, image, code, HTML, entity,
/// heading or list marker can come out of it; control characters and the Unicode line and
/// paragraph separators become U+FFFD so the text stays on its line.
fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
            out.push(c);
        } else if c.is_control() || c == '\u{2028}' || c == '\u{2029}' {
            out.push('\u{fffd}');
        } else {
            out.push(c);
        }
    }
    out
}

/// A [`Permissions::describe`](crate::Permissions::describe) line as the prompt words it:
/// "Store data …" becomes "Can store data …".
fn can(line: &str) -> String {
    let mut chars = line.chars();
    match chars.next() {
        Some(first) => format!("Can {}{}", first.to_lowercase(), chars.as_str()),
        None => String::new(),
    }
}

/// The prompt. Every value comes from the manifest or the file system and is written as plain
/// text after a host label, so none of it is read as markup and none starts a line.
fn review_markdown(review: &ReviewState) -> String {
    let mut lines = vec![
        format!("Name: {}", plain(&review.name)),
        format!("Id: {}", plain(&review.id)),
        format!("Version: {}", plain(&review.version)),
    ];
    if let Some(author) = &review.author {
        lines.push(format!("Author: {}", plain(author)));
    }
    lines.push(format!("Folder: {}", plain(&review.dir.to_string_lossy())));
    let permissions = review.permissions.describe();
    if permissions.is_empty() {
        lines.push("It asks for no permissions.".to_owned());
    } else {
        lines.push("It asks to:".to_owned());
        let list: Vec<String> = permissions
            .iter()
            .map(|line| format!("- {}", plain(&can(line))))
            .collect();
        lines.push(list.join("\n"));
    }
    if !review.widgets.is_empty() {
        let titles: Vec<String> = review.widgets.iter().map(|w| plain(&w.title)).collect();
        lines.push(format!(
            "It shows widgets in the launcher: {}",
            titles.join(", ")
        ));
    }
    lines.push(format!(
        "Content hash: {}",
        hash_prefix(&review.content_hash)
    ));
    lines.join("\n\n")
}

fn review_view(review: &ReviewState) -> DetailView {
    let action = |id: &str, title: &str| Action {
        id: id.into(),
        title: title.into(),
        shortcut: None,
    };
    DetailView {
        // The hash makes the view of one content distinct from the view of another.
        id: ViewId(format!(
            "{PROVIDER}/review-{}@{}",
            review.id,
            hash_prefix(&review.content_hash)
        )),
        title: review_title(review),
        markdown: review_markdown(review),
        metadata: Vec::new(),
        // "Not now" is the primary (Enter) action; Allow and Deny are in the action panel.
        actions: vec![
            action(NOT_NOW, "Not now"),
            action(ALLOW, "Allow"),
            action(DENY, "Deny"),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_escapes_every_ascii_punctuation() {
        assert_eq!(plain("a*b"), "a\\*b");
        assert_eq!(plain("end\\"), "end\\\\");
        assert_eq!(plain("x\ny\tz"), "x\u{fffd}y\u{fffd}z");
        assert_eq!(plain("a\u{2028}b\u{2029}c"), "a\u{fffd}b\u{fffd}c");
        assert_eq!(plain("файл 1"), "файл 1");
        let all: String = (0x21u8..0x7f)
            .map(char::from)
            .filter(char::is_ascii_punctuation)
            .collect();
        assert_eq!(plain(&all).len(), all.len() * 2);
    }

    #[test]
    fn can_lowercases_the_first_letter() {
        assert_eq!(
            can("Make web requests to a.test"),
            "Can make web requests to a.test"
        );
        assert_eq!(can(""), "");
    }
}
